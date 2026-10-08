#!/usr/bin/env python3
"""Asset Hub balance, fees, staged signing and finalized outcomes on loopback."""
import hashlib
from decimal import Decimal
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
CHAIN = sys.argv[2] if len(sys.argv) > 2 else 'polkadot'
FINNEY = CHAIN == 'bittensor'
SYMBOL = 'TAO' if FINNEY else 'DOT'
DECIMALS = 9 if FINNEY else 10
SCALE = 10 ** DECIMALS
ED = 500 if FINNEY else 100_000_000
GENESIS = '0x2f0555cc76fc2840a25a6ea3b9637146806f1f44b090c175ffde2a7e5ab36c03' if FINNEY else '0x68d56f15f85d3136970ec16946040bc1752654e906147f7e43e9d539d7c3de2f'
RELAY_GENESIS = '0x91b171bb158e2d3848fa23a9f1c25182fb8e20313b2c1eb49219da7a70ce90c3'
METADATA = '0x' + (ROOT / 'core/tests/fixtures' / ('bittensor-finney-metadata.scale' if FINNEY else 'asset-hub-polkadot-metadata.scale')).read_bytes().hex()
def units(value): return format((Decimal(value) / SCALE).normalize(), 'f')
EVENTS_KEY = '0x26aa394eea5630e07c48ae0c9558cef780d41e5e16056765bc8461851072c9d7'


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


def block_hash(number): return '0x' + number.to_bytes(32, 'big').hex()


def dispatch_event(succeeded):
    # Vec<EventRecord>: ApplyExtrinsic(0), RuntimeEvent::System,
    # ExtrinsicSuccess/Failed, DispatchEventInfo, empty topics.
    event = b'\x00' + bytes(4) + b'\x00' + bytes([int(not succeeded)])
    if not succeeded: event += b'\x02'  # DispatchError::BadOrigin
    return '0x' + (compact(1) + event + bytes(4) + b'\x00').hex()


