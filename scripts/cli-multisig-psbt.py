#!/usr/bin/env python3
"""A 2-of-3 Bitcoin multisig and its PSBTs, across three data directories
against one loopback Esplora.

A coordinator watches the account from its descriptor: discovery finds its
used receive and change addresses and the balance sums them; a direct send is
refused. It creates a PSBT; cosigner A, whose phrase upgrades its watched
copy of the account in place, imports and signs it; cosigner B does the same
with A's copy; the coordinator joins both copies and broadcasts. Every input
of the broadcast transaction carries the two signatures in the script's key
order. Refused on the way: another network's descriptor, a bad checksum,
the account watched twice, a phrase that is no cosigner, a signature for
another review digest, a PSBT spending another wallet's coins, an input
spent before signing, and a broadcast short of the threshold.
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
from urllib.parse import unquote, urlsplit

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else
                          pathlib.Path(__file__).resolve().parents[1] / 'target/debug/spectra').resolve())
root = pathlib.Path(__file__).resolve().parents[1]
fixture = json.loads((root / 'core/tests/fixtures/multisig-psbt.json').read_text())
network = fixture['networks'][0]
PHRASES = fixture['phrases']
OUTSIDER = 'zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong'
address = {(a['branch'], a['index']): a['address'] for a in network['addresses']}
RECIPIENT = 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4'
funds = {
    address[(0, 0)]: [('aa' * 32, 0, 100_000)],
    address[(0, 2)]: [('bb' * 32, 1, 300_000)],
    address[(1, 0)]: [('cc' * 32, 2, 50_000)],
}
broadcasts = []


def descriptor(cosigners, threshold=2):
    keys = ','.join(f"[{c['fingerprint']}/{c['origin']}]{c['xpub']}/<0;1>/*" for c in cosigners)
    return f'wsh(sortedmulti({threshold},{keys}))'


def txid_of(raw):
    """A SegWit transaction's id: the hash of it without its witnesses."""
    data = bytes.fromhex(raw)
    position = 0

    def take(size):
        nonlocal position
        chunk = data[position:position + size]
        position += size
        return chunk

    def compact():
        first = take(1)[0]
        return first if first < 0xfd else int.from_bytes(take({0xfd: 2, 0xfe: 4, 0xff: 8}[first]), 'little')

    version = take(4)
    assert take(2) == b'\x00\x01', 'expected a SegWit transaction'
    body_start = position
    for _ in range(compact()):
        take(36)
        take(compact())
        take(4)
    for _ in range(compact()):
        take(8)
        take(compact())
    body = data[body_start:position]
    witnesses = []
    for _ in range(int.from_bytes(body[:1], 'little')):
        witnesses.append([take(compact()) for _ in range(compact())])
    locktime = take(4)
    return hashlib.sha256(hashlib.sha256(version + body + locktime).digest()).digest()[::-1].hex(), witnesses


