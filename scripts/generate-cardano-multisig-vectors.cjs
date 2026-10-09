// Independent Cardano native-script multisig accounts and transactions, for
// core/tests/fixtures/cardano-multisig.json.
// Install the pinned libraries in a temporary directory:
// npm install --prefix /tmp/spectra-cardano-multisig --ignore-scripts @emurgo/cardano-serialization-lib-nodejs@15.0.3 bip39@3.1.0 @noble/hashes@1.8.0
// NODE_PATH=/tmp/spectra-cardano-multisig/node_modules node scripts/generate-cardano-multisig-vectors.cjs > core/tests/fixtures/cardano-multisig.json
//
// Three BIP-39 test phrases are the cosigners, each with its CIP-1854 shared
// payment key at m/1854'/1815'/0'/0/0 (Icarus root from the phrase's entropy,
// empty password). cardano-serialization-lib builds three native scripts from
// those key hashes — 2-of-3, all-of-two-keys-before-a-slot, and any-of-two —
// from its constructors, each recorded beside its cardano-cli JSON with its
// hash and its enterprise address on mainnet and Preprod. (CSL's own
// cardano-cli JSON reader, ScriptSchema.Node, is unimplemented and panics.)
// It then builds a spend from the 2-of-3 address and one from the
// time-locked address, signs each body hash with the cosigners' leaf keys and
// attaches the script to the witness set. Ed25519 is deterministic, so the
// signed transactions compare byte for byte. @noble/hashes recomputes every
// key hash, script hash (blake2b-224 over 0x00 || script CBOR) and body hash
// as a check on CSL's.
const bip39 = require('bip39');
const { blake2b } = require('@noble/hashes/blake2b');
const C = require('@emurgo/cardano-serialization-lib-nodejs');

const PHRASES = [
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
  'legal winner thank year wave sausage worth useful legal winner thank yellow',
  'letter advice cage absurd amount doctor acoustic avoid letter advice cage above',
];
const RECIPIENT = 'addr1qx2fxv2umyhttkxyxp8x0dlpdt3k6cwng5pxj3jhsydzer3n0d3vllmyqwsx5wktcd8cc3sq835lu7drv2xwl2wywfgse35a3x';
const NETWORKS = [['cardano', 1], ['cardano-preprod', 0]];

const hex = (bytes) => Buffer.from(bytes).toString('hex');
const harden = (n) => 0x80000000 + n;
const check = (condition, message) => { if (!condition) throw new Error(message); };
const blake224 = (bytes) => hex(blake2b(bytes, { dkLen: 28 }));
const blake256 = (bytes) => hex(blake2b(bytes, { dkLen: 32 }));

const cosigners = PHRASES.map((phrase) => {
  const root = C.Bip32PrivateKey.from_bip39_entropy(Buffer.from(bip39.mnemonicToEntropy(phrase), 'hex'), Buffer.alloc(0));
  const account = root.derive(harden(1854)).derive(harden(1815)).derive(harden(0));
  const leaf = account.derive(0).derive(0);
  const publicKey = leaf.to_public().to_raw_key();
  const keyHash = publicKey.hash().to_hex();
  check(keyHash === blake224(publicKey.as_bytes()), 'key hash');
  const accountPublic = account.to_public();
  return {
    phrase,
    account_path: "m/1854'/1815'/0'",
    // CSL has no acct_shared_xvk bech32 encoder: the 64-byte extended public
    // key (public key || chain code).
    account_xvk: hex(accountPublic.as_bytes()),
    path: "m/1854'/1815'/0'/0/0",
    public_key: hex(publicKey.as_bytes()),
    key_hash: keyHash,
    // The 64-byte extended private key kL || kR, without the chain code.
    private_key: hex(leaf.to_raw_key().as_bytes()),
    signer: leaf.to_raw_key(),
  };
});

