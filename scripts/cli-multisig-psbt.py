#!/usr/bin/env python3
"""A 2-of-3 Bitcoin multisig and its PSBT across three data directories,
against one loopback Esplora.

A coordinator watches the account from its descriptor. Each process builds
on what the last one recorded: a payer is handed the next receive address,
discovery finds the used receive and change addresses, the balance sums
them, and a PSBT's change goes to the next change address.
Cosigner A, whose phrase upgrades its watched copy of the account in place,
imports the PSBT and signs; cosigner B signs A's copy; the coordinator joins
both copies and broadcasts once, and the session keeps the transaction id.
Submitting and discarding a session ask for `--yes`.

Core's tests own the rules: coin selection, fees, review digests, spent
inputs and foreign coins (core/src/service/tests/multisig_psbt.rs,
core/src/send/tests/psbt.rs).
"""
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
from urllib.parse import unquote, urlsplit

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else
                          pathlib.Path(__file__).resolve().parents[1] / 'target/debug/spectra').resolve())
root = pathlib.Path(__file__).resolve().parents[1]
fixture = json.loads((root / 'core/tests/fixtures/multisig-psbt.json').read_text())
PHRASES = fixture['phrases']
NETWORK = next(entry for entry in fixture['networks'] if entry['chain'] == 'bitcoin')
GENESIS = '000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f'
USAGE = 2
funds = {}
broadcasts = []


class Esplora(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, value, text=False):
        body = (value if text else json.dumps(value)).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        parts = unquote(urlsplit(self.path).path).strip('/').split('/')
        held = funds.get(parts[1], []) if len(parts) > 1 else []
        if parts == ['block-height', '0']:
            return self.reply(GENESIS, text=True)
        if parts[0] == 'address' and len(parts) == 2:
            stats = {'funded_txo_sum': sum(v for _, _, v in held), 'spent_txo_sum': 0, 'tx_count': len(held)}
            return self.reply({'address': parts[1], 'chain_stats': stats,
                               'mempool_stats': {'funded_txo_sum': 0, 'spent_txo_sum': 0, 'tx_count': 0}})
        if parts[0] == 'address' and parts[2:] == ['utxo']:
            return self.reply([{'txid': t, 'vout': n, 'value': v, 'status': {'confirmed': True, 'block_height': 1}}
                               for t, n, v in held])
        if parts[0] == 'address' and parts[2:3] == ['txs']:
            return self.reply([])
        self.send_error(404, self.path)

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', '0'))).decode()
        if self.path.rstrip('/') == '/tx':
            broadcasts.append(body)
            # Esplora answers with the txid; core checks it only when given.
            return self.reply('', text=True)
        self.send_error(404, self.path)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Esplora)
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()


def descriptor(cosigners):
    keys = ','.join(f"[{c['fingerprint']}/{c['origin']}]{c['xpub']}/<0;1>/*" for c in cosigners)
    return f'wsh(sortedmulti(2,{keys}))'


