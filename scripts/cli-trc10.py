#!/usr/bin/env python3
"""TRC-10 account reads, history, persisted signing and local broadcast/finality.
All endpoints are throwaway loopback nodes; no chain broadcast occurs.
"""
import hashlib
import http.server
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading
import urllib.parse

BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
FIXTURE = json.loads((pathlib.Path(__file__).resolve().parents[1] / 'core/tests/fixtures/trc10-send-vectors.json').read_text())
OWNER, RECEIVER = FIXTURE['owner'], FIXTURE['receiver']
GENESIS = {'tron':'00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc',
           'tron-nile':'0000000000000000d698d4192c56cb6be724a558448e2684802de4d6cd8690dc'}
ACTIVATION_PROOF = json.loads((pathlib.Path(__file__).resolve().parents[1] /
    'docs/audits/chain-support-2026-10-04/trc10-name-activation.json').read_text())
NAME_END = {row['chain']:row['maintenance_activation_block']['block_header']['raw_data']['timestamp']
            for row in ACTIVATION_PROOF['networks']}


def base58_hex(address):
    alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
    number = 0
    for char in address:
        number = number * 58 + alphabet.index(char)
    return number.to_bytes(25, 'big')[:21].hex()


for chain in GENESIS:
    live = dict(decimals=2, balance=22345, native=10_000_000, recipient_balance=0,
                wrong_network=False, wrong_id=False, accepted=False, failed=False, calls=[], payloads=[],
                history_mode=None, legacy_duplicate=False, legacy_bad_name=False, legacy_bad_current=False)

    class Node(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def answer(self, value, code=200):
            raw = json.dumps(value).encode(); self.send_response(code)
            self.send_header('Content-Length', str(len(raw))); self.end_headers(); self.wfile.write(raw)
        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            route = urllib.parse.urlsplit(self.path).path; live['calls'].append(route)
            if route == '/wallet/getblockbynum':
                return self.answer({'blockID':GENESIS['tron-nile' if chain == 'tron' else 'tron'] if live['wrong_network'] else GENESIS[chain]})
            if route == '/wallet/getnowblock':
                return self.answer({'blockID':FIXTURE['block']['id'], 'block_header':{'raw_data':{'number':FIXTURE['block']['number']}}})
            if route == '/wallet/getassetissuebyid':
                assert body['value'] in ('1009999', '1002000', '1000001', '1000002'), body
                if body['value'] in ('1000001', '1000002'):
                    name = 'LegacyTest' if body['value'] == '1000001' else '1009999'
                    if live['legacy_bad_current']: name = 'OtherAsset'
                    return self.answer({'id':body['value'], 'name':name.encode().hex()})
                if body['value'] == '1002000':
                    return self.answer({'id':'1002000', 'name':'BitTorrent'.encode().hex(),
                                        'abbr':'BTT'.encode().hex(), 'precision':6})
                return self.answer({'id':'1002001' if live['wrong_id'] else '1009999', 'name':'LegacyTest'.encode().hex(),
                                    'abbr':'T10'.encode().hex(),'precision':live['decimals']})
            if route == '/wallet/getassetissuebyname':
                name = bytes.fromhex(body['value']).decode(); assert name in ('LegacyTest','1009999'), body
                if live['legacy_duplicate']: return self.answer({'Error':'NonUniqueObjectException: duplicate name'})
                return self.answer({'id':'1000001' if name == 'LegacyTest' else '1000002',
                    'name':('OtherAsset' if live['legacy_bad_name'] else name).encode().hex()})
            if route == '/wallet/getaccount':
                assert body['visible'] is True, body
                owner = body['address']; assert owner in (OWNER, RECEIVER), body
                return self.answer({'address':owner,'balance':live['native'] if owner == OWNER else 1,
                    'assetV2':[{'key':'1009999','value':live['balance'] if owner == OWNER else live['recipient_balance']}]})
            if route == '/wallet/getchainparameters':
                return self.answer({'chainParameter':[{'key':'getTransactionFee','value':1000},
                    {'key':'getCreateAccountFee','value':100_000},{'key':'getCreateNewAccountFeeInSystemContract','value':1_000_000}]})
            if route == '/wallet/broadcasttransaction':
                assert body['raw_data']['contract'][0]['type'] == 'TransferAssetContract', body
                assert 'fee_limit' not in body['raw_data'], body
                value = body['raw_data']['contract'][0]['parameter']['value']
                assert value == {'asset_name':'1009999'.encode().hex(), 'owner_address':base58_hex(OWNER),
                                 'to_address':base58_hex(RECEIVER),'amount':12345}, value
                assert body['txID'] == hashlib.sha256(bytes.fromhex(body['raw_data_hex'])).hexdigest(), body
                assert len(bytes.fromhex(body['signature'][0])) == 65, body
                live['payloads'].append(body); live['accepted'] = True
                return self.answer({'result':True,'txid':body['txID']})
            if route == '/walletsolidity/gettransactioninfobyid':
                if not live['accepted']: return self.answer({})
                return self.answer({'id':body['value'],'blockNumber':450,'receipt':{'net_usage':300},
                                   **({'result':'FAILED'} if live['failed'] else {})})
            return self.answer({'Error':'unexpected route'}, 400)
        def do_GET(self):
            parsed = urllib.parse.urlsplit(self.path); query = urllib.parse.parse_qs(parsed.query)
            route = parsed.path; live['calls'].append(route)
            if route == f'/v1/accounts/{OWNER}': return self.answer({'data':[{'trc20':[]}]})
            if route == f'/v1/accounts/{OWNER}/transactions/trc20': return self.answer({'data':[]})
            if route == f'/v1/accounts/{OWNER}/transactions':
                if live['history_mode']:
                    name = '1009999' if live['history_mode'] == 'numeric-name' else 'LegacyTest'
                    return self.answer({'data':[{'txID':'cc'*32, 'block_timestamp':NAME_END[chain],
                        'ret':[{'contractRet':'SUCCESS'}], 'raw_data':{'contract':[{'type':'TransferAssetContract',
                        'parameter':{'value':{'asset_name':name.encode().hex(),'amount':123,
                            'owner_address':base58_hex(RECEIVER),'to_address':base58_hex(OWNER)}}}]}}]})
                older = query.get('fingerprint') == ['older']
                return self.answer({'data':[{'txID':('aa' if older else 'bb')*32,'block_timestamp':1700000000000 if older else 1700000001000,
                    'ret':[{'contractRet':'SUCCESS'}], 'raw_data':{'contract':[{'type':'TransferAssetContract', 'parameter':{'value':{
                        'asset_name':'1009999'.encode().hex(),'amount':123 if older else 250,
                        'owner_address':base58_hex(OWNER if older else RECEIVER),
                        'to_address':base58_hex(RECEIVER if older else OWNER)}}}]}}],
                    **({} if older else {'meta':{'fingerprint':'older'}})})
            return self.answer({'Error':'unexpected route'}, 400)

    class Server(http.server.ThreadingHTTPServer):
        request_queue_size = 64

    server = Server(('127.0.0.1',0), Node)
    worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
    try:
        with tempfile.TemporaryDirectory(prefix='spectra-trc10-') as directory:
            env = {**os.environ,'SPECTRA_PRIVATE_KEY':FIXTURE['key'],'SPECTRA_PASSWORD':'trc10-vector-password',
                   'SPECTRA_LOOPBACK_ONLY':str(pathlib.Path(directory)/'network.jsonl')}
            def run(*args, success=True):
                result = subprocess.run([BINARY,'--data-dir',directory,'--json',*args], capture_output=True,text=True,timeout=60,env=env)
                assert (result.returncode == 0) == success, (args,result.stdout,result.stderr)
                return json.loads(result.stdout)
            endpoint = f'http://127.0.0.1:{server.server_port}'
            wallet = run('wallet','import','--chain',chain,'--name','Legacy','--private-key-env','SPECTRA_PRIVATE_KEY')['wallet']
            assert wallet['address'] == OWNER, wallet
            run('endpoints','--chain',chain,'--api','tron-http','--capabilities','balance,fee,verification,token-balance,broadcast','--add',endpoint)
            run('endpoints','--chain',chain,'--api','trongrid-v1','--capabilities','history,token-history,token-discovery','--add',endpoint+'/v1/accounts')
            run('token','add','--chain',chain,'--standard','TRC-10','--symbol','T10','--name','Legacy Test','--contract','1009999','--decimals','2')
            rows = run('token','discover','--wallet','Legacy')['holdings']
            token = next(row for row in rows if row['contract'] == '1009999')
            assert (token['decimals'],token['balance']) == (2,'223.45'), token
            run('refresh','--wallet','Legacy','--endpoint',endpoint)
            with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
                stored = json.loads(db.execute('SELECT payload FROM wallets').fetchone()[0])
                assert next(h for h in stored['holdings'] if h.get('contractAddress') == '1009999')['amount'] == '223.45', stored
            # The wallet also has catalogued TRC-20 assets. Their refresh may use
            # constant calls; the TRC-10 review, history and send steps must not.
            live['calls'].clear()
            holding = chain+':trc-10:1009999'
            preview = run('send','preview','--wallet','Legacy','--holding',holding,'--amount','123.45')['preview']
            assert preview['details']['maxSendable'] == '223.45' and preview['network_fee'] == '1.1', preview
            run('history','Legacy','--endpoint',endpoint)
            first = run('history','Legacy','--save','--endpoint',endpoint)
            assert first['added'] == 1 and not first['exhausted'], first
            second = run('history','Legacy','--save','--load-more','--endpoint',endpoint)
            assert second['added'] == 1 and second['exhausted'], second
            history = run('txs','--page','--wallet','Legacy')['page']['records']
            amounts = {row['transactionHash']:row['amount'] for row in history}
            assert amounts == {'aa'*32:'1.23','bb'*32:'2.5'}, history
            for mode in ('name', 'numeric-name'):
                live['history_mode'] = mode
                old = run('history','Legacy','--endpoint',endpoint)
                assert old['transactions'][0]['amount'] == '123', old
                for field in ('legacy_duplicate','legacy_bad_name','legacy_bad_current'):
                    live[field] = True; run('history','Legacy','--endpoint',endpoint,success=False); live[field] = False
            live['history_mode'] = None
            direct = ('send','build','--from','Legacy','--to',RECEIVER,'--endpoint',endpoint,
                      '--contract','1009999','--decimals','2','--amount','123.45')
            for field,value in [('wrong_network',True),('wrong_id',True),('decimals',3),('balance',0),('native',0),('recipient_balance',2**63-1)]:
                original = live[field]; live[field] = value; run(*direct,success=False); live[field] = original
            prepared = run(*direct)['artifact']
            sign = ('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',endpoint)
            live['balance'] = 0; run(*sign,success=False); live['balance'] = 22345
            signed = run(*sign)['artifact']; raw = signed['signed_payload']
            assert run('send','inspect',signed['id'])['artifact']['signed_payload'] == raw
            broadcast = ('send','broadcast-signed',signed['id'],'--endpoint',endpoint,'--yes')
            live['native'] = 0; run(*broadcast,success=False); live['native'] = 10_000_000
            accepted = run(*broadcast)['artifact']
            assert accepted['stage'] == 'Signed' and any(a['outcome'] == 'Accepted' for a in accepted['attempts']), accepted
            assert len(live['payloads']) == 1 and live['payloads'][0] == json.loads(raw), live['payloads']
            changes = run('txs','--poll-chain',chain)['changes']; assert changes and all(r['newStatus'] == 'confirmed' for r in changes), changes
            records = run('txs','--page','--wallet','Legacy')['page']['records']
            confirmed = next(r for r in records if r['transactionHash'] == accepted['transaction_hash'])
            assert confirmed['status'] == 'confirmed' and confirmed['deploymentId'] == holding and confirmed['amount'] == '123.45', confirmed
            # Reopen from a Pending record after a lost completion write, then
            # prove a committed FAILED system contract never becomes success.
            with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
                pending = dict(confirmed, status='pending')
                db.execute('UPDATE history_records SET payload=? WHERE id=?',(json.dumps(pending),confirmed['id']))
            live['failed'] = True
            failed_changes = run('txs','--poll-chain',chain)['changes']
            assert any(r['id'] == confirmed['id'] and r['newStatus'] == 'failed' for r in failed_changes), failed_changes
            failed = run('txs','--record',confirmed['id'])['record']
            assert failed['status'] == 'failed' and failed['failureReason']['kind'] == 'executionFailed', failed
            assert '/wallet/triggerconstantcontract' not in live['calls'], live['calls']
    finally:
        server.shutdown(); server.server_close(); worker.join()
print('TRC-10 mainnet/Nile metadata, discovery, refresh, legacy names, numeric-name activation, paging, preview, durable signing, broadcast and finality passed')
