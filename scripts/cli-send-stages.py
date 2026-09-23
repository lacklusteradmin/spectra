#!/usr/bin/env python3
"""Durable build/sign/explicit-node submission against loopback nodes only."""
import http.server
import concurrent.futures
import json
import os
import pathlib
import sqlite3
import socket
import subprocess
import sys
import tempfile
import threading

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
state = dict(nonce='0x7', expected='', mode='ok', submitted=[])
class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_POST(self):
        request=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        def answer(call):
            method=call['method']
            if method=='eth_call':
                if state.get('metadata_fail'):
                    return dict(jsonrpc='2.0',id=call['id'],error=dict(code=-32000,message='metadata unavailable'))
                value = '0x6' if call['params'][0]['data']=='0x313ce567' else '0x'+'TOKEN'.encode().hex().ljust(64,'0')
                return dict(jsonrpc='2.0',id=call['id'],result=value)
            if method=='eth_sendRawTransaction':
                state['submitted'].append((self.server.server_port, call['params'][0]))
                if self.server.mode=='drop':
                    self.connection.shutdown(socket.SHUT_RDWR)
                    self.connection.close()
                    raise ConnectionAbortedError('fixture lost submission response')
                if self.server.mode=='reject':
                    return dict(jsonrpc='2.0',id=call['id'],error=dict(code=-32000,message='transaction rejected'))
            values={'eth_chainId':self.server.chain_id,'eth_getTransactionCount':state['nonce'],
                'eth_getBalance':'0x8ac7230489e80000','eth_estimateGas':'0x5208','eth_getCode':'0x',
                'eth_feeHistory':{'baseFeePerGas':['0x3b9aca00'],'reward':[['0x77359400']]},
                'eth_sendRawTransaction':state['expected']}
            assert method in values, method
            return dict(jsonrpc='2.0',id=call['id'],result=values[method])
        try:
            body=json.dumps([answer(c) for c in request] if isinstance(request,list) else answer(request)).encode()
        except ConnectionAbortedError:
            return
        self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)

