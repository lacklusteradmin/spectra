#!/usr/bin/env python3
"""A TRC-10 staged send against a loopback Tron node, across processes: built,
signed, broadcast once as the TransferAssetContract that was signed, and
recorded as the token's own send until a poll confirms it. Reads, refusals,
history and receipts are tested in core."""
import hashlib
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
FIXTURE = json.loads((pathlib.Path(__file__).resolve().parents[1] / 'core/tests/fixtures/trc10-send-vectors.json').read_text())
OWNER, RECEIVER = FIXTURE['owner'], FIXTURE['receiver']
GENESIS = '00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc'


def base58_hex(address):
    alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
    number = 0
    for char in address:
        number = number * 58 + alphabet.index(char)
    return number.to_bytes(25, 'big')[:21].hex()


live = dict(payloads=[])


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass

    def answer(self, value, code=200):
        raw = json.dumps(value).encode(); self.send_response(code)
        self.send_header('Content-Length', str(len(raw))); self.end_headers(); self.wfile.write(raw)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        route = urllib.parse.urlsplit(self.path).path
        if route == '/wallet/getblockbynum':
            return self.answer({'blockID': GENESIS})
        if route == '/wallet/getnowblock':
            return self.answer({'blockID': FIXTURE['block']['id'],
                                'block_header': {'raw_data': {'number': FIXTURE['block']['number']}}})
        if route == '/wallet/getassetissuebyid':
            return self.answer({'id': '1009999', 'name': 'LegacyTest'.encode().hex(),
                                'abbr': 'T10'.encode().hex(), 'precision': 2})
        if route == '/wallet/getaccount':
            owner = body['address']
            return self.answer({'address': owner, 'balance': 10_000_000 if owner == OWNER else 1,
                                'assetV2': [{'key': '1009999', 'value': 22345 if owner == OWNER else 0}]})
        if route == '/wallet/getchainparameters':
            return self.answer({'chainParameter': [{'key': 'getTransactionFee', 'value': 1000},
                {'key': 'getCreateAccountFee', 'value': 100_000},
                {'key': 'getCreateNewAccountFeeInSystemContract', 'value': 1_000_000}]})
        if route == '/wallet/broadcasttransaction':
            assert body['raw_data']['contract'][0]['type'] == 'TransferAssetContract', body
            assert 'fee_limit' not in body['raw_data'], body
            value = body['raw_data']['contract'][0]['parameter']['value']
            assert value == {'asset_name': '1009999'.encode().hex(), 'owner_address': base58_hex(OWNER),
                             'to_address': base58_hex(RECEIVER), 'amount': 12345}, value
            assert body['txID'] == hashlib.sha256(bytes.fromhex(body['raw_data_hex'])).hexdigest(), body
            live['payloads'].append(body)
            return self.answer({'result': True, 'txid': body['txID']})
        if route == '/walletsolidity/gettransactioninfobyid':
            if not live['payloads']: return self.answer({})
            return self.answer({'id': body['value'], 'blockNumber': 450, 'receipt': {'net_usage': 300}})
        return self.answer({'Error': 'unexpected route'}, 400)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
try:
    with tempfile.TemporaryDirectory(prefix='spectra-trc10-') as directory:
        env = {**os.environ, 'SPECTRA_PRIVATE_KEY': FIXTURE['key'], 'SPECTRA_PASSWORD': 'trc10-vector-password'}

        def run(*args):
            result = subprocess.run([BINARY, '--data-dir', directory, '--json', *args],
                                    capture_output=True, text=True, timeout=60, env=env)
            assert result.returncode == 0, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        endpoint = f'http://127.0.0.1:{server.server_port}'
        run('wallet', 'import', '--chain', 'tron', '--name', 'Legacy', '--private-key-env', 'SPECTRA_PRIVATE_KEY')
        run('endpoints', '--chain', 'tron', '--api', 'tron-http',
            '--capabilities', 'balance,fee,verification,token-balance,broadcast', '--add', endpoint)
        prepared = run('send', 'build', '--from', 'Legacy', '--to', RECEIVER, '--endpoint', endpoint,
                       '--contract', '1009999', '--decimals', '2', '--amount', '123.45')['artifact']
        signed = run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                     '--endpoint', endpoint)['artifact']
        assert not live['payloads'], 'signing broadcasts nothing'
        assert run('send', 'inspect', signed['id'])['artifact']['signed_payload'] == signed['signed_payload']
        accepted = run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')['artifact']
        assert any(a['outcome'] == 'Accepted' for a in accepted['attempts']), accepted
        assert live['payloads'] == [json.loads(signed['signed_payload'])], live['payloads']
        changes = run('txs', '--poll-chain', 'tron')['changes']
        assert changes and all(r['newStatus'] == 'confirmed' for r in changes), changes
        records = run('txs', '--page', '--wallet', 'Legacy')['page']['records']
        confirmed = next(r for r in records if r['transactionHash'] == accepted['transaction_hash'])
        assert (confirmed['status'], confirmed['deploymentId'], confirmed['amount']) == \
            ('confirmed', 'tron:trc-10:1009999', '123.45'), confirmed
        print('TRC-10 staged send: signed, broadcast once as signed, recorded and confirmed')
finally:
    server.shutdown(); server.server_close(); worker.join()