const sig = (index) => ({ type: 'sig', keyHash: cosigners[index].key_hash });
const SCRIPTS = [
  ['s1', { type: 'atLeast', required: 2, scripts: [sig(0), sig(1), sig(2)] }],
  ['s2', { type: 'all', scripts: [sig(0), sig(1), { type: 'before', slot: 90000000 }] }],
  ['s3', { type: 'any', scripts: [sig(2), sig(1)] }],
];

// The script from CSL's constructors, read off its cardano-cli JSON.
function construct(json) {
  const list = (scripts) => {
    const native = C.NativeScripts.new();
    for (const script of scripts) native.add(construct(script));
    return native;
  };
  switch (json.type) {
    case 'sig': return C.NativeScript.new_script_pubkey(C.ScriptPubkey.new(C.Ed25519KeyHash.from_hex(json.keyHash)));
    case 'all': return C.NativeScript.new_script_all(C.ScriptAll.new(list(json.scripts)));
    case 'any': return C.NativeScript.new_script_any(C.ScriptAny.new(list(json.scripts)));
    case 'atLeast': return C.NativeScript.new_script_n_of_k(C.ScriptNOfK.new(json.required, list(json.scripts)));
    case 'before': return C.NativeScript.new_timelock_expiry(C.TimelockExpiry.new_timelockexpiry(C.BigNum.from_str(String(json.slot))));
    case 'after': return C.NativeScript.new_timelock_start(C.TimelockStart.new_timelockstart(C.BigNum.from_str(String(json.slot))));
    default: throw new Error(`unknown script type ${json.type}`);
  }
}

const scripts = Object.fromEntries(SCRIPTS.map(([name, json]) => {
  const native = construct(json);
  const cbor = hex(native.to_bytes());
  check(cbor === hex(C.NativeScript.from_bytes(native.to_bytes()).to_bytes()), `${name}: CBOR round trip`);
  const hash = native.hash().to_hex();
  check(hash === blake224(Buffer.concat([Buffer.from([0x00]), native.to_bytes()])), `${name}: script hash`);
  const addresses = Object.fromEntries(NETWORKS.map(([chain, network]) => {
    const address = C.EnterpriseAddress.new(network, C.Credential.from_scripthash(native.hash())).to_address();
    check(address.to_bytes()[0] === (0x70 | network), `${name}: enterprise script header`);
    return [chain, { address: address.to_bech32(), bytes: hex(address.to_bytes()) }];
  }));
  return [name, { json, cbor, hash, addresses, native }];
}));

function spend(name, scriptName, input, outputs, fee, ttl, signerSets) {
  const script = scripts[scriptName];
  const inputs = C.TransactionInputs.new();
  inputs.add(C.TransactionInput.new(C.TransactionHash.from_hex(input.tx_hash), input.tx_index));
  const txOutputs = C.TransactionOutputs.new();
  for (const out of outputs) {
    txOutputs.add(C.TransactionOutput.new(C.Address.from_bech32(out.address), C.Value.new(C.BigNum.from_str(out.lovelace))));
  }
  const total = outputs.reduce((sum, out) => sum + BigInt(out.lovelace), BigInt(fee));
  check(total === BigInt(input.lovelace), `${name}: inputs and outputs do not balance`);
  const body = C.TransactionBody.new_tx_body(inputs, txOutputs, C.BigNum.from_str(fee));
  body.set_ttl(C.BigNum.from_str(String(ttl)));
  const bodyBytes = body.to_bytes();
  const fixed = C.FixedTransaction.new_from_body_bytes(bodyBytes);
  const txHash = fixed.transaction_hash();
  check(txHash.to_hex() === blake256(bodyBytes), `${name}: body hash`);

  const witnesses = {};
  const signed = signerSets.map((signers) => {
    const vkeys = C.Vkeywitnesses.new();
    for (const index of signers) {
      const witness = C.make_vkey_witness(txHash, cosigners[index].signer);
      const vkey = witness.vkey().public_key();
      check(vkey.verify(txHash.to_bytes(), witness.signature()), `${name}: signature`);
      witnesses[`k${index}`] = { public_key: hex(vkey.as_bytes()), signature: witness.signature().to_hex() };
      vkeys.add(witness);
    }
    const nativeScripts = C.NativeScripts.new();
    nativeScripts.add(script.native);
    const witnessSet = C.TransactionWitnessSet.new();
    witnessSet.set_vkeys(vkeys);
    witnessSet.set_native_scripts(nativeScripts);
    const transaction = C.Transaction.new(body, witnessSet, undefined);
    const bytes = transaction.to_bytes();
    check(C.FixedTransaction.from_bytes(bytes).transaction_hash().to_hex() === txHash.to_hex(), `${name}: hash after signing`);
    check(hex(C.FixedTransaction.from_bytes(bytes).raw_body()) === hex(bodyBytes), `${name}: body bytes after signing`);
    return {
      signers: signers.map((index) => `k${index}`),
      witness_set: hex(witnessSet.to_bytes()),
      transaction: hex(bytes),
    };
  });
  return {
    name,
    script: scriptName,
    inputs: [input],
    outputs,
    fee,
    ttl,
    validity_start: null,
    body: hex(bodyBytes),
    hash: txHash.to_hex(),
    witnesses,
    signed,
  };
}

