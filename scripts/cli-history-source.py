#!/usr/bin/env python3
"""A stored history-source id reads as what it names, through the real CLI.

The app's detail sheet held a six-arm switch over provider ids. Five of its
arms named providers no producer writes, and `rust`, `rust.hd` and `etherscan`
fell through to the raw id. Core names them now; this seeds each id core writes
and checks what `txs` reports for it.
"""
import json, pathlib, sqlite3, subprocess, sys, tempfile

binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='spectra-history-source-') as directory:
    def run(*args):
        p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True)
        assert p.returncode == 0, (args, p.stdout, p.stderr)
        return json.loads(p.stdout)
    run('txs')  # create the core schema
    expected = {
        'rpc': {'provider': 'RPC'},
        'etherscan': {'provider': 'Etherscan'},
        'rust': 'internal',
        'rust.hd': 'internal',
        'dogecoin.providers': {'chainProviders': 'Dogecoin'},
        'none': None,
    }
    with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as db:
        for index, source in enumerate(expected):
            tx_hash = f'{index:02x}' * 32
            row = dict(id=source, walletId='wallet', walletName='Fixture', kind='receive', status='confirmed',
                       chainName='Dogecoin', symbol='DOGE', assetDisplayName='Dogecoin', amount=1,
                       address='sender', transactionHash=tx_hash, createdAt=1234,
                       transactionHistorySource=source)
            db.execute('INSERT INTO history_records (id,wallet_id,chain_name,tx_hash,created_at,payload) VALUES (?,?,?,?,?,?)',
                       (source, 'wallet', 'Dogecoin', tx_hash, 978308434 + index, json.dumps(row)))
    by_hash = {t['hash']: t.get('historySource') for t in run('txs')['transactions']}
    for index, (source, named) in enumerate(expected.items()):
        got = by_hash[f'{index:02x}' * 32]
        assert got == named, f'{source}: expected {named!r}, got {got!r}'
print('history sources: providers named, chain aggregates by chain, Spectra\'s own reader and "none" unnamed')
