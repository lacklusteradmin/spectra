#!/usr/bin/env python3
"""Offline refresh, recovery logs, configured network and staking node checks.

Run: python3 scripts/cli-diagnostics.py [path/to/spectra] [TestClass.test_name]
Uses temporary stores and loopback nodes; no public network is required.
"""
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
import unittest

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else
                          pathlib.Path(__file__).resolve().parents[1] / 'target/debug/spectra').resolve())


class DiagnosticsTests(unittest.TestCase):
    def test_offline_refresh(self):
        """Offline refreshes and unsupported rescans report their real outcomes."""
        with tempfile.TemporaryDirectory(prefix='spectra-refresh-') as directory:
            def run(*args, success=True):
                p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=30)
                assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                return json.loads(p.stdout) if success else None
            conditions = dict(appIsActive=True, isNetworkReachable=False, isConstrainedNetwork=False, isExpensiveNetwork=False, isLowPowerMode=False, batteryLevel=1, wantsPriceRefresh=True)
            for intent in ['user','scheduled','foreground','balancesUpdated',{'afterSend':{'chain_id':'ethereum'}},{'chain':{'chain_id':'bitcoin'}}]:
                result = run('diagnostics','refresh','--intent',json.dumps(intent),'--conditions',json.dumps(conditions))['refresh']
                assert result['pending'] is None and result['failures'] == []
            run('diagnostics','refresh','--intent',json.dumps({'afterSend':{'chain_id':'missing'}}),'--conditions',json.dumps(conditions),success=False)
            result = run('diagnostics','refresh','--intent',json.dumps({'deepRescan':{'chain_id':'bitcoin'}}),'--conditions',json.dumps(conditions))['refresh']
            assert result['failures'] and result['pending'] is None, 'offline rescan falsely succeeded'
            for chain in ['ethereum','missing']:
                run('diagnostics','refresh','--intent',json.dumps({'deepRescan':{'chain_id':chain}}),'--conditions',json.dumps(conditions),success=False)

    def test_staking_endpoint(self):
        """Validator queries use the configured node and reject unsupported networks."""
        with tempfile.TemporaryDirectory(prefix='spectra-refresh-') as directory:
            def run(*args, success=True):
                p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=30)
                assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                return json.loads(p.stdout) if success else None
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

    def test_configured_network(self):
        """Diagnostics use the selected network and refuse a node on the wrong chain."""
        seen = []

        chain_id = '0xaa36a7'

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                def answer(call):
                    method = call['method']; seen.append(method)
                    values = {'eth_chainId': chain_id, 'eth_blockNumber': '0x123'}
                    assert method in values, method
                    return {'jsonrpc':'2.0', 'id':call['id'], 'result':values[method]}
                result = list(map(answer, body)) if isinstance(body,list) else answer(body)
                data=json.dumps(result).encode(); self.send_response(200)
                self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)

        with tempfile.TemporaryDirectory(prefix='spectra-diagnostics-') as directory:
            def run(*args, success=True, env=None):
                p=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True,
                                 env={**os.environ, **(env or {})}, timeout=30)
                assert (p.returncode==0)==success,(args,p.stdout,p.stderr)
                return json.loads(p.stdout)
            server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
            worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
            try:
                endpoint=f'http://127.0.0.1:{server.server_port}'
                run('settings','set','rpc-endpoint.Ethereum',endpoint)
                run('settings','set','rpc-endpoint.Ethereum Sepolia',endpoint)
                run('network','set','ethereum-sepolia')
                report=run('diagnostics','configured','--chain','ethereum')['report']
                assert report['chain_id']=='ethereum-sepolia' and report['rpc_endpoint']==endpoint, report
                assert report['results'] and all(r['passed'] for r in report['results']), report
                run('network','set','ethereum')
                failed=run('diagnostics','configured','--chain','ethereum',success=False)
                assert not failed['ok'], failed
                # Explicit testnets remain explicit even when the family's selected network changes.
                assert run('diagnostics','configured','--chain','ethereum-sepolia')['ok']
                assert seen==['eth_chainId','eth_blockNumber']*3,seen
            finally:
                server.shutdown();server.server_close();worker.join()

    def test_recovery_and_offline_policy(self):
        """Persist diagnostic recovery logs and suppress offline background work."""
        with tempfile.TemporaryDirectory(prefix='spectra-refresh-') as directory:
            def run(*args, success=True):
                p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=30)
                assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
                return json.loads(p.stdout) if success else None
            run('diagnostics', 'state', '--command', json.dumps({'Degraded': {'chain_name': 'Solana', 'detail': 'timeout'}}))
            degraded = run('diagnostics', 'state')['state']
            assert degraded['degraded'], degraded
            run('diagnostics', 'state', '--command', json.dumps({'Healthy': {'chain_name': 'Solana'}}))
            recovered = run('diagnostics', 'state')['state']
            assert not recovered['degraded'] and 'Solana' in recovered['last_good_unix'], recovered
            assert len(recovered['logs']) == 2 and recovered['logs'][0]['input']['message'] == 'Chain recovered', recovered
            conditions = dict(appIsActive=True, isNetworkReachable=False, isConstrainedNetwork=False,
                              isExpensiveNetwork=False, isLowPowerMode=False, batteryLevel=1, wantsPriceRefresh=True)
            plan = run('diagnostics', 'maintenance', '--conditions', json.dumps(conditions))['plan']
            assert plan['runBackgroundTick'] is False and plan['allowHeavyBackgroundWork'] is False, plan
            assert plan['pollSeconds'] == 300, plan


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
