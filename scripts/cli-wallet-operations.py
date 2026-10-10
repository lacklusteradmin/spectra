#!/usr/bin/env python3
"""Operations a wallet's page builds, each through the stages a send takes,
against loopback nodes and a fresh process per command: closing an XRP
account and merging a Stellar one, deleting a NEAR function-call key,
refunding a NEAR token's storage deposit, merging Sui coin objects and
closing empty Solana token accounts. Each is built, signed, broadcast once
with exactly the signed bytes, and recorded in the wallet's history; a
refused closing stores nothing. What each operation lists, costs and
refuses is tested in core."""
import base64
import http.server
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
fixtures = pathlib.Path(__file__).resolve().parents[1] / 'core/tests/fixtures'
xrp = json.loads((fixtures / 'xrp-mnemonic-payment.json').read_text())
closing = json.loads((fixtures / 'account-closing.json').read_text())
merge = closing['stellar_account_merge']
XRP_DESTINATION = 'rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe'
NEAR_CONFIG = json.loads((fixtures / 'near-staking-fee-protocol86.json').read_text())
PHRASE = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
# near-seed-phrase's key for PHRASE at m/44'/397'/0': the implicit account.
NEAR_IMPLICIT = '5510e2b44cae6eb807e3e0e45d579dda058c274abcba15e5cb84636f5d1ee412'
NEAR_DEPOSIT = 1250000000000000000000
SUI_TOKEN = '0x' + 'aa' * 32 + '::usdc::USDC'
B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


def b58(data: bytes) -> str:
    number, text = int.from_bytes(data, 'big'), ''
    while number:
        number, digit = divmod(number, 58)
        text = B58[digit] + text
    return '1' * (len(data) - len(data.lstrip(b'\0'))) + text


def ed25519(hex_key: str) -> str:
    return 'ed25519:' + b58(bytes.fromhex(hex_key))


def stellar_secret(seed: bytes) -> str:
    """A Stellar secret seed's strkey: version 18 << 3, the seed, CRC16-XMODEM."""
    payload = bytes([18 << 3]) + seed
    crc = 0
    for byte in payload:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return base64.b32encode(payload + crc.to_bytes(2, 'little')).decode().rstrip('=')


