// Independent 2-of-3 P2SH multisig accounts and spends for Bitcoin Cash and
// Dogecoin, for core/tests/fixtures/p2sh-multisig.json.
// Install the pinned libraries in a temporary directory:
// npm install --prefix /tmp/spectra-p2sh-multisig --ignore-scripts bitcoinjs-lib@7.0.2 tiny-secp256k1@2.2.4 bip32@5.0.1 bip39@3.1.0 bitcore-lib-cash@10.10.5 ecash-lib@4.12.0
// NODE_PATH=/tmp/spectra-p2sh-multisig/node_modules node scripts/generate-p2sh-multisig-vectors.cjs > core/tests/fixtures/p2sh-multisig.json
//
// Three BIP-39 test phrases are the cosigners, each at m/44'/coin'/0' (what
// Paytaca's BSMS export uses for Bitcoin Cash), coin 145 for Bitcoin Cash, 3
// for Dogecoin and 1 for both testnets. Account keys are written with
// Bitcoin's xpub/tpub versions; Dogecoin mainnet's dgub form is recorded too.
// Every place's redeem script is OP_2 <the three child keys sorted, BIP-67>
// OP_3 OP_CHECKMULTISIG.
//
// bitcoinjs-lib derives the keys, scripts and base58 addresses and computes
// both sighashes: the BIP-143-style SIGHASH_FORKID digest (hash type 0x41)
// for Bitcoin Cash and the legacy SIGHASH_ALL digest for Dogecoin, each with
// the redeem script as scriptCode; tiny-secp256k1 signs them (RFC 6979,
// low S). bitcore-lib-cash builds the same transactions, recomputes every
// sighash, signs every input itself (RFC 6979, low S) and must produce the
// same signatures and finished transactions, and its script interpreter
// verifies every finished input; its CashAddr encoding is checked against
// ecashaddrjs. ecash-lib (eCash keeps Bitcoin ABC's PSBT layout, which BCHN
// shares) writes the Bitcoin Cash PSBTs, which must match byte for byte the
// ones this script assembles from BCHN's src/psbt.h key layout; it also signs
// as P1 with its own FORKID sighash (its deterministic nonce is not RFC
// 6979's, so its signatures are only verified against the recorded digests)
// and finalizes P0's and P1's partial signatures into the recorded
// transaction.
//
// Bitcoin Cash PSBTs are BCHN's: input key 0x00 holds a serialized CTxOut,
// 0x02 partial signatures, 0x04 the redeem script and 0x06 BIP-32
// derivations; output key 0x00 the redeem script and 0x02 derivations. No
// 0x03 sighash entry is written, as BCHN's walletprocesspsbt leaves it unset.
// A second PSBT holds a whole previous transaction under input key 0x00 (as
// BIP-174's non-witness UTXO does), which BCHN itself cannot read: it is for
// lenient reading only, and ecash-lib re-encodes it with CTxOut values.
//
// Dogecoin 1.14 has no PSBT; partially signed transactions travel as raw hex
// in the form signrawtransaction writes: one signer gives
// OP_0 <sig> <redeemScript>, and CombineMultisig gives
// OP_0 <sigs in redeem-script key order, at most m> [OP_0 up to m] <redeemScript>.
// signed_by_p1_padded is the second form with one signature: what a node that
// knows the redeem script but holds none of its keys makes of P1's
// transaction when it signs it.
const crypto = require('node:crypto');
const bip39 = require('bip39');
const ecc = require('tiny-secp256k1');
const { BIP32Factory } = require('bip32');
const bitcoin = require('bitcoinjs-lib');
const bitcore = require('bitcore-lib-cash');
const ecash = require('ecash-lib');
const cashaddr = require('ecashaddrjs');

