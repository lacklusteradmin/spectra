#!/usr/bin/env python3
"""Account recovery on every account UTXO network beside Litecoin and Peercoin.

For Bitcoin, Bitcoin Cash, Bitcoin SV, Dogecoin, Dash, Bitcoin Gold, Zcash,
Decred and Kaspa, each against a loopback indexer speaking that network's
own API: a password-protected phrase wallet hands out a fresh receive address
without its password; a gap scan finds used receive and change addresses past
the first, across a restart; the balance sums them; a send spends the largest
outputs it needs from several addresses, each signed with its own address's
key, change to the wallet's own address; and an output spent after review is
refused at signing. The account's public key, in its network's own
encoding, is refused where the phrase wallet already holds the account and
where it is a sibling network's key, and watched elsewhere finds the same
addresses and balance with no secret at all.
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
from urllib.parse import parse_qs, unquote, urlsplit

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else
                          pathlib.Path(__file__).resolve().parents[1] / 'target/debug/spectra').resolve())
seed = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
ZCASH_GENESIS = '00040fe8ec8471911baa1db1266ea15dd06b4a8a5c453883c000b031973dce08'
funds = {}

CHARSET = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l'


def kaspa_script(address):
    """A Kaspa address's Schnorr P2PK script: its 32-byte key behind a push."""
    values = [CHARSET.index(c) for c in address.split(':', 1)[1]][:-8]
    acc, bits, data = 0, 0, []
    for value in values:
        acc, bits = (acc << 5) | value, bits + 5
        while bits >= 8:
            bits -= 8
            data.append((acc >> bits) & 0xff)
    assert data[0] == 0 and len(data) >= 33, address
    return '20' + bytes(data[1:33]).hex() + 'ac'


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except BrokenPipeError:
            # A scan that has reached its gap drops the lookups still in flight.
            pass

    def do_GET(self):
        split = urlsplit(self.path)
        path, query = unquote(split.path), parse_qs(split.query)
        parts = path.strip('/').split('/')

        def held(address):
            return funds.get(address, [])

        def total(address):
            return sum(value for _, _, value in held(address))

        # Blockbook (Bitcoin Cash, Dash, Bitcoin Gold, Zcash).
        if path == '/api/v2/block-index/0':
            return self.reply({'blockHash': ZCASH_GENESIS})
        if path == '/api/v2':
            return self.reply({'backend': {'blocks': 3_500_000,
                                           'consensus': {'chaintip': '37a5165b', 'nextblock': '37a5165b'}}})
        if path.startswith('/api/v2/address/'):
            address = parts[3]
            return self.reply({'balance': str(total(address)), 'unconfirmedBalance': '0',
                               'txs': len(held(address)), 'unconfirmedTxs': 0, 'transactions': []})
        if path.startswith('/api/v2/utxo/'):
            return self.reply([{'txid': txid, 'vout': vout, 'value': str(value), 'confirmations': 6, 'height': 1}
                               for txid, vout, value in held(parts[3])])
        # Esplora (Bitcoin).
        if path == '/fee-estimates':
            return self.reply({'1': 3.0, '6': 2.0, '144': 1.0})
        if parts[0] == 'address' and len(parts) == 2:
            address = parts[1]
            stats = {'funded_txo_sum': total(address), 'spent_txo_sum': 0, 'tx_count': len(held(address))}
            return self.reply({'address': address, 'chain_stats': stats,
                               'mempool_stats': {'funded_txo_sum': 0, 'spent_txo_sum': 0, 'tx_count': 0}})
        if parts[0] == 'address' and parts[2:] == ['utxo']:
            return self.reply([{'txid': txid, 'vout': vout, 'value': value,
                                'status': {'confirmed': True, 'block_height': 1}}
                               for txid, vout, value in held(parts[1])])
        if parts[0] == 'address' and parts[2:3] == ['txs']:
            return self.reply([])
        # Whatsonchain (Bitcoin SV).
        if parts[0] == 'address' and parts[2:] == ['history']:
            return self.reply([{'tx_hash': txid, 'height': 1} for txid, _, _ in held(parts[1])])
        if parts[0] == 'address' and parts[2:] == ['balance']:
            return self.reply({'confirmed': total(parts[1]), 'unconfirmed': 0})
        if parts[0] == 'address' and parts[2:] == ['unspent']:
            return self.reply([{'tx_hash': txid, 'tx_pos': vout, 'value': value, 'height': 1}
                               for txid, vout, value in held(parts[1])])
        # Blockcypher (Dogecoin).
        if parts[0] == 'addrs' and parts[2:] == ['balance']:
            count = len(held(parts[1]))
            return self.reply({'balance': total(parts[1]), 'unconfirmed_balance': 0,
                               'n_tx': count, 'unconfirmed_n_tx': 0, 'final_n_tx': count})
        if parts[0] == 'addrs' and len(parts) == 2 and query.get('unspentOnly') == ['true']:
            return self.reply({'txrefs': [{'tx_hash': txid, 'tx_output_n': vout, 'value': value,
                                           'block_height': 1, 'confirmations': 6}
                                          for txid, vout, value in held(parts[1])]})
        # Insight (Decred).
        if parts[0] == 'addr' and len(parts) == 2:
            count = len(held(parts[1]))
            return self.reply({'txApperances': count, 'unconfirmedTxApperances': 0,
                               'balanceSat': total(parts[1]), 'balance': 0})
        if parts[0] == 'addr' and parts[2:] == ['utxo']:
            return self.reply([{'txid': txid, 'vout': vout, 'satoshis': value, 'confirmations': 6}
                               for txid, vout, value in held(parts[1])])
        # Kaspa's REST API.
        if parts[0] == 'addresses' and parts[2:] == ['transactions-count']:
            return self.reply({'total': len(held(parts[1]))})
        if parts[0] == 'addresses' and parts[2:] == ['balance']:
            return self.reply({'address': parts[1], 'balance': total(parts[1])})
        if parts[0] == 'addresses' and parts[2:] == ['utxos']:
            return self.reply([{'address': parts[1],
                                'outpoint': {'transactionId': txid, 'index': vout},
                                'utxoEntry': {'amount': str(value),
                                              'scriptPublicKey': {'version': 0,
                                                                  'scriptPublicKey': kaspa_script(parts[1])},
                                              'blockDaaScore': '1', 'isCoinbase': False}}
                               for txid, vout, value in held(parts[1])])
        self.send_error(404, path)


