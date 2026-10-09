#!/usr/bin/env python3
"""Sui multisig and Aptos MultiKey accounts against loopback nodes, each
watched from its policy across two data directories.

Sui: the policy of core/tests/fixtures/sui-multisig.json (an Ed25519, a
secp256k1 and a secp256r1 key weighted 1, 1 and 2) watched at the SDK's
address. A transfer is built from the account's gas coin, byte for byte the
SDK's; the wallet holding the Ed25519 key signs it as the SDK does; the
secp256k1 member's signature, made by the SDK, is read in as data and
verified; and the transaction executes with exactly the SDK's combined
`MultiSig` signature. Refused: a wallet holding no member key, a member
twice, a signature over another transaction, an execution short of the
threshold and a gas coin spent elsewhere.

Aptos: the MultiKey of core/tests/fixtures/aptos-multikey.json (two
Ed25519 keys and a secp256k1 key, two required) watched at its
authentication key. P0 signs in one data directory and P1 in the other from
P0's copy; the copies join and the signed transaction submitted in BCS
carries both signatures under one MultiKey authenticator, each verified
here (Ed25519) over the signing message. Refused: a wallet holding no key,
a submission short of the requirement, a sequence another transaction used
and an account whose key was rotated.
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
from urllib.parse import urlsplit

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
root = pathlib.Path(__file__).resolve().parents[1]
SUI = json.loads((root / 'core/tests/fixtures/sui-multisig.json').read_text())
APTOS = json.loads((root / 'core/tests/fixtures/aptos-multikey.json').read_text())
OUTSIDER = 'zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong'
state = {}

q = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
d = -121665 * pow(121666, -1, q) % q
I = pow(2, (q - 1) // 4, q)


def recover_x(y, sign):
    x2 = (y * y - 1) * pow(d * y * y + 1, -1, q)
    x = pow(x2, (q + 3) // 8, q)
    if (x * x - x2) % q:
        x = x * I % q
    if x % 2 != sign:
        x = q - x
    return x


def edwards_add(a, b):
    (x1, y1), (x2, y2) = a, b
    x3 = (x1 * y2 + x2 * y1) * pow(1 + d * x1 * x2 * y1 * y2, -1, q)
    y3 = (y1 * y2 + x1 * x2) * pow(1 - d * x1 * x2 * y1 * y2, -1, q)
    return x3 % q, y3 % q


def scalar(e, point):
    result = (0, 1)
    while e:
        if e & 1:
            result = edwards_add(result, point)
        point = edwards_add(point, point)
        e >>= 1
    return result


def decode_point(data):
    y = int.from_bytes(data, 'little') & ((1 << 255) - 1)
    return recover_x(y, data[31] >> 7), y


BASE = (recover_x(4 * pow(5, -1, q) % q, 0), 4 * pow(5, -1, q) % q)


def ed25519_verify(public, message, signature):
    r, s = decode_point(signature[:32]), int.from_bytes(signature[32:], 'little')
    h = int.from_bytes(hashlib.sha512(signature[:32] + public + message).digest(), 'little') % L
    return scalar(s, BASE) == edwards_add(r, scalar(h, decode_point(public)))


def base58(data):
    alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
    number, out = int.from_bytes(data, 'big'), ''
    while number:
        number, rest = divmod(number, 58)
        out = alphabet[rest] + out
    return '1' * (len(data) - len(data.lstrip(b'\0'))) + out


class Node(http.server.BaseHTTPRequestHandler):
    """A Sui JSON-RPC node at /sui and an Aptos REST node at /aptos."""

    def log_message(self, *_):
        pass

    def reply(self, value, status=200):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = urlsplit(self.path).path
        assert path.startswith('/aptos'), path
        path = path[len('/aptos'):] or '/'
        if path == '/':
            return self.reply({'chain_id': 2, 'ledger_version': '9'})
        if path == '/estimate_gas_price':
            return self.reply({'gas_estimate': 100})
        if path.startswith('/accounts/'):
            return self.reply({'sequence_number': str(state['aptos_sequence']),
                               'authentication_key': state['aptos_key']})
        self.reply({'error': path}, 404)

    def do_POST(self):
        body = self.rfile.read(int(self.headers['Content-Length']))
        path = urlsplit(self.path).path
        if path == '/aptos/view':
            return self.reply(['10000000000'])
        if path == '/aptos/transactions':
            assert self.headers['Content-Type'] == 'application/x.aptos.signed_transaction+bcs'
            state['aptos_submitted'].append(body)
            prefix = hashlib.sha3_256(b'APTOS::Transaction').digest()
            return self.reply({'hash': '0x' + hashlib.sha3_256(prefix + b'\0' + body).hexdigest()})
        call = json.loads(body)
        method, params = call['method'], call.get('params', [])
        if method == 'sui_getChainIdentifier':
            result = '35834a8a'
        elif method == 'sui_getCheckpoint':
            result = {'sequenceNumber': '0', 'digest': '4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S'}
        elif method == 'suix_getReferenceGasPrice':
            result = '1000'
        elif method == 'suix_getCoins':
            assert params[0] == SUI['multisig']['address'], params
            result = {'data': [{'coinObjectId': '0x' + '33' * 32, 'version': state['sui_version'],
                                'digest': '1' * 32, 'balance': '200000000'}],
                      'hasNextPage': False, 'nextCursor': None}
        elif method == 'sui_executeTransactionBlock':
            state['sui_executed'].append(params)
            transaction = base64.b64decode(params[0])
            digest = base58(hashlib.blake2b(b'TransactionData::' + transaction, digest_size=32).digest())
            result = {'digest': digest, 'effects': {'status': {'status': 'success'}}}
        else:
            raise AssertionError(method)
        self.reply({'jsonrpc': '2.0', 'id': call['id'], 'result': result})


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
state.update(sui_version='7', sui_executed=[], aptos_sequence=5,
             aptos_key=APTOS['multi_key']['authentication_key'], aptos_submitted=[])
try:
    with tempfile.TemporaryDirectory(prefix='spectra-multikey-') as directory:
        journal = pathlib.Path(directory) / 'network.jsonl'

        def run(data, *args, env=None, refusal=None):
            result = subprocess.run([binary, '--data-dir', str(pathlib.Path(directory) / data), '--json', *args],
                                    capture_output=True, text=True, timeout=120,
                                    env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(journal), **(env or {})})
            assert (result.returncode == 0) == (refusal is None), (data, args, result.stdout, result.stderr)
            if refusal is not None:
                assert refusal in result.stdout + result.stderr, (refusal, args, result.stdout, result.stderr)
                return None
            return json.loads(result.stdout)

        def signed_by(session):
            return [s['signer'] for s in session['signers'] if s['signed']]

        # ── Sui ──
        policy = json.dumps({'threshold': SUI['multisig']['threshold'], 'publicKeys': [
            {'publicKey': key['sui_public_key'], 'weight': weight}
            for key, weight in zip(SUI['keys'], SUI['multisig']['weights'])]})
        run('a', 'endpoints', '--chain', 'sui', '--api', 'sui-json-rpc', '--capabilities',
            'balance,fee,verification,broadcast', '--add', endpoint + '/sui')
        run('a', 'endpoints', '--chain', 'sui', '--custom-only', 'true')
        watched = run('a', 'wallet', 'watch', '--chain', 'sui', '--name', 'Vault', '--multisig', policy)
        assert watched['wallets'][0]['addresses']['sui'] == SUI['multisig']['address'], watched
        for name, phrase in [('Signer0', SUI['keys'][0]['phrase']), ('Outsider', OUTSIDER)]:
            run('a', 'wallet', 'import', '--chain', 'sui', '--name', name, '--no-password',
                env={'SPECTRA_SEED': phrase})
        account = run('a', 'multisig', 'account', 'Vault')['account']
        members = account['permissions'][0]['signers']
        assert [m['weight'] for m in members] == [1, 1, 2] and members[0]['walletId'], account
        run('a', 'multisig', 'import', '--wallet', 'Vault', '--data', 'not json', refusal='Not a Sui multisig session')

        created = run('a', 'multisig', 'create', '--from', 'Vault', '--to', SUI['transaction']['recipient'],
                      '--amount', '0.001')['session']
        assert created['scheme'] == 'suiMultisig', created
        assert created['transactionId'] == SUI['transaction']['digest'], created
        assert created['reviewDigest'] == SUI['transaction']['intent_message_digest'], created
        run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer',
            'Outsider', refusal="holds none of the account's keys")
        signed = run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'],
                     '--signer', 'Signer0')['session']
        assert json.loads(signed['data'])['signatures'] == [SUI['partial_signatures'][0]['signature']], signed
        run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer',
            'Signer0', refusal='already signed')
        run('a', 'multisig', 'submit', created['id'], '--yes', refusal='do not yet meet')
        # The secp256k1 member's signature, made by the SDK, arrives as data;
        # one over another transaction does not.
        k1 = SUI['partial_signatures'][1]['signature']
        forged = dict(json.loads(signed['data']), signatures=[SUI['partial_signatures'][0]['signature'][:-8] + 'AAAAAAA='])
        run('a', 'multisig', 'import', '--wallet', 'Vault', '--data', json.dumps(forged), refusal='not a valid one')
        joined = run('a', 'multisig', 'import', '--wallet', 'Vault', '--data',
                     json.dumps({'transaction': SUI['transaction']['raw_base64'], 'signatures': [k1]}))['session']
        assert joined['id'] == created['id'] and joined['complete'] and joined['signedWeight'] == 2, joined
        state['sui_version'] = '8'
        run('a', 'multisig', 'submit', created['id'], '--yes', refusal='gas object')
        state['sui_version'] = '7'
        executed = run('a', 'multisig', 'submit', created['id'], '--yes')['session']
        [params] = state['sui_executed']
        k0_k1 = next(c for c in SUI['combined_signatures'] if c['name'] == 'k0_k1')
        assert params[0] == SUI['transaction']['raw_base64'] and params[1] == [k0_k1['signature']], params
        assert executed['submittedTxid'] == SUI['transaction']['digest'], executed

        # ── Aptos ──
        keys = [f"{k['scheme']}-pub-0x{k['public_key']}" for k in APTOS['keys']]
        policy = json.dumps({'signaturesRequired': 2, 'publicKeys': keys})
        for data, name, phrase in [('a', 'Signer0', APTOS['keys'][0]['phrase']),
                                   ('b', 'Signer1', APTOS['keys'][1]['phrase'])]:
            run(data, 'endpoints', '--chain', 'aptos-testnet', '--api', 'aptos-rest', '--capabilities',
                'balance,fee,verification,broadcast', '--add', endpoint + '/aptos')
            run(data, 'endpoints', '--chain', 'aptos-testnet', '--custom-only', 'true')
            watched = run(data, 'wallet', 'watch', '--chain', 'aptos-testnet', '--name', 'Shared', '--multisig', policy)
            assert list(watched['wallets'][0]['addresses'].values()) == [APTOS['multi_key']['address']], watched
            run(data, 'wallet', 'import', '--chain', 'aptos-testnet', '--name', 'Aptos' + name, '--no-password',
                env={'SPECTRA_SEED': phrase})
        run('a', 'wallet', 'import', '--chain', 'aptos-testnet', '--name', 'AptosOutsider', '--no-password',
            env={'SPECTRA_SEED': OUTSIDER})
        created = run('a', 'multisig', 'create', '--from', 'Shared', '--to', APTOS['transaction']['recipient'],
                      '--amount', '0.01')['session']
        assert created['scheme'] == 'aptosMultiKey' and created['sequence'] == '5', created
        run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'], '--signer',
            'AptosOutsider', refusal="holds none of the account's keys")
        by_p0 = run('a', 'multisig', 'sign', created['id'], '--review-digest', created['reviewDigest'],
                    '--signer', 'AptosSigner0')['session']
        assert signed_by(by_p0) == [keys[0]], by_p0
        run('a', 'multisig', 'submit', created['id'], '--yes', refusal='requires')
        at_b = run('b', 'multisig', 'import', '--wallet', 'Shared', '--data', by_p0['data'])['session']
        by_p1 = run('b', 'multisig', 'sign', at_b['id'], '--review-digest', at_b['reviewDigest'],
                    '--signer', 'AptosSigner1')['session']
        assert by_p1['complete'], by_p1
        joined = run('a', 'multisig', 'import', '--wallet', 'Shared', '--data', by_p1['data'])['session']
        assert joined['id'] == created['id'] and joined['complete'], joined
        state['aptos_key'] = '0x' + '77' * 32
        run('a', 'multisig', 'submit', created['id'], '--yes', refusal='rotated')
        state['aptos_key'] = APTOS['multi_key']['authentication_key']
        state['aptos_sequence'] = 6
        run('a', 'multisig', 'submit', created['id'], '--yes', refusal='sequence moved on')
        state['aptos_sequence'] = 5
        sent = run('a', 'multisig', 'submit', created['id'], '--yes')['session']
        [signed] = state['aptos_submitted']
        raw = bytes.fromhex(json.loads(joined['data'])['transaction'][2:])
        assert signed.startswith(raw), 'the raw transaction is the reviewed one'
        tail = signed[len(raw):]
        multi_key = bytes.fromhex(APTOS['multi_key']['bcs'])
        assert tail[:2] == bytes([4, 3]) and tail[2:2 + len(multi_key)] == multi_key, tail.hex()
        rest = tail[2 + len(multi_key):]
        assert rest[0] == 2 and rest[-5:] == bytes([4, 0xc0, 0, 0, 0]), rest.hex()
        message = hashlib.sha3_256(b'APTOS::RawTransaction').digest() + raw
        for index, at in [(0, 1), (1, 1 + 66)]:
            assert rest[at:at + 2] == bytes([0, 64])
            public = bytes.fromhex(APTOS['keys'][index]['public_key'])
            assert ed25519_verify(public, message, rest[at + 2:at + 66])
        prefix = hashlib.sha3_256(b'APTOS::Transaction').digest()
        assert sent['submittedTxid'] == '0x' + hashlib.sha3_256(prefix + b'\0' + signed).hexdigest(), sent
        assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
    print('PASS sui and aptos multisig: watched from their policies at the SDKs\' addresses; a Sui transfer signed '
          'here and by the SDK combines into its MultiSig signature; an Aptos transfer signed in two data '
          'directories submits as one MultiKey authenticator; outsiders, double signatures, forged signatures, '
          'short thresholds, spent gas, used sequences and rotated keys refused')
finally:
    server.shutdown()
    server.server_close()
