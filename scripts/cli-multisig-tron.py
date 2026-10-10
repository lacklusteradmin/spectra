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
FIXTURE = json.loads((root / 'core/tests/fixtures/tron-multisig.json').read_text())
GENESIS = '00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc'
KEYS = FIXTURE['keys']
ACCOUNT = FIXTURE['account']
broadcasts = []


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
                return self.answer(FIXTURE['getaccount'])
            return self.answer({'address': body['address'], 'balance': 10_000_000})
        if path == '/wallet/getchainparameters':
            return self.answer({'chainParameter': [{'key': 'getMultiSignFee', 'value': 1_000_000},
                                                   {'key': 'getTransactionFee', 'value': 1000},
                                                   {'key': 'getEnergyFee', 'value': 210}]})
        if path == '/wallet/broadcasttransaction':
            broadcasts.append(body)
            return self.answer({'result': True, 'txid': body['txID']})
        self.send_error(404, path)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
try:
    with tempfile.TemporaryDirectory(prefix='spectra-tron-multisig-') as directory:

        def run(data, *args, env=None):
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120, env={**os.environ, **(env or {})})
            assert result.returncode == 0, (data, args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        def signed_by(session):
            return sorted(s['signer'] for s in session['signers'] if s['signed'])

        for data, key in [('a', 1), ('b', 2)]:
            run(data, 'endpoints', '--chain', 'tron', '--api', 'tron-http', '--capabilities',
                'balance,fee,verification,broadcast', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'tron', '--custom-only', 'true')
            run(data, 'wallet', 'watch', '--chain', 'tron', '--name', 'Account', '--address', ACCOUNT)
            run(data, 'wallet', 'import', '--chain', 'tron', '--name', 'Signer', '--no-password',
                '--private-key-env', 'KEY', env={'KEY': KEYS[key]['private_key']})

        # P1 builds the session and signs; P2 signs P1's copy.
        created = run('a', 'multisig', 'create', '--from', 'Account', '--to', FIXTURE['recipient'],
                      '--amount', '1')['session']
        by_p1 = run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'],
                    '--signer', 'Signer')['session']
        assert signed_by(by_p1) == [KEYS[1]['address']] and not by_p1['complete'], by_p1
        at_b = run('b', 'multisig', 'import', '--wallet', 'Account', '--data', by_p1['data'])['session']
        assert at_b['reviewDigest'] == created['reviewDigest'], at_b
        by_p2 = run('b', 'multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'],
                    '--signer', 'Signer')['session']
        assert by_p2['complete'], by_p2

        # P1 joins P2's copy and broadcasts it once.
        joined = run('a', 'multisig', 'import', '--wallet', 'Account', '--data', by_p2['data'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        assert signed_by(joined) == sorted([KEYS[1]['address'], KEYS[2]['address']]), joined
        assert not broadcasts
        sent = run('a', 'multisig', 'submit', created['id'], '--yes')['session']
        assert sent['submittedTxid'] == created['transactionId'], sent
        [broadcast] = broadcasts
        assert broadcast['txID'] == created['transactionId'] and len(broadcast['signature']) == 2, broadcast
        assert run('a', 'multisig', 'show', created['id'])['session']['submittedTxid'] == created['transactionId']
    print('PASS tron multisig: an active-permission session signed in two data directories, '
          'joined and broadcast once')
finally:
    server.shutdown()
    server.server_close()
