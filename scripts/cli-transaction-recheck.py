#!/usr/bin/env python3
"""Manual recheck through the real CLI: exact target, persisted results, failed reads."""
import http.server
import json
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading

binary = str(pathlib.Path(sys.argv[1]).resolve())
requests = []
response = {'confirmed': True, 'block_height': 321}
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass
    def do_GET(self):
        requests.append(self.path)
        data = json.dumps(response).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

with tempfile.TemporaryDirectory(prefix='spectra-recheck-') as directory:
    def run(*args, success=True):
        result = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True)
        assert (result.returncode == 0) == success, result.stdout + result.stderr
        return json.loads(result.stdout) if success else None
    run('txs')  # create the core schema
    path = pathlib.Path(directory) / 'spectra.sqlite'
    def rows():
        with sqlite3.connect(path) as db:
            return {key: json.loads(payload) for key, payload in db.execute('SELECT id,payload FROM history_records')}
    with sqlite3.connect(path) as db:
        for key, status, tx_hash in [('target', 'failed', 'ab'*32), ('other', 'pending', 'cd'*32)]:
            row = dict(id=key, walletId='wallet', walletName='Fixture', kind='send', status=status,
                       chainName='Bitcoin Testnet4', symbol='BTC', assetName='Bitcoin', amount=1,
                       address='recipient', transactionHash=tx_hash, createdAt=1234, failureReason='old failure')
            db.execute('INSERT INTO history_records (id,wallet_id,chain_name,tx_hash,created_at,payload) VALUES (?,?,?,?,?,?)',
                       (key,'wallet',row['chainName'],tx_hash,978308434,json.dumps(row)))
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    endpoint = f'http://127.0.0.1:{server.server_port}'
    try:
        change = run('txs','--recheck','TARGET','--endpoint',endpoint)['change']
        assert change['oldStatus']=='failed' and change['newStatus']=='confirmed'
        saved = rows()
        assert saved['target']['receiptBlockNumber']==321
        assert saved['target'].get('failureReason') is None
        assert saved['other']['status']=='pending'
        assert requests == ['/tx/'+'ab'*32+'/status']
        response = {'confirmed': False}
        run('txs','--recheck','target','--endpoint',endpoint)
        assert rows()['target']['status']=='pending'
        assert rows()['target'].get('receiptBlockNumber') is None
        before = rows()
        response = {'invalid': True}
        run('txs','--recheck','target','--endpoint',endpoint,success=False)
        assert rows()==before, 'failed provider read changed saved state'
        count=len(requests)
        run('txs','--recheck','missing','--endpoint',endpoint,success=False)
        assert len(requests)==count
    finally:
        server.shutdown()
        server.server_close()
        worker.join()
print('manual recheck targets recorded network, persists results, and preserves failed reads')
