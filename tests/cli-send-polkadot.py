#!/usr/bin/env python3
"""Asset Hub staged sends against a loopback node, across processes: a fee
raised before broadcast submits nothing; the signed extrinsic is submitted
once, and resubmitted unchanged after its nonce is used; finalized success
and failure are found from the persisted cursor, again after a lost status
write. The quote, runtime and fund rules are tested in core."""
import hashlib
import http.server
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / 'target/debug/spectra').resolve())
SCALE = 10 ** 10
GENESIS = '0x68d56f15f85d3136970ec16946040bc1752654e906147f7e43e9d539d7c3de2f'
METADATA = '0x' + (ROOT / 'core/tests/fixtures/asset-hub-polkadot-metadata.scale').read_bytes().hex()
EVENTS_KEY = '0x26aa394eea5630e07c48ae0c9558cef780d41e5e16056765bc8461851072c9d7'
RECIPIENT = '13UVJyLnbVp9RBZYFwFGyDvVd1y27Tt8tkntv6Q7JVPhFsTB'


def compact(value):
    if value < 64: return bytes([value << 2])
    if value < 16384: return ((value << 2) | 1).to_bytes(2, 'little')
    if value < 2**30: return ((value << 2) | 2).to_bytes(4, 'little')
    length = max(4, (value.bit_length() + 7) // 8)
    return bytes([((length - 4) << 2) | 3]) + value.to_bytes(length, 'little')


def block_hash(number): return '0x' + number.to_bytes(32, 'big').hex()


def dispatch_event(succeeded):
    # Vec<EventRecord>: ApplyExtrinsic(0), RuntimeEvent::System,
    # ExtrinsicSuccess/Failed, DispatchEventInfo, empty topics.
    event = b'\x00' + bytes(4) + b'\x00' + bytes([int(not succeeded)])
    if not succeeded: event += b'\x02'  # DispatchError::BadOrigin
    return '0x' + (compact(1) + event + bytes(4) + b'\x00').hex()


live = dict(nonce=7, fee=SCALE // 1000, free=10*SCALE, finalized=100, blocks={}, succeeded={}, submitted=[])


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass

    def do_POST(self):
        call = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        name, params = call['method'], call['params']
        error = None
        if name == 'chain_getBlockHash':
            result = GENESIS if params == [0] else block_hash(params[0] if params else live['finalized'])
        elif name == 'chain_getFinalizedHead': result = block_hash(live['finalized'])
        elif name == 'chain_getHeader': result = {'number': hex(int(params[0], 16))}
        elif name == 'state_getRuntimeVersion': result = dict(specVersion=2_005_000, transactionVersion=15)
        elif name == 'state_getMetadata': result = METADATA
        elif name == 'system_accountNextIndex': result = live['nonce']
        elif name == 'payment_queryInfo': result = dict(partialFee=str(live['fee']))
        elif name == 'state_getStorage':
            if params[0] == EVENTS_KEY:
                result = dispatch_event(live['succeeded'][int(params[1], 16)])
            else:
                result = '0x' + (bytes(16) + live['free'].to_bytes(16, 'little') + bytes(48)).hex()
        elif name == 'author_submitExtrinsic':
            live['submitted'].append(params[0])
            result = '0x' + hashlib.blake2b(bytes.fromhex(params[0][2:]), digest_size=32).hexdigest()
        elif name == 'chain_getBlock':
            height = int(params[0], 16)
            result = dict(block=dict(header=dict(number=hex(height), parentHash=block_hash(height-1)),
                                     extrinsics=live['blocks'].get(height, [])))
        else:
            error = dict(code=-32601, message='unexpected fixture method: ' + name)
            result = None
        response = dict(jsonrpc='2.0', id=call['id'])
        response['error' if error else 'result'] = error or result
        raw = json.dumps(response).encode()
        self.send_response(200); self.send_header('Content-Length', str(len(raw)))
        self.end_headers(); self.wfile.write(raw)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
try:
    with tempfile.TemporaryDirectory(prefix='spectra-asset-hub-') as directory:
        db_path = pathlib.Path(directory) / 'spectra.sqlite'
        endpoint = f'http://127.0.0.1:{server.server_port}'
        env = {**os.environ,
               'SPECTRA_SEED': 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'}

        def run(*args, success=True):
            result = subprocess.run([BINARY, '--data-dir', directory, '--json', *args],
                                    capture_output=True, text=True, timeout=60, env=env)
            assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        def verified_through(id):
            with sqlite3.connect(db_path) as db:
                payload = db.execute('SELECT payload FROM send_artifacts WHERE id=?', (id,)).fetchone()[0]
                return json.loads(payload)['substrate_verified_through']

        def signed_send():
            prepared = run('send', 'build', '--from', 'DOT', '--to', RECIPIENT, '--amount', '1',
                           '--endpoint', endpoint)['artifact']
            return run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                       '--endpoint', endpoint)['artifact']

        def broadcast(id, success=True):
            return run('send', 'broadcast-signed', id, '--endpoint', endpoint, '--yes', success=success)

        def poll():
            return run('txs', '--poll-chain', 'polkadot')['changes']

        run('wallet', 'import', '--chain', 'polkadot', '--name', 'DOT', '--no-password')
        run('endpoints', '--chain', 'polkadot', '--api', 'substrate-json-rpc',
            '--capabilities', 'balance,fee,verification,broadcast', '--add', endpoint)

        signed = signed_send()
        raw = json.loads(signed['signed_payload'])['extrinsic_hex']
        assert not live['submitted'], 'signing submits nothing'
        # The signature caps no fee: a higher one at broadcast submits nothing.
        live['fee'] += 1; broadcast(signed['id'], success=False); live['fee'] -= 1
        assert not live['submitted']
        broadcast(signed['id'])
        live['nonce'] += 1
        broadcast(signed['id'])  # A saved rebroadcast does not demand the old nonce.
        assert live['submitted'] == [raw, raw], live['submitted']

        live['finalized'] = 102; live['blocks'][102] = [raw]; live['succeeded'][102] = True
        changes = poll()
        assert len(changes) == 1 and changes[0]['newStatus'] == 'confirmed', changes
        assert verified_through(signed['id']) is None, 'found block must remain retriable until status commits'
        # Lose the history-status write after the block was found: a fresh
        # process finds the same outcome again from its old cursor.
        with sqlite3.connect(db_path) as db:
            payload = json.loads(db.execute('SELECT payload FROM history_records WHERE id=?', (signed['id'],)).fetchone()[0])
            payload['status'] = 'pending'
            db.execute('UPDATE history_records SET payload=? WHERE id=?', (json.dumps(payload), signed['id']))
        recovered = poll()
        assert len(recovered) == 1 and recovered[0]['newStatus'] == 'confirmed', recovered

        failed = signed_send(); broadcast(failed['id'])
        failed_raw = json.loads(failed['signed_payload'])['extrinsic_hex']
        live['finalized'] = 170; live['blocks'][169] = [failed_raw]; live['succeeded'][169] = False
        assert poll() == []
        assert verified_through(failed['id']) == 166
        failed_changes = poll()
        assert len(failed_changes) == 1 and failed_changes[0]['newStatus'] == 'failed', failed_changes
        record = run('txs', '--record', failed['id'])['record']
        assert record['failureReason']['kind'] == 'executionFailed', record
        assert verified_through(failed['id']) == 166
        assert run('txs', '--maintenance')['chains'] == []
        print('polkadot staged sends: fee recheck at broadcast, one submission, unchanged rebroadcast, '
              'finalized success and failure recovered across processes')
finally:
    server.shutdown(); server.server_close(); worker.join()
