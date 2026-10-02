#!/usr/bin/env python3
"""Check recipient output scripts through stored-wallet CLI build/sign on loopback."""
import hashlib
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
fixture = {'txid': '11' * 32}


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        if '/api/v2/utxo/' in self.path:
            result = [dict(txid=fixture['txid'], vout=0, value='10000000000', confirmations=1, height=1)]
        elif self.path.endswith('/unspent'):
            result = [dict(tx_hash=fixture['txid'], tx_pos=0, value=10000000000, height=1)]
        elif '?unspentOnly=true' in self.path:
            result = dict(txrefs=[dict(tx_hash=fixture['txid'], tx_output_n=0, value=10000000000, block_height=1)])
        else:
            raise AssertionError(self.path)
        body = json.dumps(result).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def base58(version, payload):
    alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
    data = bytes([version]) + payload
    data += hashlib.sha256(hashlib.sha256(data).digest()).digest()[:4]
    value = int.from_bytes(data, 'big')
    result = ''
    while value:
        value, digit = divmod(value, 58)
        result = alphabet[digit] + result
    return '1' * (len(data) - len(data.lstrip(b'\0'))) + result


def witness(hrp, program):
    # Independent BIP-173 encoder for version-zero HASH160 destinations.
    values = [0]
    accumulator = 0
    bits = 0
    for byte in program:
        accumulator = (accumulator << 8) | byte
        bits += 8
        while bits >= 5:
            bits -= 5
            values.append((accumulator >> bits) & 31)
    if bits:
        values.append((accumulator << (5 - bits)) & 31)
    checksum = 1
    for value in [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp] + values + [0] * 6:
        top = checksum >> 25
        checksum = ((checksum & 0x1ffffff) << 5) ^ value
        for bit, generator in enumerate([0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]):
            if (top >> bit) & 1:
                checksum ^= generator
    checksum ^= 1
    values += [(checksum >> (5 * (5 - i))) & 31 for i in range(6)]
    alphabet = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l'
    return hrp + '1' + ''.join(alphabet[v] for v in values)


def outputs(raw_hex):
    raw = bytes.fromhex(raw_hex)
    offset = 4

    def read(size):
        nonlocal offset
        value = raw[offset:offset + size]
        offset += size
        assert len(value) == size
        return value

    def compact():
        first = read(1)[0]
        return first if first < 253 else int.from_bytes(read({253: 2, 254: 4, 255: 8}[first]), 'little')

    for _ in range(compact()):
        read(36)
        read(compact())
        read(4)
    result = []
    for _ in range(compact()):
        value = int.from_bytes(read(8), 'little')
        result.append((value, read(compact()).hex()))
    assert read(4) == b'\0' * 4 and offset == len(raw)
    return result


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()
try:
    endpoint = f'http://127.0.0.1:{server.server_port}'
    with tempfile.TemporaryDirectory(prefix='spectra-utxo-output-') as directory:
        def run(*args, success=True):
            result = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True,
                                    env={**os.environ, 'SPECTRA_PASSWORD': 'utxo-fixture-password',
                                         'SPECTRA_SEED': 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'}, timeout=45)
            assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)

        counter = 0
        for chain, version, hrp in [
            ('bitcoin-cash', 0x05, None), ('bitcoin-cash-testnet', 0xc4, None),
            ('bitcoin-sv', 0x05, None), ('bitcoin-sv-testnet', 0xc4, None),
            ('dogecoin', 0x16, None), ('dogecoin-testnet', 0xc4, None),
            ('litecoin', 0x32, 'ltc'), ('litecoin-testnet', 0x3a, 'tltc'),
            ('dash', 0x10, None), ('dash-testnet', 0x13, None), ('bitcoin-gold', 0x17, 'btg'),
        ]:
            run('wallet', 'import', '--chain', chain, '--name', chain)
            digest = bytes([0x33] * 20)
            cases = [(base58(version, digest), 'a914' + digest.hex() + '87')]
            if hrp:
                cases.append((witness(hrp, digest), '0014' + digest.hex()))
            if chain == 'bitcoin-cash':
                cases.append(('bitcoincash:ppm2qsznhks23z7629mms6s4cwef74vcwvn0h829pq',
                              'a91476a04053bda0a88bda5177b86a15c3b29f55987387'))
            if chain == 'bitcoin-cash-testnet':
                cases.append(('bchtest:pr6m7j9njldwwzlg9v7v53unlr4jkmx6eyvwc0uz5t',
                              'a914f5bf48b397dae70be82b3cca4793f8eb2b6cdac987'))
            for recipient, script in cases:
                counter += 1
                fixture['txid'] = f'{counter:064x}'
                artifact = run('send', 'build', '--from', chain, '--to', recipient, '--amount', '0.001', '--endpoint', endpoint)['artifact']
                prepared = json.loads(artifact['prepared_details'])['FixedUtxo']
                assert bytes(prepared['recipient_script']).hex() == script, (chain, recipient, prepared)
                signed = run('send', 'sign', artifact['id'], '--review-digest', artifact['review_digest'], '--endpoint', endpoint)['artifact']
                assert outputs(signed['signed_payload'])[0] == (100000, script), (chain, recipient, signed)
                assert run('send', 'inspect', artifact['id'])['artifact']['signed_payload'] == signed['signed_payload']
    print('UTXO recipient output acceptance passed')
finally:
    server.shutdown()
    server.server_close()
    worker.join()
