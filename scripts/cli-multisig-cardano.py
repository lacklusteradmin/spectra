#!/usr/bin/env python3
"""A Cardano native-script account (2 of 3 CIP-1854 keys) against a loopback
Koios, across two data directories.

The script of core/tests/fixtures/cardano-multisig.json (CSL's) is watched
at its enterprise address. A transfer is built from the script's output,
its fee sized for every key it may carry; the wallet holding P0's phrase
witnesses it with its CIP-1854 key in one data directory, the wallet
holding P2's phrase in the other from the first's CBOR; the copies join and
the transaction submitted carries the script and both vkey witnesses, each
verified here (Ed25519) over the body's hash. Refused on the way: a phrase
holding none of the script's keys, a wallet holding no phrase, a key twice,
a submission the witnesses do not yet satisfy, an input spent before
signing and a last slot passed.
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
OUTSIDER = 'zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong'
PARAMS = [dict(epoch_no=660, min_fee_a=44, min_fee_b=155381, coins_per_utxo_size='4310', max_tx_size=16384,
               max_val_size=5000)]
state = {}

q = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
d = -121665 * pow(121666, -1, q) % q
I = pow(2, (q - 1) // 4, q)


def recover_x(y, sign):
    x2 = (y * y - 1) * pow(d * y * y + 1, -1, q)
    x = pow(x2, (q + 3) // 8, q)
    if (x * x - x2) % q:
        x = x * I % q
    if x % 2 != sign:
        x = q - x
    return x


def edwards_add(a, b):
    (x1, y1), (x2, y2) = a, b
    x3 = (x1 * y2 + x2 * y1) * pow(1 + d * x1 * x2 * y1 * y2, -1, q)
    y3 = (y1 * y2 + x1 * x2) * pow(1 - d * x1 * x2 * y1 * y2, -1, q)
    return x3 % q, y3 % q


def scalar(e, point):
    result = (0, 1)
    while e:
        if e & 1:
            result = edwards_add(result, point)
        point = edwards_add(point, point)
        e >>= 1
    return result


def decode_point(data):
    y = int.from_bytes(data, 'little') & ((1 << 255) - 1)
    return recover_x(y, data[31] >> 7), y


BASE = (recover_x(4 * pow(5, -1, q) % q, 0), 4 * pow(5, -1, q) % q)


def ed25519_verify(public, message, signature):
    r, s = decode_point(signature[:32]), int.from_bytes(signature[32:], 'little')
    h = int.from_bytes(hashlib.sha512(signature[:32] + public + message).digest(), 'little') % L
    return scalar(s, BASE) == edwards_add(r, scalar(h, decode_point(public)))


def cbor_item(data, at):
    """The item at `at`: its major type, its argument and where it ends."""
    head = data[at]
    major, info = head >> 5, head & 31
    at += 1
    if info < 24:
        argument = info
    else:
        size = {24: 1, 25: 2, 26: 4, 27: 8}[info]
        argument = int.from_bytes(data[at:at + size], 'big')
        at += size
    if major in (2, 3):
        return major, data[at:at + argument], at + argument
    if major == 4:
        items = []
        for _ in range(argument):
            item = cbor_item(data, at)
            items.append(item)
            at = item[2]
        return major, items, at
    if major == 5:
        pairs = []
        for _ in range(argument):
            key = cbor_item(data, at)
            value = cbor_item(data, key[2])
            pairs.append((key, value))
            at = value[2]
        return major, pairs, at
    if major == 6:
        inner = cbor_item(data, at)
        return major, (argument, inner), inner[2]
    return major, argument, at


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
        self.reply([dict(abs_slot=state['slot'])])

    def do_POST(self):
        body = self.rfile.read(int(self.headers['Content-Length']))
        if self.path.endswith('/submittx'):
            state['submitted'].append(body)
            _, items, end = cbor_item(body, 0)
            assert end == len(body)
            body_bytes = body[1:items[0][2]]
            return self.reply(hashlib.blake2b(body_bytes, digest_size=32).hexdigest(), 202)
        request = json.loads(body)
        assert self.path.endswith('/address_utxos') and request['_extended'] is True, self.path
        assert request['_addresses'] == [ADDRESS], request
        self.reply([dict(tx_hash=t, tx_index=i, value=str(v), is_spent=False, asset_list=[])
                    for t, i, v in state['utxos']])


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Koios)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
state.update(slot=1000, utxos=[('aa' * 32, 0, 10_000_000)], submitted=[])
try:
    with tempfile.TemporaryDirectory(prefix='spectra-cardano-multisig-') as directory:
        journal = pathlib.Path(directory) / 'network.jsonl'

        def run(data, *args, env=None, refusal=None):
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120,
                                    env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(journal), **(env or {})})
            assert (result.returncode == 0) == (refusal is None), (data, args, result.stdout, result.stderr)
            if refusal is not None:
                assert refusal in result.stdout + result.stderr, (refusal, args, result.stdout, result.stderr)
                return None
            return json.loads(result.stdout)

        def signed_by(session):
            return [s['signer'] for s in session['signers'] if s['signed']]

        for data in ['a', 'b']:
            run(data, 'endpoints', '--chain', 'cardano', '--api', 'koios', '--capabilities',
                'balance,utxo,fee,verification,broadcast', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'cardano', '--custom-only', 'true')
            watched = run(data, 'wallet', 'watch', '--chain', 'cardano', '--name', 'Vault', '--multisig',
                          json.dumps(SCRIPT['json']))
            assert list(watched['wallets'][0]['addresses'].values()) == [ADDRESS], watched
        for data, name, phrase in [('a', 'Cosigner0', COSIGNERS[0]['phrase']), ('a', 'Outsider', OUTSIDER),
                                   ('b', 'Cosigner2', COSIGNERS[2]['phrase'])]:
            run(data, 'wallet', 'import', '--chain', 'cardano', '--name', name, '--no-password',
                env={'SPECTRA_SEED': phrase})
        run('a', 'wallet', 'import', '--chain', 'cardano', '--name', 'KeyOnly', '--no-password',
            '--private-key-env', 'KEY', env={'KEY': COSIGNERS[0]['private_key']})

        account = run('a', 'multisig', 'account', 'Vault')['account']
        permission = account['permissions'][0]
        assert permission['threshold'] == 2 and [s['signer'] for s in permission['signers']] == [
            c['key_hash'] for c in COSIGNERS], account

        created = run('a', 'multisig', 'create', '--from', 'Vault', '--to', RECIPIENT, '--amount', '2')['session']
        assert created['scheme'] == 'cardanoNativeScript' and created['expiresAtHeight'] == 1000 + 7200, created
        payment, change = created['outputs']
        assert payment['address'] == RECIPIENT and payment['value'] == '2000000', created
        assert change['isChange'] and int(change['value']) + int(created['fee']) == 8_000_000, created
        for name, refusal in [('Outsider', "holds none of the script's keys"), ('KeyOnly', 'holds no phrase')]:
            run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer',
                name, refusal=refusal)
        by_p0 = run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'],
                    '--signer', 'Cosigner0')['session']
        assert signed_by(by_p0) == [COSIGNERS[0]['key_hash']] and not by_p0['complete'], by_p0
        run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer',
            'Cosigner0', refusal='already signed')
        run('a', 'multisig', 'submit', created['id'], '--yes', refusal='do not yet satisfy')

        at_b = run('b', 'multisig', 'import', '--wallet', 'Vault', '--data', by_p0['data'])['session']
        assert at_b['reviewDigest'] == created['reviewDigest'], at_b
        state['utxos'] = []
        run('b', 'multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'], '--signer', 'Cosigner2',
            refusal='spent or changed')
        state['utxos'] = [('aa' * 32, 0, 10_000_000)]
        by_p2 = run('b', 'multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'],
                    '--signer', 'Cosigner2')['session']
        assert by_p2['complete'], by_p2

        joined = run('a', 'multisig', 'import', '--wallet', 'Vault', '--data', by_p2['data'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        state['slot'] = 1000 + 7200
        run('a', 'multisig', 'submit', created['id'], '--yes', refusal='last slot has passed')
        state['slot'] = 1500
        sent = run('a', 'multisig', 'submit', created['id'], '--yes')['session']
        [submitted] = state['submitted']
        _, items, _ = cbor_item(submitted, 0)
        body_hash = hashlib.blake2b(submitted[1:items[0][2]], digest_size=32).digest()
        assert sent['submittedTxid'] == body_hash.hex() == created['transactionId'], sent
        witness_pairs = items[1][1]
        vkeys = next(value for key, value in witness_pairs if key[1] == 0)[1][1][1]
        scripts = next(value for key, value in witness_pairs if key[1] == 1)[1][1][1]
        assert len(scripts) == 1 and submitted[scripts[0][2] - len(bytes.fromhex(SCRIPT['cbor'])):scripts[0][2]] \
            == bytes.fromhex(SCRIPT['cbor'])
        signers = []
        for _, (vkey, signature), _ in vkeys:
            assert ed25519_verify(vkey[1], body_hash, signature[1])
            signers.append(hashlib.blake2b(vkey[1], digest_size=28).hexdigest())
        assert sorted(signers) == sorted([COSIGNERS[0]['key_hash'], COSIGNERS[2]['key_hash']]), signers
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
    print('PASS cardano multisig: a native script watched at its address; witnessed by two CIP-1854 keys in two '
          'data directories, joined and submitted with the script and both witnesses verified; outsiders, '
          'key-only wallets, double witnesses, unsatisfied scripts, spent inputs and passed slots refused')
finally:
    server.shutdown()
    server.server_close()
