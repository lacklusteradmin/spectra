#!/usr/bin/env python3
"""An XRP Ledger account with a signer list and its master key disabled,
against a loopback rippled, across two data directories.

The account's policy is read from `account_info` with `signer_lists`
(core/tests/fixtures/xrp-multisig.json, xrpl.js's keys): its signer list
(P1, P2 and a third account, quorum 2) and its master key disabled, so an
ordinary send from the wallet holding that key is refused before anything
is signed. A session pays XRP with a destination tag, its fee the base fee
for every listed signer plus one and its LastLedgerSequence the deadline;
P1 signs in one data directory, P2 in the other from P1's copy, the copies
join and the blob submitted carries both signatures in account order, each
verified here against its key over its `SMT` signing data. Refused on the
way: a key not on the list, a key twice, a submission short of the quorum,
a signer whose master key is disabled, a signer list changed after the
session was built, a sequence used by another transaction, and a deadline
passed.
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

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
root = pathlib.Path(__file__).resolve().parents[1]
FIXTURE = json.loads((root / 'core/tests/fixtures/xrp-multisig.json').read_text())
KEYS = FIXTURE['keys']
ACCOUNT = KEYS['p0']['address']
RECIPIENT = FIXTURE['transactions'][0]['tx']['Destination']
DISABLE_MASTER = 0x00100000
state = {}

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
    slope = (3 * a[0] * a[0] * pow(2 * a[1], -1, P) if a == b else (b[1] - a[1]) * pow(b[0] - a[0], -1, P)) % P
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


def point_of(compressed):
    x = int.from_bytes(compressed[1:], 'big')
    y = pow((x ** 3 + 7) % P, (P + 1) // 4, P)
    return x, y if y % 2 == compressed[0] % 2 else P - y


def verify(digest, der, public):
    assert der[0] == 0x30 and der[2] == 0x02
    r_len = der[3]
    r = int.from_bytes(der[4:4 + r_len], 'big')
    s = int.from_bytes(der[6 + r_len:6 + r_len + der[5 + r_len]], 'big')
    assert s <= N // 2, 'not low-S'
    w = pow(s, -1, N)
    z = int.from_bytes(digest, 'big')
    point = add(mul(z * w % N, G), mul(r * w % N, point_of(public)))
    return point is not None and point[0] % N == r


def sha512_half(data):
    return hashlib.sha512(data).digest()[:32]


def vl(data, at):
    length = data[at]
    return data[at + 1:at + 1 + length], at + 1 + length


def signers_of(blob):
    """The signing fields and each `Signer` of a multi-signed blob."""
    start = blob.index(bytes([0xf3]), blob.index(bytes([0x83, 0x14])) + 22)
    fields, signers, at = blob[:start], [], start + 1
    while blob[at:at + 2] == bytes([0xe0, 0x10]):
        assert blob[at + 2] == 0x73
        public, at = vl(blob, at + 3)
        assert blob[at] == 0x74
        signature, at = vl(blob, at + 1)
        assert blob[at] == 0x81
        account, at = vl(blob, at + 1)
        assert blob[at] == 0xe1
        at += 1
        signers.append((account, public, signature))
    assert blob[at:] == bytes([0xf1])
    return fields, signers


def root_of(address):
    if address == ACCOUNT:
        return {'Account': ACCOUNT, 'Balance': '50000000', 'Flags': state['flags'], 'OwnerCount': 1,
                'Sequence': state['sequence']}
    return {'Account': address, 'Balance': '30000000', 'Flags': state['signer_flags'].get(address, 0),
            'OwnerCount': 0, 'Sequence': 3}


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method, params = request['method'], (request.get('params') or [{}])[0]
        if method == 'server_info':
            result = {'info': {'network_id': 0}}
        elif method == 'fee':
            result = {'drops': {'open_ledger_fee': '10'}}
        elif method == 'server_state':
            result = {'state': {'validated_ledger': {'seq': state['ledger'] - 1, 'reserve_base': 1_000_000,
                                                     'reserve_inc': 200_000}}}
        elif method == 'account_info':
            data = root_of(params['account'])
            if params['account'] == ACCOUNT and params.get('signer_lists'):
                data['signer_lists'] = [state['list']]
            result = {'account_data': data, 'ledger_current_index': state['ledger'], 'validated': False}
        elif method == 'submit':
            blob = bytes.fromhex(params['tx_blob'])
            state['submitted'].append(blob)
            result = {'engine_result': 'tesSUCCESS', 'engine_result_message': 'ok', 'accepted': True,
                      'tx_json': {'hash': sha512_half(b'TXN\0' + blob).hex().upper()}}
        else:
            raise AssertionError(request)
        body = json.dumps({'result': result}).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


LIST = json.loads(json.dumps(FIXTURE['rpc']['account_info_v1']['response']['result']['account_data']['signer_lists'][0]))
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
state.update(flags=DISABLE_MASTER, sequence=9, ledger=94_999_990, list=json.loads(json.dumps(LIST)),
             signer_flags={}, submitted=[])
try:
    with tempfile.TemporaryDirectory(prefix='spectra-xrp-multisig-') as directory:
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

        def key_wallet(data, name, key):
            run(data, 'wallet', 'import', '--chain', 'xrp', '--name', name, '--no-password',
                env={'SPECTRA_SEED': KEYS[key]['phrase']})

        def signed_by(session):
            return sorted(s['signer'] for s in session['signers'] if s['signed'])

        for data in ['a', 'b']:
            run(data, 'endpoints', '--chain', 'xrp', '--api', 'xrpl-json-rpc', '--capabilities',
                'balance,fee,verification,broadcast', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'xrp', '--custom-only', 'true')
        key_wallet('a', 'Account', 'p0')
        key_wallet('a', 'Signer1', 'p1')
        run('b', 'wallet', 'watch', '--chain', 'xrp', '--name', 'Account', '--address', ACCOUNT)
        key_wallet('b', 'Signer2', 'p2')

        account = run('a', 'multisig', 'account', 'Account')['account']
        [listed] = account['permissions']
        assert listed['name'] == 'signer list' and listed['threshold'] == 2 and len(listed['signers']) == 3, account
        assert account['warnings'], account
        run('a', 'send', 'build', '--from', 'Account', '--to', RECIPIENT, '--amount', '1', '--endpoint', endpoint,
            refusal='master key is disabled')

        created = run('a', 'multisig', 'create', '--from', 'Account', '--to', RECIPIENT, '--amount', '1',
                      '--destination-tag', '7')['session']
        assert created['scheme'] == 'xrplSignerList' and created['sequence'] == '9', created
        assert created['fee'] == '40' and created['outputs'][0]['memo'] == '7', created
        assert created['expiresAtHeight'] == state['ledger'] + 900, created

        run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer',
            'Account', refusal="not on the account's signer list")
        by_p1 = run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'],
                    '--signer', 'Signer1')['session']
        assert signed_by(by_p1) == [KEYS['p1']['address']], by_p1
        run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer',
            'Signer1', refusal='already signed')
        run('a', 'multisig', 'submit', created['id'], '--yes', refusal='do not yet meet')

        at_b = run('b', 'multisig', 'import', '--wallet', 'Account', '--data', by_p1['data'])['session']
        assert at_b['reviewDigest'] == created['reviewDigest'], at_b
        state['signer_flags'][KEYS['p2']['address']] = DISABLE_MASTER
        run('b', 'multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'], '--signer', 'Signer2',
            refusal='cannot sign as a signer')
        state['signer_flags'].clear()
        by_p2 = run('b', 'multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'],
                    '--signer', 'Signer2')['session']
        assert by_p2['complete'], by_p2

        joined = run('a', 'multisig', 'import', '--wallet', 'Account', '--data', by_p2['data'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        assert run('a', 'multisig', 'finalize', created['id'])['raw'] == joined['data']
        sent = run('a', 'multisig', 'submit', created['id'], '--yes')['session']
        [blob] = state['submitted']
        assert sent['submittedTxid'] == sha512_half(b'TXN\0' + blob).hex().upper() == joined['transactionId']
        fields, signers = signers_of(blob)
        assert [account for account, _, _ in signers] == sorted(account for account, _, _ in signers)
        assert sorted(account.hex().upper() for account, _, _ in signers) == sorted(
            KEYS[k]['account_id'] for k in ['p1', 'p2'])
        for account, public, signature in signers:
            assert verify(sha512_half(b'SMT\0' + fields + account), signature, public)

        # A changed list, a used sequence and a passed deadline each stop a
        # signature.
        later = run('b', 'multisig', 'create', '--from', 'Account', '--to', RECIPIENT, '--amount', '1',
                    '--destination-tag', '7')['session']
        state['list']['SignerQuorum'] = 1
        run('b', 'multisig', 'sign', later['id'], '--review-digest', later['reviewDigest'], '--signer', 'Signer2',
            refusal='signer list changed')
        state['list']['SignerQuorum'] = 2
        state['sequence'] = 10
        run('b', 'multisig', 'sign', later['id'], '--review-digest', later['reviewDigest'], '--signer', 'Signer2',
            refusal='sequence moved on')
        state['sequence'] = 9
        state['ledger'] += 1000
        run('b', 'multisig', 'sign', later['id'], '--review-digest', later['reviewDigest'], '--signer', 'Signer2',
            refusal='last ledger has passed')
        assert len(state['submitted']) == 1
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
    print('PASS xrp multisig: signer list and disabled master key read, an ordinary send refused before signing; '
          'a tagged payment signed in two data directories, joined and submitted with both signatures verified; '
          'unlisted keys, double signatures, short quorums, disabled signers, changed lists, used sequences and '
          'passed deadlines refused')
finally:
    server.shutdown()
    server.server_close()
