#!/usr/bin/env python3
"""A TRC-10 staged send against a loopback Tron node, across processes: built,
signed, broadcast once as the TransferAssetContract that was signed, and
recorded as the token's own send until a poll confirms it. Reads, refusals,
history and receipts are tested in core."""
import hashlib
import json
import urllib.parse

from harness import Handler, Spectra, fixture, scratch, serve

FIXTURE = fixture('trc10-send-vectors.json')
OWNER, RECEIVER = FIXTURE['owner'], FIXTURE['receiver']
GENESIS = '00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc'


def base58_hex(address):
    alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
    number = 0
    for char in address:
        number = number * 58 + alphabet.index(char)
    return number.to_bytes(25, 'big')[:21].hex()


class Node(Handler):
    def do_POST(self):
        body = self.body()
        route = urllib.parse.urlsplit(self.path).path
        if route == '/wallet/getblockbynum':
            return self.reply({'blockID': GENESIS})
        if route == '/wallet/getnowblock':
            return self.reply({'blockID': FIXTURE['block']['id'],
                               'block_header': {'raw_data': {'number': FIXTURE['block']['number']}}})
        if route == '/wallet/getassetissuebyid':
            return self.reply({'id': '1009999', 'name': 'LegacyTest'.encode().hex(),
                               'abbr': 'T10'.encode().hex(), 'precision': 2})
        if route == '/wallet/getaccount':
            owner = body['address']
            return self.reply({'address': owner, 'balance': 10_000_000 if owner == OWNER else 1,
                               'assetV2': [{'key': '1009999', 'value': 22345 if owner == OWNER else 0}]})
        if route == '/wallet/getchainparameters':
            return self.reply({'chainParameter': [{'key': 'getTransactionFee', 'value': 1000},
                {'key': 'getCreateAccountFee', 'value': 100_000},
                {'key': 'getCreateNewAccountFeeInSystemContract', 'value': 1_000_000}]})
        if route == '/wallet/broadcasttransaction':
            assert body['raw_data']['contract'][0]['type'] == 'TransferAssetContract', body
            assert 'fee_limit' not in body['raw_data'], body
            value = body['raw_data']['contract'][0]['parameter']['value']
            assert value == {'asset_name': '1009999'.encode().hex(), 'owner_address': base58_hex(OWNER),
                             'to_address': base58_hex(RECEIVER), 'amount': 12345}, value
            assert body['txID'] == hashlib.sha256(bytes.fromhex(body['raw_data_hex'])).hexdigest(), body
            self.state['payloads'].append(body)
            return self.reply({'result': True, 'txid': body['txID']})
        if route == '/walletsolidity/gettransactioninfobyid':
            if not self.state['payloads']: return self.reply({})
            return self.reply({'id': body['value'], 'blockNumber': 450, 'receipt': {'net_usage': 300}})
        return self.reply({'Error': 'unexpected route'}, 400)


with serve(Node, payloads=[]) as node, scratch('trc10') as directory:
    payloads, endpoint = node.state['payloads'], node.url
    spectra = Spectra(directory, SPECTRA_PRIVATE_KEY=FIXTURE['key'], SPECTRA_PASSWORD='trc10-vector-password')
    spectra.run('wallet', 'import', '--chain', 'tron', '--name', 'Legacy', '--private-key-env', 'SPECTRA_PRIVATE_KEY')
    spectra.run('endpoints', '--chain', 'tron', '--api', 'tron-http',
                '--capabilities', 'balance,fee,verification,token-balance,broadcast', '--add', endpoint)
    prepared = spectra.run('send', 'build', 'Legacy', '--to', RECEIVER, '--endpoint', endpoint,
                           '--contract', '1009999', '--decimals', '2', '--amount', '123.45')['artifact']
    signed = spectra.run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                         '--endpoint', endpoint)['artifact']
    assert not payloads, 'signing broadcasts nothing'
    assert spectra.run('send', 'inspect', signed['id'])['artifact']['signed_payload'] == signed['signed_payload']
    accepted = spectra.run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')['artifact']
    assert any(a['outcome'] == 'Accepted' for a in accepted['attempts']), accepted
    assert payloads == [json.loads(signed['signed_payload'])], payloads
    changes = spectra.run('txs', '--poll-chain', 'tron')['changes']
    assert changes and all(r['newStatus'] == 'confirmed' for r in changes), changes
    records = spectra.run('txs', '--page', '--wallet', 'Legacy')['page']['records']
    confirmed = next(r for r in records if r['transactionHash'] == accepted['transaction_hash'])
    assert (confirmed['status'], confirmed['deploymentId'], confirmed['amount']) == \
        ('confirmed', 'tron:trc-10:1009999', '123.45'), confirmed
print('TRC-10 staged send: signed, broadcast once as signed, recorded and confirmed')
