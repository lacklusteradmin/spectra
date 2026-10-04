// Independent XRP signing fixture; install the pinned official XRPL packages:
// npm install --prefix /tmp/spectra-xrp-vectors --ignore-scripts ripple-binary-codec@2.7.0 ripple-keypairs@2.0.0 @scure/bip32@1.7.0 @scure/bip39@1.6.0
// NODE_PATH=/tmp/spectra-xrp-vectors/node_modules node scripts/generate-xrp-payment-vector.cjs
const fs = require('node:fs');
const codec = require('ripple-binary-codec');
const keypairs = require('ripple-keypairs');
const { HDKey } = require('@scure/bip32');
const { mnemonicToSeedSync } = require('@scure/bip39');
function paymentVector(publicKey, privateKey) {
  const transaction = {
    TransactionType: 'Payment',
    Flags: 0,
    Sequence: 7,
    Amount: '123456789',
    Fee: '12',
    SigningPubKey: publicKey,
    Account: keypairs.deriveAddress(publicKey),
    Destination: 'rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe',
  };
  const signingHex = codec.encodeForSigning(transaction);
  transaction.TxnSignature = keypairs.sign(signingHex, privateKey);
  return {
    provenance: 'ripple-binary-codec 2.7.0, ripple-keypairs 2.0.0',
    transaction,
    signing_hex: signingHex,
    signed_hex: codec.encode(transaction),
  };
}
const fixture = paymentVector(
  '0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798',
  '00' + '0'.repeat(63) + '1',
);
fs.writeFileSync('core/tests/fixtures/xrp-payment.json', JSON.stringify(fixture, null, 2) + '\n');
const mnemonic = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const path = "m/44'/144'/0'/0/0";
const derived = HDKey.fromMasterSeed(mnemonicToSeedSync(mnemonic)).derive(path);
const mnemonicFixture = paymentVector(
  Buffer.from(derived.publicKey).toString('hex').toUpperCase(),
  '00' + Buffer.from(derived.privateKey).toString('hex').toUpperCase(),
);
mnemonicFixture.provenance += ', @scure/bip32 1.7.0, @scure/bip39 1.6.0';
mnemonicFixture.mnemonic = mnemonic;
mnemonicFixture.derivation_path = path;
fs.writeFileSync('core/tests/fixtures/xrp-mnemonic-payment.json', JSON.stringify(mnemonicFixture, null, 2) + '\n');
