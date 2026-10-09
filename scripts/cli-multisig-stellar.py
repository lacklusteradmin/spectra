#!/usr/bin/env python3
"""A Stellar account with three signers and a medium threshold of two,
against a loopback Horizon, across two data directories.

The account's signers and thresholds are read from Horizon (core/tests/
fixtures/stellar-multisig.json, stellar-base's keys): its master key's
weight of one no longer meets a payment's threshold, so an ordinary send
from the wallet holding that key is refused before anything is signed. A
session pays lumens with a text memo, its sequence fixed and its time
bounds the deadline; P1 signs in one data directory, P2 in the other from
P1's copy, the copies join and the envelope submitted carries both
signatures, each verified here (ed25519) over the transaction hash. Refused
on the way: a key that is no signer, a key twice, a submission short of the
threshold, signers changed after the session was built, a sequence used by
another transaction, and a deadline passed.
"""
import base64
import hashlib
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import urlsplit

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
root = pathlib.Path(__file__).resolve().parents[1]
FIXTURE = json.loads((root / 'core/tests/fixtures/stellar-multisig.json').read_text())
KEYS = FIXTURE['keys']
ACCOUNT = KEYS['p0']['address']
RECIPIENT = FIXTURE['transactions'][0]['payment']['destination']
PASSPHRASE = 'Test SDF Network ; September 2015'
OUTSIDER = 'zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong'
state = {}

