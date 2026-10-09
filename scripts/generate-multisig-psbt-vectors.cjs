// Independent 2-of-3 P2WSH multisig accounts and PSBTs, for
// core/tests/fixtures/multisig-psbt.json.
// Install the pinned libraries in a temporary directory:
// npm install --prefix /tmp/spectra-multisig --ignore-scripts bitcoinjs-lib@7.0.2 tiny-secp256k1@2.2.4 bip32@5.0.1 bip39@3.1.0
// NODE_PATH=/tmp/spectra-multisig/node_modules node scripts/generate-multisig-psbt-vectors.cjs > core/tests/fixtures/multisig-psbt.json
//
// Three BIP-39 test phrases are the cosigners, each at BIP-48's
// m/48'/coin'/0'/2' (native SegWit multisig). bitcoinjs-lib derives each
// network's receive and change addresses (P2WSH of the BIP-67-sorted 2-of-3
// script), builds a PSBT spending two of them with BIP-174 key origins on
// every input and on the change output, signs it as cosigner B, and signs it
// as A and B and finalizes it, on Bitcoin and Litecoin mainnet. Litecoin
// needs no special handling beyond its network parameters: the same BIP-143
// sighash, and xpub/tpub versions for the keys, as Litecoin Core's
// descriptors use (BIP-48 coin 2 on mainnet, 1 on testnet). ECDSA is RFC 6979 and low-S on both sides,
// so the final transaction is compared byte for byte. The descriptor's
// checksum is not computed here: core's checksum is tested against BIP-380's
// own vector.
const bip39 = require('bip39');
const ecc = require('tiny-secp256k1');
const { BIP32Factory } = require('bip32');
const bitcoin = require('bitcoinjs-lib');

bitcoin.initEccLib(ecc);
const bip32 = BIP32Factory(ecc);
const PHRASES = [
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
  'legal winner thank year wave sausage worth useful legal winner thank yellow',
  'letter advice cage absurd amount doctor acoustic avoid letter advice cage above',
];
const NETWORKS = [
  ['bitcoin', bitcoin.networks.bitcoin, 0],
  ['bitcoin-testnet', bitcoin.networks.testnet, 1],
  ['litecoin', {
    messagePrefix: '\x19Litecoin Signed Message:\n', bech32: 'ltc',
    bip32: { public: 0x0488b21e, private: 0x0488ade4 }, pubKeyHash: 0x30, scriptHash: 0x32, wif: 0xb0,
  }, 2],
  ['litecoin-testnet', {
    messagePrefix: '\x19Litecoin Signed Message:\n', bech32: 'tltc',
    bip32: { public: 0x043587cf, private: 0x04358394 }, pubKeyHash: 0x6f, scriptHash: 0x3a, wif: 0xef,
  }, 1],
];
// The PSBT case's recipient per chain: BIP-173's P2WPKH example program.
const RECIPIENTS = {
  bitcoin: 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4',
  litecoin: 'ltc1qw508d6qejxtdg4y5r3zarvary0c5xw7kgmn4n9',
};
const THRESHOLD = 2;

function account(phrase, network, coin) {
  const root = bip32.fromSeed(bip39.mnemonicToSeedSync(phrase), network);
  const path = `m/48'/${coin}'/0'/2'`;
  return { root, path, node: root.derivePath(path), fingerprint: Buffer.from(root.fingerprint).toString('hex') };
}

function place(cosigners, branch, index) {
  const keys = cosigners.map((c) => ({
    pubkey: Buffer.from(c.node.derive(branch).derive(index).publicKey),
    masterFingerprint: Buffer.from(c.root.fingerprint),
    path: `${c.path}/${branch}/${index}`,
  }));
  const sorted = [...keys].sort((a, b) => Buffer.compare(a.pubkey, b.pubkey));
  return { keys, sorted };
}

function payment(sorted, network) {
  return bitcoin.payments.p2wsh({ redeem: bitcoin.payments.p2ms({ m: THRESHOLD, pubkeys: sorted.map((k) => k.pubkey), network }), network });
}

const networks = [];
for (const [chain, network, coin] of NETWORKS) {
  const cosigners = PHRASES.map((phrase) => account(phrase, network, coin));
  const addresses = [];
  for (const [branch, index] of [[0, 0], [0, 1], [0, 2], [1, 0], [1, 1]]) {
    addresses.push({ branch, index, address: payment(place(cosigners, branch, index).sorted, network).address });
  }
  const entry = {
    chain,
    threshold: THRESHOLD,
    cosigners: cosigners.map((c) => ({
      fingerprint: c.fingerprint,
      origin: c.path.slice(2).replaceAll("'", 'h'),
      xpub: c.node.neutered().toBase58(),
    })),
    addresses,
  };
  if (RECIPIENTS[chain]) {
    // Two inputs, at 0/0 and 1/0; a recipient and change to 1/1.
    const inputs = [
      { txid: '11'.repeat(32), vout: 0, value: 100000n, place: [0, 0] },
      { txid: '22'.repeat(32), vout: 3, value: 50000n, place: [1, 0] },
    ];
    const recipient = { address: RECIPIENTS[chain], value: 120000n };
    const change = { place: [1, 1], value: 29000n };
    const psbt = new bitcoin.Psbt({ network });
    psbt.setVersion(2);
    psbt.setLocktime(0);
    for (const input of inputs) {
      const { keys, sorted } = place(cosigners, ...input.place);
      const pay = payment(sorted, network);
      psbt.addInput({
        hash: input.txid,
        index: input.vout,
        sequence: 0xfffffffd,
        witnessUtxo: { script: pay.output, value: input.value },
        witnessScript: pay.redeem.output,
        bip32Derivation: keys,
      });
    }
    psbt.addOutput({ address: recipient.address, value: recipient.value });
    const changeKeys = place(cosigners, ...change.place);
    const changePay = payment(changeKeys.sorted, network);
    psbt.addOutput({
      script: changePay.output,
      value: change.value,
      witnessScript: changePay.redeem.output,
      bip32Derivation: changeKeys.keys,
    });
    const unsigned = psbt.toBase64();
    const byB = bitcoin.Psbt.fromBase64(unsigned, { network });
    byB.signAllInputsHD(cosigners[1].root);
    const both = bitcoin.Psbt.fromBase64(byB.toBase64(), { network });
    both.signAllInputsHD(cosigners[0].root);
    both.finalizeAllInputs();
    const tx = both.extractTransaction();
    entry.psbt = {
      inputs: inputs.map((i) => ({ txid: i.txid, vout: i.vout, value: Number(i.value), branch: i.place[0], index: i.place[1] })),
      recipient: { address: recipient.address, value: Number(recipient.value) },
      change: { branch: change.place[0], index: change.place[1], value: Number(change.value) },
      fee: 1000,
      unsigned,
      unsigned_tx: Buffer.from(psbt.data.globalMap.unsignedTx.toBuffer()).toString('hex'),
      signed_by_b: byB.toBase64(),
      final_a_b: tx.toHex(),
      txid: tx.getId(),
    };
  }
  networks.push(entry);
}
console.log(JSON.stringify({
  provenance: 'bitcoinjs-lib 7.0.2, bip32 5.0.1, bip39 3.1.0, tiny-secp256k1 2.2.4 (scripts/generate-multisig-psbt-vectors.cjs)',
  phrases: PHRASES,
  networks,
}, null, 2));
