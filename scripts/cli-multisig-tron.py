#!/usr/bin/env python3
"""A Tron account whose permissions need two keys, against a loopback node,
across two data directories.

The account's owner (2 of 3) and active (2 of 2, TRX, TRC-10 and TRC-20)
permissions are read from `wallet/getaccount` (core/tests/fixtures/
tron-multisig.json, TronWeb's keys). The account's own key no longer sends
alone, so an ordinary send is refused before anything is signed, saying
why. A session pays TRX under the active permission with a deadline an
hour out; P1 signs in one data directory, P2 in the other from P1's copy,
the copies join and the transaction broadcasts carrying `Permission_id` 2
and both signatures, each recovered here to its key. Refused on the way: a
deadline past a day, a key with no weight in the permission, a key twice, a
broadcast short of the threshold, a permission changed after the session
was built, and a signature after the deadline.
"""
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

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
root = pathlib.Path(__file__).resolve().parents[1]
FIXTURE = json.loads((root / 'core/tests/fixtures/tron-multisig.json').read_text())
GENESIS = '00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc'
KEYS = FIXTURE['keys']
ACCOUNT = FIXTURE['account']
RECIPIENT = FIXTURE['recipient']
state = {}

# secp256k1, to recover the key behind each broadcast signature.
P = 2**256 - 2**32 - 977
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
G = (0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798,
     0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8)


def add(a, b):
    if a is None:
        return b
    if b is None:
        return a
    if a[0] == b[0] and (a[1] + b[1]) % P == 0:
        return None
    if a == b:
        slope = 3 * a[0] * a[0] * pow(2 * a[1], -1, P) % P
    else:
        slope = (b[1] - a[1]) * pow(b[0] - a[0], -1, P) % P
    x = (slope * slope - a[0] - b[0]) % P
    return x, (slope * (a[0] - x) - a[1]) % P


def mul(k, point):
    result = None
    while k:
        if k & 1:
            result = add(result, point)
        point = add(point, point)
        k >>= 1
    return result


