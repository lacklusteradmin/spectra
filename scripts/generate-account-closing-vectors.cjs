// Account-closing transactions from each network's own SDK: XRP
// AccountDelete and Stellar AccountMerge, each signed with a fixed key.
// npm install --prefix /tmp/spectra-closing-vectors --ignore-scripts ripple-binary-codec@2.11.0 ripple-keypairs@3.1.0 @stellar/stellar-base@14.1.0
// NODE_PATH=/tmp/spectra-closing-vectors/node_modules node scripts/generate-account-closing-vectors.cjs
const fs = require('node:fs');
const codec = require('ripple-binary-codec');
const keypairs = require('ripple-keypairs');
const stellar = require('@stellar/stellar-base');

function xrpAccountDelete() {
  const publicKey = '0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798';
  const privateKey = '00' + '0'.repeat(63) + '1';
  const transaction = {
    TransactionType: 'AccountDelete',
    Flags: 0,
    Sequence: 93_000_000,
    Fee: '200000',
    SigningPubKey: publicKey,
    Account: keypairs.deriveAddress(publicKey),
    Destination: 'rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe',
  };
  const signingHex = codec.encodeForSigning(transaction);
  transaction.TxnSignature = keypairs.sign(signingHex, privateKey);
  return {
    key: privateKey.slice(2),
    transaction,
    signing_hex: signingHex,
    signed_hex: codec.encode(transaction),
  };
}

function stellarAccountMerge() {
  const seed = Buffer.alloc(32, 7);
  const source = stellar.Keypair.fromRawEd25519Seed(seed);
  const destination = stellar.Keypair.fromRawEd25519Seed(Buffer.alloc(32, 9)).publicKey();
  const passphrase = stellar.Networks.TESTNET;
  const { xdr } = stellar;
  // Built from XDR types: Spectra's transactions carry no time bounds, which
  // the SDK's builder always adds.
  const sequence = '123456789013';
  const body = new xdr.Transaction({
    sourceAccount: stellar.decodeAddressToMuxedAccount(source.publicKey()),
    fee: 100,
    seqNum: xdr.SequenceNumber.fromString(sequence),
    cond: xdr.Preconditions.precondNone(),
    memo: stellar.Memo.none().toXDRObject(),
    operations: [stellar.Operation.accountMerge({ destination })],
    ext: new xdr.TransactionExt(0),
  });
  const envelope = xdr.TransactionEnvelope.envelopeTypeTx(
    new xdr.TransactionV1Envelope({ tx: body, signatures: [] }),
  );
  const transaction = new stellar.Transaction(envelope, passphrase);
  transaction.sign(source);
  return {
    seed: seed.toString('hex'),
    source: source.publicKey(),
    destination,
    sequence,
    fee: transaction.fee,
    network_passphrase: passphrase,
    envelope_b64: transaction.toEnvelope().toXDR('base64'),
  };
}

const fixture = {
  provenance: 'ripple-binary-codec 2.11.0, ripple-keypairs 3.1.0, @stellar/stellar-base 14.1.0',
  xrp_account_delete: xrpAccountDelete(),
  stellar_account_merge: stellarAccountMerge(),
};
fs.writeFileSync('core/tests/fixtures/account-closing.json', JSON.stringify(fixture, null, 2) + '\n');
