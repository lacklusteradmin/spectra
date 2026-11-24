#!/usr/bin/env node
// Asset Hub `Assets` pallet references from @polkadot/types and the runtimes'
// own metadata: where `Assets.transfer` sits and the calls it encodes, and the
// `Asset`, `Account` and `Metadata` storage keys and values a transfer reads.
// node tests/vectors/generate-asset-hub-asset-vectors.cjs
const fs = require('fs');
const crypto = require('crypto');
const { TypeRegistry, Metadata } = require('@polkadot/types');
const { decorateStorage } = require('@polkadot/types/metadata/decorate');
const { compactStripLength } = require('@polkadot/util');

const OWNER = '0x' + '07'.repeat(32);
const RECIPIENT = '0x' + '09'.repeat(32);
const ISSUER = '0x' + '0b'.repeat(32);

function runtime(file) {
  const raw = fs.readFileSync(`core/tests/fixtures/${file}`);
  const registry = new TypeRegistry();
  const metadata = new Metadata(registry, raw);
  registry.setMetadata(metadata);
  const pallet = metadata.asLatest.pallets.find((p) => p.name.toString() === 'Assets');
  const variant = registry.lookup.getSiType(pallet.calls.unwrap().type).def.asVariant.variants
    .find((v) => v.name.toString() === 'transfer');
  const transfer = { pallet: pallet.index.toNumber(), call: variant.index.toNumber() };
  const call = (args) => registry.createType('Call', {
    callIndex: new Uint8Array([transfer.pallet, transfer.call]), args,
  }).toHex();
  const storage = decorateStorage(registry, metadata.asLatest, metadata.version);
  const item = (name) => pallet.storage.unwrap().items.find((i) => i.name.toString() === name);
  const value = (name, input) => {
    const type = registry.createLookupType(item(name).type.asMap.value);
    return registry.createTypeUnsafe(type, [input]);
  };
  const entry = (name, keys, input) => {
    const encoded = value(name, input);
    return {
      // The decorated key is length-prefixed, as a `Bytes` is.
      key: '0x' + Buffer.from(compactStripLength(storage.assets[name.charAt(0).toLowerCase() + name.slice(1)](...keys))[1]).toString('hex'),
      hex: encoded.toHex(),
      decoded: encoded.toJSON(),
    };
  };
  return { raw, transfer, call, entry };
}

const asset = (overrides) => ({
  owner: ISSUER, issuer: ISSUER, admin: ISSUER, freezer: ISSUER, supply: '1000000000000000',
  deposit: '0', minBalance: '700000', isSufficient: true, accounts: 5, sufficients: 5, approvals: 0,
  status: 'Live', ...overrides,
});
const account = (overrides) => ({ balance: '123456789', status: 'Liquid', reason: 'Sufficient', extra: null, ...overrides });

const output = { sdk: '@polkadot/types@17.0.2', owner: OWNER, recipient: RECIPIENT };
for (const [network, file] of [['polkadot', 'asset-hub-polkadot-metadata.scale'], ['westend', 'asset-hub-westend-metadata.scale']]) {
  const r = runtime(file);
  output[network] = {
    metadataSha256: crypto.createHash('sha256').update(r.raw).digest('hex'),
    transfer: r.transfer,
    calls: {
      usdt: r.call({ id: 1984, target: { Id: RECIPIENT }, amount: '1234567' }),
      max: r.call({ id: 4294967295, target: { Id: RECIPIENT }, amount: '340282366920938463463374607431768211455' }),
    },
  };
  if (network !== 'polkadot') continue;
  output.polkadot.state = {
    asset: r.entry('Asset', [1984], asset({})),
    asset_frozen: r.entry('Asset', [1984], asset({ status: 'Frozen' })),
    asset_insufficient: r.entry('Asset', [1984], asset({ isSufficient: false })),
    account: r.entry('Account', [1984, OWNER], account({})),
    account_frozen: r.entry('Account', [1984, OWNER], account({ status: 'Frozen' })),
    account_blocked: r.entry('Account', [1984, OWNER], account({ status: 'Blocked' })),
    recipient: r.entry('Account', [1984, RECIPIENT], account({ balance: '5000000' })),
    metadata: r.entry('Metadata', [1984], { deposit: '0', name: 'Tether USD', symbol: 'USDT', decimals: 6, isFrozen: false }),
    metadata_eight: r.entry('Metadata', [1984], { deposit: '0', name: 'Tether USD', symbol: 'USDT', decimals: 8, isFrozen: false }),
  };
}
fs.writeFileSync('core/tests/fixtures/asset-hub-assets.json', JSON.stringify(output, null, 2) + '\n');
console.log('Generated independent Asset Hub asset calls and storage vectors.');
