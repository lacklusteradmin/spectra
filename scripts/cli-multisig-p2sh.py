#!/usr/bin/env python3
"""A 2-of-3 P2SH multisig on Bitcoin Cash and on Dogecoin, across three data
directories against one loopback Blockbook.

A coordinator watches the account from its `sh(sortedmulti(…))` descriptor
(core/tests/fixtures/p2sh-multisig.json): discovery finds its used
addresses and the balance sums them. It builds a spend, paying the
network's static fee per started kilobyte; cosigner A, whose phrase
upgrades its watched copy in place, imports and signs it; cosigner B signs
A's copy; the coordinator joins both. What travels is the network's own
form: a BCHN PSBT on Bitcoin Cash, Dogecoin Core's partially signed
transaction on Dogecoin. The finished transaction's every scriptSig is
`OP_0`, two signatures and the redeem script, each signature verified here
over the network's digest (BIP-143 with SIGHASH_FORKID on Bitcoin Cash, the
legacy one on Dogecoin) against the script's keys in order. Refused on the
way: a phrase that is no cosigner, an input spent before signing, a
signature twice, a transaction spending another wallet's coins, and the
submission itself: a loopback node cannot prove which of these networks it
serves (Bitcoin Cash shares Bitcoin's genesis).
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
fixture = json.loads((root / 'core/tests/fixtures/p2sh-multisig.json').read_text())
PHRASES = fixture['phrases']
OUTSIDER = 'zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong'
STATIC_FEE = {'bitcoin-cash': 2_000, 'dogecoin': 1_000_000}
funds = {}

P = 2**256 - 2**32 - 977
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
G = (0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798,
     0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8)


def add(a, b):
    if a is None: return b
    if b is None: return a
    if a[0] == b[0] and (a[1] + b[1]) % P == 0: return None
    if a == b:
        slope = 3 * a[0] * a[0] * pow(2 * a[1], -1, P) % P
    else:
        slope = (b[1] - a[1]) * pow(b[0] - a[0], -1, P) % P
    x = (slope * slope - a[0] - b[0]) % P
    return x, (slope * (a[0] - x) - a[1]) % P


def mul(k, point):
    result = None
    while k:
        if k & 1: result = add(result, point)
        point, k = add(point, point), k >> 1
    return result


def decompress(key):
    x = int.from_bytes(key[1:], 'big')
    y = pow((x ** 3 + 7) % P, (P + 1) // 4, P)
    return x, y if y % 2 == key[0] % 2 else P - y


def ecdsa_verify(key, digest, der):
    assert der[0] == 0x30 and der[1] == len(der) - 2 and der[2] == 0x02
    r = int.from_bytes(der[4:4 + der[3]], 'big')
    rest = der[4 + der[3]:]
    assert rest[0] == 0x02
    s = int.from_bytes(rest[2:2 + rest[1]], 'big')
    assert s <= N // 2, 'a high-S signature'
    w = pow(s, -1, N)
    point = add(mul(int.from_bytes(digest, 'big') * w % N, G), mul(r * w % N, decompress(key)))
    return point is not None and point[0] % N == r


def dsha(data): return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def varint(n):
    return bytes([n]) if n < 0xfd else b'\xfd' + n.to_bytes(2, 'little')


def parse_tx(raw):
    data, at = bytes.fromhex(raw), 0

    def take(size):
        nonlocal at
        at += size
        return data[at - size:at]

    def compact():
        first = take(1)[0]
        return first if first < 0xfd else int.from_bytes(take({0xfd: 2, 0xfe: 4}[first]), 'little')
    version = int.from_bytes(take(4), 'little')
    inputs = [(take(36), take(compact()), take(4)) for _ in range(compact())]
    outputs = [(take(8), take(compact())) for _ in range(compact())]
    locktime = take(4)
    assert at == len(data)
    return version, inputs, outputs, locktime


def pushes(script):
    out, at = [], 0
    while at < len(script):
        op = script[at]
        if op == 0:
            out.append(b''); at += 1
        elif op <= 0x4b:
            out.append(script[at + 1:at + 1 + op]); at += 1 + op
        else:
            assert op == 0x4c
            out.append(script[at + 2:at + 2 + script[at + 1]]); at += 2 + script[at + 1]
    return out


def sighash(chain, tx, index, redeem, value):
    version, inputs, outputs, locktime = tx
    serialized_outputs = b''.join(amount + varint(len(script)) + script for amount, script in outputs)
    if chain == 'bitcoin-cash':
        preimage = (version.to_bytes(4, 'little') + dsha(b''.join(i[0] for i in inputs))
                    + dsha(b''.join(i[2] for i in inputs)) + inputs[index][0] + varint(len(redeem)) + redeem
                    + value.to_bytes(8, 'little') + inputs[index][2] + dsha(serialized_outputs) + locktime
                    + (0x41).to_bytes(4, 'little'))
        return dsha(preimage)
    preimage = version.to_bytes(4, 'little') + varint(len(inputs))
    for other, (outpoint, _, sequence) in enumerate(inputs):
        script = redeem if other == index else b''
        preimage += outpoint + varint(len(script)) + script + sequence
    preimage += varint(len(outputs)) + serialized_outputs + locktime + (1).to_bytes(4, 'little')
    return dsha(preimage)


class Blockbook(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = unquote(urlsplit(self.path).path)
        parts = path.strip('/').split('/')
        held = funds.get(parts[3], []) if len(parts) > 3 else []
        if path.startswith('/api/v2/address/'):
            return self.reply({'balance': str(sum(v for _, _, v in held)), 'unconfirmedBalance': '0',
                               'txs': len(held), 'unconfirmedTxs': 0, 'transactions': []})
        if path.startswith('/api/v2/utxo/'):
            return self.reply([{'txid': t, 'vout': n, 'value': str(v), 'confirmations': 6, 'height': 1}
                               for t, n, v in held])
        self.send_error(404, self.path)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Blockbook)
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()


def descriptor(cosigners, threshold=2):
    keys = ','.join(f"[{c['fingerprint']}/{c['origin']}]{c['xpub']}/<0;1>/*" for c in cosigners)
    return f'sh(sortedmulti({threshold},{keys}))'


def exercise(network):
    chain = network['chain']
    coin = 10 ** 8
    address = {(a['branch'], a['index']): a['address'] for a in network['addresses']}
    recipient = network['transaction']['recipient']['address']
    funds.clear()
    funds.update({
        address[(0, 0)]: [('aa' * 32, 0, 1 * coin)],
        address[(0, 2)]: [('bb' * 32, 1, 3 * coin)],
        address[(1, 0)]: [('cc' * 32, 2, coin // 2)],
    })
    values = {(t, n): v for held in funds.values() for t, n, v in held}
    endpoint = f'http://127.0.0.1:{server.server_port}'
    with tempfile.TemporaryDirectory(prefix='spectra-p2sh-multisig-') as directory:
        env = {**os.environ, 'SPECTRA_PASSWORD': 'multisig-password',
               'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl')}

        def run(data, *args, refusal=None, seed=None):
            command_env = dict(env, **({'SPECTRA_SEED': seed} if seed else {}))
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, env=command_env, timeout=120)
            assert (result.returncode == 0) == (refusal is None), (chain, data, args, result.stdout, result.stderr)
            if refusal is not None:
                assert refusal in result.stdout + result.stderr, (refusal, args, result.stdout, result.stderr)
                return None
            return json.loads(result.stdout)

        def signed_by(session):
            return sorted(s['signer'] for s in session['signers'] if s['signed'])

        vault = descriptor(network['cosigners'])
        for data in ['coordinator', 'a', 'b']:
            run(data, 'endpoints', '--chain', chain, '--api', 'blockbook',
                '--capabilities', 'balance,history,utxo,fee,broadcast,verification', '--add', endpoint)
            run(data, 'endpoints', '--chain', chain, '--custom-only', 'true')
            run(data, 'wallet', 'watch', '--chain', chain, '--name', 'Vault', '--multisig', vault)
        found = run('coordinator', 'pool', 'discover', 'Vault')['addresses']
        assert {address[(0, 2)], address[(1, 0)]} <= set(found), found
        assert run('coordinator', 'balance', 'Vault')['smallestUnit'] == str(4 * coin + coin // 2)

        created = run('coordinator', 'multisig', 'create', '--from', 'Vault', '--to', recipient,
                      '--amount', '3.2')['session']
        assert created['scheme'] == 'sortedMulti' and created['fee'] == str(STATIC_FEE[chain]), created
        assert [i['address'] for i in created['inputs']] == [address[(0, 2)], address[(0, 0)]], created
        payment, change = created['outputs']
        assert (payment['address'], payment['value']) == (recipient, str(32 * coin // 10)), created
        assert change['isChange'] and change['address'] == address[(1, 1)], created
        if chain == 'bitcoin-cash':
            assert created['data'].startswith('cHNidP8'), created['data']
        else:
            assert created['data'].startswith('01000000'), created['data']

        run('a', 'wallet', 'import', '--chain', chain, '--name', 'Other', '--upgrade', 'Vault',
            seed=OUTSIDER, refusal='none of the wallet')
        run('a', 'wallet', 'import', '--chain', chain, '--name', 'Other', '--upgrade', 'Vault', seed=PHRASES[0])
        at_a = run('a', 'multisig', 'import', '--wallet', 'Vault', '--data', created['data'])['session']
        assert (at_a['transactionId'], at_a['reviewDigest']) == (created['transactionId'], created['reviewDigest'])
        assert at_a['inputs'] == created['inputs'] and at_a['outputs'] == created['outputs'], at_a
        signed_a = run('a', 'multisig', 'sign', at_a['id'], '--review-digest', at_a['reviewDigest'])['session']
        assert signed_by(signed_a) == [network['cosigners'][0]['fingerprint']], signed_a
        run('a', 'multisig', 'sign', at_a['id'], '--review-digest', at_a['reviewDigest'], refusal='already signed')

        # Another wallet's coins are refused outright.
        other = descriptor(network['cosigners'][:2], threshold=1)
        run('coordinator', 'wallet', 'watch', '--chain', chain, '--name', 'Other', '--multisig', other)
        funds[run('coordinator', 'wallet', 'receive', 'Other')['address']] = [('dd' * 32, 0, coin)]
        foreign = run('coordinator', 'multisig', 'create', '--from', 'Other', '--to', recipient,
                      '--amount', '0.5')['session']
        run('a', 'multisig', 'import', '--wallet', 'Vault', '--data', foreign['data'],
            refusal="not the wallet's script" if chain == 'bitcoin-cash' else 'not one of this wallet')

        run('b', 'wallet', 'import', '--chain', chain, '--name', 'Other', '--upgrade', 'Vault', seed=PHRASES[1])
        at_b = run('b', 'multisig', 'import', '--wallet', 'Vault', '--data', signed_a['data'])['session']
        assert signed_by(at_b) == signed_by(signed_a), at_b
        held = funds.pop(address[(0, 2)])
        run('b', 'multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'], refusal='spent')
        funds[address[(0, 2)]] = held
        signed_b = run('b', 'multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'])['session']
        assert signed_b['complete'], signed_b

        run('coordinator', 'multisig', 'import', '--wallet', 'Vault', '--data', signed_a['data'])
        joined = run('coordinator', 'multisig', 'import', '--wallet', 'Vault', '--data', signed_b['data'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        assert joined['transactionId'] == signed_b['transactionId'], joined
        raw = run('coordinator', 'multisig', 'finalize', created['id'])['raw']
        tx = parse_tx(raw)
        assert hashlib.sha256(hashlib.sha256(bytes.fromhex(raw)).digest()).digest()[::-1].hex() == joined['transactionId']
        for index, (outpoint, script_sig, _) in enumerate(tx[1]):
            dummy, *signatures, redeem = pushes(script_sig)
            assert dummy == b'' and len(signatures) == 2 and redeem[0] == 0x52 and redeem[-1] == 0xae
            keys = [redeem[2 + 34 * i:35 + 34 * i] for i in range(3)]
            value = values[(outpoint[:32][::-1].hex(), int.from_bytes(outpoint[32:], 'little'))]
            digest = sighash(chain, tx, index, redeem, value)
            remaining = list(keys)
            for signature in signatures:
                assert signature[-1] == (0x41 if chain == 'bitcoin-cash' else 0x01)
                while not ecdsa_verify(remaining.pop(0), digest, signature[:-1]):
                    pass
        run('coordinator', 'multisig', 'submit', created['id'], '--yes', refusal='cannot be verified')
        journal = pathlib.Path(env['SPECTRA_LOOPBACK_ONLY'])
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()


try:
    for entry in fixture['networks']:
        if 'transaction' in entry:
            exercise(entry)
    print('PASS p2sh multisig: descriptor watch, discovery and balance, spends at the static fee, cosigner '
          'upgrade and signing in BCHN PSBTs and Dogecoin partial transactions, join and finalize with every '
          'signature verified on Bitcoin Cash and Dogecoin; outsiders, foreign coins, spent inputs, double '
          'signatures and unverifiable submission nodes refused')
finally:
    server.shutdown()
    server.server_close()
    worker.join()
