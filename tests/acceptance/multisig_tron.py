#!/usr/bin/env python3
"""A Tron account whose permissions need two keys, against a loopback node,
across two data directories.

The account's owner (2 of 3) and active (2 of 2) permissions are read from
`wallet/getaccount` (core/tests/fixtures/tron-multisig.json, TronWeb's
keys). Each data directory watches the account and holds one of the active
permission's keys: P1 builds a session and signs, P2 signs P1's copy, P1
joins P2's copy and broadcasts it once, and the session keeps the
transaction id.

Core's tests own the rules: ordinary sends the account's key cannot make
alone, deadlines, weights, thresholds and changed permissions
(core/src/send/tests/tron_multisig.rs, core/src/service/tests/multisig_tron.rs).
"""
from harness import Handler, Spectra, fixture, scratch, serve, signed_by

FIXTURE = fixture('tron-multisig.json')
GENESIS = '00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc'
KEYS = FIXTURE['keys']
ACCOUNT = FIXTURE['account']


class Node(Handler):
    def do_POST(self):
        body = self.body() or {}
        path = self.path
        if path == '/wallet/getblockbynum':
            return self.reply({'blockID': GENESIS})
        if path == '/wallet/getnowblock':
            return self.reply({'blockID': FIXTURE['block']['id'], 'block_header': {'raw_data': {
                'number': FIXTURE['block']['number']}}})
        if path == '/wallet/getaccount':
            if body['address'] == ACCOUNT:
                return self.reply(FIXTURE['getaccount'])
            return self.reply({'address': body['address'], 'balance': 10_000_000})
        if path == '/wallet/getchainparameters':
            return self.reply({'chainParameter': [{'key': 'getMultiSignFee', 'value': 1_000_000},
                                                  {'key': 'getTransactionFee', 'value': 1000},
                                                  {'key': 'getEnergyFee', 'value': 210}]})
        if path == '/wallet/broadcasttransaction':
            self.state['broadcasts'].append(body)
            return self.reply({'result': True, 'txid': body['txID']})
        self.send_error(404, path)


with serve(Node, broadcasts=[]) as node, scratch('tron-multisig') as directory:
    spectra = Spectra(directory)
    a, b = spectra / 'a', spectra / 'b'
    broadcasts = node.state['broadcasts']

    for signer, key in [(a, 1), (b, 2)]:
        signer.run('endpoints', '--chain', 'tron', '--api', 'tron-http', '--capabilities',
                   'balance,fee,verification,broadcast', '--add', node.url)
        signer.run('endpoints', '--chain', 'tron', '--custom-only', 'true')
        signer.run('wallet', 'watch', '--chain', 'tron', '--name', 'Account', '--address', ACCOUNT)
        signer.run('wallet', 'import', '--chain', 'tron', '--name', 'Signer', '--no-password',
                   '--private-key-env', 'KEY', env={'KEY': KEYS[key]['private_key']})

    # P1 builds the session and signs; P2 signs P1's copy.
    created = a.run('multisig', 'create', 'Account', '--to', FIXTURE['recipient'], '--amount', '1')['session']
    by_p1 = a.sign(created, 'Signer')
    assert signed_by(by_p1) == [KEYS[1]['address']] and not by_p1['complete'], by_p1
    at_b = b.run('multisig', 'import', 'Account', '--data', by_p1['data'])['session']
    assert at_b['reviewDigest'] == created['reviewDigest'], at_b
    by_p2 = b.sign(at_b, 'Signer')
    assert by_p2['complete'], by_p2

    # P1 joins P2's copy and broadcasts it once.
    joined = a.run('multisig', 'import', 'Account', '--data', by_p2['data'])['session']
    assert joined['id'] == created['id'] and joined['complete'], joined
    assert signed_by(joined) == sorted([KEYS[1]['address'], KEYS[2]['address']]), joined
    assert not broadcasts
    sent = a.run('multisig', 'submit', created['id'], '--yes')['session']
    assert sent['submittedTxid'] == created['transactionId'], sent
    [broadcast] = broadcasts
    assert broadcast['txID'] == created['transactionId'] and len(broadcast['signature']) == 2, broadcast
    assert a.run('multisig', 'show', created['id'])['session']['submittedTxid'] == created['transactionId']
print('PASS tron multisig: an active-permission session signed in two data directories, '
      'joined and broadcast once')
