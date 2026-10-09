#!/usr/bin/env python3
"""A 2-of-3 multisig and its PSBTs, on Bitcoin and on Litecoin, across three
data directories against one loopback Esplora.

A coordinator watches the account from its descriptor: discovery finds its
used receive and change addresses and the balance sums them; a direct send is
refused. It creates a PSBT; cosigner A, whose phrase upgrades its watched
copy of the account in place, imports and signs it; cosigner B does the same
with A's copy; the coordinator joins both copies and broadcasts. Every input
of the broadcast transaction carries the two signatures in the script's key
order. Refused on the way: another network's descriptor, a bad checksum,
the account watched twice, a phrase that is no cosigner, a signature for
another review digest, a PSBT spending another wallet's coins, an input
spent before signing, a broadcast short of the threshold, and on Litecoin a
payment to an MWEB address, which a PSBT cannot make.
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
PHRASES = fixture['phrases']
OUTSIDER = 'zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong'
GENESIS = {
    'bitcoin': '000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f',
    'litecoin': '12a765e31ffd4059bada1e25190f6e98c99d9714d334efa41a195a7e7e04bfe2',
}
funds = {}
broadcasts = []
genesis = []


def bech32(hrp, data):
    """BIP-173 bech32 of 5-bit `data` under `hrp`."""
    charset = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l'

    def polymod(values):
        chk = 1
        for value in values:
            top = chk >> 25
            chk = (chk & 0x1ffffff) << 5 ^ value
            for i, g in enumerate([0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]):
                chk ^= g if (top >> i) & 1 else 0
        return chk
    expanded = [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp]
    check = polymod(expanded + data + [0] * 6) ^ 1
    return hrp + '1' + ''.join(charset[d] for d in data + [(check >> 5 * (5 - i)) & 31 for i in range(6)])


def mweb_address():
    """An MWEB stealth address (version 0, scan and spend keys) on Litecoin."""
    key = bytes.fromhex('0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798')
    bits = ''.join(f'{b:08b}' for b in key + key)
    bits += '0' * (-len(bits) % 5)
    return bech32('ltcmweb', [0] + [int(bits[i:i + 5], 2) for i in range(0, len(bits), 5)])


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
            return self.reply(genesis[0], text=True)
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


def exercise(network):
    chain = network['chain']
    address = {(a['branch'], a['index']): a['address'] for a in network['addresses']}
    recipient = network['psbt']['recipient']['address']
    funds.clear()
    funds.update({
        address[(0, 0)]: [('aa' * 32, 0, 100_000)],
        address[(0, 2)]: [('bb' * 32, 1, 300_000)],
        address[(1, 0)]: [('cc' * 32, 2, 50_000)],
    })
    broadcasts.clear()
    genesis[:] = [GENESIS[chain]]
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
            assert (result.returncode == 0) == success, (chain, data, args, result.stdout, result.stderr)
            return json.loads(result.stdout) if result.stdout.strip() else None

        def signed_by(session):
            return sorted(s['signer'] for s in session['signers'] if s['signed'])

        vault = descriptor(network['cosigners'])
        for data in ['coordinator', 'a', 'b']:
            run(data, 'endpoints', '--chain', chain, '--api', 'esplora',
                '--capabilities', 'balance,history,utxo,fee,broadcast,verification', '--add', endpoint)
            run(data, 'wallet', 'watch', '--chain', chain, '--name', 'Vault', '--multisig', vault)
        run('coordinator', 'wallet', 'watch', '--chain', 'dogecoin', '--multisig', vault, success=False)
        run('coordinator', 'wallet', 'watch', '--chain', f'{chain}-testnet', '--multisig', vault, success=False)
        run('coordinator', 'wallet', 'watch', '--chain', chain, '--multisig', vault + '#qqqqqqqq', success=False)
        run('coordinator', 'wallet', 'watch', '--chain', chain, '--name', 'Again', '--multisig', vault, success=False)

        # The coordinator's account: 0/0 is its own address, so a payer gets
        # 0/1; discovery, the balance over three addresses, no direct send.
        assert run('coordinator', 'wallet', 'receive', 'Vault')['address'] == address[(0, 1)]
        found = run('coordinator', 'pool', 'discover', 'Vault')['addresses']
        assert {address[(0, 2)], address[(1, 0)]} <= set(found), found
        assert run('coordinator', 'balance', 'Vault')['smallestUnit'] == '450000'
        run('coordinator', 'send', 'build', '--from', 'Vault', '--to', recipient, '--amount', '0.001',
            '--endpoint', endpoint, success=False)
        account = run('coordinator', 'multisig', 'account', 'Vault')['account']
        assert account['permissions'][0]['threshold'] == 2, account
        assert len(account['permissions'][0]['signers']) == 3, account
        if chain == 'litecoin':
            run('coordinator', 'multisig', 'create', '--from', 'Vault', '--to', mweb_address(),
                '--amount', '0.0032', '--fee-rate', '2', success=False)

        created = run('coordinator', 'multisig', 'create', '--from', 'Vault', '--to', recipient,
                      '--amount', '0.0032', '--fee-rate', '2')['session']
        assert created['scheme'] == 'sortedMulti', created
        assert [i['address'] for i in created['inputs']] == [address[(0, 2)], address[(0, 0)]], created
        payment, change = created['outputs']
        assert (payment['address'], payment['value'], payment['isChange']) == (recipient, '320000', False)
        assert change['isChange'] and change['address'] == address[(1, 1)], created
        assert int(change['value']) == 80_000 - int(created['fee']) and not created['complete'], created
        run('coordinator', 'multisig', 'submit', created['id'], '--yes', success=False)

        # Cosigner A: its phrase upgrades its watched copy, and only a
        # cosigner's phrase does.
        run('a', 'wallet', 'import', '--chain', chain, '--name', 'Other', '--upgrade', 'Vault',
            seed=OUTSIDER, success=False)
        upgraded = run('a', 'wallet', 'import', '--chain', chain, '--name', 'Other', '--upgrade', 'Vault',
                       seed=PHRASES[0])
        assert upgraded['upgraded'] and upgraded['wallet']['name'] == 'Vault', upgraded
        at_a = run('a', 'multisig', 'import', '--wallet', 'Vault', '--data', created['data'])['session']
        assert (at_a['transactionId'], at_a['reviewDigest']) == (created['transactionId'], created['reviewDigest'])
        run('a', 'multisig', 'sign', at_a['id'], '--review-digest', '00' * 32, success=False)
        signed_a = run('a', 'multisig', 'sign', at_a['id'], '--review-digest', at_a['reviewDigest'])['session']
        assert signed_by(signed_a) == [network['cosigners'][0]['fingerprint']], signed_a
        # The coordinator cannot sign: it holds no key.
        run('coordinator', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'],
            success=False)

        # A PSBT spending another wallet's coins is refused outright.
        other = descriptor(network['cosigners'][:2], threshold=1)
        run('coordinator', 'wallet', 'watch', '--chain', chain, '--name', 'Other', '--multisig', other)
        funds[run('coordinator', 'wallet', 'receive', 'Other')['address']] = [('dd' * 32, 0, 90_000)]
        foreign = run('coordinator', 'multisig', 'create', '--from', 'Other', '--to', recipient,
                      '--amount', '0.0005', '--fee-rate', '2')['session']
        run('a', 'multisig', 'import', '--wallet', 'Vault', '--data', foreign['data'], success=False)

        # Cosigner B, from A's copy; a spent input stops the signature.
        run('b', 'wallet', 'import', '--chain', chain, '--name', 'Other', '--upgrade', 'Vault', seed=PHRASES[1])
        at_b = run('b', 'multisig', 'import', '--wallet', 'Vault', '--data', signed_a['data'])['session']
        held = funds.pop(address[(0, 2)])
        run('b', 'multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'], success=False)
        funds[address[(0, 2)]] = held
        signed_b = run('b', 'multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'])['session']
        assert signed_b['complete'], signed_b

        # The coordinator joins both copies into its session and broadcasts.
        run('coordinator', 'multisig', 'import', '--wallet', 'Vault', '--data', signed_a['data'])
        joined = run('coordinator', 'multisig', 'import', '--wallet', 'Vault', '--data', signed_b['data'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        assert signed_by(joined) == sorted(c['fingerprint'] for c in network['cosigners'][:2]), joined
        raw = run('coordinator', 'multisig', 'finalize', created['id'])['raw']
        assert not broadcasts
        sent = run('coordinator', 'multisig', 'submit', created['id'], '--yes')['session']
        assert broadcasts == [raw] and sent['submittedTxid'] == created['transactionId'], sent
        txid, witnesses = txid_of(raw)
        assert txid == created['transactionId']
        for witness in witnesses:
            assert len(witness) == 4 and witness[0] == b'' and witness[3][0] == 0x52, witness
        assert run('coordinator', 'multisig', 'list', 'Vault')['sessions'][0]['submittedTxid'] == txid
        journal = pathlib.Path(env['SPECTRA_LOOPBACK_ONLY'])
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()


try:
    for entry in fixture['networks']:
        if 'psbt' in entry:
            exercise(entry)
    print('PASS multisig: descriptor watch, discovery and balance, PSBT create, cosigner upgrade and signing, '
          'join, finalize and broadcast on Bitcoin and Litecoin; foreign coins, spent inputs, stale reviews, '
          'short thresholds and MWEB recipients refused')
finally:
    server.shutdown()
    server.server_close()
    worker.join()
