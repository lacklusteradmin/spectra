#!/usr/bin/env python3
"""Offline valuation and bounded history rules, including reopening persisted state."""
import json, pathlib, sqlite3, subprocess, sys, tempfile
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='spectra-boundary-') as directory:
    def run(*args):
        result = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True)
        assert result.returncode == 0, (args, result.stdout, result.stderr)
        return json.loads(result.stdout)
    run('wallet', 'watch', '--chain', 'ethereum', '--address', '0x'+'11'*20, '--name', 'Boundary')
    dbpath = pathlib.Path(directory)/'spectra.sqlite'
    with sqlite3.connect(dbpath) as db:
        wid, raw = db.execute('SELECT id,payload FROM wallets').fetchone()
        wallet = json.loads(raw)
        native = dict(name='Ethereum', symbol='ETH', coinGeckoId='ethereum', chainName='Ethereum', tokenStandard='Native', contractAddress=None, amount=2, priceUsd=999)
        usdc = dict(name='USD Coin', symbol='USDC', coinGeckoId='usd-coin', chainName='Ethereum', tokenStandard='ERC-20', contractAddress='0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48', amount=100, priceUsd=1)
        wallet['holdings'] = [native, usdc]
        db.execute('UPDATE wallets SET payload=? WHERE id=?', (json.dumps(wallet),wid))
        db.execute('INSERT OR REPLACE INTO app_state_meta VALUES (?,?)', ('quotes',json.dumps({'prices':{'ethereum:native':3000}})))
        for i in range(55):
            row = dict(id=f'tx-{i:03}', walletId=wid, walletName='Éther 测试', kind='receive', status='confirmed', chainName='Ethereum', symbol='ETH', assetDisplayName='Ether', deploymentId='ethereum:native', amount=1, address='0x'+'22'*20, transactionHash=f'0x{i:064x}', createdAt=i)
            db.execute('INSERT INTO history_records VALUES (?,?,?,?,?,?)', (row['id'],wid,'Ethereum',row['transactionHash'],978307200+i,json.dumps(row)))
        # A duplicate provider record must not consume a page slot or hide the confirmed row.
        row.update(id='duplicate', status='pending')
        db.execute('INSERT INTO history_records VALUES (?,?,?,?,?,?)', (row['id'],wid,'Ethereum',row['transactionHash'],978307999,json.dumps(row)))
    valuation = run('portfolio','--stored')['valuation']
    assert valuation['portfolio'] == dict(total=6000.0, unpricedCount=1, fiatTotal=6000.0), valuation
    assert valuation['wallets'][wid] == valuation['portfolio']
    run('currency','EUR')
    valuation = run('portfolio','--stored')['valuation']
    assert valuation['portfolio']['fiatTotal'] is None, valuation
    pages = [run('txs','--page','--offset',str(offset))['page'] for offset in (0,20,40)]
    ids = [r['id'] for page in pages for r in page['records']]
    assert len(ids)==55 and len(set(ids))==55 and 'duplicate' not in ids, ids
    assert [p['hasMore'] for p in pages] == [True,True,False], pages
    assert run('txs','--page','--filter','pending')['page']['records']==[]
    assert len(run('txs','--page','--search','éTHER 测试')['page']['records']) == 20
    assert run('txs','--page','--search','no-such-address')['page']['records']==[]
    assert run('txs','--page','--oldest-first','--limit','1')['page']['records'][0]['id']=='tx-000'
    summary = run('txs','--summary')['summary']
    assert summary['totalCount'] == 56
    assert len(summary['recentAndPending']) <= 51
    assert summary['earliest'][0]['earliestCreatedAtUnix'] == 978307200
    assert run('txs','--record','tx-000')['record']['id'] == 'tx-000'
    assert run('txs','--record','missing')['record'] is None
    run('wallet','watch','--chain','solana','--address','11111111111111111111111111111111','--name','IdentityCases')
    with sqlite3.connect(dbpath) as db:
        solana_id = db.execute("SELECT id FROM wallets WHERE name='IdentityCases'").fetchone()[0]
        for identity, txhash, deployment in [('case-upper','A'*88,'solana:native'), ('case-lower','a'*88,'solana:native'), ('unknown-one','B'*88,None), ('unknown-two','B'*88,None)]:
            record = dict(id=identity, walletId=solana_id, walletName='IdentityCases', kind='receive', status='confirmed', chainName='Solana', symbol='SOL', assetDisplayName='Solana', deploymentId=deployment, amount=1, address='11111111111111111111111111111111', transactionHash=txhash, createdAt=1)
            db.execute('INSERT INTO history_records VALUES (?,?,?,?,?,?)', (identity,solana_id,'Solana',txhash.lower(),978307201,json.dumps(record)))
    distinct = run('txs','--page','--wallet','IdentityCases')['page']['records']
    assert {row['id'] for row in distinct} == {'case-upper','case-lower','unknown-one','unknown-two'}, distinct
print('shell boundary: valuation, missing rates, deduplication and pagination passed')
