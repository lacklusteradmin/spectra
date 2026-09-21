#!/usr/bin/env python3
"""Stored Tor policy must route a fresh CLI process through its selected proxy."""
import json
import os
import pathlib
import socketserver
import struct
import subprocess
import sys
import tempfile
import threading
import unittest

binary = str(pathlib.Path(sys.argv[1]).resolve())


class TransportTests(unittest.TestCase):
    def test_custom_proxy_resolves_remotely_and_reopening_uses_saved_policy(self):
        destinations = []
        methods = []
        balance = '0x0'

        class Proxy(socketserver.StreamRequestHandler):
            def handle(self):
                version, count = self.rfile.read(2)
                assert version == 5
                self.rfile.read(count)
                self.wfile.write(b'\x05\x00'); self.wfile.flush()
                header = self.rfile.read(4)
                assert header == b'\x05\x01\x00\x03', header  # SOCKS resolves the hostname.
                size = self.rfile.read(1)[0]
                host = self.rfile.read(size).decode()
                port = struct.unpack('!H', self.rfile.read(2))[0]
                destinations.append((host, port))
                self.wfile.write(b'\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x00'); self.wfile.flush()
                self.rfile.readline()  # HTTP request line
                headers = {}
                while True:
                    line = self.rfile.readline()
                    if line == b'\r\n': break
                    key, value = line.decode().split(':', 1)
                    headers[key.lower()] = value.strip()
                request = json.loads(self.rfile.read(int(headers['content-length'])))
                def answer(call):
                    methods.append(call['method'])
                    values = {'eth_getBalance': balance, 'eth_getTransactionCount': '0x1', 'eth_chainId': '0x1'}
                    return {'jsonrpc': '2.0', 'id': call['id'], 'result': values[call['method']]}
                response = json.dumps([answer(c) for c in request] if isinstance(request, list) else answer(request)).encode()
                self.wfile.write(b'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: ' + str(len(response)).encode() + b'\r\n\r\n' + response)
                self.wfile.flush()

        with tempfile.TemporaryDirectory(prefix='spectra-transport-') as directory:
            def run(*args, success=True):
                result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                        capture_output=True, text=True, timeout=30,
                                        env={**os.environ, 'NO_PROXY': '', 'no_proxy': ''})
                self.assertEqual(result.returncode == 0, success, (args, result.stdout, result.stderr))
                return json.loads(result.stdout)
            address = '0x' + '11' * 20
            run('wallet', 'watch', '--chain', 'Ethereum', '--address', address, '--name', 'Proxy')
            run('settings', 'set', 'rpc-endpoint.Ethereum', 'http://spectra.invalid:8545')
            with socketserver.ThreadingTCPServer(('127.0.0.1', 0), Proxy) as proxy:
                worker = threading.Thread(target=proxy.serve_forever, daemon=True); worker.start()
                try:
                    run('settings', 'set', 'tor-custom-proxy', 'true')
                    run('settings', 'set', 'tor-proxy-address', f'socks5://127.0.0.1:{proxy.server_address[1]}')
                    run('settings', 'set', 'tor-kill-switch', 'true')
                    run('settings', 'set', 'tor-enabled', 'true')
                    self.assertEqual(run('tor')['status'], 'Ready')
                    self.assertEqual(run('tor', '--reconnect')['status'], 'Ready')
                    probe = ('send', 'probe', '--wallet', 'Proxy', '--asset', 'ETH', '--to', address)
                    self.assertEqual(run(*probe)['activity'], 'emptyPreviouslyUsed')
                    balance = '0x1'
                    self.assertEqual(run(*probe)['activity'], 'funded')
                    self.assertIn('eth_getBalance', methods)
                    self.assertTrue(destinations)
                    self.assertEqual(set(destinations), {('spectra.invalid', 8545)})
                    run('settings', 'set', 'tor-enabled', 'false')
                    self.assertEqual(run('tor')['status'], 'Stopped')
                finally:
                    proxy.shutdown(); worker.join()


if __name__ == '__main__':
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]])