const s1Address = scripts.s1.addresses.cardano.address;
const s2Address = scripts.s2.addresses.cardano.address;
const transactions = [
  spend('s1_spend', 's1', { tx_hash: 'aa'.repeat(32), tx_index: 0, lovelace: '10000000' }, [
    { address: RECIPIENT, lovelace: '2000000' },
    { address: s1Address, lovelace: '7800000' },
  ], '200000', 50000000, [[0, 2], [0]]),
  // The script expires at slot 90000000, so the transaction's TTL must not
  // pass it.
  spend('s2_spend', 's2', { tx_hash: 'bb'.repeat(32), tx_index: 1, lovelace: '10000000' }, [
    { address: RECIPIENT, lovelace: '2000000' },
    { address: s2Address, lovelace: '7800000' },
  ], '200000', 80000000, [[0, 1]]),
];

// Whether CSL writes a set with the CBOR tag 258 (`d9 0102`): checked on a
// body and on witness sets holding only vkeys or only native scripts.
const tagged = (bytes, offset) => hex(bytes.slice(offset, offset + 3)) === 'd90102';
const onlyVkeys = C.TransactionWitnessSet.new();
const vkeys = C.Vkeywitnesses.new();
vkeys.add(C.make_vkey_witness(C.TransactionHash.from_hex(transactions[0].hash), cosigners[0].signer));
onlyVkeys.set_vkeys(vkeys);
const onlyScripts = C.TransactionWitnessSet.new();
const nativeScripts = C.NativeScripts.new();
nativeScripts.add(scripts.s1.native);
onlyScripts.set_native_scripts(nativeScripts);
const body = Buffer.from(transactions[0].body, 'hex');
check(body[0] === 0xa4 && body[1] === 0x00, 'body layout');
const encoding = {
  body_inputs_tag_258: tagged(body, 2),
  vkey_witnesses_tag_258: tagged(onlyVkeys.to_bytes(), 2),
  native_scripts_tag_258: tagged(onlyScripts.to_bytes(), 2),
};

console.log(JSON.stringify({
  provenance: '@emurgo/cardano-serialization-lib-nodejs 15.0.3, bip39 3.1.0, @noble/hashes 1.8.0 (scripts/generate-cardano-multisig-vectors.cjs)',
  encoding,
  cosigners: cosigners.map(({ signer, ...rest }) => rest),
  scripts: Object.fromEntries(Object.entries(scripts).map(([name, { native, ...rest }]) => [name, rest])),
  recipient: { address: RECIPIENT, bytes: hex(C.Address.from_bech32(RECIPIENT).to_bytes()) },
  transactions,
}, null, 2));
