// Independent Candid, ingress IDs and Ed25519 signatures from DFINITY SDKs.
// npm install --prefix /tmp/spectra-icp-sdk --ignore-scripts @dfinity/candid@3.4.3 @dfinity/principal@3.4.3 @dfinity/agent@3.4.3 @dfinity/identity@3.4.3
// NODE_PATH=/tmp/spectra-icp-sdk/node_modules node scripts/generate-icp-staking-vectors.cjs
// Candid fields are taken from dfinity/ic rs/nns/governance/canister/governance.did.
const fs = require('node:fs'), crypto = require('node:crypto');
const { IDL } = require('@dfinity/candid');
const { Principal } = require('@dfinity/principal');
const { Cbor, requestIdOf } = require('@dfinity/agent');
const { Ed25519KeyIdentity } = require('@dfinity/identity');
const hex = value => Buffer.from(value).toString('hex');
const blob = IDL.Vec(IDL.Nat8), empty = IDL.Record({}), neuronId = IDL.Record({id:IDL.Nat64});
const accountId = IDL.Record({hash:blob});
const account = IDL.Record({owner:IDL.Opt(IDL.Principal),subaccount:IDL.Opt(blob)});
const configureOperation = IDL.Variant({StartDissolving:empty,IncreaseDissolveDelay:IDL.Record({additional_dissolve_delay_seconds:IDL.Nat32})});
const configure = IDL.Record({operation:IDL.Opt(configureOperation)});
const disburse = IDL.Record({to_account:IDL.Opt(accountId),amount:IDL.Opt(IDL.Record({e8s:IDL.Nat64}))});
const follow = IDL.Record({topic:IDL.Int32,followees:IDL.Vec(neuronId)});
const maturity = IDL.Record({percentage_to_disburse:IDL.Nat32,to_account:IDL.Opt(account),to_account_identifier:IDL.Opt(accountId)});
const manageCommand = IDL.Variant({Configure:configure,Disburse:disburse,Follow:follow,DisburseMaturity:maturity});
const selector = IDL.Variant({Subaccount:blob,NeuronId:neuronId});
const manage = IDL.Record({id:IDL.Opt(neuronId),neuron_id_or_subaccount:IDL.Opt(selector),command:IDL.Opt(manageCommand)});
const claim = IDL.Record({memo:IDL.Nat64,controller:IDL.Opt(IDL.Principal)});
const dissolve = IDL.Variant({DissolveDelaySeconds:IDL.Nat64,WhenDissolvedTimestampSeconds:IDL.Nat64});
const neuron = IDL.Record({id:IDL.Opt(neuronId),controller:IDL.Opt(IDL.Principal),account:blob,cached_neuron_stake_e8s:IDL.Nat64,
  neuron_fees_e8s:IDL.Nat64,maturity_e8s_equivalent:IDL.Nat64,staked_maturity_e8s_equivalent:IDL.Opt(IDL.Nat64),dissolve_state:IDL.Opt(dissolve),spawn_at_timestamp_seconds:IDL.Opt(IDL.Nat64),
  maturity_disbursements_in_progress:IDL.Opt(IDL.Vec(IDL.Record({amount_e8s:IDL.Opt(IDL.Nat64),timestamp_of_disbursement_seconds:IDL.Opt(IDL.Nat64),finalize_disbursement_timestamp_seconds:IDL.Opt(IDL.Nat64),account_to_disburse_to:IDL.Opt(account),account_identifier_to_disburse_to:IDL.Opt(accountId)}))) });
const listResponse = IDL.Record({full_neurons:IDL.Vec(neuron),total_pages_available:IDL.Opt(IDL.Nat64)});
const economics = IDL.Record({neuron_minimum_stake_e8s:IDL.Nat64,transaction_fee_e8s:IDL.Nat64});
const knownResponse = IDL.Record({known_neurons:IDL.Vec(IDL.Record({id:IDL.Opt(neuronId),known_neuron_data:IDL.Opt(IDL.Record({name:IDL.Text,description:IDL.Opt(IDL.Text)}))}))});
const error = IDL.Record({error_type:IDL.Int32,error_message:IDL.Text});
const claimResponse = IDL.Record({result:IDL.Opt(IDL.Variant({Error:error,NeuronId:neuronId}))});
const manageResponse = IDL.Record({command:IDL.Opt(IDL.Variant({Error:error,Configure:empty,Follow:empty,
  Disburse:IDL.Record({transfer_block_height:IDL.Nat64}),DisburseMaturity:IDL.Record({amount_disbursed_e8s:IDL.Opt(IDL.Nat64)})}))});