bitcoin.initEccLib(ecc);
const bip32 = BIP32Factory(ecc);
const hex = (bytes) => Buffer.from(bytes).toString('hex');
const PHRASES = [
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
  'legal winner thank year wave sausage worth useful legal winner thank yellow',
  'letter advice cage absurd amount doctor acoustic avoid letter advice cage above',
];
const THRESHOLD = 2;
const XPUB = { public: 0x0488b21e, private: 0x0488ade4 };
const TPUB = { public: 0x043587cf, private: 0x04358394 };
const DGUB = { public: 0x02facafd, private: 0x02fac398 };
const network = (bip32Versions, pubKeyHash, scriptHash, wif) => ({
  messagePrefix: '', bech32: '', bip32: bip32Versions, pubKeyHash, scriptHash, wif,
});
const NETWORKS = [
  { chain: 'bitcoin-cash', coin: 145, network: network(XPUB, 0x00, 0x05, 0x80), cashaddr: 'bitcoincash', bitcore: 'livenet' },
  { chain: 'bitcoin-cash-testnet', coin: 1, network: network(TPUB, 0x6f, 0xc4, 0xef), cashaddr: 'bchtest', bitcore: 'testnet' },
  { chain: 'dogecoin', coin: 3, network: network(XPUB, 0x1e, 0x16, 0x9e), dgub: network(DGUB, 0x1e, 0x16, 0x9e) },
  { chain: 'dogecoin-testnet', coin: 1, network: network(TPUB, 0x71, 0xc4, 0xf1) },
];
const PLACES = [[0, 0], [0, 1], [0, 2], [1, 0], [1, 1]];
const SEQUENCE = 0xffffffff;
const FORKID_ALL = 0x41;
const ALL = 0x01;

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function account(phrase, net) {
  const seed = bip39.mnemonicToSeedSync(phrase);
  const root = bip32.fromSeed(seed, net.network);
  const path = `m/44'/${net.coin}'/0'`;
  const cosigner = {
    root, path, node: root.derivePath(path), fingerprint: hex(root.fingerprint),
  };
  if (net.dgub) cosigner.dgub = bip32.fromSeed(seed, net.dgub).derivePath(path).neutered().toBase58();
  return cosigner;
}

function hardened(index) { return (index | 0x80000000) >>> 0; }

// The three keys at account/branch/index, in cosigner order and in the
// redeem script's (BIP-67) order.
function place(net, cosigners, branch, index) {
  const keys = cosigners.map((c, cosigner) => {
    const child = c.node.derive(branch).derive(index);
    return {
      cosigner,
      pubkey: Buffer.from(child.publicKey),
      privateKey: Buffer.from(child.privateKey),
      fingerprint: Buffer.from(c.root.fingerprint),
      path: [hardened(44), hardened(net.coin), hardened(0), branch, index],
    };
  });
  const sorted = [...keys].sort((a, b) => Buffer.compare(a.pubkey, b.pubkey));
  const redeem = bitcoin.payments.p2ms({ m: THRESHOLD, pubkeys: sorted.map((k) => k.pubkey), network: net.network });
  const p2sh = bitcoin.payments.p2sh({ redeem, network: net.network });
  // bitcore-lib-cash sorts the keys itself and must reach the same script.
  const bitcoreRedeem = bitcore.Script.buildMultisigOut(keys.map((k) => new bitcore.PublicKey(k.pubkey)), THRESHOLD);
  check(hex(bitcoreRedeem.toBuffer()) === hex(redeem.output), 'redeem script mismatch');
  return {
    keys, sorted, redeem: Buffer.from(redeem.output), scriptPubKey: Buffer.from(p2sh.output),
    hash: Buffer.from(p2sh.hash), address: p2sh.address,
  };
}

function cashAddress(net, p) {
  const address = bitcore.Address.payingTo(new bitcore.Script(p.redeem), net.bitcore).toCashAddress();
  check(address === cashaddr.encodeCashAddress(net.cashaddr, 'p2sh', hex(p.hash)), 'cashaddr mismatch');
  return address;
}

function addressEntry(net, cosigners, branch, index) {
  const p = place(net, cosigners, branch, index);
  const entry = { branch, index, address: p.address };
  if (net.cashaddr) entry.cashaddr = cashAddress(net, p);
  entry.redeem_script = hex(p.redeem);
  return entry;
}

// DER signature with the sighash byte appended (bitcoinjs only encodes
// Bitcoin's defined hash types, so the byte is replaced afterwards).
function derSignature(privateKey, digest, hashType) {
  const der = bitcoin.script.signature.encode(Buffer.from(ecc.sign(digest, privateKey)), 0x01);
  return Buffer.concat([der.subarray(0, -1), Buffer.from([hashType])]);
}

// --- BCHN PSBT layout (src/psbt.h), assembled by hand ---------------------

