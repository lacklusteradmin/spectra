#!/usr/bin/env python3
"""Real-node integration, entirely offline: --monerod /path/to/verified/monerod.

Separate from make verify because the official daemon is an optional test tool.
Production has no fakechain bypass; only this loopback proxy normalizes nettype.

After a send the official daemon accepts, the wallet pays its fresh receive
subaddress and four more in one block — 0/199 and 0/300, 49/199 and 50/0,
each second one inside wallet2's lookahead only once the first is found. The
phrase restored in another directory finds every one and rotates past them;
the view key alone, in a third, finds what was received without seeing what
was spent, refuses to send, and is upgraded in place by the phrase.
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
# The Monero seed of the wallet the recorded fixtures were made with: Monero
# reads its own 25-word seed, not BIP-39.
MONERO_SEED = ('syndrome portents apex vivid flippant dizzy bumper duplex enjoy deodorant bunch pigment wolf muppet tuition wept ailments kiwi roles against today morsel eternal excess wolf')

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
            def run(*words, success=True, data='wallet'):
                result = subprocess.run([binary, '--data-dir', f'{directory}/{data}', '--json', *words],
                    capture_output=True, text=True, timeout=180, env={**os.environ,
                    'SPECTRA_SEED': MONERO_SEED, 'SPECTRA_PASSWORD': 'fixture-password'})
                assert success is None or (result.returncode == 0) == success, (words, result.stdout, result.stderr)
                return json.loads(result.stdout) if result.stdout.strip() else None
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

            vectors = json.loads((pathlib.Path(__file__).resolve().parents[1]
                                  / 'core/tests/fixtures/monero-subaddresses.json').read_text())['networks'][0]
            subaddress = {(v['account'], v['address']): v['encoded'] for v in vectors['subaddresses']}
            assert vectors['primary'] == address
            # More outputs on chain for decoy selection to draw rings from.
            rpc('generateblocks', {'wallet_address': address, 'amount_of_blocks': 200})
            run('send', 'sync-monero', '--from', 'LocalXmr')
            # The primary address has received, so the next payer gets 0/1.
            fresh = run('wallet', 'receive', 'LocalXmr')['address']
            assert fresh == subaddress[(0, 1)], fresh
            targets = [(0, 1), (0, 199), (0, 300), (49, 199), (50, 0)]
            def build_to(destination, amount):
                # Decoy selection is random and a regtest chain is small:
                # a round limit is retried, nothing else.
                for _ in range(5):
                    built = run('send', 'build', '--from', 'LocalXmr', '--to', destination,
                                '--amount', amount, '--endpoint', endpoint, success=None)
                    if 'artifact' in built:
                        return built['artifact']
                    assert 'decoy selection round limit' in built['error'], built
                raise AssertionError('decoy selection kept failing')
            for number, target in enumerate(targets):
                artifact = build_to(subaddress[target], f'0.00{11 + number}')
                run('send', 'sign', artifact['id'], '--review-digest', artifact['review_digest'], '--endpoint', endpoint)
                run('send', 'broadcast-signed', artifact['id'], '--endpoint', endpoint, '--yes')
            pool = json.load(urllib.request.urlopen(daemon_url+'/get_transaction_pool', b'{}', timeout=30))
            assert len(pool['transactions']) == len(targets), pool
            # One block holds all five; ten more unlock them.
            rpc('generateblocks', {'wallet_address': address, 'amount_of_blocks': 11})
            used = [{'account': 0, 'address': 300}, {'account': 49, 'address': 199}, {'account': 50, 'address': 0}]
            original = run('send', 'sync-monero', '--from', 'LocalXmr')['sync']
            assert original['complete'] and original['spends_known'] and original['used_subaddresses'] == used, original
            rotated = run('wallet', 'receive', 'LocalXmr')['address']
            assert rotated not in subaddress.values(), rotated

            # The phrase, restored elsewhere, finds every subaddress again.
            run('wallet', 'import', '--chain', 'monero', '--name', 'RestoredXmr', data='restored')
            run('endpoints','--chain','monero','--api','monero-daemon-rpc','--capabilities','fee,broadcast,verification','--add', endpoint, data='restored')
            restored = run('send', 'sync-monero', '--from', 'RestoredXmr', data='restored')['sync']
            assert restored['used_subaddresses'] == used, restored
            assert restored['unlocked_piconeros'] == original['unlocked_piconeros'], (restored, original)
            assert run('wallet', 'receive', 'RestoredXmr', data='restored')['address'] == rotated
            # A restart scans on from where it stopped.
            assert run('send', 'monero-status', '--from', 'RestoredXmr', data='restored')['sync'] == restored

            # The view key alone: what arrived, never what left.
            view = vectors['private_view_key']
            other = view[:-2] + ('00' if view[-2:] != '00' else '01')
            run('wallet', 'watch', '--chain', 'monero', '--name', 'ViewXmr', '--address', address,
                '--view-key', other, data='viewonly', success=False)
            run('wallet', 'watch', '--chain', 'monero', '--name', 'ViewXmr', '--address', subaddress[(0, 1)],
                '--view-key', view, data='viewonly', success=False)
            run('wallet', 'watch', '--chain', 'monero', '--name', 'ViewXmr', '--address', address,
                '--view-key', view, data='viewonly')
            run('endpoints','--chain','monero','--api','monero-daemon-rpc','--capabilities','fee,broadcast,verification','--add', endpoint, data='viewonly')
            watched = run('send', 'sync-monero', '--from', 'ViewXmr', data='viewonly')['sync']
            assert watched['complete'] and not watched['spends_known'], watched
            assert watched['used_subaddresses'] == used, watched
            assert watched['unlocked_piconeros'] > restored['unlocked_piconeros'], (watched, restored)
            assert run('wallet', 'receive', 'ViewXmr', data='viewonly')['address'] == rotated
            run('send', 'build', '--from', 'ViewXmr', '--to', address, '--amount', '0.001',
                '--endpoint', endpoint, data='viewonly', success=False)
            assert run('send', 'monero-status', '--from', 'ViewXmr', data='viewonly')['sync'] == watched
            rpc('generateblocks', {'wallet_address': address, 'amount_of_blocks': 1})
            later = run('send', 'sync-monero', '--from', 'ViewXmr', data='viewonly')['sync']
            assert later['scanned_height'] == watched['scanned_height'] + 1, (later, watched)
            # The phrase gives the watched wallet its spend key: it scans
            # again and sees what it spent.
            upgraded = run('wallet', 'import', '--chain', 'monero', '--name', 'Upgraded', data='viewonly')
            assert upgraded['wallet']['name'] == 'ViewXmr', upgraded
            synced = run('send', 'sync-monero', '--from', 'ViewXmr', data='viewonly')['sync']
            again = run('send', 'sync-monero', '--from', 'RestoredXmr', data='restored')['sync']
            assert synced['spends_known'] and synced['unlocked_piconeros'] == again['unlocked_piconeros'], (synced, again)
            print('PASS: subaddresses recovered past wallet2 lookahead edges across accounts, receive rotation, '
                  'restart, and a view-only wallet that cannot sign and upgrades in place')
        finally:
            if server:
                server.shutdown(); server.server_close()
            process.terminate()
            try: process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait()
