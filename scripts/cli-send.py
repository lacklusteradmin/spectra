#!/usr/bin/env python3
"""A password-protected owned send, reviewed and broadcast in one process.

`send owned-broadcast` is refused without --yes; with it, and the wallet's
password from the environment, the node receives the signed transaction
exactly once. Review, fees and the history it records are core's rules,
tested with `cargo test`.

Run: python3 scripts/cli-send.py [path/to/spectra] [TestClass.test_name]
Uses a temporary store and a loopback node; no public network is required.
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
        """Refused without --yes; with it and the password, sent exactly once."""
        submitted = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                def answer(call):
                    method = call['method']
                    if method == 'eth_sendRawTransaction':
                        submitted.append(call['params'][0])
                        # The hash the stored transaction was signed to, as a node answers.
                        with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
                            artifacts = [json.loads(row[0]) for row in db.execute('SELECT payload FROM send_artifacts')]
                        artifact = next(a for a in artifacts if a['submission'] and a['submission']['payload'] == call['params'][0])
                        return {'jsonrpc':'2.0', 'id':call['id'], 'result':artifact['view']['transaction_hash']}
                    values = {'eth_chainId': '0x1', 'eth_getBalance': '0x8ac7230489e80000', 'eth_estimateGas': '0x5208',
                              'eth_getCode': '0x', 'eth_getTransactionCount': '0x7',
                              'eth_feeHistory': {'baseFeePerGas':['0x3b9aca00'], 'reward':[['0x77359400']]}}
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
                return p
            server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
            worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
            try:
                endpoint=f'http://127.0.0.1:{server.server_port}'
                run('endpoints','--chain','ethereum','--api','evm-json-rpc','--capabilities','balance,fee,broadcast,verification,token-balance','--add',endpoint)

                password='fixture-wallet-password'
                run('wallet','import','--chain','ethereum','--name','Sealed',env={
                    'SPECTRA_PASSWORD':password,
                    'SPECTRA_SEED':'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'})
                # The holding a balance refresh would have stored.
                with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
                    wid,payload=db.execute('SELECT id,payload FROM wallets').fetchone()
                    wallet=json.loads(payload)
                    wallet['holdings']=[dict(name='Ethereum',symbol='ETH',coingeckoId='ethereum',chainId='ethereum',
                        tokenStandard='Native',contractAddress=None,amount='10')]
                    db.execute('UPDATE wallets SET payload=? WHERE id=?',(json.dumps(wallet),wid))
                args=('send','owned-broadcast','--wallet','Sealed','--holding','ethereum:native','--amount','1',
                      '--destination','0x'+'22'*20)
                refused=run(*args,success=False,env={'SPECTRA_PASSWORD':password})
                assert refused.returncode==3 and '--yes' in json.loads(refused.stdout)['error'],refused
                assert not submitted,submitted
                sent=json.loads(run(*args,'--yes',env={'SPECTRA_PASSWORD':password}).stdout)
                assert sent['transactionHash'].startswith('0x') and len(sent['transactionHash']) == 66,sent
                assert len(submitted) == 1, submitted
            finally:
                server.shutdown();server.server_close();worker.join()


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
