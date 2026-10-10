#!/usr/bin/env python3
"""An XRP Ledger signer-list payment across two data directories, against a
loopback rippled.

The account of core/tests/fixtures/xrp-multisig.json (its signer list P1,
P2 and a third account, quorum 2, its master key disabled) is watched in
both directories. A tagged payment is built in one; P1 signs it there, P2
signs the copy the other directory imports, the copies join, and the blob
is finalized and submitted once, under the session's transaction id. The
rules each step is held to are core's tests (service::multisig_xrp).
"""
import hashlib
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
root = pathlib.Path(__file__).resolve().parents[1]
FIXTURE = json.loads((root / 'core/tests/fixtures/xrp-multisig.json').read_text())
KEYS = FIXTURE['keys']
ACCOUNT = KEYS['p0']['address']
RECIPIENT = FIXTURE['transactions'][0]['tx']['Destination']
LIST = FIXTURE['rpc']['account_info_v1']['response']['result']['account_data']['signer_lists'][0]
LEDGER = 94_999_990
submitted = []


def txid(blob):
    return hashlib.sha512(b'TXN\0' + blob).digest()[:32].hex().upper()


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method, params = request['method'], (request.get('params') or [{}])[0]
        if method == 'server_info':
            result = {'info': {'network_id': 0}}
        elif method == 'fee':
            result = {'drops': {'open_ledger_fee': '10'}}
        elif method == 'server_state':
            result = {'state': {'validated_ledger': {'seq': LEDGER - 1, 'reserve_base': 1_000_000,
                                                     'reserve_inc': 200_000}}}
        elif method == 'account_info' and params['account'] == ACCOUNT:
            data = {'Account': ACCOUNT, 'Balance': '50000000', 'Flags': 0x00100000, 'OwnerCount': 1, 'Sequence': 9}
            if params.get('signer_lists'):
                data['signer_lists'] = [LIST]
            result = {'account_data': data, 'ledger_current_index': LEDGER}
        elif method == 'account_info':
            result = {'account_data': {'Account': params['account'], 'Balance': '30000000', 'Flags': 0,
                                       'OwnerCount': 0, 'Sequence': 3}, 'ledger_current_index': LEDGER}
        elif method == 'submit':
            blob = bytes.fromhex(params['tx_blob'])
            submitted.append(blob)
            result = {'engine_result': 'tesSUCCESS', 'engine_result_message': 'ok', 'accepted': True,
                      'tx_json': {'hash': txid(blob)}}
        else:
            raise AssertionError(request)
        body = json.dumps({'result': result}).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
try:
    with tempfile.TemporaryDirectory(prefix='spectra-xrp-multisig-') as directory:
        def run(data, *args, env=None):
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120, env={**os.environ, **(env or {})})
            assert result.returncode == 0, (data, args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        def sign(data, session, signer):
            return run(data, 'multisig', 'sign', session['id'], '--review-digest', session['reviewDigest'],
                       '--signer', signer)['session']

        for data, signer, key in [('a', 'Signer1', 'p1'), ('b', 'Signer2', 'p2')]:
            run(data, 'endpoints', '--chain', 'xrp', '--api', 'xrpl-json-rpc', '--capabilities',
                'balance,fee,verification,broadcast', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'xrp', '--custom-only', 'true')
            run(data, 'wallet', 'watch', '--chain', 'xrp', '--name', 'Account', '--address', ACCOUNT)
            run(data, 'wallet', 'import', '--chain', 'xrp', '--name', signer, '--no-password',
                env={'SPECTRA_SEED': KEYS[key]['phrase']})

        created = run('a', 'multisig', 'create', '--from', 'Account', '--to', RECIPIENT, '--amount', '1',
                      '--destination-tag', '7')['session']
        by_p1 = sign('a', created, 'Signer1')
        at_b = run('b', 'multisig', 'import', '--wallet', 'Account', '--data', by_p1['data'])['session']
        assert at_b['reviewDigest'] == created['reviewDigest'], at_b
        by_p2 = sign('b', at_b, 'Signer2')
        assert by_p2['complete'], by_p2

        joined = run('a', 'multisig', 'import', '--wallet', 'Account', '--data', by_p2['data'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        assert run('a', 'multisig', 'finalize', created['id'])['raw'] == joined['data']
        sent = run('a', 'multisig', 'submit', created['id'], '--yes')['session']
        [blob] = submitted
        assert sent['submittedTxid'] == txid(blob) == joined['transactionId'], sent
    print('PASS xrp multisig: a tagged payment signed in two data directories, joined, finalized and submitted '
          'once under its session\'s transaction id')
finally:
    server.shutdown()
    server.server_close()
