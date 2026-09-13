#!/usr/bin/env python3
"""Owned naming, receive, movement and staking through fresh CLI processes."""
import http.server, json, pathlib, sqlite3, subprocess, sys, tempfile, threading
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='spectra-shell-') as directory:
    def run(*args, success=True):
        p=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True)
        assert (p.returncode==0)==success,(args,p.stdout,p.stderr)
        return json.loads(p.stdout) if success else None
    address='0x'+'01'*20
    for i in range(3):
        run('wallet','watch','--chain','ethereum','--address','0x'+f'{i+1:02x}'*20)
    wallets=run('wallet','list')['wallets']
    assert {w['name'] for w in wallets}=={'Wallet 1','Wallet 2','Wallet 3'},wallets
    received=run('pool','receive','Wallet 1')
    assert address in json.dumps(received), received
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
    change_wallets(lambda w: w.update(includeInPortfolioTotal=False) if w['name']=='Wallet 2' else None)
    assert run('alert','movement')['notification'] is None
    quote(0)
    assert run('alert','movement')['notification'] is None
    quote(2000)
    assert run('alert','movement')['notification'] is None
    run('staking','endpoints','--chain','bitcoin',success=False)
    run('staking','endpoints','--chain','solana-devnet',success=False)
    seen=[]
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self,*args):pass
        def do_POST(self):
            req=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            seen.append(req['method']); assert req['method']=='getVoteAccounts',req
            data=json.dumps({'jsonrpc':'2.0','id':req['id'],'result':{'current':[{'votePubkey':'Validator11111111111111111111111111111111','activatedStake':1000,'commission':5}],'delinquent':[]}}).encode()
            self.send_response(200);self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    try:
        endpoint=f'http://127.0.0.1:{server.server_port}'
        run('settings','set','rpc-endpoint.Solana',endpoint)
        assert run('staking','endpoints','--chain','solana')['endpoints'][0]==endpoint
        assert len(run('staking','validators','--chain','solana')['validators'])==1
        assert seen==['getVoteAccounts'],seen
    finally:
        server.shutdown();server.server_close();thread.join()
print('shell ownership CLI: durable default names, receive address, persisted movement policy, configured staking and early refusal')
