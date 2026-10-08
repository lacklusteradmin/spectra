#!/usr/bin/env python3
"""NEAR native/NEP-141 protocol fees, storage and retry rules; loopback only.
A token send to a recipient the token has registered is ft_transfer alone; to
one it has not, storage_deposit with the contract's minimum first, reviewed
as part of the cost; a contract that answers no NEP-145 query is refused.

Protocol-86 costs were captured independently from the mainnet RPC. Byte-level
signing is also covered by the official NEAR SDK transaction vectors in Rust.
"""
import base64
import decimal
import hashlib
import http.server
import importlib.util
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT/'target/debug/spectra').resolve())
spec = importlib.util.spec_from_file_location('staking_fixture', ROOT/'scripts/cli-staking.py')
staking = importlib.util.module_from_spec(spec)
spec.loader.exec_module(staking)
CONFIG = json.loads((ROOT/'core/tests/fixtures/near-staking-fee-protocol86.json').read_text())
UNIT = 10**24


def display(raw):
    return format(decimal.Decimal(raw)/decimal.Decimal(UNIT),'f').rstrip('0').rstrip('.') if raw else '0'


REGISTRATION=1250000000000000000000


def budget(receiver,token,price=100000000,register=False):
    costs=CONFIG['runtime_config']['transaction_costs']
    receipt=costs['action_receipt_creation_config']
    actions=costs['action_creation_config']
    parts=[receipt]
    gas=0
    if token:
        # These are the exact deterministic argument bytes signed by
        # storage_deposit, when the recipient needs registering, and ft_transfer.
        calls=[('ft_transfer',{'amount':'123456789','receiver_id':receiver})]
        if register:
            calls.insert(0,('storage_deposit',{'account_id':receiver,'registration_only':True}))
        for method,args in calls:
            count=len(method)+len(json.dumps(args,separators=(',',':')).encode())
            parts.extend([actions['function_call_cost'],{key:value*count for key,value in actions['function_call_cost_per_byte'].items()}])
            gas+=30000000000000
    else:
        parts.append(actions['transfer_cost'])
        if len(receiver)==64:
            parts.extend([actions['create_account_cost'],actions['add_key_cost']['full_access_cost']])
    burnt=sum(part['send_not_sir'] for part in parts)
    remaining=gas+sum(part['execution'] for part in parts)
    return burnt*price+remaining*max(price,int(CONFIG['runtime_config']['min_gas_purchase_price']))+(1 if token else 0)


class Node(staking.Node):
    def near(self,method,params):
        s=self.server.state
        if method=='status': return {'chain_id':s['network']}
        if method=='query':
            if params['request_type']=='view_account':
                return {'amount':str(s['balance']),'locked':str(s['locked']),'storage_usage':s['storage']}
            if params['request_type']=='call_function':
                name=params['method_name']
                if name=='ft_metadata': value={'spec':'ft-1.0.0','name':'Fixture','symbol':'TEST','decimals':s['decimals']}
                elif name=='ft_balance_of':
                    owner=json.loads(base64.b64decode(params['args_base64']))['account_id']
                    assert owner in (s['owner'],'22'*32),(owner,params)
                    value=str(s['token_balance'] if owner==s['owner'] else 0)
                elif name in ('storage_balance_of','storage_balance_bounds') and s['nep145']=='absent':
                    # What nearcore answers for a method the contract lacks.
                    return {'error':'wasm execution failed with error: FunctionCallError(MethodResolveError(MethodNotFound))',
                            'logs':[],'block_height':100}
                elif name=='storage_balance_of':
                    assert json.loads(base64.b64decode(params['args_base64']))=={'account_id':'22'*32},params
                    value={'total':str(REGISTRATION),'available':'0'} if s['nep145']=='registered' else None
                elif name=='storage_balance_bounds':
                    value={'min':str(REGISTRATION),'max':str(REGISTRATION)}
                else: raise AssertionError(name)
                return {'result':list(json.dumps(value).encode()),'block_height':100,'logs':[]}
        return super().near(method,params)


