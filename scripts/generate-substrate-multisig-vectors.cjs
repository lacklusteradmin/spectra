// Independent pallet-multisig accounts, calls and storage on Polkadot Asset
// Hub and Bittensor, for core/tests/fixtures/substrate-multisig.json.
// Install the pinned libraries in a temporary directory:
// npm install --prefix /tmp/spectra-substrate-multisig --ignore-scripts @polkadot/types@17.0.2 @polkadot/keyring@14.0.3 @polkadot/util-crypto@14.0.3 @polkadot/util@14.0.3 @polkadot/wasm-crypto@7.5.4
// NODE_PATH=/tmp/spectra-substrate-multisig/node_modules node scripts/generate-substrate-multisig-vectors.cjs > core/tests/fixtures/substrate-multisig.json
//
// Three BIP-39 test phrases are the signatories, each the sr25519 root key of
// its phrase (no derivation path), as @polkadot/keyring reads it. The
// multisig account is createKeyMulti's blake2_256 of ("modlpy/utilisuba",
// sorted signatories, threshold), recomputed here byte by byte. Calls are
// encoded with @polkadot/types against the runtime metadata fixtures in the
// repo: a Balances.transfer_keep_alive as the inner call, and the
// approve_as_multi, as_multi and cancel_as_multi calls a 2-of-3 approval
// round makes around it, with other_signatories sorted and the submitter
// left out. The Multisig.Multisigs storage key is built by polkadot.js's
// decorated storage and checked against the hashers the metadata declares;
// a sample stored value is encoded with the metadata's own type.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { Keyring } = require('@polkadot/keyring');
const { TypeRegistry, Metadata } = require('@polkadot/types');
const { expandMetadata } = require('@polkadot/types/metadata/decorate');
const { u8aToHex, u8aSorted, u8aConcat, stringToU8a, u8aEq, compactStripLength, stringCamelCase } = require('@polkadot/util');
const {
  cryptoWaitReady, createKeyMulti, encodeAddress, blake2AsU8a, xxhashAsU8a,
  mnemonicToMiniSecret, sr25519PairFromSeed,
} = require('@polkadot/util-crypto');

const PHRASES = [
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
  'legal winner thank year wave sausage worth useful legal winner thank yellow',
  'letter advice cage absurd amount doctor acoustic avoid letter advice cage above',
];
const RECIPIENT = '0x' + 'dd'.repeat(32);
const TIMEPOINT = { height: 123, index: 2 };
const MAX_WEIGHT = { refTime: '1000000000', proofSize: '100000' };
const DEPOSIT = '20000000000';
const RUNTIMES = [
  { name: 'polkadot-asset-hub', metadata: 'core/tests/fixtures/asset-hub-polkadot-metadata.scale', value: '12345678900' },
  { name: 'bittensor', metadata: 'core/tests/fixtures/bittensor-finney-metadata.scale', value: '1234567890' },
];

const root = path.resolve(__dirname, '..');
const check = (condition, message) => { if (!condition) throw new Error(message); };

// createKeyMulti, by hand: blake2_256 over the SCALE tuple
// (b"modlpy/utilisuba", Vec<AccountId> sorted, u16 threshold).
function multiAccount(keys, threshold) {
  const sorted = u8aSorted(keys.map((key) => key.slice()));
  const length = new Uint8Array([sorted.length << 2]);
  const encoded = u8aConcat(stringToU8a('modlpy/utilisuba'), length, ...sorted, new Uint8Array([threshold & 0xff, threshold >> 8]));
  const id = createKeyMulti(keys, threshold);
  check(u8aEq(id, blake2AsU8a(encoded, 256)), 'createKeyMulti');
  return { account_id: u8aToHex(id), polkadot: encodeAddress(id, 0), substrate: encodeAddress(id, 42) };
}

