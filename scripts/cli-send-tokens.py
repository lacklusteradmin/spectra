#!/usr/bin/env python3
"""Token preview/build/sign survives restart; local nodes, no broadcasts.
SDK byte-level proof lives in token-send-vectors.json; this proves core routing.
Solana Token-2022 transfers state their fee and carry their hook's accounts,
and every rule the network would refuse them for is refused before building.
"""
import base64
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
import urllib.parse

BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
TOKEN_BALANCE = 223456789
RECEIVER = '22' * 32
ASSET = '44' * 32
# The 0x01 * 32 key's TON accounts, by wallet version.
OWNER_TON = {'w5': '0:9d1e1843624c4d175a695a8c2de8a5a61f03b93336e8caa4e164bf6cbbab205e',
             'v4R2': '0:efaff4bac220f88b2e98eb1d9cffcca3bfe3b66ece31a7d6c5890d30dfd7afa5'}


T22 = json.loads((pathlib.Path(__file__).resolve().parents[1] / 'core/tests/fixtures/solana-token-2022.json').read_text())
TOKEN_2022 = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb'
B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


def b58(data: bytes) -> str:
    number, text = int.from_bytes(data, 'big'), ''
    while number:
        number, digit = divmod(number, 58)
        text = B58[digit] + text
    return '1' * (len(data) - len(data.lstrip(b'\0'))) + text


class SolanaNode(http.server.BaseHTTPRequestHandler):
    """A Solana node holding one Token-2022 mint, the owner's account of it
    and, when set, the recipient's and a transfer hook's validation account."""
    state = {}
    submitted = []

    def log_message(self, *_):
        pass

    def do_POST(self):
        call = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method, params, state = call['method'], call.get('params') or [], self.state
        def account(holder, extensions=(), status='initialized'):
            return {'owner': TOKEN_2022, 'data': {'parsed': {'type': 'account', 'info': {
                'mint': T22['mint'], 'owner': T22[holder], 'state': status, 'extensions': list(extensions),
                'tokenAmount': {'amount': '5000000', 'decimals': 6}}}}}
        if method == 'getGenesisHash':
            result = '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d'
        elif method == 'getAccountInfo':
            address, raw = params[0], params[1]['encoding'] == 'base64'
            value = None
            if address == T22['mint'] and not raw:
                value = {'owner': TOKEN_2022, 'data': {'parsed': {'type': 'mint', 'info': {
                    'isInitialized': True, 'decimals': 6, 'extensions': state['mint_extensions']}}}}
            elif address == T22['source']:
                value = ({'owner': TOKEN_2022, 'data': [T22['source_data'], 'base64']} if raw
                         else account('owner'))
            elif address == T22['destination'] and not raw and state.get('destination') is not None:
                value = account('recipient', *state['destination'])
            elif address == T22['validation_account'] and raw and state.get('validation'):
                value = {'owner': T22['hook_program'], 'data': [state['validation'], 'base64']}
            result = {'context': {'slot': 1}, 'value': value}
        elif method == 'getEpochInfo':
            result = {'epoch': 700, 'slotIndex': 1000, 'slotsInEpoch': 432000, 'absoluteSlot': 1}
        elif method == 'getLatestBlockhash':
            result = {'value': {'blockhash': b58(bytes([5] * 32)), 'lastValidBlockHeight': 100}}
        elif method == 'simulateTransaction':
            result = {'value': {'err': state.get('hook_error'), 'logs': []}}
        elif method == 'isBlockhashValid':
            result = {'value': True}
        elif method == 'sendTransaction':
            self.submitted.append(params[0])
            result = b58(base64.b64decode(params[0])[1:65])
        else:
            raise AssertionError(call)
        data = json.dumps({'jsonrpc': '2.0', 'id': call['id'], 'result': result}).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)


