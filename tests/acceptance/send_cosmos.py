#!/usr/bin/env python3
"""A Cosmos Hub send with a memo, staged across processes against a loopback
node: built from a simulation, signed, left uncertain while the node refuses
it and accepted as the same bytes, then confirmed by its status poll. The transfer's
bytes, its pricing and every refusal before signing are tested in core."""
import base64
import hashlib

from harness import ABANDON, Handler, Spectra, checked, protobuf, scratch, serve

RECIPIENT = 'cosmos1tehv5km5e9y706rc2gzk9yyun9dljjjnvyt3u0'
MEMO = '104857600'


class Node(Handler):
    """A Cosmos SDK node's REST gateway, for the one account a send reads."""

    def do_GET(self):
        s = self.state
        path, _, query = self.path.partition('?')
        if path == '/cosmos/base/tendermint/v1beta1/blocks/latest':
            return self.reply({'block': {'header': {'chain_id': 'cosmoshub-4', 'height': str(s['height'])}}})
        if path == '/feemarket/v1/gas_price/uatom':
            return self.reply({'price': {'denom': 'uatom', 'amount': '0.005000000000000000'}})
        if path == f"/cosmos/auth/v1beta1/accounts/{s['owner']}":
            return self.reply({'account': {'@type': '/cosmos.auth.v1beta1.BaseAccount', 'address': s['owner'],
                                           'account_number': '789012', 'sequence': str(s['sequence'])}})
        if path in (f"/cosmos/bank/v1beta1/spendable_balances/{s['owner']}/by_denom",
                    f"/cosmos/bank/v1beta1/balances/{s['owner']}/by_denom"):
            assert query == 'denom=uatom', query
            return self.reply({'balance': {'denom': 'uatom', 'amount': str(s['balance'])}})
        if path.startswith('/cosmos/tx/v1beta1/txs/'):
            if not s['included']:
                return self.reply({'code': 5, 'message': 'tx not found'}, 404)
            return self.reply({'tx_response': {'txhash': s['hash'], 'height': '1001', 'code': 0}})
        raise AssertionError(self.path)

    def do_POST(self):
        s = self.state
        body = self.body()
        if self.path == '/cosmos/tx/v1beta1/simulate':
            return self.reply({'gas_info': {'gas_wanted': '0', 'gas_used': '70000'}})
        if self.path == '/cosmos/tx/v1beta1/txs':
            raw = base64.b64decode(body['tx_bytes'])
            s['submitted'].append(raw)
            if s['refuse']:
                return self.reply({'tx_response': {'code': 13, 'raw_log': 'insufficient fee', 'txhash': ''}})
            return self.reply({'tx_response': {'code': 0, 'txhash': hashlib.sha256(raw).hexdigest().upper()}})
        raise AssertionError(self.path)


def transfer(raw):
    """A submitted `TxRaw`'s memo, timeout height and signatures."""
    fields = dict(protobuf(raw)[:2])
    tx_body = protobuf(fields[1])
    memo = [value for number, value in tx_body if number == 2]
    timeout = [value for number, value in tx_body if number == 3]
    signatures = [value for number, value in protobuf(raw) if number == 3]
    return memo[0].decode(), timeout[0], signatures


with serve(Node, owner='', sequence=7, height=1000, balance=5_000_000, refuse=True, included=False,
           submitted=[], errors=[], hash=None) as node, scratch('cosmos-send') as directory:
    s, endpoint = node.state, node.url
    run = checked(Spectra(directory, SPECTRA_SEED=ABANDON), s['errors'])
    s['owner'] = run('wallet', 'import', '--chain', 'cosmos-hub', '--name', 'Hub', '--no-password')['wallet']['address']
    run('endpoints', 'add', '--chain', 'cosmos-hub', '--api', 'cosmos-rest',
        '--capabilities', 'balance,fee,verification,broadcast', endpoint)
    prepared = run('send', 'build', 'Hub', '--to', RECIPIENT, '--amount', '1.5', '--memo-text', MEMO,
                   '--endpoint', endpoint)['artifact']
    assert prepared['memo'] == {'kind': 'memoText', 'value': MEMO}, prepared['memo']
    signed = run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                 '--endpoint', endpoint)['artifact']
    s['hash'] = signed['transaction_hash']
    assert not s['submitted'], 'signing submits nothing'
    broadcast = ('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')
    # A refusal leaves the submission uncertain, with the node's reason.
    refused = run(*broadcast)['artifact']['attempts'][-1]
    assert refused['outcome'] == 'Uncertain' and 'insufficient fee' in refused['detail'], refused
    s['refuse'] = False
    accepted = run(*broadcast)['artifact']
    assert accepted['attempts'][-1]['outcome'] == 'Accepted', accepted
    assert accepted['transaction_hash'] == s['hash']
    # The retry submitted the bytes signed in another process, unchanged.
    first, second = s['submitted']
    assert first == second and hashlib.sha256(second).hexdigest().upper() == s['hash']
    memo, timeout, signatures = transfer(second)
    assert memo == MEMO and timeout == 1000 + 107 and [len(sig) for sig in signatures] == [64]
    # The node includes it, and the status poll records the outcome.
    s['included'] = True
    run('txs', 'poll', 'cosmos-hub')
    record = run('txs', 'show', signed['id'])['record']
    assert record['status'] == 'confirmed' and record['transactionHash'] == s['hash'], record
    print('cosmos hub: memo send left uncertain, retried as signed and confirmed', flush=True)
