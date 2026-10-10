#!/usr/bin/env python3
"""ERC-721 and ERC-1155 transfers as staged sends against a loopback EVM
node, across processes: each is built, signed and broadcast, and the node
receives exactly the transaction ethers.js signs for the same fields
(core/tests/fixtures/nft-transfer-vectors.json). Standards, holdings,
refusals, reviews and history are tested in core."""
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
import unittest

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
VECTORS = json.loads((pathlib.Path(__file__).resolve().parents[1]
                      / 'core/tests/fixtures/nft-transfer-vectors.json').read_text())['signed']
OWNER, RECIPIENT = VECTORS['signer'], VECTORS['recipient']
APES, ITEMS = VECTORS['erc721']['to'], VECTORS['erc1155']['to']
SIGNED = {VECTORS['erc721']['raw']: VECTORS['erc721']['hash'], VECTORS['erc1155']['raw']: VECTORS['erc1155']['hash']}


def word(value):
    return '0x' + format(value, '064x')


class Node(http.server.BaseHTTPRequestHandler):
    """An Ethereum node where the wallet owns Apes #1234 and five of item 7."""
    submitted = []

    def log_message(self, *_):
        pass

    def reply(self, payload):
        data = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def call(self, to, data):
        """An `eth_call`'s answer, or None for a revert."""
        selector, argument = data[2:10], data[10:]
        if selector == '01ffc9a7':
            return word(argument[:8] == {APES: '80ac58cd', ITEMS: 'd9b67a26'}.get(to))
        if selector == '6352211e' and to == APES and int(argument[:64], 16) == 1234:
            return word(int(OWNER, 16))
        if selector == '00fdd58e' and to == ITEMS:
            return word(5 if int(argument[64:128], 16) == 7 else 0)
        return None

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))

        def answer(call):
            method, params = call['method'], call.get('params', [])
            if method == 'eth_call':
                result = self.call(params[0]['to'].lower(), params[0]['data'])
                if result is None:
                    return {'jsonrpc': '2.0', 'id': call['id'], 'error': {'code': 3, 'message': 'execution reverted'}}
                return {'jsonrpc': '2.0', 'id': call['id'], 'result': result}
            if method == 'eth_sendRawTransaction':
                raw = params[0]
                # Only the transactions ethers signs for the same fields.
                assert raw in SIGNED, raw
                self.submitted.append(raw)
                return {'jsonrpc': '2.0', 'id': call['id'], 'result': SIGNED[raw]}
            values = {'eth_chainId': '0x1', 'eth_getBalance': hex(10 * 10**18), 'eth_getCode': '0x',
                      'eth_getTransactionCount': '0x3', 'eth_estimateGas': '0x15f90', 'eth_blockNumber': '0x20',
                      'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]}}
            assert method in values, method
            return {'jsonrpc': '2.0', 'id': call['id'], 'result': values[method]}

        self.reply(list(map(answer, body)) if isinstance(body, list) else answer(body))


class NftTests(unittest.TestCase):
    def setUp(self):
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.rpc = f'http://127.0.0.1:{self.server.server_port}/rpc'
        self.directory = tempfile.TemporaryDirectory(prefix='spectra-nfts-')
        Node.submitted.clear()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.directory.cleanup()

    def run_cli(self, *args, env=None):
        p = subprocess.run([binary, '--data-dir', self.directory.name, '--json', *args], capture_output=True,
                           text=True, timeout=60, env={**os.environ, **(env or {})})
        assert p.returncode == 0, (args, p.stdout, p.stderr)
        return json.loads(p.stdout)

    def send(self, contract, token_id, quantity, vector):
        """Builds, signs and broadcasts the transfer: the node receives the
        signed bytes, which are ethers' for the same fields."""
        before = len(Node.submitted)
        built = self.run_cli('wallet', 'send-nft', 'Collector', '--contract', contract, '--token-id', token_id,
                             '--quantity', quantity, '--to', RECIPIENT)['artifact']
        signed = self.run_cli('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                              '--endpoint', self.rpc)['artifact']
        assert signed['signed_payload'] == vector['raw'], signed['signed_payload']
        assert len(Node.submitted) == before, 'signing broadcasts nothing'
        self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.rpc, '--yes')

    def test_erc721_and_erc1155_transfers_are_broadcast_as_signed(self):
        imported = self.run_cli('wallet', 'import', '--chain', 'ethereum', '--name', 'Collector', '--no-password',
                                '--private-key-env', 'KEY', env={'KEY': '01' * 32})
        assert imported['wallet']['address'].lower() == OWNER, imported
        self.run_cli('endpoints', '--chain', 'ethereum', '--api', 'evm-json-rpc', '--capabilities',
                     'balance,fee,broadcast,verification,token-balance', '--add', self.rpc)
        self.run_cli('endpoints', '--chain', 'ethereum', '--custom-only', 'true')
        self.send(APES, '1234', '1', VECTORS['erc721'])
        self.send(ITEMS, '7', '3', VECTORS['erc1155'])
        assert Node.submitted == [VECTORS['erc721']['raw'], VECTORS['erc1155']['raw']], Node.submitted


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
