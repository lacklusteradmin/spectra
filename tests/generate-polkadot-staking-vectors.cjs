#!/usr/bin/env node
// Run with the fixed official SDK installed at the directory argument.
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const prefix = process.argv[2] || '/private/tmp/spectra-polkadot-staking-sdk';
const { TypeRegistry, Metadata } = require(path.join(prefix, 'node_modules/@polkadot/types'));
const { xxhashAsU8a } = require(path.join(prefix, 'node_modules/@polkadot/util-crypto'));
const root = path.resolve(__dirname, '..');
const raw = fs.readFileSync(path.join(root, 'core/tests/fixtures/asset-hub-polkadot-metadata.scale'));
const registry = new TypeRegistry();
const metadata = new Metadata(registry, raw);
registry.setMetadata(metadata);
const owner = '0x' + '07'.repeat(32);
const pallet = (name) => metadata.asLatest.pallets.find(p => p.name.toString() === name);
const call = (palletName, name, args) => {
  const p = pallet(palletName);
  const variant = registry.lookup.getSiType(p.calls.unwrap().type).def.asVariant.variants.find(v => v.name.toString() === name);
  return registry.createType('Call', { callIndex: new Uint8Array([p.index.toNumber(), variant.index.toNumber()]), args });
};
const calls = {
  join: call('NominationPools','join',{amount:'10000000000',pool_id:7}),
  bondExtra: call('NominationPools','bond_extra',{extra:{FreeBalance:'10000000000'}}),
  unbond: call('NominationPools','unbond',{member_account:{Id:owner},unbonding_points:'50000000000'}),
  withdraw: call('NominationPools','withdraw_unbonded',{member_account:{Id:owner},num_slashing_spans:0}),
  claim: call('NominationPools','claim_payout',{}),
  migratePool: call('NominationPools','migrate_pool_to_delegate_stake',{pool_id:7}),
  migrateMember: call('NominationPools','migrate_delegation',{member_account:{Id:owner}}),
  applySlash: call('NominationPools','apply_slash',{member_account:{Id:owner}}),
};
calls.batchAll = call('Utility','batch_all',{calls:[calls.migratePool,calls.migrateMember,calls.applySlash,calls.claim]});
const storage = (palletName,item,key,input) => {
  const p=pallet(palletName);
  const entry=p.storage.unwrap().items.find(i=>i.name.toString()===item);
  const valueType=entry.type.isMap?entry.type.asMap.value:entry.type.asPlain;
  const value=registry.createTypeUnsafe(registry.createLookupType(valueType),[input]);
  const prefix=Buffer.concat([Buffer.from(xxhashAsU8a(p.storage.unwrap().prefix.toString(),128)),Buffer.from(xxhashAsU8a(item,128))]);
  let storageKey=prefix;
  if(entry.type.isMap){
    const encoded=registry.createTypeUnsafe(registry.createLookupType(entry.type.asMap.key),[key]).toU8a();
    if(entry.type.asMap.hashers[0].toString()!=='Twox64Concat')throw Error('Unexpected hasher');
    storageKey=Buffer.concat([prefix,Buffer.from(xxhashAsU8a(encoded,64)),Buffer.from(encoded)]);
  }
  return {key:'0x'+storageKey.toString('hex'),hex:'0x'+Buffer.from(value.toU8a()).toString('hex'),decoded:value.toJSON()};
};
const state = {
  member: storage('NominationPools','PoolMembers',owner,{poolId:7,points:'1000000000000',lastRecordedRewardCounter:'0',unbondingEras:new Map([[5,'100000000000'],[20,'50000000000']])}),
  pool: storage('NominationPools','BondedPools',7,{commission:{current:[100000000,owner],max:200000000,changeRate:null,throttleFrom:null,claimPermission:null},memberCounter:2,points:'2000000000000',roles:{depositor:owner,root:null,nominator:null,bouncer:null},state:'Open'}),
  subpools: storage('NominationPools','SubPoolsStorage',7,{noEra:{points:'1000000000000',balance:'900000000000'},withEra:new Map([[20,{points:'1000000000000',balance:'900000000000'}]])}),
  name: storage('NominationPools','Metadata',7,'0x5370656374726120506f6f6c'),
  minimum: storage('NominationPools','MinJoinBond',null,'10000000000'),
  era: storage('Staking','ActiveEra',null,{index:10,start:0}),
};
const output={sdk:'@polkadot/types@17.0.2',metadataSha256:crypto.createHash('sha256').update(raw).digest('hex'),owner,
  calls:Object.fromEntries(Object.entries(calls).map(([name,c])=>[name,c.toHex()])),state};
fs.writeFileSync(path.join(root,'core/tests/fixtures/polkadot-staking-vectors.json'),JSON.stringify(output,null,2)+'\n');
console.log('Generated independent Polkadot staking SCALE calls and storage vectors.');
