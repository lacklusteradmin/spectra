#!/usr/bin/env node
// Substrate native transfers from @polkadot/types and each runtime's own
// metadata, for core/tests/fixtures/substrate-transfers.json: where
// `Balances.transfer_keep_alive` sits, the existential deposit and the
// transaction extensions, and for each case the call, the signing payload
// and the signed v4 extrinsic, once with a fixed signature and once with
// polkadot.js's own sr25519 signature over the payload. sr25519 signatures
// are randomized: the signed ones are for verifying, not for reproducing.
// Also each captured finalized block's dispatch outcomes, read from its
// `System.Events` (core/tests/fixtures/substrate-blocks.json).
// node tests/vectors/generate-substrate-transfer-vectors.cjs
const fs = require('fs');
const crypto = require('crypto');
const { TypeRegistry, Metadata } = require('@polkadot/types');
const { Keyring } = require('@polkadot/keyring');
const { cryptoWaitReady } = require('@polkadot/util-crypto');
const { u8aToHex, u8aConcat } = require('@polkadot/util');

// Block zero's hash, as `chain_getBlockHash(0)` answers on each network.
const NETWORKS = {
  polkadot: {
    file: 'asset-hub-polkadot-metadata.scale',
    ss58: 0,
    genesis: '0x68d56f15f85d3136970ec16946040bc1752654e906147f7e43e9d539d7c3de2f',
  },
  bittensor: {
    file: 'bittensor-finney-metadata.scale',
    ss58: 42,
    genesis: '0x2f0555cc76fc2840a25a6ea3b9637146806f1f44b090c175ffde2a7e5ab36c03',
  },
  kusama: {
    file: 'asset-hub-kusama-metadata.scale',
    ss58: 2,
    genesis: '0x48239ef607d7928874027a43a67689209727dfb3d3dc5e5b03a39bdc2eda771a',
  },
  astar: {
    file: 'astar-metadata.scale',
    ss58: 5,
    genesis: '0x9eb76c5184c4ab8679d2d5d819fdf90b9c001403e9e17da2e14b6d8aec4029c6',
  },
};
const SEED = '0x' + '07'.repeat(32);
const RECIPIENT = '0x' + '09'.repeat(32);
const FIXED_SIGNATURE = u8aConcat(new Uint8Array([1]), new Uint8Array(64).fill(0x42));

function runtime(file) {
  const raw = fs.readFileSync(`core/tests/fixtures/${file}`);
  const registry = new TypeRegistry();
  const metadata = new Metadata(registry, raw);
  registry.setMetadata(metadata);
  const v14 = metadata.asV14;
  const pallet = (name) => v14.pallets.find((p) => p.name.toString() === name);
  const balances = pallet('Balances');
  const variant = registry.lookup.getSiType(balances.calls.unwrap().type).def.asVariant.variants
    .find((v) => v.name.toString() === 'transfer_keep_alive');
  const constant = (p, name) => {
    const c = p.constants.find((x) => x.name.toString() === name);
    return registry.createTypeUnsafe(registry.createLookupType(c.type), [c.value]);
  };
  const version = constant(pallet('System'), 'Version');
  const value = registry.lookup.getTypeDef(variant.fields[1].type).type;
  return {
    raw,
    registry,
    // The largest amount the runtime's `Balance` holds.
    max: { 'Compact<u64>': '18446744073709551615', 'Compact<u128>': '340282366920938463463374607431768211455' }[value],
    transfer: { pallet: balances.index.toNumber(), call: variant.index.toNumber() },
    existentialDeposit: constant(balances, 'ExistentialDeposit').toString(),
    extensions: v14.extrinsic.signedExtensions.map((e) => e.identifier.toString()),
    specVersion: version.specVersion.toNumber(),
    transactionVersion: version.transactionVersion.toNumber(),
  };
}

// Each finalized extrinsic's outcome: `System.ExtrinsicSuccess` or
// `ExtrinsicFailed` in the `ApplyExtrinsic` phase naming it.
function outcomes(registry, events) {
  const out = {};
  for (const record of registry.createType('Vec<FrameSystemEventRecord>', events)) {
    if (!record.phase.isApplyExtrinsic || record.event.section !== 'system') continue;
    const method = record.event.method;
    if (method !== 'ExtrinsicSuccess' && method !== 'ExtrinsicFailed') continue;
    const index = record.phase.asApplyExtrinsic.toString();
    if (index in out) throw new Error(`two outcomes for extrinsic ${index}`);
    out[index] = method === 'ExtrinsicSuccess';
  }
  return out;
}

(async () => {
  await cryptoWaitReady();
  const blocks = JSON.parse(fs.readFileSync('core/tests/fixtures/substrate-blocks.json', 'utf8'));
  const output = {
    sdk: '@polkadot/types@17.0.2, @polkadot/keyring@14.0.3',
    seed: SEED,
    recipient: RECIPIENT,
    fixedSignature: u8aToHex(FIXED_SIGNATURE),
  };
  for (const [network, { file, ss58, genesis }] of Object.entries(NETWORKS)) {
    const r = runtime(file);
    const pair = new Keyring({ type: 'sr25519', ss58Format: ss58 }).addFromSeed(Buffer.from(SEED.slice(2), 'hex'));
    if (!r.max) throw new Error(`${network}: unexpected Balance`);
    const cases = [];
    for (const [nonce, amount] of [[0, '1000000000000'], [70000, r.max]]) {
      const call = r.registry.createType('Call', {
        callIndex: new Uint8Array([r.transfer.pallet, r.transfer.call]),
        args: { dest: { Id: RECIPIENT }, value: amount },
      });
      const payload = r.registry.createType('ExtrinsicPayload', {
        method: call.toHex(),
        era: '0x00',
        nonce,
        tip: 0,
        specVersion: r.specVersion,
        transactionVersion: r.transactionVersion,
        genesisHash: genesis,
        blockHash: genesis,
        mode: 0,
        metadataHash: null,
        assetId: null,
      }, { version: 4 });
      const fixed = r.registry.createType('Extrinsic', { method: call }, { version: 4 });
      fixed.addSignature(pair.publicKey, FIXED_SIGNATURE, payload.toHex());
      const { signature } = payload.sign(pair);
      const signed = r.registry.createType('Extrinsic', { method: call }, { version: 4 });
      signed.addSignature(pair.publicKey, signature, payload.toHex());
      cases.push({
        nonce,
        amount,
        call: call.toHex(),
        signingPayload: u8aToHex(payload.toU8a({ method: true })),
        signedWithFixedSignature: fixed.toHex(),
        signature,
        signed: signed.toHex(),
      });
    }
    output[network] = {
      metadataSha256: crypto.createHash('sha256').update(r.raw).digest('hex'),
      sender: pair.address,
      genesisHash: genesis,
      specVersion: r.specVersion,
      transactionVersion: r.transactionVersion,
      transfer: r.transfer,
      existentialDeposit: r.existentialDeposit,
      extensions: r.extensions,
      cases,
    };
    if (blocks[network]) {
      output[network].block = {
        number: blocks[network].number,
        outcomes: outcomes(r.registry, blocks[network].events),
      };
    }
  }
  fs.writeFileSync('core/tests/fixtures/substrate-transfers.json', JSON.stringify(output, null, 2) + '\n');
  console.log('Generated independent Substrate transfer vectors.');
})();
