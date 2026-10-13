#!/usr/bin/env python3
"""Cardano staged send against a loopback Koios, across processes: an ADA
payment is built, signed without being submitted, and broadcast once,
exactly as signed. Inputs, witnesses, tokens, expiry and the network check
are tested in core."""
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
submitted = []
PARAMS = [dict(epoch_no=660, min_fee_a=44, min_fee_b=155381, coins_per_utxo_size='4310', max_tx_size=16384,
               max_val_size=5000)]


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass

    def reply(self, data, status=200):
        raw = json.dumps(data).encode()
        self.send_response(status); self.send_header('Content-Length', str(len(raw)))
        self.end_headers(); self.wfile.write(raw)

    def do_GET(self):
        if self.path.endswith('/epoch_params?order=epoch_no.desc&limit=1'):
            return self.reply(PARAMS)
        if self.path.endswith('/genesis'):
            return self.reply([dict(networkmagic='764824073', networkid='Mainnet')])
        assert self.path.endswith('/tip'), self.path
        self.reply([dict(abs_slot=0)])

    def do_POST(self):
        if self.path.endswith('/submittx'):
            submitted.append(self.rfile.read(int(self.headers['Content-Length'])).hex())
            return self.reply('ab' * 32, 202)
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        assert self.path.endswith('/address_utxos') and request['_extended'] is True, self.path
        self.reply([dict(tx_hash='00'*32, tx_index=0, value='1170000', is_spent=False, asset_list=[])])


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
try:
    with tempfile.TemporaryDirectory(prefix='spectra-cardano-') as directory:
        endpoint = f'http://127.0.0.1:{server.server_port}'
        env = {**os.environ, 'SPECTRA_SEED': vector['mnemonic'], 'SPECTRA_PASSWORD': 'cardano-test-password'}

        def run(*args):
            result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                    env=env, capture_output=True, text=True, timeout=45)
            assert result.returncode == 0, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        run('wallet', 'import', '--chain', 'cardano', '--name', 'Cardano', '--no-password')
        run('endpoints', '--chain', 'cardano', '--api', 'koios',
            '--capabilities', 'balance,utxo,fee,verification,broadcast', '--add', endpoint)
        run('endpoints', '--chain', 'cardano', '--custom-only', 'true')
        prepared = run('send', 'build', '--from', 'Cardano', '--to', vector['address'], '--amount', '1',
                       '--endpoint', endpoint)['artifact']
        signed = run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                     '--endpoint', endpoint)['artifact']
        assert signed['transaction_hash'] and not submitted, signed
        assert run('send', 'inspect', signed['id'])['artifact'] == signed
        run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')
        assert submitted == [json.loads(signed['signed_payload'])['cbor_hex']], submitted
        print('Cardano staged send: built, signed without submitting, broadcast once as signed')
finally:
    server.shutdown(); server.server_close(); worker.join()