try:
    address = {(a['branch'], a['index']): a['address'] for a in NETWORK['addresses']}
    recipient = NETWORK['psbt']['recipient']['address']
    funds.update({
        address[(0, 0)]: [('aa' * 32, 0, 100_000)],
        address[(0, 2)]: [('bb' * 32, 1, 300_000)],
        address[(1, 0)]: [('cc' * 32, 2, 50_000)],
    })
    endpoint = f'http://127.0.0.1:{server.server_port}'
    with tempfile.TemporaryDirectory(prefix='spectra-multisig-') as directory:
        env = {**os.environ, 'SPECTRA_PASSWORD': 'multisig-password'}

        def run(data, *args, seed=None, refusal=None):
            """`args` in data directory `data`; `refusal` is the exit code and
            error a refused command must give."""
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120,
                                    env=dict(env, **({'SPECTRA_SEED': seed} if seed else {})))
            output = json.loads(result.stdout) if result.stdout.strip() else None
            if refusal is not None:
                code, message = refusal
                assert result.returncode == code and output['error'] == message, (args, result.stdout, result.stderr)
                return None
            assert result.returncode == 0, (data, args, result.stdout, result.stderr)
            return output

        def signed_by(session):
            return sorted(s['signer'] for s in session['signers'] if s['signed'])

        vault = descriptor(NETWORK['cosigners'])
        for data in ['coordinator', 'a', 'b']:
            run(data, 'endpoints', '--chain', 'bitcoin', '--api', 'esplora',
                '--capabilities', 'balance,history,utxo,fee,broadcast,verification', '--add', endpoint)
            run(data, 'wallet', 'watch', '--chain', 'bitcoin', '--name', 'Vault', '--multisig', vault)

        # The keypool is kept between processes: 0/0 is the account's own
        # address, so a payer gets 0/1; discovery records the used 0/2 and
        # 1/0, the balance sums them, and change goes to 1/1.
        assert run('coordinator', 'wallet', 'receive', 'Vault')['address'] == address[(0, 1)]
        found = run('coordinator', 'pool', 'discover', 'Vault')['addresses']
        assert {address[(0, 2)], address[(1, 0)]} <= set(found), found
        assert run('coordinator', 'balance', 'Vault')['smallestUnit'] == '450000'
        created = run('coordinator', 'multisig', 'create', '--from', 'Vault', '--to', recipient,
                      '--amount', '0.0032', '--fee-rate', '2')['session']
        assert [i['address'] for i in created['inputs']] == [address[(0, 2)], address[(0, 0)]], created
        change = created['outputs'][1]
        assert change['isChange'] and change['address'] == address[(1, 1)], created

        # Submitting and discarding ask for --yes, and without it change
        # nothing.
        run('coordinator', 'multisig', 'submit', created['id'], refusal=(USAGE, 'submit requires --yes'))
        run('coordinator', 'multisig', 'discard', created['id'], refusal=(USAGE, 'discard requires --yes'))
        assert [s['id'] for s in run('coordinator', 'multisig', 'list', 'Vault')['sessions']] == [created['id']]

        # Each cosigner's phrase upgrades its watched copy; A signs the
        # coordinator's PSBT, B signs A's copy.
        copy = created['data']
        for data, cosigner in [('a', 0), ('b', 1)]:
            upgraded = run(data, 'wallet', 'import', '--chain', 'bitcoin', '--name', 'Other', '--upgrade', 'Vault',
                           seed=PHRASES[cosigner])
            assert upgraded['upgraded'] and upgraded['wallet']['name'] == 'Vault', upgraded
            imported = run(data, 'multisig', 'import', '--wallet', 'Vault', '--data', copy)['session']
            assert imported['reviewDigest'] == created['reviewDigest'], imported
            signed = run(data, 'multisig', 'sign', imported['id'], '--review-digest', imported['reviewDigest'])['session']
            assert signed['complete'] == (cosigner == 1), signed
            run('coordinator', 'multisig', 'import', '--wallet', 'Vault', '--data', signed['data'])
            copy = signed['data']

        # The coordinator's session joined both copies; it broadcasts once
        # and keeps the transaction id.
        joined = run('coordinator', 'multisig', 'show', created['id'])['session']
        assert joined['complete'], joined
        assert signed_by(joined) == sorted(c['fingerprint'] for c in NETWORK['cosigners'][:2]), joined
        raw = run('coordinator', 'multisig', 'finalize', created['id'])['raw']
        assert not broadcasts
        sent = run('coordinator', 'multisig', 'submit', created['id'], '--yes')['session']
        assert broadcasts == [raw] and sent['submittedTxid'] == created['transactionId'], sent
        listed = run('coordinator', 'multisig', 'list', 'Vault')['sessions']
        assert [s['submittedTxid'] for s in listed] == [created['transactionId']], listed
    print('PASS multisig: descriptor watch, discovery kept across processes, a PSBT signed by two cosigners in '
          'their own data directories, joined and broadcast once; submit and discard ask for --yes')
finally:
    server.shutdown()
    server.server_close()
    worker.join()