class Reader:
    def __init__(self, raw):
        self.raw, self.at = raw, 0

    def take(self, size):
        value = self.raw[self.at:self.at + size]
        assert len(value) == size
        self.at += size
        return value

    def compact(self):
        first = self.take(1)[0]
        return first if first < 253 else int.from_bytes(self.take({253: 2, 254: 4, 255: 8}[first]), 'little')


def bitcoin_format(raw_hex):
    """Inputs (outpoint) and outputs (value, script) of a Bitcoin-format transaction."""
    r = Reader(bytes.fromhex(raw_hex))
    r.take(4)
    witness = r.raw[4:6] == b'\0\1'
    if witness:
        r.take(2)
    inputs = []
    for _ in range(r.compact()):
        outpoint = r.take(36)
        r.take(r.compact())
        r.take(4)
        inputs.append((outpoint[:32][::-1].hex(), int.from_bytes(outpoint[32:], 'little')))
    outputs = [(int.from_bytes(r.take(8), 'little'), r.take(r.compact()).hex()) for _ in range(r.compact())]
    if witness:
        for _ in inputs:
            for _ in range(r.compact()):
                r.take(r.compact())
    r.take(4)
    assert r.at == len(r.raw)
    return inputs, outputs


def zcash(raw_hex):
    """Inputs and outputs of a transparent Zcash V5 transaction."""
    r = Reader(bytes.fromhex(raw_hex))
    assert int.from_bytes(r.take(4), 'little') == (1 << 31) | 5
    r.take(16)
    inputs = []
    for _ in range(r.compact()):
        outpoint = r.take(36)
        r.take(r.compact())
        r.take(4)
        inputs.append((outpoint[:32][::-1].hex(), int.from_bytes(outpoint[32:], 'little')))
    outputs = [(int.from_bytes(r.take(8), 'little'), r.take(r.compact()).hex()) for _ in range(r.compact())]
    assert r.take(3) == b'\0\0\0' and r.at == len(r.raw)
    return inputs, outputs


