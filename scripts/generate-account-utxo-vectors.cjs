// Independent signed transactions spending a wallet account's addresses, each
// input with its own key, for core/tests/fixtures/account-utxo-transactions.json.
// Install the pinned libraries in a temporary directory:
// npm install --prefix /tmp/spectra-account-utxo --ignore-scripts bitcoinjs-lib@7.0.2 ecpair@3.0.1 tiny-secp256k1@2.2.4 kaspa-wasm@0.13.0
// NODE_PATH=/tmp/spectra-account-utxo/node_modules node scripts/generate-account-utxo-vectors.cjs core/tests/fixtures/account-utxo-transactions.json
// (kaspa-wasm writes its own lines to stdout, so the fixture is written to the path given.)
//
// bitcoinjs-lib computes every Bitcoin-format sighash: legacy (Dogecoin,
// Dash), BIP143 (SegWit v0 and, with SIGHASH_FORKID in the hash type, Bitcoin
// Cash, Bitcoin SV and Bitcoin Gold's fork id 79) and BIP341 (Taproot).
// ECDSA signatures are RFC 6979 and low-S on both sides, so those
// transactions are compared byte for byte. A Schnorr signature carries
// auxiliary randomness, so a Taproot vector gives its unsigned transaction and
// its sighashes, which a signature must verify against. kaspa-wasm gives
// Kaspa's per-input sighashes the same way.
const crypto = require('node:crypto');
const fs = require('node:fs');
const bitcoin = require('bitcoinjs-lib');
const ecc = require('tiny-secp256k1');
const { ECPairFactory } = require('ecpair');
const kaspa = require('kaspa-wasm');

bitcoin.initEccLib(ecc);
const ECPair = ECPairFactory(ecc);
const hex = (bytes) => Buffer.from(bytes).toString('hex');
// Three keys: two the account's inputs are spent with, one the recipient's.
const key = (index) => ECPair.fromPrivateKey(crypto.createHash('sha256').update(`spectra account key ${index}`).digest());
const keys = [key(0), key(1), key(2)];
const txid = (index) => crypto.createHash('sha256').update(`spectra account input ${index}`).digest('hex');

function p2pkh(pair) { return bitcoin.payments.p2pkh({ pubkey: pair.publicKey }).output; }
function p2wpkh(pair) { return bitcoin.payments.p2wpkh({ pubkey: pair.publicKey }).output; }
function nested(pair) { return bitcoin.payments.p2sh({ redeem: bitcoin.payments.p2wpkh({ pubkey: pair.publicKey }) }).output; }
function internal(pair) { return Buffer.from(pair.publicKey.subarray(1, 33)); }
function p2tr(pair) { return bitcoin.payments.p2tr({ internalPubkey: internal(pair) }).output; }

// Inputs on keys 0 and 1 (one each, then one more on key 0), paying key 2
// and returning change to key 0's script.
function spend(script, version, sequence) {
  const inputs = [0, 1, 0].map((owner, index) => ({
    txid: txid(index), vout: index, value: 100_000 + 10_000 * index, owner,
    script: hex(script(keys[owner])),
  }));
  const outputs = [
    { script: hex(script(keys[2])), value: 150_000 },
    { script: hex(script(keys[0])), value: 177_000 },
  ];
  const tx = new bitcoin.Transaction();
  tx.version = version;
  for (const input of inputs) tx.addInput(Buffer.from(input.txid, 'hex').reverse(), input.vout, sequence);
  for (const output of outputs) tx.addOutput(Buffer.from(output.script, 'hex'), BigInt(output.value));
  return { inputs, outputs, tx };
}

function signature(pair, digest, hashTypeByte) {
  return Buffer.concat([bitcoin.script.signature.encode(Buffer.from(pair.sign(digest)), 0x01).subarray(0, -1), Buffer.from([hashTypeByte])]);
}

function vector(name, inputs, outputs, extra) {
  return { name, inputs: inputs.map(({ owner, ...input }) => ({ ...input, key: hex(keys[owner].privateKey) })), outputs, ...extra };
}