def case(chain,token):
    with tempfile.TemporaryDirectory(prefix='spectra-near-send-') as directory:
        s=dict(chain='near',network='testnet' if chain.endswith('testnet') else 'mainnet',owner='',nonce=7,
               balance=100*UNIT,locked=0,storage=1000,token_balance=223456789,decimals=6,
               gas_price=100000000,authority=True,foreign=True,final=False,succeeded=True,
               submitted=[],reads=[],errors=[],hash=None,nep145='registered')
        server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Node);server.state=s
        worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
        try:
            def run(*args,success=True):
                result=subprocess.run([BINARY,'--data-dir',directory,'--json',*args],capture_output=True,text=True,timeout=60,
                    env={**os.environ,'PYTHONDONTWRITEBYTECODE':'1','SPECTRA_SEED':staking.MNEMONIC,'SPECTRA_LOOPBACK_ONLY':str(pathlib.Path(directory)/'network.jsonl')})
                assert (result.returncode==0)==success,(chain,token,args,result.stdout,result.stderr,s['errors'])
                assert not s['errors'],s['errors']
                return json.loads(result.stdout)
            endpoint=f'http://127.0.0.1:{server.server_port}'
            s['owner']=run('wallet','import','--chain',chain,'--name','Near','--no-password')['wallet']['address']
            run('endpoints','--chain',chain,'--api','near-json-rpc','--capabilities','balance,fee,verification,token-balance,broadcast','--add',endpoint)
            contract='token.testnet' if chain.endswith('testnet') else 'token.near'
            if token: run('token','add','--chain',chain,'--standard','NEP-141','--symbol','TEST','--name','Fixture','--contract',contract,'--decimals','6')
            with sqlite3.connect(pathlib.Path(directory)/'spectra.sqlite') as db:
                wid,payload=db.execute('SELECT id,payload FROM wallets').fetchone();wallet=json.loads(payload)
                native=dict(name='NEAR',symbol='NEAR',coingeckoId='near',chainId=chain,tokenStandard='Native',contractAddress=None,amount='100')
                wallet['holdings']=[native]
                if token:wallet['holdings'].append(dict(name='Fixture',symbol='TEST',coingeckoId='',chainId=chain,tokenStandard='NEP-141',contractAddress=contract,amount='223.456789'))
                db.execute('UPDATE wallets SET payload=? WHERE id=?',(json.dumps(wallet),wid))
            holding=chain+(':nep-141:'+contract if token else ':native')
            receiver='22'*32
            amount='123.456789' if token else '1'
            preview_args=('send','preview','--wallet','Near','--holding',holding,'--amount',amount,'--destination',receiver)
            preview=run(*preview_args)['preview']; expected=budget(receiver,token)
            assert preview['network_fee']==display(expected),preview
            assert decimal.Decimal(preview['network_fee'])*UNIT==expected,preview
            assert preview['details']['spendableBalance']==('223.456789' if token else '99.99'),preview
            quote=run('send','quote','--wallet','Near','--holding',holding,'--amount',amount,'--destination',receiver)['quote']
            assert quote['request']['fee_amount']==display(expected),quote
            if not token:
                named=run('send','preview','--wallet','Near','--holding',holding,'--amount',amount,'--destination','receiver.near')['preview']
                assert named['network_fee']==display(budget('receiver.near',False)) and decimal.Decimal(named['network_fee'])<decimal.Decimal(preview['network_fee']),named
                s['storage']=770
                assert run(*preview_args)['preview']['details']['spendableBalance']=='100'
                s['storage']=1000;s['locked']=UNIT
                assert run(*preview_args)['preview']['details']['spendableBalance']=='100'
                s['locked']=0
            build=('send','build','--from','Near','--to',receiver,'--amount',amount,'--endpoint',endpoint)+ (('--contract',contract,'--decimals','6') if token else ())
            # These balances cover the old 0.001 fee but not the real protocol budget.
            s['balance']=12*UNIT//1000 if token else UNIT+11*UNIT//1000
            refused=run(*build,success=False);assert 'Insufficient spendable NEAR' in refused['error'],refused
            s['balance']=100*UNIT
            s['authority']=False;run(*build,success=False);s['authority']=True
            if token:
                s['decimals']=7;run(*build,success=False);s['decimals']=6
                s['token_balance']=0;run(*build,success=False);s['token_balance']=223456789
            if token:
                registration(s,run,build,receiver)
            prepared=run(*build)['artifact']
            assert json.loads(prepared['prepared_details'])['Near']['fee_budget']==str(expected),prepared
            assert json.loads(prepared['prepared_details'])['Near']['registration_deposit'] is None,prepared
            assert prepared['review']['transfer_terms'] is None,prepared
            sign=('send','sign',prepared['id'],'--review-digest',prepared['review_digest'],'--endpoint',endpoint)
            s['gas_price']*=2;run(*sign,success=False);s['gas_price']//=2
            s['balance']=0;run(*sign,success=False);s['balance']=100*UNIT
            s['authority']=False;run(*sign,success=False);s['authority']=True
            s['nonce']+=1;run(*sign,success=False);s['nonce']-=1
            signed=run(*sign)['artifact'];s['hash']=signed['transaction_hash']
            assert signed==run('send','inspect',signed['id'])['artifact']
            broadcast=('send','broadcast-signed',signed['id'],'--endpoint',endpoint,'--yes')
            s['gas_price']*=2;run(*broadcast,success=False);s['gas_price']//=2
            assert not s['submitted']
            uncertain=run(*broadcast)['artifact'];assert uncertain['attempts'][-1]['outcome']=='Uncertain',uncertain
            s['gas_price']*=2;run(*broadcast,success=False);s['gas_price']//=2
            assert len(s['submitted'])==1
            s['foreign']=False
            accepted=run(*broadcast)['artifact'];assert accepted['attempts'][-1]['outcome']=='Accepted',accepted
            assert s['submitted'][0]==s['submitted'][1]
            # Exact final status recovery comes before stale nonce, expiry and fee checks.
            s['final']=True;s['expired']=True;s['nonce']+=1;s['gas_price']*=2;s['balance']=0
            refusal=run(*broadcast,success=False);assert 'already confirmed' in refusal['error'],refusal
            assert len(s['submitted'])==2
            record=run('txs','--record',signed['id'])['record'];assert record['status']=='confirmed' and record['transactionHash']==s['hash'],record
            journal=pathlib.Path(directory)/'network.jsonl'
            assert not journal.exists() or not journal.read_text().strip()
            print(chain+(' NEP-141' if token else ' native')+': live protocol fees, storage/authority, '
                  +('recipient registration, ' if token else '')+'durable review and retry recovery passed',flush=True)
        finally:
            server.shutdown();server.server_close();worker.join()


