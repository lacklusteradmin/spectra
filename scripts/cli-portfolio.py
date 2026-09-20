#!/usr/bin/env python3
"""Persisted balances, token identity, valuations and movement notifications.

Run: python3 scripts/cli-portfolio.py [path/to/spectra] [TestClass.test_name]
Uses temporary stores and loopback nodes; no public network is required.
"""
import http.server
import json
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else
                          pathlib.Path(__file__).resolve().parents[1] / 'target/debug/spectra').resolve())


class PortfolioTests(unittest.TestCase):
    def test_movement_notifications(self):
        """Persist movement baselines; do not repeat or fabricate notifications."""
        with tempfile.TemporaryDirectory(prefix='spectra-wallets-') as directory:
            def run(*args, success=True):
                p=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True, timeout=60)
                assert (p.returncode==0)==success,(args,p.stdout,p.stderr)
                return json.loads(p.stdout) if success else None
            for i in range(3):
                run('wallet','watch','--chain','ethereum','--address','0x'+f'{i+1:02x}'*20)
            dbpath=pathlib.Path(directory)/'spectra.sqlite'
            def change_wallets(change):
                with sqlite3.connect(dbpath) as db:
                    for id,payload in db.execute('SELECT id,payload FROM wallets').fetchall():
                        w=json.loads(payload); change(w)
                        db.execute('UPDATE wallets SET payload=? WHERE id=?',(json.dumps(w),id))
            def seed(w):
                w['holdings']=[dict(name='Ethereum',symbol='ETH',coinGeckoId='ethereum',chainName='Ethereum',
                    tokenStandard='Native',contractAddress=None,amount=1,priceUsd=0)]
            change_wallets(seed)
            def quote(price):
                with sqlite3.connect(dbpath) as db:
                    db.execute('INSERT OR REPLACE INTO app_state_meta (key,value) VALUES (?,?)',('quotes',json.dumps({'prices':{'ethereum:native':price}})))
            quote(1000)
            assert run('alert','movement')['notification'] is None
            quote(1200)
            movement=run('alert','movement')['notification']
            assert movement['absoluteDelta']==600 and movement['directionUp'],movement
            assert run('alert','movement')['notification'] is None
            quote(1600)
            assert run('alert','movement','--active')['notification'] is None
            assert run('alert','movement')['notification'] is None
            run('wallet', 'inclusion', 'Wallet 2', 'false')
            assert run('alert','movement')['notification'] is None
            quote(0)
            assert run('alert','movement')['notification'] is None
            quote(2000)
            assert run('alert','movement')['notification'] is None

    def test_valuation_and_inclusion(self):
        """Missing quotes stay incomplete; inclusion changes persist and alter totals."""
        with tempfile.TemporaryDirectory(prefix='spectra-valuation-') as directory:
            def run(*args):
                result = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
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
            valuation = run('portfolio','--stored')['valuation']
            assert valuation['portfolio'] == dict(total=6000.0, unpricedCount=1, fiatTotal=6000.0), valuation
            assert valuation['wallets'][wid] == valuation['portfolio']
            # Each read starts a new CLI process: the command must persist both
            # the flag and its effect, without deleting the wallet's holdings.
            expected_wallet_value = valuation['wallets'][wid]
            def stored_wallet():
                with sqlite3.connect(dbpath) as db:
                    return json.loads(db.execute('SELECT payload FROM wallets WHERE id=?', (wid,)).fetchone()[0])
            before = stored_wallet()
            run('wallet', 'inclusion', 'Boundary', 'false')
            excluded = stored_wallet()
            assert excluded['includeInPortfolioTotal'] is False, excluded
            assert excluded['holdings'] == before['holdings'], excluded
            valuation = run('portfolio', '--stored')['valuation']
            assert valuation['portfolio'] == dict(total=0.0, unpricedCount=0, fiatTotal=0.0), valuation
            assert valuation['wallets'][wid] == expected_wallet_value, valuation
            run('wallet', 'inclusion', 'Boundary', 'true')
            assert stored_wallet()['includeInPortfolioTotal'] is True
            assert run('portfolio', '--stored')['valuation']['portfolio'] == expected_wallet_value
            run('currency','EUR')
            valuation = run('portfolio','--stored')['valuation']
            assert valuation['portfolio']['fiatTotal'] is None, valuation

    def test_balance_refresh(self):
        """Save refreshed balances and preserve token balances when their query fails."""
        usdc = '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48'

        phase = 1

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass
            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                def answer(item):
                    method = item['method']
                    if method == 'eth_getBalance':
                        result = hex(10**18 if phase == 1 else 0)
                    elif method == 'eth_call':
                        token = item['params'][0]['to'].lower()
                        selector = item['params'][0]['data'][:10]
                        if selector == '0x313ce567':
                            result = hex(6 if token == usdc else 18)
                        elif selector in ('0x95d89b41', '0x06fdde03'):
                            text = 'USDC' if token == usdc else 'TOKEN'
                            result = '0x' + f'{32:064x}{len(text):064x}' + text.encode().hex().ljust(64, '0')
                        else:
                            result = hex(2_000_000) if token == usdc else '0x0'
                            if phase == 2 and token == usdc:
                                result = 'not-a-balance'
                    else:
                        raise AssertionError(method)
                    return {'jsonrpc': '2.0', 'id': item.get('id', 1), 'result': result}
                data = json.dumps([answer(item) for item in request] if isinstance(request, list) else answer(request)).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        with tempfile.TemporaryDirectory(prefix='spectra-balance-') as directory:
            def run(*args):
                result = subprocess.run([binary, '--data-dir', directory, '--json', *args], check=True, capture_output=True, text=True, timeout=60)
                return json.loads(result.stdout)
            run('wallet', 'watch', '--chain', 'Ethereum', '--address', '0x1111111111111111111111111111111111111111', '--name', 'Fixture')
            server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
            worker = threading.Thread(target=server.serve_forever, daemon=True)
            worker.start()
            try:
                endpoint = f'http://127.0.0.1:{server.server_port}'
                def balances():
                    with sqlite3.connect(str(pathlib.Path(directory) / 'spectra.sqlite')) as db:
                        wallet = json.loads(db.execute('SELECT payload FROM wallets').fetchone()[0])
                        return {h['symbol']: h['amount'] for h in wallet['holdings']}
                assert run('refresh', '--wallet', 'Fixture', '--endpoint', endpoint)['refreshed'] == 1
                assert balances()['ETH'] == 1 and balances()['USDC'] == 2
                phase = 2
                assert run('refresh', '--wallet', 'Fixture', '--endpoint', endpoint)['refreshed'] == 1
                assert balances()['ETH'] == 0 and balances()['USDC'] == 2, 'failed token read must preserve its last balance'
            finally:
                server.shutdown()
                server.server_close()
                worker.join()

    def test_network_token_identity(self):
        """Keep network/deployment identities and testnet values distinct."""
        with tempfile.TemporaryDirectory(prefix="spectra-identity-") as directory:
            def run(*args, succeeds=True):
                result = subprocess.run([binary, "--data-dir", directory, "--json", *args], text=True, capture_output=True, timeout=60)
                assert (result.returncode == 0) == succeeds, (args, result.stdout, result.stderr)
                return json.loads(result.stdout) if succeeds else None

            networks = {n["id"]: n for n in run("chains", "--testnets")["chains"]}
            assert not networks["ethereum"]["isTestnet"] and networks["ethereum-sepolia"]["isTestnet"]
            assert networks["ethereum"]["family"] == networks["ethereum-sepolia"]["family"]
            assert networks["arbitrum"]["nativeSymbol"] == "ETH"
            def catalog(network):
                return run("token", "catalog", "--chain", network)["tokens"]
            eth = next(t for t in catalog("ethereum") if t["id"] == "ethereum:native")
            btc = next(t for t in catalog("bitcoin") if t["id"] == "bitcoin:native")
            mnt_native = next(t for t in catalog("mantle") if t["id"] == "mantle:native")
            mnt_token = next(t for t in catalog("ethereum") if t["symbol"] == "MNT")
            assert eth["kind"] == btc["kind"] == mnt_native["kind"] == "Native"
            assert mnt_token["token_id"] == mnt_native["token_id"] and mnt_token["id"] != mnt_native["id"]
            test_eth = catalog("ethereum-sepolia")[0]
            assert test_eth["coingecko_id"] == "" and test_eth["token_id"] != eth["token_id"]
            run("token", "catalog", "--chain", "ETH", succeeds=False)
            assembly = run("send", "assemble", "--chain", "ethereum", "--symbol", "ETH", "--contract", "0x1111111111111111111111111111111111111111", "--decimals", "6", "--from", "0x2222222222222222222222222222222222222222", "--to", "0x3333333333333333333333333333333333333333", "--amount", "1")
            assert assembly["isNative"] is False and assembly["valueWei"] == "0"
            run("wallet", "watch", "--chain", "ethereum", "--name", "Identity", "--address", "0x1111111111111111111111111111111111111111")
            # Seed deterministic balances, then let separate CLI processes read/group/route them.
            with sqlite3.connect(pathlib.Path(directory) / "spectra.sqlite") as db:
                wallet = json.loads(db.execute("SELECT payload FROM wallets").fetchone()[0])
                def holding(network, amount, contract=None):
                    return dict(name="Ether", symbol="ETH", coinGeckoId="ethereum", chainName=network, tokenStandard="ERC-20" if contract else "Native", contractAddress=contract, amount=amount, priceUsd=100)
                wallet["holdings"] = [holding("Ethereum", 1), holding("Base", 2), holding("Ethereum Sepolia", 3), holding("Ethereum", 4, "0x1111111111111111111111111111111111111111")]
                db.execute("UPDATE wallets SET payload=?", (json.dumps(wallet),))
            groups = run("portfolio", "--stored", "--pin-token", "ethereum")["groups"]
            assert all(g["isPinned"] == (g["id"] == "ethereum") for g in groups)
            options = run("portfolio", "--pin-options")["options"]
            assert any(o["token_id"] == "bitcoin" for o in options)
            assert len([o for o in options if o["symbol"] == "ETH"]) >= 2
            run("portfolio", "--stored", "--pin-token", "ETH", succeeds=False)
            eth_group = next(g for g in groups if g["id"] == eth["token_id"])
            assert sum(h["coin"]["amount"] for h in eth_group["holdings"]) == 3
            test_group = next(g for g in groups if g["id"] == test_eth["token_id"])
            assert all(h["valueUsd"] is None for h in test_group["holdings"])
            assert any(g["id"].startswith("custom:ethereum:erc-20:") for g in groups)
            run("send", "preview", "--wallet", "Identity", "--holding", "Ethereum|ETH", "--amount", "1", succeeds=False)
            run("token", "add", "--chain", "ethereum", "--symbol", "ETH", "--name", "Lookalike", "--contract", "invalid", "--decimals", "18", succeeds=False)
            run("network", "set", "ethereum-sepolia")
            run("network", "set", "ethereum")
            with sqlite3.connect(pathlib.Path(directory) / "spectra.sqlite") as db:
                wallet = json.loads(db.execute("SELECT payload FROM wallets").fetchone()[0])
                assert wallet["networkId"] == "ethereum"
            assert len(next(n for n in run("network", "list")["families"] if n["family"] == "ethereum")["choices"]) >= 3

    def test_price_alerts(self):
        """Persist precise targets and reject invalid or duplicate alerts."""
        with tempfile.TemporaryDirectory(prefix='spectra-alerts-') as directory:
            def run(*args, success=True):
                p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=30)
                assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                return json.loads(p.stdout) if success else None
            run('alert','add','--chain','ethereum','--target','0.000001')
            first=run('alert','list')['alerts'][0]; assert first['target']==0.000001
            run('alert','add','--chain','ethereum','--target','0.000001',success=False)
            run('alert','add','--chain','ethereum','--target','0',success=False)
            run('alert','add','--chain','ethereum','--target','1','--currency','MISSING',success=False)
            run('alert','toggle',first['id']); assert not run('alert','list')['alerts'][0]['enabled']
            run('alert','remove',first['id']); assert not run('alert','list')['alerts']


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
