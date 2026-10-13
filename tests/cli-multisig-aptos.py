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
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
from urllib.parse import urlsplit

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
root = pathlib.Path(__file__).resolve().parents[1]
APTOS = json.loads((root / 'core/tests/fixtures/aptos-multikey.json').read_text())
submitted = []


def transaction_hash(signed):
    prefix = hashlib.sha3_256(b'APTOS::Transaction').digest()
    return '0x' + hashlib.sha3_256(prefix + b'\0' + signed).hexdigest()


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, value, status=200):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

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
        body = self.rfile.read(int(self.headers['Content-Length']))
        path = urlsplit(self.path).path
        if path == '/view':
            return self.reply(['10000000000'])
        assert path == '/transactions', path
        assert self.headers['Content-Type'] == 'application/x.aptos.signed_transaction+bcs'
        submitted.append(body)
        self.reply({'hash': transaction_hash(body)})


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
try:
    with tempfile.TemporaryDirectory(prefix='spectra-multikey-') as directory:
        def run(data, *args, env=None):
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120, env={**os.environ, **(env or {})})
            assert result.returncode == 0, (data, args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        def sign(data, session, signer):
            return run(data, 'multisig', 'sign', session['id'], '--review-digest', session['reviewDigest'],
                       '--signer', signer)['session']

        keys = [f"{k['scheme']}-pub-0x{k['public_key']}" for k in APTOS['keys']]
        policy = json.dumps({'signaturesRequired': 2, 'publicKeys': keys})
        for data, name, phrase in [('a', 'Signer0', APTOS['keys'][0]['phrase']),
                                   ('b', 'Signer1', APTOS['keys'][1]['phrase'])]:
            run(data, 'endpoints', '--chain', 'aptos-testnet', '--api', 'aptos-rest', '--capabilities',
                'balance,fee,verification,broadcast', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'aptos-testnet', '--custom-only', 'true')
            run(data, 'wallet', 'watch', '--chain', 'aptos-testnet', '--name', 'Shared', '--multisig', policy)
            run(data, 'wallet', 'import', '--chain', 'aptos-testnet', '--name', name, '--no-password',
                env={'SPECTRA_SEED': phrase})

        created = run('a', 'multisig', 'create', '--from', 'Shared', '--to', APTOS['transaction']['recipient'],
                      '--amount', '0.01')['session']
        by_p0 = sign('a', created, 'Signer0')
        at_b = run('b', 'multisig', 'import', '--wallet', 'Shared', '--data', by_p0['data'])['session']
        assert at_b['reviewDigest'] == created['reviewDigest'], at_b
        by_p1 = sign('b', at_b, 'Signer1')
        assert by_p1['complete'], by_p1
        joined = run('a', 'multisig', 'import', '--wallet', 'Shared', '--data', by_p1['data'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        sent = run('a', 'multisig', 'submit', created['id'], '--yes')['session']
        [signed] = submitted
        assert sent['submittedTxid'] == transaction_hash(signed) == joined['transactionId'], sent
    print('PASS aptos multisig: a MultiKey transfer signed in two data directories, joined and submitted once '
          'under its session\'s transaction hash')
finally:
    server.shutdown()
    server.server_close()
