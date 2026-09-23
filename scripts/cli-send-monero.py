#!/usr/bin/env python3
"""Offline Monero CLI ownership/network guards. Signature fixture runs in Rust."""
import http.server, json, os, pathlib, subprocess, sys, tempfile, threading
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
                'SPECTRA_PASSWORD': password, 'SPECTRA_SEED': 'abandon '*11+'about'})
            assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)
        run('wallet', 'import', '--chain', 'monero', '--name', 'LocalXmr')
        run('endpoints','--chain','monero','--api','monero-daemon-rpc','--add', f'http://127.0.0.1:{server.server_port}')
        initial = run('send', 'monero-status', '--from', 'LocalXmr')['sync']
        assert not initial['complete'] and initial['scanned_height'] == 0 and not requests
        run('send', 'sync-monero', '--from', 'LocalXmr', password='wrong', success=False)
        assert not requests
        run('send', 'sync-monero', '--from', 'LocalXmr', '--chain', 'zcash', success=False)
        assert not requests
        failed = run('send', 'sync-monero', '--from', 'LocalXmr', success=False)
        assert 'wrong network' in failed['error']
        assert requests == [('/get_info', b'{}')], 'keys must not be sent to the daemon'
        assert run('send', 'monero-status', '--from', 'LocalXmr')['sync'] == initial
        print('PASS Monero: durable status, local authorization and wrong-network refusal without sending keys')
finally:
    server.shutdown(); server.server_close()
