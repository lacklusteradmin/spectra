#!/usr/bin/env python3
"""A NEAR staged send's durable retries against a loopback node, across
processes: a gas price past the reviewed budget submits nothing; a reply
naming another transaction leaves it uncertain, and the retry submits the
same bytes; once the network reports it final, that outcome is recorded
before any stale nonce, expiry or fee is considered. Quotes, NEP-145
registration and the build and signing refusals are tested in core."""
import http.server
import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ROOT/'target/debug/spectra').resolve())
spec = importlib.util.spec_from_file_location('staking_fixture', ROOT/'scripts/cli-staking.py')
staking = importlib.util.module_from_spec(spec)
spec.loader.exec_module(staking)
UNIT = 10**24


class Node(staking.Node):
    """The staking suite's NEAR node, on mainnet, with the sender's account as
    a send reads it."""
    def near(self, method, params):
        s = self.server.state
        if method == 'status':
            return {'chain_id': 'mainnet'}
        if method == 'query' and params['request_type'] == 'view_account':
            return {'amount': str(s['balance']), 'locked': '0', 'storage_usage': 1000}
        return super().near(method, params)


with tempfile.TemporaryDirectory(prefix='spectra-near-send-') as directory:
    s = dict(chain='near', owner='', nonce=7, balance=100*UNIT, gas_price=100000000, authority=True, foreign=True,
             final=False, succeeded=True, submitted=[], reads=[], errors=[], hash=None)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node); server.state = s
    worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
    try:
        def run(*args, success=True):
            result = subprocess.run([BINARY, '--data-dir', directory, '--json', *args], capture_output=True, text=True,
                                    timeout=60, env={**os.environ, 'SPECTRA_SEED': staking.MNEMONIC})
            assert (result.returncode == 0) == success, (args, result.stdout, result.stderr, s['errors'])
            assert not s['errors'], s['errors']
            return json.loads(result.stdout)

        endpoint = f'http://127.0.0.1:{server.server_port}'
        s['owner'] = run('wallet', 'import', '--chain', 'near', '--name', 'Near', '--no-password')['wallet']['address']
        run('endpoints', '--chain', 'near', '--api', 'near-json-rpc',
            '--capabilities', 'balance,fee,verification,token-balance,broadcast', '--add', endpoint)
        prepared = run('send', 'build', '--from', 'Near', '--to', '22'*32, '--amount', '1', '--endpoint', endpoint)['artifact']
        signed = run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                     '--endpoint', endpoint)['artifact']
        s['hash'] = signed['transaction_hash']
        assert signed == run('send', 'inspect', signed['id'])['artifact']
        broadcast = ('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')
        # The signed bytes cap no gas price: past the reviewed budget, nothing is submitted.
        s['gas_price'] *= 2; run(*broadcast, success=False); s['gas_price'] //= 2
        assert not s['submitted']
        uncertain = run(*broadcast)['artifact']
        assert uncertain['attempts'][-1]['outcome'] == 'Uncertain', uncertain
        s['gas_price'] *= 2; run(*broadcast, success=False); s['gas_price'] //= 2
        assert len(s['submitted']) == 1
        s['foreign'] = False
        accepted = run(*broadcast)['artifact']
        assert accepted['attempts'][-1]['outcome'] == 'Accepted', accepted
        assert s['submitted'][0] == s['submitted'][1]
        # The final status is recovered before stale nonce, expiry and fee checks.
        s['final'] = True; s['expired'] = True; s['nonce'] += 1; s['gas_price'] *= 2; s['balance'] = 0
        refusal = run(*broadcast, success=False)
        assert 'already confirmed' in refusal['error'], refusal
        assert len(s['submitted']) == 2
        record = run('txs', '--record', signed['id'])['record']
        assert record['status'] == 'confirmed' and record['transactionHash'] == s['hash'], record
        print('near native: durable retries and final status recovery passed', flush=True)
    finally:
        server.shutdown(); server.server_close(); worker.join()
