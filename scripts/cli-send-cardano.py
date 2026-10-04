#!/usr/bin/env python3
"""Cardano extended witnesses and pure ADA input selection, on loopback."""
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading

root = pathlib.Path(__file__).resolve().parents[1]
binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else root / 'target/debug/spectra').resolve())
vector = json.loads((root / 'core/tests/fixtures/cardano-emurgo-witness.json').read_text())
live = dict(token_only=False, omit_assets=False, changed=False, reads=0)
asset = dict(policy_id='a'*56, asset_name='01', quantity='1')

class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def reply(self, data):
        raw = json.dumps(data).encode()
        self.send_response(200); self.send_header('Content-Length', str(len(raw)))
        self.end_headers(); self.wfile.write(raw)
    def do_GET(self):
        assert self.path.endswith('/tip'), self.path
        self.reply([dict(abs_slot=0)])
    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        assert self.path.endswith('/address_utxos'), self.path
        assert request['_extended'] is True
        live['reads'] += 1
        entries = [dict(tx_hash='11'*32, tx_index=0, value='2000000', is_spent=False, asset_list=[asset])]
        if not live['token_only']:
            entries.append(dict(tx_hash='00'*32, tx_index=0, value='1170000', is_spent=False,
                asset_list=[asset] if live['changed'] else []))
        if live['omit_assets']:
            for entry in entries: del entry['asset_list']
        self.reply(entries)

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
try:
    with tempfile.TemporaryDirectory(prefix='spectra-cardano-') as directory:
        endpoint = f'http://127.0.0.1:{server.server_port}'
        env = {**os.environ, 'SPECTRA_SEED': vector['mnemonic'], 'CARDANO_KEY': vector['privateKey'],
            'SPECTRA_PASSWORD': 'cardano-test-password', 'SHORT_CARDANO_KEY': vector['privateKey'][:64],
            'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl')}
        def run(*args, success=True):
            result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                env=env, capture_output=True, text=True, timeout=45)
            assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)
        wallet = run('wallet', 'import', '--chain', 'cardano', '--name', 'Cardano', '--no-password')['wallet']
        assert wallet['address'] == vector['address'], wallet
        imported = run('wallet', 'import', '--chain', 'cardano', '--name', 'Extended', '--private-key-env', 'CARDANO_KEY')['wallet']
        assert imported['address'] == vector['address'], imported
        run('wallet', 'import', '--chain', 'cardano', '--name', 'Short key', '--private-key-env', 'SHORT_CARDANO_KEY', success=False)
        assert run('send', 'identity', '--from', 'Extended')['address'] == vector['address']
        run('endpoints', '--chain', 'cardano', '--api', 'koios',
            '--capabilities', 'balance,utxo,verification,broadcast', '--add', endpoint)
        def build(success=True):
            return run('send', 'build', '--from', 'Cardano', '--to', vector['address'],
                '--amount', '1', '--endpoint', endpoint, success=success)
        prepared = build()['artifact']
        details = json.loads(prepared['prepared_details'])['Cardano']
        assert details['inputs'] == [['00'*32, 0, 1170000]], details
        signed = run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'], '--endpoint', endpoint)['artifact']
        assert json.loads(signed['signed_payload'])['cbor_hex'] == vector['cliTransaction'], signed
        pending = build()['artifact']
        live['changed'] = True
        run('send', 'sign', pending['id'], '--review-digest', pending['review_digest'], '--endpoint', endpoint, success=False)
        live['changed'] = False
        live['token_only'] = True; build(success=False)
        live['token_only'] = False; live['omit_assets'] = True; build(success=False)
        assert live['reads'] >= 5
        print('Cardano offline CLI: SDK-matching extended witness, safe pure ADA inputs, token-only/incomplete/changed inputs refused')
finally:
    server.shutdown(); server.server_close(); worker.join()