class Node(http.server.BaseHTTPRequestHandler):
    """An XRPL JSON-RPC node under /xrpl, a Horizon under /horizon, and NEAR,
    Sui and Solana nodes under /near, /sui and /solana."""
    state = {}
    submitted = []

    def log_message(self, *_):
        pass

    def reply(self, payload, status=200):
        data = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        state = self.state
        path = self.path.removeprefix('/horizon')
        if path == '/' or path == '':
            return self.reply({'network_passphrase': merge['network_passphrase']})
        if path.startswith('/ledgers'):
            return self.reply({'_embedded': {'records': [
                {'sequence': state['ledger'], 'base_reserve_in_stroops': 5_000_000}]}})
        if path.startswith('/fee_stats'):
            return self.reply({'fee_charged': {'mode': '100'}})
        if path.startswith('/accounts/'):
            account = state['stellar_accounts'].get(path.split('/')[2])
            return self.reply(account, 200) if account else self.reply({'status': 404}, 404)
        raise AssertionError(self.path)

    def near(self, call):
        method, params, state = call['method'], call['params'], self.state
        block = {'header': {'hash': b58(bytes([2] * 32)), 'height': 100}}
        if method == 'status':
            return {'chain_id': 'mainnet'}
        if method == 'block':
            return block
        if method == 'gas_price':
            return {'gas_price': '100000000'}
        if method == 'EXPERIMENTAL_protocol_config':
            runtime = {**NEAR_CONFIG['runtime_config'], 'storage_amount_per_byte': '10000000000000000000'}
            return {**NEAR_CONFIG, 'runtime_config': runtime, 'transaction_validity_period': 86400}
        if method == 'broadcast_tx_commit':
            self.submitted.append(params[0])
            return {'transaction': {'hash': b58(bytes([5] * 32))}}
        assert method == 'query', call
        kind = params['request_type']
        if kind == 'view_access_key_list':
            assert params['account_id'] == state['account'], params
            return {'keys': state['keys']}
        if kind == 'view_access_key':
            entry = next(k for k in state['keys'] if k['public_key'] == params['public_key'])
            return entry['access_key']
        if kind == 'view_account':
            return {'amount': str(10 ** 24), 'locked': '0', 'storage_usage': 500}
        if kind == 'call_function':
            assert json.loads(base64.b64decode(params['args_base64'])) == {'account_id': state['account']}, params
            answer = state['tokens'][params['account_id']][params['method_name']]
            return {'result': list(json.dumps(answer).encode()), 'logs': [], 'block_height': 100}
        raise AssertionError(call)

    def sui(self, call):
        method, params = call['method'], call['params']
        coin = lambda n, version, balance: {'coinObjectId': '0x' + n * 32, 'version': str(version),
                                            'digest': '1' * 32, 'balance': str(balance)}
        coins = {'0x2::sui::SUI': [coin('33', 7, 4000000)],
                 SUI_TOKEN: [coin('66', 10, 1000000), coin('77', 11, 1000000), coin('88', 12, 1000000)]}
        if method == 'sui_getChainIdentifier':
            return '35834a8a'
        if method == 'sui_getCheckpoint':
            return {'sequenceNumber': '0', 'digest': '4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S'}
        if method == 'suix_getCoins':
            return {'data': coins[params[1]], 'nextCursor': None, 'hasNextPage': False}
        if method == 'suix_getBalance':
            return {'totalBalance': '4000000'}
        if method == 'suix_getReferenceGasPrice':
            return '1000'
        if method == 'sui_dryRunTransactionBlock':
            return {'effects': {'status': {'status': 'success'}, 'gasUsed': {
                'computationCost': '1000000', 'storageCost': '2000000', 'storageRebate': '2500000'}}}
        if method == 'sui_executeTransactionBlock':
            self.submitted.append(params[0])
            return {'digest': 'D1gest', 'effects': {'status': {'status': 'success'}}}
        raise AssertionError(call)

    def solana(self, call):
        method, params, state = call['method'], call.get('params') or [], self.state
        if method == 'getGenesisHash':
            return '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d'
        if method == 'getTokenAccountsByOwner':
            assert params[0] == state['owner'], params
            program = params[1]['programId']
            return {'value': [row for row in state['token_accounts'] if row['program'] == program]}
        if method == 'getLatestBlockhash':
            return {'value': {'blockhash': b58(bytes([5] * 32)), 'lastValidBlockHeight': 100}}
        if method == 'getFeeForMessage':
            return {'value': 5000}
        if method == 'getBalance':
            return {'value': 1000000}
        if method == 'isBlockhashValid':
            return {'value': True}
        if method == 'sendTransaction':
            self.submitted.append(params[0])
            return b58(base64.b64decode(params[0])[1:65])
        raise AssertionError(call)

    def do_POST(self):
        body = self.rfile.read(int(self.headers['Content-Length']))
        if self.path.startswith('/solana'):
            call = json.loads(body)
            return self.reply({'jsonrpc': '2.0', 'id': call['id'], 'result': self.solana(call)})
        if self.path.startswith('/sui'):
            call = json.loads(body)
            return self.reply({'jsonrpc': '2.0', 'id': call['id'], 'result': self.sui(call)})
        if self.path.startswith('/near'):
            call = json.loads(body)
            return self.reply({'jsonrpc': '2.0', 'id': call['id'], 'result': self.near(call)})
        if self.path.startswith('/horizon/transactions'):
            self.submitted.append(json.loads(body)['tx'])
            return self.reply({'hash': 'cc' * 32})
        call = json.loads(body)
        method, params = call['method'], (call.get('params') or [{}])[0]
        state = self.state
        if method == 'server_info':
            result = {'info': {'network_id': 0}}
        elif method == 'server_state':
            result = {'state': {'validated_ledger': {
                'seq': state['ledger'], 'reserve_base': 1_000_000, 'reserve_inc': 200_000}}}
        elif method == 'account_info':
            root = state['xrp_accounts'].get(params['account'])
            result = {'account_data': root} if root else {'error': 'actNotFound', 'status': 'error'}
        elif method == 'account_objects':
            assert params['deletion_blockers_only'] is True, params
            result = {'account_objects': state['blockers']}
        elif method == 'submit':
            self.submitted.append(params['tx_blob'])
            result = {'engine_result': 'tesSUCCESS', 'accepted': True, 'tx_json': {'hash': 'AB' * 32}}
        else:
            raise AssertionError(call)
        self.reply({'result': result})


