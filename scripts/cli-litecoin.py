#!/usr/bin/env python3
"""LTC transparent wallet recovery and durable signing against a loopback node."""
import hashlib
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
from decimal import Decimal
from urllib.parse import unquote, urlsplit

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else
                          pathlib.Path(__file__).resolve().parents[1] / 'target/debug/spectra').resolve())
seed = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
funds = {}
queries = []


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        path = unquote(urlsplit(self.path).path)
        if path.startswith('/api/v2/utxo/'):
            address = path.removeprefix('/api/v2/utxo/')
            queries.append(('utxo', address))
            result = funds.get(address, [])
        elif path.startswith('/api/v2/address/'):
            address = path.removeprefix('/api/v2/address/')
            queries.append(('address', address))
            rows = funds.get(address, [])
            result = dict(balance=str(sum(int(r['value']) for r in rows if r['confirmations'] > 0)),
                          unconfirmedBalance='0', txs=len(rows), unconfirmedTxs=0)
        elif path == '/api/v2':
            result = {'blockbook': {'bestHeight': 100}, 'backend': {'blocks': 100}}
        elif path == '/api/v2/estimatefee/3':
            result = {'result': '0.00001'}  # 1 litoshi/vB
        else:
            self.send_error(404, path)
            return
        body = json.dumps(result).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def decode_transaction(raw_hex):
    """Independent wire parser, including stripped serialization for txid."""
    raw = bytes.fromhex(raw_hex)
    offset = 4
    version = raw[:4]
    witness = raw[4:6] == b'\0\1'
    if witness:
        offset += 2

    def read(size):
        nonlocal offset
        value = raw[offset:offset + size]
        offset += size
        assert len(value) == size
        return value

    def compact():
        first = read(1)[0]
        return first if first < 253 else int.from_bytes(read({253: 2, 254: 4, 255: 8}[first]), 'little')

    body_start = offset
    inputs = []
    for _ in range(compact()):
        outpoint = read(36)
        script = read(compact())
        sequence = read(4)
        inputs.append((outpoint, script, sequence))
    outputs = []
    for _ in range(compact()):
        value = int.from_bytes(read(8), 'little')
        outputs.append((value, read(compact())))
    body_end = offset
    stacks = [[read(compact()) for _ in range(compact())] for _ in inputs] if witness else [[] for _ in inputs]
    locktime = read(4)
    assert offset == len(raw) and locktime == b'\0' * 4
    stripped = version + raw[body_start:body_end] + locktime
    txid = hashlib.sha256(hashlib.sha256(stripped).digest()).digest()[::-1].hex()
    weight = len(stripped) * 3 + len(raw)
    return inputs, outputs, stacks, txid, (weight + 3) // 4