function compactSize(n) {
  if (n < 0xfd) return Buffer.from([n]);
  if (n <= 0xffff) { const b = Buffer.alloc(3); b[0] = 0xfd; b.writeUInt16LE(n, 1); return b; }
  const b = Buffer.alloc(5); b[0] = 0xfe; b.writeUInt32LE(n, 1); return b;
}
function pair(key, value) { return Buffer.concat([compactSize(key.length), key, compactSize(value.length), value]); }
function ctxout(value, script) {
  const amount = Buffer.alloc(8);
  amount.writeBigUInt64LE(BigInt(value));
  return Buffer.concat([amount, compactSize(script.length), script]);
}
function keyOrigin(key) {
  const path = Buffer.alloc(4 * key.path.length);
  key.path.forEach((step, i) => path.writeUInt32LE(step, 4 * i));
  return Buffer.concat([key.fingerprint, path]);
}
const hash160 = (bytes) => crypto.createHash('ripemd160').update(crypto.createHash('sha256').update(bytes).digest()).digest();

// inputs: [{ utxo, partialSigs: [{pubkey, signature}], redeem, keys }]
// outputs: [{ redeem, keys }]. Partial signatures are a map keyed by CKeyID
// (HASH160 of the key) and derivations a map keyed by CPubKey, so each is
// written in that order.
function bchnPsbt(unsignedTx, inputs, outputs) {
  const parts = [Buffer.from('psbt\xff', 'latin1'), pair(Buffer.from([0x00]), unsignedTx), Buffer.from([0x00])];
  for (const input of inputs) {
    if (input.utxo) parts.push(pair(Buffer.from([0x00]), input.utxo));
    const sigs = [...input.partialSigs].sort((a, b) => Buffer.compare(hash160(a.pubkey), hash160(b.pubkey)));
    for (const { pubkey, signature } of sigs) parts.push(pair(Buffer.concat([Buffer.from([0x02]), pubkey]), signature));
    parts.push(pair(Buffer.from([0x04]), input.redeem));
    for (const key of [...input.keys].sort((a, b) => Buffer.compare(a.pubkey, b.pubkey))) {
      parts.push(pair(Buffer.concat([Buffer.from([0x06]), key.pubkey]), keyOrigin(key)));
    }
    parts.push(Buffer.from([0x00]));
  }
  for (const output of outputs) {
    if (output.redeem) parts.push(pair(Buffer.from([0x00]), output.redeem));
    for (const key of [...(output.keys || [])].sort((a, b) => Buffer.compare(a.pubkey, b.pubkey))) {
      parts.push(pair(Buffer.concat([Buffer.from([0x02]), key.pubkey]), keyOrigin(key)));
    }
    parts.push(Buffer.from([0x00]));
  }
  return Buffer.concat(parts);
}

// The same PSBT written by ecash-lib. Derivations, and a whole previous
// transaction under input key 0x00, go in as pairs ecash-lib keeps verbatim.
function ecashPsbt(unsignedTx, inputs, outputs) {
  const derivations = (type, keys) => keys.map((key) => ({
    key: new Uint8Array(Buffer.concat([Buffer.from([type]), key.pubkey])),
    value: new Uint8Array(keyOrigin(key)),
  }));
  return new ecash.Psbt({
    unsignedTx: ecash.Tx.deser(new Uint8Array(unsignedTx)),
    signDataPerInput: inputs.map((i) => ({ sats: BigInt(i.value), redeemScript: new ecash.Script(new Uint8Array(i.redeem)) })),
    inputPartialSigs: inputs.map(() => new Map()),
    unknownInputPairs: inputs.map((i) => [
      ...(i.previousTx ? [{ key: new Uint8Array([0x00]), value: new Uint8Array(i.previousTx) }] : []),
      ...derivations(0x06, i.keys),
    ]),
    unknownOutputPairs: outputs.map((o) => (o.redeem
      ? [{ key: new Uint8Array([0x00]), value: new Uint8Array(o.redeem) }, ...derivations(0x02, o.keys)]
      : [])),
    inputWitnessIncomplete: inputs.map((i) => Boolean(i.previousTx)),
  });
}

// --- Transactions ----------------------------------------------------------

