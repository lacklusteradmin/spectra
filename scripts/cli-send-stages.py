#!/usr/bin/env python3
"""A staged EVM send across processes, against a loopback node.

Build, sign and broadcast each run in their own process on one store: the
signed send is read back as signed, nothing reaches the node until
`broadcast-signed --yes`, and a retry sends the same bytes. Two processes
racing to sign sends at one nonce: one wins, the other is refused. What each
stage checks is core's, tested with `cargo test`.
"""
import concurrent.futures
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
seed = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
password = 'stage-fixture-password'
state = dict(expected='', submitted=[])


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))

        def answer(call):
            method = call['method']
            if method == 'eth_sendRawTransaction':
                state['submitted'].append(call['params'][0])
            values = {'eth_chainId': '0x1', 'eth_getTransactionCount': '0x7', 'eth_getBalance': hex(10**37),
                      'eth_estimateGas': '0x5208', 'eth_getCode': '0x',
                      'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]},
                      'eth_sendRawTransaction': state['expected']}
            assert method in values, method
            return dict(jsonrpc='2.0', id=call['id'], result=values[method])

        body = json.dumps([answer(c) for c in request] if isinstance(request, list) else answer(request)).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


node = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
threading.Thread(target=node.serve_forever, daemon=True).start()
url = f'http://127.0.0.1:{node.server_port}'
try:
    with tempfile.TemporaryDirectory(prefix='spectra-stages-') as directory:
        environment = {**os.environ, 'SPECTRA_PASSWORD': password, 'SPECTRA_SEED': seed}

        def process(*args):
            return subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                  capture_output=True, text=True, env=environment, timeout=45)

        def run(*args):
            result = process(*args)
            assert result.returncode == 0, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)['artifact'] if args[0] == 'send' else json.loads(result.stdout)

        run('wallet', 'import', '--chain', 'ethereum', '--name', 'Stages')
        run('endpoints', '--chain', 'ethereum', '--api', 'evm-json-rpc',
            '--capabilities', 'balance,fee,broadcast,verification,token-balance', '--add', url)

        def build():
            return run('send', 'build', '--from', 'Stages', '--to', '0x' + '22' * 20, '--amount', '0.01',
                       '--endpoint', url)

        def sign(artifact):
            return process('send', 'sign', artifact['id'], '--review-digest', artifact['review_digest'],
                           '--endpoint', url)

        built = build()
        assert built['stage'] == 'Prepared' and not state['submitted'], built
        signed = sign(built)
        assert signed.returncode == 0, (signed.stdout, signed.stderr)
        signed = json.loads(signed.stdout)['artifact']
        assert signed['stage'] == 'Signed' and signed['signed_payload'] and not state['submitted'], signed
        assert run('send', 'inspect', signed['id']) == signed

        state['expected'] = signed['transaction_hash']
        sent = run('send', 'broadcast-signed', signed['id'], '--endpoint', url, '--yes')
        assert [attempt['outcome'] for attempt in sent['attempts']] == ['Accepted'], sent
        retried = run('send', 'broadcast-signed', signed['id'], '--endpoint', url, '--yes')
        assert [attempt['outcome'] for attempt in retried['attempts']] == ['Accepted', 'Accepted'], retried
        assert state['submitted'] == [signed['signed_payload']] * 2, state['submitted']
        assert run('send', 'inspect', signed['id']) == retried

        # Both contenders are built at the nonce after the signed send's.
        contenders = [build(), build()]
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as workers:
            raced = list(workers.map(sign, contenders))
        assert sum(r.returncode == 0 for r in raced) == 1, [(r.stdout, r.stderr) for r in raced]
        assert any('reserved' in r.stdout for r in raced if r.returncode), [(r.stdout, r.stderr) for r in raced]
        assert len(state['submitted']) == 2, state['submitted']
    print('transparent send stage acceptance passed')
finally:
    node.shutdown()
    node.server_close()
