#!/usr/bin/env python3
"""A Safe on Sepolia against a loopback EVM node, across two owners' data
directories.

The Safe is watched as an address and its policy read from the node (the
official 1.4.1 proxy and singleton, three owners, threshold two). Owner P0
builds a transaction and signs it; owner P1, in its own data directory,
reads P0's copy and signs; P0 joins P1's copy and executes it once, paying
the gas, and the session keeps the executing transaction's hash: a second
submission is refused as already submitted.

Core's tests own the rules: hashes and calldata as protocol-kit makes them,
official proxies, owners, thresholds, nonces, modules and guards
(core/src/send/tests/safe.rs, core/src/service/tests/multisig_safe.rs).
"""
from harness import JsonRpc, Spectra, fixture, scratch, serve, signed_by

FIXTURE = fixture('safe-multisig.json')
SAFE = FIXTURE['safe'].lower()
OWNERS = [owner['address'].lower() for owner in FIXTURE['owners']]
NATIVE = FIXTURE['transactions'][0]
# The 1.4.1 factory's proxy, its singleton in slot 0.
PROXY = ('0x608060405273ffffffffffffffffffffffffffffffffffffffff600054167fa619486e000000000000000000'
         '0000000000000000000000000000000000000060003514156050578060005260206000f35b3660008037600080'
         '366000845af43d6000803e60008114156070573d6000fd5b3d6000f3fea264697066735822122003d1488ee65e08'
         'fa41e58e888a9865554c535f2c77126a82cb4c0f917f31441364736f6c63430007060033')
SINGLETON = '41675c099f32341bf84bfc5382af534df5c7461a'
REJECTED = 3
ROUNDS = [0x0000000000000001, 0x0000000000008082, 0x800000000000808A, 0x8000000080008000,
          0x000000000000808B, 0x0000000080000001, 0x8000000080008081, 0x8000000000008009,
          0x000000000000008A, 0x0000000000000088, 0x0000000080008009, 0x000000008000000A,
          0x000000008000808B, 0x800000000000008B, 0x8000000000008089, 0x8000000000008003,
          0x8000000000008002, 0x8000000000000080, 0x000000000000800A, 0x800000008000000A,
          0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008]
# Keccak lane rotations, indexed x + 5y.
ROTATIONS = [0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15, 21, 8, 18, 2, 61, 56, 14]
LANE = (1 << 64) - 1


def word(value):
    return format(value, '064x')