function accountIdentifier(principal,subaccount=Buffer.alloc(32)) {
  const hash=crypto.createHash('sha224').update(Buffer.from('\x0aaccount-id')).update(principal.toUint8Array()).update(subaccount).digest();
  let crc=0xffffffff;for(const byte of hash){crc^=byte;for(let i=0;i<8;i++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}
  const prefix=Buffer.alloc(4);prefix.writeUInt32BE((crc^0xffffffff)>>>0);return Buffer.concat([prefix,hash]);
}
const encode = (type,value) => Buffer.from(IDL.encode([type],[value]));
const queryReply = (type,value) => hex(Cbor.encode({status:'replied',reply:{arg:encode(type,value)}}));
async function main() {
  const identity=Ed25519KeyIdentity.generate(new Uint8Array(32).fill(1)), owner=identity.getPrincipal();
  const governance=Principal.fromText('rrkah-fqaaa-aaaaa-aaaaq-cai');
  const index=Buffer.alloc(8);index.writeBigUInt64BE(42n);
  const subaccount=crypto.createHash('sha256').update(Buffer.from('\x0cneuron-stake')).update(owner.toUint8Array()).update(index).digest();
  const id={id:42n}, base={id:[id],controller:[owner],account:subaccount,cached_neuron_stake_e8s:200000000n,neuron_fees_e8s:10000n,maturity_e8s_equivalent:200000000n,staked_maturity_e8s_equivalent:[],spawn_at_timestamp_seconds:[],maturity_disbursements_in_progress:[]};
  const owned = state => ({...base,dissolve_state:[state]});
  const foreign = {...owned({DissolveDelaySeconds:600n}),id:[{id:43n}],controller:[Ed25519KeyIdentity.generate(new Uint8Array(32).fill(2)).getPrincipal()]};
  const fixtures={provenance:'@dfinity/candid, principal, agent, identity 3.4.3; official NNS governance.did',
    public_key:hex(identity.getPublicKey().toRaw()),controller:owner.toText(),owner:hex(accountIdentifier(owner)),neuron_nonce:'42',subaccount:hex(subaccount),neuron_account:hex(accountIdentifier(governance,subaccount)),
    query_replies:{
      economics:queryReply(economics,{neuron_minimum_stake_e8s:100000000n,transaction_fee_e8s:10000n}),
      known:queryReply(knownResponse,{known_neurons:[{id:[{id:1n}],known_neuron_data:[{name:'Fixture known neuron',description:[]}]}]}),
      active:queryReply(listResponse,{full_neurons:[owned({DissolveDelaySeconds:600n}),foreign],total_pages_available:[1n]}),
      unlocking:queryReply(listResponse,{full_neurons:[owned({WhenDissolvedTimestampSeconds:1900000000n}),foreign],total_pages_available:[1n]}),
      ready:queryReply(listResponse,{full_neurons:[owned({DissolveDelaySeconds:0n}),foreign],total_pages_available:[1n]}),
      queued:queryReply(listResponse,{full_neurons:[{...owned({DissolveDelaySeconds:600n}),maturity_e8s_equivalent:0n,maturity_disbursements_in_progress:[[{amount_e8s:[1000000n],timestamp_of_disbursement_seconds:[1800000000n],finalize_disbursement_timestamp_seconds:[1800604800n],account_to_disburse_to:[],account_identifier_to_disburse_to:[{hash:accountIdentifier(owner)}]}]]}],total_pages_available:[1n]}),
    },
    replies:{claim:hex(encode(claimResponse,{result:[{NeuronId:id}]})),configure:hex(encode(manageResponse,{command:[{Configure:{}}]})),follow:hex(encode(manageResponse,{command:[{Follow:{}}]})),disburse:hex(encode(manageResponse,{command:[{Disburse:{transfer_block_height:100n}}]})),maturity:hex(encode(manageResponse,{command:[{DisburseMaturity:{amount_disbursed_e8s:[1000000n]}}]})),error:hex(encode(manageResponse,{command:[{Error:{error_type:5,error_message:'not ready'}}]}))},
    calls:[], ingress_expiry:'1800000240000000000'};
  const target={Subaccount:subaccount}, selected=command=>({id:[],neuron_id_or_subaccount:[target],command:[command]});
  const calls=[
    {kind:'Claim',method:'claim_or_refresh_neuron_from_account',argument:encode(claim,{memo:42n,controller:[]})},
    {kind:'Configure',method:'manage_neuron',argument:encode(manage,selected({Configure:{operation:[{IncreaseDissolveDelay:{additional_dissolve_delay_seconds:1}}]}}))},
    {kind:'Configure',method:'manage_neuron',argument:encode(manage,selected({Configure:{operation:[{StartDissolving:{}}]}}))},
    {kind:'Follow',method:'manage_neuron',argument:encode(manage,selected({Follow:{topic:0,followees:[{id:1n}]}}))},
    {kind:'Disburse',method:'manage_neuron',argument:encode(manage,selected({Disburse:{to_account:[{hash:accountIdentifier(owner)}],amount:[{e8s:100010000n}]}}))},
    {kind:'DisburseMaturity',method:'manage_neuron',argument:encode(manage,selected({DisburseMaturity:{percentage_to_disburse:100,to_account:[],to_account_identifier:[{hash:accountIdentifier(owner)}]}}))},
  ];
  fixtures.full_disburse_argument_hex=hex(encode(manage,selected({Disburse:{to_account:[{hash:accountIdentifier(owner)}],amount:[]}})));
  for(const call of calls){
    const nonce=IDL.encode([IDL.Nat64],[42n]);
    const content={request_type:'call',canister_id:governance.toUint8Array(),method_name:call.method,arg:call.argument,sender:owner.toUint8Array(),ingress_expiry:BigInt(fixtures.ingress_expiry),nonce};
    const requestId=requestIdOf(content);
    const read={request_type:'read_state',sender:owner.toUint8Array(),ingress_expiry:BigInt(fixtures.ingress_expiry),paths:[[new TextEncoder().encode('request_status'),requestId]]};
    const domain=new TextEncoder().encode('\x0aic-request');
    const challenge=id=>new Uint8Array([...domain,...id]);
    fixtures.calls.push({kind:call.kind,method:call.method,argument_hex:hex(call.argument),nonce_hex:hex(nonce),request_id:hex(requestId),signature:hex(await identity.sign(challenge(requestId))),read_state_signature:hex(await identity.sign(challenge(requestIdOf(read))))});
  }
  fs.writeFileSync('core/tests/fixtures/icp-staking-vectors.json',JSON.stringify(fixtures,null,2)+'\n');
}
main().catch(error=>{process.stderr.write(String(error)+'\n');process.exitCode=1;});
