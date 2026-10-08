#!/usr/bin/env python3
"""Wallet import, naming, receive addresses and password validation.

Run: python3 scripts/cli-wallets.py [path/to/spectra] [TestClass.test_name]
Uses temporary stores and loopback nodes; no public network is required.
"""
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

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else
                          pathlib.Path(__file__).resolve().parents[1] / 'target/debug/spectra').resolve())


class WalletsTests(unittest.TestCase):
    def test_evm_custom_paths_match_signing_identity(self):
        """An L2's explicit path controls both import and signing after restart."""
        with tempfile.TemporaryDirectory(prefix='spectra-evm-path-') as directory:
            root = pathlib.Path(directory)
            seed = root / 'seed.txt'
            seed.write_text('test test test test test test test test test test test junk')
            def run(*args):
                p = subprocess.run([binary, '--data-dir', str(root / 'state'), '--json', *args], capture_output=True, text=True, timeout=60)
                assert p.returncode == 0, (args, p.stdout, p.stderr)
                return json.loads(p.stdout)
            path = "m/44'/60'/3'/0/0"
            expected = run('wallet', 'import', '--chain', 'ethereum', '--path', path,
                           '--seed-file', str(seed), '--no-password', '--name', 'Ethereum')['wallet']['address']
            for chain in ['arbitrum', 'base', 'ethereum-sepolia']:
                imported = run('wallet', 'import', '--chain', chain, '--path', path,
                               '--seed-file', str(seed), '--no-password', '--name', chain)['wallet']
                assert imported['address'] == expected, imported
                identity = run('send', 'identity', '--from', chain)
                assert identity['address'] == expected, identity

    def test_new_evm_networks_derive_and_sign_their_own_chain_ids(self):
        """Shared EVM keys retain network identity, including in signed transaction bytes."""
        live = {'chain_id': 1, 'balance': 10 * 10**18}
        seen = []
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                def answer(call):
                    method = call['method']; seen.append(method)
                    if method == 'eth_call':
                        selector = call['params'][0]['data'][2:10]
                        amounts = {'f1c7a58b': 1000, '275aedd2': 1000,
                                   '70a08231': 10**24, '313ce567': 18}
                        if selector == '95d89b41':
                            value = '0x' + 'TOK'.encode().hex().ljust(64, '0')
                        else:
                            assert selector in amounts, call
                            value = '0x' + f'{amounts[selector]:064x}'
                        return {'jsonrpc': '2.0', 'id': call['id'], 'result': value}
                    values = {'eth_chainId': hex(live['chain_id']),
                              'eth_getBalance': hex(live['balance']),
                              'eth_getTransactionCount': '0x7',
                              'eth_estimateGas': '0x5208', 'eth_getCode': '0x',
                              'eth_gasPrice': '0xb2d05e00',
                              'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'],
                                                 'reward': [['0x77359400']]}}
                    assert method in values, method
                    return {'jsonrpc': '2.0', 'id': call['id'], 'result': values[method]}
                result = list(map(answer, body)) if isinstance(body, list) else answer(body)
                data = json.dumps(result).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(data)))
                self.end_headers(); self.wfile.write(data)
        def signed_chain_id(payload):
            encoded = bytes.fromhex(payload.removeprefix('0x'))
            assert encoded[0] == 2, encoded
            # EIP-1559's first RLP-list field is the replay-protection chain ID.
            prefix = encoded[1]
            assert prefix >= 0xc0, prefix
            start = 2 if prefix <= 0xf7 else 2 + prefix - 0xf7
            prefix = encoded[start]
            if prefix < 0x80:
                return prefix
            assert prefix <= 0xb7, prefix
            return int.from_bytes(encoded[start + 1:start + 1 + prefix - 0x80], 'big')
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        try:
            for chain, chain_id, symbol in [('plasma', 9745, 'XPL'), ('monad', 143, 'MON'),
                                            ('world-chain', 480, 'ETH')]:
                with self.subTest(chain=chain), tempfile.TemporaryDirectory(prefix=f'spectra-{chain}-') as directory:
                    root = pathlib.Path(directory)
                    seed = root/'seed.txt'
                    seed.write_text('abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about')
                    journal = root/'network.jsonl'
                    def run(*args, success=True):
                        result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                                capture_output=True, text=True, timeout=60,
                                                env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(journal)})
                        assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
                        return json.loads(result.stdout)
                    imported = run('wallet', 'import', '--chain', chain, '--name', 'Sender',
                                   '--seed-file', str(seed), '--no-password')['wallet']
                    address = '0x9858effd232b4033e47d90003d41ec34ecaeda94'
                    assert imported['chain'] == chain and imported['address'].lower() == address, imported
                    assert imported['derivationPath'] == "m/44'/60'/0'/0/0", imported
                    identity = run('send', 'identity', '--from', 'Sender', '--chain', chain)
                    assert identity['chain'] == chain and identity['address'].lower() == address, identity
                    assert run('wallet', 'receive', 'Sender')['address'].lower() == address
                    endpoint = f'http://127.0.0.1:{server.server_port}'
                    build = ('send', 'build', '--from', 'Sender', '--to', '0x' + '22' * 20,
                             '--amount', '0.01', '--endpoint', endpoint)
                    live.update(chain_id=1, balance=10 * 10**18)
                    seen.clear()
                    run(*build, success=False)
                    with sqlite3.connect(root/'spectra.sqlite') as db:
                        assert db.execute('SELECT COUNT(*) FROM send_artifacts').fetchone()[0] == 0
                    live['chain_id'] = chain_id
                    prepared = run(*build)['artifact']
                    details = json.loads(prepared['prepared_details'])['Evm']
                    assert details['chain_id'] == chain_id and details['value_wei'] == 10**16, details
                    assert prepared['chain_id'] == chain and prepared['symbol'] == symbol, prepared
                    sign = ('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                            '--endpoint', endpoint)
                    maximum_fee = details['gas_limit'] * details['max_fee_per_gas'] + details['additional_fee_wei']
                    if chain == 'world-chain':
                        assert details['additional_fee_wei'] > 0, details
                    # Cover an empty account, transfer-only funds and a one-wei budget shortfall.
                    for balance in [0, details['value_wei'], details['value_wei'] + maximum_fee - 1]:
                        live['balance'] = balance
                        run(*sign, success=False)
                        refused = run('send', 'inspect', prepared['id'])['artifact']
                        assert refused['stage'] == 'Prepared' and not refused['signed_payload'], refused
                    live['balance'] = 10 * 10**18
                    signed = run(*sign)['artifact']
                    assert signed['stage'] == 'Signed' and signed_chain_id(signed['signed_payload']) == chain_id, signed
                    assert run('send', 'inspect', signed['id'])['artifact'] == signed
                    assert 'eth_sendRawTransaction' not in seen, seen
                    if journal.exists():
                        assert not journal.read_text(), journal.read_text()
        finally:
            server.shutdown(); server.server_close(); worker.join()

    def test_derivation_inputs(self):
        """Preserve passphrases and reject invalid imports without saving wallets."""
        with tempfile.TemporaryDirectory(prefix='spectra-import-') as directory:
            root = pathlib.Path(directory)
            seed = root/'seed.txt'
            seed.write_text('abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about')
            def run(*args, success=True):
                p = subprocess.run([binary, '--data-dir', str(root/'state'), '--json', *args], capture_output=True, text=True, timeout=60)
                assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                return json.loads(p.stdout) if success else None
            def import_with(name, fields, success=True, chain='ethereum'):
                path = root/'input.json'; path.write_text(json.dumps(fields))
                return run('wallet', 'import', '--chain', chain, '--name', name, '--seed-file', str(seed), '--no-password', '--derivation-input-file', str(path), success=success)
            spaced = import_with('Spaced', {'passphrase':' secret '})['wallet']
            trimmed = import_with('Trimmed', {'passphrase':'secret'})['wallet']
            assert spaced['address'] != trimmed['address'], 'passphrase whitespace changed silently'
            run('send','identity','--from','Spaced')
            run('send','identity','--from','Trimmed')
            for fields in [{'iterationCount':s} for s in ['abc','0','4294967296','2048']] + [{'curve':'ed25519'}, {'saltPrefix':'custom'}, {'hmacKey':'custom'}]:
                import_with('Refused', fields, success=False)
            import_with('Monero refusal', {'passphrase':'secret'}, success=False, chain='monero')
            assert len(run('wallet','list')['wallets']) == 2, 'rejected inputs persisted wallets'
            # The phrase as typed is read canonically: it is Trimmed's wallet,
            # so a second import of it is refused naming that wallet.
            seed.write_text('  ' + ' \t\n'.join(seed.read_text().upper().split()) + '  ')
            (root/'input.json').write_text(json.dumps({'passphrase':'secret'}))
            again = subprocess.run([binary, '--data-dir', str(root/'state'), '--json', 'wallet', 'import',
                                    '--chain', 'ethereum', '--name', 'Canonical mnemonic', '--seed-file', str(seed),
                                    '--no-password', '--derivation-input-file', str(root/'input.json')],
                                   capture_output=True, text=True, timeout=60)
            assert again.returncode == 3 and 'Trimmed' in again.stdout, (again.stdout, again.stderr)

    def test_monero_and_ton_take_their_own_phrases(self):
        """Monero and TON refuse BIP-39 and restore their own formats to the fixtures' addresses."""
        fixtures = pathlib.Path(__file__).resolve().parents[1] / 'core/tests/fixtures'
        monero = json.loads((fixtures / 'monero-phrases.json').read_text())
        ton = json.loads((fixtures / 'ton-mnemonics.json').read_text())
        with tempfile.TemporaryDirectory(prefix='spectra-phrases-') as directory:
            root = pathlib.Path(directory)
            def run(*args, phrase=None, success=True):
                env = {**os.environ, 'SPECTRA_SEED': phrase or ''}
                p = subprocess.run([binary, '--data-dir', str(root / 'state'), '--json', *args],
                                   capture_output=True, text=True, timeout=60, env=env)
                assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                return json.loads(p.stdout) if success else p
            bip39 = 'abandon ' * 11 + 'about'
            for chain in ['monero', 'ton']:
                refused = run('wallet', 'import', '--chain', chain, '--no-password', phrase=bip39, success=False)
                assert refused.returncode == 3, refused
            assert run('wallet', 'list')['wallets'] == [], 'a refused phrase stored a wallet'
            electrum = monero['electrum'][2]
            wallet = run('wallet', 'import', '--chain', 'monero', '--name', 'Electrum', '--no-password',
                         phrase=electrum['phrase'])['wallet']
            assert wallet['address'] == electrum['address'] and wallet['restoreHeight'] == 0, wallet
            polyseed = monero['polyseed'][0]
            wallet = run('wallet', 'import', '--chain', 'monero', '--name', 'Polyseed', '--no-password',
                         phrase=polyseed['phrase'])['wallet']
            assert wallet['address'] == polyseed['address'], wallet
            # The birthday's checkpoint: before the wallet was made, after Polyseed's epoch.
            assert 2_480_000 <= wallet['restoreHeight'] < 2_480_000 + (polyseed['created'] - 1_635_000_000) // 120, wallet
            # W5 unless the import names another version; ton-mnemonics.json
            # records each mnemonic's v4R2 account, ton-w5.json its W5 one.
            mnemonic = ton['mnemonics'][0]
            w5 = json.loads((fixtures / 'ton-w5.json').read_text())['addresses'][0]
            assert w5['mnemonic'] == mnemonic['mnemonic'], w5
            wallet = run('wallet', 'import', '--chain', 'ton', '--name', 'TON', '--no-password',
                         phrase=mnemonic['mnemonic'])['wallet']
            assert wallet['address'] == w5['mainnet'], wallet
            wallet = run('wallet', 'import', '--chain', 'ton', '--name', 'TON v4R2', '--no-password',
                         '--ton-wallet', 'v4R2', phrase=mnemonic['mnemonic'])['wallet']
            assert wallet['address'] == mnemonic['address'], wallet
            for chain, words in [('monero', 25), ('ton', 24)]:
                created = run('wallet', 'new', '--chain', chain, '--name', 'New ' + chain, '--no-password')
                assert len(created['seedPhrase'].split()) == words, created
            assert run('wallet', 'show', 'New monero')['wallet']['restoreHeight'] > 3_700_000
            checked = run('wallet', 'check-seed', '--chain', 'monero', phrase=polyseed['phrase'])
            assert checked['isValid'] and checked['format'] == 'polyseed', checked

    def test_private_keys_import_in_each_chains_own_encoding(self):
        """WIF, Solana keypairs, Stellar seeds, Sui, Aptos and NEAR keys import; wrong ones are refused."""
        fixtures = json.loads((pathlib.Path(__file__).resolve().parents[1]
                               / 'core/tests/fixtures/private-key-formats.json').read_text())
        with tempfile.TemporaryDirectory(prefix='spectra-keys-') as directory:
            root = pathlib.Path(directory)
            def run(chain, name, key, success=True):
                env = {**os.environ, 'COVERAGE_KEY': key}
                p = subprocess.run([binary, '--data-dir', str(root / 'state'), '--json', 'wallet', 'import',
                                    '--chain', chain, '--name', name, '--no-password',
                                    '--private-key-env', 'COVERAGE_KEY'],
                                   capture_output=True, text=True, timeout=60, env=env)
                assert (p.returncode == 0) == success, (chain, name, p.stdout, p.stderr)
                return json.loads(p.stdout)['wallet'] if success else p.returncode
            for name, chain in [('stellar', 'stellar'), ('sui', 'sui'), ('aptos', 'aptos')]:
                vector = fixtures[name]
                assert run(chain, name, vector['secret'])['address'] == vector['address'], name
            solana = fixtures['solana']
            assert run('solana', 'solana-base58', solana['base58'])['address'] == solana['address']
            # The JSON keypair is the same key: it is that wallet, and refused.
            assert run('solana', 'solana-json', solana['json'], success=False) == 3
            near = fixtures['near']
            assert run('near', 'near', near['secret'])['address'] == near['implicit_account']
            assert run('near', 'near-mismatched', fixtures['near_mismatched'], success=False) == 3
            for vector in fixtures['wif']:
                if vector['chain'] not in ('bitcoin', 'litecoin', 'dogecoin-testnet'):
                    continue
                chain = vector['chain']
                run(chain, chain + '-wif', vector['compressed'])
                # The key's hex is the same key: that wallet, so refused.
                assert run(chain, chain + '-hex', vector['key'], success=False) == 3, chain
                assert run(chain, chain + '-uncompressed', vector['uncompressed'], success=False) == 3
            mainnet = next(v for v in fixtures['wif'] if v['chain'] == 'bitcoin')
            assert run('bitcoin-testnet', 'wrong-network', mainnet['compressed'], success=False) == 3
            assert run('ethereum', 'wif-on-ethereum', mainnet['compressed'], success=False) == 3

    def test_derivation_profiles_choose_the_account_and_refuse_what_no_profile_is(self):
        """A profile and account import the independent vector's address and survive reopening; bad choices exit before storing."""
        fixture = json.loads((pathlib.Path(__file__).resolve().parents[1]
                              / 'core/tests/fixtures/derivation-profiles.json').read_text())
        def vector(chain, profile, account):
            return next(v for v in fixture['vectors']
                        if (v['chain'], v['profile'], v['account']) == (chain, profile, account))
        with tempfile.TemporaryDirectory(prefix='spectra-profiles-') as directory:
            root = pathlib.Path(directory)
            def run(*args, success=True):
                env = {**os.environ, 'SPECTRA_SEED': fixture['phrase']}
                p = subprocess.run([binary, '--data-dir', str(root / 'state'), '--json', *args],
                                   capture_output=True, text=True, timeout=60, env=env)
                assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                return json.loads(p.stdout) if success else p.returncode
            for chain, profile in [('bitcoin', 'taproot'), ('litecoin', 'nestedSegWit'), ('solana', 'legacy'), ('near', 'standard')]:
                expected = vector(chain, profile, 1)
                wallet = run('wallet', 'import', '--chain', chain, '--profile', profile, '--account', '1',
                             '--no-password', '--name', chain + ' ' + profile)['wallet']
                assert wallet['address'] == expected['address'], (wallet, expected)
                assert wallet['derivationPath'] == expected['path'], wallet
            # Reopened from disk, the account's path is the one stored.
            shown = run('wallet', 'show', 'bitcoin taproot')['wallet']
            assert shown['derivationPath'] == vector('bitcoin', 'taproot', 1)['path'], shown
            assert run('wallet', 'import', '--chain', 'bitcoin', '--profile', 'sideways',
                       '--no-password', success=False) == 2
            assert run('wallet', 'import', '--chain', 'ethereum', '--profile', 'taproot',
                       '--no-password', success=False) == 3
            assert run('wallet', 'import', '--chain', 'bitcoin', '--path', 'not a path',
                       '--no-password', success=False) == 3
            assert run('wallet', 'import', '--chain', 'polkadot', '--path', "m/44'/354'/1'",
                       '--no-password', success=False) == 3
            assert len(run('wallet', 'list')['wallets']) == 4

    def test_a_preview_shows_what_the_import_stores_and_stores_nothing(self):
        """Phrase, key and watched-account previews name the stored address without storing a wallet."""
        phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
        # BIP-84's own test vector: the account key and its first receive address.
        zpub = 'zpub6rFR7y4Q2AijBEqTUquhVz398htDFrtymD9xYYfG1m4wAcvPhXNfE3EfH1r1ADqtfSdVCToUG868RvUUkgDKf31mGDtKsAYz2oz2AGutZYs'
        first = 'bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu'
        with tempfile.TemporaryDirectory(prefix='spectra-preview-') as directory:
            root = pathlib.Path(directory)
            def run(*args, env=None, success=True):
                p = subprocess.run([binary, '--data-dir', str(root / 'state'), '--json', *args],
                                   capture_output=True, text=True, timeout=60, env={**os.environ, **(env or {})})
                assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                return json.loads(p.stdout) if success else p.returncode
            seed = {'SPECTRA_SEED': phrase}
            previewed = run('wallet', 'import', '--chain', 'bitcoin', '--preview', env=seed)
            assert previewed['addresses'] == [first], previewed
            assert run('wallet', 'watch', '--chain', 'bitcoin', '--xpub', zpub, '--preview')['addresses'] == [first]
            key = {'PREVIEW_KEY': '4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318'}
            keyed = run('wallet', 'import', '--chain', 'ethereum', '--private-key-env', 'PREVIEW_KEY', '--preview', env=key)
            watched = run('wallet', 'watch', '--chain', 'ethereum', '--address', keyed['addresses'][0],
                          '--address', 'not-an-address', '--preview')
            assert watched['addresses'] == keyed['addresses'] and watched['rejectedAddresses'] == ['not-an-address'], watched
            assert run('wallet', 'list')['wallets'] == []
            assert run('wallet', 'import', '--chain', 'monero', '--preview', env=seed, success=False) == 3
            stored = run('wallet', 'import', '--chain', 'bitcoin', '--no-password', env=seed)['wallet']
            assert stored['address'] == first, stored

    def test_a_signing_import_upgrades_a_watched_wallet_and_other_duplicates_are_refused(self):
        """The upgraded wallet keeps its id, name and history, signs, survives reopening; every other duplicate exits 3."""
        phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
        address = '0x9858effd232b4033e47d90003d41ec34ecaeda94'
        chain, chain_id = 'ethereum-sepolia', 11155111
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def reply(self, payload):
                data = json.dumps(payload).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(data)))
                self.end_headers(); self.wfile.write(data)
            def do_GET(self):
                # A Blockscout-style indexer: one received transfer.
                query = dict(part.split('=', 1) for part in self.path.split('?', 1)[1].split('&'))
                assert query['address'] == address, self.path
                rows = [] if query['action'] != 'txlist' else [{
                    'hash': '0x' + 'ab' * 32, 'blockNumber': '42', 'timeStamp': '1700000000',
                    'from': '0x' + '22' * 20, 'to': address, 'value': '1250000000000000000',
                    'gasPrice': '1000000000', 'gasUsed': '21000', 'isError': '0', 'txreceipt_status': '1'}]
                self.reply({'status': '1', 'message': 'OK', 'result': rows})
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                values = {'eth_chainId': hex(chain_id), 'eth_getBalance': hex(10 * 10**18),
                          'eth_getTransactionCount': '0x0', 'eth_estimateGas': '0x5208', 'eth_getCode': '0x',
                          'eth_gasPrice': '0xb2d05e00',
                          'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]}}
                answer = lambda call: {'jsonrpc': '2.0', 'id': call['id'], 'result': values[call['method']]}
                self.reply(list(map(answer, body)) if isinstance(body, list) else answer(body))
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        endpoint = f'http://127.0.0.1:{server.server_port}'
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-upgrade-') as directory:
                root = pathlib.Path(directory)
                journal = root / 'network.jsonl'
                def run(*args, env=None, success=True):
                    p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True,
                                       text=True, timeout=60,
                                       env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(journal), **(env or {})})
                    assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                    return json.loads(p.stdout) if success else (p.returncode, p.stdout)
                seed = {'SPECTRA_SEED': phrase}
                watched = run('wallet', 'watch', '--chain', chain, '--address', address, '--name', 'Cold')['wallet']
                run('endpoints', '--chain', chain, '--api', 'blockscout', '--capabilities', 'history,token-history',
                    '--add', endpoint)
                run('endpoints', '--chain', chain, '--api', 'evm-json-rpc', '--capabilities',
                    'balance,fee,verification,broadcast', '--add', endpoint)
                assert run('history', 'Cold', '--save', '--endpoint', endpoint)['added'] == 1
                assert run('wallet', 'import', '--chain', chain, '--preview', env=seed)['upgradesWallet'] == 'Cold'
                upgraded = run('wallet', 'import', '--chain', chain, '--no-password', '--name', 'Ignored', env=seed)
                assert upgraded['upgraded'] and upgraded['wallet']['id'] == watched['id'], upgraded
                # A fresh process reads it back: same wallet, now signing, its history kept.
                shown = run('wallet', 'show', 'Cold')['wallet']
                assert shown['id'] == watched['id'] and not shown['isWatchOnly'], shown
                assert shown['derivationPath'] == "m/44'/60'/0'/0/0", shown
                assert len(run('wallet', 'list')['wallets']) == 1
                assert len(run('txs', '--page', '--wallet', 'Cold')['page']['records']) == 1
                prepared = run('send', 'build', '--from', 'Cold', '--to', '0x' + '22' * 20, '--amount', '0.01',
                               '--endpoint', endpoint)['artifact']
                signed = run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                             '--endpoint', endpoint)['artifact']
                assert signed['stage'] == 'Signed' and signed['signed_payload'], signed
                # Every other duplicate is refused, naming the wallet.
                for args, env in [(('wallet', 'import', '--chain', chain, '--no-password'), seed),
                                  (('wallet', 'watch', '--chain', chain, '--address', address), None)]:
                    code, output = run(*args, env=env, success=False)
                    assert code == 3 and 'Cold' in output, (args, output)
                assert len(run('wallet', 'list')['wallets']) == 1
                if journal.exists():
                    assert not journal.read_text(), journal.read_text()
        finally:
            server.shutdown(); server.server_close(); worker.join()

    def test_capabilities_name_limits_and_endpoints_and_custom_only_contacts_nothing_else(self):
        """The setup summary is the registry's; with custom-only set, the catalog's endpoints are never contacted."""
        class Node(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                values = {'eth_chainId': hex(84532), 'eth_getBalance': hex(5 * 10**17)}
                answer = lambda call: {'jsonrpc': '2.0', 'id': call['id'], 'result': values[call['method']]}
                data = json.dumps(list(map(answer, body)) if isinstance(body, list) else answer(body)).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(data)))
                self.end_headers(); self.wfile.write(data)
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        endpoint = f'http://127.0.0.1:{server.server_port}'
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-capabilities-') as directory:
                journal = pathlib.Path(directory) / 'network.jsonl'
                def run(*args, success=True):
                    p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True,
                                       text=True, timeout=60, env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(journal)})
                    assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                    return json.loads(p.stdout) if p.stdout.strip() else None
                summary = lambda chain: run('wallet', 'capabilities', '--chain', chain)['summary']
                assert summary('dash-testnet')['balance'] == 'needsCustomEndpoint'
                assert 'singleAddress' in summary('dash-testnet')['limits']
                assert summary('bnb')['history'] == 'needsCustomEndpoint', summary('bnb')
                assert summary('xrp')['limits'] == ['accountReserve']
                assert summary('monero')['limits'] == ['scansOnDevice']
                assert summary('zcash')['limits'] == ['singleAddress', 'shieldedScan']
                assert summary('ethereum')['staking'] is False and summary('solana')['staking'] is True
                base = summary('base-sepolia')
                assert base['balance'] == 'configured' and base['endpoints'], base
                assert all(e['isBuiltIn'] for e in base['endpoints']), base
                run('wallet', 'watch', '--chain', 'base-sepolia', '--name', 'Watched',
                    '--address', '0x9858effd232b4033e47d90003d41ec34ecaeda94')
                # Only the user's endpoints: none yet, so nothing is read.
                run('endpoints', '--chain', 'base-sepolia', '--custom-only', 'true')
                only = summary('base-sepolia')
                assert only['customEndpointsOnly'] and not only['endpoints'], only
                assert only['balance'] == 'needsCustomEndpoint', only
                run('balance', 'Watched', success=False)
                run('endpoints', '--chain', 'base-sepolia', '--api', 'evm-json-rpc', '--capabilities',
                    'balance,verification', '--add', endpoint)
                only = summary('base-sepolia')
                assert [e['endpoint'] for e in only['endpoints']] == [endpoint], only
                assert run('balance', 'Watched')
                # Both settings survive the process; turned off, the catalog's return.
                run('endpoints', '--chain', 'base-sepolia', '--custom-only', 'false')
                both = summary('base-sepolia')
                assert not both['customEndpointsOnly'] and both['endpoints'][0]['endpoint'] == endpoint, both
                assert any(e['isBuiltIn'] for e in both['endpoints']), both
                if journal.exists():
                    assert not journal.read_text(), journal.read_text()
        finally:
            server.shutdown(); server.server_close(); worker.join()

    def test_finding_used_accounts_names_endpoints_keeps_order_and_reports_failures(self):
        """A network's account scan names whom it asks, reads in profile order, and an unread account is not unused."""
        phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
        accounts = {}
        class Node(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def reply(self, status, payload):
                data = json.dumps(payload).encode()
                self.send_response(status); self.send_header('Content-Length', str(len(data)))
                self.end_headers(); self.wfile.write(data)
            def do_GET(self):
                # The indexer: account 0 was used and emptied.
                address = self.path.split('address=')[1].split('&')[0].lower()
                rows = [] if address != accounts[0] or 'action=txlist' not in self.path else [{
                    'hash': '0x' + 'cd' * 32, 'blockNumber': '7', 'timeStamp': '1700000000',
                    'from': address, 'to': '0x' + '22' * 20, 'value': '1', 'gasPrice': '1',
                    'gasUsed': '21000', 'isError': '0', 'txreceipt_status': '1'}]
                self.reply(200, {'status': '1', 'message': 'OK', 'result': rows})
            def do_POST(self):
                call = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if call['method'] == 'eth_chainId':
                    return self.reply(200, {'jsonrpc': '2.0', 'id': call['id'], 'result': hex(11155111)})
                address = call['params'][0].lower()
                if address == accounts[2]:
                    return self.reply(500, {'error': 'unavailable'})
                balance = hex(3 * 10**17) if address == accounts[1] else '0x0'
                self.reply(200, {'jsonrpc': '2.0', 'id': call['id'], 'result': balance})
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        rpc, scout = f'http://127.0.0.1:{server.server_port}/rpc', f'http://127.0.0.1:{server.server_port}/scout'
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-find-accounts-') as directory:
                journal = pathlib.Path(directory) / 'network.jsonl'
                def run(*args, success=True):
                    p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True,
                                       text=True, timeout=60,
                                       env={**os.environ, 'SPECTRA_SEED': phrase, 'SPECTRA_LOOPBACK_ONLY': str(journal)})
                    assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                    return json.loads(p.stdout) if success else p.returncode
                chain = 'ethereum-sepolia'
                for account in range(3):
                    accounts[account] = run('wallet', 'import', '--chain', chain, '--account', str(account),
                                            '--preview')['addresses'][0].lower()
                run('endpoints', '--chain', chain, '--custom-only', 'true')
                run('endpoints', '--chain', chain, '--api', 'evm-json-rpc', '--capabilities', 'balance,verification', '--add', rpc)
                run('endpoints', '--chain', chain, '--api', 'blockscout', '--capabilities', 'history', '--add', scout)
                listed = run('rescan', '--chain', chain, '--dry-run')
                assert {e['endpoint'] for e in listed['endpoints']} == {rpc, scout}, listed
                scanned = run('rescan', '--chain', chain)
                reads = scanned['reads']
                assert [r['account'] for r in reads] == [0, 1, 2], reads
                assert [r['address'].lower() for r in reads] == [accounts[0], accounts[1], accounts[2]], reads
                assert reads[0]['used'] and not reads[0]['funded'] and not reads[0]['error'], reads[0]
                assert reads[1]['funded'] and reads[1]['used'], reads[1]
                assert reads[2]['error'] and not reads[2]['used'], reads[2]
                assert scanned['unreachable'] == 1, scanned
                # One account per phrase has nothing to find.
                assert run('rescan', '--chain', 'polkadot', success=False) == 3
                assert run('rescan', '--chain', 'monero', success=False) == 3
                # The found account imports as itself.
                found = run('wallet', 'import', '--chain', chain, '--profile', reads[1]['profile'],
                            '--account', str(reads[1]['account']), '--no-password')['wallet']
                assert found['address'].lower() == accounts[1] and found['derivationPath'] == reads[1]['path'], found
                if journal.exists():
                    assert not journal.read_text(), journal.read_text()
        finally:
            server.shutdown(); server.server_close(); worker.join()

    def test_ton_wallet_versions_are_separate_accounts_of_one_key(self):
        """A TON key imports as W5 or v4R2, each its own account, and signs as the version stored."""
        fixtures = pathlib.Path(__file__).resolve().parents[1] / 'core/tests/fixtures'
        fixture = json.loads((fixtures / 'ton-w5.json').read_text())
        mnemonic = json.loads((fixtures / 'ton-mnemonics.json').read_text())['mnemonics'][0]
        raw = lambda address: '0:' + __import__('base64').urlsafe_b64decode(address + '==')[2:34].hex()
        with tempfile.TemporaryDirectory(prefix='spectra-ton-versions-') as directory:
            def run(*args, success=True):
                p = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                   capture_output=True, text=True, timeout=60,
                                   env={**os.environ, 'TON_KEY': '01' * 32, 'SPECTRA_SEED': mnemonic['mnemonic'],
                                        'SPECTRA_PASSWORD': 'versions-password'})
                assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                return json.loads(p.stdout) if success else p
            key = ('--private-key-env', 'TON_KEY')
            options = run('wallet', 'methods', '--chain', 'ton')['options']
            asking = sorted(o['method'] for o in options if 'tonWalletVersion' in o['fields'])
            assert asking == ['importPhrase', 'importPrivateKey'], options
            for chain, network in [('ton', 'mainnet'), ('ton-testnet', 'testnet')]:
                preview = run('wallet', 'import', '--chain', chain, *key, '--preview')
                assert preview['addresses'] == [fixture['key_address'][network]], preview
            w5 = fixture['key_address']['mainnet']
            v4r2 = run('wallet', 'import', '--chain', 'ton', *key, '--ton-wallet', 'v4R2',
                       '--preview')['addresses'][0]
            # The v4R2 account the v4R2 send vectors sign from.
            assert raw(v4r2) == '0:efaff4bac220f88b2e98eb1d9cffcca3bfe3b66ece31a7d6c5890d30dfd7afa5', v4r2
            # A watched v4R2 account takes the key in place; W5 is another wallet.
            run('wallet', 'watch', '--chain', 'ton', '--name', 'Old', '--address', v4r2)
            old = run('wallet', 'import', '--chain', 'ton', '--name', 'Ignored', *key, '--ton-wallet', 'v4R2')
            assert old['upgraded'] and old['wallet']['name'] == 'Old', old
            new = run('wallet', 'import', '--chain', 'ton', '--name', 'New', *key)
            assert not new['upgraded'] and new['wallet']['address'] == w5, new
            for name, address in [('Old', v4r2), ('New', w5)]:
                assert run('send', 'identity', '--from', name)['address'] == address, name
            refused = run('wallet', 'import', '--chain', 'ton', '--name', 'Again', *key, '--ton-wallet', 'w5',
                          success=False)
            assert refused.returncode == 3 and '“New”' in json.loads(refused.stdout)['error'], refused
            refused = run('wallet', 'import', '--chain', 'solana', *key, '--ton-wallet', 'w5', '--preview',
                          success=False)
            assert refused.returncode == 3 and 'Only a TON key import' in json.loads(refused.stdout)['error'], refused
            unknown = run('wallet', 'import', '--chain', 'ton', *key, '--ton-wallet', 'v3R2', '--preview',
                          success=False)
            assert unknown.returncode == 2, unknown
            assert len(run('wallet', 'list')['wallets']) == 2
            # Finding a phrase's used accounts reads each version's, W5 first.
            listed = run('rescan', '--chain', 'ton', '--dry-run')['candidates']
            assert [(c['tonWallet'], c['address']) for c in listed] == [
                ('w5', fixture['addresses'][0]['mainnet']), ('v4R2', mnemonic['address'])], listed

    def test_a_near_named_account_and_reserves_are_read_from_the_network(self):
        """A NEAR named account is stored once its key is confirmed full-access; XRP and Stellar reserves are the network's."""
        phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
        # near-seed-phrase's key for this phrase at m/44'/397'/0'.
        implicit = '5510e2b44cae6eb807e3e0e45d579dda058c274abcba15e5cb84636f5d1ee412'
        alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
        number, encoded = int(implicit, 16), ''
        while number:
            number, digit = divmod(number, 58)
            encoded = alphabet[digit] + encoded
        key = 'ed25519:' + encoded
        class Node(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def reply(self, payload):
                data = json.dumps(payload).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(data)))
                self.end_headers(); self.wfile.write(data)
            def do_GET(self):
                if self.path.startswith('/horizon/ledgers'):
                    return self.reply({'_embedded': {'records': [{'base_reserve_in_stroops': 5000000}]}})
                self.reply({'network_passphrase': 'Public Global Stellar Network ; September 2015'})
            def do_POST(self):
                call = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                method, params = call['method'], call.get('params') or {}
                if self.path.startswith('/xrpl'):
                    result = {'server_info': {'info': {'network_id': 0}},
                              'server_state': {'state': {'validated_ledger': {'reserve_base': 1000000}}}}[method]
                    return self.reply({'result': result})
                if method == 'status':
                    return self.reply({'jsonrpc': '2.0', 'id': call['id'], 'result': {'chain_id': 'mainnet'}})
                full = params.get('account_id') == 'alice.near' and params.get('public_key') == key
                if full:
                    return self.reply({'jsonrpc': '2.0', 'id': call['id'], 'result': {'nonce': 3, 'permission': 'FullAccess'}})
                self.reply({'jsonrpc': '2.0', 'id': call['id'],
                            'error': {'code': -32000, 'message': 'Server error', 'data': 'access key does not exist'}})
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        base = f'http://127.0.0.1:{server.server_port}'
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-named-') as directory:
                journal = pathlib.Path(directory) / 'network.jsonl'
                def run(*args, success=True):
                    p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True,
                                       text=True, timeout=60,
                                       env={**os.environ, 'SPECTRA_SEED': phrase, 'SPECTRA_LOOPBACK_ONLY': str(journal)})
                    assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                    return json.loads(p.stdout) if success else (p.returncode, p.stdout)
                assert 'namedAccount' in [f for o in run('wallet', 'methods', '--chain', 'near')['options'] for f in o['fields']]
                run('endpoints', '--chain', 'near', '--api', 'near-json-rpc', '--capabilities', 'balance,verification', '--add', base + '/near')
                assert run('wallet', 'import', '--chain', 'near', '--named-account', 'alice.near', '--preview')['addresses'] == ['alice.near']
                code, output = run('wallet', 'import', '--chain', 'near', '--named-account', 'bob.near', '--no-password', success=False)
                assert code == 3 and 'bob.near' in output, output
                code, _ = run('wallet', 'import', '--chain', 'ethereum', '--named-account', 'alice.near', '--no-password', success=False)
                assert code == 3
                wallet = run('wallet', 'import', '--chain', 'near', '--named-account', 'alice.near', '--no-password', '--name', 'Named')['wallet']
                assert wallet['address'] == 'alice.near', wallet
                assert run('send', 'identity', '--from', 'Named')['address'] == 'alice.near'
                run('endpoints', '--chain', 'xrp', '--api', 'xrpl-json-rpc', '--capabilities', 'balance,verification', '--add', base + '/xrpl')
                run('endpoints', '--chain', 'stellar', '--api', 'horizon', '--capabilities', 'balance,verification', '--add', base + '/horizon')
                assert run('wallet', 'reserve', '--chain', 'xrp')['amount'] == '1'
                assert run('wallet', 'reserve', '--chain', 'stellar')['amount'] == '1'
                assert run('wallet', 'reserve', '--chain', 'ethereum', success=False)[0] == 3
                if journal.exists():
                    assert not journal.read_text(), journal.read_text()
        finally:
            server.shutdown(); server.server_close(); worker.join()

