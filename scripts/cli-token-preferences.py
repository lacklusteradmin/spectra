#!/usr/bin/env python3
"""Offline: the catalog is listed whole, and editable provider metadata survives process restarts."""
import json
import subprocess
import sys
import tempfile

with tempfile.TemporaryDirectory() as directory:
    def run(*args, code=0):
        result = subprocess.run([sys.argv[1], '--data-dir', directory, '--json', 'token', *args], capture_output=True, text=True)
        assert result.returncode == code, (args, result.stdout, result.stderr)
        return json.loads(result.stdout) if code == 0 else None

    def rows():
        return run('list')['tokens']

    initial = rows()
    usdc = [r for r in initial if r['token_id'] == 'usd-coin']
    assert len({r['chain_id'] for r in usdc}) > 2
    assert all('isEnabled' not in r for r in initial)

    address = '0x1111111111111111111111111111111111111111'
    base = ['--chain', 'Base', '--contract', address, '--symbol', 'USDC', '--name', 'Independent token', '--decimals', '6']
    run('add', *base, '--coinpaprika-id', 'custom-independent')
    custom = next(r for r in rows() if not r['isBuiltIn'])
    assert custom['coingecko_id'] == '' and custom['coinpaprika_id'] == 'custom-independent'
    run('edit', *base, '--coingecko-id', 'usd-coin', '--coinpaprika-id', 'usdc-usd-coin')
    edited = next(r for r in rows() if r['id'] == custom['id'])
    assert edited['token_id'] == custom['token_id']
    assert edited['coingecko_id'] == 'usd-coin' and edited['coinpaprika_id'] == 'usdc-usd-coin'
    run('edit', *base, '--coinpaprika-id', 'https://coinpaprika.com/coin/example', code=3)
    assert next(r for r in rows() if r['id'] == custom['id'])['coinpaprika_id'] == 'usdc-usd-coin'
    run('edit', *base)
    cleared = next(r for r in rows() if r['id'] == custom['id'])
    assert cleared['coinpaprika_id'] == '' and cleared['coingecko_id'] == ''
    run('remove', '--chain', 'Base', '--contract', address)
    assert all(r['isBuiltIn'] for r in rows())
print('The catalog is listed whole and independent price sources survive reopening')