def decred(raw_hex):
    """Inputs and outputs of a full Decred transaction."""
    r = Reader(bytes.fromhex(raw_hex))
    assert r.take(4) == bytes([1, 0, 0, 0])
    inputs = []
    for _ in range(r.compact()):
        outpoint = r.take(36)
        r.take(1 + 4)
        inputs.append((outpoint[:32][::-1].hex(), int.from_bytes(outpoint[32:], 'little')))
    outputs = []
    for _ in range(r.compact()):
        value = int.from_bytes(r.take(8), 'little')
        assert r.take(2) == b'\0\0'
        outputs.append((value, r.take(r.compact()).hex()))
    r.take(8)
    assert r.compact() == len(inputs)
    for _ in inputs:
        r.take(16)
        r.take(r.compact())
    assert r.at == len(r.raw)
    return inputs, outputs


def kaspa(payload):
    body = json.loads(payload)['transaction']
    inputs = [(i['previousOutpoint']['transactionId'], i['previousOutpoint']['index']) for i in body['inputs']]
    outputs = [(int(o['value']), o['scriptPublicKey']['scriptPublicKey']) for o in body['outputs']]
    return inputs, outputs


# (chain, indexer API, account path, payload decoder, scale): amounts are
# the scale times 0.001, 0.003 and 0.005 coins; Dogecoin's fee is 0.01 DOGE
# a kilobyte, so its are a hundred times larger.
CHAINS = [
    ('bitcoin', 'esplora', "m/84'/0'/0'", bitcoin_format, 1),
    ('bitcoin-cash', 'blockbook', "m/44'/145'/0'", bitcoin_format, 1),
    ('bitcoin-sv', 'whatsonchain', "m/44'/236'/0'", bitcoin_format, 1),
    ('dogecoin', 'blockcypher', "m/44'/3'/0'", bitcoin_format, 100),
    ('dash', 'blockbook', "m/44'/5'/0'", bitcoin_format, 1),
    ('bitcoin-gold', 'blockbook', "m/44'/156'/0'", bitcoin_format, 1),
    ('zcash', 'blockbook', "m/44'/133'/0'", zcash, 1),
    ('decred', 'insight', "m/44'/42'/0'", decred, 1),
    ('kaspa', 'kaspa-rest', "m/44'/111111'/0'", kaspa, 1),
]

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()
try:
    endpoint = f'http://127.0.0.1:{server.server_port}'
    with tempfile.TemporaryDirectory(prefix='spectra-account-utxo-') as directory:
        env = {**os.environ, 'SPECTRA_SEED': seed, 'SPECTRA_PASSWORD': 'account-utxo-password'}
        env.setdefault('SPECTRA_LOOPBACK_ONLY', str(pathlib.Path(directory) / 'network.jsonl'))

        watching = str(pathlib.Path(directory) / 'watching')
        keys = json.loads((pathlib.Path(__file__).resolve().parents[1]
                           / 'core/tests/fixtures/account-keys.json').read_text())['vectors']

        def account_key(chain):
            """The abandon phrase's account 0 in the network's last-listed
            encoding: Bitcoin's zpub, Dash's drkp."""
            return [vector['xpub'] for vector in keys if vector['chain'] == chain][-1]

        def run(*args, success=True, password='account-utxo-password', data=directory):
            command_env = dict(env)
            if password is None:
                command_env.pop('SPECTRA_PASSWORD', None)
            result = subprocess.run([binary, '--data-dir', data, '--json', *args],
                                    capture_output=True, text=True, env=command_env, timeout=120)
            assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        def address_at(chain, path):
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / 'vectors'), '--json',
                                     'wallet', 'import', '--chain', chain, '--path', path, '--preview'],
                                    capture_output=True, text=True, env=env, timeout=60)
            assert result.returncode == 0, (chain, path, result.stdout, result.stderr)
            return json.loads(result.stdout)['addresses'][0]

        serial = 0
        for chain, api, account, decode, scale in CHAINS:
            run('endpoints', '--chain', chain, '--api', api,
                '--capabilities', 'balance,history,utxo,fee,broadcast,verification', '--add', endpoint)
            name = f'{chain} account'
            root = run('wallet', 'import', '--chain', chain, '--name', name,
                       '--path', account + '/0/0')['wallet']['address']
            # A fresh receive address, without the wallet's password: the
            # account's public key is stored.
            received = run('wallet', 'receive', name, password=None)['address']
            assert received == address_at(chain, account + '/0/1') != root, (chain, root, received)
            places = {(0, 0): root}
            for (branch, index), value in [((0, 0), 100_000 * scale), ((0, 7), 300_000 * scale),
                                           ((1, 3), 500_000 * scale)]:
                address = places.get((branch, index)) or address_at(chain, f'{account}/{branch}/{index}')
                places[(branch, index)] = address
                serial += 1
                funds[address] = [(f'{serial:064x}', 1, value)]
            discovered = run('pool', 'discover', name, password=None)
            assert {places[(0, 7)], places[(1, 3)]} <= set(discovered['addresses']), (chain, discovered)
            pool = run('pool', 'show', name)
            assert pool['nextExternalIndex'] == 8 and pool['nextChangeIndex'] == 4, (chain, pool)
            # Another process: what the scan found is stored.
            balance = run('balance', name, password=None)
            assert balance['smallestUnit'] == str(900_000 * scale), (chain, balance)

            # 0.006 (times the scale) needs the two largest outputs, on two
            # addresses.
            amount = f'{0.006 * scale:g}'
            artifact = run('send', 'build', '--from', name, '--to', received, '--amount', amount,
                           '--endpoint', endpoint)['artifact']
            prepared = json.loads(artifact['prepared_details'])['AccountTransfer']
            sources = [(i['source']['address'], i['source']['derivation_path']) for i in prepared['inputs']]
            assert sources == [(places[(1, 3)], account + '/1/3'), (places[(0, 7)], account + '/0/7')], (chain, sources)
            assert prepared['outputs'][0][1] == 600_000 * scale, prepared
            change = prepared['outputs'][1]
            assert change[1] == 200_000 * scale - prepared['fee'], prepared
            signed = run('send', 'sign', artifact['id'], '--review-digest', artifact['review_digest'],
                         '--endpoint', endpoint)['artifact']
            inputs, outputs = decode(signed['signed_payload'])
            assert inputs == [(funds[places[(1, 3)]][0][0], 1), (funds[places[(0, 7)]][0][0], 1)], (chain, inputs)
            assert outputs[0][0] == 600_000 * scale, (chain, outputs)
            assert outputs[1] == (change[1], bytes(change[0]).hex()), (chain, outputs)

            # An output spent after review is refused at signing.
            again = run('send', 'build', '--from', name, '--to', received, '--amount', f'{0.001 * scale:g}',
                        '--endpoint', endpoint)['artifact']
            spent = json.loads(again['prepared_details'])['AccountTransfer']['inputs'][0]['source']['address']
            held = funds.pop(spent)
            run('send', 'sign', again['id'], '--review-digest', again['review_digest'],
                '--endpoint', endpoint, success=False)
            funds[spent] = held

            # The account's public key: refused where the phrase wallet
            # holds the account, and as a sibling network's key; watched
            # elsewhere, it finds what the phrase wallet found.
            key = account_key(chain)
            run('wallet', 'watch', '--chain', chain, '--xpub', key, success=False)
            sibling = account_key(f'{chain}-testnet') if any(v['chain'] == f'{chain}-testnet' for v in keys) \
                else account_key('litecoin')
            run('wallet', 'watch', '--chain', chain, '--xpub', sibling, success=False, data=watching)
            run('endpoints', '--chain', chain, '--api', api,
                '--capabilities', 'balance,history,utxo,fee,broadcast,verification', '--add', endpoint,
                data=watching)
            watched = run('wallet', 'watch', '--chain', chain, '--name', name, '--xpub', f' {key} ',
                          password=None, data=watching)['wallet']
            assert watched['isWatchOnly'], (chain, watched)
            receive = run('wallet', 'receive', name, password=None, data=watching)['address']
            assert receive == received, (chain, receive)
            discovered = run('pool', 'discover', name, password=None, data=watching)
            assert {places[(0, 7)], places[(1, 3)]} <= set(discovered['addresses']), (chain, discovered)
            balance = run('balance', name, password=None, data=watching)
            assert balance['smallestUnit'] == str(900_000 * scale), (chain, balance)
            print(f'PASS {chain}: sealed receive rotation, gap scan across restart, account balance, '
                  'multi-address signing, stale-input refusal and a watched account key')
        journal = pathlib.Path(env['SPECTRA_LOOPBACK_ONLY'])
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
finally:
    server.shutdown()
    server.server_close()
    worker.join()
