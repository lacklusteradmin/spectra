#!/usr/bin/env python3
"""A 2-of-3 pallet-multisig transfer on Asset Hub approved across two data
directories, against a loopback node that keeps the pallet's state.

The account is watched from its policy in both directories. A transfer is
built in one; its first signatory approves it by hash from its own account;
the call goes to the other directory, which reads that approval's timepoint
from the node, and its signatory's approval carries the call, within the
weight the node reports, and the node executes it. The session there is
then submitted, and a second approval refused. The rules each step is held
to are core's tests (service::multisig_substrate, send::substrate_multisig,
the Substrate API's).
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

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / 'target/debug/spectra').resolve())
SCALE = 10 ** 10
GENESIS = '0x68d56f15f85d3136970ec16946040bc1752654e906147f7e43e9d539d7c3de2f'
METADATA = '0x' + (ROOT / 'core/tests/fixtures/asset-hub-polkadot-metadata.scale').read_bytes().hex()
VECTORS = json.loads((ROOT / 'core/tests/fixtures/substrate-multisig.json').read_text())['runtimes'][
    'polkadot-asset-hub']
BALANCES, MULTISIG = VECTORS['balances']['pallet_index'], VECTORS['multisig']['pallet_index']
DEPOSIT = int(VECTORS['multisig']['deposit_base']) + 2 * int(VECTORS['multisig']['deposit_factor'])
ACCOUNT_PREFIX = bytes.fromhex('26aa394eea5630e07c48ae0c9558cef7b99d880ec681799c0cf30e8886371da9')
MULTISIGS_PREFIX = bytes.fromhex(VECTORS['storage']['prefix'][2:])
WEIGHT = (1_234_567, 8_910)
PHRASES = ['abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
           'legal winner thank year wave sausage worth useful legal winner thank yellow',
           'letter advice cage absurd amount doctor acoustic avoid letter advice cage above']
RECIPIENT = bytes([0xdd]) * 32
ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


def account_of(address):
    number = 0
    for char in address:
        number = number * 58 + ALPHABET.index(char)
    raw = number.to_bytes((number.bit_length() + 7) // 8, 'big')
    return (bytes(len(address) - len(address.lstrip('1'))) + raw)[1:33]


def ss58(account):
    payload = bytes([0]) + account
    raw = payload + hashlib.blake2b(b'SS58PRE' + payload).digest()[:2]
    number, out = int.from_bytes(raw, 'big'), ''
    while number:
        number, rest = divmod(number, 58)
        out = ALPHABET[rest] + out
    return '1' * (len(raw) - len(raw.lstrip(b'\0'))) + out


def compact(value):
    if value < 64: return bytes([value << 2])
    if value < 16384: return ((value << 2) | 1).to_bytes(2, 'little')
    return ((value << 2) | 2).to_bytes(4, 'little')


def read_compact(raw):
    mode = raw[0] & 3
    count = [1, 2, 4, (raw[0] >> 2) + 5][mode]
    if mode == 3: return int.from_bytes(raw[1:count], 'little'), raw[count:]
    return int.from_bytes(raw[:count], 'little') >> 2, raw[count:]


def block_hash(number): return '0x' + number.to_bytes(32, 'big').hex()


live = dict(pending={}, height=200, submitted=[], executed=[])


def apply(sender, call):
    """pallet-multisig's dispatch of an approval, as the mock keeps it."""
    assert call[0] == MULTISIG, call
    index = call[1]
    count, rest = read_compact(call[4:])
    rest = rest[count * 32:]
    timepoint = None
    if rest[0] == 1:
        timepoint, rest = (int.from_bytes(rest[1:5], 'little'), int.from_bytes(rest[5:9], 'little')), rest[9:]
    else:
        rest = rest[1:]
    if index == VECTORS['multisig']['calls']['approve_as_multi']:
        call_hash, rest, inner = rest[:32], rest[32:], None
    else:
        assert index == VECTORS['multisig']['calls']['as_multi'], index
        assert rest[:3] == bytes([BALANCES, 3, 0]), rest[:3]
        _, tail = read_compact(rest[35:])
        inner, rest = rest[:len(rest) - len(tail)], tail
        call_hash = hashlib.blake2b(inner, digest_size=32).digest()
    ref_time, rest = read_compact(rest)
    proof_size, rest = read_compact(rest)
    assert not rest
    pending = live['pending'].get(call_hash)
    if pending is None:
        assert timepoint is None and inner is None, 'a first approval names no timepoint and carries no call'
        live['pending'][call_hash] = dict(when=(live['height'], 1), depositor=sender, approvals=[sender])
        return
    assert timepoint == pending['when'] and sender not in pending['approvals'], (timepoint, pending)
    assert inner is not None and (ref_time, proof_size) >= WEIGHT, 'MaxWeightTooLow'
    del live['pending'][call_hash]
    live['executed'].append((inner[3:35], read_compact(inner[35:])[0]))


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass

    def do_POST(self):
        call = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        name, params = call['method'], call['params']
        if name == 'chain_getBlockHash':
            result = GENESIS if params == [0] else block_hash(params[0] if params else live['height'])
        elif name == 'chain_getFinalizedHead': result = block_hash(live['height'])
        elif name == 'chain_getHeader': result = {'number': hex(int(params[0], 16))}
        elif name == 'state_getRuntimeVersion': result = dict(specVersion=2_005_000, transactionVersion=15)
        elif name == 'state_getMetadata': result = METADATA
        elif name == 'system_accountNextIndex': result = 0
        elif name == 'payment_queryInfo':
            result = dict(partialFee=str(SCALE // 1000), weight=dict(refTime=WEIGHT[0], proofSize=WEIGHT[1]))
        elif name == 'state_getStorage':
            key = bytes.fromhex(params[0][2:])
            if key.startswith(ACCOUNT_PREFIX):
                result = '0x' + (bytes(16) + (10 * SCALE).to_bytes(16, 'little') + bytes(48)).hex()
            else:
                assert key.startswith(MULTISIGS_PREFIX), key.hex()
                pending = live['pending'].get(key[88:120])
                result = None if pending is None else '0x' + (
                    pending['when'][0].to_bytes(4, 'little') + pending['when'][1].to_bytes(4, 'little')
                    + DEPOSIT.to_bytes(16, 'little') + pending['depositor']
                    + compact(len(pending['approvals'])) + b''.join(pending['approvals'])).hex()
        elif name == 'author_submitExtrinsic':
            raw = bytes.fromhex(params[0][2:])
            size, body = read_compact(raw)
            assert size == len(body) and body[:2] == b'\x84\x00' and body[34] == 1 and body[99] == 0
            sender = body[2:34]
            _, after_nonce = read_compact(body[100:])
            assert after_nonce[:3] == bytes(3)
            live['submitted'].append(sender)
            apply(sender, after_nonce[3:])
            live['height'] += 1
            result = '0x' + hashlib.blake2b(raw, digest_size=32).hexdigest()
        else:
            raise AssertionError('unexpected method: ' + name)
        raw = json.dumps(dict(jsonrpc='2.0', id=call['id'], result=result)).encode()
        self.send_response(200); self.send_header('Content-Length', str(len(raw)))
        self.end_headers(); self.wfile.write(raw)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
try:
    with tempfile.TemporaryDirectory(prefix='spectra-substrate-multisig-') as directory:
        def run(data, *args, env=None, refusal=None):
            result = subprocess.run([BINARY, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120, env={**os.environ, **(env or {})})
            assert (result.returncode == 0) == (refusal is None), (data, args, result.stdout, result.stderr)
            if refusal is not None:
                assert refusal in result.stdout + result.stderr, (refusal, args, result.stdout, result.stderr)
                return None
            return json.loads(result.stdout)

        def sign(data, session, signer, **kw):
            result = run(data, 'multisig', 'sign', session['id'], '--review-digest', session['reviewDigest'],
                         '--signer', signer, **kw)
            return result and result['session']

        def signed_by(session):
            return sorted(s['signer'] for s in session['signers'] if s['signed'])

        addresses = {}
        for data, name, phrase in [('a', 'S0', PHRASES[0]), ('a', 'S2', PHRASES[2]), ('b', 'S1', PHRASES[1])]:
            addresses[name] = run(data, 'wallet', 'import', '--chain', 'polkadot', '--name', name, '--no-password',
                                  env={'SPECTRA_SEED': phrase})['wallet']['address']
        policy = json.dumps(dict(threshold=2, signatories=[addresses['S2'], addresses['S0'], addresses['S1']]))
        for data in ['a', 'b']:
            run(data, 'endpoints', '--chain', 'polkadot', '--api', 'substrate-json-rpc', '--capabilities',
                'balance,fee,verification,broadcast', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'polkadot', '--custom-only', 'true')
            run(data, 'wallet', 'watch', '--chain', 'polkadot', '--name', 'Vault', '--multisig', policy)

        created = run('a', 'multisig', 'create', '--from', 'Vault', '--to', ss58(RECIPIENT), '--amount', '1')['session']
        by_s0 = sign('a', created, 'S0')
        assert signed_by(by_s0) == [addresses['S0']] and not by_s0['submittedTxid'], by_s0

        at_b = run('b', 'multisig', 'import', '--wallet', 'Vault', '--data', by_s0['data'])['session']
        assert signed_by(at_b) == [addresses['S0']] and at_b['sequence'] == '200-1', at_b
        assert at_b['reviewDigest'] == created['reviewDigest'], at_b
        executed = sign('b', at_b, 'S1')
        assert executed['complete'] and executed['submittedTxid'], executed
        assert live['executed'] == [(RECIPIENT, SCALE)] and not live['pending'], live
        sign('b', at_b, 'S1', refusal='already submitted')
        assert live['submitted'] == [account_of(addresses['S0']), account_of(addresses['S1'])], live['submitted']
    print('PASS asset hub multisig: a transfer approved by hash in one data directory and with its call in '
          'another, executed once, its session there submitted')
finally:
    server.shutdown()
    server.server_close()
