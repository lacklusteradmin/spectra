#!/usr/bin/env python3
"""Mined OP Stack fees and durable pending outcomes through the CLI; loopback only."""
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
requests = []
errors = []


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        try:
            request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            requests.append(request)
            method = request['method']
            if method == 'eth_getTransactionReceipt':
                case = request['params'][0][2:4]
                result = {'blockNumber': '0x7', 'status': '0x0' if case == 'bb' else '0x1',
                          'gasUsed': '0x5208', 'effectiveGasPrice': '0x2'}
                if case != 'cc':
                    result['l1Fee'] = '0x64'
            elif method == 'eth_call':
                assert request['params'] == [
                    {'to': '0x420000000000000000000000000000000000000F',
                     'data': '0x275aedd2' + format(21000, '064x')}, '0x7'], request
                result = '0x' + format(253, '064x')
            else:
                raise AssertionError(request)
            body = json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}).encode()
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except Exception as error:
            errors.append(str(error))
            self.send_error(500)


server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()
try:
    with tempfile.TemporaryDirectory(prefix='spectra-receipt-fees-') as directory:
        def run(*args):
            result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                    capture_output=True, text=True, timeout=60, env=os.environ)
            assert result.returncode == 0, (args, result.stdout, result.stderr, errors)
            return json.loads(result.stdout)

        wallet = run('wallet', 'watch', '--chain', 'world-chain', '--name', 'World',
                     '--address', '0x' + '12' * 20)['wallet']
        run('endpoints', '--chain', 'world-chain', '--api', 'evm-json-rpc',
            '--capabilities', 'verification', '--add', f'http://127.0.0.1:{server.server_port}')
        with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as database:
            for case in ['aa', 'bb', 'cc']:
                row = dict(id=case, walletId=wallet['id'], walletName='World', kind='send',
                           status='pending', chainId='world-chain', symbol='ETH',
                           assetDisplayName='Ethereum', deploymentId='world-chain:native',
                           amount='1', address='0x' + '34' * 20, transactionHash='0x' + case * 32,
                           createdAtUnix=1700000000, receiptGasUsed='21000',
                           receiptEffectiveGasPriceGwei='0.000000002',
                           receiptNetworkFee='0.000000000000042',
                           confirmedNetworkFee='0.000000000000042')
                database.execute('INSERT INTO history_records '
                                 '(id,wallet_id,chain_id,tx_hash,created_at,payload) VALUES (?,?,?,?,?,?)',
                                 (case, wallet['id'], 'world-chain', row['transactionHash'],
                                  row['createdAtUnix'], json.dumps(row)))
        changes = run('txs', '--poll-chain', 'world-chain')['changes']
        assert len(changes) == 3, (changes, errors)
        for case, status in [('aa', 'confirmed'), ('bb', 'failed'), ('cc', 'confirmed')]:
            # Every query is a new process, proving persistence instead of a cached projection.
            row = run('txs', '--record', case)['record']
            assert row['status'] == status, row
            if case == 'cc':
                assert row.get('receiptNetworkFee') is None, row
                assert row.get('confirmedNetworkFee') is None, row
            else:
                assert row['receiptNetworkFee'] == '0.000000000000042353', row
                assert row['confirmedNetworkFee'] == row['receiptNetworkFee'], row
        assert sum(request['method'] == 'eth_call' for request in requests) == 2, requests
        assert not errors, errors
finally:
    server.shutdown()
    server.server_close()
    worker.join()
print('complete OP Stack actual fees, reverted cost and unknown components passed')