// bitcore's SCRIPT_VERIFY_LOW_S check strips the sighash byte twice and
// rejects every signature, so low S is checked on each signature instead.
const FLAGS = bitcore.Script.Interpreter;
const BITCORE_FLAGS = {
  bch: FLAGS.SCRIPT_VERIFY_P2SH | FLAGS.SCRIPT_VERIFY_STRICTENC | FLAGS.SCRIPT_VERIFY_DERSIG | FLAGS.SCRIPT_VERIFY_NULLDUMMY
    | FLAGS.SCRIPT_VERIFY_NULLFAIL | FLAGS.SCRIPT_VERIFY_CLEANSTACK | FLAGS.SCRIPT_VERIFY_SIGPUSHONLY
    | FLAGS.SCRIPT_ENABLE_SIGHASH_FORKID,
  doge: FLAGS.SCRIPT_VERIFY_P2SH | FLAGS.SCRIPT_VERIFY_STRICTENC | FLAGS.SCRIPT_VERIFY_DERSIG | FLAGS.SCRIPT_VERIFY_NULLDUMMY
    | FLAGS.SCRIPT_VERIFY_NULLFAIL,
};

// bitcore's network only affects how it prints the key, never a signature.
const bitcoreKey = (net, key) => new bitcore.PrivateKey(hex(key.privateKey), net.bitcore || 'livenet');

// spec: { version, hashType, inputs: [{txid, vout, value, place}], recipient, change }
function spend(net, cosigners, spec) {
  const forkid = spec.hashType === FORKID_ALL;
  const inputs = spec.inputs.map((input) => ({ ...input, ...place(net, cosigners, ...input.place) }));
  const change = { ...spec.change, ...place(net, cosigners, ...spec.change.place) };
  const recipientScript = Buffer.from(bitcoin.address.toOutputScript(spec.recipient.address, net.network));
  check(inputs.reduce((sum, i) => sum + i.value, 0) - spec.recipient.value - change.value === spec.fee, 'fee');

  const tx = new bitcoin.Transaction();
  tx.version = spec.version;
  tx.locktime = 0;
  for (const input of inputs) tx.addInput(Buffer.from(input.txid, 'hex').reverse(), input.vout, SEQUENCE);
  tx.addOutput(recipientScript, BigInt(spec.recipient.value));
  tx.addOutput(change.scriptPubKey, BigInt(change.value));
  const unsigned = tx.clone();

  // bitcore-lib-cash builds the same transaction and signs as P0 and P1.
  const btx = new bitcore.Transaction();
  for (const input of inputs) {
    btx.from({ txId: input.txid, outputIndex: input.vout, satoshis: input.value, script: hex(input.scriptPubKey) },
      input.keys.map((k) => hex(k.pubkey)), THRESHOLD);
  }
  btx.addOutput(new bitcore.Transaction.Output({ script: new bitcore.Script(recipientScript), satoshis: spec.recipient.value }));
  btx.addOutput(new bitcore.Transaction.Output({ script: new bitcore.Script(change.scriptPubKey), satoshis: change.value }));
  btx.version = spec.version;
  btx.nLockTime = 0;
  check(btx.uncheckedSerialize() === unsigned.toHex(), 'bitcore unsigned transaction mismatch');

  const signatures = inputs.map((input, index) => {
    const digest = forkid
      ? tx.hashForWitnessV0(index, input.redeem, BigInt(input.value), spec.hashType)
      : tx.hashForSignature(index, input.redeem, spec.hashType);
    const bitcoreDigest = bitcore.Transaction.Sighash.sighash(btx, spec.hashType, index, new bitcore.Script(input.redeem),
      new bitcore.crypto.BN(input.value), forkid ? undefined : 0);
    check(hex(Buffer.from(bitcoreDigest).reverse()) === hex(digest), `sighash mismatch on input ${index}`);
    // P0 and P1 sign every input; P2 signs input 0 too.
    const signers = input.keys.filter((k) => k.cosigner < 2 || index === 0);
    return {
      digest,
      sigs: signers.map((key) => {
        const signature = derSignature(key.privateKey, digest, spec.hashType);
        const [theirs] = btx.getSignatures(bitcoreKey(net, key), spec.hashType, 'ecdsa').filter((s) => s.inputIndex === index);
        check(hex(Buffer.concat([theirs.signature.toDER('ecdsa'), Buffer.from([theirs.sigtype])])) === hex(signature),
          `bitcore signature mismatch on input ${index}`);
        check(bitcore.crypto.Signature.fromTxFormat(signature).hasLowS(), `high S on input ${index}`);
        return { key, signature };
      }),
    };
  });

  // scriptSig in redeem-script key order; `pad` fills empty slots up to m.
  const scriptSig = (input, index, cosignerSet, pad) => {
    const sigs = input.sorted.flatMap((key) => (cosignerSet.includes(key.cosigner)
      ? [signatures[index].sigs.find((s) => s.key.cosigner === key.cosigner).signature] : [])).slice(0, THRESHOLD);
    const padding = pad ? Array(THRESHOLD - sigs.length).fill(bitcoin.opcodes.OP_0) : [];
    return bitcoin.script.compile([bitcoin.opcodes.OP_0, ...sigs, ...padding, input.redeem]);
  };
  const signedWith = (cosignerSet, pad) => {
    const signed = unsigned.clone();
    inputs.forEach((input, index) => signed.setInputScript(index, scriptSig(input, index, cosignerSet, pad)));
    return signed;
  };
  const final = signedWith([0, 1], false);

  for (const cosigner of [0, 1]) {
    btx.sign(inputs.map((input) => bitcoreKey(net, input.keys[cosigner])), spec.hashType, 'ecdsa');
  }
  check(btx.isFullySigned(), 'bitcore transaction not fully signed');
  check(btx.uncheckedSerialize() === final.toHex(), 'bitcore final transaction mismatch');
  inputs.forEach((input, index) => {
    const interpreter = new bitcore.Script.Interpreter();
    const ok = interpreter.verify(new bitcore.Script(Buffer.from(final.ins[index].script)), new bitcore.Script(input.scriptPubKey),
      new bitcore.Transaction(final.toHex()), index, forkid ? BITCORE_FLAGS.bch : BITCORE_FLAGS.doge, new bitcore.crypto.BN(input.value));
    check(ok, `bitcore interpreter rejects input ${index}: ${interpreter.errstr}`);
  });

  return { inputs, change, recipientScript, unsigned, signatures, signedWith, final };
}

