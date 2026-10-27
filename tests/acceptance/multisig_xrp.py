#!/usr/bin/env python3
"""An XRP Ledger signer-list payment across two data directories, against a
loopback rippled.

The account of core/tests/fixtures/xrp-multisig.json (its signer list P1,
P2 and a third account, quorum 2, its master key disabled) is watched in
both directories. A tagged payment is built in one; P1 signs it there, P2
signs the copy the other directory imports, the copies join, and the blob
is finalized and submitted once, under the session's transaction id. The
rules each step is held to are core's tests (service::multisig_xrp).
"""
import hashlib

from harness import Handler, Spectra, fixture, scratch, serve

FIXTURE = fixture('xrp-multisig.json')
KEYS = FIXTURE['keys']
ACCOUNT = KEYS['p0']['address']
RECIPIENT = FIXTURE['transactions'][0]['tx']['Destination']
LIST = FIXTURE['rpc']['account_info_v1']['response']['result']['account_data']['signer_lists'][0]
LEDGER = 94_999_990


def txid(blob):
    return hashlib.sha512(b'TXN\0' + blob).digest()[:32].hex().upper()


class Node(Handler):
    def do_POST(self):
        request = self.body()
        method, params = request['method'], (request.get('params') or [{}])[0]
        if method == 'server_info':
            result = {'info': {'network_id': 0}}
        elif method == 'fee':
            result = {'drops': {'open_ledger_fee': '10'}}
        elif method == 'server_state':
            result = {'state': {'validated_ledger': {'seq': LEDGER - 1, 'reserve_base': 1_000_000,
                                                     'reserve_inc': 200_000}}}
        elif method == 'account_info' and params['account'] == ACCOUNT:
            data = {'Account': ACCOUNT, 'Balance': '50000000', 'Flags': 0x00100000, 'OwnerCount': 1, 'Sequence': 9}
            if params.get('signer_lists'):
                data['signer_lists'] = [LIST]
            result = {'account_data': data, 'ledger_current_index': LEDGER}
        elif method == 'account_info':
            result = {'account_data': {'Account': params['account'], 'Balance': '30000000', 'Flags': 0,
                                       'OwnerCount': 0, 'Sequence': 3}, 'ledger_current_index': LEDGER}
        elif method == 'submit':
            blob = bytes.fromhex(params['tx_blob'])
            self.state['submitted'].append(blob)
            result = {'engine_result': 'tesSUCCESS', 'engine_result_message': 'ok', 'accepted': True,
                      'tx_json': {'hash': txid(blob)}}
        else:
            raise AssertionError(request)
        self.reply({'result': result})


with serve(Node, submitted=[]) as node, scratch('xrp-multisig') as directory:
    spectra = Spectra(directory)
    a, b = spectra / 'a', spectra / 'b'

    for signer, name, key in [(a, 'Signer1', 'p1'), (b, 'Signer2', 'p2')]:
        signer.run('endpoints', '--chain', 'xrp', '--api', 'xrpl-json-rpc', '--capabilities',
                   'balance,fee,verification,broadcast', '--add', node.url)
        signer.run('endpoints', '--chain', 'xrp', '--custom-only', 'true')
        signer.run('wallet', 'watch', '--chain', 'xrp', '--name', 'Account', '--address', ACCOUNT)
        signer.run('wallet', 'import', '--chain', 'xrp', '--name', name, '--no-password',
                   env={'SPECTRA_SEED': KEYS[key]['phrase']})

    created = a.run('multisig', 'create', 'Account', '--to', RECIPIENT, '--amount', '1',
                    '--destination-tag', '7')['session']
    by_p1 = a.sign(created, 'Signer1')
    at_b = b.run('multisig', 'import', 'Account', '--data', by_p1['data'])['session']
    assert at_b['reviewDigest'] == created['reviewDigest'], at_b
    by_p2 = b.sign(at_b, 'Signer2')
    assert by_p2['complete'], by_p2

    joined = a.run('multisig', 'import', 'Account', '--data', by_p2['data'])['session']
    assert joined['id'] == created['id'] and joined['complete'], joined
    assert a.run('multisig', 'finalize', created['id'])['raw'] == joined['data']
    sent = a.run('multisig', 'submit', created['id'], '--yes')['session']
    [blob] = node.state['submitted']
    assert sent['submittedTxid'] == txid(blob) == joined['transactionId'], sent
print('PASS xrp multisig: a tagged payment signed in two data directories, joined, finalized and submitted '
      'once under its session\'s transaction id')