# Ed25519 verification (RFC 8032), to check each envelope signature.
q = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
d = -121665 * pow(121666, -1, q) % q
I = pow(2, (q - 1) // 4, q)


def recover_x(y, sign):
    x2 = (y * y - 1) * pow(d * y * y + 1, -1, q)
    x = pow(x2, (q + 3) // 8, q)
    if (x * x - x2) % q:
        x = x * I % q
    if x % 2 != sign:
        x = q - x
    return x


def edwards_add(a, b):
    (x1, y1), (x2, y2) = a, b
    x3 = (x1 * y2 + x2 * y1) * pow(1 + d * x1 * x2 * y1 * y2, -1, q)
    y3 = (y1 * y2 + x1 * x2) * pow(1 - d * x1 * x2 * y1 * y2, -1, q)
    return x3 % q, y3 % q


def scalar(e, point):
    result = (0, 1)
    while e:
        if e & 1:
            result = edwards_add(result, point)
        point = edwards_add(point, point)
        e >>= 1
    return result


def decode_point(data):
    y = int.from_bytes(data, 'little') & ((1 << 255) - 1)
    return recover_x(y, data[31] >> 7), y


BASE = (recover_x(4 * pow(5, -1, q) % q, 0), 4 * pow(5, -1, q) % q)


def ed25519_verify(public, message, signature):
    r, s = decode_point(signature[:32]), int.from_bytes(signature[32:], 'little')
    h = int.from_bytes(hashlib.sha512(signature[:32] + public + message).digest(), 'little') % L
    return scalar(s, BASE) == edwards_add(r, scalar(h, decode_point(public)))


def split_envelope(envelope):
    """The transaction and the signatures of a payment envelope."""
    at = 4 + 36 + 4 + 8 + 4 + 16
    memo = int.from_bytes(envelope[at:at + 4], 'big')
    at += 4
    if memo == 1:
        length = int.from_bytes(envelope[at:at + 4], 'big')
        at += 4 + length + (-length % 4)
    elif memo == 2:
        at += 8
    at += 4 + 4 + 4 + 36 + 4 + 8 + 4
    tx, rest = envelope[4:at], envelope[at:]
    count = int.from_bytes(rest[:4], 'big')
    signatures, at = [], 4
    for _ in range(count):
        hint = rest[at:at + 4]
        length = int.from_bytes(rest[at + 4:at + 8], 'big')
        signatures.append((hint, rest[at + 8:at + 8 + length]))
        at += 8 + length
    assert at == len(rest)
    return tx, signatures


def account_json(address):
    if address == ACCOUNT:
        return state['account']
    return {'id': address, 'account_id': address, 'sequence': '700', 'subentry_count': 0,
            'thresholds': {'low_threshold': 0, 'med_threshold': 0, 'high_threshold': 0},
            'balances': [{'balance': '50.0000000', 'asset_type': 'native', 'buying_liabilities': '0.0000000',
                          'selling_liabilities': '0.0000000'}],
            'signers': [{'weight': 1, 'key': address, 'type': 'ed25519_public_key'}], 'data': {},
            'flags': {'auth_immutable': False}}


class Horizon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, value, status=200):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

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
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        envelope = base64.b64decode(body['tx'])
        state['submitted'].append(envelope)
        tx, _ = split_envelope(envelope)
        digest = hashlib.sha256(hashlib.sha256(PASSPHRASE.encode()).digest() + (2).to_bytes(4, 'big') + tx)
        self.reply({'hash': digest.hexdigest(), 'successful': True})


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Horizon)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
state.update(account=json.loads(json.dumps(FIXTURE['horizon']['account_response'])), submitted=[])
try:
    with tempfile.TemporaryDirectory(prefix='spectra-stellar-multisig-') as directory:
        journal = pathlib.Path(directory) / 'network.jsonl'

        def run(data, *args, env=None, refusal=None):
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120,
                                    env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(journal), **(env or {})})
            assert (result.returncode == 0) == (refusal is None), (data, args, result.stdout, result.stderr)
            if refusal is not None:
                assert refusal in result.stdout + result.stderr, (refusal, args, result.stdout, result.stderr)
                return None
            return json.loads(result.stdout)

        def key_wallet(data, name, phrase):
            run(data, 'wallet', 'import', '--chain', 'stellar-testnet', '--name', name, '--no-password',
                env={'SPECTRA_SEED': phrase})

        def signed_by(session):
            return sorted(s['signer'] for s in session['signers'] if s['signed'])

        for data in ['a', 'b']:
            run(data, 'endpoints', '--chain', 'stellar-testnet', '--api', 'horizon', '--capabilities',
                'balance,fee,verification,broadcast', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'stellar-testnet', '--custom-only', 'true')
        key_wallet('a', 'Account', KEYS['p0']['phrase'])
        key_wallet('a', 'Signer1', KEYS['p1']['phrase'])
        key_wallet('a', 'Outsider', OUTSIDER)
        run('b', 'wallet', 'watch', '--chain', 'stellar-testnet', '--name', 'Account', '--address', ACCOUNT)
        key_wallet('b', 'Signer2', KEYS['p2']['phrase'])

        account = run('a', 'multisig', 'account', 'Account')['account']
        assert [p['threshold'] for p in account['permissions']] == [1, 2, 3], account
        assert len(account['permissions'][1]['signers']) == 3 and account['warnings'], account
        run('a', 'send', 'build', '--from', 'Account', '--to', RECIPIENT, '--amount', '1', '--endpoint', endpoint,
            refusal='cannot send from the account alone')

        created = run('a', 'multisig', 'create', '--from', 'Account', '--to', RECIPIENT, '--amount', '10',
                      '--memo-text', 'rent')['session']
        assert created['scheme'] == 'stellarSigners' and created['threshold'] == 2, created
        assert created['sequence'] == '123456789013' and created['fee'] == '100', created
        assert created['outputs'][0]['memo'] == 'rent' and created['outputs'][0]['value'] == '100000000', created
        run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer',
            'Outsider', refusal="not one of the account's signers")
        by_p1 = run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'],
                    '--signer', 'Signer1')['session']
        assert signed_by(by_p1) == [KEYS['p1']['address']] and by_p1['signedWeight'] == 1, by_p1
        run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer',
            'Signer1', refusal='already signed')
        run('a', 'multisig', 'submit', created['id'], '--yes', refusal='do not yet meet')

        at_b = run('b', 'multisig', 'import', '--wallet', 'Account', '--data', by_p1['data'])['session']
        assert at_b['reviewDigest'] == created['reviewDigest'], at_b
        by_p2 = run('b', 'multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'],
                    '--signer', 'Signer2')['session']
        assert by_p2['complete'], by_p2

        joined = run('a', 'multisig', 'import', '--wallet', 'Account', '--data', by_p2['data'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        sent = run('a', 'multisig', 'submit', created['id'], '--yes')['session']
        [envelope] = state['submitted']
        tx, signatures = split_envelope(envelope)
        digest = hashlib.sha256(hashlib.sha256(PASSPHRASE.encode()).digest() + (2).to_bytes(4, 'big') + tx).digest()
        assert sent['submittedTxid'] == digest.hex() == created['transactionId'], sent
        keys = {bytes.fromhex(KEYS[k]['public_key'])[28:]: bytes.fromhex(KEYS[k]['public_key']) for k in ['p1', 'p2']}
        assert sorted(hint for hint, _ in signatures) == sorted(keys)
        for hint, signature in signatures:
            assert ed25519_verify(keys[hint], digest, signature)

        # Changed signers, a used sequence and a passed deadline each stop a
        # signature.
        later = run('b', 'multisig', 'create', '--from', 'Account', '--to', RECIPIENT, '--amount', '1')['session']
        state['account']['thresholds']['med_threshold'] = 1
        run('b', 'multisig', 'sign', later['id'], '--review-digest', later['reviewDigest'], '--signer', 'Signer2',
            refusal='signers or thresholds changed')
        state['account']['thresholds']['med_threshold'] = 2
        state['account']['sequence'] = '123456789013'
        run('b', 'multisig', 'sign', later['id'], '--review-digest', later['reviewDigest'], '--signer', 'Signer2',
            refusal='sequence moved on')
        brief = run('b', 'multisig', 'create', '--from', 'Account', '--to', RECIPIENT, '--amount', '1',
                    '--expires-in', '1')['session']
        time.sleep(1.5)
        run('b', 'multisig', 'sign', brief['id'], '--review-digest', brief['reviewDigest'], '--signer', 'Signer2',
            refusal='deadline has passed')
        assert len(state['submitted']) == 1
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
    print('PASS stellar multisig: signers and thresholds read, an ordinary send the key cannot make alone refused; '
          'a payment with a memo signed in two data directories, joined and submitted with both signatures '
          'verified; outsiders, double signatures, short thresholds, changed signers, used sequences and passed '
          'deadlines refused')
finally:
    server.shutdown()
    server.server_close()
