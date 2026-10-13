#!/usr/bin/env python3
"""ICP and transparent Zcash sends staged across processes, against loopback
providers; no real funds or keys.

Each network's transfer is built, signed and broadcast in its own process:
nothing is submitted before `broadcast-signed --yes`, the signed send is read
back as signed, and a retry submits the same bytes. What each stage checks
is core's, tested with `cargo test`.
"""
import http.server, json, os, pathlib, subprocess, sys, tempfile, threading
binary=str(pathlib.Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/spectra').resolve())
# NU6.3 (Ironwood) is the network's upgrade since block 3,428,143.
state=dict(submitted=[],expected='',branch='37a5165b',height=3500000,genesis='00040fe8ec8471911baa1db1266ea15dd06b4a8a5c453883c000b031973dce08')
network={'blockchain':'Internet Computer','network':'00000000000000020101'}
class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self,*_):pass
    def reply(self,value):
        body=json.dumps(value).encode();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
    def do_GET(self):
        if self.path=='/api/v2/block-index/0':return self.reply({'blockHash':state['genesis']})
        if self.path=='/api/v2':return self.reply({'backend':{'blocks':state['height'],'consensus':{'chaintip':state['branch'],'nextblock':state['branch']}}})
        if self.path.startswith('/api/v2/utxo/'):
            return self.reply([{'txid':'11'*32,'vout':2,'value':'1000000','confirmations':100}])
        raise AssertionError(self.path)
    def do_POST(self):
        raw=self.rfile.read(int(self.headers['Content-Length']))
        if self.path=='/api/v2/sendtx/':
            state['submitted'].append(raw.decode());return self.reply({'result':state['expected']})
        body=json.loads(raw)
        if self.path=='/network/list':return self.reply({'network_identifiers':[network]})
        if self.path=='/construction/preprocess':return self.reply({'options':{'request_types':['TRANSACTION']}})
        if self.path=='/construction/metadata':return self.reply({'suggested_fee':[{'value':'10000','currency':{'symbol':'ICP','decimals':8}}]})
        if self.path=='/construction/submit':
            assert body['network_identifier']==network
            assert body['signed_transaction']
            state['submitted'].append(body['signed_transaction']);return self.reply({'transaction_identifier':{'hash':state['expected']}})
        raise AssertionError(self.path)
node=http.server.ThreadingHTTPServer(('127.0.0.1',0),Node)
threading.Thread(target=node.serve_forever,daemon=True).start()
url=f'http://127.0.0.1:{node.server_port}'
try:
 with tempfile.TemporaryDirectory(prefix='spectra-icp-zec-') as directory:
    def run(*args):
        r=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True,timeout=60,env={**os.environ,'SPECTRA_PASSWORD':'fixture-password','SPECTRA_SEED':'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'})
        assert r.returncode==0,(args,r.stdout,r.stderr)
        return json.loads(r.stdout)
    for chain,name in [('internet-computer','IcpStages'),('zcash','ZecStages')]:
        run('wallet','import','--chain',chain,'--name',name)
        run('endpoints','--chain',chain,'--api','icp-rosetta' if chain == 'internet-computer' else 'blockbook','--capabilities','balance,history,fee,broadcast,verification' + (',utxo' if chain != 'internet-computer' else ''),'--add',url)
        # Sending back to the derived account is a protocol fixture, no real network.
        destination=run('send','identity','--from',name)['address']
        before=len(state['submitted'])
        prepared=run('send','build','--from',name,'--to',destination,'--amount','0.001','--endpoint',url)['artifact']
        assert prepared['stage']=='Prepared' and len(state['submitted'])==before
        signed=run('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',url)['artifact']
        assert signed['stage']=='Signed' and len(state['submitted'])==before
        assert run('send','inspect',prepared['id'])['artifact']==signed
        state['expected']=signed['transaction_hash'];assert len(state['expected'])==64
        sent=run('send','broadcast-signed',prepared['id'],'--endpoint',url,'--yes')['artifact']
        assert sent['attempts'][-1]['outcome']=='Accepted',sent
        run('send','broadcast-signed',prepared['id'],'--endpoint',url,'--yes')
        assert len(state['submitted'])==before+2 and state['submitted'][-1]==state['submitted'][-2]
        print(f'PASS {chain}: build, sign and broadcast in separate processes, and an identical retry')
finally:node.shutdown();node.server_close()
