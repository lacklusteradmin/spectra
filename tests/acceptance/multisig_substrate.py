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
import json

from harness import ABANDON, FIXTURES, LEGAL, JsonRpc, Spectra, fixture, scratch, serve, signed_by

SCALE = 10 ** 10
GENESIS = '0x68d56f15f85d3136970ec16946040bc1752654e906147f7e43e9d539d7c3de2f'
METADATA = '0x' + (FIXTURES / 'asset-hub-polkadot-metadata.scale').read_bytes().hex()
VECTORS = fixture('substrate-multisig.json')['runtimes']['polkadot-asset-hub']
BALANCES, MULTISIG = VECTORS['balances']['pallet_index'], VECTORS['multisig']['pallet_index']
DEPOSIT = int(VECTORS['multisig']['deposit_base']) + 2 * int(VECTORS['multisig']['deposit_factor'])
ACCOUNT_PREFIX = bytes.fromhex('26aa394eea5630e07c48ae0c9558cef7b99d880ec681799c0cf30e8886371da9')
MULTISIGS_PREFIX = bytes.fromhex(VECTORS['storage']['prefix'][2:])
WEIGHT = (1_234_567, 8_910)
PHRASES = [ABANDON, LEGAL, 'letter advice cage absurd amount doctor acoustic avoid letter advice cage above']
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


def apply(live, sender, call):
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


class Node(JsonRpc):
    def call(self, name, params):
        live = self.state
        if name == 'chain_getBlockHash':
            return GENESIS if params == [0] else block_hash(params[0] if params else live['height'])
        if name == 'chain_getFinalizedHead': return block_hash(live['height'])
        if name == 'chain_getHeader': return {'number': hex(int(params[0], 16))}
        if name == 'state_getRuntimeVersion': return dict(specVersion=2_005_000, transactionVersion=15)
        if name == 'state_getMetadata': return METADATA
        if name == 'system_accountNextIndex': return 0
        if name == 'payment_queryInfo':
            return dict(partialFee=str(SCALE // 1000), weight=dict(refTime=WEIGHT[0], proofSize=WEIGHT[1]))
        if name == 'state_getStorage':
            key = bytes.fromhex(params[0][2:])
            if key.startswith(ACCOUNT_PREFIX):
                return '0x' + (bytes(16) + (10 * SCALE).to_bytes(16, 'little') + bytes(48)).hex()
            assert key.startswith(MULTISIGS_PREFIX), key.hex()
            pending = live['pending'].get(key[88:120])
            return None if pending is None else '0x' + (
                pending['when'][0].to_bytes(4, 'little') + pending['when'][1].to_bytes(4, 'little')
                + DEPOSIT.to_bytes(16, 'little') + pending['depositor']
                + compact(len(pending['approvals'])) + b''.join(pending['approvals'])).hex()
        assert name == 'author_submitExtrinsic', 'unexpected method: ' + name
        raw = bytes.fromhex(params[0][2:])
        size, body = read_compact(raw)
        assert size == len(body) and body[:2] == b'\x84\x00' and body[34] == 1 and body[99] == 0
        sender = body[2:34]
        _, after_nonce = read_compact(body[100:])
        assert after_nonce[:3] == bytes(3)
        live['submitted'].append(sender)
        apply(live, sender, after_nonce[3:])
        live['height'] += 1
        return '0x' + hashlib.blake2b(raw, digest_size=32).hexdigest()


with serve(Node, pending={}, height=200, submitted=[], executed=[]) as node, \
        scratch('substrate-multisig') as directory:
    live = node.state
    spectra = Spectra(directory)
    a, b = spectra / 'a', spectra / 'b'

    addresses = {}
    for signer, name, phrase in [(a, 'S0', PHRASES[0]), (a, 'S2', PHRASES[2]), (b, 'S1', PHRASES[1])]:
        addresses[name] = signer.run('wallet', 'import', '--chain', 'polkadot', '--name', name, '--no-password',
                                     env={'SPECTRA_SEED': phrase})['wallet']['address']
    policy = json.dumps(dict(threshold=2, signatories=[addresses['S2'], addresses['S0'], addresses['S1']]))
    for signer in [a, b]:
        signer.run('endpoints', '--chain', 'polkadot', '--api', 'substrate-json-rpc', '--capabilities',
                   'balance,fee,verification,broadcast', '--add', node.url)
        signer.run('endpoints', '--chain', 'polkadot', '--custom-only', 'true')
        signer.run('wallet', 'watch', '--chain', 'polkadot', '--name', 'Vault', '--multisig', policy)

    created = a.run('multisig', 'create', 'Vault', '--to', ss58(RECIPIENT), '--amount', '1')['session']
    by_s0 = a.sign(created, 'S0')
    assert signed_by(by_s0) == [addresses['S0']] and not by_s0['submittedTxid'], by_s0

    at_b = b.run('multisig', 'import', 'Vault', '--data', by_s0['data'])['session']
    assert signed_by(at_b) == [addresses['S0']] and at_b['sequence'] == '200-1', at_b
    assert at_b['reviewDigest'] == created['reviewDigest'], at_b
    executed = b.sign(at_b, 'S1')
    assert executed['complete'] and executed['submittedTxid'], executed
    assert live['executed'] == [(RECIPIENT, SCALE)] and not live['pending'], live
    refusal = b.fail('multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'], '--signer', 'S1')
    assert 'already submitted' in refusal['error'], refusal
    assert live['submitted'] == [account_of(addresses['S0']), account_of(addresses['S1'])], live['submitted']
print('PASS asset hub multisig: a transfer approved by hash in one data directory and with its call in '
      'another, executed once, its session there submitted')
