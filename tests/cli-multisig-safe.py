#!/usr/bin/env python3
"""A Safe on Sepolia against a loopback EVM node, across two owners' data
directories.

The Safe is watched as an address and its policy read from the node (the
official 1.4.1 proxy and singleton, three owners, threshold two). Owner P0
builds a transaction and signs it; owner P1, in its own data directory,
reads P0's copy and signs; P0 joins P1's copy and executes it once, paying
the gas, and the session keeps the executing transaction's hash: a second
submission is refused as already submitted.

Core's tests own the rules: hashes and calldata as protocol-kit makes them,
official proxies, owners, thresholds, nonces, modules and guards
(core/src/send/tests/safe.rs, core/src/service/tests/multisig_safe.rs).
"""
import http.server
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import threading

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
root = pathlib.Path(__file__).resolve().parents[1]
FIXTURE = json.loads((root / 'core/tests/fixtures/safe-multisig.json').read_text())
SAFE = FIXTURE['safe'].lower()
OWNERS = [owner['address'].lower() for owner in FIXTURE['owners']]
NATIVE = FIXTURE['transactions'][0]
# The 1.4.1 factory's proxy, its singleton in slot 0.
PROXY = ('0x608060405273ffffffffffffffffffffffffffffffffffffffff600054167fa619486e000000000000000000'
         '0000000000000000000000000000000000000060003514156050578060005260206000f35b3660008037600080'
         '366000845af43d6000803e60008114156070573d6000fd5b3d6000f3fea264697066735822122003d1488ee65e08'
         'fa41e58e888a9865554c535f2c77126a82cb4c0f917f31441364736f6c63430007060033')
SINGLETON = '41675c099f32341bf84bfc5382af534df5c7461a'
REJECTED = 3
submitted = []


def word(value):
    return format(value, '064x')


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))

        def answer(call):
            method, params = call['method'], call.get('params', [])
            if method == 'eth_call':
                assert params[0]['to'].lower() == SAFE, params
                result = {
                    'a0e67e2b': '0x' + word(32) + word(len(OWNERS)) + ''.join(word(int(o, 16)) for o in OWNERS),
                    'e75235b8': '0x' + word(2),
                    'affed0e0': '0x' + word(NATIVE['nonce']),
                    'ffa1ad74': '0x' + word(32) + word(5) + b'1.4.1'.hex().ljust(64, '0'),
                    'cc2f8452': '0x' + word(64) + word(1) + word(0),
                }[params[0]['data'][2:10]]
            elif method == 'eth_getCode':
                result = PROXY if params[0].lower() == SAFE else '0x'
            elif method == 'eth_getStorageAt':
                singleton = params[0].lower() == SAFE and int(params[1], 16) == 0
                result = '0x' + word(int(SINGLETON, 16) if singleton else 0)
            elif method == 'eth_sendRawTransaction':
                submitted.append(params[0])
                # A node's answer is the hash; core checks it only when given.
                result = ''
            else:
                result = {'eth_chainId': hex(11155111), 'eth_getBalance': hex(10 * 10**18),
                          'eth_getTransactionCount': '0x4', 'eth_estimateGas': '0x186a0', 'eth_blockNumber': '0x20',
                          'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]}}[method]
            return {'jsonrpc': '2.0', 'id': call['id'], 'result': result}

        self.reply(list(map(answer, body)) if isinstance(body, list) else answer(body))


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
try:
    with tempfile.TemporaryDirectory(prefix='spectra-safe-') as directory:

        def run(data, *args, env=None, refusal=None):
            """`args` in data directory `data`; `refusal` is the exit code and
            error a refused command must give."""
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120, env={**os.environ, **(env or {})})
            output = json.loads(result.stdout) if result.stdout.strip() else None
            if refusal is not None:
                code, message = refusal
                assert result.returncode == code and output['error'] == message, (args, result.stdout, result.stderr)
                return None
            assert result.returncode == 0, (data, args, result.stdout, result.stderr)
            return output

        def signed_by(session):
            return sorted(signer['signer'] for signer in session['signers'] if signer['signed'])

        for data, owner in [('p0', 0), ('p1', 1)]:
            run(data, 'endpoints', '--chain', 'ethereum-sepolia', '--api', 'evm-json-rpc', '--capabilities',
                'balance,fee,broadcast,verification', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'ethereum-sepolia', '--custom-only', 'true')
            run(data, 'wallet', 'watch', '--chain', 'ethereum-sepolia', '--name', 'Safe', '--address', SAFE)
            run(data, 'wallet', 'import', '--chain', 'ethereum-sepolia', '--name', 'Owner', '--no-password',
                '--private-key-env', 'KEY', env={'KEY': FIXTURE['owners'][owner]['private_key'][2:]})

        # P0 builds and signs; P1 reads P0's copy and signs.
        created = run('p0', 'multisig', 'create', '--from', 'Safe', '--to', NATIVE['to'], '--amount', '0.01')['session']
        signed = run('p0', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'],
                     '--signer', 'Owner')['session']
        assert signed_by(signed) == [OWNERS[0]] and not signed['complete'], signed
        at_p1 = run('p1', 'multisig', 'import', '--wallet', 'Safe', '--data', signed['data'])['session']
        assert at_p1['reviewDigest'] == created['reviewDigest'] and signed_by(at_p1) == [OWNERS[0]], at_p1
        by_p1 = run('p1', 'multisig', 'sign', at_p1['id'], '--review-digest', at_p1['reviewDigest'],
                    '--signer', 'Owner')['session']
        assert by_p1['complete'], by_p1

        # P0 joins P1's copy and executes once, paying the gas; the session
        # keeps the executing transaction's hash and is not submitted again.
        joined = run('p0', 'multisig', 'import', '--wallet', 'Safe', '--data', by_p1['data'])['session']
        assert joined['id'] == created['id'] and signed_by(joined) == sorted(OWNERS[:2]), joined
        executed = run('p0', 'multisig', 'submit', created['id'], '--executor', 'Owner', '--yes')['session']
        assert len(submitted) == 1, submitted
        assert re.fullmatch('0x[0-9a-f]{64}', executed['submittedTxid'] or ''), executed
        run('p0', 'multisig', 'submit', created['id'], '--executor', 'Owner', '--yes',
            refusal=(REJECTED, 'This session was already submitted.'))
        assert run('p0', 'multisig', 'show', created['id'])['session']['submittedTxid'] == executed['submittedTxid']
        assert len(submitted) == 1, submitted
    print('PASS safe: policy read from the node; two owners sign in their own data directories, the copies join '
          'and an owner executes once; a second submission refused')
finally:
    server.shutdown()
    server.server_close()
