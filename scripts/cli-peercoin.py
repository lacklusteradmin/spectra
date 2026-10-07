#!/usr/bin/env python3
"""Peercoin native lifecycle, reward maturity and durable signing; loopback only."""
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
from decimal import Decimal
from urllib.parse import parse_qs, unquote, urlsplit

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else
                          pathlib.Path(__file__).resolve().parents[1] / 'target/debug/spectra').resolve())
seed = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
funds = {}
transactions = {}
requests = []
submitted = []
foreign_receipt = False
serial = 0


def hash256(raw):
    return hashlib.sha256(hashlib.sha256(raw).digest()).digest()


def compact(value):
    if value < 253:
        return bytes([value])
    width, marker = (2, 253) if value <= 65535 else (4, 254) if value <= 4294967295 else (8, 255)
    return bytes([marker]) + value.to_bytes(width, 'little')


def address_script(address):
    """Decode the independently verified output type without a wallet library."""
    if address.startswith(('pc1', 'tpc1')):
        alphabet = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l'
        encoded = [alphabet.index(c) for c in address.split('1', 1)[1]]
        assert encoded[0] in (0, 1)
        accumulator = bits = 0
        program = bytearray()
        for value in encoded[1:-6]:
            accumulator = (accumulator << 5) | value
            bits += 5
            if bits >= 8:
                bits -= 8
                program.append((accumulator >> bits) & 255)
        assert len(program) in (20, 32)
        return bytes([0 if encoded[0] == 0 else 0x51, len(program)]) + program
    alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
    number = 0
    for character in address:
        number = number * 58 + alphabet.index(character)
    raw = number.to_bytes((number.bit_length() + 7) // 8, 'big')
    raw = b'\0' * (len(address) - len(address.lstrip('1'))) + raw
    assert len(raw) == 25 and hash256(raw[:-4])[:4] == raw[-4:]
    if raw[0] in (55, 111):
        return b'\x76\xa9\x14' + raw[1:21] + b'\x88\xac'
    assert raw[0] in (117, 196)
    return b'\xa9\x14' + raw[1:21] + b'\x87'


def decode_transaction(raw_hex):
    raw = bytes.fromhex(raw_hex)
    assert raw[:4] == b'\x03\0\0\0', 'current Peercoin version must be 3'
    offset = 4
    witness = raw[4:6] == b'\0\1'
    if witness:
        offset += 2

    def read(size):
        nonlocal offset
        value = raw[offset:offset + size]
        offset += size
        assert len(value) == size
        return value

    def count():
        first = read(1)[0]
        return first if first < 253 else int.from_bytes(read({253: 2, 254: 4, 255: 8}[first]), 'little')

    start = offset
    inputs = []
    for _ in range(count()):
        outpoint = read(36)
        script = read(count())
        inputs.append((outpoint, script, read(4)))
    outputs = []
    for _ in range(count()):
        value = int.from_bytes(read(8), 'little')
        outputs.append((value, read(count())))
    end = offset
    stacks = [[read(count()) for _ in range(count())] for _ in inputs] if witness else [[] for _ in inputs]
    locktime = read(4)
    assert offset == len(raw) and locktime == b'\0' * 4
    txid = hash256(raw[:4] + raw[start:end] + locktime)[::-1].hex()
    return inputs, outputs, stacks, txid, len(raw)


def fund(chain, address, value, confirmations=1, reward=None, script=None, append=False):
    """Build the actual parent independently; the provider cannot invent its txid."""
    global serial
    serial += 1
    script = address_script(address) if script is None else script
    outputs = [(0, b'')] if reward == 'stake' else []
    outputs.append((value, script))
    if reward == 'coinbase':
        outpoint, unlocking = b'\0' * 32 + b'\xff' * 4, b'\x01\x01'
    else:
        outpoint = hashlib.sha256(str(serial).encode()).digest() + serial.to_bytes(4, 'little')
        unlocking = b'\x51'
    raw = b'\x03\0\0\0\x01' + outpoint + compact(len(unlocking)) + unlocking + b'\xff' * 4
    raw += compact(len(outputs))
    for amount, output in outputs:
        raw += amount.to_bytes(8, 'little') + compact(len(output)) + output
    raw += b'\0' * 4
    txid = hash256(raw)[::-1].hex()
    vout = len(outputs) - 1
    row = dict(txid=txid, vout=vout, value=str(value), confirmations=confirmations, height=1)
    transactions[txid] = dict(txid=txid, hex=raw.hex(), confirmations=confirmations,
                            blockHeight=1, blockTime=1700000000 + serial, fees='1920',
                            vin=[dict(addresses=[], value='0')],
                            vout=[dict(addresses=[address] if output else [], value=str(amount))
                                  for amount, output in outputs])
    if append:
        funds.setdefault((chain, address), []).append(row)
    else:
        funds[chain, address] = [row]
    return row


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        url = urlsplit(self.path)
        parts = unquote(url.path).split('/', 2)
        chain, path = parts[1], '/' + parts[2]
        assert chain in ('peercoin', 'peercoin-testnet'), self.path
        requests.append((chain, path))
        if path == '/api/v2':
            return self.reply(dict(blockbook=dict(coin='Peercoin Testnet' if chain.endswith('testnet') else 'Peercoin', decimals=6,
                                                  bestHeight=900000),
                                   backend=dict(chain='testnet' if chain.endswith('testnet') else 'livenet', blocks=900000)))
        if path.startswith('/api/v2/utxo/'):
            return self.reply(funds.get((chain, path.removeprefix('/api/v2/utxo/')), []))
        if path.startswith('/api/v2/address/'):
            address = path.removeprefix('/api/v2/address/')
            rows = funds.get((chain, address), [])
            if parse_qs(url.query).get('details') == ['txs']:
                return self.reply(dict(totalPages=1, transactions=[transactions[row['txid']] for row in rows]))
            return self.reply(dict(balance=str(sum(int(row['value']) for row in rows)),
                                   unconfirmedBalance='0', txs=len(rows), unconfirmedTxs=0))
        if path.startswith('/api/v2/tx/'):
            return self.reply(transactions[path.removeprefix('/api/v2/tx/')])
        if path.startswith('/api/v2/estimatefee/'):
            return self.reply(dict(result='0.01'))
        self.send_error(404, self.path)

    def do_POST(self):
        chain, path = self.path.lstrip('/').split('/', 1)
        assert path == 'api/v2/sendtx/', self.path
        payload = self.rfile.read(int(self.headers['Content-Length'])).decode()
        txid = decode_transaction(payload)[3]
        submitted.append((chain, payload, txid))
        self.reply(dict(result='ab' * 32 if foreign_receipt else txid))


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()
try:
    with tempfile.TemporaryDirectory(prefix='spectra-peercoin-') as directory:
        env = dict(os.environ, SPECTRA_SEED=seed, SPECTRA_PASSWORD='ppc-acceptance-password',
                   PPC_PRIVATE_KEY='0' * 63 + '1')
        env.setdefault('SPECTRA_LOOPBACK_ONLY', str(pathlib.Path(directory) / 'network.jsonl'))

        def run(*args, success=True, password='ppc-acceptance-password'):
            command_env = dict(env)
            if password is None:
                command_env.pop('SPECTRA_PASSWORD', None)
            else:
                command_env['SPECTRA_PASSWORD'] = password
            result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                    capture_output=True, text=True, env=command_env, timeout=60)
            assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        def endpoint(chain):
            return f'http://127.0.0.1:{server.server_port}/{chain}'

        def import_path(chain, name, path, protected=False):
            return run('wallet', 'import', '--chain', chain, '--name', name, '--path', path,
                       *([] if protected else ['--no-password']))['wallet']

        def address_at(chain, path):
            # Previewed in an empty store: here the account is already a wallet,
            # and a second import of it is refused.
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / 'vectors'), '--json',
                                     'wallet', 'import', '--chain', chain, '--path', path, '--preview'],
                                    capture_output=True, text=True, env=env, timeout=60)
            assert result.returncode == 0, (chain, path, result.stdout, result.stderr)
            return json.loads(result.stdout)['addresses'][0]

        catalog = run('chains', '--filter', 'peercoin', '--testnets')['chains']
        assert {chain['id'] for chain in catalog} == {'peercoin', 'peercoin-testnet'}, catalog
        assert all({'importPrivateKey', 'watchAddresses'} <= set(chain['setupMethods']) and
                   chain['supportsSeparateSigning'] and 'utxo' in chain['tags'] for chain in catalog), catalog
        for chain in ('peercoin', 'peercoin-testnet'):
            run('endpoints', '--chain', chain, '--api', 'blockbook', '--capabilities',
                'balance,history,utxo,fee,broadcast,verification', '--add', endpoint(chain))
            token = run('token', 'catalog', '--chain', chain)['tokens'][0]
            assert token['decimals'] == 6 and token['deployment_id'] == chain + ':native', token
            assert token['coingecko_id'] == ('peercoin' if chain == 'peercoin' else ''), token
            if chain == 'peercoin':
                assert token['coinpaprika_id'] == 'ppc-peercoin', token
            assert run('token', 'artwork', '--chain-id', chain)['artworkName'] == 'peercoin'
            assert run('send', 'amount', '--chain', chain, '--amount', '1.234567')['rawAmount'] == '1234567'
            run('send', 'amount', '--chain', chain, '--amount', '0.0000001', success=False)
        test_price = run('price', 'peercoin-testnet')
        assert test_price['priceUsd'] is None and test_price['price'] is None, test_price
        for path in ("m/44'/0'/0'/0/0", "m/84'/6'/0'/2/0", "m/87'/6'/0'/0/0"):
            run('wallet', 'import', '--chain', 'peercoin', '--name', 'invalid-path', '--path', path,
                '--no-password', success=False)
            run('wallet', 'show', 'invalid-path', success=False)

        for chain, coin in [('peercoin', 6), ('peercoin-testnet', 1)]:
            for purpose in (44, 49, 84, 86):
                name = f'{chain}-{purpose}'
                path = f"m/{purpose}'/{coin}'/0'"
                wallet = import_path(chain, name, path + '/0/0', protected=True)
                root = wallet['address']
                received = run('wallet', 'receive', name, password=None)['address']
                expected = address_at(chain, path + '/0/1')
                assert received == expected and root != received
                assert run('send', 'scan', '--chain', chain,
                           f'peercoin:{received}?amount=0.5&label=History')['address'] == received
                fund(chain, root, 1_000_000)
                fund(chain, received, 2_000_000)
                count, total = 2, 3_000_000
                if purpose == 84:
                    # Recovery must scan across nineteen empty addresses and
                    # spend discovered external and change branches together.
                    recovered = []
                    for branch, index, value in [(0, 7, 3_000_000), (0, 27, 4_000_000), (1, 3, 5_000_000)]:
                        address = address_at(chain, path + f'/{branch}/{index}')
                        fund(chain, address, value)
                        recovered.append(address)
                    discovered = run('pool', 'discover', name, password=None)
                    assert all(address in discovered['addresses'] for address in recovered), discovered
                    pool = run('pool', 'show', name)
                    assert pool['nextExternalIndex'] == 28 and pool['nextChangeIndex'] == 4, pool
                    count, total = 5, 15_000_000
                balance = run('balance', name, password=None)
                assert balance['smallestUnit'] == str(total) and balance['chain'] == chain, balance
                refreshed = run('refresh', '--wallet', name, '--endpoint', endpoint(chain), password=None)
                assert refreshed['refreshed'] == 1 and refreshed['errors'] == 0, refreshed
                history = run('history', name, '--save', '--endpoint', endpoint(chain), password=None)
                assert history['added'] >= count, (name, history, requests[-20:])
                records = run('txs', '--page', '--wallet', name)['page']['records']
                assert any(record['deploymentId'] == chain + ':native' and record['amount'] == '1' for record in records), records
                holding = chain + ':native'
                small_preview = run('send', 'preview', '--wallet', name, '--holding', holding,
                                    '--amount', '0.5', '--destination', received, password=None)['preview']
                assert small_preview['details']['selectedInputCount'] == 1, small_preview
                amount_units = total - 100_000
                amount = str(Decimal(amount_units) / 1_000_000)
                preview = run('send', 'preview', '--wallet', name, '--holding', holding,
                              '--amount', amount, '--destination', received, password=None)['preview']
                details = preview['details']
                assert Decimal(details['spendableBalance']) * 1_000_000 == total, preview
                assert details['selectedInputCount'] == count and details['usesChangeOutput'], preview
                built = run('send', 'build-owned', '--wallet', name, '--holding', holding,
                            '--amount', amount, '--destination', received, password=None)['artifact']
                prepared = json.loads(built['prepared_details'])['Peercoin']
                assert len(prepared['inputs']) == count, prepared
                assert run('send', 'inspect', built['id'])['artifact']['review_digest'] == built['review_digest']
                run('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                    '--endpoint', endpoint(chain), password='wrong', success=False)
                signed = run('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                             '--endpoint', endpoint(chain))['artifact']
                inputs, outputs, stacks, txid, raw_size = decode_transaction(signed['signed_payload'])
                assert len(inputs) == count and outputs[0] == (amount_units, address_script(received)), signed
                assert txid == signed['transaction_hash']
                assert total - sum(value for value, _ in outputs) == prepared['fee']
                assert prepared['fee'] >= max(1_000, raw_size * 10)
                assert prepared['fee'] == int(Decimal(preview['network_fee']) * 1_000_000), preview
                for (_, script, sequence), stack in zip(inputs, stacks):
                    assert sequence == b'\xff' * 4
                    if purpose == 44:
                        assert len(script) > 100 and not stack
                    elif purpose == 86:
                        assert len(stack) == 1 and len(stack[0]) == 64 and not script
                    else:
                        assert len(stack) == 2 and len(stack[1]) == 33
                        assert (not script) if purpose == 84 else (len(script) == 23 and script[:3] == b'\x16\0\x14')
                foreign_receipt = True
                uncertain = run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint(chain), '--yes')['artifact']
                assert uncertain['attempts'][-1]['outcome'] == 'Uncertain', uncertain
                assert uncertain['transaction_hash'] == txid
                foreign_receipt = False
                accepted = run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint(chain), '--yes')['artifact']
                assert accepted['attempts'][-1]['outcome'] == 'Accepted', accepted
                assert run('send', 'inspect', signed['id'])['artifact'] == accepted
                assert submitted[-1] == submitted[-2] == (chain, signed['signed_payload'], txid)

                # A provider changing the reviewed prevout value is refused
                # before a fresh signature or an irreversible submission.
                for source_input in prepared['inputs']:
                    funds[chain, source_input['source']['address']] = []
                row = fund(chain, root, 1_000_000)
                stale = run('send', 'build', '--from', name, '--to', received, '--amount', '0.5',
                            '--endpoint', endpoint(chain), password=None)['artifact']
                row['value'] = '1000001'
                run('send', 'sign', stale['id'], '--review-digest', stale['review_digest'],
                    '--endpoint', endpoint(chain), success=False)
                assert run('send', 'inspect', stale['id'])['artifact']['stage'] == 'Prepared'
                row['value'] = '1000000'
                run('send', 'build', '--from', name, '--to', received, '--amount', '0.009999',
                    '--endpoint', endpoint(chain), success=False)

            # Generated coinbase and coinstake outputs use independent maturity
            # rules on each network. Keep the genuine P2PK reward script.
            # Watched, the address cannot send; the key then upgrades that
            # wallet in place.
            raw_name = chain + '-key'
            sender = run('wallet', 'import', '--chain', chain, '--private-key-env', 'PPC_PRIVATE_KEY',
                         '--preview')['addresses'][0]
            watched = run('wallet', 'watch', '--chain', chain, '--name', raw_name, '--address', sender)['wallet']
            assert watched['address'] == sender
            run('send', 'build', '--from', raw_name, '--to', sender, '--amount', '0.5',
                '--endpoint', endpoint(chain), success=False)
            upgraded = run('wallet', 'import', '--chain', chain, '--name', 'ignored',
                           '--private-key-env', 'PPC_PRIVATE_KEY', '--no-password')
            assert upgraded['upgraded'] and upgraded['wallet']['id'] == watched['id'], upgraded
            maturity = 500 if chain == 'peercoin' else 60
            public = bytes.fromhex('0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798')
            reward_script = b'\x21' + public + b'\xac'
            mature = fund(chain, sender, 1_000_000, maturity, 'stake', reward_script)
            immature = fund(chain, sender, 2_000_000, maturity - 1, 'stake', reward_script, append=True)
            fund(chain, sender, 3_000_000, maturity - 1, 'coinbase', append=True)
            preview = run('send', 'preview', '--wallet', raw_name, '--holding', chain + ':native',
                          '--amount', '0.5', '--destination', sender)['preview']
            assert preview['details']['selectedInputCount'] == 1
            assert preview['details']['spendableBalance'] == '1', preview
            built = run('send', 'build', '--from', raw_name, '--to', sender, '--amount', '0.5',
                        '--endpoint', endpoint(chain))['artifact']
            prepared = json.loads(built['prepared_details'])['Peercoin']
            assert len(prepared['inputs']) == 1 and prepared['inputs'][0]['utxo'][0] == mature['txid']
            assert bytes(prepared['inputs'][0]['utxo'][3]) == reward_script
            assert prepared['inputs'][0]['utxo'][0] != immature['txid']
            signed = run('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                         '--endpoint', endpoint(chain))['artifact']
            reward_inputs, _, stacks, _, size = decode_transaction(signed['signed_payload'])
            assert len(reward_inputs) == 1 and len(reward_inputs[0][1]) < 76 and not stacks[0]
            assert prepared['fee'] >= max(1_000, size * 10)
            # The wallet's coins name the rewards still maturing, which its
            # total holds and its sends cannot spend.
            coins = run('wallet', 'coins', raw_name)['coins']
            assert coins['maturing'] == '5' and coins['tipHeight'] == 900000, coins
            spendable = {row['txid']: row['spendable'] for row in coins['outputs']}
            assert spendable[mature['txid']] and not spendable[immature['txid']], coins
            assert [row['amount'] for row in coins['outputs']] == ['3', '2', '1'], coins
            assert coins['addresses'][0]['address'] == sender and coins['addresses'][0]['balance'] == '6', coins

        # Catalog market identities drive the stored valuation without a live
        # quote request; production price-provider URLs stay outside this test.
        with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as db:
            db.execute('INSERT OR REPLACE INTO app_state_meta (key,value) VALUES (?,?)',
                       ('quotes', json.dumps(dict(prices={'peercoin:native': 0.4}))))
        stored = run('portfolio', '--stored')
        assert stored['assetPrecision']['byDeploymentId']['peercoin:native'] == 6
        assert stored['assetPrecision']['byDeploymentId']['peercoin-testnet:native'] == 6
        assert run('price', '--stored')['quotes']['prices']['peercoin:native'] == 0.4
        journal = pathlib.Path(env['SPECTRA_LOOPBACK_ONLY'])
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
    print('Peercoin main/test legacy/native/nested/Taproot recovery, history, maturity, quote, durable signing and local-hash submission acceptance passed')
finally:
    server.shutdown()
    server.server_close()
    worker.join()
