// Issued assets as each network's own SDK builds and signs them: XRP Ledger
// issued-currency payments (with SendMax for an issuer's transfer rate) and
// TrustSet, rippled's amount encoding, and Stellar credit-asset payments and
// ChangeTrust.
// npm install --prefix /tmp/spectra-issued-vectors --ignore-scripts ripple-binary-codec@2.11.0 ripple-keypairs@3.1.0 @stellar/stellar-base@14.1.0
// NODE_PATH=/tmp/spectra-issued-vectors/node_modules node scripts/generate-issued-asset-vectors.cjs
const fs = require('node:fs');
const codec = require('ripple-binary-codec');
const keypairs = require('ripple-keypairs');
const stellar = require('@stellar/stellar-base');

const ISSUER = 'rhub8VRN55s94qWKDv6jmDy1pUykJzF3wq';
const DESTINATION = 'rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe';
const SOLO = '534F4C4F00000000000000000000000000000000';

function xrpl() {
  const publicKey = '0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798';
  const privateKey = '00' + '0'.repeat(63) + '1';
  const account = keypairs.deriveAddress(publicKey);
  const sign = transaction => {
    const signingHex = codec.encodeForSigning(transaction);
    transaction.TxnSignature = keypairs.sign(signingHex, privateKey);
    return { transaction, signing_hex: signingHex, signed_hex: codec.encode(transaction) };
  };
  const base = (type, sequence, flags) => ({
    TransactionType: type, Flags: flags, Sequence: sequence, Fee: '12', SigningPubKey: publicKey, Account: account,
  });
  const values = ['1', '-1', '0', '123.456', '0.000001', '1e-81', '9999999999999999e80', '1234567890123456', '100.2'];
  return {
    key: privateKey.slice(2),
    account,
    issuer: ISSUER,
    amounts: values.map(value => ({
      value,
      hex: codec.coreTypes.Amount.from({ currency: 'USD', issuer: ISSUER, value }).toHex(),
    })),
    payments: [
      sign({ ...base('Payment', 7, 0), Destination: DESTINATION,
        Amount: { currency: 'USD', issuer: ISSUER, value: '123.456' },
        SendMax: { currency: 'USD', issuer: ISSUER, value: '123.702912' } }),
      sign({ ...base('Payment', 8, 0), Destination: DESTINATION,
        Amount: { currency: SOLO, issuer: ISSUER, value: '0.5' } }),
    ],
    trust_sets: [
      sign({ ...base('TrustSet', 9, 0x00020000),
        LimitAmount: { currency: 'USD', issuer: ISSUER, value: '9999999999999999e80' } }),
      sign({ ...base('TrustSet', 10, 0x00020000),
        LimitAmount: { currency: SOLO, issuer: ISSUER, value: '0' } }),
    ],
  };
}

function stellarVectors() {
  const seed = Buffer.alloc(32, 7);
  const source = stellar.Keypair.fromRawEd25519Seed(seed);
  const destination = stellar.Keypair.fromRawEd25519Seed(Buffer.alloc(32, 9)).publicKey();
  const issuer = stellar.Keypair.fromRawEd25519Seed(Buffer.alloc(32, 11)).publicKey();
  const passphrase = stellar.Networks.TESTNET;
  const { xdr } = stellar;
  // Built from XDR types: Spectra's transactions carry no time bounds, which
  // the SDK's builder always adds.
  const envelope = (sequence, operation) => {
    const body = new xdr.Transaction({
      sourceAccount: stellar.decodeAddressToMuxedAccount(source.publicKey()),
      fee: 100,
      seqNum: xdr.SequenceNumber.fromString(sequence),
      cond: xdr.Preconditions.precondNone(),
      memo: stellar.Memo.none().toXDRObject(),
      operations: [operation],
      ext: new xdr.TransactionExt(0),
    });
    const transaction = new stellar.Transaction(
      xdr.TransactionEnvelope.envelopeTypeTx(new xdr.TransactionV1Envelope({ tx: body, signatures: [] })),
      passphrase);
    transaction.sign(source);
    return transaction.toEnvelope().toXDR('base64');
  };
  const usdc = new stellar.Asset('USDC', issuer);
  const long = new stellar.Asset('LONGASSET12', issuer);
  return {
    seed: seed.toString('hex'),
    source: source.publicKey(),
    destination,
    issuer,
    network_passphrase: passphrase,
    payments: [
      { asset: `USDC:${issuer}`, amount: '12.3456789', sequence: '123456789013',
        envelope_b64: envelope('123456789013', stellar.Operation.payment({ destination, asset: usdc, amount: '12.3456789' })) },
      { asset: `LONGASSET12:${issuer}`, amount: '0.0000001', sequence: '123456789014',
        envelope_b64: envelope('123456789014', stellar.Operation.payment({ destination, asset: long, amount: '0.0000001' })) },
    ],
    trustlines: [
      { asset: `USDC:${issuer}`, limit: '922337203685.4775807', sequence: '123456789015',
        envelope_b64: envelope('123456789015', stellar.Operation.changeTrust({ asset: usdc })) },
      { asset: `LONGASSET12:${issuer}`, limit: '0', sequence: '123456789016',
        envelope_b64: envelope('123456789016', stellar.Operation.changeTrust({ asset: long, limit: '0' })) },
    ],
  };
}

fs.writeFileSync('core/tests/fixtures/issued-assets.json', JSON.stringify({
  provenance: 'ripple-binary-codec 2.11.0, ripple-keypairs 3.1.0, @stellar/stellar-base 14.1.0',
  xrpl: xrpl(),
  stellar: stellarVectors(),
}, null, 2) + '\n');
