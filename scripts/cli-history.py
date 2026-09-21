#!/usr/bin/env python3
"""History pagination, labels, corrupt storage and transaction status rechecks.

Run: python3 scripts/cli-history.py [path/to/spectra] [TestClass.test_name]
Uses temporary stores and loopback nodes; no public network is required.
"""
import http.server
import json
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else
                          pathlib.Path(__file__).resolve().parents[1] / 'target/debug/spectra').resolve())


class HistoryTests(unittest.TestCase):
    def test_blockbook_history_is_shared_across_networks(self):
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def do_GET(self):
                assert self.path.startswith('/api/v2/address/'), self.path
                body = json.dumps({'transactions': [{'txid': 'ab' * 32, 'blockHeight': 42,
                    'blockTime': 1700000000, 'value': '123456789', 'fees': '1000', 'vin': []}]}).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(body)))
                self.end_headers(); self.wfile.write(body)
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-api-history-') as directory:
                def run(*args):
                    p = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                        input='abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
                        capture_output=True, text=True, timeout=60)
                    assert p.returncode == 0, (p.stdout, p.stderr)
                    return json.loads(p.stdout)
                for chain in ('dash', 'zcash'):
                    run('wallet', 'import', '--chain', chain, '--name', chain, '--seed-file', '-')
                    history = run('history', chain, '--endpoint', f'http://127.0.0.1:{server.server_port}')
                    assert history['count'] == 1, history
                    tx = history['transactions'][0]
                    assert tx['hash'] == 'ab' * 32 and tx['amount'] == 1.23456789, tx
        finally:
            server.shutdown(); server.server_close(); worker.join()

    def test_stored_pages(self):
        """History pages deduplicate, sort, search Unicode and keep distinct identities."""
        with tempfile.TemporaryDirectory(prefix='spectra-history-pages-') as directory:
            def run(*args):
                result = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
                assert result.returncode == 0, (args, result.stdout, result.stderr)
                return json.loads(result.stdout)
            run('wallet', 'watch', '--chain', 'ethereum', '--address', '0x'+'11'*20, '--name', 'Boundary')
            dbpath = pathlib.Path(directory)/'spectra.sqlite'
            with sqlite3.connect(dbpath) as db:
                wid = db.execute('SELECT id FROM wallets').fetchone()[0]
                for i in range(55):
                    row = dict(id=f'tx-{i:03}', walletId=wid, walletName='Éther 测试', kind='receive', status='confirmed', chainName='Ethereum', symbol='ETH', assetDisplayName='Ether', deploymentId='ethereum:native', amount=1, address='0x'+'22'*20, transactionHash=f'0x{i:064x}', createdAt=i)
                    db.execute('INSERT INTO history_records VALUES (?,?,?,?,?,?)', (row['id'],wid,'Ethereum',row['transactionHash'],978307200+i,json.dumps(row)))
                # A duplicate provider record must not consume a page slot or hide the confirmed row.
                row.update(id='duplicate', status='pending')
                db.execute('INSERT INTO history_records VALUES (?,?,?,?,?,?)', (row['id'],wid,'Ethereum',row['transactionHash'],978307999,json.dumps(row)))
            pages = [run('txs','--page','--offset',str(offset))['page'] for offset in (0,20,40)]
            ids = [r['id'] for page in pages for r in page['records']]
            assert len(ids)==55 and len(set(ids))==55 and 'duplicate' not in ids, ids
            assert [p['hasMore'] for p in pages] == [True,True,False], pages
            assert run('txs','--page','--filter','pending')['page']['records']==[]
            assert len(run('txs','--page','--search','éTHER 测试')['page']['records']) == 20
            assert run('txs','--page','--search','no-such-address')['page']['records']==[]
            assert run('txs','--page','--oldest-first','--limit','1')['page']['records'][0]['id']=='tx-000'
            summary = run('txs','--summary')['summary']
            assert summary['totalCount'] == 56
            assert len(summary['recentAndPending']) <= 51
            assert summary['earliest'][0]['earliestCreatedAtUnix'] == 978307200
            assert run('txs','--record','tx-000')['record']['id'] == 'tx-000'
            assert run('txs','--record','missing')['record'] is None
            run('wallet','watch','--chain','solana','--address','11111111111111111111111111111111','--name','IdentityCases')
            with sqlite3.connect(dbpath) as db:
                solana_id = db.execute("SELECT id FROM wallets WHERE name='IdentityCases'").fetchone()[0]
                for identity, txhash, deployment in [('case-upper','A'*88,'solana:native'), ('case-lower','a'*88,'solana:native'), ('unknown-one','B'*88,None), ('unknown-two','B'*88,None)]:
                    record = dict(id=identity, walletId=solana_id, walletName='IdentityCases', kind='receive', status='confirmed', chainName='Solana', symbol='SOL', assetDisplayName='Solana', deploymentId=deployment, amount=1, address='11111111111111111111111111111111', transactionHash=txhash, createdAt=1)
                    db.execute('INSERT INTO history_records VALUES (?,?,?,?,?,?)', (identity,solana_id,'Solana',txhash.lower(),978307201,json.dumps(record)))
            distinct = run('txs','--page','--wallet','IdentityCases')['page']['records']
            assert {row['id'] for row in distinct} == {'case-upper','case-lower','unknown-one','unknown-two'}, distinct

    def test_bitcoin_pagination(self):
        """Fetch and persist every transaction across provider pages."""
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
                result = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
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
                expected_hashes = {f'{i:064x}' for i in range(1, 62)}
                assert len(stored['transactions']) == 61, stored
                assert {row['hash'] for row in stored['transactions']} == expected_hashes, stored
                repeated = cli('history', 'Pager', '--save', '--pages', '10', '--limit', '10', '--endpoint', f'http://127.0.0.1:{server.server_port}')
                assert repeated['added'] == 0 and repeated['walletsFailed'] == 0, repeated
                reopened = cli('txs', '--wallet', 'Pager')['transactions']
                assert len(reopened) == 61 and {row['hash'] for row in reopened} == expected_hashes, reopened
            finally:
                server.shutdown()
                server.server_close()
                worker.join()

    def test_invalid_history_is_refused_on_write(self):
        """Identity/status corruption is rejected before it can poison any page."""
        with tempfile.TemporaryDirectory(prefix="spectra-history-check-") as directory:
            result = subprocess.run([binary, '--data-dir', directory, '--json', 'txs'],
                                    capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as db:
                for raw in ['{"id":42}', '{}', '{"id":"fault","kind":"receive","status":"unknown"}']:
                    with self.assertRaises(sqlite3.IntegrityError):
                        db.execute('INSERT INTO history_records(id,chain_name,created_at,payload) VALUES(?,?,?,?)',
                                   ('fault', 'Bitcoin', 0, raw))
                self.assertEqual(db.execute('SELECT COUNT(*) FROM history_records').fetchone()[0], 0)
            for mode in ('--page', '--summary', '--replaceable'):
                result = subprocess.run([binary, '--data-dir', directory, '--json', 'txs', mode],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_source_labels(self):
        """Expose the correct source label for stored provider identities."""
        with tempfile.TemporaryDirectory(prefix='spectra-history-source-') as directory:
            def run(*args):
                p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
                assert p.returncode == 0, (args, p.stdout, p.stderr)
                return json.loads(p.stdout)
            run('txs')  # create the core schema
            expected = {
                'rpc': {'provider': 'RPC'},
                'etherscan': {'provider': 'Etherscan'},
                'rust': 'internal',
                'rust.hd': 'internal',
                'dogecoin.providers': {'chainProviders': 'Dogecoin'},
                'none': None,
            }
            with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as db:
                for index, source in enumerate(expected):
                    tx_hash = f'{index:02x}' * 32
                    row = dict(id=source, walletId='wallet', walletName='Fixture', kind='receive', status='confirmed',
                               chainName='Dogecoin', symbol='DOGE', assetDisplayName='Dogecoin', amount=1,
                               address='sender', transactionHash=tx_hash, createdAt=1234,
                               transactionHistorySource=source)
                    db.execute('INSERT INTO history_records (id,wallet_id,chain_name,tx_hash,created_at,payload) VALUES (?,?,?,?,?,?)',
                               (source, 'wallet', 'Dogecoin', tx_hash, 978308434 + index, json.dumps(row)))
            by_hash = {t['hash']: t.get('historySource') for t in run('txs')['transactions']}
            for index, (source, named) in enumerate(expected.items()):
                got = by_hash[f'{index:02x}' * 32]
                assert got == named, f'{source}: expected {named!r}, got {got!r}'

    def test_status_recheck(self):
        """Recheck only the target transaction; failed reads preserve stored state."""
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
                result = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
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
                               chainName='Bitcoin Testnet4', symbol='BTC', assetDisplayName='Bitcoin', amount=1,
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


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
