#!/usr/bin/env python3
"""A 2-of-3 TON multisig v2 against a loopback toncenter that keeps the
multisig's and its orders' state, across two data directories.

The multisig (core/tests/fixtures/ton-multisig.json, its code the
contract's build) is watched. An order is built in one data directory; the
first signer's W5 wallet proposes it to the multisig, which deploys it at
the address its number derives; the order travels to the other directory,
which reads its approvals from the network, and the second signer's wallet
approves it there, which executes it. Every wallet message is parsed and
its signature verified here. Refused on the way: a wallet that is no
signer, an approval twice, an approval before the proposal is processed,
an order the multisig cannot pay, an executed order, and `multisig
submit`, which approvals make needless.
"""
import base64
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
import urllib.parse

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / 'target/debug/spectra').resolve())
FIXTURE = json.loads((ROOT / 'core/tests/fixtures/ton-multisig.json').read_text())
MNEMONICS = json.loads((ROOT / 'core/tests/fixtures/ton-mnemonics.json').read_text())['mnemonics']
CODE = (ROOT / 'core/tests/fixtures/ton-multisig-code.boc').read_bytes()
ORDER_CODE_HASH = bytes.fromhex(FIXTURE['order_code_hash'])
ZERO_STATE = {'workchain': -1, 'seqno': 0, 'root_hash': 'F6OpKZKqvqeFp6CQmFomXNMfMj2EnaUSOXN+Mh+wVWk=',
              'file_hash': 'XplPz01CXAps5qeSWUtxcyBfdAo5zVb1N979KLSKD24='}
TON = 10 ** 9
DESTINATION = '0:' + '55' * 32


