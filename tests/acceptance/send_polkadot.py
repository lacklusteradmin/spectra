#!/usr/bin/env python3
"""Asset Hub staged sends against a loopback node, across processes: a fee
raised before broadcast submits nothing; the signed extrinsic is submitted
once, and resubmitted unchanged after its nonce is used; finalized success
and failure are found from the persisted cursor, again after a lost status
write. The quote, runtime and fund rules are tested in core."""
import hashlib
import json
import sqlite3

from harness import ABANDON, FIXTURES, JsonRpc, RpcError, Spectra, scratch, serve

SCALE = 10 ** 10
GENESIS = '0x68d56f15f85d3136970ec16946040bc1752654e906147f7e43e9d539d7c3de2f'
METADATA = '0x' + (FIXTURES / 'asset-hub-polkadot-metadata.scale').read_bytes().hex()
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


class Node(JsonRpc):
    def call(self, name, params):
        live = self.state
        if name == 'chain_getBlockHash':
            return GENESIS if params == [0] else block_hash(params[0] if params else live['finalized'])
        if name == 'chain_getFinalizedHead': return block_hash(live['finalized'])
        if name == 'chain_getHeader': return {'number': hex(int(params[0], 16))}
        if name == 'state_getRuntimeVersion': return dict(specVersion=2_005_000, transactionVersion=15)
        if name == 'state_getMetadata': return METADATA
        if name == 'system_accountNextIndex': return live['nonce']
        if name == 'payment_queryInfo': return dict(partialFee=str(live['fee']))
        if name == 'state_getStorage':
            if params[0] == EVENTS_KEY:
                return dispatch_event(live['succeeded'][int(params[1], 16)])
            return '0x' + (bytes(16) + live['free'].to_bytes(16, 'little') + bytes(48)).hex()
        if name == 'author_submitExtrinsic':
            live['submitted'].append(params[0])
            return '0x' + hashlib.blake2b(bytes.fromhex(params[0][2:]), digest_size=32).hexdigest()
        if name == 'chain_getBlock':
            height = int(params[0], 16)
            return dict(block=dict(header=dict(number=hex(height), parentHash=block_hash(height-1)),
                                   extrinsics=live['blocks'].get(height, [])))
        raise RpcError(-32601, 'unexpected fixture method: ' + name)


with serve(Node, nonce=7, fee=SCALE // 1000, free=10*SCALE, finalized=100, blocks={}, succeeded={},
           submitted=[]) as node, scratch('asset-hub') as directory:
    live, endpoint = node.state, node.url
    db_path = directory / 'spectra.sqlite'
    spectra = Spectra(directory, SPECTRA_SEED=ABANDON)
    run = spectra.run

    def verified_through(id):
        with sqlite3.connect(db_path) as db:
            payload = db.execute('SELECT payload FROM send_artifacts WHERE id=?', (id,)).fetchone()[0]
            return json.loads(payload)['substrate_verified_through']

    def signed_send():
        prepared = run('send', 'build', 'DOT', '--to', RECIPIENT, '--amount', '1',
                       '--endpoint', endpoint)['artifact']
        return run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                   '--endpoint', endpoint)['artifact']

    def broadcast(id, success=True):
        arguments = ('send', 'broadcast-signed', id, '--endpoint', endpoint, '--yes')
        return spectra.run(*arguments) if success else spectra.fail(*arguments)

    def poll():
        return run('txs', '--poll-chain', 'polkadot')['changes']

    run('wallet', 'import', '--chain', 'polkadot', '--name', 'DOT', '--no-password')
    run('endpoints', '--chain', 'polkadot', '--api', 'substrate-json-rpc',
        '--capabilities', 'balance,fee,verification,broadcast', '--add', endpoint)

    signed = signed_send()
    raw = signed['signed_payload']
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
    spectra.lose_status(signed['id'])
    recovered = poll()
    assert len(recovered) == 1 and recovered[0]['newStatus'] == 'confirmed', recovered

    failed = signed_send(); broadcast(failed['id'])
    failed_raw = failed['signed_payload']
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
