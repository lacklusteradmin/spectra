#!/usr/bin/env python3
"""Owned send/alert intents through CLI, with only a loopback provider."""
import http.server, json, pathlib, sqlite3, subprocess, sys, tempfile, threading
binary = str(pathlib.Path(sys.argv[1]).resolve())
requests = []
fail = False
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        def answer(call):
            method = call['method']; requests.append(method)
            values = {'eth_getBalance': '0x8ac7230489e80000', 'eth_estimateGas': '0x5208',
                      'eth_getCode': '0x', 'eth_getTransactionCount': '0x7', 'eth_getTransactionByHash': {'nonce':'0x7'},
                      'eth_feeHistory': {'baseFeePerGas':['0x3b9aca00'], 'reward':[['0x77359400']]}}
            assert method in values, method
            return {'jsonrpc':'2.0', 'id':call['id'], 'result':None if fail else values[method]}
        result = list(map(answer, body)) if isinstance(body,list) else answer(body)
        data=json.dumps(result).encode(); self.send_response(200)
        self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
with tempfile.TemporaryDirectory(prefix='spectra-owned-') as directory:
    def run(*args, success=True, rejection=None):
        p=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True)
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
        w['holdings']=[dict(name='Ethereum',symbol='ETH',coinGeckoId='ethereum',chainName='Ethereum',
            tokenStandard='Native',contractAddress=None,amount=10,priceUsd=2000)]
    update_wallets(seed)
    run('alert','add','--chain','ethereum','--target','0.000001')
    first=run('alert','list')['alerts'][0]; assert first['target']==0.000001
    run('alert','add','--chain','ethereum','--target','0.000001',success=False)
    run('alert','add','--chain','ethereum','--target','0',success=False)
    run('alert','add','--chain','ethereum','--target','1','--currency','MISSING',success=False)
    run('alert','toggle',first['id']); assert not run('alert','list')['alerts'][0]['enabled']
    run('alert','remove',first['id']); assert not run('alert','list')['alerts']
    base=['--wallet','Source','--holding','ethereum:native']
    own=run('send','self-check',*base,'--amount','1','--destination',addresses[1])['confirmation']
    assert own['requiresConfirmation'], own
    other=run('send','self-check',*base,'--amount','1','--destination','0x'+'33'*20)['confirmation']
    assert not other['requiresConfirmation'], other
    run('send','self-check',*base,'--amount','0','--destination',addresses[0])
    for amount in ['NaN','-1','0.0000000000000000001']:
        run('send','preview',*base,f'--amount={amount}',success=False,rejection='amount')
    run('send','owned-broadcast',*base,'--amount','1','--destination',addresses[1],success=False)
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
    worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
    try:
        run('settings','set','rpc-endpoint.Ethereum',f'http://127.0.0.1:{server.server_port}')
        quote=run('send','quote',*base,'--amount','1','--destination',addresses[1])['quote']
        assert quote['request']['chain_id']=='ethereum',quote
        assert quote['request']['amount_str']=='1' and quote['request']['to_address']==addresses[1]
        assert quote['preview'] is not None
        assert quote['requires_self_send_confirmation']
        assert quote['request']['evm_overrides']['nonce'] == 7
        run('send','owned-broadcast',*base,'--amount','1','--destination',addresses[1],'--yes',success=False)
        run('send','quote',*base,'--amount','10','--destination',addresses[1],success=False)
        with sqlite3.connect(dbpath) as db:
            wid=db.execute("SELECT id FROM wallets WHERE name='Source'").fetchone()[0]
            row=dict(id='pending',walletId=wid,walletName='Source',kind='send',status='pending',
                chainName='Ethereum',symbol='ETH',assetDisplayName='Ethereum',deploymentId='ethereum:native',
                amount=0.123456789012,address=addresses[1],transactionHash='0x'+'aa'*32,createdAt=1234)
            db.execute('INSERT INTO history_records (id,wallet_id,chain_name,tx_hash,created_at,payload) VALUES (?,?,?,?,?,?)',
                ('pending',wid,'Ethereum',row['transactionHash'],978308434,json.dumps(row)))
        draft=run('send','replacement','pending')['draft']; assert draft['amount']=='0.123456789012',draft
        draft=run('send','replacement','pending','--cancel')['draft']
        assert draft['amount']=='0' and draft['destination']==addresses[0]
        fail=True
        run('send','quote',*base,'--amount','1','--destination',addresses[1],success=False)
        run('send','replacement','pending','--cancel',success=False)
        before=len(requests)
        update_wallets(lambda w: w.update(networkId='ethereum-sepolia'))
        run('send','preview',*base,'--amount','1',success=False)
        assert len(requests)==before,'mismatched network reached provider'
        derived=run('wallet','derived')
        projection=derived.get('derived',derived)['resolved_addresses_by_wallet_id'][wid]
        assert 'Ethereum Sepolia' not in projection and 'Ethereum' not in projection
        assert not any('sendRawTransaction' in method for method in requests)
    finally:
        server.shutdown(); server.server_close(); worker.join()
print('owned send CLI: exact alerts, all-wallet ownership, owned quote, fee refusal, exact replacement, network refusal')