def recover(digest, signature):
    r, s, v = int.from_bytes(signature[:32], 'big'), int.from_bytes(signature[32:64], 'big'), signature[64] - 27
    x = r
    y = pow((x ** 3 + 7) % P, (P + 1) // 4, P)
    if y % 2 != v:
        y = P - y
    z = int.from_bytes(digest, 'big')
    inverse = pow(r, -1, N)
    return add(mul(s * inverse % N, (x, y)), mul((-z * inverse) % N, G))


def keccak(data):
    rc = [0x0000000000000001, 0x0000000000008082, 0x800000000000808A, 0x8000000080008000, 0x000000000000808B,
          0x0000000080000001, 0x8000000080008081, 0x8000000000008009, 0x000000000000008A, 0x0000000000000088,
          0x0000000080008009, 0x000000008000000A, 0x000000008000808B, 0x800000000000008B, 0x8000000000008089,
          0x8000000000008003, 0x8000000000008002, 0x8000000000000080, 0x000000000000800A, 0x800000008000000A,
          0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008]
    rot = [[0, 36, 3, 41, 18], [1, 44, 10, 45, 2], [62, 6, 43, 15, 61], [28, 55, 25, 21, 56], [27, 20, 39, 8, 14]]
    mask = (1 << 64) - 1
    data = bytearray(data) + b'\x01'
    data += b'\x00' * (-len(data) % 136)
    data[-1] |= 0x80
    lanes = [[0] * 5 for _ in range(5)]
    for offset in range(0, len(data), 136):
        for i in range(17):
            lanes[i % 5][i // 5] ^= int.from_bytes(data[offset + 8 * i:offset + 8 * i + 8], 'little')
        for constant in rc:
            c = [lanes[x][0] ^ lanes[x][1] ^ lanes[x][2] ^ lanes[x][3] ^ lanes[x][4] for x in range(5)]
            d = [c[(x - 1) % 5] ^ (((c[(x + 1) % 5] << 1) | (c[(x + 1) % 5] >> 63)) & mask) for x in range(5)]
            lanes = [[lanes[x][y] ^ d[x] for y in range(5)] for x in range(5)]
            b = [[0] * 5 for _ in range(5)]
            for x in range(5):
                for y in range(5):
                    r = rot[x][y]
                    b[y][(2 * x + 3 * y) % 5] = ((lanes[x][y] << r) | (lanes[x][y] >> (64 - r))) & mask if r else lanes[x][y]
            lanes = [[b[x][y] ^ ((~b[(x + 1) % 5][y]) & b[(x + 2) % 5][y]) for y in range(5)] for x in range(5)]
            lanes[0][0] ^= constant
    return b''.join(lanes[i % 5][i // 5].to_bytes(8, 'little') for i in range(4))


def tron_hex(point):
    public = point[0].to_bytes(32, 'big') + point[1].to_bytes(32, 'big')
    return '41' + keccak(public)[12:].hex()


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def answer(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))) or b'{}')
        path = self.path
        if path == '/wallet/getblockbynum':
            return self.answer({'blockID': GENESIS})
        if path == '/wallet/getnowblock':
            return self.answer({'blockID': FIXTURE['block']['id'], 'block_header': {'raw_data': {
                'number': FIXTURE['block']['number']}}})
        if path == '/wallet/getaccount':
            if body['address'] == ACCOUNT:
                return self.answer(state['account'])
            return self.answer({'address': body['address'], 'balance': 10_000_000})
        if path == '/wallet/getchainparameters':
            return self.answer({'chainParameter': [{'key': 'getMultiSignFee', 'value': 1_000_000},
                                                   {'key': 'getTransactionFee', 'value': 1000},
                                                   {'key': 'getEnergyFee', 'value': 210}]})
        if path == '/wallet/broadcasttransaction':
            state['broadcasts'].append(body)
            return self.answer({'result': True, 'txid': body['txID']})
        self.send_error(404, path)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
state.update(account=json.loads(json.dumps(FIXTURE['getaccount'])), broadcasts=[])
try:
    with tempfile.TemporaryDirectory(prefix='spectra-tron-multisig-') as directory:
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

        def key_wallet(data, name, index):
            run(data, 'wallet', 'import', '--chain', 'tron', '--name', name, '--no-password',
                '--private-key-env', 'KEY', env={'KEY': KEYS[index]['private_key']})

        def signed_by(session):
            return sorted(s['signer'] for s in session['signers'] if s['signed'])

        for data in ['a', 'b']:
            run(data, 'endpoints', '--chain', 'tron', '--api', 'tron-http', '--capabilities',
                'balance,fee,verification,broadcast', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'tron', '--custom-only', 'true')
        key_wallet('a', 'Account', 0)
        key_wallet('a', 'Signer1', 1)
        run('b', 'wallet', 'watch', '--chain', 'tron', '--name', 'Account', '--address', ACCOUNT)
        key_wallet('b', 'Signer2', 2)

        # The permissions, from the network; the account's own key no
        # longer sends alone, and an ordinary send says so before signing.
        account = run('a', 'multisig', 'account', 'Account')['account']
        owner, active = account['permissions']
        assert (owner['threshold'], len(owner['signers']), owner['covers']) == (2, 3, []), owner
        assert active['threshold'] == 2 and 'TransferContract' in active['covers'], active
        assert account['warnings'], account
        run('a', 'send', 'build', '--from', 'Account', '--to', RECIPIENT, '--amount', '1', '--endpoint', endpoint,
            refusal='cannot send from the account alone')

        run('a', 'multisig', 'create', '--from', 'Account', '--to', RECIPIENT, '--amount', '1',
            '--expires-in', str(25 * 3600), refusal='within a day')
        created = run('a', 'multisig', 'create', '--from', 'Account', '--to', RECIPIENT, '--amount', '1',
                      '--expires-in', '3600')['session']
        assert created['scheme'] == 'tronPermissions' and created['threshold'] == 2, created
        assert created['outputs'][0]['value'] == '1000000' and created['fee'] == '1000000', created
        assert created['expiresAt'] - time.time() > 3000, created
        data = json.loads(created['data'])
        assert data['raw_data']['contract'][0]['Permission_id'] == 2, data
        assert data['txID'] == hashlib.sha256(bytes.fromhex(data['raw_data_hex'])).hexdigest()

        run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer',
            'Account', refusal='holds no weight')
        by_p1 = run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'],
                    '--signer', 'Signer1')['session']
        assert signed_by(by_p1) == [KEYS[1]['address']] and by_p1['signedWeight'] == 1, by_p1
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
        assert sent['submittedTxid'] == created['transactionId'], sent
        [broadcast] = state['broadcasts']
        raw = bytes.fromhex(broadcast['raw_data_hex'])
        digest = hashlib.sha256(raw).digest()
        assert broadcast['txID'] == digest.hex() == created['transactionId']
        signers = sorted(tron_hex(recover(digest, bytes.fromhex(s))) for s in broadcast['signature'])
        assert signers == sorted([KEYS[1]['hex'], KEYS[2]['hex']]), signers
        assert broadcast['raw_data']['contract'][0]['Permission_id'] == 2

        # A permission changed after the session was built is not signed
        # for, and nothing is signed after its deadline.
        later = run('b', 'multisig', 'create', '--from', 'Account', '--to', RECIPIENT, '--amount', '1')['session']
        state['account']['active_permission'][0]['threshold'] = 1
        run('b', 'multisig', 'sign', later['id'], '--review-digest', later['reviewDigest'], '--signer', 'Signer2',
            refusal='permission changed')
        brief = run('b', 'multisig', 'create', '--from', 'Account', '--to', RECIPIENT, '--amount', '1',
                    '--expires-in', '1')['session']
        time.sleep(1.5)
        run('b', 'multisig', 'sign', brief['id'], '--review-digest', brief['reviewDigest'], '--signer', 'Signer2',
            refusal='deadline has passed')
        assert len(state['broadcasts']) == 1
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
    print('PASS tron multisig: permissions read, an ordinary send the key cannot make alone refused before '
          'signing; an active-permission session signed in two data directories, joined and broadcast with both '
          'keys; long deadlines, keys without weight, double signatures, short thresholds, changed permissions '
          'and expired sessions refused')
finally:
    server.shutdown()
    server.server_close()