class ApprovalsTests(unittest.TestCase):
    def test_approvals_are_confirmed_live_and_revoked_through_the_send_stages(self):
        """Logged approvals are shown only while a live read says they stand; a
        revocation is approve(spender, 0), signed and broadcast as a send; an
        Ethereum name shows only when it resolves back; no indexer is refused."""
        phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
        owner = '0x9858effd232b4033e47d90003d41ec34ecaeda94'
        token, other = '0x' + 'aa' * 20, '0x' + 'bb' * 20
        unlimited, spent, nft = '0x' + '11' * 20, '0x' + '22' * 20, '0x' + '33' * 20
        resolver = '0x' + '44' * 20
        state = {'ens': owner}
        submitted, seen = [], []
        word = lambda hex_: '0x' + hex_.removeprefix('0x').rjust(64, '0')
        abi_string = lambda text: '0x' + format(32, '064x') + format(len(text), '064x') + text.encode().hex().ljust(64, '0')
        def log(token_, spender, fourth=None):
            return {'address': token_, 'blockNumber': '0x10', 'data': word('0'), 'logIndex': hex(len(seen)),
                    'transactionHash': '0x' + os.urandom(32).hex(), 'timeStamp': '0x1',
                    'topics': ['0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925',
                               word(owner), word(spender), fourth]}
        directory_holder = {}
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def reply(self, value):
                data = json.dumps(value).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(data)))
                self.end_headers(); self.wfile.write(data)
            def do_GET(self):
                query = dict(part.split('=', 1) for part in self.path.split('?', 1)[1].split('&'))
                assert query['module'] == 'logs' and query['action'] == 'getLogs', self.path
                assert query['topic1'] == word(owner), query
                rows = [log(token, unlimited), log(token, spent), log(other, nft, word('7')), log(token, unlimited)]
                self.reply({'status': '1', 'message': 'OK', 'result': rows})
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                def answer(call):
                    method = call['method']; seen.append(method)
                    if method == 'eth_call':
                        data = call['params'][0]['data'][2:]
                        selector, to = data[:8], call['params'][0]['to']
                        if selector == 'dd62ed3e':
                            result = word('f' * 64) if data[-40:] == unlimited[2:] else word('0')
                        elif selector == '313ce567':
                            result = word('12')
                        elif selector == '95d89b41':
                            result = abi_string('TKA')
                        elif selector == '01ffc9a7':
                            result = word('0')
                        elif selector == '0178b8bf':
                            result = word(resolver)
                        elif selector == '691f3431':
                            assert to == resolver, call
                            result = abi_string('spectra.eth')
                        elif selector == '3b3b57de':
                            result = word(state['ens'])
                        else:
                            raise AssertionError(call)
                        return {'jsonrpc': '2.0', 'id': call['id'], 'result': result}
                    if method == 'eth_sendRawTransaction':
                        submitted.append(call['params'][0])
                        with sqlite3.connect(pathlib.Path(directory_holder['path']) / 'spectra.sqlite') as db:
                            artifacts = [json.loads(row[0]) for row in db.execute('SELECT payload FROM send_artifacts')]
                        artifact = next(a for a in artifacts if a['submission'] and a['submission']['payload'] == call['params'][0])
                        return {'jsonrpc': '2.0', 'id': call['id'], 'result': artifact['view']['transaction_hash']}
                    values = {'eth_chainId': '0x1', 'eth_getBalance': hex(10 * 10**18), 'eth_getCode': '0x',
                              'eth_getTransactionCount': '0x3', 'eth_estimateGas': '0xb000', 'eth_gasPrice': '0xb2d05e00',
                              'eth_blockNumber': '0x20',
                              'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]}}
                    assert method in values, method
                    return {'jsonrpc': '2.0', 'id': call['id'], 'result': values[method]}
                self.reply(list(map(answer, body)) if isinstance(body, list) else answer(body))
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        endpoint = f'http://127.0.0.1:{server.server_port}'
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-approvals-') as directory:
                directory_holder['path'] = directory
                journal = pathlib.Path(directory) / 'network.jsonl'
                def run(*args, success=True):
                    p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True,
                                       text=True, timeout=60,
                                       env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(journal), 'SPECTRA_SEED': phrase})
                    assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                    return json.loads(p.stdout) if success else p.returncode
                run('wallet', 'import', '--chain', 'ethereum', '--name', 'Approver', '--no-password')
                for chain in ['ethereum', 'bnb']:
                    run('endpoints', '--chain', chain, '--api', 'evm-json-rpc', '--capabilities',
                        'balance,fee,broadcast,verification,token-balance', '--add', endpoint)
                    run('endpoints', '--chain', chain, '--custom-only', 'true')
                # No indexer: refused, not an empty list.
                run('wallet', 'import', '--chain', 'bnb', '--name', 'NoIndexer', '--no-password')
                assert run('wallet', 'approvals', 'NoIndexer', success=False) == 3
                run('endpoints', '--chain', 'ethereum', '--api', 'blockscout', '--capabilities', 'history', '--add', endpoint)
                approvals = run('wallet', 'approvals', 'Approver')['approvals']
                assert approvals['complete'], approvals
                # The spent approval is gone; the NFT one is not ERC-20's.
                assert [(a['token'], a['spender'], a['symbol'], a['unlimited']) for a in approvals['approvals']] == \
                    [(token, unlimited, 'TKA', True)], approvals
                assert run('wallet', 'revoke', 'Approver', '--token', token, '--spender', spent, success=False) == 3
                built = run('wallet', 'revoke', 'Approver', '--token', token, '--spender', unlimited)['artifact']
                revocation = built['operation']
                assert revocation['kind'] == 'revoke_approval', built
                assert (revocation['token'], revocation['spender']) == (token, unlimited), built
                # 0xb000 gas plus Ethereum's 20% buffer (54068), at 2 × 1 gwei
                # base + 2 gwei tip.
                assert revocation['network_fee'] == '0.000216272', revocation
                prepared = json.loads(built['prepared_details'])['Evm']
                assert prepared['to'] == token and prepared['value_wei'] == 0, prepared
                assert bytes(prepared['data']).hex() == '095ea7b3' + unlimited[2:].rjust(64, '0') + '0' * 64, prepared
                # The reopened artifact signs and broadcasts as any send does.
                signed = run('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                             '--endpoint', endpoint)['artifact']
                run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')
                assert len(submitted) == 1, submitted
                with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as db:
                    kinds = [json.loads(row[0])['kind'] for row in db.execute('SELECT payload FROM history_records')]
                assert kinds == ['revokeApproval'], kinds
                assert run('wallet', 'ens', 'Approver')['name'] == 'spectra.eth'
                state['ens'] = '0x' + '55' * 20
                assert run('wallet', 'ens', 'Approver')['name'] is None
                assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
        finally:
            server.shutdown()


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
