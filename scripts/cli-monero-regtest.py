#!/usr/bin/env python3
"""Real-node integration, entirely offline: --monerod /path/to/verified/monerod.

Separate from make verify because the official daemon is an optional test tool.
Production has no fakechain bypass; only this loopback proxy normalizes nettype.
"""
import argparse
import http.server
import json
import os
import pathlib
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--monerod', required=True)
parser.add_argument('--binary', default='target/debug/spectra')
args = parser.parse_args()
binary = str(pathlib.Path(args.binary).resolve())
daemon = str(pathlib.Path(args.monerod).resolve())
with tempfile.TemporaryDirectory(prefix='spectra-monero-regtest-') as directory:
    with socket.socket() as port:
        port.bind(('127.0.0.1', 0))
        rpc_port = port.getsockname()[1]
    daemon_url = f'http://127.0.0.1:{rpc_port}'
    with open(pathlib.Path(directory)/'daemon.log', 'w') as log:
        process = subprocess.Popen([daemon, '--regtest', '--offline', '--fixed-difficulty', '1',
            '--no-igd', '--no-zmq', '--p2p-bind-port', '0', '--hide-my-port', '--non-interactive', '--disable-dns-checkpoints',
            '--check-updates', 'disabled', '--rpc-bind-ip', '127.0.0.1',
            '--rpc-bind-port', str(rpc_port), '--data-dir', directory+'/node'], stdout=log, stderr=log)
        server = None
        try:
            def rpc(method, params=None):
                request = {'jsonrpc': '2.0', 'id': '0', 'method': method, 'params': params or {}}
                return json.load(urllib.request.urlopen(daemon_url+'/json_rpc', json.dumps(request).encode(), timeout=120))['result']
            for _ in range(100):
                try:
                    info = rpc('get_info')
                    assert info['nettype'] == 'fakechain' and info['offline']
                    break
                except (OSError, KeyError):
                    assert process.poll() is None, pathlib.Path(directory, 'daemon.log').read_text()
                    time.sleep(.1)
            else:
                raise AssertionError('monerod did not start')
            submissions = []
            class Proxy(http.server.BaseHTTPRequestHandler):
                def log_message(self, *_): pass
                def do_POST(self):
                    body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
                    if self.path == '/send_raw_transaction': submissions.append(body)
                    raw = urllib.request.urlopen(daemon_url+self.path, body, timeout=120).read()
                    if self.path == '/get_info':
                        info = json.loads(raw)
                        assert info['nettype'] == 'fakechain' and info['offline']
                        info['nettype'] = 'mainnet'
                        raw = json.dumps(info).encode()
                    self.send_response(200); self.send_header('Content-Length', str(len(raw)))
                    self.end_headers(); self.wfile.write(raw)
            server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
            threading.Thread(target=server.serve_forever, daemon=True).start()
            endpoint = f'http://127.0.0.1:{server.server_port}'
            def run(*words, success=True):
                result = subprocess.run([binary, '--data-dir', directory+'/wallet', '--json', *words],
                    capture_output=True, text=True, timeout=180, env={**os.environ,
                    'SPECTRA_SEED': 'abandon '*11+'about', 'SPECTRA_PASSWORD': 'fixture-password'})
                assert (result.returncode == 0) == success, (words, result.stdout, result.stderr)
                return json.loads(result.stdout)
            run('wallet', 'import', '--chain', 'monero', '--name', 'LocalXmr')
            run('endpoints','--chain','monero','--api','monero-daemon-rpc','--capabilities','fee,broadcast,verification','--add', endpoint)
            address = run('send', 'identity', '--from', 'LocalXmr')['address']
            rpc('generateblocks', {'wallet_address': address, 'amount_of_blocks': 180})
            synced = run('send', 'sync-monero', '--from', 'LocalXmr')['sync']
            assert synced['complete'] and synced['unlocked_piconeros'] > 0
            assert run('send', 'monero-status', '--from', 'LocalXmr')['sync'] == synced
            def build():
                return run('send', 'build', '--from', 'LocalXmr', '--to', address,
                           '--amount', '0.001', '--endpoint', endpoint)['artifact']
            prepared, competing = build(), build()
            signed = run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'], '--endpoint', endpoint)['artifact']
            assert not submissions, 'build/sign must not broadcast'
            run('send', 'sign', competing['id'], '--review-digest', competing['review_digest'], '--endpoint', endpoint, success=False)
            assert run('send', 'inspect', prepared['id'])['artifact'] == signed
            accepted = run('send', 'broadcast-signed', prepared['id'], '--endpoint', endpoint, '--yes')['artifact']
            assert accepted['attempts'][-1]['outcome'] == 'Accepted', accepted
            assert len(submissions) == 1
            pool = json.load(urllib.request.urlopen(daemon_url+'/get_transaction_pool', b'{}', timeout=30))
            assert pool['transactions'][0]['id_hash'] == signed['transaction_hash']
            print('PASS: official monerod accepted local CLSAG/Bulletproof+ signing; no early broadcast, durable restart and input reservation verified')
        finally:
            if server:
                server.shutdown(); server.server_close()
            process.terminate()
            try: process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait()
