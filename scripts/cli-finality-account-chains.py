#!/usr/bin/env python3
"""Hash-specific XRP, Stellar and Tron finality; loopback only, no broadcast."""
import http.server
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading

BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
SEED = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
GENESIS = {
    'tron': '00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc',
    'tron-nile': '0000000000000000d698d4192c56cb6be724a558448e2684802de4d6cd8690dc',
}
HASHES = {name: f'{index:02x}' * 32 for index, name in enumerate(
    ['success', 'failure', 'pending', 'wrong-hash', 'unknown'], 1)}
NAMES = {value: name for name, value in HASHES.items()}
live = {'chain': '', 'requests': []}


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def answer(self, value, code=200):
        raw = json.dumps(value).encode()
        self.send_response(code)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        live['requests'].append(self.path)
        assert live['chain'].startswith('stellar'), self.path
        if self.path == '/':
            phrase = ('Test SDF Network ; September 2015' if live['chain'].endswith('testnet')
                      else 'Public Global Stellar Network ; September 2015')
            return self.answer({'network_passphrase': phrase})
        assert self.path.startswith('/transactions/'), self.path
        tx_hash = self.path.rsplit('/', 1)[1]
        name = NAMES[tx_hash]
        if name == 'pending':
            return self.answer({'title': 'Resource Missing'}, 404)
        result = {'hash': tx_hash, 'ledger': 450, 'successful': name != 'failure'}
        if name == 'wrong-hash':
            result['hash'] = 'ff' * 32
        if name == 'unknown':
            del result['successful']
        self.answer(result)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        live['requests'].append(body.get('method', self.path))
        if live['chain'].startswith('xrp'):
            if body['method'] == 'server_info':
                return self.answer({'result': {'info': {'network_id': int(live['chain'].endswith('testnet'))}}})
            assert body['method'] == 'tx' and body['params'][0]['binary'] is False, body
            tx_hash = body['params'][0]['transaction']
            name = NAMES[tx_hash]
            if name == 'pending':
                return self.answer({'result': {'status': 'error', 'error': 'txnNotFound'}})
            result = {'hash': tx_hash, 'validated': True, 'ledger_index': 450,
                      'meta': {'TransactionResult': 'tecUNFUNDED_PAYMENT' if name == 'failure' else 'tesSUCCESS'}}
            if name == 'wrong-hash':
                result['hash'] = 'ff' * 32
            if name == 'unknown':
                del result['meta']
            return self.answer({'result': result})
        assert live['chain'].startswith('tron'), self.path
        if self.path == '/wallet/getblockbynum':
            assert body == {'num': 0}, body
            return self.answer({'blockID': GENESIS[live['chain']]})
        assert self.path == '/walletsolidity/gettransactioninfobyid', self.path
        tx_hash = body['value']
        name = NAMES[tx_hash]
        if name == 'pending':
            return self.answer({})
        result = {'id': tx_hash, 'blockNumber': 450, 'receipt': {'net_usage': 280}}
        if name == 'failure':
            result['receipt']['result'] = 'OUT_OF_ENERGY'
            result['result'] = 'FAILED'
        if name == 'wrong-hash':
            result['id'] = 'ff' * 32
        if name == 'unknown':
            result['result'] = 'unrecognized'
        self.answer(result)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()
try:
    for chain, api, symbol in [
        ('xrp', 'xrpl-json-rpc', 'XRP'), ('xrp-testnet', 'xrpl-json-rpc', 'XRP'),
        ('stellar', 'horizon', 'XLM'), ('stellar-testnet', 'horizon', 'XLM'),
        ('tron', 'tron-http', 'TRX'), ('tron-nile', 'tron-http', 'TRX'),
    ]:
        live.update(chain=chain, requests=[])
        with tempfile.TemporaryDirectory(prefix='spectra-account-finality-') as directory:
            def run(*args):
                result = subprocess.run([BINARY, '--data-dir', directory, '--json', *args],
                    input=SEED, capture_output=True, text=True, timeout=60,
                    env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl')})
                assert result.returncode == 0, (args, result.stdout, result.stderr)
                return json.loads(result.stdout)

            wallet = run('wallet', 'import', '--chain', chain, '--name', chain,
                         '--seed-file', '-', '--no-password')['wallet']
            endpoint = f'http://127.0.0.1:{server.server_port}'
            run('endpoints', '--chain', chain, '--api', api, '--capabilities', 'verification', '--add', endpoint)
            with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as db:
                for name, tx_hash in HASHES.items():
                    row = dict(id=name, walletId=wallet['id'], walletName=chain, kind='send', status='pending',
                        chainId=chain, symbol=symbol, assetDisplayName=chain, amount='1', address=wallet['address'],
                        sourceAddress=wallet['address'], transactionHash=tx_hash, createdAtUnix=1_700_000_000)
                    db.execute('INSERT INTO history_records (id,wallet_id,chain_id,tx_hash,created_at,payload) VALUES (?,?,?,?,?,?)',
                               (name, wallet['id'], chain, tx_hash, row['createdAtUnix'], json.dumps(row)))
            changes = run('txs', '--poll-chain', chain)['changes']
            assert {r['id']: r['newStatus'] for r in changes} == {'success': 'confirmed', 'failure': 'failed'}, (chain, changes, live['requests'])
            for name in HASHES:
                stored = run('txs', '--record', name)['record']
                assert stored['status'] == {'success': 'confirmed', 'failure': 'failed'}.get(name, 'pending'), stored
                if name in ('success', 'failure'):
                    assert stored['receiptBlockNumber'] == 450, stored
                if name == 'failure':
                    assert stored['failureReason']['kind'] == 'executionFailed', stored
            assert len(live['requests']) == 10, (chain, live['requests'])
    print('XRP, Stellar and Tron exact finality, failed execution and durable pending acceptance passed')
finally:
    server.shutdown()
    server.server_close()
    worker.join()