class WalletOperationsTests(unittest.TestCase):
    def setUp(self):
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.base = f'http://127.0.0.1:{self.server.server_port}'
        self.directory = tempfile.TemporaryDirectory(prefix='spectra-closing-')
        Node.submitted.clear()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.directory.cleanup()

    def run_cli(self, *args, success=True, env=None):
        p = subprocess.run([binary, '--data-dir', self.directory.name, '--json', *args], capture_output=True,
                           text=True, timeout=60, env={**os.environ, **(env or {})})
        assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
        return json.loads(p.stdout) if success else (p.returncode, p.stdout + p.stderr)

    def endpoint(self, chain, api, path):
        self.run_cli('endpoints', '--chain', chain, '--api', api, '--capabilities',
                     'balance,fee,broadcast,verification', '--add', self.base + path)
        self.run_cli('endpoints', '--chain', chain, '--custom-only', 'true')

    def sign_and_broadcast(self, built, path):
        """Sign the reviewed artifact and broadcast it once; its signed payload."""
        signed = self.run_cli('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                              '--endpoint', self.base + path)['artifact']
        self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.base + path, '--yes')
        return signed['signed_payload']

    def history(self):
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            return [json.loads(row[0]) for row in db.execute('SELECT payload FROM history_records')]

    def test_xrp_deletes_the_account_once_nothing_blocks_it(self):
        sender = xrp['transaction']['Account']
        Node.state = {
            'ledger': 2_000,
            'xrp_accounts': {
                sender: {'Balance': '25000000', 'Sequence': 1_000, 'OwnerCount': 2, 'Flags': 0},
                XRP_DESTINATION: {'Balance': '50000000', 'Sequence': 5, 'OwnerCount': 0, 'Flags': 0},
            },
            'blockers': [{'LedgerEntryType': 'RippleState'}],
        }
        self.run_cli('wallet', 'import', '--chain', 'xrp', '--name', 'XRP', '--no-password',
                     '--path', xrp['derivation_path'], env={'SPECTRA_SEED': xrp['mnemonic']})
        self.endpoint('xrp', 'xrpl-json-rpc', '/xrpl')
        # Refused by the network's rules, exit 3, and nothing on disk.
        code, output = self.run_cli('wallet', 'close', 'XRP', '--to', XRP_DESTINATION, success=False)
        assert code == 3 and 'trust lines' in output, output
        assert self.run_cli('send', 'list')['artifacts'] == [], 'a refusal builds nothing'
        Node.state['blockers'] = []
        built = self.run_cli('wallet', 'close', 'XRP', '--to', XRP_DESTINATION)['artifact']
        blob = json.loads(self.sign_and_broadcast(built, '/xrpl'))['tx_blob_hex']
        assert Node.submitted == [blob], Node.submitted
        assert [(row['kind'], row['amount']) for row in self.history()] == [('send', '24.8')], self.history()

    def test_stellar_merges_the_account_as_the_sdk_signs_it(self):
        sender, destination = merge['source'], merge['destination']
        def account(sequence):
            return {'balances': [{'balance': '5.0000000', 'asset_type': 'native'}], 'sequence': sequence,
                    'subentry_count': 0, 'num_sponsoring': 0, 'num_sponsored': 0,
                    'flags': {'auth_immutable': False}, 'data': {}}
        Node.state = {'ledger': 1_000, 'stellar_accounts': {
            sender: account(str(int(merge['sequence']) - 1)), destination: account('1')}}
        self.run_cli('wallet', 'import', '--chain', 'stellar-testnet', '--name', 'XLM', '--no-password',
                     '--private-key-env', 'CLOSING_KEY',
                     env={'CLOSING_KEY': stellar_secret(bytes.fromhex(merge['seed']))})
        self.endpoint('stellar-testnet', 'horizon', '/horizon')
        built = self.run_cli('wallet', 'close', 'XLM', '--to', destination)['artifact']
        signed = json.loads(self.sign_and_broadcast(built, '/horizon'))['signed_xdr_b64']
        assert Node.submitted == [signed], Node.submitted
        assert [row['kind'] for row in self.history()] == ['send'], self.history()

    def test_near_deletes_a_function_call_key(self):
        signer, dapp = ed25519(NEAR_IMPLICIT), ed25519('33' * 32)
        Node.state = {'account': NEAR_IMPLICIT, 'keys': [
            {'public_key': dapp, 'access_key': {'nonce': 7, 'permission': {'FunctionCall': {
                'allowance': None, 'receiver_id': 'zapp.near', 'method_names': ['play']}}}},
            {'public_key': signer, 'access_key': {'nonce': 7, 'permission': 'FullAccess'}},
        ]}
        self.run_cli('wallet', 'import', '--chain', 'near', '--name', 'NEAR', '--no-password', env={'SPECTRA_SEED': PHRASE})
        self.endpoint('near', 'near-json-rpc', '/near')
        built = self.run_cli('wallet', 'delete-key', 'NEAR', '--key', dapp)['artifact']
        signed = json.loads(self.sign_and_broadcast(built, '/near'))['signed_tx_b64']
        assert Node.submitted == [signed], Node.submitted
        assert [row['kind'] for row in self.history()] == ['deleteAccessKey'], self.history()

    def test_near_refunds_a_token_storage_deposit(self):
        signer = ed25519(NEAR_IMPLICIT)
        Node.state = {'account': NEAR_IMPLICIT,
                      'keys': [{'public_key': signer, 'access_key': {'nonce': 7, 'permission': 'FullAccess'}}],
                      'tokens': {'empty.near': {'storage_balance_of': {'total': str(NEAR_DEPOSIT), 'available': '0'},
                                                'ft_balance_of': '0'}}}
        self.run_cli('wallet', 'import', '--chain', 'near', '--name', 'NEAR', '--no-password', env={'SPECTRA_SEED': PHRASE})
        self.endpoint('near', 'near-json-rpc', '/near')
        built = self.run_cli('wallet', 'refund-storage', 'NEAR', '--contract', 'empty.near')['artifact']
        signed = json.loads(self.sign_and_broadcast(built, '/near'))['signed_tx_b64']
        assert Node.submitted == [signed], Node.submitted
        assert [(row['kind'], row['amount']) for row in self.history()] == [('refundTokenStorage', '0.00125')], \
            self.history()

    def test_sui_merges_a_tokens_objects_with_gas_apart(self):
        Node.state = {}
        self.run_cli('wallet', 'import', '--chain', 'sui', '--name', 'SUI', '--no-password', env={'SPECTRA_SEED': PHRASE})
        self.endpoint('sui', 'sui-json-rpc', '/sui')
        built = self.run_cli('wallet', 'merge', 'SUI', '--coin-type', SUI_TOKEN)['artifact']
        signed = json.loads(self.sign_and_broadcast(built, '/sui'))['tx_bytes_b64']
        assert Node.submitted == [signed], Node.submitted
        assert [row['kind'] for row in self.history()] == ['mergeCoins'], self.history()

    def test_solana_closes_its_empty_token_accounts(self):
        spl, t22 = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA', 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb'
        owner = self.run_cli('wallet', 'import', '--chain', 'solana', '--name', 'SOL', '--no-password',
                             env={'SPECTRA_SEED': PHRASE})['wallet']['address']
        def account(n, program, lamports):
            return {'program': program, 'pubkey': b58(bytes([n] * 32)), 'account': {'lamports': lamports, 'data': {
                'parsed': {'info': {'mint': b58(bytes([n + 1] * 32)), 'owner': owner, 'state': 'initialized',
                                    'tokenAmount': {'amount': '0', 'decimals': 6}}}}}}
        Node.state = {'owner': owner, 'token_accounts': [account(0x11, spl, 2039280), account(0x22, t22, 2074080)]}
        self.endpoint('solana', 'solana-json-rpc', '/solana')
        built = self.run_cli('wallet', 'close-token-accounts', 'SOL')['artifact']
        signed = self.sign_and_broadcast(built, '/solana')
        # One signature, then exactly the reviewed message.
        assert Node.submitted == [signed], Node.submitted
        assert base64.b64decode(signed)[65:].hex() == built['signing_payload_hex']
        assert [(row['kind'], row['amount']) for row in self.history()] == [('closeTokenAccounts', '0.00411336')], \
            self.history()


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
