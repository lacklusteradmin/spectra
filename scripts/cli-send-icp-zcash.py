#!/usr/bin/env python3
"""ICP/Zcash stages with loopback-only providers; no real funds or keys."""
import hashlib, http.server, json, os, pathlib, subprocess, sys, tempfile, threading
binary=str(pathlib.Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/spectra').resolve())
# NU6.3 (Ironwood) is the network's upgrade since block 3,428,143.
state=dict(submitted=[],expected='',branch='37a5165b',height=3500000,genesis='00040fe8ec8471911baa1db1266ea15dd06b4a8a5c453883c000b031973dce08',spent=False,fee='10000')
network={'blockchain':'Internet Computer','network':'00000000000000020101'}
class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self,*_):pass
    def reply(self,value):
        body=json.dumps(value).encode();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
    def do_GET(self):
        if self.path=='/api/v2/block-index/0':return self.reply({'blockHash':state['genesis']})
        if self.path=='/api/v2':return self.reply({'backend':{'blocks':state['height'],'consensus':{'chaintip':state['branch'],'nextblock':state['branch']}}})
        if self.path.startswith('/api/v2/utxo/'):
            return self.reply([] if state['spent'] else [{'txid':'11'*32,'vout':2,'value':'1000000','confirmations':100}])
        raise AssertionError(self.path)
    def do_POST(self):
        raw=self.rfile.read(int(self.headers['Content-Length']))
        if self.path=='/api/v2/sendtx/':
            state['submitted'].append(raw.decode());return self.reply({'result':state['expected']})
        body=json.loads(raw)
        if self.path=='/network/list':return self.reply({'network_identifiers':[network]})
        if self.path=='/construction/preprocess':return self.reply({'options':{'request_types':['TRANSACTION']}})
        if self.path=='/construction/metadata':return self.reply({'suggested_fee':[{'value':state['fee'],'currency':{'symbol':'ICP','decimals':8}}]})
        if self.path=='/construction/submit':
            assert body['network_identifier']==network
            assert body['signed_transaction']
            state['submitted'].append(body['signed_transaction']);return self.reply({'transaction_identifier':{'hash':state['expected']}})
        raise AssertionError(self.path)