servers=[]
try:
    for mode,chain in [('ok','0x1'),('reject','0x1'),('ok','0x2')]:
        node=http.server.ThreadingHTTPServer(('127.0.0.1',0),Node);node.mode=mode;node.chain_id=chain
        threading.Thread(target=node.serve_forever,daemon=True).start();servers.append(node)
    urls=[f'http://127.0.0.1:{s.server_port}' for s in servers]
    with tempfile.TemporaryDirectory(prefix='spectra-stages-') as directory:
        def run(*args, success=True):
            result=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True,
                env={**os.environ,'SPECTRA_PASSWORD':'stage-fixture-password','SPECTRA_SEED':'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'},timeout=45)
            assert (result.returncode==0)==success,(args,result.stdout,result.stderr)
            return json.loads(result.stdout)
        run('wallet','import','--chain','ethereum','--name','Stages')
        run('settings','set','rpc-endpoint.Ethereum',urls[0])
        def build():
            return run('send','build','--from','Stages','--to','0x'+'22'*20,'--amount','0.01','--endpoint',urls[0])['artifact']
        exact=run('send','build','--from','Stages','--to','0x'+'22'*20,
            '--amount','9007199254.740993000000000001','--endpoint',urls[0])['artifact']
        assert json.loads(exact['prepared_details'])['Evm']['value_wei']==9007199254740993000000000001
        token_args=('send','build','--from','Stages','--to','0x'+'22'*20,
            '--contract','0x'+'44'*20,'--amount','9007199254.740993','--endpoint',urls[0])
        token=run(*token_args,'--decimals','6')['artifact']
        raw=json.loads(token['prepared_details'])['Evm']
        assert raw['value_wei']==0 and int.from_bytes(bytes(raw['data'][-32:]),'big')==9007199254740993
        run(*token_args,'--decimals','18',success=False)
        state['metadata_fail']=True
        run(*token_args,'--decimals','6',success=False)
        state['metadata_fail']=False
        first=build();competing=build();assert first['stage']=='Prepared' and not state['submitted']
        assert run('send','inspect',first['id'])['artifact']==first
        assert first['review']['warnings'] == [{'code':'new_address'}]
        self_send=run('send','build','--from','Stages','--to','0x9858EfFD232B4033E47d90003D41EC34EcaEda94',
            '--amount','0.000000000000000001','--endpoint',urls[0])['artifact']
        assert self_send['review']['requires_self_send_confirmation']
        restored=run('send','inspect',self_send['id'])['artifact']
        assert restored['review']==self_send['review'] and restored['amount']=='0.000000000000000001'
        with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
            data=json.loads(db.execute('SELECT payload FROM send_artifacts WHERE id=?',(self_send['id'],)).fetchone()[0])
            data['view']['review']['requires_self_send_confirmation']=False
            db.execute('UPDATE send_artifacts SET payload=? WHERE id=?',(json.dumps(data),self_send['id']))
        run('send','inspect',self_send['id'],success=False)
        run('send','sign',self_send['id'],'--review-digest',self_send['review_digest'],'--endpoint',urls[0],success=False)
        data['view']['review']['requires_self_send_confirmation']=True
        with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
            db.execute('UPDATE send_artifacts SET payload=? WHERE id=?',(json.dumps(data),self_send['id']))
        run('send','sign',first['id'],'--review-digest','altered','--endpoint',urls[0],success=False)
        assert not state['submitted']
        signed=run('send','sign',first['id'],'--review-digest',first['review_digest'],'--endpoint',urls[0])['artifact']
        assert signed['stage']=='Signed' and signed['signed_payload'] and not state['submitted']
        run('send','sign',competing['id'],'--review-digest',competing['review_digest'],'--endpoint',urls[0],success=False)
        assert run('send','inspect',first['id'])['artifact']==signed
        state['expected']=signed['transaction_hash']
        run('send','broadcast-signed',first['id'],'--endpoint',urls[0],'--endpoint',urls[2],'--yes',success=False)
        assert not state['submitted'],'all endpoints must validate before first submission'
        result=run('send','broadcast-signed',first['id'],'--endpoint',urls[0],'--endpoint',urls[1],'--yes')['artifact']
        assert result['attempts'][0]['outcome']=='Accepted'
        assert result['attempts'][1]['outcome'] in ('Rejected','Uncertain')
        run('send','broadcast-signed',first['id'],'--endpoint',urls[0],'--yes')
        assert len(state['submitted'])>=3 and all(payload==signed['signed_payload'] for _,payload in state['submitted'])
        run('send','sign',first['id'],'--review-digest',first['review_digest'],'--endpoint',urls[0],success=False)
        servers[1].mode='drop'
        lost=run('send','broadcast-signed',first['id'],'--endpoint',urls[1],'--yes')['artifact']
        assert lost['attempts'][-1]['outcome']=='Uncertain'
        assert all(payload==signed['signed_payload'] for _,payload in state['submitted'])
        assert run('send','inspect',first['id'])['artifact']['selected_endpoints']==[urls[1]]
        replacement=run('send','build','--from','Stages','--to','0x'+'33'*20,'--amount','0',
            '--nonce','7','--max-fee-gwei','10','--priority-fee-gwei','4','--endpoint',urls[0])['artifact']
        replaced=run('send','sign',replacement['id'],'--review-digest',replacement['review_digest'],
            '--endpoint',urls[0])['artifact']
        assert replaced['signed_payload'] != signed['signed_payload']
        assert run('send','inspect',first['id'])['artifact']['signed_payload']==signed['signed_payload']
        stale=build();state['nonce']='0x9'
        run('send','sign',stale['id'],'--review-digest',stale['review_digest'],'--endpoint',urls[0],success=False)
        contenders=[build(),build()]
        def race_sign(artifact):
            return subprocess.run([binary,'--data-dir',directory,'--json','send','sign',artifact['id'],
                '--review-digest',artifact['review_digest'],'--endpoint',urls[0]], capture_output=True,text=True,
                env={**os.environ,'SPECTRA_PASSWORD':'stage-fixture-password'},timeout=45)
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as workers:
            raced=list(workers.map(race_sign, contenders))
        assert sum(r.returncode==0 for r in raced)==1, [(r.stdout,r.stderr) for r in raced]
        assert any('reserved' in r.stdout for r in raced if r.returncode), [(r.stdout,r.stderr) for r in raced]
        altered=build()
        with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
            data=json.loads(db.execute('SELECT payload FROM send_artifacts WHERE id=?',(altered['id'],)).fetchone()[0])
            data['request']['amount_str']='9'
            db.execute('UPDATE send_artifacts SET payload=? WHERE id=?',(json.dumps(data),altered['id']))
        run('send','inspect',altered['id'],success=False)
        configured=run('send','configured-endpoints','ethereum')['endpoints']
        assert urls[0] in configured
        with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
            data=json.loads(db.execute('SELECT payload FROM send_artifacts WHERE id=?',(first['id'],)).fetchone()[0])
            data['submission']['payload']='0xdeadbeef'
            db.execute('UPDATE send_artifacts SET payload=? WHERE id=?',(json.dumps(data),first['id']))
        run('send','broadcast-signed',first['id'],'--endpoint',urls[0],'--yes',success=False)
        with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
            for (data,) in db.execute('SELECT payload FROM send_artifacts'):
                assert 'abandon' not in data and 'private_key' not in data
        run('wallet','delete','Stages','--yes')
        with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
            assert db.execute('SELECT count(*) FROM send_artifacts').fetchone()[0] == 0
            assert db.execute('SELECT count(*) FROM send_reservations').fetchone()[0] == 0
    print('transparent send stage acceptance passed')
finally:
    for server in servers:server.shutdown();server.server_close()
