#!/usr/bin/env python3
"""Offline Monero CLI ownership/network guards. Signature fixture runs in Rust."""
import http.server, json, os, pathlib, subprocess, sys, tempfile, threading
# The Monero seed of the wallet the recorded fixtures were made with: Monero
# reads its own 25-word seed, not BIP-39.
MONERO_SEED = ('syndrome portents apex vivid flippant dizzy bumper duplex enjoy deodorant bunch pigment wolf muppet tuition wept ailments kiwi roles against today morsel eternal excess wolf')
binary = str(pathlib.Path(sys.argv[1]).resolve())
requests = []
class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
        requests.append((self.path, body))
        assert self.path == '/get_info' and body == b'{}'
        raw = b'{"nettype":"stagenet","synchronized":true}'
        self.send_response(200); self.send_header('Content-Length', str(len(raw)))
        self.end_headers(); self.wfile.write(raw)
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
threading.Thread(target=server.serve_forever, daemon=True).start()
try:
    with tempfile.TemporaryDirectory(prefix='spectra-local-xmr-') as directory:
        def run(*args, success=True, password='fixture-password'):
            result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                capture_output=True, text=True, timeout=60, env={**os.environ,
                'SPECTRA_PASSWORD': password, 'SPECTRA_SEED': MONERO_SEED})
            assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
            return json.loads(result.stdout) if result.stdout.strip() else None
        run('wallet', 'import', '--chain', 'monero', '--name', 'LocalXmr', '--restore-height', '3000000')
        assert run('wallet', 'show', 'LocalXmr')['wallet']['restoreHeight'] == 3000000
        # The height is the wallet's from import: no sync takes another.
        run('send', 'sync-monero', '--from', 'LocalXmr', '--restore-height', '1', success=False)
        run('wallet', 'import', '--chain', 'monero', '--name', 'Future', '--restore-height', '99999999999', success=False)
        run('wallet', 'import', '--chain', 'bitcoin', '--name', 'NotMonero', '--restore-height', '1', success=False)
        run('endpoints','--chain','monero','--api','monero-daemon-rpc','--capabilities','fee,broadcast,verification','--add', f'http://127.0.0.1:{server.server_port}')
        initial = run('send', 'monero-status', '--from', 'LocalXmr')['sync']
        # The first scan starts at the stored restore height.
        assert not initial['complete'] and initial['scanned_height'] == 3000000 and not requests
        run('send', 'sync-monero', '--from', 'LocalXmr', password='wrong', success=False)
        assert not requests
        run('send', 'sync-monero', '--from', 'LocalXmr', '--chain', 'zcash', success=False)
        assert not requests
        failed = run('send', 'sync-monero', '--from', 'LocalXmr', success=False)
        assert 'wrong network' in failed['error']
        assert requests == [('/get_info', b'{}')], 'keys must not be sent to the daemon'
        assert run('send', 'monero-status', '--from', 'LocalXmr')['sync'] == initial
        print('PASS Monero: stored restore height, durable status, local authorization and wrong-network refusal without sending keys')

        # A view key watches its own primary address, and nothing else.
        vectors = json.loads((pathlib.Path(__file__).resolve().parents[1]
                              / 'core/tests/fixtures/monero-subaddresses.json').read_text())['networks']
        mainnet, stagenet = vectors
        primary, view = mainnet['primary'], mainnet['private_view_key']
        # The phrase wallet hands out its primary address without a
        # password: its view key was stored at import.
        assert run('wallet', 'receive', 'LocalXmr', password='')['address'] == primary
        for address, key, chain in [(primary, view[:-1] + 'f', 'monero'),
                                    (mainnet['subaddresses'][1]['encoded'], view, 'monero'),
                                    (stagenet['primary'], stagenet['private_view_key'], 'monero'),
                                    (primary, view, 'bitcoin')]:
            run('wallet', 'watch', '--chain', chain, '--name', 'ViewXmr', '--address', address,
                '--view-key', key, success=False)
        # The phrase wallet already holds it.
        run('wallet', 'watch', '--chain', 'monero', '--address', primary, '--view-key', view, success=False)
        run('wallet', 'delete', 'LocalXmr', '--yes')
        watched = run('wallet', 'watch', '--chain', 'monero', '--name', 'ViewXmr', '--address', f' {primary} ',
                      '--view-key', view.upper(), '--restore-height', '3000000')['wallet']
        assert watched['isWatchOnly'] and watched['restoreHeight'] == 3000000, watched
        assert run('wallet', 'receive', 'ViewXmr', password='')['address'] == primary
        status = run('send', 'monero-status', '--from', 'ViewXmr', password='')['sync']
        assert not status['spends_known'] and status['used_subaddresses'] == [], status
        run('send', 'build', '--from', 'ViewXmr', '--to', primary, '--amount', '0.001', password='', success=False)
        # The phrase gives it its spend key, in place.
        upgraded = run('wallet', 'import', '--chain', 'monero', '--name', 'Other', '--restore-height', '3000000')['wallet']
        assert upgraded['id'] == watched['id'] and upgraded['name'] == 'ViewXmr' and not upgraded['isWatchOnly'], upgraded
        assert run('send', 'monero-status', '--from', 'ViewXmr')['sync']['spends_known']
        print('PASS Monero view key: refused unless it opens its primary address, receives and reports without '
              'a password, cannot send, upgraded in place by its phrase')
finally:
    server.shutdown(); server.server_close()