function transactionEntry(spec, s) {
  return {
    version: spec.version,
    locktime: 0,
    sequence: SEQUENCE,
    sighash_type: spec.hashType,
    inputs: s.inputs.map((input, index) => ({
      txid: input.txid,
      vout: input.vout,
      value: input.value,
      branch: input.place[0],
      index: input.place[1],
      redeem_script: hex(input.redeem),
      script_pubkey: hex(input.scriptPubKey),
      script_order: input.sorted.map((k) => k.cosigner),
      sighash: hex(s.signatures[index].digest),
      signatures: s.signatures[index].sigs.map(({ key, signature }) => ({
        cosigner: key.cosigner, pubkey: hex(key.pubkey), signature: hex(signature),
      })),
    })),
    recipient: { address: spec.recipient.address, script_pubkey: hex(s.recipientScript), value: spec.recipient.value },
    change: {
      branch: s.change.place[0], index: s.change.place[1], value: s.change.value,
      address: s.change.address, script_pubkey: hex(s.change.scriptPubKey),
    },
    fee: spec.fee,
    unsigned_tx: s.unsigned.toHex(),
  };
}

const p1Signature = (s, index) => s.signatures[index].sigs.find((x) => x.key.cosigner === 1);

// Unsigned and P1-signed BCHN PSBTs for a spend, from both writers.
function bchPsbts(s, previousTxs) {
  const unsignedBytes = Buffer.from(s.unsigned.toBuffer());
  const inputs = s.inputs.map((input, index) => ({
    value: input.value,
    utxo: previousTxs ? previousTxs[index] : ctxout(input.value, input.scriptPubKey),
    previousTx: previousTxs ? previousTxs[index] : undefined,
    redeem: input.redeem,
    keys: input.keys,
    partialSigs: [],
  }));
  const outputs = [{}, { redeem: s.change.redeem, keys: s.change.keys }];
  const unsigned = bchnPsbt(unsignedBytes, inputs, outputs);
  const fromEcash = ecashPsbt(unsignedBytes, inputs, outputs);
  check(hex(fromEcash.toBytes()) === hex(unsigned), 'ecash-lib unsigned PSBT mismatch');
  const parsed = ecash.Psbt.fromBytes(new Uint8Array(unsigned));
  s.inputs.forEach((input, index) => {
    check(!parsed.inputWitnessIncomplete[index] && parsed.signDataPerInput[index].sats === BigInt(input.value),
      `ecash-lib does not read input ${index}'s value`);
  });
  const ctxoutForm = bchnPsbt(unsignedBytes, inputs.map((i, index) => ({ ...i, utxo: ctxout(i.value, s.inputs[index].scriptPubKey) })), outputs);
  check(hex(parsed.toBytes()) === hex(ctxoutForm), 'ecash-lib does not re-encode with CTxOut values');
  if (previousTxs) return { unsigned, ctxoutForm };

  // P1 signs through ecash-lib, which hashes each input itself. Its ECDSA
  // nonce is deterministic but not RFC 6979, so its signatures are checked
  // against bitcoinjs's digests and the PSBT written carries the RFC 6979
  // signatures every other library agrees on.
  let signedByEcash = fromEcash;
  s.inputs.forEach((input, index) => {
    signedByEcash = signedByEcash.addMultisigSignatureFromKey({
      inputIdx: index,
      sk: new Uint8Array(input.keys[1].privateKey),
      signData: signedByEcash.signDataPerInput[index],
    });
  });
  s.inputs.forEach((input, index) => {
    const sigs = signedByEcash.inputPartialSigs[index];
    const theirs = Buffer.from(sigs.get(hex(input.keys[1].pubkey)) || []);
    check(sigs.size === 1 && theirs.at(-1) === FORKID_ALL, `ecash-lib partial signature on input ${index}`);
    const { signature } = bitcoin.script.signature.decode(Buffer.concat([theirs.subarray(0, -1), Buffer.from([0x01])]));
    check(ecc.verify(s.signatures[index].digest, input.keys[1].pubkey, signature), `ecash-lib sighash mismatch on input ${index}`);
  });
  const withSigs = (cosigners) => new ecash.Psbt({
    unsignedTx: signedByEcash.unsignedTx,
    signDataPerInput: signedByEcash.signDataPerInput,
    inputPartialSigs: s.inputs.map((input, index) => new Map(cosigners.map((cosigner) => [
      hex(input.keys[cosigner].pubkey),
      new Uint8Array(s.signatures[index].sigs.find((x) => x.key.cosigner === cosigner).signature),
    ]))),
    unknownInputPairs: signedByEcash.unknownInputPairs,
    unknownOutputPairs: signedByEcash.unknownOutputPairs,
    inputWitnessIncomplete: signedByEcash.inputWitnessIncomplete,
  });
  const signedByP1 = bchnPsbt(unsignedBytes, inputs.map((i, index) => ({
    ...i, partialSigs: [{ pubkey: s.inputs[index].keys[1].pubkey, signature: p1Signature(s, index).signature }],
  })), outputs);
  check(hex(withSigs([1]).toBytes()) === hex(signedByP1), 'ecash-lib P1-signed PSBT mismatch');
  // ecash-lib finalizes P0's and P1's partial signatures into the same transaction.
  check(hex(withSigs([0, 1]).toTx().ser()) === s.final.toHex(), 'ecash-lib final transaction mismatch');
  return { unsigned, signedByP1 };
}

