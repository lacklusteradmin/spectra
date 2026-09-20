#!/usr/bin/env python3
"""Configured diagnostics, typed password verdicts and sealed owned sends, offline."""
import http.server, json, os, pathlib, sqlite3, subprocess, sys, tempfile, threading
binary = str(pathlib.Path(sys.argv[1]).resolve())
seen = []
chain_id = '0xaa36a7'
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        def answer(call):
            method = call['method']; seen.append(method)
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
with tempfile.TemporaryDirectory(prefix='spectra-five-fixes-') as directory:
    def run(*args, success=True, env=None):
        p=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True,
                         env={**os.environ, **(env or {})}, timeout=30)
        assert (p.returncode==0)==success,(args,p.stdout,p.stderr)
        return json.loads(p.stdout)
    for password, confirmation, reason in [('', '', None), ('abc','abc','tooShort'),
            ('密碼','密碼','tooShort'), ('密碼測試','密碼測試',None), ('abcd','abce','confirmationMismatch')]:
        result=run('wallet','check-password',env={'SPECTRA_PASSWORD':password,'SPECTRA_PASSWORD_CONFIRMATION':confirmation})
        assert result=={'valid':reason is None,'rejection':reason}, result
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
    worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
    try:
        endpoint=f'http://127.0.0.1:{server.server_port}'
        run('settings','set','rpc-endpoint.Ethereum',endpoint)
        run('settings','set','rpc-endpoint.Ethereum Sepolia',endpoint)
        run('network','set','ethereum-sepolia')
        report=run('diagnostics','configured','--chain','ethereum')['report']
        assert report['chain_id']=='ethereum-sepolia' and report['rpc_endpoint']==endpoint, report
        assert all(r['passed'] for r in report['results']), report
        run('network','set','ethereum')
        failed=run('diagnostics','configured','--chain','ethereum',success=False)
        assert not failed['ok'], failed
        # Explicit testnets remain explicit even when the family's selected network changes.
        assert run('diagnostics','configured','--chain','ethereum-sepolia')['ok']
        assert seen==['eth_chainId','eth_blockNumber']*3,seen
        chain_id='0x1'
        password='fixture-wallet-password'
        run('wallet','import','--chain','ethereum','--name','Sealed',env={
            'SPECTRA_PASSWORD':password,
            'SPECTRA_SEED':'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'})
        with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
            wid,payload=db.execute('SELECT id,payload FROM wallets').fetchone()
            wallet=json.loads(payload)
            wallet['holdings']=[dict(name='Ethereum',symbol='ETH',coinGeckoId='ethereum',chainName='Ethereum',
                tokenStandard='Native',contractAddress=None,amount=10,priceUsd=0)]
            db.execute('UPDATE wallets SET payload=? WHERE id=?',(json.dumps(wallet),wid))
        args=('send','owned-broadcast','--wallet','Sealed','--holding','ethereum:native','--amount','1',
              '--destination','0x'+'22'*20,'--yes')
        for wrong in ['', 'incorrect']:
            run(*args,success=False,env={'SPECTRA_PASSWORD':wrong})
            assert 'eth_sendRawTransaction' not in seen,seen
        sent=run(*args,env={'SPECTRA_PASSWORD':password})
        assert sent['transactionHash']=='0x'+'11'*32,sent
        assert seen.count('eth_sendRawTransaction')==1,seen
    finally:
        server.shutdown();server.server_close();worker.join()
print('five shell fixes: typed validation, configured network diagnostics and sealed owned send passed')