class Cell:
    def __init__(self, bits='', refs=(), library=False):
        self.bits, self.refs, self.library = bits, list(refs), library

    def depth(self):
        return max((ref.depth() + 1 for ref in self.refs), default=0)

    def padded(self):
        bits = self.bits
        if len(bits) % 8:
            bits += '1' + '0' * (7 - len(bits) % 8)
        return bytes(int(bits[i:i + 8], 2) for i in range(0, len(bits), 8))

    def descriptors(self):
        return bytes([len(self.refs) + (8 if self.library else 0), len(self.bits) // 8 + (len(self.bits) + 7) // 8])

    def hash(self):
        return hashlib.sha256(self.descriptors() + self.padded()
                              + b''.join(ref.depth().to_bytes(2, 'big') for ref in self.refs)
                              + b''.join(ref.hash() for ref in self.refs)).digest()


def bits_of(value, width): return format(value, f'0{width}b') if width else ''
def address_bits(raw):
    workchain, account = raw.split(':')
    return '100' + bits_of(int(workchain) & 0xff, 8) + bits_of(int(account, 16), 256)


def parse_boc(data):
    assert data[:4] == bytes.fromhex('b5ee9c72')
    flags, size, offset = data[4], data[4] & 7, data[5]
    read = lambda at, n: int.from_bytes(data[at:at + n], 'big')
    count, at = read(6, size), 6 + size
    roots, at = read(at, size), at + size
    at += size + offset
    root, at = read(at, size), at + size * roots
    if flags & 0x80: at += count * offset
    rows = []
    for _ in range(count):
        d1, d2 = data[at], data[at + 1]
        body = data[at + 2:at + 2 + (d2 + 1) // 2]
        at += 2 + (d2 + 1) // 2
        refs = [read(at + i * size, size) for i in range(d1 & 7)]
        at += size * (d1 & 7)
        bits = ''.join(format(byte, '08b') for byte in body)
        if d2 % 2: bits = bits[:bits.rindex('1')]
        rows.append((bits, refs, bool(d1 & 8)))
    def build(i):
        bits, refs, library = rows[i]
        return Cell(bits, [build(r) for r in refs], library)
    return build(root)


def to_boc(root):
    rows = []
    def flatten(cell):
        index = len(rows); rows.append(None)
        refs = [flatten(ref) for ref in cell.refs]
        rows[index] = cell.descriptors() + cell.padded() + bytes(refs)
        return index
    flatten(root)
    body = b''.join(rows)
    return (bytes.fromhex('b5ee9c72') + bytes([1, 2, len(rows), 1, 0]) + len(body).to_bytes(2, 'big')
            + b'\0' + body)


class Reader:
    def __init__(self, cell): self.cell, self.at, self.ref = cell, 0, 0
    def uint(self, width):
        value = int(self.cell.bits[self.at:self.at + width] or '0', 2)
        assert self.at + width <= len(self.cell.bits); self.at += width
        return value
    def reference(self):
        self.ref += 1; return self.cell.refs[self.ref - 1]
    def address(self):
        assert self.uint(3) == 4
        workchain = self.uint(8)
        return f'{workchain - 256 if workchain > 127 else workchain}:{self.uint(256):064x}'
    def coins(self): return self.uint(8 * self.uint(4))
    def rest(self): return len(self.cell.bits) - self.at


q = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
d = -121665 * pow(121666, -1, q) % q
I = pow(2, (q - 1) // 4, q)


def recover_x(y, sign):
    x2 = (y * y - 1) * pow(d * y * y + 1, -1, q)
    x = pow(x2, (q + 3) // 8, q)
    if (x * x - x2) % q: x = x * I % q
    return q - x if x % 2 != sign else x


def edwards_add(a, b):
    (x1, y1), (x2, y2) = a, b
    return ((x1 * y2 + x2 * y1) * pow(1 + d * x1 * x2 * y1 * y2, -1, q) % q,
            (y1 * y2 + x1 * x2) * pow(1 - d * x1 * x2 * y1 * y2, -1, q) % q)


def scalar(e, point):
    result = (0, 1)
    while e:
        if e & 1: result = edwards_add(result, point)
        point, e = edwards_add(point, point), e >> 1
    return result


def decode_point(data):
    y = int.from_bytes(data, 'little') & ((1 << 255) - 1)
    return recover_x(y, data[31] >> 7), y


BASE = (recover_x(4 * pow(5, -1, q) % q, 0), 4 * pow(5, -1, q) % q)


def ed25519_verify(public, message, signature):
    r, s = decode_point(signature[:32]), int.from_bytes(signature[32:], 'little')
    h = int.from_bytes(hashlib.sha512(signature[:32] + public + message).digest(), 'little') % L
    return scalar(s, BASE) == edwards_add(r, scalar(h, decode_point(public)))


def order_address(multisig, seqno):
    data = Cell(address_bits(multisig) + bits_of(seqno, 256))
    library = Cell('00000010' + bits_of(int.from_bytes(ORDER_CODE_HASH, 'big'), 256), library=True)
    return '0:' + Cell('00110', [library, data]).hash().hex()


def friendly(raw, bounceable=True):
    workchain, account = raw.split(':')
    body = bytes([0x11 if bounceable else 0x51, int(workchain) & 0xff]) + bytes.fromhex(account)
    crc = 0
    for byte in body:
        crc ^= byte << 8
        for _ in range(8):
            crc = (crc << 1) ^ 0x1021 if crc & 0x8000 else crc << 1
    return base64.urlsafe_b64encode(body + (crc & 0xffff).to_bytes(2, 'big')).decode()


def raw_of(address):
    if ':' in address: return address
    data = base64.urlsafe_b64decode(address)
    return f'{data[1] - 256 if data[1] > 127 else data[1]}:{data[2:34].hex()}'


MULTISIG = FIXTURE['multisig']['raw']
SIGNERS = [signer['raw'] for signer in FIXTURE['signers']]
KEYS = {signer['raw']: bytes.fromhex(signer['public_key']) for signer in FIXTURE['signers']}
SIGNERS_CELL = parse_boc(bytes.fromhex(FIXTURE['multisig']['signers_cell']['boc_hex']))
PROPOSERS_CELL = parse_boc(bytes.fromhex(FIXTURE['multisig']['proposers_cell']['boc_hex']))
for vector in FIXTURE['order_addresses']:
    assert order_address(MULTISIG, vector['seqno']) == vector['raw'], vector

live = dict(next=5, balance=3 * TON, orders={}, wallets={raw: dict(seqno=4, balance=2 * TON) for raw in SIGNERS},
            include=True, queued=[], executed=[], messages=[], errors=[])


def multisig_cell():
    cell = Cell(bits_of(live['next'], 256) + bits_of(2, 8) + bits_of(3, 8) + '1' + '0',
                [SIGNERS_CELL, PROPOSERS_CELL])
    return cell


def order_cell(seqno):
    order = live['orders'][seqno]
    mask = sum(1 << index for index in order['approvals'])
    return Cell(address_bits(MULTISIG) + bits_of(seqno, 256) + bits_of(2, 8) + str(int(order['sent']))
                + bits_of(mask, 256) + bits_of(len(order['approvals']), 8) + bits_of(order['expiration'], 48),
                [SIGNERS_CELL, order['order']])


def execute(seqno):
    order = live['orders'][seqno]
    order['sent'] = True
    # The order dictionary's one entry: label `same` (7 bits), then ^Action.
    assert order['order'].bits == '1101000', order['order'].bits
    action = Reader(order['order'].refs[0])
    assert action.uint(32) == 0xf1381e5b and action.uint(8) == 3
    message = Reader(action.reference())
    assert message.uint(1) == 0 and message.uint(1) == 1
    bounce = message.uint(1)
    assert message.uint(1) == 0 and message.uint(2) == 0
    destination, value = message.address(), message.coins()
    assert value <= live['balance']
    live['balance'] -= value
    live['executed'].append((destination, value, bounce))


def deliver(sender, internal):
    message = Reader(internal)
    assert message.uint(1) == 0 and message.uint(1) == 1 and message.uint(1) == 1  # bounceable
    assert message.uint(1) == 0 and message.uint(2) == 0
    destination, value = message.address(), message.coins()
    message.uint(1); message.coins(); message.coins(); message.uint(64); message.uint(32)
    assert message.uint(1) == 0
    body = Reader(message.reference()) if message.uint(1) else message
    op = body.uint(32)
    index = SIGNERS.index(sender)
    if op == 0xf718510f:
        assert destination == MULTISIG and value >= TON // 5, (destination, value)
        body.uint(64)
        seqno, signer, claimed, expiration = body.uint(256), body.uint(1), body.uint(8), body.uint(48)
        order = body.reference()
        assert signer == 1 and claimed == index and expiration >= time.time()
        if seqno != live['next']:
            live['errors'].append('invalid_new_order'); return
        live['next'] += 1
        live['orders'][seqno] = dict(approvals={index}, expiration=expiration, order=order, sent=False)
        return
    assert op == 0xa762230f and value >= TON // 10, (op, value)
    body.uint(64)
    assert body.uint(8) == index
    seqno = next(s for s in live['orders'] if order_address(MULTISIG, s) == destination)
    order = live['orders'][seqno]
    if order['sent'] or index in order['approvals']:
        live['errors'].append('approve_rejected'); return
    order['approvals'].add(index)
    if len(order['approvals']) >= 2:
        execute(seqno)


def receive(boc):
    external = parse_boc(boc)
    message = Reader(external)
    assert message.uint(2) == 2 and message.uint(2) == 0
    sender = message.address()
    assert message.coins() == 0 and message.uint(1) == 0 and message.uint(1) == 1  # no init, body by ref
    body = message.reference()
    signing = Cell(body.bits[:-512], body.refs)
    signature = int(body.bits[-512:], 2).to_bytes(64, 'big')
    assert ed25519_verify(KEYS[sender], signing.hash(), signature), 'bad wallet signature'
    signed = Reader(signing)
    assert signed.uint(32) == 0x7369676e
    signed.uint(32)
    assert signed.uint(32) >= time.time() and signed.uint(32) == live['wallets'][sender]['seqno']
    assert signed.uint(1) == 1
    actions = signed.reference()
    assert signed.uint(1) == 0 and signed.rest() == 0
    action = Reader(actions)
    assert action.reference().bits == '' and action.uint(32) == 0x0ec3c86d and action.uint(8) == 3
    live['wallets'][sender]['seqno'] += 1
    live['messages'].append(sender)
    if live['include']:
        deliver(sender, action.reference())
    else:
        live['queued'].append((sender, action.reference()))
    return base64.b64encode(external.hash()).decode()


def state_of(address):
    raw = raw_of(address)
    if raw == MULTISIG:
        return dict(state='active', balance=str(live['balance']), code=base64.b64encode(CODE).decode(),
                    data=base64.b64encode(to_boc(multisig_cell())).decode())
    for seqno in live['orders']:
        if order_address(MULTISIG, seqno) == raw:
            return dict(state='active', balance='0', code='', data=base64.b64encode(to_boc(order_cell(seqno))).decode())
    if raw in live['wallets']:
        return dict(state='active', balance=str(live['wallets'][raw]['balance']), code='', data='')
    return dict(state='uninitialized', balance='0', code='', data='')


class Toncenter(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass

    def reply(self, result):
        raw = json.dumps(dict(ok=True, result=result)).encode()
        self.send_response(200); self.send_header('Content-Length', str(len(raw)))
        self.end_headers(); self.wfile.write(raw)

    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        query = dict(urllib.parse.parse_qsl(url.query))
        if url.path.endswith('/getMasterchainInfo'): return self.reply(dict(init=ZERO_STATE))
        if url.path.endswith('/getAddressInformation'): return self.reply(state_of(query['address']))
        assert url.path.endswith('/getAddressBalance'), self.path
        self.reply(state_of(query['address'])['balance'])

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if self.path.endswith('/runGetMethod'):
            assert request['method'] == 'seqno'
            seqno = live['wallets'][raw_of(request['address'])]['seqno']
            return self.reply(dict(exit_code=0, stack=[['num', hex(seqno)]]))
        assert self.path.endswith('/sendBocReturnHash'), self.path
        self.reply(dict(hash=receive(base64.b64decode(request['boc']))))


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Toncenter)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
try:
    with tempfile.TemporaryDirectory(prefix='spectra-ton-multisig-') as directory:
        journal = pathlib.Path(directory) / 'network.jsonl'

        def run(data, *args, env=None, refusal=None):
            result = subprocess.run([BINARY, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120,
                                    env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(journal), **(env or {})})
            assert (result.returncode == 0) == (refusal is None), (data, args, result.stdout, result.stderr)
            if refusal is not None:
                assert refusal in result.stdout + result.stderr, (refusal, args, result.stdout, result.stderr)
                return None
            return json.loads(result.stdout)

        def signed_by(session):
            return sorted(s['signer'] for s in session['signers'] if s['signed'])

        for data in ['a', 'b']:
            run(data, 'endpoints', '--chain', 'ton', '--api', 'toncenter-v2', '--capabilities',
                'balance,verification,broadcast', '--add', endpoint)
            run(data, 'endpoints', '--chain', 'ton', '--custom-only', 'true')
            run(data, 'wallet', 'watch', '--chain', 'ton', '--name', 'Vault', '--address', FIXTURE['multisig']['address'])
        for data, name, index, extra in [('a', 'T0', 0, []), ('a', 'T2', 2, []), ('b', 'T1', 1, []),
                                         ('a', 'Outsider', 1, ['--ton-wallet', 'v4R2'])]:
            wallet = run(data, 'wallet', 'import', '--chain', 'ton', '--name', name, '--no-password', *extra,
                         env={'SPECTRA_SEED': MNEMONICS[index]['mnemonic']})['wallet']
            assert (raw_of(wallet['address']) == SIGNERS[index]) == (name != 'Outsider'), wallet
        addresses = [friendly(raw) for raw in SIGNERS]

        account = run('a', 'multisig', 'account', 'Vault')['account']
        permission = account['permissions'][0]
        assert account['scheme'] == 'tonMultisig' and permission['threshold'] == 2, account
        assert [s['signer'] for s in permission['signers']] == addresses, account
        assert [s['walletId'] is not None for s in permission['signers']] == [True, False, True], account
        assert any('Proposers' in warning for warning in account['warnings']), account

        run('a', 'multisig', 'create', '--from', 'Vault', '--to', friendly(DESTINATION, False), '--amount', '4',
            refusal='cannot pay this')
        created = run('a', 'multisig', 'create', '--from', 'Vault', '--to', friendly(DESTINATION, False),
                      '--amount', '1')['session']
        assert created['scheme'] == 'tonMultisig' and created['sequence'] is None, created
        assert created['outputs'][0]['address'] == friendly(DESTINATION, False), created
        assert created['outputs'][0]['value'] == str(TON) and created['expiresAt'] > time.time() + 6 * 86400, created
        sign = lambda data, session, signer, **kw: run(data, 'multisig', 'sign', session['id'], '--review-digest',
                                                       session['reviewDigest'], '--signer', signer, **kw)
        sign('a', created, 'Outsider', refusal='not one of the multisig')

        live['include'] = False
        proposed = sign('a', created, 'T0')['session']
        assert proposed['sequence'] == '5' and signed_by(proposed) == [addresses[0]], proposed
        assert proposed['transactionId'] == friendly(order_address(MULTISIG, 5)) and not proposed['complete'], proposed
        sign('a', created, 'T2', refusal='not on the network yet')
        live['include'] = True
        deliver(*live['queued'].pop())
        assert live['next'] == 6 and live['orders'][5]['approvals'] == {0}
        sign('a', created, 'T0', refusal='already approved')
        run('a', 'multisig', 'submit', created['id'], '--yes', refusal='goes to the network as it signs')

        at_b = run('b', 'multisig', 'import', '--wallet', 'Vault', '--data', proposed['data'])['session']
        assert at_b['sequence'] == '5' and signed_by(at_b) == [addresses[0]], at_b
        assert at_b['reviewDigest'] == created['reviewDigest'], at_b
        executed = sign('b', at_b, 'T1')['session']
        assert executed['complete'] and executed['submittedTxid'], executed
        assert signed_by(executed) == sorted([addresses[0], addresses[1]]), executed
        assert live['executed'] == [(DESTINATION, TON, 0)] and live['balance'] == 2 * TON, live['executed']
        sign('a', created, 'T2', refusal='already executed')
        assert live['messages'] == [SIGNERS[0], SIGNERS[1]] and not live['errors'], live
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
    print('PASS ton multisig: a v2 multisig watched and its code checked; an order proposed by one signer\'s W5 '
          'wallet and approved by another\'s in a second data directory executes, every wallet message '
          'verified; outsiders, double and early approvals, unpayable and executed orders and needless '
          'submissions refused')
finally:
    server.shutdown()
    server.server_close()
