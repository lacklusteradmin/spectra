#!/usr/bin/env python3
"""A 2-of-3 pallet-multisig account on Asset Hub or Bittensor (the second
argument) against a loopback node that keeps the pallet's state, across two
data directories.

The account is watched from its policy at the address polkadot.js derives.
A transfer is built in one data directory; its first signatory approves it
by hash from its own account, reserving the deposit; the call goes to the
other directory, which reads the approval from the node, and its signatory's
approval carries the call, within the weight the node reports, and the node
executes it. Refused on the way: an outsider, an approval twice, a second
approval before the first is in a block, a transfer the account cannot pay,
the same transfer built twice, and `multisig submit`, which approvals make
needless.
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
CHAIN = sys.argv[2] if len(sys.argv) > 2 else 'polkadot'
FINNEY = CHAIN == 'bittensor'
SCALE = 10 ** (9 if FINNEY else 10)
WIDTH = 8 if FINNEY else 16
PREFIX = 42 if FINNEY else 0
GENESIS = ('0x2f0555cc76fc2840a25a6ea3b9637146806f1f44b090c175ffde2a7e5ab36c03' if FINNEY
           else '0x68d56f15f85d3136970ec16946040bc1752654e906147f7e43e9d539d7c3de2f')
METADATA = '0x' + (ROOT / 'core/tests/fixtures' / ('bittensor-finney-metadata.scale' if FINNEY
                                                     else 'asset-hub-polkadot-metadata.scale')).read_bytes().hex()
VECTORS = json.loads((ROOT / 'core/tests/fixtures/substrate-multisig.json').read_text())['runtimes'][
    'bittensor' if FINNEY else 'polkadot-asset-hub']
BALANCES, MULTISIG = VECTORS['balances']['pallet_index'], VECTORS['multisig']['pallet_index']
DEPOSIT = int(VECTORS['multisig']['deposit_base']) + 2 * int(VECTORS['multisig']['deposit_factor'])
ACCOUNT_PREFIX = bytes.fromhex('26aa394eea5630e07c48ae0c9558cef7b99d880ec681799c0cf30e8886371da9')
MULTISIGS_PREFIX = bytes.fromhex(VECTORS['storage']['prefix'][2:])
WEIGHT = (1_234_567, 8_910)
FEE = SCALE // 1000
PHRASES = ['abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
           'legal winner thank year wave sausage worth useful legal winner thank yellow',
           'letter advice cage absurd amount doctor acoustic avoid letter advice cage above',
           'zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong']
RECIPIENT = bytes([0xdd]) * 32
ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


def b58decode(text):
    number = 0
    for char in text:
        number = number * 58 + ALPHABET.index(char)
    raw = number.to_bytes((number.bit_length() + 7) // 8, 'big')
    return bytes(len(text) - len(text.lstrip('1'))) + raw


def b58encode(raw):
    number, out = int.from_bytes(raw, 'big'), ''
    while number:
        number, rest = divmod(number, 58)
        out = ALPHABET[rest] + out
    return '1' * (len(raw) - len(raw.lstrip(b'\0'))) + out


def ss58(account):
    payload = bytes([PREFIX]) + account
    return b58encode(payload + hashlib.blake2b(b'SS58PRE' + payload).digest()[:2])


def account_of(address):
    return b58decode(address)[1:33]


def compact(value):
    if value < 64: return bytes([value << 2])
    if value < 16384: return ((value << 2) | 1).to_bytes(2, 'little')
    if value < 2**30: return ((value << 2) | 2).to_bytes(4, 'little')
    length = max(4, (value.bit_length() + 7) // 8)
    return bytes([((length - 4) << 2) | 3]) + value.to_bytes(length, 'little')


def read_compact(raw):
    mode = raw[0] & 3
    count = [1, 2, 4, (raw[0] >> 2) + 5][mode]
    if mode == 3: return int.from_bytes(raw[1:count], 'little'), raw[count:]
    return int.from_bytes(raw[:count], 'little') >> 2, raw[count:]


def multi_account(signatories, threshold):
    ordered = sorted(signatories)
    return hashlib.blake2b(b'modlpy/utilisuba' + compact(len(ordered)) + b''.join(ordered)
                           + threshold.to_bytes(2, 'little'), digest_size=32).digest()


def block_hash(number): return '0x' + number.to_bytes(32, 'big').hex()


live = dict(free={}, reserved={}, pending={}, include=True, queued=[], height=200, submitted=[], executed=[],
            errors=[], vault=None)


def apply(sender, call):
    """pallet-multisig's dispatch of an approval, as the mock keeps it."""
    assert call[0] == MULTISIG, call
    index, threshold = call[1], int.from_bytes(call[2:4], 'little')
    count, rest = read_compact(call[4:])
    others, rest = [rest[i * 32:i * 32 + 32] for i in range(count)], rest[count * 32:]
    assert others == sorted(others) and sender not in others
    timepoint = None
    if rest[0] == 1:
        timepoint, rest = (int.from_bytes(rest[1:5], 'little'), int.from_bytes(rest[5:9], 'little')), rest[9:]
    else:
        rest = rest[1:]
    account = multi_account(others + [sender], threshold)
    assert account == live['vault'], 'an approval for another account'
    if index == VECTORS['multisig']['calls']['approve_as_multi']:
        call_hash, rest = rest[:32], rest[32:]
        inner = None
    else:
        assert index == VECTORS['multisig']['calls']['as_multi'], index
        assert rest[:3] == bytes([BALANCES, 3, 0]), rest[:3]
        amount, tail = read_compact(rest[35:])
        inner, rest = rest[:len(rest) - len(tail)], tail
        call_hash = hashlib.blake2b(inner, digest_size=32).digest()
    ref_time, rest = read_compact(rest)
    proof_size, rest = read_compact(rest)
    assert not rest
    pending = live['pending'].get(call_hash)
    if pending is None:
        assert timepoint is None and inner is None, 'a first approval names no timepoint and carries no call'
        live['reserved'][sender] = live['reserved'].get(sender, 0) + DEPOSIT
        live['free'][sender] -= DEPOSIT
        live['pending'][call_hash] = dict(when=(live['height'], 1), depositor=sender, approvals=[sender])
        return
    assert timepoint == pending['when'], (timepoint, pending)
    if sender in pending['approvals']:
        live['errors'].append('AlreadyApproved')
        return
    if inner is not None and len(pending['approvals']) + 1 >= threshold:
        assert (ref_time, proof_size) >= WEIGHT, 'MaxWeightTooLow'
        del live['pending'][call_hash]
        live['reserved'][pending['depositor']] -= DEPOSIT
        live['free'][pending['depositor']] += DEPOSIT
        dest, value = inner[3:35], read_compact(inner[35:])[0]
        live['free'][account] -= value
        live['free'][dest] = live['free'].get(dest, 0) + value
        live['executed'].append((dest, value))
        return
    assert inner is None, 'a call carried below the threshold'
    pending['approvals'] = sorted(pending['approvals'] + [sender])


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
            result = dict(partialFee=str(FEE), weight=dict(refTime=WEIGHT[0], proofSize=WEIGHT[1]))
        elif name == 'state_getStorage':
            key = bytes.fromhex(params[0][2:])
            if key.startswith(ACCOUNT_PREFIX):
                account = key[-32:]
                record = bytes(16) + live['free'].get(account, 0).to_bytes(WIDTH, 'little')
                record += live['reserved'].get(account, 0).to_bytes(WIDTH, 'little') + bytes(WIDTH) + bytes(16)
                result = '0x' + record.hex()
            else:
                assert key.startswith(MULTISIGS_PREFIX) and key[40:72] == live['vault'], key.hex()
                pending = live['pending'].get(key[88:120])
                result = None if pending is None else '0x' + (
                    pending['when'][0].to_bytes(4, 'little') + pending['when'][1].to_bytes(4, 'little')
                    + DEPOSIT.to_bytes(WIDTH, 'little') + pending['depositor']
                    + compact(len(pending['approvals'])) + b''.join(pending['approvals'])).hex()
        elif name == 'author_submitExtrinsic':
            raw = bytes.fromhex(params[0][2:])
            size, body = read_compact(raw)
            assert size == len(body) and body[:2] == b'\x84\x00' and body[34] == 1 and body[99] == 0
            sender = body[2:34]
            _, after_nonce = read_compact(body[100:])
            extra = 2 if FINNEY else 3
            assert after_nonce[:extra] == bytes(extra)
            live['submitted'].append(sender)
            live['free'][sender] -= FEE
            if live['include']:
                apply(sender, after_nonce[extra:])
                live['height'] += 1
            else:
                live['queued'].append((sender, after_nonce[extra:]))
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
        journal = pathlib.Path(directory) / 'network.jsonl'

        def run(data, *args, env=None, refusal=None):
            result = subprocess.run([BINARY, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120,
                                    env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(journal), **(env or {})})
            assert (result.returncode == 0) == (refusal is None), (data, args, result.stdout, result.stderr)
            if refusal is not None:
                assert refusal in result.stdout + result.stderr, (refusal, args, result.stdout, result.stderr)
                return None
            return json.loads(result.stdout)

        def signed_by(session):
            return sorted(s['signer'] for s in session['signers'] if s['signed'])

        addresses = {}
        for data, name, phrase in [('a', 'S0', PHRASES[0]), ('a', 'S2', PHRASES[2]), ('a', 'Outsider', PHRASES[3]),
                                   ('b', 'S1', PHRASES[1])]:
            wallet = run(data, 'wallet', 'import', '--chain', CHAIN, '--name', name, '--no-password',
                         env={'SPECTRA_SEED': phrase})['wallet']
            addresses[name] = wallet['address']
            live['free'][account_of(wallet['address'])] = 10 * SCALE
        signatories = [addresses['S2'], addresses['S0'], addresses['S1']]
        live['vault'] = multi_account([account_of(a) for a in signatories], 2)
        live['free'][live['vault']] = 10 * SCALE
        policy = json.dumps(dict(threshold=2, signatories=signatories))
        for data in ['a', 'b']:
            run(data, 'endpoints', '--chain', CHAIN, '--api', 'substrate-json-rpc', '--capabilities',
                'balance,fee,verification,broadcast', '--add', endpoint)
            run(data, 'endpoints', '--chain', CHAIN, '--custom-only', 'true')
            watched = run(data, 'wallet', 'watch', '--chain', CHAIN, '--name', 'Vault', '--multisig', policy)
            assert list(watched['wallets'][0]['addresses'].values()) == [ss58(live['vault'])], watched
        ordered = [ss58(account) for account in sorted(account_of(a) for a in signatories)]
        account = run('a', 'multisig', 'account', 'Vault')['account']
        assert account['scheme'] == 'substrateMultisig' and account['address'] == ss58(live['vault']), account
        assert [s['signer'] for s in account['permissions'][0]['signers']] == ordered, account

        run('a', 'multisig', 'create', '--from', 'Vault', '--to', ss58(RECIPIENT), '--amount', '10',
            refusal='keep its existential deposit')
        created = run('a', 'multisig', 'create', '--from', 'Vault', '--to', ss58(RECIPIENT), '--amount', '1')['session']
        call = bytes([BALANCES, 3, 0]) + RECIPIENT + compact(SCALE)
        assert created['data'] == '0x' + call.hex() and created['outputs'][0]['value'] == str(SCALE), created
        assert created['transactionId'] == '0x' + hashlib.blake2b(call, digest_size=32).hexdigest(), created
        run('a', 'multisig', 'create', '--from', 'Vault', '--to', ss58(RECIPIENT), '--amount', '1',
            refusal='already makes this transfer')
        sign = lambda data, session, signer, **kw: run(data, 'multisig', 'sign', session['id'], '--review-digest',
                                                       session['reviewDigest'], '--signer', signer, **kw)
        sign('a', created, 'Outsider', refusal='not one of the account')

        live['include'] = False
        by_s0 = sign('a', created, 'S0')['session']
        assert signed_by(by_s0) == [addresses['S0']] and not by_s0['complete'] and not by_s0['submittedTxid'], by_s0
        sign('a', created, 'S2', refusal='not in a block yet')
        live['include'] = True
        apply(*live['queued'].pop())
        live['height'] += 1
        assert live['reserved'][account_of(addresses['S0'])] == DEPOSIT
        sign('a', created, 'S0', refusal='already approved')
        run('a', 'multisig', 'submit', created['id'], '--yes', refusal='goes to the network as it signs')

        at_b = run('b', 'multisig', 'import', '--wallet', 'Vault', '--data', by_s0['data'])['session']
        assert signed_by(at_b) == [addresses['S0']] and at_b['sequence'] == '200-1', at_b
        assert at_b['reviewDigest'] == created['reviewDigest'], at_b
        executed = sign('b', at_b, 'S1')['session']
        assert executed['complete'] and executed['submittedTxid'], executed
        assert signed_by(executed) == sorted([addresses['S0'], addresses['S1']]), executed
        assert live['executed'] == [(RECIPIENT, SCALE)] and not live['pending'] and not live['errors'], live
        assert live['reserved'][account_of(addresses['S0'])] == 0
        sign('b', at_b, 'S1', refusal='already submitted')
        sign('a', created, 'S2', refusal='executed or cancelled')
        assert live['submitted'] == [account_of(addresses['S0']), account_of(addresses['S1'])], live['submitted']
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
    print(f'PASS {CHAIN} multisig: a pallet-multisig account watched at its derived address; approved by hash '
          'with the deposit reserved, then by the call within the reported weight in another data directory, '
          'and executed; outsiders, double approvals, unincluded first approvals, unpayable and duplicate '
          'transfers and needless submissions refused')
finally:
    server.shutdown()
    server.server_close()
