#!/usr/bin/env python3
"""Operations a wallet's page builds, against loopback nodes. Closing an XRP
or Stellar account: each prerequisite the network enforces is refused before
anything is built, and a closing that passes is built, signed exactly as each
network's SDK signs it and broadcast as a send. NEAR access keys: listed with
the signing key marked; a function-call key deleted, every other refused.
NEAR token storage: the contracts the inventory, holdings and history name
that hold a deposit beside an empty balance are listed, one is unregistered
without `force`, and held tokens, missing deposits and watched wallets are
refused. Cardano: a phrase wallet's account shows its stake address, rewards
and delegation as Koios reports them; a raw key's address has none."""
import base64
import decimal
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
CARDANO = json.loads((fixtures / 'cardano-stake-addresses.json').read_text())
CARDANO_KEY = json.loads((fixtures / 'cardano-emurgo-witness.json').read_text())['privateKey']
NEAR_DEPOSIT = 1250000000000000000000
METHOD_NOT_FOUND = {'error': 'wasm execution failed with error: FunctionCallError(MethodResolveError(MethodNotFound))',
                    'logs': [], 'block_height': 100}
SUI_TOKEN = '0x' + 'aa' * 32 + '::usdc::USDC'
SUI_SINGLE = '0x' + 'bb' * 32 + '::one::ONE'
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
    """An XRPL JSON-RPC node under /xrpl and a Horizon under /horizon."""
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
        if self.path.startswith('/koios/genesis'):
            return self.reply([{'networkmagic': '764824073', 'networkid': 'Mainnet'}])
        if self.path.startswith('/nearblocks/'):
            assert self.path == f"/nearblocks/accounts/{state['account']}/assets/fts?limit=250", self.path
            return self.reply({'data': [{'contract': contract, 'amount': amount, 'meta': {'decimals': 6}}
                                        for contract, amount in state['inventory']], 'meta': {}})
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
            answers = state['tokens'].get(params['account_id'], {})
            if params['method_name'] not in answers:
                return METHOD_NOT_FOUND
            state['calls'].append((params['account_id'], params['method_name']))
            return {'result': list(json.dumps(answers[params['method_name']]).encode()), 'logs': [],
                    'block_height': 100}
        raise AssertionError(call)

    def sui(self, call):
        method, params, state = call['method'], call['params'], self.state
        coin = lambda n, version, balance: {'coinObjectId': '0x' + n * 32, 'version': str(version),
                                            'digest': '1' * 32, 'balance': str(balance)}
        # An executed transaction gives every object it used a new version.
        bump = len(self.submitted)
        coins = {'0x2::sui::SUI': [coin('33', 7 + bump, 4000000), coin('44', 8 + bump, 4000000),
                                   coin('55', 9 + bump, 4000000)],
                 SUI_TOKEN: [coin('66', 10, 1000000), coin('77', 11, 1000000), coin('88', 12, 1000000)],
                 SUI_SINGLE: [coin('99', 13, 5)]}
        if method == 'sui_getChainIdentifier':
            return '35834a8a'
        if method == 'sui_getCheckpoint':
            return {'sequenceNumber': '0', 'digest': '4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S'}
        if method == 'suix_getAllBalances':
            return [{'coinType': kind, 'coinObjectCount': len(rows),
                     'totalBalance': str(sum(int(row['balance']) for row in rows))} for kind, rows in coins.items()]
        if method == 'suix_getCoins':
            return {'data': coins[params[1]], 'nextCursor': None, 'hasNextPage': False}
        if method == 'suix_getBalance':
            return {'totalBalance': '12000000'}
        if method == 'suix_getCoinMetadata':
            return {'decimals': 6}
        if method == 'suix_getReferenceGasPrice':
            return '1000'
        if method == 'sui_dryRunTransactionBlock':
            state['dry_runs'].append(params[0])
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
        if self.path.startswith('/koios/account_info'):
            [stake] = json.loads(body)['_stake_addresses']
            row = self.state['stake_accounts'].get(stake)
            return self.reply([{'stake_address': stake, **row}] if row else [])
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
        self.journal = pathlib.Path(self.directory.name) / 'network.jsonl'
        Node.submitted.clear()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        assert not self.journal.exists() or not self.journal.read_text().strip(), self.journal.read_text()
        self.directory.cleanup()

    def run_cli(self, *args, success=True, env=None):
        p = subprocess.run([binary, '--data-dir', self.directory.name, '--json', *args], capture_output=True,
                           text=True, timeout=60,
                           env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(self.journal), **(env or {})})
        assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
        return json.loads(p.stdout) if success else (p.returncode, p.stdout + p.stderr)

    def endpoint(self, chain, api, path):
        self.run_cli('endpoints', '--chain', chain, '--api', api, '--capabilities',
                     'balance,fee,broadcast,verification', '--add', self.base + path)
        self.run_cli('endpoints', '--chain', chain, '--custom-only', 'true')

    def refused(self, wallet, destination, words):
        self.refuses(words, 'wallet', 'close', wallet, '--to', destination)

    def refuses(self, words, *args):
        code, output = self.run_cli(*args, success=False)
        assert code == 3 and words in output, (words, output)

    def test_xrp_deletes_the_account_once_nothing_blocks_it(self):
        sender = xrp['transaction']['Account']
        Node.state = {
            'ledger': 2_000,
            'xrp_accounts': {
                sender: {'Balance': '25000000', 'Sequence': 1_000, 'OwnerCount': 2, 'Flags': 0},
                XRP_DESTINATION: {'Balance': '50000000', 'Sequence': 5, 'OwnerCount': 0, 'Flags': 0},
            },
            'blockers': [],
        }
        self.run_cli('wallet', 'import', '--chain', 'xrp', '--name', 'XRP', '--no-password',
                     '--path', xrp['derivation_path'], env={'SPECTRA_SEED': xrp['mnemonic']})
        self.endpoint('xrp', 'xrpl-json-rpc', '/xrpl')
        account = self.run_cli('wallet', 'account', 'XRP')
        assert account['closable'] and account['account']['totalReserve'] == '1.4', account
        accounts = Node.state['xrp_accounts']

        Node.state['blockers'] = [{'LedgerEntryType': 'RippleState'}]
        self.refused('XRP', XRP_DESTINATION, 'trust lines')
        Node.state['blockers'] = []
        Node.state['ledger'] = 1_255
        self.refused('XRP', XRP_DESTINATION, 'after ledger 1256')
        Node.state['ledger'] = 2_000
        accounts[XRP_DESTINATION]['Flags'] = 0x00020000
        self.refused('XRP', XRP_DESTINATION, 'destination tag')
        accounts[XRP_DESTINATION]['Flags'] = 0
        self.refused('XRP', 'rDsbeomae4FXwgQTJp9Rs64Qg9vDiTCdBv', 'not on the network')
        self.refused('XRP', sender, 'into itself')
        assert self.run_cli('send', 'list')['artifacts'] == [], 'a refusal builds nothing'

        built = self.run_cli('wallet', 'close', 'XRP', '--to', XRP_DESTINATION)['artifact']
        # Everything less one owner reserve as the fee; the reserve and the
        # two objects go with it.
        assert built['amount'] == '24.8' and built['recipient'] == XRP_DESTINATION, built
        assert built['operation'] == {'kind': 'close_account', 'destination': XRP_DESTINATION, 'reserve': '1.4',
                                      'removed_objects': 2, 'network_fee': '0.2'}, built
        assert json.loads(built['prepared_details']) == {
            'XrpAccountDelete': {'sequence': 1_000, 'fee_drops': 200_000}}, built
        # A blocker that appears before signing is refused again.
        Node.state['blockers'] = [{'LedgerEntryType': 'Escrow'}]
        code, output = self.run_cli('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                                    '--endpoint', self.base + '/xrpl', success=False)
        assert code == 3 and 'escrows' in output, output
        Node.state['blockers'] = []
        signed = self.run_cli('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                              '--endpoint', self.base + '/xrpl')['artifact']
        blob = json.loads(signed['signed_payload'])['tx_blob_hex']
        # AccountDelete (21), sequence 1000, fee 0.2 XRP, no Amount.
        assert blob.startswith('120015220000000024000003E8684000000000030D40'), blob
        self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.base + '/xrpl', '--yes')
        assert Node.submitted == [blob], Node.submitted
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            rows = [json.loads(row[0]) for row in db.execute('SELECT payload FROM history_records')]
        assert [(row['kind'], row['amount']) for row in rows] == [('send', '24.8')], rows

    def test_stellar_merges_the_account_as_the_sdk_signs_it(self):
        sender, destination = merge['source'], merge['destination']
        def account(sequence, subentries=0, sponsoring=0, data=None):
            return {'balances': [{'balance': '5.0000000', 'asset_type': 'native'}], 'sequence': sequence,
                    'subentry_count': subentries, 'num_sponsoring': sponsoring, 'num_sponsored': 0,
                    'flags': {'auth_immutable': False}, 'data': data or {}}
        Node.state = {'ledger': 1_000, 'stellar_accounts': {
            sender: account(str(int(merge['sequence']) - 1)), destination: account('1')}}
        self.run_cli('wallet', 'import', '--chain', 'stellar-testnet', '--name', 'XLM', '--no-password',
                     '--private-key-env', 'CLOSING_KEY',
                     env={'CLOSING_KEY': stellar_secret(bytes.fromhex(merge['seed']))})
        self.endpoint('stellar-testnet', 'horizon', '/horizon')
        assert self.run_cli('wallet', 'account', 'XLM')['closable']

        accounts = Node.state['stellar_accounts']
        accounts[sender]['subentry_count'] = 1
        self.refused('XLM', destination, 'trust lines')
        accounts[sender]['subentry_count'] = 0
        accounts[sender]['num_sponsoring'] = 1
        self.refused('XLM', destination, 'sponsors')
        accounts[sender]['num_sponsoring'] = 0
        accounts[destination]['data'] = {'config.memo_required': 'MQ=='}
        self.refused('XLM', destination, 'memo')
        accounts[destination]['data'] = {}

        built = self.run_cli('wallet', 'close', 'XLM', '--to', destination)['artifact']
        assert built['amount'] == '4.99999' and built['operation']['reserve'] == '1', built
        signed = self.run_cli('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                              '--endpoint', self.base + '/horizon')['artifact']
        assert json.loads(signed['signed_payload'])['signed_xdr_b64'] == merge['envelope_b64'], signed
        self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.base + '/horizon', '--yes')
        assert Node.submitted == [merge['envelope_b64']], Node.submitted

    def test_near_function_call_keys_are_listed_and_deleted_and_nothing_else(self):
        signer = ed25519(NEAR_IMPLICIT)
        other_full, dapp, unlimited = ed25519('11' * 32), ed25519('33' * 32), ed25519('44' * 32)
        def key(public_key, permission):
            return {'public_key': public_key, 'access_key': {'nonce': 7, 'permission': permission}}
        Node.state = {'account': NEAR_IMPLICIT, 'keys': [
            key(dapp, {'FunctionCall': {'allowance': '250000000000000000000000', 'receiver_id': 'zapp.near',
                                        'method_names': ['play']}}),
            key(other_full, 'FullAccess'),
            key(unlimited, {'FunctionCall': {'allowance': None, 'receiver_id': 'app.near', 'method_names': []}}),
            key(signer, 'FullAccess'),
        ]}
        self.run_cli('wallet', 'import', '--chain', 'near', '--name', 'NEAR', '--no-password', env={'SPECTRA_SEED': PHRASE})
        self.endpoint('near', 'near-json-rpc', '/near')
        assert any(a['action'] == 'accessKeys' for a in self.run_cli('wallet', 'actions', 'NEAR')['actions']['actions'])
        keys = self.run_cli('wallet', 'keys', 'NEAR')['keys']['keys']
        # The signing key first, then full access, then function-call keys by contract.
        assert [(k['publicKey'], k['signs'], k['fullAccess'], k['receiver'], k['allowance']) for k in keys] == [
            (signer, True, True, None, None), (other_full, False, True, None, None),
            (unlimited, False, False, 'app.near', None), (dapp, False, False, 'zapp.near', '0.25')], keys

        self.refuses('full-access', 'wallet', 'delete-key', 'NEAR', '--key', other_full)
        self.refuses('full-access', 'wallet', 'delete-key', 'NEAR', '--key', signer)
        self.refuses('not a key', 'wallet', 'delete-key', 'NEAR', '--key', ed25519('55' * 32))
        assert self.run_cli('send', 'list')['artifacts'] == [], 'a refusal builds nothing'
        built = self.run_cli('wallet', 'delete-key', 'NEAR', '--key', dapp)['artifact']
        costs = NEAR_CONFIG['runtime_config']['transaction_costs']
        parts = [costs['action_receipt_creation_config'], costs['action_creation_config']['delete_key_cost']]
        price = 100000000
        fee = sum(p['send_sir'] for p in parts) * price + sum(p['execution'] for p in parts) * max(
            price, int(NEAR_CONFIG['runtime_config']['min_gas_purchase_price']))
        display = format(decimal.Decimal(fee) / 10 ** 24, 'f').rstrip('0').rstrip('.')
        assert built['operation'] == {'kind': 'delete_access_key', 'public_key': dapp, 'receiver': 'zapp.near',
                                      'network_fee': display}, built
        assert built['recipient'] == built['sender'] == NEAR_IMPLICIT and built['amount'] == '0', built
        signed = self.run_cli('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                              '--endpoint', self.base + '/near')['artifact']
        raw = base64.b64decode(json.loads(signed['signed_payload'])['signed_tx_b64'])
        # One DeleteKey action (6) for the dapp's Ed25519 key, then the signature.
        assert raw[-65 - 38:-65] == bytes([1, 0, 0, 0, 6, 0]) + bytes.fromhex('33' * 32), raw.hex()
        self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.base + '/near', '--yes')
        assert Node.submitted == [json.loads(signed['signed_payload'])['signed_tx_b64']], Node.submitted
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            kinds = [json.loads(row[0])['kind'] for row in db.execute('SELECT payload FROM history_records')]
        assert kinds == ['deleteAccessKey'], kinds

    def test_a_named_near_account_signs_with_its_recorded_key(self):
        signer = ed25519(NEAR_IMPLICIT)
        Node.state = {'account': 'alice.near', 'keys': [
            {'public_key': ed25519('11' * 32), 'access_key': {'nonce': 3, 'permission': 'FullAccess'}},
            {'public_key': signer, 'access_key': {'nonce': 7, 'permission': 'FullAccess'}},
        ]}
        self.endpoint('near', 'near-json-rpc', '/near')
        self.run_cli('wallet', 'import', '--chain', 'near', '--named-account', 'alice.near', '--name', 'Named',
                     '--no-password', env={'SPECTRA_SEED': PHRASE})
        keys = self.run_cli('wallet', 'keys', 'Named')['keys']['keys']
        # Two full-access keys: the one the phrase derives is the one it signs with.
        assert [(k['publicKey'], k['signs']) for k in keys][0] == (signer, True), keys
        assert not keys[1]['signs'], keys

    def test_near_token_storage_is_refunded_only_beside_an_empty_balance(self):
        signer = ed25519(NEAR_IMPLICIT)
        registered = {'total': str(NEAR_DEPOSIT), 'available': '0'}
        Node.state = {'account': NEAR_IMPLICIT, 'calls': [],
                      'keys': [{'public_key': signer, 'access_key': {'nonce': 7, 'permission': 'FullAccess'}}],
                      # The indexer's inventory, emptied tokens included.
                      'inventory': [('empty.near', '0'), ('held.near', '5'), ('unregistered.near', '0'),
                                    ('nostorage.near', '0')],
                      'tokens': {
                          'empty.near': {'storage_balance_of': registered, 'ft_balance_of': '0'},
                          'held.near': {'storage_balance_of': registered, 'ft_balance_of': '5'},
                          'unregistered.near': {'storage_balance_of': None, 'ft_balance_of': '0'},
                          'nostorage.near': {'ft_balance_of': '0'},
                          'holding.near': {'storage_balance_of': {'total': str(2 * NEAR_DEPOSIT), 'available': '0'},
                                           'ft_balance_of': '0'},
                          'history.near': {'storage_balance_of': registered, 'ft_balance_of': '0'},
                      }}
        self.run_cli('wallet', 'import', '--chain', 'near', '--name', 'NEAR', '--no-password', env={'SPECTRA_SEED': PHRASE})
        self.endpoint('near', 'near-json-rpc', '/near')
        self.run_cli('endpoints', '--chain', 'near', '--api', 'nearblocks', '--capabilities',
                     'history,token-history,token-discovery', '--add', self.base + '/nearblocks')
        assert any(a['action'] == 'tokenStorage' for a in self.run_cli('wallet', 'actions', 'NEAR')['actions']['actions'])
        # A holding the wallet tracks and a token its history names, which the
        # inventory does not.
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            wid, payload = db.execute('SELECT id, payload FROM wallets').fetchone()
            wallet = json.loads(payload)
            wallet['holdings'].append(dict(name='Holding', symbol='HOLD', coingeckoId='', chainId='near',
                                           tokenStandard='NEP-141', contractAddress='holding.near', amount='0'))
            db.execute('UPDATE wallets SET payload=? WHERE id=?', (json.dumps(wallet), wid))
            record = dict(id='received', walletId=wid, kind='receive', status='confirmed', walletName='NEAR',
                          assetDisplayName='History', symbol='HIST', chainId='near', amount='1',
                          address=NEAR_IMPLICIT, deploymentId='near:nep-141:history.near', createdAtUnix=1.0)
            db.execute('INSERT INTO history_records (id,wallet_id,chain_id,tx_hash,created_at,payload) '
                       'VALUES (?,?,?,?,?,?)', ('received', wid, 'near', None, 1.0, json.dumps(record)))
        storage = self.run_cli('wallet', 'token-storage', 'NEAR')['storage']
        assert [(d['contract'], d['symbol'], d['refund']) for d in storage['deposits']] == [
            ('empty.near', 'empty.near', '0.00125'), ('history.near', 'history.near', '0.00125'),
            ('holding.near', 'HOLD', '0.0025')], storage
        assert storage['refundable'] == '0.005' and storage['account'] == NEAR_IMPLICIT, storage

        self.refuses('still holds this token', 'wallet', 'refund-storage', 'NEAR', '--contract', 'held.near')
        self.refuses('holds no storage deposit', 'wallet', 'refund-storage', 'NEAR', '--contract', 'unregistered.near')
        self.refuses('holds no storage deposit', 'wallet', 'refund-storage', 'NEAR', '--contract', 'nostorage.near')
        self.refuses('not a NEAR token contract', 'wallet', 'refund-storage', 'NEAR', '--contract', 'Not A Contract')
        assert self.run_cli('send', 'list')['artifacts'] == [], 'a refusal builds nothing'

        built = self.run_cli('wallet', 'refund-storage', 'NEAR', '--contract', 'empty.near')['artifact']
        costs = NEAR_CONFIG['runtime_config']['transaction_costs']
        actions = costs['action_creation_config']
        count = len('storage_unregister') + len('{}')
        parts = [costs['action_receipt_creation_config'], actions['function_call_cost'],
                 {key: value * count for key, value in actions['function_call_cost_per_byte'].items()}]
        price = 100000000
        fee = sum(p['send_not_sir'] for p in parts) * price + (30000000000000 + sum(
            p['execution'] for p in parts)) * max(price, int(NEAR_CONFIG['runtime_config']['min_gas_purchase_price']))
        display = format(decimal.Decimal(fee) / 10 ** 24, 'f').rstrip('0').rstrip('.')
        assert built['operation'] == {'kind': 'refund_token_storage', 'contract': 'empty.near', 'refund': '0.00125',
                                      'network_fee': display}, built
        assert built['recipient'] == built['sender'] == NEAR_IMPLICIT and built['amount'] == '0.00125', built
        sign = ('send', 'sign', built['id'], '--review-digest', built['review_digest'], '--endpoint', self.base + '/near')
        # A token that arrives before signing stops it: the contract would refuse.
        Node.state['tokens']['empty.near']['ft_balance_of'] = '1'
        code, output = self.run_cli(*sign, success=False)
        assert 'still holds this token' in output, output
        Node.state['tokens']['empty.near']['ft_balance_of'] = '0'
        signed = self.run_cli(*sign)['artifact']
        raw = base64.b64decode(json.loads(signed['signed_payload'])['signed_tx_b64'])
        # To the token: one FunctionCall (2), storage_unregister with `{}`,
        # 30 Tgas and one yoctoNEAR, then the signature.
        call = (bytes([1, 0, 0, 0, 2]) + len('storage_unregister').to_bytes(4, 'little') + b'storage_unregister'
                + (2).to_bytes(4, 'little') + b'{}' + (30000000000000).to_bytes(8, 'little') + (1).to_bytes(16, 'little'))
        assert raw[:-65].endswith(call) and b'empty.near' in raw and b'force' not in raw, raw.hex()
        self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.base + '/near', '--yes')
        assert Node.submitted == [json.loads(signed['signed_payload'])['signed_tx_b64']], Node.submitted
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            kinds = sorted(json.loads(row[0])['kind'] for row in db.execute('SELECT payload FROM history_records'))
        assert kinds == ['receive', 'refundTokenStorage'], kinds

        # A watched account builds nothing.
        self.run_cli('wallet', 'watch', '--chain', 'near', '--name', 'Watched', '--address', '22' * 32)
        self.refuses('watch-only', 'wallet', 'refund-storage', 'Watched', '--contract', 'history.near')

    def test_a_cardano_account_shows_its_stake_address_rewards_and_delegation(self):
        phrase, script = CARDANO['phrase_vectors'][0], CARDANO['address_vectors'][2]
        pool = 'pool1pu5jlj4q9w9jlxeu370a3c9myx47md5j5m2str0naunn2q3lkdy'
        Node.state = {'stake_accounts': {phrase['stake_address']: {
            'status': 'registered', 'delegated_pool': pool, 'delegated_drep': 'drep_always_abstain',
            'total_balance': '9000000', 'rewards': '2000000', 'withdrawals': '500000',
            'rewards_available': '1500000', 'deposit': '2000000'}}}
        self.run_cli('wallet', 'import', '--chain', 'cardano', '--name', 'ADA', '--no-password',
                     env={'SPECTRA_SEED': CARDANO['phrase']})
        self.endpoint('cardano', 'koios', '/koios')
        assert any(a['action'] == 'networkAccount' for a in self.run_cli('wallet', 'actions', 'ADA')['actions']['actions'])
        account = self.run_cli('wallet', 'account', 'ADA')['account']
        assert account == {'kind': 'cardano', 'stakeAddress': phrase['stake_address'], 'registered': True,
                           'rewards': '1.5', 'delegatedPool': pool, 'delegatedDrep': 'drep_always_abstain'}, account
        # A watched base address with a script's stake credential names a
        # type-15 stake address, which the chain has not seen.
        self.run_cli('wallet', 'watch', '--chain', 'cardano', '--name', 'Script', '--address', script['address'])
        assert self.run_cli('wallet', 'account', 'Script')['account'] == {
            'kind': 'cardano', 'stakeAddress': script['stake_address'], 'registered': False, 'rewards': '0',
            'delegatedPool': None, 'delegatedDrep': None}
        # A raw key derives an enterprise address, which names no stake key.
        self.run_cli('wallet', 'import', '--chain', 'cardano', '--name', 'Key', '--no-password',
                     '--private-key-env', 'CARDANO_KEY', env={'CARDANO_KEY': CARDANO_KEY})
        assert not any(a['action'] == 'networkAccount'
                       for a in self.run_cli('wallet', 'actions', 'Key')['actions']['actions'])
        self.refuses('names no stake key', 'wallet', 'account', 'Key')

    def test_sui_objects_merge_by_type_with_gas_apart(self):
        Node.state = {'dry_runs': []}
        self.run_cli('wallet', 'import', '--chain', 'sui', '--name', 'SUI', '--no-password', env={'SPECTRA_SEED': PHRASE})
        self.endpoint('sui', 'sui-json-rpc', '/sui')
        types = self.run_cli('wallet', 'objects', 'SUI')['types']
        assert [(t['coinType'], t['objects'], t['balance'], t['mergeable']) for t in types] == [
            ('0x2::sui::SUI', 3, '0.012', True), (SUI_TOKEN, 3, '3', True), (SUI_SINGLE, 1, '0.000005', False)], types
        self.refuses('nothing to merge', 'wallet', 'merge', 'SUI', '--coin-type', SUI_SINGLE)
        # The budget is the dry run's computation and storage with a fifth to spare.
        for coin_type, gas, merged in [(SUI_TOKEN, 1, 3), ('0x2::sui::SUI', 3, 0)]:
            built = self.run_cli('wallet', 'merge', 'SUI', '--coin-type', coin_type)['artifact']
            assert built['operation'] == {'kind': 'merge_coins', 'coin_type': coin_type, 'objects': 3,
                                          'network_fee': '0.0036'}, built
            prepared = json.loads(built['prepared_details'])['SuiMerge']
            assert (len(prepared['gas']), len(prepared['merged'])) == (gas, merged), prepared
            assert prepared['transaction']['gas_budget'] == 3600000, prepared
            signed = self.run_cli('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                                  '--endpoint', self.base + '/sui')['artifact']
            self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.base + '/sui', '--yes')
            assert Node.submitted[-1] == json.loads(signed['signed_payload'])['tx_bytes_b64'], Node.submitted
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            kinds = [json.loads(row[0])['kind'] for row in db.execute('SELECT payload FROM history_records')]
        assert kinds == ['mergeCoins', 'mergeCoins'], kinds

    def test_solana_closes_empty_token_accounts_the_network_would_close(self):
        spl, t22 = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA', 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb'
        owner = self.run_cli('wallet', 'import', '--chain', 'solana', '--name', 'SOL', '--no-password',
                             env={'SPECTRA_SEED': PHRASE})['wallet']['address']
        def account(n, program, amount=0, lamports=2039280, **info):
            return {'program': program, 'pubkey': b58(bytes([n] * 32)), 'account': {'lamports': lamports, 'data': {
                'parsed': {'info': {'mint': b58(bytes([n + 1] * 32)), 'owner': owner, 'state': 'initialized',
                                    'tokenAmount': {'amount': str(amount), 'decimals': 6}, **info}}}}}
        closable, withheld, held, foreign, closable_2022 = (
            account(0x11, spl), account(0x33, t22, extensions=[
                {'extension': 'transferFeeAmount', 'state': {'withheldAmount': 5}}]),
            account(0x44, spl, amount=7), account(0x55, spl, closeAuthority=b58(bytes([9] * 32))),
            account(0x22, t22, lamports=2074080))
        Node.state = {'owner': owner, 'token_accounts': [closable, withheld, held, foreign, closable_2022]}
        self.endpoint('solana', 'solana-json-rpc', '/solana')
        empty = self.run_cli('wallet', 'token-accounts', 'SOL')['empty']
        assert [(a['address'], a['token2022'], a['rent'], a['blocked'] is None) for a in empty['accounts']] == [
            (closable['pubkey'], False, '0.00203928', True), (foreign['pubkey'], False, '0.00203928', False),
            (withheld['pubkey'], True, '0.00203928', False), (closable_2022['pubkey'], True, '0.00207408', True)], empty
        assert empty['reclaimable'] == '0.00411336', empty
        for address, words in [(withheld['pubkey'], 'withheld'), (held['pubkey'], 'holds tokens'),
                               (foreign['pubkey'], 'close authority'), (b58(bytes([0x66] * 32)), 'not a token account')]:
            self.refuses(words, 'wallet', 'close-token-accounts', 'SOL', '--account', address)
        assert self.run_cli('send', 'list')['artifacts'] == [], 'a refusal builds nothing'
        built = self.run_cli('wallet', 'close-token-accounts', 'SOL')['artifact']
        assert built['operation'] == {'kind': 'close_token_accounts',
                                      'accounts': [closable['pubkey'], closable_2022['pubkey']],
                                      'rent': '0.00411336', 'network_fee': '0.000005'}, built
        assert built['amount'] == '0.00411336' and built['recipient'] == owner, built
        signed = self.run_cli('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                              '--endpoint', self.base + '/solana')['artifact']
        # One signature, then exactly the reviewed message.
        raw = base64.b64decode(signed['signed_payload'])
        assert raw[65:].hex() == built['signing_payload_hex'], raw.hex()
        self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.base + '/solana', '--yes')
        assert len(Node.submitted) == 1, Node.submitted
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            kinds = [json.loads(row[0])['kind'] for row in db.execute('SELECT payload FROM history_records')]
        assert kinds == ['closeTokenAccounts'], kinds

    def test_other_networks_and_watched_accounts_do_not_close(self):
        phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
        self.run_cli('wallet', 'import', '--chain', 'solana', '--name', 'SOL', '--no-password',
                     env={'SPECTRA_SEED': phrase})
        self.refused('SOL', 'BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX', 'cannot be closed')
        self.run_cli('wallet', 'watch', '--chain', 'xrp', '--name', 'Watched', '--address', XRP_DESTINATION)
        self.refused('Watched', xrp['transaction']['Account'], 'watch-only')


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