def keccak256(data):
    """Keccak-256 as Ethereum hashes: the original padding, which hashlib's
    sha3_256 does not use."""
    def rotl(value, n):
        return ((value << n) | (value >> (64 - n))) & LANE

    padded = bytearray(data) + b'\x01' + bytes(-(len(data) + 1) % 136)
    padded[-1] |= 0x80
    a = [0] * 25
    for block in range(0, len(padded), 136):
        for i in range(17):
            a[i] ^= int.from_bytes(padded[block + 8 * i:block + 8 * i + 8], 'little')
        for rc in ROUNDS:
            c = [a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20] for x in range(5)]
            a = [a[i] ^ c[(i - 1) % 5] ^ rotl(c[(i + 1) % 5], 1) for i in range(25)]
            b = [0] * 25
            for i in range(25):
                x, y = i % 5, i // 5
                b[y + 5 * ((2 * x + 3 * y) % 5)] = rotl(a[i], ROTATIONS[i])
            a = [b[i] ^ (~b[(i + 1) % 5 + i // 5 * 5] & b[(i + 2) % 5 + i // 5 * 5]) for i in range(25)]
            a[0] ^= rc
    return b''.join(lane.to_bytes(8, 'little') for lane in a[:4])


def transaction_hash(raw):
    return '0x' + keccak256(bytes.fromhex(raw[2:])).hex()


class Node(JsonRpc):
    def call(self, method, params):
        if method == 'eth_call':
            assert params[0]['to'].lower() == SAFE, params
            return {
                'a0e67e2b': '0x' + word(32) + word(len(OWNERS)) + ''.join(word(int(o, 16)) for o in OWNERS),
                'e75235b8': '0x' + word(2),
                'affed0e0': '0x' + word(NATIVE['nonce']),
                'ffa1ad74': '0x' + word(32) + word(5) + b'1.4.1'.hex().ljust(64, '0'),
                'cc2f8452': '0x' + word(64) + word(1) + word(0),
            }[params[0]['data'][2:10]]
        if method == 'eth_getCode':
            return PROXY if params[0].lower() == SAFE else '0x'
        if method == 'eth_getStorageAt':
            singleton = params[0].lower() == SAFE and int(params[1], 16) == 0
            return '0x' + word(int(SINGLETON, 16) if singleton else 0)
        if method == 'eth_sendRawTransaction':
            self.state['submitted'].append(params[0])
            return transaction_hash(params[0])
        return {'eth_chainId': hex(11155111), 'eth_getBalance': hex(10 * 10**18),
                'eth_getTransactionCount': '0x4', 'eth_estimateGas': '0x186a0', 'eth_blockNumber': '0x20',
                'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]}}[method]


with serve(Node, submitted=[]) as node, scratch('safe') as directory:
    spectra = Spectra(directory)
    p0, p1 = spectra / 'p0', spectra / 'p1'
    submitted = node.state['submitted']

    for owner, signer in enumerate([p0, p1]):
        signer.run('endpoints', '--chain', 'ethereum-sepolia', '--api', 'evm-json-rpc', '--capabilities',
                   'balance,fee,broadcast,verification', '--add', node.url)
        signer.run('endpoints', '--chain', 'ethereum-sepolia', '--custom-only', 'true')
        signer.run('wallet', 'watch', '--chain', 'ethereum-sepolia', '--name', 'Safe', '--address', SAFE)
        signer.run('wallet', 'import', '--chain', 'ethereum-sepolia', '--name', 'Owner', '--no-password',
                   '--private-key-env', 'KEY', env={'KEY': FIXTURE['owners'][owner]['private_key'][2:]})

    # P0 builds and signs; P1 reads P0's copy and signs.
    created = p0.run('multisig', 'create', 'Safe', '--to', NATIVE['to'], '--amount', '0.01')['session']
    signed = p0.sign(created, 'Owner')
    assert signed_by(signed) == [OWNERS[0]] and not signed['complete'], signed
    at_p1 = p1.run('multisig', 'import', 'Safe', '--data', signed['data'])['session']
    assert at_p1['reviewDigest'] == created['reviewDigest'] and signed_by(at_p1) == [OWNERS[0]], at_p1
    by_p1 = p1.sign(at_p1, 'Owner')
    assert by_p1['complete'], by_p1

    # P0 joins P1's copy and executes once, paying the gas; the session
    # keeps the executing transaction's hash and is not submitted again.
    joined = p0.run('multisig', 'import', 'Safe', '--data', by_p1['data'])['session']
    assert joined['id'] == created['id'] and signed_by(joined) == sorted(OWNERS[:2]), joined
    executed = p0.run('multisig', 'submit', created['id'], '--executor', 'Owner', '--yes')['session']
    assert len(submitted) == 1, submitted
    assert executed['submittedTxid'] == transaction_hash(submitted[0]), executed
    refusal = p0.fail('multisig', 'submit', created['id'], '--executor', 'Owner', '--yes', code=REJECTED)
    assert refusal['error'] == 'This session was already submitted.', refusal
    assert p0.run('multisig', 'show', created['id'])['session']['submittedTxid'] == executed['submittedTxid']
    assert len(submitted) == 1, submitted
print('PASS safe: policy read from the node; two owners sign in their own data directories, the copies join '
      'and an owner executes once; a second submission refused')
