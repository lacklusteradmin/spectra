#!/usr/bin/env python3
"""Offline CLI pagination check. Only an in-process loopback Esplora fixture is used."""
import json
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ADDRESS = 'bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu'
requests = []

class Esplora(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass
    def do_GET(self):
        requests.append(self.path)
        prefix = f'/address/{ADDRESS}/txs'
        if self.path == prefix:
            upper = 61
        elif self.path.startswith(prefix + '/chain/'):
            upper = int(self.path.rsplit('/', 1)[1], 16) - 1
        else:
            self.send_error(404)
            return
        rows = [{'txid': f'{i:064x}', 'vin': [], 'vout': [{'scriptpubkey_address': ADDRESS, 'value': 100}],
                 'fee': 1, 'status': {'confirmed': True, 'block_height': i, 'block_time': 1700000000 + i}}
                for i in list(range(upper, 0, -1))[:25]]
        data = json.dumps(rows).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

with tempfile.TemporaryDirectory(prefix='spectra-cli-history-') as directory:
    def cli(*args):
        result = subprocess.run([sys.argv[1], '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
        if result.returncode:
            raise AssertionError((args, result.stdout, result.stderr))
        return json.loads(result.stdout)
    cli('wallet', 'watch', '--chain', 'bitcoin', '--address', ADDRESS, '--name', 'Pager')
    server = ThreadingHTTPServer(('127.0.0.1', 0), Esplora)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        result = cli('history', 'Pager', '--save', '--pages', '10', '--limit', '10', '--endpoint', f'http://127.0.0.1:{server.server_port}')
        assert result['added'] == 61 and result['updated'] == 0, result
        assert result['pages'] == 7 and result['exhausted'] and result['walletsFailed'] == 0, result
        assert len(requests) == 3 and sum('/chain/' in p for p in requests) == 2, requests
        # A separate process proves all pages were persisted, not just cached in the session.
        stored = cli('txs', '--wallet', 'Pager')
        assert len(stored['transactions']) == 61, stored
    finally:
        server.shutdown()
        server.server_close()
        worker.join()
print('61 transactions persisted across 7 UI pages and 3 provider pages')
