#!/usr/bin/env python3
"""A Safe on Sepolia against a loopback EVM node, across two owners' data
directories.

The Safe is watched as an address; its policy is read from the network
(the official 1.4.1 proxy and singleton, three owners, threshold two), and
a module and a guard show as warnings. Owner P0 builds a transaction at the
Safe's nonce and signs it; owner P1, in its own data directory, reads P0's
copy and signs; P0 joins P1's copy and executes it, paying the gas: the
executing transaction carries exactly the `execTransaction` call
protocol-kit encodes for the same signatures (core/tests/fixtures/
safe-multisig.json). Refused on the way: a contract that is not an official
Safe proxy, a delegate call, a signature for another review digest, a
wallet that is no owner, an owner signing twice, an execution short of the
threshold, a nonce already used, and owners changed after the session was
built.
"""
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
FIXTURE = json.loads((root / 'core/tests/fixtures/safe-multisig.json').read_text())
SAFE = FIXTURE['safe'].lower()
FAKE = '0x' + '5a' * 20
OWNERS = [owner['address'].lower() for owner in FIXTURE['owners']]
OUTSIDER_KEY = '42' * 32
NATIVE = FIXTURE['transactions'][0]
SEPOLIA = next(c for c in NATIVE['chains'] if c['chain_id'] == 11155111)
# The 1.4.1 factory's proxy, its singleton in slot 0.
PROXY = ('0x608060405273ffffffffffffffffffffffffffffffffffffffff600054167fa619486e000000000000000000'
         '0000000000000000000000000000000000000060003514156050578060005260206000f35b3660008037600080'
         '366000845af43d6000803e60008114156070573d6000fd5b3d6000f3fea264697066735822122003d1488ee65e08'
         'fa41e58e888a9865554c535f2c77126a82cb4c0f917f31441364736f6c63430007060033')
SINGLETON = '41675c099f32341bf84bfc5382af534df5c7461a'
GUARD_SLOT = '0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8'
MODULE = '77' * 20
GUARD = '88' * 20
state = {}


def word(value):
    return format(value, '064x')


