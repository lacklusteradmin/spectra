#!/usr/bin/env python3
"""Network-correct key import, signer resolution and testnet watch wallets; offline."""
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
with tempfile.TemporaryDirectory(prefix='spectra-chain-coverage-') as directory:
    def run(*args, success=True):
        result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
            env={**os.environ, 'SPECTRA_PASSWORD': 'coverage-password', 'COVERAGE_KEY': '0' * 63 + '1'},
            capture_output=True, text=True, timeout=45)
        assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
        return json.loads(result.stdout)

    chains = ['bitcoin-testnet', 'bitcoin-testnet-4', 'bitcoin-signet',
        'bitcoin-cash-testnet', 'dogecoin-testnet', 'decred-testnet',
        'bitcoin-sv', 'bitcoin-sv-testnet', 'bitcoin-gold', 'dash', 'dash-testnet',
        'zcash', 'zcash-testnet', 'kaspa', 'kaspa-testnet', 'tron', 'tron-nile', 'xrp', 'xrp-testnet',
        'solana', 'solana-devnet', 'stellar', 'stellar-testnet', 'sui', 'sui-testnet',
        'aptos', 'aptos-testnet', 'ton', 'ton-testnet', 'near', 'near-testnet', 'internet-computer',
        'polkadot', 'polkadot-westend', 'bittensor']
    for chain in chains:
        # A test network's address is watched first; the key then upgrades
        # that wallet in place rather than adding a second one.
        is_test_network = 'testnet' in chain or chain in ['bitcoin-signet', 'tron-nile', 'solana-devnet', 'polkadot-westend']
        if is_test_network:
            address = run('wallet', 'import', '--chain', chain, '--private-key-env', 'COVERAGE_KEY',
                          '--preview')['addresses'][0]
            watched = run('wallet', 'watch', '--chain', chain, '--name', chain, '--address', address)
            assert watched['wallet']['address'] == address, (chain, watched)
        imported = run('wallet', 'import', '--chain', chain, '--name', 'key-' + chain,
            '--private-key-env', 'COVERAGE_KEY')
        wallet = imported['wallet']
        assert imported['upgraded'] == is_test_network, (chain, imported)
        if is_test_network:
            assert wallet['id'] == watched['wallet']['id'] and wallet['name'] == chain, (chain, wallet)
        else:
            run('wallet', 'rename', 'key-' + chain, chain)
        address = wallet['address']
        identity = run('send', 'identity', '--from', chain)
        assert identity['address'] == address, (chain, identity, wallet)
    public_keys = [
        'tpubDC2Q4xK4XH72FwNnYwkrsSkPfMMZjYWmLL2sD5jNN5ECff1DCnyCq7mrqSRVavcW5WrX2eKcPL5iwVAybPTyNytaAgJ5cE7nhoHZ1KsTbgx',
        'upub5D9ydiUdMxX8SFcabgQu3G8WpKpZvJ3D3MsuWLzZt9Lfm4tpNbdRpV8cXnTx4hhW4gyCAe24UFC2bSqg3SNh6qPUW4TWAy16JfWD5XMU4ya',
        'vpub5XzEwP9YWe4cHYohS3CXFME1zHy1rv2hxUQ8HjtTG9iYpAi3dFnzSYnkYzRY4cMRUL5zv7ccvuYaUjTEm8nhu555NQ9vkspaaPZrU8SMfdg']
    for prefix_key in public_keys:
        name = prefix_key[:4] + '-watch'
        watched = run('wallet', 'watch', '--chain', 'bitcoin-testnet-4', '--name', name, '--xpub', prefix_key)
        with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as db:
            payload = db.execute('SELECT payload FROM wallets WHERE id=?', (watched['wallet']['id'],)).fetchone()[0]
        assert json.loads(payload)['xpub'] == prefix_key, watched
        receive = run('wallet', 'receive', name)
        assert receive['address'].startswith(('m', 'n', '2', 'tb1')), receive
        run('wallet', 'watch', '--chain', 'bitcoin', '--name', name + '-wrong-network', '--xpub', prefix_key, success=False)
        # The three prefixes write one account key; watching it again under
        # another is refused, so each is checked on its own.
        run('wallet', 'delete', name, '--yes')
print('network-correct raw keys, signing identities and testnet watch imports passed')
