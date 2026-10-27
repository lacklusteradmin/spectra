#!/usr/bin/env python3
"""An Aptos MultiKey transfer its members sign across two data directories,
against a loopback Aptos node.

The MultiKey of core/tests/fixtures/aptos-multikey.json (two Ed25519 keys
and a secp256k1 key, two required) is watched from its policy in both
directories. A transfer is built in one; P0 signs it there, P1 signs the
copy the other directory imports, the copies join, and the signed
transaction is submitted once, under the session's transaction hash. The
rules each step is held to, and Sui's, are core's tests
(service::multisig_aptos, service::multisig_sui, send::aptos_multikey,
send::sui_multisig).
"""
import hashlib
import json
from urllib.parse import urlsplit

from harness import Handler, Spectra, fixture, scratch, serve

APTOS = fixture('aptos-multikey.json')


def transaction_hash(signed):
    prefix = hashlib.sha3_256(b'APTOS::Transaction').digest()
    return '0x' + hashlib.sha3_256(prefix + b'\0' + signed).hexdigest()


class Node(Handler):
    def do_GET(self):
        path = urlsplit(self.path).path
        if path == '/':
            return self.reply({'chain_id': 2, 'ledger_version': '9'})
        if path == '/estimate_gas_price':
            return self.reply({'gas_estimate': 100})
        if path.startswith('/accounts/'):
            return self.reply({'sequence_number': '5', 'authentication_key': APTOS['multi_key']['authentication_key']})
        self.reply({'error': path}, 404)

    def do_POST(self):
        body = self.raw_body()
        path = urlsplit(self.path).path
        if path == '/view':
            return self.reply(['10000000000'])
        assert path == '/transactions', path
        assert self.headers['Content-Type'] == 'application/x.aptos.signed_transaction+bcs'
        self.state['submitted'].append(body)
        self.reply({'hash': transaction_hash(body)})


with serve(Node, submitted=[]) as node, scratch('multikey') as directory:
    spectra = Spectra(directory)

    a, b = spectra / 'a', spectra / 'b'
    keys = [f"{k['scheme']}-pub-0x{k['public_key']}" for k in APTOS['keys']]
    policy = json.dumps({'signaturesRequired': 2, 'publicKeys': keys})
    for signer, name, phrase in [(a, 'Signer0', APTOS['keys'][0]['phrase']),
                                 (b, 'Signer1', APTOS['keys'][1]['phrase'])]:
        signer.run('endpoints', '--chain', 'aptos-testnet', '--api', 'aptos-rest', '--capabilities',
                   'balance,fee,verification,broadcast', '--add', node.url)
        signer.run('endpoints', '--chain', 'aptos-testnet', '--custom-only', 'true')
        signer.run('wallet', 'watch', '--chain', 'aptos-testnet', '--name', 'Shared', '--multisig', policy)
        signer.run('wallet', 'import', '--chain', 'aptos-testnet', '--name', name, '--no-password',
                   env={'SPECTRA_SEED': phrase})

    created = a.run('multisig', 'create', 'Shared', '--to', APTOS['transaction']['recipient'],
                    '--amount', '0.01')['session']
    by_p0 = a.sign(created, 'Signer0')
    at_b = b.run('multisig', 'import', 'Shared', '--data', by_p0['data'])['session']
    assert at_b['reviewDigest'] == created['reviewDigest'], at_b
    by_p1 = b.sign(at_b, 'Signer1')
    assert by_p1['complete'], by_p1
    joined = a.run('multisig', 'import', 'Shared', '--data', by_p1['data'])['session']
    assert joined['id'] == created['id'] and joined['complete'], joined
    sent = a.run('multisig', 'submit', created['id'], '--yes')['session']
    [signed] = node.state['submitted']
    assert sent['submittedTxid'] == transaction_hash(signed) == joined['transactionId'], sent
print('PASS aptos multisig: a MultiKey transfer signed in two data directories, joined and submitted once '
      'under its session\'s transaction hash')
