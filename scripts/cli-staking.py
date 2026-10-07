#!/usr/bin/env python3
"""Real account-chain staking through durable CLI stages; loopback only.

The fixture node models owned accounts/pools/objects, not a UI promise. Every
invocation opens the database afresh. No request can reach a public endpoint.
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
        state['reads'].append(path)
        if state['chain'] == 'near':
            assert path == '/v1/account/' + state['owner'] + '/staking', path
            return self.reply({'account_id': state['owner'], 'pools': [{'pool_id': state['target']}]})
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
        state['reads'].append(method)
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
        if method == 'getEpochInfo':
            return {'epoch': 10}
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
            return {'value': s['fee']}
        if method == 'simulateTransaction':
            assert params[1]['sigVerify'] is False
            raw = base64.b64decode(params[0])
            assert raw[0] == 1 and raw[1:65] == bytes(64)
            if s.get('simulation_preflight_error'):
                return {'value':{'err':'InsufficientFundsForFee'}}
            # The full withdrawal used for readiness is unavailable until cooldown.
            withdraw_amount=int.from_bytes(raw[-8:],'little') if raw[-12:-8]==(4).to_bytes(4,'little') else 0
            allowed = s['simulation'] and (not withdraw_amount or s['unlocked'] or withdraw_amount<=s.get('surplus',0))
            return {'value': {'err': None if allowed else {'InstructionError': [0, 'Custom']}}}
        if method in ('getProgramAccounts', 'getAccountInfo'):
            if method == 'getProgramAccounts':
                assert params[0] == 'Stake11111111111111111111111111111111111111'
                assert params[1]['filters'][1]['memcmp']['offset'] in (12, 44)
            account = {'lamports': 2002282880+s.get('surplus',0), 'owner': 'Stake11111111111111111111111111111111111111',
                       'data': {'parsed': {'type': 'delegated', 'info': {'meta': {'rentExemptReserve': '2282880',
                       'authorized': {'staker': s['owner'] if s['authority'] else s['target'], 'withdrawer': s['owner'] if s['authority'] else s['target']},
                       'lockup': {'epoch': 0, 'unixTimestamp': 0}}, 'stake': {'delegation': {'voter': s['target'],
                       'stake': '2000000000', 'activationEpoch': '8', 'deactivationEpoch': '9' if s['unlocked'] else str(2**64-1)}}}}}}
            return [{'pubkey': s['position'], 'account': account}] if method == 'getProgramAccounts' else {'value': account}
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
        if method == 'suix_getStakes':
            return [{'validatorAddress': s['target'], 'stakingPool': '0x' + '99' * 32, 'stakes': [{'stakedSuiId': s['position'],
                    'stakeActiveEpoch': '8', 'principal': '2000000000', 'status': 'Active', 'estimatedReward': '1234567'}]}]
        if method == 'suix_getCoins':
            return {'data': [{'coinObjectId': '0x' + '33' * 32, 'version': '7', 'digest': '1' * 32,
                    'balance': str(s['balance'])}], 'hasNextPage': False, 'nextCursor': None}
        if method == 'sui_getObject':
            if params[0] == '0x5':
                return {'data': {'type': '0x3::sui_system::SuiSystemState', 'owner': {'Shared': {'initial_shared_version': 1}}}}
            stake = params[0] == s['position']
            assert stake or params[0] == '0x' + '33' * 32, params
            fields = {'principal': '2000000000', 'pool_id': '0x' + '99' * 32, 'stake_activation_epoch': '8'} if stake else {'balance': str(s['balance'])}
            return {'data': {'objectId': params[0], 'version': '8' if stake else '7', 'digest': '1' * 32,
                    'owner': {'AddressOwner': s['owner'] if s['authority'] else s['target']},
                    'type': '0x3::staking_pool::StakedSui' if stake else '0x2::coin::Coin<0x2::sui::SUI>', 'content': {'fields': fields}}}
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
        if 'query' in body:
            assert 'current_delegator_balances' in body['query']
            rows = [] if body['variables']['after'] else [{'pool_address': s['target']}]
            return self.reply({'data': {'ledger_infos': [{'chain_id': 1}], 'current_delegator_balances': rows}})
        if path == '/view':
            function = body['function'].rsplit('::', 1)[1]
            if function == 'delegation_pool_exists':
                result = [s['real_pool']]
            elif function == 'get_stake':
                result = ['2000000000', '1000000000', '0']
            elif function == 'get_pending_withdrawal':
                result = [s['unlocked'], '1000000000']
            elif function == 'operator_commission_percentage':
                result = ['500']
            elif function == 'delegator_allowlisted':
                result = [s['authority']]
            elif function == 'get_add_stake_fee':
                result = ['1000000']
            elif function == 'balance':
                result = [str(s['balance'])]
            else:
                raise AssertionError(function)
            return self.reply(result)
        if path == '/transactions/simulate':
            assert body['signature']['signature'] == '0x' + '00'*64
            name = body['payload']['function'].rsplit('::', 1)[1]
            event, field = {'add_stake': ('AddStake', 'amount_added'), 'unlock': ('UnlockStake', 'amount_unlocked'), 'withdraw': ('WithdrawStake', 'amount_withdrawn')}[name]
            amount = int(body['payload']['arguments'][1])
            result = {'success': s['simulation'], 'vm_status': 'Executed' if s['simulation'] else 'ABORT', 'gas_used': '10000',
                      'max_gas_amount': '200000' if 'estimate_max' in self.path else body['max_gas_amount'],
                      'events': [{'type': '0x1::delegation_pool::' + event, 'data': {field: str(amount + int(s['adjust_amount']))}}]}
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
            reference = b58(bytes([0x33])*32)
            if params.get('finality') == 'optimistic':
                return {'header': {'hash': b58(bytes([0x66])*32), 'height': 96501 if s.get('expired') else 10001}}
            return {'header': {'hash': b58(bytes([0x55])*32) if isinstance(params.get('block_id'),int) and s.get('fork') else reference, 'height': 10000}}
        if method == 'gas_price':
            return {'gas_price': str(s['gas_price'])}
        if method == 'EXPERIMENTAL_protocol_config':
            config=json.loads((ROOT/'core/tests/fixtures/near-staking-fee-protocol86.json').read_text())
            config['transaction_validity_period']=86400
            config['runtime_config']['storage_amount_per_byte']='10000000000000000000'
            if s.get('fee_schedule_changed'):
                config['runtime_config']['transaction_costs']['action_creation_config']['function_call_cost']['execution']*=2
            return config
        if method == 'query':
            request = params['request_type']
            if request == 'view_access_key':
                return {'nonce': s['nonce'], 'permission': 'FullAccess' if s['authority'] else {'FunctionCall': {}}}
            if request == 'view_account':
                return {'amount': str(s['balance']), 'storage_usage': 1000, 'locked': '0'}
            assert request == 'call_function'
            name = params['method_name']
            if name == 'is_whitelisted':
                value = s['real_pool']
            elif name == 'get_owner_id':
                value = 'operator.near'
            elif name == 'get_account':
                owner = json.loads(base64.b64decode(params['args_base64']))['account_id']
                value = {'account_id': owner, 'staked_balance': str(2*10**24), 'unstaked_balance': str(10**24), 'can_withdraw': s['unlocked']}
            elif name == 'get_reward_fee_fraction':
                value = {'numerator': 5, 'denominator': 100}
            elif name == 'is_staking_paused':
                value = False
            else:
                raise AssertionError(name)
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


def account_case(chain, action):
    with tempfile.TemporaryDirectory(prefix='spectra-staking-') as directory:
        s = dict(chain=chain, owner='', nonce=7, balance=100*10**{'solana':9, 'sui':9, 'aptos':8, 'near':24}[chain],
                 gas_price=100 if chain=='aptos' else 100000000, fee=5000, authority=True, real_pool=True,
                 simulation=True, unlocked=action=='withdraw', block_valid=True, adjust_amount=False,
                 foreign=True, final=False, succeeded=action!='withdraw', submitted=[], reads=[], errors=[], hash=None)
        s['target'] = b58(bytes([0x22])*32) if chain=='solana' else ('validator.poolv1.near' if chain=='near' else '0x'+'22'*32)
        s['position'] = b58(bytes([0x44])*32) if chain=='solana' else ('validator.poolv1.near' if chain=='near' else ('0x' + ('22' if chain=='aptos' else '44')*32))
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
        server.state = s
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            def run(*args, success=True):
                result = subprocess.run([BINARY, '--data-dir', directory, '--json', *args], capture_output=True, text=True, timeout=60,
                    env={**os.environ, 'PYTHONDONTWRITEBYTECODE':'1', 'SPECTRA_SEED':MNEMONIC, 'SPECTRA_LOOPBACK_ONLY':str(pathlib.Path(directory)/'network.jsonl')})
                assert (result.returncode==0)==success, (chain, action, args, result.stdout, result.stderr, s['errors'])
                assert not s['errors'], s['errors']
                return json.loads(result.stdout)
            endpoint = f'http://127.0.0.1:{server.server_port}'
            api = {'solana':'solana-json-rpc', 'sui':'sui-json-rpc', 'aptos':'aptos-rest', 'near':'near-json-rpc'}[chain]
            s['owner'] = run('wallet','import','--chain',chain,'--name','Stake','--no-password')['wallet']['address']
            run('endpoints','--chain',chain,'--api',api,'--capabilities','staking,balance,fee,verification,broadcast','--add',endpoint)
            if chain in ('aptos','near'):
                run('endpoints','--chain',chain,'--api','aptos-indexer' if chain=='aptos' else 'fastnear',
                    '--capabilities','history' if chain=='aptos' else 'staking','--add',endpoint)
            positions = run('staking','positions','--from','Stake','--chain',chain,'--pool',s['position'])['positions']
            assert len(positions)==1 and positions[0]['owner']==s['owner'], positions
            discovered=run('staking','positions','--from','Stake','--chain',chain)['positions']
            assert discovered==positions,(positions,discovered)
            if chain=='sui':
                assert positions[0]['claimable_rewards_smallest_unit']=='1234567', positions
            if chain=='solana' and action=='stake':
                s['surplus']=1000000000
                surplus=run('staking','positions','--from','Stake','--chain',chain)['positions'][0]
                assert surplus['staked_amount_smallest_unit']=='2000000000' and surplus['withdrawable_amount_smallest_unit']=='1000000000',surplus
                s['surplus']=0
                s['simulation_preflight_error']=True
                run('staking','positions','--from','Stake','--chain',chain,success=False)
                s['simulation_preflight_error']=False
            amount = {'solana':'2.00228288' if action=='withdraw' else '2', 'sui':'2', 'aptos':'10' if action=='withdraw' else '20', 'near':'1' if action=='withdraw' else '2'}[chain]
            args = ('staking','build','--from','Stake','--chain',chain,'--action',action,
                    '--validator' if action=='stake' else '--position',s['target'] if action=='stake' else s['position'],'--amount',amount)
            if action=='stake':
                # Another address: the owner's is already this store's signing wallet.
                observed = b58(bytes([0x33])*32) if chain=='solana' else ('33'*32 if chain=='near' else '0x'+'33'*32)
                run('wallet','watch','--chain',chain,'--name','Observe','--address',observed)
                run(*(args[:3]+('Observe',)+args[4:]),success=False)
                s['balance']=0; run(*args, success=False); s['balance']=100*10**{'solana':9,'sui':9,'aptos':8,'near':24}[chain]
                if chain in ('solana','sui'):
                    run(*(args[:-1]+('0.5',)),success=False)
                    s['authority']=False
                    run('staking','positions','--from','Stake','--chain',chain,'--pool',s['position'],success=False)
                    s['authority']=True
            if action=='withdraw' and chain!='sui':
                s['unlocked']=False; run(*args, success=False); s['unlocked']=True
            if chain=='aptos':
                s['adjust_amount']=True; run(*args, success=False); s['adjust_amount']=False
            if chain=='near':
                # Enough for the former gas*current-price quote, insufficient
                # for the protocol's minimum purchase price and receipt fees.
                s['balance']=(2*10**24 if action=='stake' else 0)+2*10**22
                run(*args,success=False)
                s['balance']=100*10**24
            prepared = run(*args)['artifact']
            assert prepared['staking']['action']==action and prepared['review']['staking']['network_fee'], prepared
            if chain=='near':
                config=json.loads((ROOT/'core/tests/fixtures/near-staking-fee-protocol86.json').read_text())['runtime_config']
                fees=config['transaction_costs']; receipt=fees['action_receipt_creation_config']; fc=fees['action_creation_config']['function_call_cost']; byte=fees['action_creation_config']['function_call_cost_per_byte']
                method={'stake':'deposit_and_stake','unstake':'unstake','withdraw':'withdraw'}[action]
                arg={} if action=='stake' else {'amount':str((1 if action=='withdraw' else 2)*10**24)}
                size=len(method)+len(json.dumps(arg,separators=(',',':')))
                budget=(receipt['send_not_sir']+fc['send_not_sir']+byte['send_not_sir']*size)*s['gas_price']+(100000000000000+receipt['execution']+fc['execution']+byte['execution']*size)*max(s['gas_price'],int(config['min_gas_purchase_price']))
                decimal=(str(budget//10**24)+'.'+str(budget%10**24).zfill(24).rstrip('0')).rstrip('.')
                assert prepared['review']['staking']['network_fee']==decimal,prepared
            assert run('send','inspect',prepared['id'])['artifact']==prepared
            sign_args = ('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',endpoint)
            if chain=='solana':
                s['fee']=10000; run(*sign_args, success=False); s['fee']=5000
            elif chain=='sui':
                s['authority']=False; run(*sign_args, success=False); s['authority']=True
            elif chain=='aptos':
                s['simulation']=False; run(*sign_args, success=False); s['simulation']=True
            else:
                s['authority']=False; run(*sign_args, success=False); s['authority']=True
                s['gas_price']*=2; run(*sign_args, success=False); s['gas_price']//=2
                s['fee_schedule_changed']=True;run(*sign_args,success=False);s['fee_schedule_changed']=False
                s['expired']=True; run(*sign_args,success=False); s['expired']=False
                s['fork']=True; run(*sign_args,success=False); s['fork']=False
            signed = run(*sign_args)['artifact']; s['hash']=signed['transaction_hash']
            assert s['hash'] and signed['signed_payload'] and not s['submitted'], signed
            broadcast = ('send','broadcast-signed',signed['id'],'--endpoint',endpoint,'--yes')
            if chain in ('solana','sui','aptos'):
                s['simulation']=False; run(*broadcast, success=False); s['simulation']=True
            else:
                s['authority']=False; run(*broadcast, success=False); s['authority']=True
                s['expired']=True; run(*broadcast,success=False); s['expired']=False
                s['fork']=True; run(*broadcast,success=False); s['fork']=False
            assert not s['submitted']
            s['foreign']=action!='stake'
            first = run(*broadcast)['artifact']
            assert first['attempts'][-1]['outcome']==('Accepted' if action=='stake' else 'Uncertain') and first['transaction_hash']==s['hash'], first
            assert run('send','inspect',signed['id'])['artifact']==first
            if action=='unstake':
                s['foreign']=False
                accepted=run(*broadcast)['artifact']
                assert accepted['attempts'][-1]['outcome']=='Accepted',accepted
                assert len(s['submitted'])==2 and s['submitted'][0]==s['submitted'][1],s['submitted']
            # The first response was lost/foreign. Discovery must read this exact
            # signed hash before inspecting stale nonces or sending another byte.
            s['final']=True; s['nonce']+=1; s['block_valid']=False
            s['expired']=chain=='near'
            if action=='withdraw':
                run(*broadcast, success=False)
            else:
                run('staking','recheck','--id',signed['id'])
            assert len(s['submitted'])==(2 if action=='unstake' else 1), s['submitted']
            record = run('txs','--record',signed['id'])['record']
            assert record['transactionHash']==s['hash'], record
            assert record['status']==('confirmed' if s['succeeded'] else 'failed'), record
            journal = pathlib.Path(directory)/'network.jsonl'
            if journal.exists():
                assert not journal.read_text().strip(), journal.read_text()
            print(f'{chain} {action}: owned position, exact review, sign, durable submission and exact finality recovery passed', flush=True)
        finally:
            server.shutdown(); server.server_close(); worker.join()


def polkadot_case():
    with tempfile.TemporaryDirectory(prefix='spectra-staking-dot-') as directory:
        s=dict(chain='polkadot',owner='',public=bytes(),nonce=7,balance=1000000000000000,fee=1000000,
               finalized=100,foreign=True,submitted=[],reads=[],errors=[],hash=None,
               fixture=json.loads((ROOT/'core/tests/fixtures/polkadot-staking-vectors.json').read_text()))
        server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Node);server.state=s
        worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
        try:
            def run(*args,success=True):
                result=subprocess.run([BINARY,'--data-dir',directory,'--json',*args],capture_output=True,text=True,timeout=60,
                    env={**os.environ,'SPECTRA_SEED':MNEMONIC,'SPECTRA_LOOPBACK_ONLY':str(pathlib.Path(directory)/'network.jsonl')})
                assert (result.returncode==0)==success,(args,result.stdout,result.stderr,s['errors'])
                assert not s['errors'],s['errors']
                return json.loads(result.stdout)
            endpoint=f'http://127.0.0.1:{server.server_port}'
            s['owner']=run('wallet','import','--chain','polkadot','--name','Stake','--no-password')['wallet']['address']
            s['public']=unb58(s['owner'])[1:33]
            run('endpoints','--chain','polkadot','--api','substrate-json-rpc','--capabilities','staking,balance,fee,verification,broadcast','--add',endpoint)
            positions=run('staking','positions','--from','Stake')['positions']
            assert len(positions)==1 and positions[0]['owner']==s['owner'],positions
            assert positions[0]['claimable_rewards_smallest_unit']=='30000000000',positions
            prepared=run('staking','build','--from','Stake','--action','stake','--validator','7','--amount','1')['artifact']
            sign=('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',endpoint)
            s['fee']*=2;run(*sign,success=False);s['fee']//=2
            signed=run(*sign)['artifact'];s['hash']=signed['transaction_hash']
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
            journal=pathlib.Path(directory)/'network.jsonl'
            if journal.exists():
                assert not journal.read_text().strip(),journal.read_text()
            print('polkadot: real nomination-pool state, SDK metadata, reviewed fee, durable uncertain submission and finalized event recovery passed',flush=True)
        finally:
            server.shutdown();server.server_close();worker.join()


if __name__=='__main__':
    selected = sys.argv[2:]
    for chain in ('solana','sui','aptos','near'):
        if selected and chain not in selected:
            continue
        for action in ('stake','unstake','withdraw'):
            account_case(chain,action)
    if not selected or 'polkadot' in selected:
        polkadot_case()
