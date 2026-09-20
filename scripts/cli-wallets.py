#!/usr/bin/env python3
"""Wallet import, naming, receive addresses and password validation.

Run: python3 scripts/cli-wallets.py [path/to/spectra] [TestClass.test_name]
Uses temporary stores and loopback nodes; no public network is required.
"""
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else
                          pathlib.Path(__file__).resolve().parents[1] / 'target/debug/spectra').resolve())


class WalletsTests(unittest.TestCase):
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
            for password, confirmation, reason in [('', '', None), ('abc','abc','tooShort'),
                    ('密碼','密碼','tooShort'), ('密碼測試','密碼測試',None), ('abcd','abce','confirmationMismatch')]:
                result=run('wallet','check-password',env={'SPECTRA_PASSWORD':password,'SPECTRA_PASSWORD_CONFIRMATION':confirmation})
                assert result=={'valid':reason is None,'rejection':reason}, result


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
