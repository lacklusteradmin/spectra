#!/usr/bin/env python3
"""Send review, replacement and password-protected broadcast with local nodes.

Run: python3 scripts/cli-send.py [path/to/spectra] [TestClass.test_name]
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


class SendTests(unittest.TestCase):
    def test_password_protected_broadcast(self):
        """Wrong passwords never broadcast; the right password sends exactly once."""
        seen = []
        submitted = []

        chain_id = '0x1'

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                def answer(call):
                    method = call['method']; seen.append(method)
                    if method == 'eth_sendRawTransaction':
                        submitted.append(call['params'][0])
                        with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
                            artifacts = [json.loads(row[0]) for row in db.execute('SELECT payload FROM send_artifacts')]
                        artifact = next(a for a in artifacts if a['submission'] and a['submission']['payload'] == call['params'][0])
                        return {'jsonrpc':'2.0', 'id':call['id'], 'result':artifact['view']['transaction_hash']}
                    values = {'eth_chainId': chain_id, 'eth_blockNumber': '0x123',
                              'eth_getBalance': '0x8ac7230489e80000', 'eth_estimateGas': '0x5208',
                              'eth_getCode': '0x', 'eth_getTransactionCount': '0x7',
                              'eth_feeHistory': {'baseFeePerGas':['0x3b9aca00'], 'reward':[['0x77359400']]},
                              'eth_sendRawTransaction': '0x'+'11'*32}
                    assert method in values, method
                    return {'jsonrpc':'2.0', 'id':call['id'], 'result':values[method]}
                result = list(map(answer, body)) if isinstance(body,list) else answer(body)
                data=json.dumps(result).encode(); self.send_response(200)
                self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)

        with tempfile.TemporaryDirectory(prefix='spectra-send-password-') as directory:
            def run(*args, success=True, env=None):
                p=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True,
                                 env={**os.environ, **(env or {})}, timeout=30)
                assert (p.returncode==0)==success,(args,p.stdout,p.stderr)
                return json.loads(p.stdout)
            server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
            worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
            try:
                endpoint=f'http://127.0.0.1:{server.server_port}'
                run('endpoints','--chain','ethereum','--api','evm-json-rpc','--capabilities','balance,fee,broadcast,verification,token-balance','--add',endpoint)

                password='fixture-wallet-password'
                run('wallet','import','--chain','ethereum','--name','Sealed',env={
                    'SPECTRA_PASSWORD':password,
                    'SPECTRA_SEED':'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'})
                with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
                    wid,payload=db.execute('SELECT id,payload FROM wallets').fetchone()
                    wallet=json.loads(payload)
                    wallet['holdings']=[dict(name='Ethereum',symbol='ETH',coingeckoId='ethereum',chainId='ethereum',
                        tokenStandard='Native',contractAddress=None,amount='10')]
                    db.execute('UPDATE wallets SET payload=? WHERE id=?',(json.dumps(wallet),wid))
                args=('send','owned-broadcast','--wallet','Sealed','--holding','ethereum:native','--amount','1',
                      '--destination','0x'+'22'*20,'--yes')
                for wrong in ['', 'incorrect']:
                    run(*args,success=False,env={'SPECTRA_PASSWORD':wrong})
                    assert 'eth_sendRawTransaction' not in seen,seen
                sent=run(*args,env={'SPECTRA_PASSWORD':password})
                assert sent['transactionHash'].startswith('0x') and len(sent['transactionHash']) == 66,sent
                assert seen.count('eth_sendRawTransaction')==1,seen
                assert len(submitted) == 1 and submitted[0].startswith('0x'), submitted
                assert len(bytes.fromhex(submitted[0][2:])) > 65, submitted
                with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
                    saved = [json.loads(row[0]) for row in db.execute('SELECT payload FROM history_records')]
                sent_records = [row for row in saved if row.get('transactionHash') == sent['transactionHash']]
                assert len(sent_records) == 1, saved
                record = sent_records[0]
                assert record['walletId'] == wid and record['chainId'] == 'ethereum', record
                assert record['amount'] == '1' and record['address'] == '0x'+'22'*20, record
                assert record['status'] == 'pending', record
            finally:
                server.shutdown();server.server_close();worker.join()

    def test_review_and_replacement(self):
        """Validate send review, self-send confirmation, fees and replacement identity."""
        requests = []

        fail = False

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                def answer(call):
                    method = call['method']; requests.append(method)
                    values = {'eth_chainId': '0x1', 'eth_getBalance': '0x8ac7230489e80000', 'eth_estimateGas': '0x5208',
                              'eth_getCode': '0x', 'eth_getTransactionCount': '0x7', 'eth_getTransactionByHash': {'nonce':'0x7'},
                              'eth_feeHistory': {'baseFeePerGas':['0x3b9aca00'], 'reward':[['0x77359400']]}}
                    assert method in values, method
                    return {'jsonrpc':'2.0', 'id':call['id'], 'result':None if fail else values[method]}
                result = list(map(answer, body)) if isinstance(body,list) else answer(body)
                data=json.dumps(result).encode(); self.send_response(200)
                self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)

        with tempfile.TemporaryDirectory(prefix='spectra-review-') as directory:
            def run(*args, success=True, rejection=None):
                p=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True, timeout=60)
                assert (p.returncode==0)==success,(args,p.stdout,p.stderr)
                if rejection is not None:
                    assert p.returncode == 3,(args,p.stdout,p.stderr)
                    assert rejection in json.loads(p.stdout)['error'].lower(),(args,p.stdout,p.stderr)
                return json.loads(p.stdout) if success else None
            addresses=['0x'+'11'*20, '0x'+'22'*20]
            for name,address in zip(['Source','Other'],addresses):
                run('wallet','watch','--chain','ethereum','--address',address,'--name',name)
            dbpath=pathlib.Path(directory)/'spectra.sqlite'
            def update_wallets(change):
                with sqlite3.connect(dbpath) as db:
                    for id,payload in db.execute('SELECT id,payload FROM wallets').fetchall():
                        w=json.loads(payload); change(w)
                        db.execute('UPDATE wallets SET payload=? WHERE id=?',(json.dumps(w),id))
            def seed(w):
                w['holdings']=[dict(name='Ethereum',symbol='ETH',coingeckoId='ethereum',chainId='ethereum',
                    tokenStandard='Native',contractAddress=None,amount='10')]
            update_wallets(seed)
            base=['--wallet','Source','--holding','ethereum:native']
            assert run('send','self-check',*base,'--destination',addresses[1])['ownAddress']
            assert not run('send','self-check',*base,'--destination','0x'+'33'*20)['ownAddress']
            for amount in ['NaN','-1','0.0000000000000000001']:
                run('send','preview',*base,f'--amount={amount}',success=False,rejection='amount')
            run('send','owned-broadcast',*base,'--amount','1','--destination',addresses[1],success=False)
            server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
            worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
            try:
                run('endpoints','--chain','ethereum','--api','evm-json-rpc','--capabilities','balance,fee,broadcast,verification,token-balance','--add',f'http://127.0.0.1:{server.server_port}')
                quote=run('send','quote',*base,'--amount','1','--destination',addresses[1])['quote']
                assert quote['request']['chain_id']=='ethereum',quote
                assert quote['request']['amount_str']=='1' and quote['request']['to_address']==addresses[1]
                assert quote['preview'] is not None
                preview=run('send','preview',*base,'--amount','1','--destination',addresses[1])['preview']
                assert preview['holding_key']=='ethereum:native' and preview['chain_id']=='ethereum', preview
                assert set(preview['shortcuts'])=={'25','50','75','100'}, preview
                assert 0 < float(preview['shortcuts']['100']) < 10, preview
                assert float(preview['details']['maxSendable']) < 10, preview
                risk=run('send','probe','--wallet','Source','--asset','ETH','--to',addresses[1])
                assert risk['activity']=='funded', risk
                assert quote['requires_self_send_confirmation']
                artifact=run('send','build-owned',*base,'--amount','1','--destination',addresses[1])['artifact']
                assert artifact['review']['requires_self_send_confirmation']
                assert artifact['review']['warnings']==quote['warnings']
                assert artifact['review']['recipient_warnings']==quote['recipient_warnings']
                assert run('send','inspect',artifact['id'])['artifact']==artifact

                assert quote['request']['evm_overrides']['nonce'] == 7
                run('send','owned-broadcast',*base,'--amount','1','--destination',addresses[1],'--yes',success=False)
                run('send','quote',*base,'--amount','10','--destination',addresses[1],success=False)
                with sqlite3.connect(dbpath) as db:
                    wid=db.execute("SELECT id FROM wallets WHERE name='Source'").fetchone()[0]
                    row=dict(id='pending',walletId=wid,walletName='Source',kind='send',status='pending',
                        chainId='ethereum',symbol='ETH',assetDisplayName='Ethereum',deploymentId='ethereum:native',
                        amount='0.123456789012',address=addresses[1],transactionHash='0x'+'aa'*32,createdAtUnix=1234)
                    db.execute('INSERT INTO history_records (id,wallet_id,chain_id,tx_hash,created_at,payload) VALUES (?,?,?,?,?,?)',
                        ('pending',wid,'ethereum',row['transactionHash'],1234,json.dumps(row)))
                ends=run('txs','--endpoints','pending')['endpoints']
                # Mine means the sending wallet's, not any wallet the user holds.
                assert ends=={'from':None,'to':{'address':addresses[1],'isMine':False}},ends
                draft=run('send','replacement','pending')['draft']; assert draft['amount']=='0.123456789012',draft
                draft=run('send','replacement','pending','--cancel')['draft']
                assert draft['amount']=='0' and draft['destination']==addresses[0]
                fail=True
                run('send','quote',*base,'--amount','1','--destination',addresses[1],success=False)
                run('send','replacement','pending','--cancel',success=False)
                before=len(requests)
                update_wallets(lambda w: w.update(chainId='ethereum-sepolia'))
                run('send','preview',*base,'--amount','1',success=False)
                assert len(requests)==before,'mismatched network reached provider'
                derived=run('wallet','derived')
                sendable=derived.get('derived',derived)['send_coins_by_wallet_id'].get(wid,[])
                assert not sendable,sendable
                assert not any('sendRawTransaction' in method for method in requests)
            finally:
                server.shutdown(); server.server_close(); worker.join()


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
