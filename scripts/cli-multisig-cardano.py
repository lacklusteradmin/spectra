#!/usr/bin/env python3
"""A Cardano native-script spend its cosigners witness across two data
directories, against a loopback Koios.

The 2-of-3 script of core/tests/fixtures/cardano-multisig.json (CSL's) is
watched in both directories. A transfer is built from the script's output
in one; the wallet holding cosigner 0's phrase witnesses it there with its
CIP-1854 key, the one holding cosigner 2's witnesses the copy the other
directory imports, the copies join, and the transaction is submitted once,
under the session's transaction id. The rules each step is held to are
core's tests (service::multisig_cardano, send::cardano_multisig).
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
FIXTURE = json.loads((root / 'core/tests/fixtures/cardano-multisig.json').read_text())
SCRIPT = FIXTURE['scripts']['s1']
ADDRESS = SCRIPT['addresses']['cardano']['address']
RECIPIENT = FIXTURE['recipient']['address']
COSIGNERS = FIXTURE['cosigners']
PARAMS = [dict(epoch_no=660, min_fee_a=44, min_fee_b=155381, coins_per_utxo_size='4310', max_tx_size=16384,
               max_val_size=5000)]
submitted = []


def item_end(data, at):
    """Where the CBOR item at `at` ends."""
    major, info = data[at] >> 5, data[at] & 31
    at += 1
    if info < 24:
        argument = info
    else:
        size = {24: 1, 25: 2, 26: 4, 27: 8}[info]
        argument = int.from_bytes(data[at:at + size], 'big')
        at += size
    if major in (2, 3):
        return at + argument
    if major in (4, 5):
        for _ in range(argument * (2 if major == 5 else 1)):
            at = item_end(data, at)
        return at
    if major == 6:
        return item_end(data, at)
    return at


def transaction_id(transaction):
    """The hash of a transaction's body, the first item of its array."""
    return hashlib.blake2b(transaction[1:item_end(transaction, 1)], digest_size=32).hexdigest()


class Koios(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, data, status=200):
        raw = json.dumps(data).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        if self.path.endswith('/epoch_params?order=epoch_no.desc&limit=1'):
            return self.reply(PARAMS)
        if self.path.endswith('/genesis'):
            return self.reply([dict(networkmagic='764824073', networkid='Mainnet')])
        assert self.path.endswith('/tip'), self.path
        self.reply([dict(abs_slot=1000)])

    def do_POST(self):
        body = self.rfile.read(int(self.headers['Content-Length']))
        if self.path.endswith('/submittx'):
            submitted.append(body)
            return self.reply(transaction_id(body), 202)
        request = json.loads(body)
        assert self.path.endswith('/address_utxos') and request['_addresses'] == [ADDRESS], request
        self.reply([dict(tx_hash='aa' * 32, tx_index=0, value='10000000', is_spent=False, asset_list=[])])


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Koios)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
try:
    with tempfile.TemporaryDirectory(prefix='spectra-cardano-multisig-') as directory:
        def run(data, *args, env=None):
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120, env={**os.environ, **(env or {})})
            assert result.returncode == 0, (data, args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        def sign(data, session, signer):
            return run(data, 'multisig', 'sign', session['id'], '--review-digest', session['reviewDigest'],
                       '--signer', signer)['session']

        for data, name, cosigner in [('a', 'Cosigner0', COSIGNERS[0]), ('b', 'Cosigner2', COSIGNERS[2])]:
            run(data, 'endpoints', '--chain', 'cardano', '--api', 'koios', '--capabilities',
                'balance,utxo,fee,verification,broadcast', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'cardano', '--custom-only', 'true')
            run(data, 'wallet', 'watch', '--chain', 'cardano', '--name', 'Vault', '--multisig',
                json.dumps(SCRIPT['json']))
            run(data, 'wallet', 'import', '--chain', 'cardano', '--name', name, '--no-password',
                env={'SPECTRA_SEED': cosigner['phrase']})

        created = run('a', 'multisig', 'create', '--from', 'Vault', '--to', RECIPIENT, '--amount', '2')['session']
        by_c0 = sign('a', created, 'Cosigner0')
        at_b = run('b', 'multisig', 'import', '--wallet', 'Vault', '--data', by_c0['data'])['session']
        assert at_b['reviewDigest'] == created['reviewDigest'], at_b
        by_c2 = sign('b', at_b, 'Cosigner2')
        assert by_c2['complete'], by_c2

        joined = run('a', 'multisig', 'import', '--wallet', 'Vault', '--data', by_c2['data'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        sent = run('a', 'multisig', 'submit', created['id'], '--yes')['session']
        [transaction] = submitted
        assert sent['submittedTxid'] == transaction_id(transaction) == created['transactionId'], sent
    print('PASS cardano multisig: a native-script spend witnessed by two CIP-1854 keys in two data directories, '
          'joined and submitted once under its session\'s transaction id')
finally:
    server.shutdown()
    server.server_close()
