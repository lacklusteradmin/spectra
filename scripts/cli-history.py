#!/usr/bin/env python3
"""History pagination, labels, corrupt storage and transaction status rechecks.

Run: python3 scripts/cli-history.py [path/to/spectra] [TestClass.test_name]
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
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else
                          pathlib.Path(__file__).resolve().parents[1] / 'target/debug/spectra').resolve())


class HistoryTests(unittest.TestCase):
    def test_evm_testnet_token_history_and_filtered_pages(self):
        """Testnet deployments stay local, and unknown tokens cannot end pagination."""
        address = ''
        requests = []
        test_contract = '0xcac524bca292aaade2df8a05cc58f0a65b1b3bb9'
        main_contract = '0x6c3ea9036406852006290770bedfcaba0e23a0e8'
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def do_GET(self):
                query = parse_qs(urlsplit(self.path).query)
                action, page = query['action'][0], int(query['page'][0])
                requests.append((action, page))
                assert query['address'] == [address.lower()], self.path
                if action == 'txlist':
                    rows = []
                elif action == 'tokentx':
                    # A full first page includes a mainnet-only deployment and
                    # otherwise unknown contracts. The actual test token is older.
                    contracts = ([main_contract] + ['0x' + f'{i:040x}' for i in range(1, 20)]
                                 if page == 1 else [test_contract] if page == 2 else [])
                    rows = [{'hash': '0x' + f'{page * 100 + i:064x}', 'blockNumber': '42',
                             'timeStamp': '1700000000', 'from': '0x' + '22' * 20,
                             'to': address, 'contractAddress': contract,
                             'tokenName': 'Untrusted provider name', 'tokenSymbol': 'UNTRUSTED',
                             'tokenDecimal': '18', 'value': '1250000', 'logIndex': str(i)}
                            for i, contract in enumerate(contracts)]
                elif action in ('tokennfttx', 'token1155tx'):
                    rows = []
                else:
                    raise AssertionError(self.path)
                body = json.dumps({'status': '1', 'message': 'OK', 'result': rows}).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(body)))
                self.end_headers(); self.wfile.write(body)
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-evm-token-pages-') as directory:
                env = {**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl')}
                def run(*args):
                    result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                            input='abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
                                            capture_output=True, text=True, timeout=60, env=env)
                    assert result.returncode == 0, (args, result.stdout, result.stderr)
                    return json.loads(result.stdout)
                wallet = run('wallet', 'import', '--chain', 'ethereum-sepolia', '--name', 'Sepolia',
                             '--seed-file', '-', '--no-password')['wallet']
                address = wallet['address']
                endpoint = f'http://127.0.0.1:{server.server_port}'
                run('endpoints', '--chain', 'ethereum-sepolia', '--api', 'blockscout',
                    '--capabilities', 'history,token-history', '--add', endpoint)
                saved = run('history', 'Sepolia', '--save', '--pages', '2', '--limit', '20',
                            '--endpoint', endpoint)
                assert saved['walletsFailed'] == 0 and saved['pages'] == 2, saved
                assert saved['exhausted'] and saved['added'] == 1, saved
                assert ('tokentx', 2) in requests, requests
                rows = run('txs', '--page', '--wallet', 'Sepolia')['page']['records']
                assert len(rows) == 1, rows
                row = rows[0]
                assert (row['chainId'], row['symbol'], row['assetDisplayName'], row['amount']) == (
                    'ethereum-sepolia', 'tPYUSD', 'Test PayPal USD', '1.25'), row
                assert row['deploymentId'] == f'ethereum-sepolia:erc-20:{test_contract}', row
                assert run('txs', '--record', row['id'])['record'] == row
        finally:
            server.shutdown(); server.server_close(); worker.join()

    def test_evm_testnet_native_history_labels(self):
        """Stored native history uses the wallet's concrete network after reopening."""
        addresses = {}
        requests = []
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def do_GET(self):
                url = urlsplit(self.path)
                network = url.path.split('/')[1]
                query = parse_qs(url.query)
                assert url.path == f'/{network}/api', self.path
                assert network in addresses and query['module'] == ['account'], self.path
                assert query['address'] == [addresses[network].lower()], self.path
                action = query['action'][0]
                requests.append((network, action))
                if action == 'txlist':
                    rows = [{'hash': '0x' + 'ab' * 32, 'blockNumber': '42',
                             'timeStamp': '1700000000', 'from': '0x' + '22' * 20,
                             'to': addresses[network], 'value': '1250000000000000000',
                             'gasPrice': '1000000000', 'gasUsed': '21000',
                             'isError': '0', 'txreceipt_status': '1'}]
                elif action in ('tokentx', 'tokennfttx', 'token1155tx'):
                    rows = []
                else:
                    raise AssertionError(self.path)
                body = json.dumps({'status': '1', 'message': 'OK', 'result': rows}).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(body)))
                self.end_headers(); self.wfile.write(body)
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-evm-testnet-history-') as directory:
                env = {**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl')}
                def run(*args):
                    result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                            input='abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
                                            capture_output=True, text=True, timeout=60, env=env)
                    assert result.returncode == 0, (args, result.stdout, result.stderr)
                    return json.loads(result.stdout)
                for chain, symbol, name in [('ethereum-sepolia', 'tETH', 'Test Ethereum'),
                                            ('avalanche-fuji', 'tAVAX', 'Test Avalanche')]:
                    with self.subTest(chain=chain):
                        wallet = run('wallet', 'import', '--chain', chain, '--name', chain,
                                     '--seed-file', '-', '--no-password')['wallet']
                        assert wallet['chain'] == chain, wallet
                        addresses[chain] = wallet['address']
                        endpoint = f'http://127.0.0.1:{server.server_port}/{chain}'
                        run('endpoints', '--chain', chain, '--api', 'blockscout',
                            '--capabilities', 'history,token-history', '--add', endpoint)
                        saved = run('history', chain, '--save', '--endpoint', endpoint)
                        assert saved['walletsRefreshed'] == 1 and saved['walletsFailed'] == 0, saved
                        assert saved['added'] == 1, saved
                        # Both reads start a fresh process and recover the record from SQLite.
                        rows = run('txs', '--page', '--wallet', chain)['page']['records']
                        assert len(rows) == 1, rows
                        row = rows[0]
                        assert (row['chainId'], row['symbol'], row['assetDisplayName'], row['deploymentId']) == (
                            chain, symbol, name, f'{chain}:native'), row
                        assert row['amount'] == '1.25' and row['status'] == 'confirmed', row
                        assert run('txs', '--record', row['id'])['record'] == row
                        assert (chain, 'txlist') in requests, requests
        finally:
            server.shutdown(); server.server_close(); worker.join()

    def test_hash_specific_finality_persists_failed_account_transactions(self):
        """Failed executions close after restart without reading successful history."""
        alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
        def base58(raw):
            number = int.from_bytes(raw, 'big'); encoded = ''
            while number:
                number, remainder = divmod(number, 58); encoded = alphabet[remainder] + encoded
            return '1' * (len(raw) - len(raw.lstrip(b'\0'))) + encoded
        owners = {'solana': '11111111111111111111111111111111',
                  'aptos': '0x' + '12' * 32, 'sui': '0x' + '12' * 32, 'near': 'me.near'}
        cases = ['success', 'failed', 'optimistic', 'mismatch', 'unknown']
        hashes = {chain: {case: ('0x' + f'{index:02x}' * 32 if chain == 'aptos'
                               else base58(bytes([index]) * (64 if chain == 'solana' else 32)))
                          for index, case in enumerate(cases, 1)} for chain in owners}
        reverse = {chain: {hash_value: case for case, hash_value in rows.items()}
                   for chain, rows in hashes.items()}
        requests = []; errors = []; polled = []
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def respond(self, response, status=200):
                body = json.dumps(response).encode(); self.send_response(status)
                self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
            def do_GET(self):
                try:
                    requests.append(('GET', self.path))
                    if self.path == '/aptos/': return self.respond({'chain_id': 1, 'ledger_version': '500'})
                    prefix = '/aptos/transactions/by_hash/'
                    assert self.path.startswith(prefix), self.path
                    hash_value = self.path[len(prefix):]; case = reverse['aptos'][hash_value]; polled.append(('aptos', case))
                    if case == 'unknown': return self.respond({'error_code': 'transaction_not_found'}, 404)
                    self.respond({'hash': '0x' + 'aa' * 32 if case == 'mismatch' else hash_value,
                                  'type': 'pending_transaction' if case == 'optimistic' else 'user_transaction',
                                  'version': '123', 'success': case != 'failed'})
                except Exception as error:
                    errors.append(str(error)); self.send_error(500)
            def do_POST(self):
                try:
                    request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                    chain = self.path.strip('/'); method = request['method']; params = request['params']
                    requests.append((chain, method)); rpc_error = None
                    if method == 'getGenesisHash': result = '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d'
                    elif method == 'sui_getChainIdentifier': result = '35834a8a'
                    elif method == 'sui_getCheckpoint': result = {'sequenceNumber': '0', 'digest': '4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S'}
                    elif method == 'status': result = {'chain_id': 'mainnet'}
                    elif method == 'getSignatureStatuses':
                        assert params[1] == {'searchTransactionHistory': True}, request
                        case = reverse[chain][params[0][0]]; polled.append((chain, case))
                        row = None if case == 'unknown' else {'slot': 42, 'err': {'InstructionError': [0, 'Custom']} if case in ('failed', 'optimistic') else None,
                                  'confirmationStatus': 'confirmed' if case == 'optimistic' else 'finalized'}
                        result = {'value': [row, row] if case == 'mismatch' else [row]}
                    elif method == 'sui_getTransactionBlock':
                        hash_value = params[0]; case = reverse[chain][hash_value]; polled.append((chain, case))
                        result = {'digest': 'wrong-digest' if case == 'mismatch' else hash_value,
                                  'checkpoint': None if case == 'optimistic' else '42',
                                  'effects': {'status': {'status': 'failure' if case in ('failed', 'optimistic') else 'success'}}}
                        if case == 'unknown': rpc_error = {'code': -32000, 'message': 'Transaction not found'}
                    elif method == 'tx':
                        assert params['wait_until'] == 'FINAL' and params['sender_account_id'] == owners[chain], request
                        hash_value = params['tx_hash']; case = reverse[chain][hash_value]; polled.append((chain, case))
                        result = {'transaction': {'hash': hash_value, 'signer_id': 'other.near' if case == 'mismatch' else owners[chain]},
                                  'final_execution_status': 'INCLUDED' if case == 'optimistic' else 'FINAL',
                                  'status': {'Failure': {'ActionError': {}}} if case in ('failed', 'optimistic') else {'SuccessValue': ''}}
                        if case == 'unknown': rpc_error = {'code': -32000, 'cause': {'name': 'UNKNOWN_TRANSACTION'}}
                    else: raise AssertionError(('history must not serve as a status feed', request))
                    self.respond({'jsonrpc': '2.0', 'id': request['id'], 'error': rpc_error} if rpc_error else
                                 {'jsonrpc': '2.0', 'id': request['id'], 'result': result})
                except Exception as error:
                    errors.append(str(error)); self.send_error(500)
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        try:
            for chain, api, symbol in [('solana', 'solana-json-rpc', 'SOL'), ('aptos', 'aptos-rest', 'APT'),
                                       ('sui', 'sui-json-rpc', 'SUI'), ('near', 'near-json-rpc', 'NEAR')]:
                with self.subTest(chain=chain), tempfile.TemporaryDirectory(prefix='spectra-hash-finality-') as directory:
                    env = {**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl')}
                    def run(*args):
                        result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                                capture_output=True, text=True, timeout=60, env=env)
                        assert result.returncode == 0, (args, result.stdout, result.stderr, errors)
                        return json.loads(result.stdout)
                    wallet = run('wallet', 'watch', '--chain', chain, '--name', chain, '--address', owners[chain])['wallet']
                    run('endpoints', '--chain', chain, '--api', api, '--capabilities', 'verification',
                        '--add', f'http://127.0.0.1:{server.server_port}/{chain}')
                    with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as database:
                        for case in cases:
                            row = dict(id=case, walletId=wallet['id'], walletName=chain, kind='send', status='pending',
                                       chainId=chain, symbol=symbol, assetDisplayName=symbol, deploymentId=f'{chain}:native',
                                       amount='1', address=owners[chain], sourceAddress=owners[chain],
                                       transactionHash=hashes[chain][case], createdAtUnix=1700000000)
                            database.execute('INSERT INTO history_records (id,wallet_id,chain_id,tx_hash,created_at,payload) VALUES (?,?,?,?,?,?)',
                                             (case, wallet['id'], chain, row['transactionHash'], row['createdAtUnix'], json.dumps(row)))
                    changes = run('txs', '--poll-chain', chain)['changes']
                    assert len(changes) == 2, (chain, changes, errors, requests)
                    for case in cases:
                        row = run('txs', '--record', case)['record']
                        assert row['status'] == {'success': 'confirmed', 'failed': 'failed'}.get(case, 'pending'), (chain, case, row)
                    count = len(polled)
                    assert run('txs', '--poll-chain', chain)['changes'] == []
                    assert set(polled[count:]) <= {(chain, case) for case in ['optimistic', 'mismatch', 'unknown']}, polled[count:]
            assert not errors, errors
        finally:
            server.shutdown(); server.server_close(); worker.join()

    def test_near_complete_origins_survive_provider_page_boundaries(self):
        owner='me.near';origin='H'*44;token='token.near'
        requests=[]
        def receipt(identity, sender,receiver,amount,children):
            return {'receipt_id':identity,'predecessor_account_id':sender,'receiver_account_id':receiver,'actions':[{'action':'TRANSFER','args':{'deposit':amount}}],'outcome':{'status':True},'receipts':children,'block':{'block_timestamp':'1700000000000000000'}}
        tree=receipt('root','other.near',owner,'10000000000000000000000000',[receipt('child',owner,'contract.near','3000000000000000000000000',[])])
        def ft(index,amount):
            return {'affected_account_id':owner,'involved_account_id':'other.near','contract_account_id':token,'delta_amount':amount,'cause':'TRANSFER','receipt_id':f'ft-{index}','event_index':index,'event_type':1,'block_timestamp':'1700000000000000000','meta':{'decimals':6},'transaction_hash':origin}
        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*args):pass
            def do_GET(self):
                url=urlsplit(self.path);query=parse_qs(url.query);requests.append(self.path)
                if url.path==f'/v3/accounts/{owner}/receipts':
                    data=[{'transaction_hash':origin}]* (1 if query.get('next') else 50)
                    result={'data':data,'meta':{'next_page':None if query.get('next') else 'native-next'}}
                elif url.path==f'/v3/accounts/{owner}/ft-txns':
                    result={'data':[ft(51,'123')],'meta':{'next_page':None}} if query.get('next') else {'data':[ft(i,'1') for i in range(50)],'meta':{'next_page':'ft-next'}}
                elif url.path==f'/v3/txns/{origin}/receipts':result={'data':tree}
                elif url.path==f'/v3/txns/{origin}/fts':result={'data':[ft(0,'9000000'),ft(1,'-2000000')]}
                else:raise AssertionError(self.path)
                body=json.dumps(result).encode();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        server=ThreadingHTTPServer(('127.0.0.1',0),Handler);worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-near-origins-') as directory:
                env={**os.environ,'SPECTRA_LOOPBACK_ONLY':str(pathlib.Path(directory)/'network.jsonl')}
                def run(*args):
                    result=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True,timeout=60,env=env)
                    assert result.returncode==0,(args,result.stdout,result.stderr)
                    return json.loads(result.stdout)
                run('wallet','watch','--chain','near','--address',owner,'--name','NEAR')
                base=f'http://127.0.0.1:{server.server_port}'
                run('endpoints','--chain','near','--api','nearblocks','--capabilities','history,token-history,token-discovery','--add',base+'/v3')
                first=run('history','NEAR','--save','--endpoint',base+'/rpc')
                assert first['added']==2 and not first['exhausted'] and first['walletsFailed']==0,first
                rows=run('txs','--page','--wallet','NEAR')['page']['records']
                assert len(rows)==2 and {row['amount'] for row in rows}=={'7'},rows
                second=run('history','NEAR','--save','--load-more','--endpoint',base+'/rpc')
                assert second['exhausted'] and second['added']==0 and second['updated']==0 and second['walletsFailed']==0,second
                assert len(run('txs','--page','--wallet','NEAR')['page']['records'])==2
                assert sum(path.endswith('/receipts') and '/txns/' in path for path in requests)==2,requests
                assert sum(path.endswith('/fts') and '/txns/' in path for path in requests)==2,requests
        finally:server.shutdown();server.server_close();worker.join()

    def test_aptos_incoming_orderless_and_tokens_are_indexed(self):
        versions=[]
        owner='0x'+'12'.rjust(64,'0')
        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*args):pass
            def respond(self,result):
                body=json.dumps(result).encode();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
            def do_POST(self):
                request=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                assert self.path=='/graphql',self.path
                assert request['variables']['owner']==owner,request
                before=request['variables'].get('before');versions.append(before)
                batch=list(range(100,50,-1)) if before is None else [50]
                activity={'amount':'1250000','asset_type':'0xbeef','type':'0x1::fungible_asset::Deposit','is_gas_fee':False,'is_transaction_success':True,'metadata':{'decimals':6}}
                gas={'amount':'100','asset_type':'0x1::aptos_coin::AptosCoin','type':'0x1::coin::WithdrawEvent','is_gas_fee':True,'is_transaction_success':True}
                self.respond({'data':{'ledger_infos':[{'chain_id':1}],'account_transactions':[{'transaction_version':str(v),'fungible_asset_activities':[activity,gas] if v==50 else []} for v in batch]}})
            def do_GET(self):
                if self.path=='/v1/':result={'chain_id':1,'ledger_version':'101'}
                else:
                    assert self.path.startswith('/v1/transactions/by_version/'),self.path
                    version=self.path.rsplit('/',1)[1]
                    result={'version':version,'hash':'0x'+f'{int(version):064x}','timestamp':'1700000000000000','success':True,'sender':'0x99','replay_protection_nonce':'123'}
                self.respond(result)
        server=ThreadingHTTPServer(('127.0.0.1',0),Handler);worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-aptos-indexed-') as directory:
                env={**os.environ,'SPECTRA_LOOPBACK_ONLY':str(pathlib.Path(directory)/'network.jsonl')}
                def run(*args):
                    result=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True,timeout=60,env=env)
                    assert result.returncode==0,(args,result.stdout,result.stderr)
                    return json.loads(result.stdout)
                run('wallet','watch','--chain','aptos','--address',owner,'--name','APT')
                base=f'http://127.0.0.1:{server.server_port}'
                run('endpoints','--chain','aptos','--api','aptos-indexer','--capabilities','history,token-history,token-discovery','--add',base+'/graphql')
                first=run('history','APT','--save','--endpoint',base+'/v1')
                assert first['added']==0 and not first['exhausted'] and first['walletsFailed']==0,first
                second=run('history','APT','--save','--load-more','--endpoint',base+'/v1')
                assert second['added']==1 and second['exhausted'] and second['walletsFailed']==0,second
                assert versions==[None,'51'],versions
                rows=run('txs','--page','--wallet','APT')['page']['records']
                assert len(rows)==1 and rows[0]['amount']=='1.25' and rows[0]['kind']=='receive',rows
                assert rows[0]['symbol']=='0x'+ 'beef'.rjust(64,'0') or rows[0]['symbol']=='0xbeef',rows
                assert rows[0]['deploymentId']=='aptos:aip-21:0xbeef',rows
        finally:server.shutdown();server.server_close();worker.join()

    def test_provider_cursor_survives_restart_and_filtered_empty_page(self):
        owner = '11111111111111111111111111111111'
        requests = []
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def do_POST(self):
                request=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if request['method']=='getSignaturesForAddress':
                    before=request['params'][1].get('before')
                    requests.append(before)
                    result=([{'signature':f'{i:088d}'} for i in range(50)] if before is None else [{'signature':'A'*88}])
                elif request['method']=='getTransaction':
                    signature=request['params'][0]
                    result={'slot':42,'blockTime':1700000000,'transaction':{'message':{'accountKeys':[owner]}},'meta':{'fee':0,'preBalances':[100],'postBalances':[200 if signature=='A'*88 else 100]}}
                else: raise AssertionError(request)
                body=json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}).encode()
                self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
        worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-provider-pages-') as directory:
                env={**os.environ,'SPECTRA_LOOPBACK_ONLY':str(pathlib.Path(directory)/'network.jsonl')}
                def run(*args):
                    result=subprocess.run([binary,'--data-dir',directory,'--json',*args],capture_output=True,text=True,timeout=60,env=env)
                    assert result.returncode==0,(args,result.stdout,result.stderr)
                    return json.loads(result.stdout)
                run('wallet','watch','--chain','solana','--address',owner,'--name','Paged')
                endpoint=f'http://127.0.0.1:{server.server_port}'
                first=run('history','Paged','--save','--endpoint',endpoint)
                assert first['added']==0 and not first['exhausted'] and first['walletsFailed']==0,first
                with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
                    cursor=db.execute('SELECT cursor,exhausted FROM history_pagination').fetchone()
                    assert cursor==(f'{49:088d}',0),cursor
                second=run('history','Paged','--save','--load-more','--endpoint',endpoint)
                assert second['added']==1 and second['exhausted'] and second['walletsFailed']==0,second
                assert requests==[None,f'{49:088d}'],requests
                rows=run('txs','--page','--wallet','Paged')['page']['records']
                assert len(rows)==1 and rows[0]['amount']=='0.0000001',rows
                # A successful head refresh retains the older, already stored record.
                refreshed=run('history','Paged','--save','--endpoint',endpoint)
                assert not refreshed['exhausted'] and refreshed['added']==0,refreshed
                assert len(run('txs','--page','--wallet','Paged')['page']['records'])==1
        finally:
            server.shutdown();server.server_close();worker.join()

    def test_solana_token_history_labels(self):
        """RPC mint addresses resolve to tickers before storage and survive reopening."""
        owner = '11111111111111111111111111111111'
        mints = ['EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
                 'Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB',
                 'So11111111111111111111111111111111111111113']
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if request['method'] == 'getSignaturesForAddress':
                    result = [{'signature': 'A' * 88}]
                elif request['method'] == 'getTransaction':
                    result = {'slot': 42, 'blockTime': 1700000000,
                              'transaction': {'message': {'accountKeys': [owner]}},
                              'meta': {'fee': 0, 'preBalances': [100], 'postBalances': [100],
                                       'preTokenBalances': [], 'postTokenBalances': [
                                           {'owner': owner, 'mint': mint, 'accountIndex': i,
                                            'uiTokenAmount': {'amount': '42500000', 'decimals': 6}}
                                           for i, mint in enumerate(mints)]}}
                else:
                    raise AssertionError(request)
                body = json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(body)))
                self.end_headers(); self.wfile.write(body)
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-spl-labels-') as directory:
                def run(*args):
                    result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                                            capture_output=True, text=True, timeout=60)
                    assert result.returncode == 0, (args, result.stdout, result.stderr)
                    return json.loads(result.stdout)
                run('wallet', 'watch', '--chain', 'solana', '--address', owner, '--name', 'SPL')
                endpoint = f'http://127.0.0.1:{server.server_port}'
                expected = {'USDC', 'USDT', mints[2]}
                live = run('history', 'SPL', '--endpoint', endpoint)['transactions']
                assert {row['symbol'] for row in live} == expected, live
                run('history', 'SPL', '--save', '--endpoint', endpoint)
                rows = run('txs', '--page', '--wallet', 'SPL')['page']['records']
                assert {row['symbol'] for row in rows} == expected, rows
                assert all(row['amount'] == '42.5' for row in rows), rows
                usdc = next(row for row in rows if row['symbol'] == 'USDC')
                assert usdc['assetDisplayName'] == 'USD Coin', usdc
                assert usdc['deploymentId'] == f'solana:spl:{mints[0]}', usdc
                # Refresh must repair an address label without duplicating the transfer.
                with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as db:
                    db.execute("UPDATE history_records SET payload = json_set(payload, '$.symbol', ?, '$.assetDisplayName', ?) WHERE id = ?",
                               (mints[0], mints[0], usdc['id']))
                run('history', 'SPL', '--save', '--endpoint', endpoint)
                refreshed = run('txs', '--page', '--wallet', 'SPL')['page']['records']
                assert len(refreshed) == 3, refreshed
                assert {row['symbol'] for row in refreshed} == expected, refreshed
                searched = run('txs', '--page', '--search', 'USDC')['page']['records']
                assert len(searched) == 1 and searched[0]['id'] == usdc['id'], searched
                assert run('txs', '--record', usdc['id'])['record']['symbol'] == 'USDC'
        finally:
            server.shutdown(); server.server_close(); worker.join()

    def test_blockbook_history_is_shared_across_networks(self):
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def do_GET(self):
                assert self.path.startswith('/api/v2/address/'), self.path
                address = self.path.split('/')[4].split('?')[0]
                # The amount is the address's own net: what the outputs pay it.
                body = json.dumps({'transactions': [{'txid': 'ab' * 32, 'blockHeight': 42,
                    'blockTime': 1700000000, 'value': '223456789', 'fees': '1000', 'vin': [],
                    'vout': [{'addresses': [address], 'value': '123456789'},
                             {'addresses': ['someone-else'], 'value': '100000000'}]}]}).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(body)))
                self.end_headers(); self.wfile.write(body)
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-api-history-') as directory:
                def run(*args):
                    p = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                        input='abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
                        capture_output=True, text=True, timeout=60)
                    assert p.returncode == 0, (p.stdout, p.stderr)
                    return json.loads(p.stdout)
                for chain in ('dash', 'zcash'):
                    run('wallet', 'import', '--chain', chain, '--name', chain, '--seed-file', '-')
                    history = run('history', chain, '--endpoint', f'http://127.0.0.1:{server.server_port}')
                    assert history['count'] == 1, history
                    tx = history['transactions'][0]
                    assert tx['hash'] == 'ab' * 32 and tx['amount'] == '1.23456789', tx
        finally:
            server.shutdown(); server.server_close(); worker.join()

    def test_stored_pages(self):
        """History pages deduplicate, sort, search Unicode and keep distinct identities."""
        with tempfile.TemporaryDirectory(prefix='spectra-history-pages-') as directory:
            def run(*args):
                result = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
                assert result.returncode == 0, (args, result.stdout, result.stderr)
                return json.loads(result.stdout)
            run('wallet', 'watch', '--chain', 'ethereum', '--address', '0x'+'11'*20, '--name', 'Boundary')
            dbpath = pathlib.Path(directory)/'spectra.sqlite'
            with sqlite3.connect(dbpath) as db:
                wid = db.execute('SELECT id FROM wallets').fetchone()[0]
                for i in range(55):
                    row = dict(id=f'tx-{i:03}', walletId=wid, walletName='Éther 测试', kind='receive', status='confirmed', chainId='ethereum', symbol='ETH', assetDisplayName='Ether', deploymentId='ethereum:native', amount='1', address='0x'+'22'*20, transactionHash=f'0x{i:064x}', createdAtUnix=i+1)
                    db.execute('INSERT INTO history_records VALUES (?,?,?,?,?,?)', (row['id'],wid,'ethereum',row['transactionHash'],i+1,json.dumps(row)))
                # A duplicate provider record must not consume a page slot or hide the confirmed row.
                row.update(id='duplicate', kind='send', status='pending')
                db.execute('INSERT INTO history_records VALUES (?,?,?,?,?,?)', (row['id'],wid,'ethereum',row['transactionHash'],799,json.dumps(row)))
            pages = [run('txs', '--page')['page']]
            while pages[-1]['hasMore']:
                pages.append(run('txs', '--page', '--cursor', pages[-1]['nextCursor'])['page'])
            ids = [r['id'] for page in pages for r in page['records']]
            assert len(ids)==55 and len(set(ids))==55 and 'duplicate' not in ids, ids
            assert [p['hasMore'] for p in pages] == [True,True,False], pages
            assert run('txs','--page','--filter','pending')['page']['records']==[]
            assert len(run('txs','--page','--search','éTHER 测试')['page']['records']) == 20
            assert run('txs','--page','--search','no-such-address')['page']['records']==[]
            assert run('txs','--page','--oldest-first','--limit','1')['page']['records'][0]['id']=='tx-000'
            # Past core's page cap a limit is cut to it, not refused.
            assert len(run('txs','--page','--limit','100000')['page']['records']) == 55
            summary = run('txs','--summary')['summary']
            assert summary['totalCount'] == 55
            assert len(summary['recentAndPending']) == 50
            assert 'duplicate' not in {r['id'] for r in summary['recentAndPending']}
            assert summary['replaceable'] == []
            # Times start at 1: 0 is a transaction without one.
            assert summary['earliest'][0]['earliestCreatedAtUnix'] == 1
            assert 'tx-000' not in {r['id'] for r in summary['recentAndPending']}
            old_record = run('txs','--record','tx-000')['record']
            assert old_record['id'] == 'tx-000' and old_record['status'] == 'confirmed', old_record
            assert run('txs','--record','missing')['record'] is None
            run('wallet','watch','--chain','solana','--address','11111111111111111111111111111111','--name','IdentityCases')
            with sqlite3.connect(dbpath) as db:
                solana_id = db.execute("SELECT id FROM wallets WHERE name='IdentityCases'").fetchone()[0]
                for identity, txhash, deployment in [('case-upper','A'*88,'solana:native'), ('case-lower','a'*88,'solana:native'), ('unknown-one','B'*88,None), ('unknown-two','B'*88,None)]:
                    record = dict(id=identity, walletId=solana_id, walletName='IdentityCases', kind='receive', status='confirmed', chainId='solana', symbol='SOL', assetDisplayName='Solana', deploymentId=deployment, amount='1', address='11111111111111111111111111111111', transactionHash=txhash, createdAtUnix=1)
                    db.execute('INSERT INTO history_records VALUES (?,?,?,?,?,?)', (identity,solana_id,'solana',txhash.lower(),1,json.dumps(record)))
            distinct = run('txs','--page','--wallet','IdentityCases')['page']['records']
            assert {row['id'] for row in distinct} == {'case-upper','case-lower','unknown-one','unknown-two'}, distinct

    def test_cursor_changes_and_ties(self):
        """Cursor survives anchor deletion and inserts; ties work in both directions."""
        with tempfile.TemporaryDirectory(prefix='spectra-history-cursor-') as directory:
            def run(*args, ok=True):
                result = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
                assert (result.returncode == 0) == ok, (args, result.stdout, result.stderr)
                return json.loads(result.stdout) if ok else None
            run('wallet', 'watch', '--chain', 'ethereum', '--address', '0x'+'11'*20, '--name', 'Cursor')
            dbpath = pathlib.Path(directory)/'spectra.sqlite'
            with sqlite3.connect(dbpath) as db:
                wid = db.execute('SELECT id FROM wallets').fetchone()[0]
                def insert(identity, timestamp):
                    row = dict(id=identity, walletId=wid, walletName='Cursor', kind='receive', status='confirmed', chainId='ethereum', symbol='ETH', assetDisplayName='Ether', deploymentId='ethereum:native', amount='1', address='0x'+'22'*20, transactionHash=identity, createdAtUnix=timestamp)
                    db.execute('INSERT INTO history_records VALUES (?,?,?,?,?,?)', (identity,wid,'ethereum',identity,timestamp,json.dumps(row)))
                for i in range(9): insert(f'tie-{i}', i//3)
            for flags in [(), ('--oldest-first',)]:
                page = run('txs', '--page', '--limit', '2', *flags)['page']
                ids = [r['id'] for r in page['records']]
                while page['hasMore']:
                    page = run('txs', '--page', '--limit', '2', *flags, '--cursor', page['nextCursor'])['page']
                    ids += [r['id'] for r in page['records']]
                expected = list(range(9)) if flags else [6,7,8,3,4,5,0,1,2]
                assert ids == [f'tie-{i}' for i in expected], ids
                assert page['nextCursor'] is None
            first = run('txs', '--page', '--limit', '2')['page']
            with sqlite3.connect(dbpath) as db:
                db.execute("DELETE FROM history_records WHERE id = 'tie-7'")
                insert('newest', 10)
            second = run('txs', '--page', '--limit', '2', '--cursor', first['nextCursor'])['page']
            assert [r['id'] for r in second['records']] == ['tie-8', 'tie-3'], second
            assert run('txs', '--page')['page']['records'][0]['id'] == 'newest'
            for changed in [('--oldest-first',), ('--filter','pending'), ('--search','Cursor')]:
                run('txs', '--page', '--cursor', first['nextCursor'], *changed, ok=False)
            run('txs', '--page', '--cursor', 'broken', ok=False)

    def test_bitcoin_pagination(self):
        """Fetch and persist every transaction across provider pages."""
        ADDRESS = 'bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu'

        requests = []

        class Esplora(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass
            def do_GET(self):
                requests.append(self.path)
                prefix = f'/address/{ADDRESS}/txs'
                if self.path == prefix:
                    upper = 61
                elif self.path.startswith(prefix + '/chain/'):
                    upper = int(self.path.rsplit('/', 1)[1], 16) - 1
                else:
                    self.send_error(404)
                    return
                rows = [{'txid': f'{i:064x}', 'vin': [], 'vout': [{'scriptpubkey_address': ADDRESS, 'value': 100}],
                         'fee': 1, 'status': {'confirmed': True, 'block_height': i, 'block_time': 1700000000 + i}}
                        for i in list(range(upper, 0, -1))[:25]]
                data = json.dumps(rows).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        with tempfile.TemporaryDirectory(prefix='spectra-cli-history-') as directory:
            def cli(*args):
                result = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
                if result.returncode:
                    raise AssertionError((args, result.stdout, result.stderr))
                return json.loads(result.stdout)
            cli('wallet', 'watch', '--chain', 'bitcoin', '--address', ADDRESS, '--name', 'Pager')
            server = ThreadingHTTPServer(('127.0.0.1', 0), Esplora)
            worker = threading.Thread(target=server.serve_forever, daemon=True)
            worker.start()
            try:
                result = cli('history', 'Pager', '--save', '--pages', '10', '--limit', '10', '--endpoint', f'http://127.0.0.1:{server.server_port}')
                assert result['added'] == 61 and result['updated'] == 0, result
                assert result['pages'] == 7 and result['exhausted'] and result['walletsFailed'] == 0, result
                assert len(requests) == 3 and sum('/chain/' in p for p in requests) == 2, requests
                # A separate process proves all pages were persisted, not just cached in the session.
                stored = cli('txs', '--wallet', 'Pager')
                expected_hashes = {f'{i:064x}' for i in range(1, 62)}
                assert len(stored['transactions']) == 61, stored
                assert {row['hash'] for row in stored['transactions']} == expected_hashes, stored
                repeated = cli('history', 'Pager', '--save', '--pages', '10', '--limit', '10', '--endpoint', f'http://127.0.0.1:{server.server_port}')
                assert repeated['added'] == 0 and repeated['walletsFailed'] == 0, repeated
                reopened = cli('txs', '--wallet', 'Pager')['transactions']
                assert len(reopened) == 61 and {row['hash'] for row in reopened} == expected_hashes, reopened
            finally:
                server.shutdown()
                server.server_close()
                worker.join()

    def test_invalid_history_is_refused_on_write(self):
        """Identity/status corruption is rejected before it can poison any page."""
        with tempfile.TemporaryDirectory(prefix="spectra-history-check-") as directory:
            result = subprocess.run([binary, '--data-dir', directory, '--json', 'txs'],
                                    capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as db:
                for raw in ['{"id":42}', '{}', '{"id":"fault","kind":"receive","status":"unknown"}']:
                    with self.assertRaises(sqlite3.IntegrityError):
                        db.execute('INSERT INTO history_records(id,chain_id,created_at,payload) VALUES(?,?,?,?)',
                                   ('fault', 'bitcoin', 0, raw))
                self.assertEqual(db.execute('SELECT COUNT(*) FROM history_records').fetchone()[0], 0)
            for mode in ('--page', '--summary', '--replaceable'):
                result = subprocess.run([binary, '--data-dir', directory, '--json', 'txs', mode],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_source_labels(self):
        """Expose the correct source label for stored provider identities."""
        with tempfile.TemporaryDirectory(prefix='spectra-history-source-') as directory:
            def run(*args):
                p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
                assert p.returncode == 0, (args, p.stdout, p.stderr)
                return json.loads(p.stdout)
            run('txs')  # create the core schema
            expected = {
                'rpc': {'provider': 'RPC'},
                'etherscan': {'provider': 'Etherscan'},
                'rust': 'internal',
                'rust.hd': 'internal',
                'dogecoin.providers': {'chainProviders': 'dogecoin'},
                'none': None,
            }
            with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as db:
                for index, source in enumerate(expected):
                    tx_hash = f'{index:02x}' * 32
                    row = dict(id=source, walletId='wallet', walletName='Fixture', kind='receive', status='confirmed',
                               chainId='dogecoin', symbol='DOGE', assetDisplayName='Dogecoin', amount='1',
                               address='sender', transactionHash=tx_hash, createdAtUnix=1234,
                               transactionHistorySource=source)
                    db.execute('INSERT INTO history_records (id,wallet_id,chain_id,tx_hash,created_at,payload) VALUES (?,?,?,?,?,?)',
                               (source, 'wallet', 'dogecoin', tx_hash, 1234 + index, json.dumps(row)))
            by_hash = {t['hash']: t.get('historySource') for t in run('txs')['transactions']}
            for index, (source, named) in enumerate(expected.items()):
                got = by_hash[f'{index:02x}' * 32]
                assert got == named, f'{source}: expected {named!r}, got {got!r}'

    def test_action_projection_and_unix_timestamp(self):
        with tempfile.TemporaryDirectory(prefix='spectra-actions-') as directory:
            def run(*args):
                result = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60)
                assert result.returncode == 0, result.stdout + result.stderr
                return json.loads(result.stdout)
            run('wallet','watch','--chain','ethereum','--address','0x'+'11'*20)
            path = pathlib.Path(directory)/'spectra.sqlite'
            with sqlite3.connect(path) as db:
                wid = db.execute('SELECT id FROM wallets').fetchone()[0]
                for identity, chain, status, fmt, payload, txhash in [
                    ('evm', 'ethereum', 'pending', 'evm.raw_hex', '0x1234', '0x'+'ab'*32),
                    ('bad-format', 'ethereum', 'pending', 'solana.rust_json', 'anything', '0x'+'ab'*32),
                    ('confirmed', 'ethereum', 'confirmed', 'evm.raw_hex', '0x1234', '0x'+'cd'*32),
                    ('utxo', 'bitcoin', 'failed', None, None, 'ef'*32),
                    ('bad-hash', 'bitcoin', 'pending', None, None, 'invalid')]:
                    row = dict(id=identity, walletId=wid, walletName='Fixture', kind='send', status=status,
                        chainId=chain, symbol='COIN', assetDisplayName=chain, amount='1', address='recipient',
                        transactionHash=txhash, createdAtUnix=1700000000.125,
                        signedTransactionPayload=payload, signedTransactionPayloadFormat=fmt)
                    db.execute('INSERT INTO history_records(id,wallet_id,chain_id,tx_hash,created_at,payload) VALUES(?,?,?,?,?,?)',
                        (identity,wid,chain,txhash,row['createdAtUnix'],json.dumps(row)))
            for identity, recheck, rebroadcast in [('evm',False,True),('bad-format',False,False),('confirmed',False,False),('utxo',True,False),('bad-hash',False,False)]:
                output = run('txs','--record',identity)
                actions = output['actions']
                assert (actions['recheckUnavailableReason'] is None) == recheck, output
                assert (actions['rebroadcastUnavailableReason'] is None) == rebroadcast, output
                assert output['record']['createdAtUnix'] == 1700000000.125
            page = run('txs','--page')
            assert page['actions']['utxo']['recheckUnavailableReason'] is None
            assert run('txs','--summary')['summary']['earliest'][0]['earliestCreatedAtUnix'] == 1700000000.125

if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