live = dict(genesis=GENESIS, nonce=7, fee=SCALE // 1000, free=10*SCALE,
            reserved=0, frozen=0, spec=2_005_000, metadata=METADATA, finalized=100,
            blocks={}, succeeded={}, submitted=[], calls=[])


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass

    def do_POST(self):
        call = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        name, params = call['method'], call['params']
        live['calls'].append(name)
        error = None
        if name == 'chain_getBlockHash':
            result = live['genesis'] if params == [0] else block_hash(params[0] if params else live['finalized'])
        elif name == 'chain_getFinalizedHead': result = block_hash(live['finalized'])
        elif name == 'chain_getHeader': result = {'number': hex(int(params[0], 16))}
        elif name == 'state_getRuntimeVersion': result = dict(specVersion=live['spec'], transactionVersion=15)
        elif name == 'state_getMetadata': result = live['metadata']
        elif name == 'system_accountNextIndex': result = live['nonce']
        elif name == 'payment_queryInfo':
            raw = bytes.fromhex(params[0][2:]); size, body = read_compact(raw)
            assert size == len(body) and body[:2] == b'\x84\x00' and body[34] == 1
            _, after_nonce = read_compact(body[100:])
            extra_len = 2 if FINNEY else 3
            assert after_nonce[:extra_len] == bytes(extra_len), 'native fee + metadata hash mode must be encoded'
            assert after_nonce[extra_len:extra_len+3] == bytes([5 if FINNEY else 10, 3, 0]), 'runtime Balances transfer_keep_alive call'
            result = dict(partialFee=str(live['fee']))
        elif name == 'state_getStorage':
            if params[0] == EVENTS_KEY:
                height = int(params[1], 16)
                result = dispatch_event(live['succeeded'][height])
            else:
                width = 8 if FINNEY else 16
                record = bytes(16) + live['free'].to_bytes(width, 'little')
                record += live['reserved'].to_bytes(width, 'little') + live['frozen'].to_bytes(width, 'little') + bytes(16)
                result = '0x' + record.hex()
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
        env = {**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory)/'network.jsonl'),
               'SPECTRA_SEED': 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'}

        def run(*args, success=True):
            result = subprocess.run([BINARY, '--data-dir', directory, '--json', *args],
                                    capture_output=True, text=True, timeout=60, env=env)
            assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        def artifact(id):
            with sqlite3.connect(db_path) as db:
                return json.loads(db.execute('SELECT payload FROM send_artifacts WHERE id=?', (id,)).fetchone()[0])

        def build(amount='1'):
            return run('send', 'build', '--from', SYMBOL, '--to', recipient,
                       '--amount', amount, '--endpoint', endpoint)['artifact']

        def sign(prepared, success=True):
            return run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                       '--endpoint', endpoint, success=success)

        def broadcast(id, success=True):
            return run('send', 'broadcast-signed', id, '--endpoint', endpoint, '--yes', success=success)

        def poll():
            return run('txs', '--poll-chain', CHAIN)['changes']

        imported = run('wallet', 'import', '--chain', CHAIN, '--name', SYMBOL, '--no-password')['wallet']
        recipient = imported['address'] if FINNEY else '13UVJyLnbVp9RBZYFwFGyDvVd1y27Tt8tkntv6Q7JVPhFsTB'
        run('endpoints', '--chain', CHAIN, '--api', 'substrate-json-rpc',
            '--capabilities', 'balance,fee,verification,broadcast', '--add', endpoint)
        with sqlite3.connect(db_path) as db:
            wid, payload = db.execute('SELECT id,payload FROM wallets').fetchone()
            wallet = json.loads(payload)
            wallet['holdings'] = [dict(name=CHAIN, symbol=SYMBOL, coingeckoId=CHAIN, chainId=CHAIN,
                                      tokenStandard='Native', contractAddress=None, amount='10')]
            db.execute('UPDATE wallets SET payload=? WHERE id=?', (json.dumps(wallet), wid))
        owned = ('--wallet', SYMBOL, '--holding', CHAIN + ':native', '--destination', recipient)
        preview = run('send', 'preview', *owned, '--amount', '1')['preview']
        assert preview['network_fee'] == '0.001' and preview['details']['maxSendable'] == units(10*SCALE-ED-SCALE//1000), preview
        assert preview['details']['estimatedTransactionBytes'] > 145, preview
        quote = run('send', 'quote', *owned, '--amount', '1')['quote']
        assert quote['request']['fee_amount'] == '0.001', quote
        live['frozen'], live['reserved'] = 5*SCALE, SCALE
        locked = run('send', 'preview', *owned, '--amount', '1')['preview']
        assert locked['details']['maxSendable'] == units(6*SCALE-SCALE//1000), locked
        live['frozen'], live['reserved'] = 0, 0
        live['genesis'] = RELAY_GENESIS
        before = len(live['calls'])
        run('send', 'preview', *owned, '--amount', '1', success=False)
        assert set(live['calls'][before:]) == {'chain_getBlockHash'}, live['calls'][before:]
        build_result = run('send', 'build', '--from', SYMBOL, '--to', recipient, '--amount', '1',
                           '--endpoint', endpoint, success=False)
        assert 'wrong Substrate' in str(build_result), build_result
        live['genesis'] = GENESIS
        live['metadata'] = '0x00'
        run('send', 'preview', *owned, '--amount', '1', success=False)
        live['metadata'] = METADATA
        run('send', 'build', '--from', SYMBOL, '--to', recipient, '--amount', '10', '--endpoint', endpoint, success=False)
        prepared = build()
        content = json.loads(prepared['prepared_details'])['Substrate']
        assert content['runtime']['transfer_pallet'] == (5 if FINNEY else 10) and content['runtime']['existential_deposit'] == ED
        assert content['fee'] == SCALE//1000 and content['finalized_number'] == 100
        assert prepared['signing_payload_hex'] and not live['submitted']
        live['spec'] += 1; sign(prepared, success=False); live['spec'] -= 1
        live['nonce'] += 1; sign(prepared, success=False); live['nonce'] -= 1
        live['fee'] += 1; sign(prepared, success=False); live['fee'] -= 1
        live['free'] = content['amount'] + content['fee'] + ED - 1
        sign(prepared, success=False); live['free'] = 10*SCALE
        signed = sign(prepared)['artifact']
        assert not live['submitted'] and run('send', 'inspect', signed['id'])['artifact'] == signed
        raw = json.loads(signed['signed_payload'])['extrinsic_hex']
        assert signed['transaction_hash'] == '0x' + hashlib.blake2b(bytes.fromhex(raw[2:]), digest_size=32).hexdigest()
        live['fee'] += 1; broadcast(signed['id'], success=False); live['fee'] -= 1
        assert not live['submitted']
        broadcast(signed['id'])
        live['nonce'] += 1
        broadcast(signed['id'])  # A saved rebroadcast does not demand the old nonce.
        assert live['submitted'] == [raw, raw]
        live['finalized'] = 102; live['blocks'][102] = [raw]; live['succeeded'][102] = True
        calls_before_poll = len(live['calls'])
        changes = poll()
        assert len(changes) == 1 and changes[0]['newStatus'] == 'confirmed', (changes, live['calls'][calls_before_poll:], run('txs', '--record', signed['id']))
        assert artifact(signed['id'])['substrate_verified_through'] is None, 'found block must remain retriable until status commits'
        # Simulate losing the history-status write after finding the block.
        # A fresh process must find the same outcome again from its old cursor.
        with sqlite3.connect(db_path) as db:
            payload = json.loads(db.execute('SELECT payload FROM history_records WHERE id=?', (signed['id'],)).fetchone()[0])
            payload['status'] = 'pending'
            db.execute('UPDATE history_records SET payload=? WHERE id=?', (json.dumps(payload), signed['id']))
        recovered = poll()
        assert len(recovered) == 1 and recovered[0]['newStatus'] == 'confirmed', recovered
        failed_prepared = build(); failed_signed = sign(failed_prepared)['artifact']; broadcast(failed_signed['id'])
        failed_raw = json.loads(failed_signed['signed_payload'])['extrinsic_hex']
        live['finalized'] = 170; live['blocks'][169] = [failed_raw]; live['succeeded'][169] = False
        assert poll() == []
        assert artifact(failed_signed['id'])['substrate_verified_through'] == 166
        failed_changes = poll()
        assert len(failed_changes) == 1 and failed_changes[0]['newStatus'] == 'failed', failed_changes
        failed_record = run('txs', '--record', failed_signed['id'])['record']
        assert failed_record['failureReason']['kind'] == 'executionFailed', failed_record
        assert artifact(failed_signed['id'])['substrate_verified_through'] == 166
        assert run('txs', '--maintenance')['chains'] == []
        # A wallet on a soft junction path signs as polkadot.js's account
        # for it, with the expanded key no seed gives.
        vector = next(v for v in json.loads((ROOT / 'core/tests/fixtures/substrate-paths.json').read_text())['vectors']
                      if v['path'] == '//polkadot//0/1')
        junction = run('wallet', 'import', '--chain', CHAIN, '--name', 'Junction', '--path', '//polkadot//0/1',
                       '--no-password')['wallet']
        assert junction['address'] == vector['substrate' if FINNEY else 'polkadot'], junction
        with sqlite3.connect(db_path) as db:
            wid, payload = db.execute("SELECT id,payload FROM wallets WHERE json_extract(payload,'$.name')='Junction'").fetchone()
            wallet = json.loads(payload); wallet['holdings'] = [dict(name=CHAIN, symbol=SYMBOL, coingeckoId=CHAIN, chainId=CHAIN,
                                      tokenStandard='Native', contractAddress=None, amount='10')]
            db.execute('UPDATE wallets SET payload=? WHERE id=?', (json.dumps(wallet), wid))
        junction_prepared = run('send', 'build', '--from', 'Junction', '--to', recipient, '--amount', '1',
                                '--endpoint', endpoint)['artifact']
        junction_raw = json.loads(sign(junction_prepared)['artifact']['signed_payload'])['extrinsic_hex']
        _, junction_body = read_compact(bytes.fromhex(junction_raw[2:]))
        assert junction_body[2:34].hex() == vector['publicKey'], junction_body[2:34].hex()
        assert all(url.startswith('http://127.0.0.1:') for url in [endpoint])
        print(CHAIN + ' offline CLI: identity, metadata, keep-alive/freeze budgets, fees, staged sr25519 on root and junction-path keys and finalized success/failure passed')
finally:
    server.shutdown(); server.server_close(); worker.join()
