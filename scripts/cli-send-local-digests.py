#!/usr/bin/env python3
"""Sui/Aptos saved local identifiers reject foreign receipts across restarts.

Loopback only. Official SDK byte/digest fixtures are verified by Rust tests.
"""
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
import urllib.parse


BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
FIXTURE = json.loads((pathlib.Path(__file__).resolve().parents[1] / 'core/tests/fixtures/send-audit-vectors.json').read_text())


for chain, api in [('sui', 'sui-json-rpc'), ('aptos', 'aptos-rest')]:
    with tempfile.TemporaryDirectory(prefix='spectra-local-digest-') as directory:
        live = {'hash': None, 'wrong': True}
        submitted = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass

            def reply(self, value):
                data = json.dumps(value).encode()
                self.send_response(200)
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                path = urllib.parse.urlsplit(self.path).path
                if path == '/': self.reply({'chain_id': 1, 'ledger_version': '1'})
                elif path == '/estimate_gas_price': self.reply({'gas_estimate': 100})
                elif path.startswith('/accounts/'): self.reply({'sequence_number': '7'})
                elif path.startswith('/transactions/by_hash/'):
                    assert path == '/transactions/by_hash/' + live['hash'], path
                    self.reply({'hash': live['hash'], 'type': 'pending_transaction'})
                else: raise AssertionError(path)

            def do_POST(self):
                value = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                path = urllib.parse.urlsplit(self.path).path
                if path == '/view': self.reply(['10000000000']); return
                if path == '/transactions':
                    submitted.append(value)
                    self.reply({'hash': '0x' + 'ab' * 32 if live['wrong'] else live['hash']})
                    return
                method = value['method']
                if method == 'sui_getChainIdentifier': result = '35834a8a'
                elif method == 'sui_getCheckpoint':
                    result = {'sequenceNumber': '0', 'digest': '4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S'}
                elif method == 'suix_getReferenceGasPrice': result = '1000'
                elif method == 'suix_getCoins':
                    result = {'data': [{'coinObjectId': '0x' + '33' * 32, 'version': '7', 'digest': '1' * 32,
                                        'balance': '200000000'}], 'hasNextPage': False, 'nextCursor': None}
                elif method == 'sui_executeTransactionBlock':
                    submitted.append(value['params'])
                    result = {'digest': '1' * 32 if live['wrong'] else live['hash'],
                              'effects': {'status': {'status': 'success'}}}
                else: raise AssertionError(method)
                self.reply({'jsonrpc': '2.0', 'id': value['id'], 'result': result})

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            def run(*args):
                result = subprocess.run([BINARY, '--data-dir', directory, '--json', *args],
                    capture_output=True, text=True, timeout=60, env={**os.environ,
                    'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl'),
                    'SPECTRA_SEED': FIXTURE['mnemonic']})
                assert result.returncode == 0, (args, result.stdout, result.stderr)
                return json.loads(result.stdout)

            endpoint = f'http://127.0.0.1:{server.server_port}'
            run('wallet', 'import', '--chain', chain, '--name', 'Digest', '--no-password')
            run('endpoints', '--chain', chain, '--api', api,
                '--capabilities', 'balance,fee,verification,broadcast', '--add', endpoint)
            prepared = run('send', 'build', '--from', 'Digest', '--to', '0x' + '22' * 32,
                '--amount', '0.123456789' if chain == 'sui' else '1.23456789', '--endpoint', endpoint)['artifact']
            signed = run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                '--endpoint', endpoint)['artifact']
            live['hash'] = signed['transaction_hash']
            assert live['hash'] and submitted == [], signed
            if chain == 'sui': assert live['hash'] == FIXTURE['sui']['transaction_digest'], signed
            else: assert live['hash'].startswith('0x') and len(live['hash']) == 66, signed
            broadcast = ('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')
            uncertain = run(*broadcast)['artifact']
            assert uncertain['attempts'][-1]['outcome'] == 'Uncertain', uncertain
            assert uncertain['transaction_hash'] == live['hash'], uncertain
            assert run('send', 'inspect', signed['id'])['artifact'] == uncertain
            live['wrong'] = False
            accepted = run(*broadcast)['artifact']
            assert accepted['attempts'][-1]['outcome'] == 'Accepted', accepted
            assert accepted['attempts'][-1]['transaction_hash'] == live['hash'], accepted
            assert accepted['transaction_hash'] == live['hash'], accepted
            assert len(submitted) == 2 and submitted[0] == submitted[1], submitted
            assert run('send', 'inspect', signed['id'])['artifact'] == accepted
            print(chain + ': foreign receipt refused; identical saved payload accepted with local hash after restart')
        finally:
            server.shutdown()
            server.server_close()
            worker.join()
