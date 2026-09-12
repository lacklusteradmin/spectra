#!/usr/bin/env python3
"""Real CLI refresh, loopback provider, persisted native/token balances."""
import http.server
import json
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading

binary = str(pathlib.Path(sys.argv[1]).resolve())
usdc = '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48'
phase = 1
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass
    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        def answer(item):
            method = item['method']
            if method == 'eth_getBalance':
                result = hex(10**18 if phase == 1 else 0)
            elif method == 'eth_call':
                token = item['params'][0]['to'].lower()
                selector = item['params'][0]['data'][:10]
                if selector == '0x313ce567':
                    result = hex(6 if token == usdc else 18)
                elif selector in ('0x95d89b41', '0x06fdde03'):
                    text = 'USDC' if token == usdc else 'TOKEN'
                    result = '0x' + f'{32:064x}{len(text):064x}' + text.encode().hex().ljust(64, '0')
                else:
                    result = hex(2_000_000) if token == usdc else '0x0'
                    if phase == 2 and token == usdc:
                        result = 'not-a-balance'
            else:
                raise AssertionError(method)
            return {'jsonrpc': '2.0', 'id': item.get('id', 1), 'result': result}
        data = json.dumps([answer(item) for item in request] if isinstance(request, list) else answer(request)).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

with tempfile.TemporaryDirectory(prefix='spectra-balance-') as directory:
    def run(*args):
        result = subprocess.run([binary, '--data-dir', directory, '--json', *args], check=True, capture_output=True, text=True)
        return json.loads(result.stdout)
    run('wallet', 'watch', '--chain', 'Ethereum', '--address', '0x1111111111111111111111111111111111111111', '--name', 'Fixture')
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        endpoint = f'http://127.0.0.1:{server.server_port}'
        def balances():
            with sqlite3.connect(str(pathlib.Path(directory) / 'spectra.sqlite')) as db:
                wallet = json.loads(db.execute('SELECT payload FROM wallets').fetchone()[0])
                return {h['symbol']: h['amount'] for h in wallet['holdings']}
        assert run('refresh', '--wallet', 'Fixture', '--endpoint', endpoint)['refreshed'] == 1
        assert balances()['ETH'] == 1 and balances()['USDC'] == 2
        phase = 2
        assert run('refresh', '--wallet', 'Fixture', '--endpoint', endpoint)['refreshed'] == 1
        assert balances()['ETH'] == 0 and balances()['USDC'] == 2, 'failed token read must preserve its last balance'
    finally:
        server.shutdown()
        server.server_close()
        worker.join()
print('native/token refresh commits and preserves unreadable tokens')