const bitcoinVectors = [];
for (const [name, script] of [['p2pkh', p2pkh], ['p2wpkh', p2wpkh], ['p2sh-p2wpkh', nested]]) {
  const { inputs, outputs, tx } = spend(script, 2, 0xfffffffd);
  const unsigned = tx.clone();
  inputs.forEach((input, index) => {
    const pair = keys[input.owner];
    if (name === 'p2pkh') {
      const digest = tx.hashForSignature(index, Buffer.from(input.script, 'hex'), 0x01);
      tx.setInputScript(index, bitcoin.script.compile([signature(pair, digest, 0x01), pair.publicKey]));
    } else {
      const digest = tx.hashForWitnessV0(index, p2pkh(pair), BigInt(input.value), 0x01);
      tx.setWitness(index, [signature(pair, digest, 0x01), Buffer.from(pair.publicKey)]);
      if (name === 'p2sh-p2wpkh') tx.setInputScript(index, bitcoin.script.compile([p2wpkh(pair)]));
    }
  });
  bitcoinVectors.push(vector(name, inputs, outputs, { version: 2, sequence: 0xfffffffd, unsigned: unsigned.toHex(), raw: tx.toHex(), txid: tx.getId() }));
}
{
  const { inputs, outputs, tx } = spend(p2tr, 2, 0xfffffffd);
  const scripts = inputs.map((input) => Buffer.from(input.script, 'hex'));
  const values = inputs.map((input) => BigInt(input.value));
  const sighashes = inputs.map((_, index) => hex(tx.hashForWitnessV1(index, scripts, values, bitcoin.Transaction.SIGHASH_DEFAULT)));
  bitcoinVectors.push(vector('p2tr', inputs, outputs, { version: 2, sequence: 0xfffffffd, unsigned: tx.toHex(), sighashes }));
}

// Version 1 P2PKH spends: legacy SIGHASH_ALL, or BIP143 with SIGHASH_FORKID
// (0x41) and the fork id in the hash type's upper bits.
const legacyVectors = [];
for (const [name, forkId] of [['legacy', null], ['forkid-0', 0], ['forkid-79', 79]]) {
  const { inputs, outputs, tx } = spend(p2pkh, 1, 0xffffffff);
  inputs.forEach((input, index) => {
    const pair = keys[input.owner];
    const digest = forkId === null
      ? tx.hashForSignature(index, Buffer.from(input.script, 'hex'), 0x01)
      : tx.hashForWitnessV0(index, Buffer.from(input.script, 'hex'), BigInt(input.value), (forkId << 8) | 0x41);
    tx.setInputScript(index, bitcoin.script.compile([signature(pair, digest, forkId === null ? 0x01 : 0x41), pair.publicKey]));
  });
  legacyVectors.push(vector(name, inputs, outputs, { fork_id: forkId, raw: tx.toHex(), txid: tx.getId() }));
}

// Kaspa: Schnorr P2PK scripts (`<x-only key> OP_CHECKSIG`), version 0.
function kaspaScript(pair) { return '20' + hex(internal(pair)) + 'ac'; }
const kaspaInputs = [0, 1, 0].map((owner, index) => ({
  txid: txid(index), vout: index, value: 100_000_000 + 10_000_000 * index, owner, script: kaspaScript(keys[owner]),
}));
const kaspaOutputs = [
  { script: kaspaScript(keys[2]), value: 150_000_000 },
  { script: kaspaScript(keys[0]), value: 179_990_000 },
];
const kaspaTx = new kaspa.Transaction({
  version: 0,
  inputs: kaspaInputs.map((input) => new kaspa.TransactionInput({
    previousOutpoint: new kaspa.TransactionOutpoint(new kaspa.Hash(input.txid), input.vout),
    signatureScript: '', sequence: 0n, sigOpCount: 1,
  })),
  outputs: kaspaOutputs.map((output) => new kaspa.TransactionOutput(BigInt(output.value), new kaspa.ScriptPublicKey(0, output.script))),
  lockTime: 0n, subnetworkId: '0000000000000000000000000000000000000000', gas: 0n, payload: '',
});
const entries = new kaspa.UtxoEntries(kaspaInputs.map((input) => ({
  address: kaspa.createAddress(hex(keys[input.owner].publicKey), kaspa.NetworkType.Mainnet).toString(),
  outpoint: { transactionId: input.txid, index: input.vout },
  utxoEntry: { amount: BigInt(input.value), scriptPublicKey: new kaspa.ScriptPublicKey(0, input.script), blockDaaScore: 0n, isCoinbase: false },
})));
const signable = new kaspa.SignableTransaction(kaspaTx, entries);
const kaspaSighashes = signable.getScriptHashes().map((hash) => String(hash));

fs.writeFileSync(process.argv[2], JSON.stringify({
  provenance: 'bitcoinjs-lib 7.0.2, ecpair 3.0.1, tiny-secp256k1 2.2.4, kaspa-wasm 0.13.0 (scripts/generate-account-utxo-vectors.cjs)',
  bitcoin: bitcoinVectors,
  legacy: legacyVectors,
  kaspa: vector('kaspa', kaspaInputs, kaspaOutputs, { sighashes: kaspaSighashes }),
}, null, 2) + '\n');