function runtime({ name, metadata: file, value }, signatories) {
  const raw = fs.readFileSync(path.join(root, file));
  const registry = new TypeRegistry();
  const metadata = new Metadata(registry, raw);
  registry.setMetadata(metadata);
  const decorated = expandMetadata(registry, metadata);
  const pallet = (palletName) => metadata.asLatest.pallets.find((p) => p.name.toString() === palletName);
  const constant = (palletName, constName) => {
    const c = pallet(palletName).constants.find((entry) => entry.name.toString() === constName);
    return registry.createTypeUnsafe(registry.createLookupType(c.type), [c.value]);
  };
  const callIndices = (palletName, names) => {
    const variants = registry.lookup.getSiType(pallet(palletName).calls.unwrap().type).def.asVariant.variants;
    return Object.fromEntries(names.map((callName) => [callName, variants.find((v) => v.name.toString() === callName).index.toNumber()]));
  };
  const call = (palletName, callName, ...args) => {
    const created = decorated.tx[stringCamelCase(palletName)][stringCamelCase(callName)](...args);
    const index = [pallet(palletName).index.toNumber(), callIndices(palletName, [callName])[callName]];
    check(created.callIndex[0] === index[0] && created.callIndex[1] === index[1], `${name}: ${callName} index`);
    return created;
  };

  const balances = callIndices('Balances', ['transfer_keep_alive']);
  const multisigCalls = callIndices('Multisig', ['as_multi_threshold_1', 'as_multi', 'approve_as_multi', 'cancel_as_multi']);
  const inner = call('Balances', 'transfer_keep_alive', { Id: RECIPIENT }, value);
  const callHash = blake2AsU8a(inner.toU8a(), 256);
  const others = (submitter) => u8aSorted(signatories.filter((_, i) => i !== submitter).map((s) => s.pair.publicKey.slice())).map((key) => u8aToHex(key));

  // Multisig.Multisigs: double map (multisig account, call hash).
  const multisigAccount = createKeyMulti(signatories.map((s) => s.pair.publicKey), 2);
  const entry = pallet('Multisig').storage.unwrap().items.find((item) => item.name.toString() === 'Multisigs');
  const hashers = entry.type.asMap.hashers.map((h) => h.toString());
  check(hashers.join() === 'Twox64Concat,Blake2_128Concat', `${name}: unexpected Multisigs hashers ${hashers}`);
  const prefix = u8aConcat(xxhashAsU8a('Multisig', 128), xxhashAsU8a('Multisigs', 128));
  const key = u8aConcat(prefix, xxhashAsU8a(multisigAccount, 64), multisigAccount, blake2AsU8a(callHash, 128), callHash);
  // polkadot.js returns the key with a compact length prefix.
  check(u8aEq(key, compactStripLength(decorated.query.multisig.multisigs(multisigAccount, callHash))[1]), `${name}: storage key`);
  const p0 = u8aToHex(signatories[0].pair.publicKey);
  const sample = { when: TIMEPOINT, deposit: DEPOSIT, depositor: p0, approvals: [p0] };
  const stored = registry.createTypeUnsafe(registry.createLookupType(entry.type.asMap.value), [sample]);

  const version = constant('System', 'Version');
  return [name, {
    metadata: file,
    metadata_sha256: crypto.createHash('sha256').update(raw).digest('hex'),
    spec_name: version.specName.toString(),
    spec_version: version.specVersion.toNumber(),
    ss58_prefix: constant('System', 'SS58Prefix').toNumber(),
    balances: { pallet_index: pallet('Balances').index.toNumber(), calls: balances },
    multisig: {
      pallet_index: pallet('Multisig').index.toNumber(),
      calls: multisigCalls,
      deposit_base: constant('Multisig', 'DepositBase').toString(),
      deposit_factor: constant('Multisig', 'DepositFactor').toString(),
      max_signatories: constant('Multisig', 'MaxSignatories').toNumber(),
    },
    inner: {
      call: 'Balances.transfer_keep_alive',
      dest: RECIPIENT,
      value,
      hex: inner.toHex(),
      call_hash: u8aToHex(callHash),
    },
    calls: {
      // P0 opens the round with only the call hash.
      approve_as_multi_first: {
        submitter: 0,
        other_signatories: others(0),
        hex: call('Multisig', 'approve_as_multi', 2, others(0), null, callHash, MAX_WEIGHT).toHex(),
      },
      // P1 completes it, naming the round's timepoint and carrying the call.
      as_multi_final: {
        submitter: 1,
        other_signatories: others(1),
        hex: call('Multisig', 'as_multi', 2, others(1), TIMEPOINT, inner, MAX_WEIGHT).toHex(),
      },
      // P0 opens the round carrying the call itself.
      as_multi_first: {
        submitter: 0,
        other_signatories: others(0),
        hex: call('Multisig', 'as_multi', 2, others(0), null, inner, MAX_WEIGHT).toHex(),
      },
      cancel_as_multi: {
        submitter: 0,
        other_signatories: others(0),
        hex: call('Multisig', 'cancel_as_multi', 2, others(0), TIMEPOINT, callHash).toHex(),
      },
    },
    storage: {
      pallet_prefix: 'Multisig',
      item: 'Multisigs',
      hashers,
      prefix: u8aToHex(prefix),
      multisig_account: u8aToHex(multisigAccount),
      call_hash: u8aToHex(callHash),
      key: u8aToHex(key),
      value: {
        type: registry.lookup.getTypeDef(entry.type.asMap.value).namespace,
        fields: JSON.parse(registry.lookup.getTypeDef(entry.type.asMap.value).type),
        input: sample,
        hex: stored.toHex(),
      },
    },
  }];
}

(async () => {
  await cryptoWaitReady();
  const keyring = new Keyring({ type: 'sr25519' });
  const signatories = PHRASES.map((phrase) => {
    const pair = keyring.addFromUri(phrase);
    check(u8aEq(pair.publicKey, sr25519PairFromSeed(mnemonicToMiniSecret(phrase, '')).publicKey), 'root key');
    return { phrase, pair };
  });
  const sortedOrder = u8aSorted(signatories.map((s) => s.pair.publicKey.slice()))
    .map((key) => signatories.findIndex((s) => u8aEq(s.pair.publicKey, key)));
  const runtimes = Object.fromEntries(RUNTIMES.map((r) => runtime(r, signatories)));
  console.log(JSON.stringify({
    provenance: '@polkadot/types 17.0.2, @polkadot/keyring 14.0.3, @polkadot/util-crypto 14.0.3, @polkadot/util 14.0.3, @polkadot/wasm-crypto 7.5.4 (scripts/generate-substrate-multisig-vectors.cjs)',
    signatories: signatories.map(({ phrase, pair }) => ({
      phrase,
      public_key: u8aToHex(pair.publicKey),
      polkadot: encodeAddress(pair.publicKey, 0),
      substrate: encodeAddress(pair.publicKey, 42),
    })),
    // Signatory indices in ascending public-key order.
    sorted_order: sortedOrder,
    multisig: {
      threshold_2: multiAccount(signatories.map((s) => s.pair.publicKey), 2),
      threshold_3: multiAccount(signatories.map((s) => s.pair.publicKey), 3),
    },
    recipient: RECIPIENT,
    timepoint: TIMEPOINT,
    max_weight: { ref_time: MAX_WEIGHT.refTime, proof_size: MAX_WEIGHT.proofSize },
    runtimes,
  }, null, 2));
})();
