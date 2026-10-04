#!/usr/bin/env python3
"""ICP staking preparation, signing and certificate refusal; loopback only.

The independent DFINITY SDK fixture supplies Candid replies. A fake certificate
must not trigger even the first funding call. Core tests cover request binding,
reply semantics, retained proofs and recovery that cannot fund a neuron twice.
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
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / 'target/debug/spectra').resolve())
FIXTURE = json.loads((ROOT / 'core/tests/fixtures/icp-staking-vectors.json').read_text())


def cbor(value):
    def head(kind, size):
        if size < 24:
            return bytes([kind * 32 + size])
        for marker, width in [(24, 1), (25, 2), (26, 4), (27, 8)]:
            if size < 1 << (width * 8):
                return bytes([kind * 32 + marker]) + size.to_bytes(width, 'big')
        raise ValueError('fixture integer overflow')
    if isinstance(value, int):
        return head(0, value)
    if isinstance(value, bytes):
        return head(2, len(value)) + value
    if isinstance(value, str):
        raw = value.encode()
        return head(3, len(raw)) + raw
    if isinstance(value, list):
        return head(4, len(value)) + b''.join(cbor(item) for item in value)
    if isinstance(value, dict):
        pairs = sorted([(cbor(key), cbor(item)) for key, item in value.items()], key=lambda pair: (len(pair[0]), pair[0]))
        return head(5, len(pairs)) + b''.join(key + item for key, item in pairs)
    raise ValueError('fixture CBOR type')


def decode_cbor(raw):
    offset = 0
    def read():
        nonlocal offset
        first = raw[offset]
        offset += 1
        kind, size = divmod(first, 32)
        if size >= 24:
            width = {24: 1, 25: 2, 26: 4, 27: 8}[size]
            size = int.from_bytes(raw[offset:offset + width], 'big')
            offset += width
        if kind == 0:
            return size
        if kind in (2, 3):
            value = raw[offset:offset + size]
            offset += size
            return value if kind == 2 else value.decode()
        if kind == 4:
            return [read() for _ in range(size)]
        if kind == 5:
            return {read(): read() for _ in range(size)}
        if kind == 6:
            return read()
        raise ValueError('fixture CBOR shape')
    result = read()
    assert offset == len(raw)
    return result


def unsigned_leb(value):
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def respond(self, raw, content_type='application/cbor'):
        self.send_response(200)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_POST(self):
        state = self.server.state
        raw = self.rfile.read(int(self.headers['Content-Length']))
        try:
            if self.path.startswith('/api/v2/canister/'):
                envelope = decode_cbor(raw)
                content = envelope['content']
                assert len(envelope['sender_sig']) == 64
                if self.path.endswith('/query'):
                    assert content['request_type'] == 'query'
                    method = content['method_name']
                    name = {'get_network_economics_parameters': 'economics', 'list_known_neurons': 'known', 'list_neurons': state['mode']}[method]
                    state['reads'].append(method)
                    return self.respond(bytes.fromhex(FIXTURE['query_replies'][name]))
                if self.path.endswith('/read_state'):
                    assert content['request_type'] == 'read_state'
                    assert content['paths'][0][0] == b'request_status'
                    state['reads'].append('read_state')
                    # Fresh timestamp, invalid signature: exercise BLS refusal,
                    # rather than succeeding through a missing-time short circuit.
                    cert = cbor({'tree': [2, b'time', [3, unsigned_leb(time.time_ns())]], 'signature': bytes(48)})
                    return self.respond(cbor({'certificate': cert}))
                assert self.path.endswith('/call')
                state['submissions'].append(content)
                return self.respond(b'')
            body = json.loads(raw)
            state['reads'].append(self.path)
            network = {'blockchain': 'Internet Computer', 'network': '00000000000000020101'}
            if self.path == '/network/list':
                value = {'network_identifiers': [network]}
            elif self.path == '/account/balance':
                value = {'balances': [{'value': str(state['balance']), 'currency': {'symbol': 'ICP', 'decimals': 8}}]}
            elif self.path == '/construction/preprocess':
                assert body['operations'][0]['account']['address'] == FIXTURE['owner']
                value = {'options': {}}
            elif self.path == '/construction/metadata':
                value = {'suggested_fee': [{'value': '10000', 'currency': {'symbol': 'ICP', 'decimals': 8}}]}
            else:
                raise AssertionError(self.path)
            self.respond(json.dumps(value).encode(), 'application/json')
        except (AssertionError, KeyError, ValueError, IndexError) as error:
            state['errors'].append(str(error))
            self.send_error(500, 'fixture request refused')


def main():
    with tempfile.TemporaryDirectory(prefix='spectra-icp-staking-') as directory:
        state = {'mode': 'active', 'balance': 300010000, 'reads': [], 'submissions': [], 'errors': []}
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
        server.state = state
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            def run(*args, success=True):
                if args[:2] in (('staking', 'positions'), ('staking', 'build'), ('staking', 'recheck'), ('staking', 'repair'), ('send', 'sign')):
                    args = (*args, '--password-env', 'SPECTRA_PASSWORD')
                env = {**os.environ, 'PYTHONDONTWRITEBYTECODE': '1', 'SPECTRA_PRIVATE_KEY': '01' * 32, 'SPECTRA_PASSWORD': 'fixture-password',
                       'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl')}
                result = subprocess.run([BINARY, '--data-dir', directory, '--json', *args], text=True, capture_output=True, timeout=60, env=env)
                assert (result.returncode == 0) == success, (args, result.stdout, result.stderr, state['errors'])
                assert not state['errors'], state['errors']
                return json.loads(result.stdout)
            endpoint = f'http://127.0.0.1:{server.server_port}'
            wallet = run('wallet', 'import', '--chain', 'internet-computer', '--name', 'Neuron', '--private-key-env', 'SPECTRA_PRIVATE_KEY', '--no-password')['wallet']
            assert wallet['address'] == FIXTURE['owner']
            assert run('send', 'identity', '--from', 'Neuron')['address'] == FIXTURE['owner']
            run('endpoints', '--chain', 'internet-computer', '--api', 'icp-replica', '--capabilities', 'staking,verification,broadcast', '--add', endpoint)
            run('endpoints', '--chain', 'internet-computer', '--api', 'icp-rosetta', '--capabilities', 'balance,fee,verification,broadcast', '--add', endpoint)
            validators = run('staking', 'validators', '--chain', 'internet-computer')['validators']
            assert len(validators) == 1 and validators[0]['identifier'] == '1' and validators[0]['apy'] is None
            active = run('staking', 'positions', '--from', 'Neuron')['positions']
            assert len(active) == 1 and active[0]['id'] == '42' and active[0]['staked_amount_smallest_unit'] == '199990000'
            assert 'withdraw' not in active[0]['available_actions']
            base = ('staking', 'build', '--from', 'Neuron', '--action', 'stake', '--validator', '1', '--lockup-seconds', '600')
            run(*base, '--amount', '0.5', success=False)
            run('staking', 'build', '--from', 'Neuron', '--action', 'stake', '--validator', '1', '--amount', '2', success=False)
            run(*base[:-2], '--lockup-seconds', '0', '--amount', '2', success=False)
            run(*base[:-2], '--lockup-seconds', '63115201', '--amount', '2', success=False)
            state['balance'] = 1
            run(*base, '--amount', '2', success=False)
            state['balance'] = 300010000
            built = run(*base, '--amount', '2')['artifact']
            assert built['amount'] == '2' and built['staking']['action'] == 'stake'
            assert built['review']['staking']['network_fee'] == '0.0001' and built['review']['staking']['lockup_seconds'] == 600
            prepared = json.loads(built['prepared_details'])['IcpStaking']
            assert built['recipient'] == prepared['funding']['recipient']
            assert len(prepared['calls']) == 6 and sum(call['kind'] == 'Follow' for call in prepared['calls']) == 3
            nonce = int(prepared['neuron_nonce'])
            expected = hashlib.sha256(b'\x0cneuron-stake' + bytes.fromhex(prepared['controller_hex']) + nonce.to_bytes(8, 'big')).hexdigest()
            assert prepared['subaccount_hex'] == expected
            assert prepared['calls'][0]['kind'] == 'Fund' and prepared['calls'][1]['kind'] == 'Claim'
            signed = run('send', 'sign', built['id'], '--review-digest', built['review_digest'], '--endpoint', endpoint)['artifact']
            requests = json.loads(signed['signed_payload'])
            assert len({call['request_id'] for call in requests}) == len(requests)
            assert all(len(call['request_id']) == 64 and call['request_id'] != '0' * 64 for call in requests)
            reopened = run('send', 'inspect', signed['id'])['artifact']
            assert reopened['signed_payload'] == signed['signed_payload']
            broadcast = run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')['artifact']
            assert broadcast['attempts'][0]['outcome'] == 'Uncertain' and not state['submissions'], broadcast
            run('staking', 'recheck', '--id', signed['id'], success=False)
            run('staking', 'repair', '--id', signed['id'], success=False)
            assert run('send', 'inspect', signed['id'])['artifact']['signed_payload'] == signed['signed_payload']
            run('staking', 'build', '--from', 'Neuron', '--action', 'withdraw', '--position', '42', success=False)
            run('staking', 'build', '--from', 'Neuron', '--action', 'unstake', '--position', '43', success=False)
            unlock = run('staking', 'build', '--from', 'Neuron', '--action', 'unstake', '--position', '42')['artifact']
            assert unlock['amount'] == '1.9999' and unlock['review']['staking']['network_fee'] == '0'
            state['mode'] = 'ready'
            withdraw = run('staking', 'build', '--from', 'Neuron', '--action', 'withdraw', '--position', '42')['artifact']
            assert withdraw['amount'] == '1.9999'
            state['mode'] = 'active'
            claim = run('staking', 'build', '--from', 'Neuron', '--action', 'claim-rewards', '--position', '42')['artifact']
            assert claim['review']['staking']['reward_payout_is_delayed']
            state['mode'] = 'queued'
            queued = run('staking', 'positions', '--from', 'Neuron')['positions'][0]
            assert queued['claimable_rewards_smallest_unit'] == '0' and queued['pending_rewards_smallest_unit'] == '1000000'
            assert queued['rewards_unlock_time_unix'] == 1800604800
            assert 'claimRewards' not in queued['available_actions']
            assert not state['submissions']
            print('ICP staking: SDK Candid positions, owned authority, prepare/sign/reopen and forged-certificate refusal passed')
        finally:
            server.shutdown()
            worker.join()
            server.server_close()


if __name__ == '__main__':
    main()