def addresses(items):
    return '0x' + word(32) + word(len(items)) + ''.join(word(int(item, 16)) for item in items)


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def call(self, to, data):
        if to != SAFE:
            return None
        selector = data[2:10]
        if selector == 'a0e67e2b':
            return addresses(state['owners'])
        if selector == 'e75235b8':
            return '0x' + word(state['threshold'])
        if selector == 'affed0e0':
            return '0x' + word(state['nonce'])
        if selector == 'ffa1ad74':
            return '0x' + word(32) + word(5) + b'1.4.1'.hex().ljust(64, '0')
        if selector == 'cc2f8452':
            modules = [MODULE] if state['module'] else []
            return '0x' + word(64) + word(1) + word(len(modules)) + ''.join(word(int(m, 16)) for m in modules)
        return None

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))

        def answer(call):
            method, params = call['method'], call.get('params', [])
            done = lambda result: {'jsonrpc': '2.0', 'id': call['id'], 'result': result}
            if method == 'eth_call':
                result = self.call(params[0]['to'].lower(), params[0]['data'])
                if result is None:
                    return {'jsonrpc': '2.0', 'id': call['id'], 'error': {'code': 3, 'message': 'execution reverted'}}
                return done(result)
            if method == 'eth_getCode':
                return done(PROXY if params[0].lower() == SAFE else ('0x6080' if params[0].lower() == FAKE else '0x'))
            if method == 'eth_getStorageAt':
                if params[0].lower() == SAFE and int(params[1], 16) == 0:
                    return done('0x' + word(int(SINGLETON, 16)))
                if params[0].lower() == SAFE and params[1] == GUARD_SLOT and state['guard']:
                    return done('0x' + word(int(GUARD, 16)))
                return done('0x' + word(0))
            if method == 'eth_sendRawTransaction':
                raw = params[0]
                # The executing transaction calls the Safe with protocol-kit's
                # execTransaction for P0's and P1's signatures.
                assert SEPOLIA['exec_transaction_p0_p1'][2:] in raw, raw
                state['submitted'].append(raw)
                return done(state['hash_of'](raw))
            values = {'eth_chainId': hex(11155111), 'eth_getBalance': hex(10 * 10**18),
                      'eth_getTransactionCount': '0x4', 'eth_estimateGas': '0x186a0', 'eth_blockNumber': '0x20',
                      'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]}}
            assert method in values, method
            return done(values[method])

        self.reply(list(map(answer, body)) if isinstance(body, list) else answer(body))


def keccak(data):
    """Keccak-256 (the pre-standard padding Ethereum uses)."""
    rc = [0x0000000000000001, 0x0000000000008082, 0x800000000000808A, 0x8000000080008000, 0x000000000000808B,
          0x0000000080000001, 0x8000000080008081, 0x8000000000008009, 0x000000000000008A, 0x0000000000000088,
          0x0000000080008009, 0x000000008000000A, 0x000000008000808B, 0x800000000000008B, 0x8000000000008089,
          0x8000000000008003, 0x8000000000008002, 0x8000000000000080, 0x000000000000800A, 0x800000008000000A,
          0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008]
    rot = [[0, 36, 3, 41, 18], [1, 44, 10, 45, 2], [62, 6, 43, 15, 61], [28, 55, 25, 21, 56], [27, 20, 39, 8, 14]]
    mask = (1 << 64) - 1
    rate = 136
    data = bytearray(data) + b'\x01'
    data += b'\x00' * (-len(data) % rate)
    data[-1] |= 0x80
    lanes = [[0] * 5 for _ in range(5)]
    for offset in range(0, len(data), rate):
        for i in range(rate // 8):
            lanes[i % 5][i // 5] ^= int.from_bytes(data[offset + 8 * i:offset + 8 * i + 8], 'little')
        for constant in rc:
            c = [lanes[x][0] ^ lanes[x][1] ^ lanes[x][2] ^ lanes[x][3] ^ lanes[x][4] for x in range(5)]
            d = [c[(x - 1) % 5] ^ (((c[(x + 1) % 5] << 1) | (c[(x + 1) % 5] >> 63)) & mask) for x in range(5)]
            lanes = [[lanes[x][y] ^ d[x] for y in range(5)] for x in range(5)]
            b = [[0] * 5 for _ in range(5)]
            for x in range(5):
                for y in range(5):
                    r = rot[x][y]
                    b[y][(2 * x + 3 * y) % 5] = ((lanes[x][y] << r) | (lanes[x][y] >> (64 - r))) & mask if r else lanes[x][y]
            lanes = [[b[x][y] ^ ((~b[(x + 1) % 5][y]) & b[(x + 2) % 5][y]) for y in range(5)] for x in range(5)]
            lanes[0][0] ^= constant
    return b''.join(lanes[i % 5][i // 5].to_bytes(8, 'little') for i in range(4))


assert keccak(b'').hex() == 'c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470'
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
state.update(owners=list(OWNERS), threshold=2, nonce=7, module=True, guard=True, submitted=[],
             hash_of=lambda raw: '0x' + keccak(bytes.fromhex(raw[2:])).hex())
try:
    with tempfile.TemporaryDirectory(prefix='spectra-safe-') as directory:
        journal = pathlib.Path(directory) / 'network.jsonl'

        def run(data, *args, success=True, env=None, refusal=None):
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120,
                                    env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(journal), **(env or {})})
            assert (result.returncode == 0) == (success and refusal is None), (data, args, result.stdout,
                                                                               result.stderr)
            if refusal is not None:
                assert refusal in result.stdout + result.stderr, (refusal, args, result.stdout, result.stderr)
                return None
            return json.loads(result.stdout) if result.stdout.strip() else None

        def signed_by(session):
            return sorted(signer['signer'] for signer in session['signers'] if signer['signed'])

        for data, owner in [('p0', 0), ('p1', 1)]:
            run(data, 'endpoints', '--chain', 'ethereum-sepolia', '--api', 'evm-json-rpc', '--capabilities',
                'balance,fee,broadcast,verification', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'ethereum-sepolia', '--custom-only', 'true')
            run(data, 'wallet', 'watch', '--chain', 'ethereum-sepolia', '--name', 'Safe', '--address', SAFE)
            run(data, 'wallet', 'import', '--chain', 'ethereum-sepolia', '--name', 'Owner', '--no-password',
                '--private-key-env', 'KEY', env={'KEY': FIXTURE['owners'][owner]['private_key'][2:]})
        run('p0', 'wallet', 'import', '--chain', 'ethereum-sepolia', '--name', 'Outsider', '--no-password',
            '--private-key-env', 'KEY', env={'KEY': OUTSIDER_KEY})
        run('p0', 'wallet', 'watch', '--chain', 'ethereum-sepolia', '--name', 'Fake', '--address', FAKE)
        actions = [offer['action'] for offer in run('p0', 'wallet', 'actions', 'Safe')['actions']['actions']]
        assert 'multisig' in actions and 'send' not in actions, actions

        # The policy, from the network; a contract that is no official
        # proxy is no Safe.
        account = run('p0', 'multisig', 'account', 'Safe')['account']
        permission = account['permissions'][0]
        assert permission['threshold'] == 2 and [s['signer'] for s in permission['signers']] == OWNERS, account
        assert permission['signers'][0]['walletId'], account
        assert any(MODULE in w for w in account['warnings']) and any(GUARD in w for w in account['warnings']), account
        run('p0', 'multisig', 'account', 'Fake', refusal='not an official Safe proxy')

        # P0 builds and signs; P1 reads P0's copy and signs.
        created = run('p0', 'multisig', 'create', '--from', 'Safe', '--to', NATIVE['to'], '--amount', '0.01')['session']
        assert created['scheme'] == 'safe' and created['sequence'] == '7', created
        assert created['transactionId'] == SEPOLIA['safe_tx_hash'] == created['reviewDigest'], created
        run('p0', 'multisig', 'sign', created['id'], '--review-digest', '0x' + '00' * 32, '--signer', 'Owner',
            refusal='not the one reviewed')
        run('p0', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer',
            'Outsider', refusal="is not one of the Safe's owners")
        signed = run('p0', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'],
                     '--signer', 'Owner')['session']
        assert signed_by(signed) == [OWNERS[0]] and not signed['complete'], signed
        run('p0', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer', 'Owner',
            refusal='already signed')
        run('p0', 'multisig', 'submit', created['id'], '--executor', 'Owner', '--yes',
            refusal='enough owners')

        at_p1 = run('p1', 'multisig', 'import', '--wallet', 'Safe', '--data', signed['data'])['session']
        assert at_p1['reviewDigest'] == created['reviewDigest'] and signed_by(at_p1) == [OWNERS[0]], at_p1
        by_p1 = run('p1', 'multisig', 'sign', at_p1['id'], '--review-digest', at_p1['reviewDigest'],
                    '--signer', 'Owner')['session']
        assert by_p1['complete'], by_p1

        # A delegate call is refused when read.
        delegate = json.loads(signed['data'])
        delegate['transaction']['operation'] = 1
        delegate['signatures'] = []
        run('p0', 'multisig', 'import', '--wallet', 'Safe', '--data', json.dumps(delegate), refusal='delegate call')

        # P0 joins P1's copy and executes, paying the gas.
        joined = run('p0', 'multisig', 'import', '--wallet', 'Safe', '--data', by_p1['data'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        assert signed_by(joined) == sorted(OWNERS[:2]), joined
        assert run('p0', 'multisig', 'finalize', created['id'])['raw'] == SEPOLIA['exec_transaction_p0_p1']
        state['threshold'] = 3
        run('p0', 'multisig', 'submit', created['id'], '--executor', 'Owner', '--yes', refusal='changed since')
        state['threshold'] = 2
        run('p0', 'multisig', 'submit', created['id'], '--executor', 'Outsider', '--yes',
            refusal="is not one of the Safe's owners")
        assert not state['submitted']
        executed = run('p0', 'multisig', 'submit', created['id'], '--executor', 'Owner', '--yes')['session']
        assert len(state['submitted']) == 1, state['submitted']
        assert executed['submittedTxid'] == state['hash_of'](state['submitted'][0]), executed
        run('p0', 'multisig', 'submit', created['id'], '--executor', 'Owner', '--yes', refusal='already submitted')

        # P1's session's nonce is used now; signing it again is refused.
        state['nonce'] = 8
        run('p1', 'multisig', 'sign', at_p1['id'], '--review-digest', at_p1['reviewDigest'], '--signer', 'Owner',
            refusal='already used')
        run('p1', 'multisig', 'import', '--wallet', 'Safe', '--data', by_p1['data'], refusal='already used')
        # The next one takes nonce 8, and a session built before an owner
        # change is not signed after it.
        later = run('p1', 'multisig', 'create', '--from', 'Safe', '--to', NATIVE['to'], '--amount', '0.01')['session']
        assert later['sequence'] == '8', later
        state['owners'] = OWNERS[:2] + ['0x' + '99' * 20]
        run('p1', 'multisig', 'sign', later['id'], '--review-digest', later['reviewDigest'], '--signer', 'Owner',
            refusal='changed since')
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
    print('PASS safe: policy, modules and guard read from the network, an unofficial proxy refused; two owners '
          'sign in their own data directories, the copies join, an owner executes with protocol-kit\'s calldata; '
          'stale reviews, outsiders, double signatures, short thresholds, used nonces, delegate calls and '
          'changed owners refused')
finally:
    server.shutdown()
    server.server_close()