// A previous transaction paying `value` to `script` at output `vout`, with
// small payments to `filler` in the outputs before it.
function previousTransaction(seed, vout, value, script, filler) {
  const tx = new bitcoin.Transaction();
  tx.version = 2;
  tx.addInput(Buffer.from(seed.repeat(32), 'hex'), 0, SEQUENCE, bitcoin.script.compile([Buffer.from(seed.repeat(8), 'hex')]));
  for (let i = 0; i < vout; i += 1) tx.addOutput(filler, BigInt(1000 * (i + 1)));
  tx.addOutput(script, BigInt(value));
  return tx;
}

const SPENDS = {
  'bitcoin-cash': { version: 2, hashType: FORKID_ALL, recipient: '1BoatSLRHtKNngkdXEeobR76b53LETtpyT' },
  dogecoin: { version: 1, hashType: ALL, recipient: 'DH5yaieqoZN36fDVciNyRueRGvGLR3mr7L' },
};

const networks = [];
for (const net of NETWORKS) {
  const cosigners = PHRASES.map((phrase) => account(phrase, net));
  const entry = {
    chain: net.chain,
    coin: net.coin,
    threshold: THRESHOLD,
    cosigners: cosigners.map((c) => ({
      fingerprint: c.fingerprint,
      origin: c.path.slice(2).replaceAll("'", 'h'),
      xpub: c.node.neutered().toBase58(),
      ...(c.dgub ? { dgub: c.dgub } : {}),
    })),
    addresses: PLACES.map(([branch, index]) => addressEntry(net, cosigners, branch, index)),
  };
  const kind = SPENDS[net.chain];
  if (kind) {
    const spec = {
      version: kind.version,
      hashType: kind.hashType,
      inputs: [
        { txid: '11'.repeat(32), vout: 0, value: 100000000, place: [0, 0] },
        { txid: '22'.repeat(32), vout: 3, value: 50000000, place: [1, 0] },
      ],
      recipient: { address: kind.recipient, value: 120000000 },
      change: { place: [1, 1], value: 29000000 },
      fee: 1000000,
    };
    const s = spend(net, cosigners, spec);
    const transaction = transactionEntry(spec, s);
    if (kind.hashType === FORKID_ALL) {
      const psbts = bchPsbts(s);
      transaction.psbt = { unsigned: psbts.unsigned.toString('base64'), signed_by_p1: psbts.signedByP1.toString('base64') };
      transaction.final_p0_p1 = s.final.toHex();
      transaction.txid = s.final.getId();

      // The same spend from previous transactions given whole.
      const recipientScript = s.recipientScript;
      const previous = spec.inputs.map((input, index) => previousTransaction(index === 0 ? 'aa' : 'bb', input.vout, input.value,
        s.inputs[index].scriptPubKey, recipientScript));
      const fullSpec = { ...spec, inputs: spec.inputs.map((input, index) => ({ ...input, txid: previous[index].getId() })) };
      const full = spend(net, cosigners, fullSpec);
      const fullPsbts = bchPsbts(full, previous.map((tx) => Buffer.from(tx.toBuffer())));
      const fullEntry = transactionEntry(fullSpec, full);
      transaction.full_previous = {
        previous_transactions: previous.map((tx) => ({ txid: tx.getId(), hex: tx.toHex() })),
        inputs: fullEntry.inputs,
        unsigned_tx: fullEntry.unsigned_tx,
        psbt_unsigned: fullPsbts.unsigned.toString('base64'),
        psbt_unsigned_ctxout: fullPsbts.ctxoutForm.toString('base64'),
        final_p0_p1: full.final.toHex(),
        txid: full.final.getId(),
      };
    } else {
      transaction.signed_by_p1 = s.signedWith([1], false).toHex();
      transaction.signed_by_p0 = s.signedWith([0], false).toHex();
      transaction.signed_by_p1_padded = s.signedWith([1], true).toHex();
      transaction.final_p0_p1 = s.final.toHex();
      transaction.txid = s.final.getId();
    }
    entry.transaction = transaction;
  }
  networks.push(entry);
}

console.log(JSON.stringify({
  provenance: 'bitcoinjs-lib 7.0.2, bip32 5.0.1, bip39 3.1.0, tiny-secp256k1 2.2.4: keys, scripts, base58 addresses, both '
    + 'sighashes, RFC 6979 signatures and Dogecoin scriptSigs; bitcore-lib-cash 10.10.5: CashAddr, every sighash, signature '
    + 'and finished transaction recomputed and its interpreter run on every finished input; ecashaddrjs 2.0.1: CashAddr '
    + 'cross-check; ecash-lib 4.12.0: wrote and re-read the Bitcoin Cash PSBTs, matched byte for byte against BCHN '
    + 'src/psbt.h (abd433abe04f74780744b9eac06731f3690ce68a) as assembled in scripts/generate-p2sh-multisig-vectors.cjs, '
    + 'signed as P1 against the same digests and finalized P0+P1 into the same transaction. Dogecoin scriptSigs follow '
    + 'Dogecoin Core 1.14 signrawtransaction (SignN, CombineMultisig) by construction, not by running it.',
  phrases: PHRASES,
  networks,
}, null, 2));