def fund(address, value, serial, confirmed=True):
    funds[address] = [dict(txid=f'{serial:064x}', vout=0, value=str(value),
                           confirmations=1 if confirmed else 0, height=1 if confirmed else 0)]


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()
try:
    endpoint = f'http://127.0.0.1:{server.server_port}'
    with tempfile.TemporaryDirectory(prefix='spectra-litecoin-') as directory:
        env = {**os.environ, 'SPECTRA_SEED': seed, 'SPECTRA_PASSWORD': 'ltc-acceptance-password'}
        # Also enforce the network boundary when run independently of the main gate.
        env.setdefault('SPECTRA_LOOPBACK_ONLY', str(pathlib.Path(directory) / 'network.jsonl'))

        def run(*args, success=True, password='ltc-acceptance-password'):
            command_env = dict(env)
            if password is None:
                command_env.pop('SPECTRA_PASSWORD', None)
            else:
                command_env['SPECTRA_PASSWORD'] = password
            result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                    capture_output=True, text=True, env=command_env, timeout=60)
            assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        def import_path(chain, name, path, protected=False):
            flags = [] if protected else ['--no-password']
            return run('wallet', 'import', '--chain', chain, '--name', name, '--path', path, *flags)['wallet']

        def address_at(chain, path):
            # Previewed in an empty store: here the account is already a wallet,
            # and a second import of it is refused.
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / 'vectors'), '--json',
                                     'wallet', 'import', '--chain', chain, '--path', path, '--preview'],
                                    capture_output=True, text=True, env=env, timeout=60)
            assert result.returncode == 0, (chain, path, result.stdout, result.stderr)
            return json.loads(result.stdout)['addresses'][0]

        for chain in ['litecoin', 'litecoin-testnet']:
            run('endpoints', '--chain', chain, '--api', 'blockbook',
                '--capabilities', 'balance,history,utxo,fee,broadcast,verification', '--add', endpoint)

        for path in ["m/84'/0'/0'/0/0", "m/84'/2'/0'/2/0", "m/49'/2'/0'/0'/0"]:
            run('wallet', 'import', '--chain', 'litecoin', '--name', 'invalid-path',
                '--path', path, '--no-password', success=False)
            run('wallet', 'show', 'invalid-path', success=False)

        serial = 100
        for chain, coin in [('litecoin', 2), ('litecoin-testnet', 1)]:
            for purpose in [44, 49, 84]:
                name = f'{chain}-{purpose}'
                path = f"m/{purpose}'/{coin}'/0'"
                wallet = import_path(chain, name, path + '/0/0', protected=True)
                root = wallet['address']
                received = run('wallet', 'receive', name, password=None)['address']
                expected = address_at(chain, path + '/0/1')
                assert received == expected and received != root, (chain, purpose, root, received)
                if purpose == 84:
                    assert root.startswith('tltc1q' if coin == 1 else 'ltc1q'), root
                serial += 1
                fund(root, 100_000, serial)
                serial += 1
                fund(received, 200_000, serial)
                if purpose == 84:
                    # Nineteen empty addresses between indices 7 and 27 must not
                    # terminate recovery, and change-branch funds must be found.
                    recovered = []
                    for branch, index, value in [(0, 7, 300_000), (0, 27, 400_000), (1, 3, 500_000)]:
                        address = address_at(chain, path + f'/{branch}/{index}')
                        serial += 1
                        fund(address, value, serial)
                        recovered.append(address)
                    discovered = run('pool', 'discover', name, password=None)
                    assert all(address in discovered['addresses'] for address in recovered), discovered
                    assert discovered['chain'] == chain, discovered
                    pool = run('pool', 'show', name)
                    assert pool['nextExternalIndex'] == 28 and pool['nextChangeIndex'] == 4, pool
                    # Each address in its place, with what it holds; every
                    # output one block deep at a tip of 100.
                    coins = run('wallet', 'coins', name, password=None)['coins']
                    places = {row['address']: (row['branch'], row['index'], row['balance'], row['used'])
                              for row in coins['addresses']}
                    assert places[root] == ('receive', 0, '0.001', True), coins
                    assert places[recovered[0]] == ('receive', 7, '0.003', True), coins
                    assert places[recovered[2]] == ('change', 3, '0.005', True), coins
                    # The reserved address has received, so the next is past
                    # every used one.
                    assert coins['nextReceiveAddress'] == address_at(chain, path + '/0/28'), coins
                    assert len(coins['outputs']) == 5 and all(o['confirmations'] == 100 for o in coins['outputs']), coins
                    assert coins['outputs'][0]['amount'] == '0.005' and coins['maturing'] == '0', coins

                balance = run('balance', name, password=None)
                total = 1_500_000 if purpose == 84 else 300_000
                assert balance['smallestUnit'] == str(total) and balance['chain'] == chain, balance
                holding = f'{chain}:native'
                preview = run('send', 'preview', '--wallet', name, '--holding', holding,
                              '--amount', '0.001', '--destination', received)['preview']
                details = preview['details']
                count = 5 if purpose == 84 else 2
                assert Decimal(details['spendableBalance']) * 100_000_000 == total, preview
                assert details['selectedInputCount'] == count and details['usesChangeOutput'], preview

                built = run('send', 'build-owned', '--wallet', name, '--holding', holding,
                            '--amount', '0.001', '--destination', received, password=None)['artifact']
                prepared = json.loads(built['prepared_details'])['Litecoin']
                assert len(prepared['inputs']) == count, prepared
                assert sum(i['utxo'][2] for i in prepared['inputs']) == total, prepared
                # Every CLI invocation reopens the store; review/sign must use
                # exactly the persisted paths and the persisted transaction.
                assert run('send', 'inspect', built['id'])['artifact']['review_digest'] == built['review_digest']
                signed = run('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                             '--endpoint', endpoint)['artifact']
                inputs, outputs, stacks, txid, vsize = decode_transaction(signed['signed_payload'])
                assert len(inputs) == count and outputs[0][0] == 100_000, signed
                assert txid == signed['transaction_hash'], (txid, signed)
                assert total - sum(value for value, _ in outputs) == prepared['fee'], prepared
                assert prepared['fee'] >= vsize and prepared['fee'] == int(Decimal(preview['network_fee']) * 100_000_000), preview
                for (_, script, _), stack in zip(inputs, stacks):
                    if purpose == 44:
                        assert len(script) > 100 and not stack, (script.hex(), stack)
                    else:
                        assert len(stack) == 2 and len(stack[1]) == 33, stack
                        assert (not script) if purpose == 84 else (len(script) == 23 and script[:3] == b'\x16\x00\x14'), script.hex()

                # Changed input values cannot silently replace a reviewed spend.
                serial += 1
                fund(root, 100_000, serial)
                stale = run('send', 'build', '--from', name, '--to', received,
                            '--amount', '0.001', '--endpoint', endpoint)['artifact']
                funds[root][0]['value'] = '100001'
                run('send', 'sign', stale['id'], '--review-digest', stale['review_digest'],
                    '--endpoint', endpoint, success=False)
                assert run('send', 'inspect', stale['id'])['artifact']['stage'] == 'Prepared'

        # Building a protected wallet is secret-free. Unlocking occurs only
        # when the reviewed transaction is signed, after reopening its store.
        protected = import_path('litecoin', 'protected', "m/84'/2'/1'/0/0", protected=True)
        serial += 1
        fund(protected['address'], 100_000, serial)
        built = run('send', 'build', '--from', 'protected', '--to', protected['address'],
                    '--amount', '0.0001', '--endpoint', endpoint, password=None)['artifact']
        run('send', 'sign', built['id'], '--review-digest', built['review_digest'],
            '--endpoint', endpoint, password='wrong', success=False)
        signed = run('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                     '--endpoint', endpoint)['artifact']
        assert decode_transaction(signed['signed_payload'])[3] == signed['transaction_hash']
        before = len(queries)
        run('send', 'build', '--from', 'protected', '--to', 'ltcmweb1unsupported',
            '--amount', '0.0001', '--endpoint', endpoint, success=False)
        assert len(queries) == before, 'a malformed MWEB address is refused before provider reads'
        journal = pathlib.Path(env['SPECTRA_LOOPBACK_ONLY'])
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
    print('Litecoin legacy/native/nested recovery, balance, quote and durable signing acceptance passed')
finally:
    server.shutdown()
    server.server_close()
    worker.join()
