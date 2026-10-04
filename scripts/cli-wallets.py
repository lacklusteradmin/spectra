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
    def test_bitcoin_account_xpub_validation(self):
        """Only complete Bitcoin public keys may reach a watched wallet."""
        xpub = 'xpub6BemYiVNp19Zz9Bw6kmmfXR2LEFukA1hnhSZrXgE2AUJvNLW8a87gg72bQLi4RfGHcKcR4ojrEFgFJgNCXcjVYSH75YmvhTZ7qh9FCrxv3a'
        with tempfile.TemporaryDirectory(prefix='spectra-xpub-') as directory:
            def run(*args, success=True):
                p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
                assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                return json.loads(p.stdout) if success else None
            for invalid in ['xpubgarbage', 'ypubgarbage', 'zpubgarbage', xpub[:-1] + '1']:
                run('wallet', 'watch', '--chain', 'bitcoin', '--xpub', invalid, success=False)
            run('wallet', 'watch', '--chain', 'ethereum', '--xpub', xpub, success=False)
            run('wallet', 'watch', '--chain', 'bitcoin', '--xpub', xpub,
                '--address', 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4', success=False)
            assert run('wallet', 'list')['wallets'] == [], 'invalid xpub persisted a wallet'
            watched = run('wallet', 'watch', '--chain', 'bitcoin', '--xpub', '  ' + xpub + '  ')
            assert watched['count'] == 1, watched
            assert run('wallet', 'list')['wallets'][0]['isWatchOnly']

    def test_evm_alias_identity_uses_wallets_own_path(self):
        """Ethereum and ETC aliases resolve one recorded key with its actual path."""
        with tempfile.TemporaryDirectory(prefix='spectra-evm-alias-') as directory:
            root = pathlib.Path(directory)
            seed = root / 'seed.txt'
            seed.write_text('test test test test test test test test test test test junk')
            def run(*args):
                p = subprocess.run([binary, '--data-dir', str(root / 'state'), '--json', *args], capture_output=True, text=True, timeout=60)
                assert p.returncode == 0, (args, p.stdout, p.stderr)
                return json.loads(p.stdout)
            for source, path in [('ethereum', "m/44'/60'/3'/0/0"),
                                 ('ethereum-classic', "m/44'/61'/2'/0/0")]:
                imported = run('wallet', 'import', '--chain', source, '--path', path,
                               '--seed-file', str(seed), '--no-password', '--name', source)['wallet']
                assert imported['addresses'] == {source: imported['address']}, imported
                for requested in ['ethereum', 'ethereum-classic', 'arbitrum', 'ethereum-sepolia']:
                    identity = run('send', 'identity', '--from', source, '--chain', requested)
                    assert identity['address'] == imported['address'], (source, requested, identity)

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
            seed.write_text('  ' + ' \t\n'.join(seed.read_text().upper().split()) + '  ')
            canonical = import_with('Canonical mnemonic', {'passphrase':'secret'})['wallet']
            assert canonical['address'] == trimmed['address'], 'raw mnemonic derived a different identity'
            run('send','identity','--from','Canonical mnemonic')

    def test_names_and_receive(self):
        """Unnamed wallets get unique durable names and their own receive address."""
        with tempfile.TemporaryDirectory(prefix='spectra-wallets-') as directory:
            def run(*args, success=True):
                p=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True, timeout=60)
                assert (p.returncode==0)==success,(args,p.stdout,p.stderr)
                return json.loads(p.stdout) if success else None
            address='0x'+'01'*20
            for i in range(3):
                run('wallet','watch','--chain','ethereum','--address','0x'+f'{i+1:02x}'*20)
            wallets=run('wallet','list')['wallets']
            assert {w['name'] for w in wallets}=={'Wallet 1','Wallet 2','Wallet 3'},wallets
            received=run('wallet','receive','Wallet 1')
            assert received['address'] == address, received

    def test_password_validation(self):
        """Password validation counts Unicode characters and checks confirmation."""
        with tempfile.TemporaryDirectory(prefix='spectra-password-') as directory:
            def run(*args, success=True, env=None):
                p=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True,
                                 env={**os.environ, **(env or {})}, timeout=30)
                assert (p.returncode==0)==success,(args,p.stdout,p.stderr)
                return json.loads(p.stdout)
            # Only empty fields mean no password; whitespace is a blank one.
            # A short password names core's minimum, so no client restates it.
            short={'tooShort':{'minChars':4}}
            for password, confirmation, reason in [('', '', None), ('   ','   ',short), ('abc','abc',short),
                    ('密碼','密碼',short), ('密碼測試','密碼測試',None), ('abcd','abce','confirmationMismatch')]:
                result=run('wallet','check-password',env={'SPECTRA_PASSWORD':password,'SPECTRA_PASSWORD_CONFIRMATION':confirmation})
                assert result=={'valid':reason is None,'rejection':reason}, result


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
