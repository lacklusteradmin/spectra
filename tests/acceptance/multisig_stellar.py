#!/usr/bin/env python3
"""A Stellar payment its signers sign across two data directories, against
a loopback Horizon.

The account of core/tests/fixtures/stellar-multisig.json (three signers,
medium threshold two) is watched in both directories. A payment with a
text memo is built in one; P1 signs it there, P2 signs the copy the other
directory imports, the copies join, and the envelope is submitted once,
under the session's transaction hash. The rules each step is held to are
core's tests (service::multisig_stellar).
"""
import base64
import hashlib
from urllib.parse import urlsplit

from harness import Handler, Spectra, fixture, scratch, serve

FIXTURE = fixture('stellar-multisig.json')
KEYS = FIXTURE['keys']
ACCOUNT = KEYS['p0']['address']
RECIPIENT = FIXTURE['transactions'][0]['payment']['destination']
PASSPHRASE = 'Test SDF Network ; September 2015'


def transaction_hash(envelope):
    """The hash of a payment envelope's transaction, as Horizon answers it."""
    at = 4 + 36 + 4 + 8 + 4 + 16
    memo = int.from_bytes(envelope[at:at + 4], 'big')
    at += 4
    if memo == 1:
        length = int.from_bytes(envelope[at:at + 4], 'big')
        at += 4 + length + (-length % 4)
    elif memo == 2:
        at += 8
    at += 4 + 4 + 4 + 36 + 4 + 8 + 4
    network = hashlib.sha256(PASSPHRASE.encode()).digest()
    return hashlib.sha256(network + (2).to_bytes(4, 'big') + envelope[4:at]).hexdigest()


def account_json(address):
    if address == ACCOUNT:
        return FIXTURE['horizon']['account_response']
    return {'id': address, 'account_id': address, 'sequence': '700', 'subentry_count': 0,
            'thresholds': {'low_threshold': 0, 'med_threshold': 0, 'high_threshold': 0},
            'balances': [{'balance': '50.0000000', 'asset_type': 'native', 'buying_liabilities': '0.0000000',
                          'selling_liabilities': '0.0000000'}],
            'signers': [{'weight': 1, 'key': address, 'type': 'ed25519_public_key'}], 'data': {},
            'flags': {'auth_immutable': False}}


class Horizon(Handler):
    def do_GET(self):
        path = urlsplit(self.path).path.rstrip('/')
        if path == '':
            return self.reply({'network_passphrase': PASSPHRASE})
        if path == '/fee_stats':
            return self.reply({'fee_charged': {'mode': '100'}})
        if path == '/ledgers':
            return self.reply({'_embedded': {'records': [{'base_reserve_in_stroops': 5_000_000}]}})
        if path.startswith('/accounts/'):
            return self.reply(account_json(path.split('/')[2]))
        self.reply({'status': 404}, 404)

    def do_POST(self):
        envelope = base64.b64decode(self.body()['tx'])
        self.state['submitted'].append(envelope)
        self.reply({'hash': transaction_hash(envelope), 'successful': True})


with serve(Horizon, submitted=[]) as node, scratch('stellar-multisig') as directory:
    spectra = Spectra(directory)

    a, b = spectra / 'a', spectra / 'b'
    for signer, name, key in [(a, 'Signer1', 'p1'), (b, 'Signer2', 'p2')]:
        signer.run('endpoints', '--chain', 'stellar-testnet', '--api', 'horizon', '--capabilities',
                   'balance,fee,verification,broadcast', '--add', node.url)
        signer.run('endpoints', '--chain', 'stellar-testnet', '--custom-only', 'true')
        signer.run('wallet', 'watch', '--chain', 'stellar-testnet', '--name', 'Account', '--address', ACCOUNT)
        signer.run('wallet', 'import', '--chain', 'stellar-testnet', '--name', name, '--no-password',
                   env={'SPECTRA_SEED': KEYS[key]['phrase']})

    created = a.run('multisig', 'create', 'Account', '--to', RECIPIENT, '--amount', '10',
                    '--memo-text', 'rent')['session']
    by_p1 = a.sign(created, 'Signer1')
    at_b = b.run('multisig', 'import', 'Account', '--data', by_p1['data'])['session']
    assert at_b['reviewDigest'] == created['reviewDigest'], at_b
    by_p2 = b.sign(at_b, 'Signer2')
    assert by_p2['complete'], by_p2

    joined = a.run('multisig', 'import', 'Account', '--data', by_p2['data'])['session']
    assert joined['id'] == created['id'] and joined['complete'], joined
    sent = a.run('multisig', 'submit', created['id'], '--yes')['session']
    [envelope] = node.state['submitted']
    assert sent['submittedTxid'] == transaction_hash(envelope) == created['transactionId'], sent
print('PASS stellar multisig: a payment with a memo signed in two data directories, joined and submitted '
      'once under its session\'s transaction hash')