def actions(signed):
    """The (method, JSON args, deposit) of each FunctionCall a signed transaction carries."""
    raw=base64.b64decode(json.loads(signed['signed_payload'])['signed_tx_b64'])
    at=0
    def take(n):
        nonlocal at
        at+=n
        return raw[at-n:at]
    def string(): return take(int.from_bytes(take(4),'little'))
    string();take(33);take(8);string();take(32)
    calls=[]
    for _ in range(int.from_bytes(take(4),'little')):
        assert take(1)==b'\x02'
        method=string().decode();args=json.loads(string());take(8)
        calls.append((method,args,int.from_bytes(take(16),'little')))
    assert len(raw)-at==65
    return calls


def registration(s,run,build,receiver):
    """A recipient the token has not registered: storage_deposit first, its
    deposit reviewed, counted against the balance and read again at signing;
    a contract with no NEP-145 answer refused before anything is built."""
    s['nep145']='absent'
    refused=run(*build,success=False);assert 'NEP-145' in refused['error'],refused
    s['nep145']='unregistered'
    # Its own nonce, so signing it reserves none the later sends use.
    s['nonce']+=100
    expected=budget(receiver,True,register=True)
    # Enough for the fee and the 0.01 NEAR storage stake, not the registration too.
    s["balance"]=expected+REGISTRATION-1+10**22
    refused=run(*build,success=False);assert 'Insufficient spendable NEAR' in refused['error'],refused
    s['balance']=100*UNIT
    unregistered=run(*build)['artifact']
    near=json.loads(unregistered['prepared_details'])['Near']
    assert (near['fee_budget'],near['registration_deposit'])==(str(expected),str(REGISTRATION)),near
    assert unregistered['review']['transfer_terms']['recipient_registration']=='0.00125',unregistered
    sign=('send','sign',unregistered['id'],'--review-digest',unregistered['review_digest'])
    # Registered between the review and signing: built again, not signed.
    s['nep145']='registered'
    refused=run(*sign,success=False);assert 'registration with the token changed' in refused['error'],refused
    s['nep145']='unregistered'
    calls=actions(run(*sign)['artifact'])
    assert calls==[('storage_deposit',{'account_id':receiver,'registration_only':True},REGISTRATION),
                   ('ft_transfer',{'amount':'123456789','receiver_id':receiver},1)],calls
    s['nep145']='registered'
    s['nonce']-=100


if __name__=='__main__':
    for chain in ('near','near-testnet'):
        for token in (False,True):case(chain,token)