class Esplora(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, value, text=False):
        body = (value if text else json.dumps(value)).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except BrokenPipeError:
            pass

    def do_GET(self):
        parts = unquote(urlsplit(self.path).path).strip('/').split('/')
        held = funds.get(parts[1], []) if len(parts) > 1 else []
        if parts == ['block-height', '0']:
            return self.reply('000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f', text=True)
        if parts == ['fee-estimates']:
            return self.reply({'1': 3.0, '6': 2.0, '144': 1.0})
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
            return self.reply(txid_of(body)[0], text=True)
        self.send_error(404, self.path)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Esplora)
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()
try:
    endpoint = f'http://127.0.0.1:{server.server_port}'
    with tempfile.TemporaryDirectory(prefix='spectra-multisig-') as directory:
        env = {**os.environ, 'SPECTRA_PASSWORD': 'multisig-password'}
        env.setdefault('SPECTRA_LOOPBACK_ONLY', str(pathlib.Path(directory) / 'network.jsonl'))

        def run(data, *args, success=True, seed=None):
            command_env = dict(env)
            if seed:
                command_env['SPECTRA_SEED'] = seed
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, env=command_env, timeout=120)
            assert (result.returncode == 0) == success, (data, args, result.stdout, result.stderr)
            return json.loads(result.stdout) if result.stdout.strip() else None

        vault = descriptor(network['cosigners'])
        for data in ['coordinator', 'a', 'b']:
            run(data, 'endpoints', '--chain', 'bitcoin', '--api', 'esplora',
                '--capabilities', 'balance,history,utxo,fee,broadcast,verification', '--add', endpoint)
            run(data, 'wallet', 'watch', '--chain', 'bitcoin', '--name', 'Vault', '--descriptor', vault)
        run('coordinator', 'wallet', 'watch', '--chain', 'litecoin', '--descriptor', vault, success=False)
        run('coordinator', 'wallet', 'watch', '--chain', 'bitcoin', '--descriptor', vault + '#qqqqqqqq', success=False)
        run('coordinator', 'wallet', 'watch', '--chain', 'bitcoin', '--name', 'Again', '--descriptor', vault, success=False)

        # The coordinator's account: 0/0 is its own address, so a payer gets
        # 0/1; discovery, the balance over three addresses, no direct send.
        assert run('coordinator', 'wallet', 'receive', 'Vault')['address'] == address[(0, 1)]
        found = run('coordinator', 'pool', 'discover', 'Vault')['addresses']
        assert {address[(0, 2)], address[(1, 0)]} <= set(found), found
        assert run('coordinator', 'balance', 'Vault')['smallestUnit'] == '450000'
        run('coordinator', 'send', 'build', '--from', 'Vault', '--to', RECIPIENT, '--amount', '0.001',
            '--endpoint', endpoint, success=False)

        created = run('coordinator', 'psbt', 'create', '--from', 'Vault', '--to', RECIPIENT,
                      '--amount', '0.0032', '--fee-rate', '2')['session']
        assert [i['address'] for i in created['inputs']] == [address[(0, 2)], address[(0, 0)]], created
        payment, change = created['outputs']
        assert (payment['address'], payment['value_sat'], payment['is_change']) == (RECIPIENT, 320_000, False)
        assert change['is_change'] and change['address'] == address[(1, 1)], created
        assert change['value_sat'] == 80_000 - created['fee_sat'] and not created['complete'], created
        run('coordinator', 'psbt', 'broadcast', created['id'], '--yes', success=False)

        # Cosigner A: its phrase upgrades its watched copy, and only a
        # cosigner's phrase does.
        run('a', 'wallet', 'import', '--chain', 'bitcoin', '--name', 'Other', '--upgrade', 'Vault',
            seed=OUTSIDER, success=False)
        upgraded = run('a', 'wallet', 'import', '--chain', 'bitcoin', '--name', 'Other', '--upgrade', 'Vault',
                       seed=PHRASES[0])
        assert upgraded['upgraded'] and upgraded['wallet']['name'] == 'Vault', upgraded
        at_a = run('a', 'psbt', 'import', '--wallet', 'Vault', '--psbt', created['psbt'])['session']
        assert (at_a['txid'], at_a['review_digest']) == (created['txid'], created['review_digest'])
        run('a', 'psbt', 'sign', at_a['id'], '--review-digest', '00' * 32, success=False)
        signed_a = run('a', 'psbt', 'sign', at_a['id'], '--review-digest', at_a['review_digest'])['session']
        assert signed_a['signed_by'] == [network['cosigners'][0]['fingerprint']], signed_a
        # The coordinator cannot sign: it holds no key.
        run('coordinator', 'psbt', 'sign', created['id'], '--review-digest', created['review_digest'], success=False)

        # A PSBT spending another wallet's coins is refused outright.
        other = descriptor(network['cosigners'][:2], threshold=1)
        run('coordinator', 'wallet', 'watch', '--chain', 'bitcoin', '--name', 'Other', '--descriptor', other)
        funds[json.loads(json.dumps(run('coordinator', 'wallet', 'receive', 'Other')))['address']] = [('dd' * 32, 0, 90_000)]
        foreign = run('coordinator', 'psbt', 'create', '--from', 'Other', '--to', RECIPIENT,
                      '--amount', '0.0005', '--fee-rate', '2')['session']
        run('a', 'psbt', 'import', '--wallet', 'Vault', '--psbt', foreign['psbt'], success=False)

        # Cosigner B, from A's copy; a spent input stops the signature.
        run('b', 'wallet', 'import', '--chain', 'bitcoin', '--name', 'Other', '--upgrade', 'Vault', seed=PHRASES[1])
        at_b = run('b', 'psbt', 'import', '--wallet', 'Vault', '--psbt', signed_a['psbt'])['session']
        held = funds.pop(address[(0, 2)])
        run('b', 'psbt', 'sign', at_b['id'], '--review-digest', at_b['review_digest'], success=False)
        funds[address[(0, 2)]] = held
        signed_b = run('b', 'psbt', 'sign', at_b['id'], '--review-digest', at_b['review_digest'])['session']
        assert signed_b['complete'], signed_b

        # The coordinator joins both copies into its session and broadcasts.
        run('coordinator', 'psbt', 'import', '--wallet', 'Vault', '--psbt', signed_a['psbt'])
        joined = run('coordinator', 'psbt', 'import', '--wallet', 'Vault', '--psbt', signed_b['psbt'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        assert sorted(joined['signed_by']) == sorted(c['fingerprint'] for c in network['cosigners'][:2]), joined
        raw = run('coordinator', 'psbt', 'finalize', created['id'])['raw']
        assert not broadcasts
        sent = run('coordinator', 'psbt', 'broadcast', created['id'], '--yes')['session']
        assert broadcasts == [raw] and sent['broadcast_txid'] == created['txid'], sent
        txid, witnesses = txid_of(raw)
        assert txid == created['txid']
        for witness in witnesses:
            assert len(witness) == 4 and witness[0] == b'' and witness[3][0] == 0x52, witness
        assert run('coordinator', 'psbt', 'list', 'Vault')['sessions'][0]['broadcast_txid'] == txid
        print('PASS multisig: descriptor watch, discovery and balance, PSBT create, cosigner upgrade and signing, '
              'join, finalize and broadcast; foreign coins, spent inputs, stale reviews and short thresholds refused')
        journal = pathlib.Path(env['SPECTRA_LOOPBACK_ONLY'])
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
finally:
    server.shutdown()
    server.server_close()
    worker.join()