class TokenSendTests(unittest.TestCase):
    def test_solana_token_2022_fees_and_hooks_are_reviewed_and_refusals_build_nothing(self):
        fee = lambda bps: {'extension': 'transferFeeConfig', 'state': {'withheldAmount': 0,
            'olderTransferFee': {'epoch': 0, 'transferFeeBasisPoints': bps, 'maximumFee': 10**12},
            'newerTransferFee': {'epoch': 0, 'transferFeeBasisPoints': bps, 'maximumFee': 10**12}}}
        hook = {'extension': 'transferHook', 'state': {'authority': None, 'programId': T22['hook_program']}}
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), SolanaNode)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        SolanaNode.submitted.clear()
        try:
            with tempfile.TemporaryDirectory(prefix='spectra-token-2022-') as directory:
                def run(*args, success=True, code=0):
                    result = subprocess.run([BINARY, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60,
                                            env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl'),
                                                 'SPECTRA_PRIVATE_KEY': '01' * 32})
                    assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
                    assert success or not code or result.returncode == code, (args, result.returncode, result.stderr)
                    return json.loads(result.stdout) if success else result.stdout + result.stderr
                endpoint = f'http://127.0.0.1:{server.server_port}'
                owner = run('wallet', 'import', '--chain', 'solana', '--name', 'SOL', '--no-password',
                            '--private-key-env', 'SPECTRA_PRIVATE_KEY')['wallet']['address']
                assert owner == T22['owner'], owner
                run('endpoints', '--chain', 'solana', '--api', 'solana-json-rpc', '--capabilities',
                    'balance,fee,verification,token-balance,broadcast', '--add', endpoint)
                build = ('send', 'build', '--from', 'SOL', '--to', T22['recipient'], '--endpoint', endpoint,
                         '--contract', T22['mint'], '--decimals', '6', '--amount', '1')

                # A 0.5% fee: withheld from the amount, stated in the transfer.
                SolanaNode.state = {'mint_extensions': [fee(50)]}
                built = run(*build)['artifact']
                assert built['review']['transfer_terms'] == {'debited': '1', 'received': '0.995', 'fee': '0.005',
                                                             'hook_program': None, 'carried_native': None,
                                                             'recipient_registration': None}, built['review']
                token = json.loads(built['prepared_details'])['Solana']['token']
                assert (token['amount'], token['fee'], token['hook']) == (1000000, 5000, None), token
                sign = ('send', 'sign', built['id'], '--review-digest', built['review_digest'], '--endpoint', endpoint)
                # A fee raised after review is a different transfer.
                SolanaNode.state['mint_extensions'] = [fee(100)]
                assert 'changed' in run(*sign, success=False)
                SolanaNode.state['mint_extensions'] = [fee(50)]
                signed = run(*sign)['artifact']
                raw = base64.b64decode(signed['signed_payload'])
                assert raw[65:].hex() == built['signing_payload_hex'], raw.hex()
                run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')
                assert SolanaNode.submitted == [signed['signed_payload']], SolanaNode.submitted

                # A hook: its program and resolved accounts follow the transfer's own four.
                SolanaNode.state = {'mint_extensions': [hook], 'validation': T22['validation_data']}
                hooked = run(*build)['artifact']
                assert hooked['review']['transfer_terms'] == {'debited': '1', 'received': '1', 'fee': '0',
                                                              'hook_program': T22['hook_program'], 'carried_native': None,
                                                              'recipient_registration': None}, hooked['review']
                resolved = json.loads(hooked['prepared_details'])['Solana']['token']['hook']
                expected = T22['cases']['hook'][1]['keys'][4:-2]
                assert [(a['address'], a['writable']) for a in resolved['accounts']] == [
                    (k['pubkey'], k['writable']) for k in expected], resolved
                assert resolved['validation_account'] == T22['validation_account'], resolved

                # Each refusal names its reason and builds nothing.
                before = len(run('send', 'list')['artifacts'])
                signing_hook = bytearray(base64.b64decode(T22['validation_data']))
                signing_hook[8 + 4 + 4 + 33] = 1
                for state, words in [
                    ({'mint_extensions': [{'extension': 'nonTransferable'}]}, 'cannot be transferred'),
                    ({'mint_extensions': [{'extension': 'pausableConfig', 'state': {'authority': None, 'paused': True}}]}, 'paused'),
                    ({'mint_extensions': [{'extension': 'scaledUiAmountConfig', 'state': {}}]}, 'scaled'),
                    ({'mint_extensions': [{'extension': 'somethingNew', 'state': {}}]}, 'somethingNew'),
                    ({'mint_extensions': [], 'destination': [[{'extension': 'memoTransfer', 'state': {
                        'requireIncomingTransferMemos': True}}]]}, 'memo'),
                    ({'mint_extensions': [], 'destination': [[], 'frozen']}, 'frozen'),
                    ({'mint_extensions': [{'extension': 'defaultAccountState', 'state': {'accountState': 'frozen'}}]}, 'starts frozen'),
                    ({'mint_extensions': [hook], 'validation': base64.b64encode(signing_hook).decode()}, 'signature'),
                    ({'mint_extensions': [hook], 'validation': T22['validation_data'],
                      'hook_error': {'InstructionError': [1, {'Custom': 6000}]}}, 'refused this transfer'),
                ]:
                    SolanaNode.state = state
                    output = run(*build, success=False, code=3)
                    assert words in output, (words, output)
                assert len(run('send', 'list')['artifacts']) == before, 'a refusal builds nothing'
        finally:
            server.shutdown()
            server.server_close()
            worker.join()

    def test_token_routes_read_real_precision_review_and_sign(self):
        cases = [('sui', 'sui-json-rpc', 'Sui Coin', '0x' + ASSET + '::coins::USD'),
                 ('aptos', 'aptos-rest', 'Aptos Coin', '0x' + ASSET + '::coins::USD'),
                 ('aptos', 'aptos-rest', 'AIP-21', '0x' + ASSET),
                 ('ton', 'toncenter-v2', 'TEP-74', '0:' + ASSET, 'w5'),
                 ('ton', 'toncenter-v2', 'TEP-74', '0:' + ASSET, 'v4R2'),
                 ('ethereum-classic', 'evm-json-rpc', 'ERC-20', '0x' + '44' * 20),
                 ('ethereum-classic-mordor', 'evm-json-rpc', 'ERC-20', '0x' + '44' * 20),
                 ('hyperliquid', 'evm-json-rpc', 'ERC-20', '0x' + '44' * 20),
                 ('hyperliquid-testnet', 'evm-json-rpc', 'ERC-20', '0x' + '44' * 20)]
        for chain, api, standard, contract, *version in cases:
            if os.environ.get('SPECTRA_TOKEN_CASE') and chain != os.environ['SPECTRA_TOKEN_CASE']: continue
            # A TON key signs as the wallet version it was imported under.
            ton_wallet = version[0] if version else None
            with self.subTest(chain=chain, contract=contract, version=ton_wallet), tempfile.TemporaryDirectory(prefix='spectra-token-send-') as directory:
                live = dict(decimals=6, version=8, seqno=7, wrong_owner=False, native_balance=10000000000, token_balance=TOKEN_BALANCE)
                calls = []
                chain_ids = {'ethereum-classic':61, 'ethereum-classic-mordor':63, 'hyperliquid':999, 'hyperliquid-testnet':998}
                class Handler(http.server.BaseHTTPRequestHandler):
                    def log_message(self, *_): pass
                    def reply(self, value):
                        data = json.dumps(value).encode(); self.send_response(200)
                        self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data)
                    def do_GET(self):
                        path = urllib.parse.urlsplit(self.path).path
                        query = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
                        calls.append(path)
                        if path == '/': self.reply({'chain_id':1,'ledger_version':'1'})
                        elif path == '/estimate_gas_price': self.reply({'gas_estimate':100})
                        elif path.startswith('/accounts/'): self.reply({'sequence_number':str(live['seqno'])})
                        elif path == '/getAddressBalance': self.reply({'ok':True,'result':str(live['native_balance'])})
                        elif path == '/getMasterchainInfo': self.reply({'ok':True,'result':{'init':{'workchain':-1,'seqno':0,'root_hash':'F6OpKZKqvqeFp6CQmFomXNMfMj2EnaUSOXN+Mh+wVWk=','file_hash':'XplPz01CXAps5qeSWUtxcyBfdAo5zVb1N979KLSKD24='}}})
                        elif path == '/getAddressInformation': self.reply({'ok':True,'result':{'state':'active'}})
                        elif path == '/v3/jetton/masters': self.reply({'jetton_masters':[{'jetton_content':{'decimals':str(live['decimals'])}}]})
                        elif path == '/v3/masterchainInfo': self.reply({'first':{'workchain':-1,'shard':'8000000000000000','seqno':1,'global_id':-3 if live.get('wrong_v3_network') else -239,'root_hash':'8GYhhrigd8CwZGrRT59iulLDcgiTYuvOAzFJxugc0Ts=','file_hash':'V+XzykEwun4yePZhAEPZk77RbMfMOgS/S4GiJkSKY6s='}})
                        elif path == '/v3/traces':
                            import base64
                            assert query['msg_hash'] == [base64.b64decode(live['external_hash']).hex()], query
                            self.reply(live['trace_response'])
                        elif path == '/v3/jetton/wallets':
                            owner = query.get('owner_address', [OWNER_TON[ton_wallet]])[0]
                            if live['wrong_owner']: owner = '0:' + '55' * 32
                            self.reply({'jetton_wallets':[{'address':'0:'+'33'*32,'owner':owner,'jetton':contract,'balance':str(live['token_balance'])}], 'metadata':{contract:{'token_info':[{'type':'jetton_masters','extra':{'decimals':str(live['decimals'])}}]}}})
                        else: raise AssertionError(self.path)
                    def do_POST(self):
                        value = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                        path = urllib.parse.urlsplit(self.path).path
                        if path == '/view':
                            fn = value['function']; calls.append(fn)
                            if fn.endswith('::decimals'): self.reply([live['decimals']])
                            else:
                                native = value.get('type_arguments') == ['0x1::aptos_coin::AptosCoin']
                                self.reply([str(live['native_balance'] if native else live['token_balance'])])
                            return
                        if path == '/runGetMethod': self.reply({'ok':True,'result':{'exit_code':0,'stack':[['num',hex(live['seqno'])]]}}); return
                        if path == '/sendBocReturnHash': calls.append(path); self.reply({'ok':True,'result':{'hash':live['external_hash']}}); return
                        def answer(call):
                            method = call['method']; params = call.get('params', []); calls.append(method)
                            if method == 'sui_getChainIdentifier': result = '35834a8a'
                            elif method == 'sui_getCheckpoint': result = {'sequenceNumber':'0','digest':'4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S'}
                            elif method == 'suix_getCoinMetadata': result = {'decimals':live['decimals']}
                            elif method == 'suix_getReferenceGasPrice': result = '1000'
                            elif method == 'suix_getBalance': result = {'totalBalance':str(live['native_balance'] if params[1] == '0x2::sui::SUI' else live['token_balance'])}
                            elif method == 'suix_getCoins':
                                native = params[1] == '0x2::sui::SUI'
                                result = {'data':[{'coinObjectId':'0x'+('33' if native else '44')*32,'version':str(7 if native else live['version']),'digest':'1'*32,'balance':str(live['native_balance'] if native else live['token_balance'])}], 'hasNextPage':False,'nextCursor':None}
                            elif method == 'eth_call':
                                data = params[0]['data']
                                if data.startswith('0x313ce567'): result = hex(live['decimals'])
                                elif data.startswith('0x95d89b41'): result = '0x' + ('TEST'.encode().hex()).ljust(64,'0')
                                elif data.startswith('0x01ffc9a7'): result = '0x' + '0'*64
                                else: result = hex(live['token_balance'])
                            else:
                                values = {'eth_chainId':hex(chain_ids.get(chain,1)), 'eth_getBalance':hex(live['native_balance']*10**9), 'eth_estimateGas':'0x186a0','eth_getCode':'0x6000', 'eth_getTransactionCount':hex(live['seqno']), 'eth_blockNumber':'0x123', 'eth_gasPrice':'0x3b9aca00','eth_feeHistory':{'baseFeePerGas':['0x3b9aca00'],'reward':[['0x77359400']]}}
                                assert method in values, method
                                result = values[method]
                            return {'jsonrpc':'2.0','id':call['id'],'result':result}
                        self.reply([answer(call) for call in value] if isinstance(value,list) else answer(value))
                server = http.server.ThreadingHTTPServer(('127.0.0.1',0), Handler)
                worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
                try:
                    def run(*args, success=True):
                        result = subprocess.run([BINARY,'--data-dir',directory,'--json',*args],capture_output=True,text=True,timeout=60,env={**os.environ,'SPECTRA_LOOPBACK_ONLY':str(pathlib.Path(directory)/'network.jsonl'),'SPECTRA_PRIVATE_KEY':'01'*32,'SPECTRA_PASSWORD':'token-vector-password'})
                        assert (result.returncode == 0) == success, (args,result.stdout,result.stderr)
                        return json.loads(result.stdout)
                    endpoint = f'http://127.0.0.1:{server.server_port}'
                    run('wallet','import','--chain',chain,'--name','Token','--private-key-env','SPECTRA_PRIVATE_KEY',
                        *(('--ton-wallet',ton_wallet) if ton_wallet else ()))
                    run('endpoints','--chain',chain,'--api',api,'--capabilities','balance,fee,verification,token-balance,broadcast','--add',endpoint)
                    if chain == 'ton': run('endpoints','--chain',chain,'--api','toncenter-v3','--capabilities','verification,token-balance,token-discovery','--add',endpoint+'/v3')
                    destination = ('0:'+RECEIVER) if chain == 'ton' else ('0x'+(RECEIVER[:40] if chain in chain_ids else RECEIVER))
                    run('token','add','--chain',chain,'--standard',standard,'--symbol','TEST','--name','Test','--contract',contract,'--decimals','6')
                    with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
                        wid,payload = db.execute('SELECT id,payload FROM wallets').fetchone(); wallet = json.loads(payload)
                        wallet['holdings'] = [dict(name='Test',symbol='TEST',coingeckoId='',chainId=chain,tokenStandard=standard,contractAddress=contract,amount='223.456789')]
                        db.execute('UPDATE wallets SET payload=? WHERE id=?',(json.dumps(wallet),wid))
                    identifier = chain + ':' + standard.lower() + ':' + contract
                    preview = run('send','preview','--wallet','Token','--holding',identifier,'--amount','123.456789')['preview']
                    assert preview['details']['maxSendable'] == '223.456789', preview
                    assert preview['shortcuts'], preview
                    if chain == 'ton': assert preview['network_fee'] == '0.107', preview
                    direct = ('send','build','--from','Token','--to',destination,'--endpoint',endpoint,'--contract',contract,'--decimals','6','--amount','123.456789')
                    live['decimals'] = 7; run(*direct, success=False); live['decimals'] = 6
                    if chain == 'ton':
                        live['wrong_owner'] = True; run(*direct, success=False); live['wrong_owner'] = False
                        reads = calls.count('/v3/jetton/wallets') + calls.count('/v3/jetton/masters')
                        live['wrong_v3_network'] = True; run(*direct, success=False); live['wrong_v3_network'] = False
                        assert calls.count('/v3/jetton/wallets') + calls.count('/v3/jetton/masters') == reads, calls
                    prepared = run(*direct)['artifact']
                    details = json.loads(prepared['prepared_details'])
                    if chain == 'aptos':
                        body = details['Aptos']['body']; expected = '0x1::coin::transfer' if '::' in contract else '0x1::primary_fungible_store::transfer'
                        assert body['payload']['function'] == expected, body
                        assert body['payload']['arguments'][-1] == '123456789', body
                    elif chain == 'ton': assert details['Ton']['jetton']['master'] == contract, details
                    elif chain == 'sui': assert len(details['Sui']['objects']) == 2, details
                    sign = ('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',endpoint)
                    if chain == 'sui': live['version'] += 1
                    else: live['seqno'] += 1
                    run(*sign,success=False)
                    if chain == 'sui': live['version'] -= 1
                    else: live['seqno'] -= 1
                    live['native_balance'] = 0; run(*sign,success=False); live['native_balance'] = 10000000000
                    live['token_balance'] = 0; run(*sign,success=False); live['token_balance'] = TOKEN_BALANCE
                    if chain == 'ton':
                        live['wrong_owner'] = True; run(*sign,success=False); live['wrong_owner'] = False
                        reads = calls.count('/v3/jetton/wallets') + calls.count('/v3/jetton/masters')
                        live['wrong_v3_network'] = True; run(*sign,success=False); live['wrong_v3_network'] = False
                        assert calls.count('/v3/jetton/wallets') + calls.count('/v3/jetton/masters') == reads, calls
                        # A TON message expires 60 s after it is built, and the
                        # refusals above can outlast that on a loaded machine;
                        # the one signed is built fresh.
                        prepared = run(*direct)['artifact']
                        sign = ('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',endpoint)
                    signed = run(*sign)['artifact']
                    assert signed['signed_payload'], signed
                    assert run('send','inspect',signed['id'])['artifact'] == signed
                    if chain == 'sui': live['version'] += 1
                    else: live['seqno'] += 1
                    run('send','broadcast-signed',signed['id'],'--endpoint',endpoint,'--yes',success=False)
                    assert not any('sendRawTransaction' in call or 'executeTransaction' in call or call == '/transactions' for call in calls), calls
                    if chain == 'ton':
                        # The durable hash is the locally derived external-message
                        # hash. A real v3 trace uses a different transaction hash.
                        live['seqno'] -= 1
                        live['external_hash'] = signed['transaction_hash']
                        assert live['external_hash'], signed
                        fixture = json.loads((pathlib.Path(__file__).resolve().parents[1]/'core/tests/fixtures/ton-status-v3.json').read_text())
                        details = fixture['traces'][0]['actions'][0]['details']
                        replacements = {details['sender']:OWNER_TON[ton_wallet], details['receiver']:destination,
                            details['asset']:contract, details['sender_jetton_wallet']:'0:'+'33'*32,
                            details['receiver_jetton_wallet']:'0:'+'55'*32,
                            fixture['traces'][0]['external_hash']:live['external_hash']}
                        def replace(value):
                            if isinstance(value,str): return replacements.get(value,value)
                            if isinstance(value,list): return [replace(row) for row in value]
                            if isinstance(value,dict): return {key:replace(row) for key,row in value.items()}
                            return value
                        fixture = replace(fixture)
                        trace = fixture['traces'][0]
                        trace['actions'][0]['details']['amount'] = '123456789'
                        trace['actions'][0]['details']['query_id'] = '7'
                        trace['is_incomplete'] = True
                        live['trace_response'] = fixture
                        run('send','broadcast-signed',signed['id'],'--endpoint',endpoint,'--yes')
                        assert run('txs','--poll-chain','ton')['changes'] == []
                        trace['is_incomplete'] = False
                        root = trace['transactions'][trace['trace']['tx_hash']]
                        root['description']['aborted'] = True
                        changes = run('txs','--poll-chain','ton')['changes']
                        assert len(changes)==1 and changes[0]['newStatus']=='failed', changes
                        # Restart after loss of the final status write; the exact
                        # same external message must resolve again from its trace.
                        with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
                            row = json.loads(db.execute('SELECT payload FROM history_records WHERE id=?',(signed['id'],)).fetchone()[0])
                            row['status'] = 'pending'
                            db.execute('UPDATE history_records SET payload=? WHERE id=?',(json.dumps(row),signed['id']))
                        root['description']['aborted'] = False
                        changes = run('txs','--poll-chain','ton')['changes']
                        assert len(changes)==1 and changes[0]['newStatus']=='confirmed', changes
                        assert '/sendBocReturnHash' in calls and calls.count('/v3/traces')==3, calls
                        # TEP-74 uses a 120-bit VarUInteger, not u64. Eighteen
                        # decimals must not cap a reviewed send at 18.4 tokens.
                        live['decimals'] = 18
                        live['token_balance'] = 10**27
                        live['seqno'] = 8
                        large = run('send','build','--from','Token','--to',destination,'--endpoint',endpoint,
                            '--contract',contract,'--decimals','18','--amount','1000000.000000000123456789')['artifact']
                        assert json.loads(large['prepared_details'])['Ton']['amount']=='1000000000000000123456789', large
                        large_signed = run('send','sign',large['id'],'--review-digest',large['review_digest'],'--endpoint',endpoint)['artifact']
                        assert run('send','inspect',large['id'])['artifact']==large_signed
                        run('send','build','--from','Token','--to',destination,'--endpoint',endpoint,
                            '--contract',contract,'--decimals','18','--amount','1329227995784915872.903807060280344576',success=False)
                finally:
                    server.shutdown(); server.server_close(); worker.join()


    def test_near_and_aptos_discovery_consumes_all_pages_and_refuses_bad_metadata(self):
        for chain, api, address in [('near','nearblocks','holder.near'),('aptos','aptos-indexer','0x'+'11'*32)]:
            with self.subTest(chain=chain), tempfile.TemporaryDirectory(prefix='spectra-token-discovery-') as directory:
                live = dict(bad=False)
                seen = []
                class Handler(http.server.BaseHTTPRequestHandler):
                    def log_message(self, *_): pass
                    def reply(self, value):
                        body=json.dumps(value).encode();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
                    def do_GET(self):
                        parsed=urllib.parse.urlsplit(self.path);query=urllib.parse.parse_qs(parsed.query);seen.append(query)
                        assert parsed.path == '/accounts/holder.near/assets/fts', self.path
                        if 'next' not in query: self.reply({'data':[{'contract':'first.near','amount':'1000000','meta':{'decimals':6}}], 'meta':{'next_page':'opaque+/='}})
                        else:
                            assert query['next'] == ['opaque+/='], query
                            self.reply({'data':[{'contract':'second.near','amount':'2500000','meta':{'decimals':39 if live['bad'] else 6}}], 'meta':{'next_page':None}})
                    def do_POST(self):
                        body=json.loads(self.rfile.read(int(self.headers['Content-Length'])));seen.append(body['variables'])
                        after=body['variables']['after']
                        rows=([{'storage_id':f'{n:04}','asset_type':'0x1::aptos_coin::AptosCoin','amount':'1'} for n in range(1000)] if not after else [{'storage_id':'1000','asset_type':'0x'+ASSET,'amount':'2500000','metadata':{'decimals':39 if live['bad'] else 6}}])
                        if after: assert after == '0999', after
                        self.reply({'data':{'ledger_infos':[{'chain_id':1}],'current_fungible_asset_balances':rows}})
                server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler);worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
                try:
                    def run(*args, success=True):
                        r=subprocess.run([BINARY,'--data-dir',directory,'--json',*args],capture_output=True,text=True,timeout=60,env={**os.environ,'SPECTRA_LOOPBACK_ONLY':str(pathlib.Path(directory)/'network.jsonl')})
                        assert (r.returncode==0)==success,(args,r.stdout,r.stderr)
                        return json.loads(r.stdout)
                    run('wallet','watch','--chain',chain,'--name','Holder','--address',address)
                    run('endpoints','--chain',chain,'--api',api,'--capabilities','token-discovery','--add',f'http://127.0.0.1:{server.server_port}')
                    rows=run('token','discover','--wallet','Holder')['holdings']
                    assert len(rows)==(2 if chain=='near' else 1),rows
                    assert rows[-1]['balance']=='2.5' and rows[-1]['decimals']==6,rows
                    assert len(seen)==2,seen
                    live['bad']=True;run('token','discover','--wallet','Holder',success=False)
                finally:
                    server.shutdown();server.server_close();worker.join()


if __name__ == '__main__':
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]])
