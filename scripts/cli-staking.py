#!/usr/bin/env python3
"""Account-chain staking through durable CLI stages; loopback only.

One stake per network, each command a fresh process over the same data
directory: built and reopened exactly, refused at broadcast without a
submission, lost in flight, retried with the same bytes, and recovered from
the exact signed hash. Polkadot's pool stake recovers from a finalized block.
What a position, a build or a signature refuses is tested in core.
"""
import base64
import hashlib
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
import urllib.parse

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / 'target/debug/spectra').resolve())
MNEMONIC = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


def b58(raw):
    number = int.from_bytes(raw, 'big')
    text = ''
    while number:
        number, digit = divmod(number, 58)
        text = ALPHABET[digit] + text
    return '1' * (len(raw) - len(raw.lstrip(b'\0'))) + text


def unb58(text):
    number = 0
    for char in text:
        number = number*58 + ALPHABET.index(char)
    return b'\0'*(len(text)-len(text.lstrip('1'))) + number.to_bytes((number.bit_length()+7)//8,'big')


def compact(value):
    if value<64:
        return bytes([value<<2])
    if value<16384:
        return ((value<<2)|1).to_bytes(2,'little')
    if value<2**30:
        return ((value<<2)|2).to_bytes(4,'little')
    count=max(4,(value.bit_length()+7)//8)
    return bytes([((count-4)<<2)|3])+value.to_bytes(count,'little')


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, value, code=200):
        raw = json.dumps(value).encode()
        self.send_response(code)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        state = self.server.state
        path = urllib.parse.urlsplit(self.path).path
        assert state['chain'] == 'aptos', path
        if path == '/':
            return self.reply({'chain_id': 1, 'ledger_version': '100', 'ledger_timestamp': '1800000000000000'})
        if path == '/estimate_gas_price':
            return self.reply({'gas_estimate': state['gas_price']})
        if '/resource/' in path:
            return self.reply({'data': {'locked_until_secs': '1900000000'}})
        if path.startswith('/accounts/'):
            return self.reply({'sequence_number': str(state['nonce'])})
        if path.startswith('/transactions/by_hash/'):
            return self.reply({'hash': path.rsplit('/', 1)[1], 'type': 'pending_transaction'} if not state['final'] else
                              {'hash': state['hash'], 'type': 'user_transaction', 'version': '101', 'success': state['succeeded']})
        state['errors'].append(path)
        self.reply({'message': 'unexpected GET'}, 404)

    def do_POST(self):
        state = self.server.state
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        path = urllib.parse.urlsplit(self.path).path
        method = body.get('method', path)
        try:
            if state['chain'] == 'aptos':
                return self.aptos(body, path)
            result = getattr(self, state['chain'])(method, body.get('params', []))
            self.reply({'jsonrpc': '2.0', 'id': body['id'], 'result': result})
        except (AssertionError, KeyError, ValueError) as error:
            state['errors'].append((method, str(error)))
            self.reply({'jsonrpc': '2.0', 'id': body.get('id', 1), 'error': {'code': -32601, 'message': 'fixture rejected ' + method}})

    def solana(self, method, params):
        s = self.server.state
        if method == 'getGenesisHash':
            return '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d'
        if method == 'getLatestBlockhash':
            return {'value': {'blockhash': b58(bytes([0x33]) * 32), 'lastValidBlockHeight': 1000}}
        if method == 'isBlockhashValid':
            return {'value': s['block_valid']}
        if method == 'getVoteAccounts':
            return {'current': [{'votePubkey': s['target'], 'commission': 5, 'activatedStake': 2000000000}], 'delinquent': []}
        if method == 'getStakeMinimumDelegation':
            return {'value': 1000000000}
        if method == 'getMinimumBalanceForRentExemption':
            assert params[0] == 200
            return 2282880
        if method == 'getBalance':
            return {'value': s['balance']}
        if method == 'getFeeForMessage':
            return {'value': 5000}
        if method == 'simulateTransaction':
            assert params[1]['sigVerify'] is False
            raw = base64.b64decode(params[0])
            assert raw[0] == 1 and raw[1:65] == bytes(64)
            return {'value': {'err': None if s['simulation'] else {'InstructionError': [0, 'Custom']}}}
        if method == 'sendTransaction':
            raw = base64.b64decode(params[0])
            assert b58(raw[1:65]) == s['hash']
            s['submitted'].append(params[0])
            return b58(bytes([0x55]) * 64) if s['foreign'] else s['hash']
        if method == 'getSignatureStatuses':
            return {'value': [None if not s['final'] else {'confirmationStatus': 'finalized', 'slot': 101, 'err': None if s['succeeded'] else {'InstructionError': [0, 'Custom']}}]}
        raise AssertionError(method)

    def sui(self, method, params):
        s = self.server.state
        if method == 'sui_getChainIdentifier':
            return '35834a8a'
        if method == 'sui_getCheckpoint':
            return {'sequenceNumber': '0', 'digest': '4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S'}
        if method == 'suix_getReferenceGasPrice':
            return '1000'
        if method == 'suix_getLatestSuiSystemState':
            return {'activeValidators': [{'suiAddress': s['target'], 'name': 'Fixture validator', 'description': '',
                    'projectUrl': '', 'commissionRate': '500', 'stakingPoolSuiBalance': '2000000000'}]}
        if method == 'suix_getCoins':
            return {'data': [{'coinObjectId': '0x' + '33' * 32, 'version': '7', 'digest': '1' * 32,
                    'balance': str(s['balance'])}], 'hasNextPage': False, 'nextCursor': None}
        if method == 'sui_getObject':
            if params[0] == '0x5':
                return {'data': {'type': '0x3::sui_system::SuiSystemState', 'owner': {'Shared': {'initial_shared_version': 1}}}}
            assert params[0] == '0x' + '33' * 32, params
            return {'data': {'objectId': params[0], 'version': '7', 'digest': '1' * 32, 'owner': {'AddressOwner': s['owner']},
                    'type': '0x2::coin::Coin<0x2::sui::SUI>', 'content': {'fields': {'balance': str(s['balance'])}}}}
        if method == 'sui_dryRunTransactionBlock':
            assert base64.b64decode(params[0]).startswith(b'\0\0')
            return {'effects': {'status': {'status': 'success' if s['simulation'] else 'failure'}}}
        if method == 'sui_executeTransactionBlock':
            raw = base64.b64decode(params[0])
            assert b58(hashlib.blake2b(b'TransactionData::' + raw, digest_size=32).digest()) == s['hash']
            s['submitted'].append(params)
            return {'digest': b58(bytes([0x55])*32) if s['foreign'] else s['hash'], 'effects': {'status': {'status': 'success'}}}
        if method == 'sui_getTransactionBlock':
            if not s['final']:
                return {'digest': s['hash']}
            return {'digest': s['hash'], 'checkpoint': '101', 'effects': {'status': {'status': 'success' if s['succeeded'] else 'failure'}}}
        raise AssertionError(method)

    def aptos(self, body, path):
        s = self.server.state
        if path == '/view':
            function = body['function'].rsplit('::', 1)[1]
            result = {'delegation_pool_exists': [True], 'get_stake': ['2000000000', '1000000000', '0'],
                      'get_pending_withdrawal': [False, '1000000000'], 'operator_commission_percentage': ['500'],
                      'delegator_allowlisted': [True], 'get_add_stake_fee': ['1000000'],
                      'balance': [str(s['balance'])]}[function]
            return self.reply(result)
        if path == '/transactions/simulate':
            assert body['signature']['signature'] == '0x' + '00'*64
            assert body['payload']['function'].endswith('::add_stake')
            result = {'success': s['simulation'], 'vm_status': 'Executed' if s['simulation'] else 'ABORT', 'gas_used': '10000',
                      'max_gas_amount': '200000' if 'estimate_max' in self.path else body['max_gas_amount'],
                      'events': [{'type': '0x1::delegation_pool::AddStake', 'data': {'amount_added': body['payload']['arguments'][1]}}]}
            return self.reply([result])
        if path == '/transactions':
            s['submitted'].append(body)
            return self.reply({'hash': '0x' + '55'*32 if s['foreign'] else s['hash']})
        raise AssertionError(path)

    def near(self, method, params):
        s = self.server.state
        if method == 'status':
            return {'chain_id': 'mainnet'}
        if method == 'block':
            if params.get('finality') == 'optimistic':
                return {'header': {'hash': b58(bytes([0x66])*32), 'height': 96501 if s.get('expired') else 10001}}
            return {'header': {'hash': b58(bytes([0x33])*32), 'height': 10000}}
        if method == 'gas_price':
            return {'gas_price': str(s['gas_price'])}
        if method == 'EXPERIMENTAL_protocol_config':
            config=json.loads((ROOT/'core/tests/fixtures/near-staking-fee-protocol86.json').read_text())
            config['transaction_validity_period']=86400
            config['runtime_config']['storage_amount_per_byte']='10000000000000000000'
            return config
        if method == 'query':
            request = params['request_type']
            if request == 'view_access_key':
                return {'nonce': s['nonce'], 'permission': 'FullAccess' if s['authority'] else {'FunctionCall': {}}}
            if request == 'view_account':
                return {'amount': str(s['balance']), 'storage_usage': 1000, 'locked': '0'}
            assert request == 'call_function'
            name = params['method_name']
            if name == 'get_account':
                owner = json.loads(base64.b64decode(params['args_base64']))['account_id']
                value = {'account_id': owner, 'staked_balance': str(2*10**24), 'unstaked_balance': str(10**24), 'can_withdraw': False}
            else:
                value = {'is_whitelisted': True, 'get_owner_id': 'operator.near', 'is_staking_paused': False,
                         'get_reward_fee_fraction': {'numerator': 5, 'denominator': 100}}[name]
            return {'result': list(json.dumps(value).encode()), 'block_height': 100, 'logs': []}
        if method == 'broadcast_tx_commit':
            raw = base64.b64decode(params[0])
            assert b58(hashlib.sha256(raw[:-65]).digest()) == s['hash']
            s['submitted'].append(params[0])
            return {'transaction': {'hash': b58(bytes([0x55])*32) if s['foreign'] else s['hash']}}
        if method == 'tx':
            return {'transaction': {'hash': s['hash'], 'signer_id': s['owner']},
                    'final_execution_status': 'FINAL' if s['final'] else 'INCLUDED',
                    'status': {'SuccessValue': ''} if s['succeeded'] else {'Failure': {'ActionError': {}}}}
        raise AssertionError(method)

    def polkadot(self,method,params):
        s=self.server.state
        fixture=s['fixture']
        block=lambda number:'0x'+number.to_bytes(32,'big').hex()
        if method=='chain_getBlockHash':
            return '0x68d56f15f85d3136970ec16946040bc1752654e906147f7e43e9d539d7c3de2f' if params==[0] else block(params[0] if params else s['finalized'])
        if method=='chain_getFinalizedHead':
            return block(s['finalized'])
        if method=='chain_getHeader':
            return {'number':hex(int(params[0],16))}
        if method=='state_getRuntimeVersion':
            return {'specVersion':2005000,'transactionVersion':15}
        if method=='state_getMetadata':
            return '0x'+(ROOT/'core/tests/fixtures/asset-hub-polkadot-metadata.scale').read_bytes().hex()
        if method=='state_getKeysPaged':
            return [fixture['state']['pool']['key']] if len(params)<3 or params[2] is None else []
        if method=='state_getStorage':
            key=params[0]
            if key=='0x26aa394eea5630e07c48ae0c9558cef780d41e5e16056765bc8461851072c9d7':
                # One System::ExtrinsicSuccess event with empty topics.
                return '0x'+(compact(1)+bytes(5)+bytes(2)+bytes(4)+bytes(1)).hex()
            if key.startswith('0x26aa394eea5630e07c48ae0c9558cef7b99d'):
                return '0x'+(s['nonce'].to_bytes(4,'little')+bytes(12)+s['balance'].to_bytes(16,'little')+bytes(48)).hex()
            member_key=fixture['state']['member']['key']
            # Metadata declares Twox64Concat. The independent SDK fixture proves
            # that layout; this fresh owner must occupy this exact storage map.
            if len(key)==len(member_key) and key.startswith(member_key[:66]) and key.endswith(s['public'].hex()):
                return fixture['state']['member']['hex']
            for name,row in fixture['state'].items():
                if key==row['key']:
                    return row['hex']
            raise AssertionError(key)
        if method=='state_call':
            call=params[0]
            raw=bytes.fromhex(params[1][2:])
            if call=='NominationPoolsApi_points_to_balance':
                amount=int.from_bytes(raw[4:],'little')*9//10
                return '0x'+amount.to_bytes(16,'little').hex()
            if call=='NominationPoolsApi_balance_to_points':
                amount=int.from_bytes(raw[4:],'little')*10//9
                return '0x'+amount.to_bytes(16,'little').hex()
            if call=='NominationPoolsApi_pending_rewards':
                return '0x01'+(30000000000).to_bytes(16,'little').hex()
            if call in ('NominationPoolsApi_member_needs_delegate_migration','NominationPoolsApi_pool_needs_delegate_migration'):
                return '0x00'
            if call=='NominationPoolsApi_member_pending_slash':
                return '0x'+'00'*16
            raise AssertionError(call)
        if method=='system_accountNextIndex':
            return s['nonce']
        if method=='payment_queryInfo':
            return {'partialFee':str(s['fee'])}
        if method=='author_submitExtrinsic':
            assert '0x'+hashlib.blake2b(bytes.fromhex(params[0][2:]),digest_size=32).hexdigest()==s['hash']
            s['submitted'].append(params[0])
            return '0x'+'55'*32 if s['foreign'] else s['hash']
        if method=='chain_getBlock':
            return {'block':{'header':{'number':hex(int(params[0],16)), 'parentHash':block(int(params[0],16)-1)},'extrinsics':s['submitted'][:1] if int(params[0],16)==101 else []}}
        raise AssertionError(method)


def account_case(chain):
    """A stake lost in flight, retried with the same bytes from a new
    process, then recovered from its exact hash once final."""
    with tempfile.TemporaryDirectory(prefix='spectra-staking-') as directory:
        s = dict(chain=chain, owner='', nonce=7, balance=100*10**{'solana':9, 'sui':9, 'aptos':8, 'near':24}[chain],
                 gas_price=100 if chain=='aptos' else 100000000, authority=True, simulation=True, block_valid=True,
                 foreign=True, final=False, succeeded=True, submitted=[], errors=[], hash=None)
        s['target'] = b58(bytes([0x22])*32) if chain=='solana' else ('validator.poolv1.near' if chain=='near' else '0x'+'22'*32)
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
        server.state = s
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            def run(*args, success=True):
                result = subprocess.run([BINARY, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60,
                    env={**os.environ, 'PYTHONDONTWRITEBYTECODE':'1', 'SPECTRA_SEED':MNEMONIC})
                assert (result.returncode==0)==success, (chain, args, result.stdout, result.stderr, s['errors'])
                assert not s['errors'], s['errors']
                return json.loads(result.stdout)
            endpoint = f'http://127.0.0.1:{server.server_port}'
            api = {'solana':'solana-json-rpc', 'sui':'sui-json-rpc', 'aptos':'aptos-rest', 'near':'near-json-rpc'}[chain]
            s['owner'] = run('wallet','import','--chain',chain,'--name','Stake','--no-password')['wallet']['address']
            run('endpoints','--chain',chain,'--api',api,'--capabilities','staking,balance,fee,verification,broadcast','--add',endpoint)
            amount = '20' if chain=='aptos' else '2'
            prepared = run('staking','build','--from','Stake','--chain',chain,'--action','stake','--validator',s['target'],'--amount',amount)['artifact']
            assert prepared['staking']['action']=='stake' and prepared['review']['staking']['network_fee'], prepared
            assert run('send','inspect',prepared['id'])['artifact']==prepared
            signed = run('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',endpoint)['artifact']
            s['hash'] = signed['transaction_hash']
            assert s['hash'] and signed['signed_payload'] and not s['submitted'], signed
            broadcast = ('send','broadcast-signed',signed['id'],'--endpoint',endpoint,'--yes')
            # Broadcast reads the network again: a refusal submits nothing.
            refusal = 'authority' if chain=='near' else 'simulation'
            s[refusal]=False; run(*broadcast, success=False); s[refusal]=True
            assert not s['submitted']
            # The node's answer names another hash: submitted, outcome unknown.
            first = run(*broadcast)['artifact']
            assert first['attempts'][-1]['outcome']=='Uncertain' and first['transaction_hash']==s['hash'], first
            assert run('send','inspect',signed['id'])['artifact']==first
            s['foreign']=False
            accepted = run(*broadcast)['artifact']
            assert accepted['attempts'][-1]['outcome']=='Accepted', accepted
            assert len(s['submitted'])==2 and s['submitted'][0]==s['submitted'][1], s['submitted']
            # Final now, with a newer nonce and an expired blockhash or block:
            # the exact signed hash decides, and nothing is sent again.
            s['final']=True; s['nonce']+=1; s['block_valid']=False; s['expired']=chain=='near'
            run('staking','recheck','--id',signed['id'])
            refused = run(*broadcast, success=False)
            assert 'already confirmed' in refused['error'], refused
            assert len(s['submitted'])==2, s['submitted']
            record = run('txs','--record',signed['id'])['record']
            assert record['transactionHash']==s['hash'] and record['status']=='confirmed', record
            print(f'{chain}: reviewed stake, refused broadcast, uncertain retry and exact finality recovery passed', flush=True)
        finally:
            server.shutdown(); server.server_close(); worker.join()


def polkadot_case():
    with tempfile.TemporaryDirectory(prefix='spectra-staking-dot-') as directory:
        s=dict(chain='polkadot',owner='',public=bytes(),nonce=7,balance=1000000000000000,fee=1000000,
               finalized=100,foreign=True,submitted=[],errors=[],hash=None,
               fixture=json.loads((ROOT/'core/tests/fixtures/polkadot-staking-vectors.json').read_text()))
        server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Node);server.state=s
        worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
        try:
            def run(*args,success=True):
                result=subprocess.run([BINARY,'--data-dir',directory,'--json',*args],capture_output=True,text=True,timeout=60,
                    env={**os.environ,'SPECTRA_SEED':MNEMONIC})
                assert (result.returncode==0)==success,(args,result.stdout,result.stderr,s['errors'])
                assert not s['errors'],s['errors']
                return json.loads(result.stdout)
            endpoint=f'http://127.0.0.1:{server.server_port}'
            s['owner']=run('wallet','import','--chain','polkadot','--name','Stake','--no-password')['wallet']['address']
            s['public']=unb58(s['owner'])[1:33]
            run('endpoints','--chain','polkadot','--api','substrate-json-rpc','--capabilities','staking,balance,fee,verification,broadcast','--add',endpoint)
            prepared=run('staking','build','--from','Stake','--action','stake','--validator','7','--amount','1')['artifact']
            signed=run('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',endpoint)['artifact']
            s['hash']=signed['transaction_hash']
            assert run('send','inspect',signed['id'])['artifact']==signed
            broadcast=('send','broadcast-signed',signed['id'],'--endpoint',endpoint,'--yes')
            uncertain=run(*broadcast)['artifact']
            assert uncertain['attempts'][-1]['outcome']=='Uncertain',uncertain
            s['finalized']=101
            refusal=run(*broadcast,success=False)
            assert 'already confirmed' in refusal['error'],refusal
            assert len(s['submitted'])==1,s['submitted']
            run('staking','recheck','--id',signed['id'])
            refusal=run(*broadcast,success=False)
            assert 'already final; do not rebroadcast' in refusal['error'],refusal
            record=run('txs','--record',signed['id'])['record']
            assert record['transactionHash']==s['hash'] and record['status']=='confirmed',record
            print('polkadot: nomination-pool stake, durable uncertain submission and finalized event recovery passed',flush=True)
        finally:
            server.shutdown();server.server_close();worker.join()


if __name__=='__main__':
    selected = sys.argv[2:]
    for chain in ('solana','sui','aptos','near'):
        if not selected or chain in selected:
            account_case(chain)
    if not selected or 'polkadot' in selected:
        polkadot_case()