B58='123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
def b58check(text):
    n=0
    for c in text:n=n*58+B58.index(c)
    raw=n.to_bytes((n.bit_length()+7)//8,'big')
    assert hashlib.sha256(hashlib.sha256(raw[:-4]).digest()).digest()[:4]==raw[-4:]
    return raw[:-4]
def tex_address(transparent):
    """ZIP-320: Bech32m of the P2PKH address's key hash, under `tex`."""
    data,acc,bits=[],0,0
    for byte in b58check(transparent)[2:]:
        acc=(acc<<8)|byte;bits+=8
        while bits>=5:bits-=5;data.append((acc>>bits)&31)
    if bits:data.append((acc<<(5-bits))&31)
    def polymod(values):
        chk=1
        for v in values:
            top=chk>>25;chk=(chk&0x1ffffff)<<5^v
            for i,g in enumerate([0x3b6a57b2,0x26508e6d,0x1ea119fa,0x3d4233dd,0x2a1462b3]):chk^=g if (top>>i)&1 else 0
        return chk
    hrp='tex';expanded=[ord(c)>>5 for c in hrp]+[0]+[ord(c)&31 for c in hrp]
    check=polymod(expanded+data+[0]*6)^0x2bc830a3
    return hrp+'1'+''.join('qpzry9x8gf2tvdw0s3jn54khce6mua7l'[d] for d in data+[(check>>5*(5-i))&31 for i in range(6)])
# ZIP-320's reference pair.
assert tex_address('t1VmmGiyjVNeCjxDZzg7vZmd99WyzVby9yC')=='tex1s2rt77ggv6q989lr49rkgzmh5slsksa9khdgte'
node=http.server.ThreadingHTTPServer(('127.0.0.1',0),Node)
threading.Thread(target=node.serve_forever,daemon=True).start()
url=f'http://127.0.0.1:{node.server_port}'
try:
 with tempfile.TemporaryDirectory(prefix='spectra-icp-zec-') as directory:
    def run(*args,success=True):
        r=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True,timeout=60,env={**os.environ,'SPECTRA_PASSWORD':'fixture-password','SPECTRA_SEED':'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'})
        assert (r.returncode==0)==success,(args,r.stdout,r.stderr)
        return json.loads(r.stdout)
    for chain,display,name in [('internet-computer','Internet Computer','IcpStages'),('zcash','Zcash','ZecStages')]:
        run('wallet','import','--chain',chain,'--name',name)
        run('endpoints','--chain',chain,'--api','icp-rosetta' if chain == 'internet-computer' else 'blockbook','--capabilities','balance,history,fee,broadcast,verification' + (',utxo' if chain != 'internet-computer' else ''),'--add',url)
        # Sending back to the derived account is a protocol fixture, no real network.
        identity=run('send','identity','--from',name)
        destination=identity['address']
        def build():return run('send','build','--from',name,'--to',destination,'--amount','0.001','--endpoint',url)['artifact']
        before=len(state['submitted']);prepared=build()
        assert prepared['stage']=='Prepared' and len(state['submitted'])==before
        competing=build()
        if chain=='zcash':
            # Transparent funds pay a TEX address (ZIP-320) the P2PKH script
            # of its key hash, and refuse a shielded one.
            tex=run('send','build','--from',name,'--to',tex_address(destination),'--amount','0.001','--endpoint',url)['artifact']
            outputs=json.loads(tex['prepared_details'])['Zcash']['outputs']
            assert bytes(outputs[0][0])==bytes([0x76,0xa9,0x14])+b58check(destination)[2:]+bytes([0x88,0xac]) and outputs[0][1]==100000,outputs
            unified=json.loads((pathlib.Path(__file__).resolve().parents[1]/'core/tests/fixtures/zcash-addresses.json').read_text())['unified']['vectors'][0]['unified_addr']
            refused=run('send','build','--from',name,'--to',unified,'--amount','0.001','--endpoint',url,success=False)
            assert "paid from the wallet's shielded funds" in refused['error'],refused
            # A branch other than the next block's is refused: NU5's, or
            # NU6.2's, which NU6.3 replaced.
            for stale in ('c2d6d0b4','5437f330'):
                state['branch']=stale;run('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',url,success=False)
            state['branch']='37a5165b';state['spent']=True
            run('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',url,success=False);state['spent']=False
        else:
            state['fee']='10001';run('send','build','--from',name,'--to',destination,'--amount','0.001','--endpoint',url,success=False);state['fee']='10000'
            run('send','build','--from',name,'--to','00'*32,'--amount','0.001','--endpoint',url,success=False)
        signed=run('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',url)['artifact']
        assert signed['stage']=='Signed' and len(state['submitted'])==before
        assert run('send','inspect',prepared['id'])['artifact']==signed
        if chain=='zcash':run('send','sign',competing['id'],'--review-digest',competing['review_digest'],'--endpoint',url,success=False)
        state['expected']=signed['transaction_hash'];assert len(state['expected'])==64
        sent=run('send','broadcast-signed',prepared['id'],'--endpoint',url,'--yes')['artifact']
        assert sent['attempts'][-1]['outcome']=='Accepted',sent
        run('send','broadcast-signed',prepared['id'],'--endpoint',url,'--yes')
        assert len(state['submitted'])==before+2 and state['submitted'][-1]==state['submitted'][-2]
        if chain=='zcash':
            state['height']+=41
            run('send','broadcast-signed',prepared['id'],'--endpoint',url,'--yes',success=False)
            assert len(state['submitted'])==before+2
        print(f'PASS {chain}: local build/sign, restart, rejection, explicit broadcast and identical retry')
finally:node.shutdown();node.server_close()
