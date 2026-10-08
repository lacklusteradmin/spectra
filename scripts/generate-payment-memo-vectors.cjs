// Destination tags and memos as each network's own SDK builds and signs
// them: XRP Payments (XRP and an issued currency) and AccountDelete with a
// DestinationTag, Stellar payments (native and a credit asset) and
// AccountMerge with a text or ID memo.
// npm install --prefix /tmp/spectra-memo-vectors --ignore-scripts ripple-binary-codec@2.11.0 ripple-keypairs@3.1.0 @stellar/stellar-base@14.1.0
// NODE_PATH=/tmp/spectra-memo-vectors/node_modules node scripts/generate-payment-memo-vectors.cjs
const fs = require('node:fs');
const codec = require('ripple-binary-codec');
const keypairs = require('ripple-keypairs');
const stellar = require('@stellar/stellar-base');

const ISSUER = 'rhub8VRN55s94qWKDv6jmDy1pUykJzF3wq';
const DESTINATION = 'rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe';

function xrpl() {
  const publicKey = '0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798';
  const privateKey = '00' + '0'.repeat(63) + '1';
  const account = keypairs.deriveAddress(publicKey);
  const sign = transaction => {
    const signingHex = codec.encodeForSigning(transaction);
    transaction.TxnSignature = keypairs.sign(signingHex, privateKey);
    return { transaction, signed_hex: codec.encode(transaction) };
  };
  const base = (type, sequence) => ({
    TransactionType: type, Flags: 0, Sequence: sequence, Fee: '12', SigningPubKey: publicKey, Account: account,
  });
  return {
    key: privateKey.slice(2),
    payments: [
      sign({ ...base('Payment', 7), Destination: DESTINATION, DestinationTag: 0, Amount: '1000000' }),
      sign({ ...base('Payment', 8), Destination: DESTINATION, DestinationTag: 4294967295, Amount: '1' }),
      sign({ ...base('Payment', 9), Destination: DESTINATION, DestinationTag: 123456,
        Amount: { currency: 'USD', issuer: ISSUER, value: '123.456' },
        SendMax: { currency: 'USD', issuer: ISSUER, value: '123.702912' } }),
    ],
    account_delete: sign({ ...base('AccountDelete', 93_000_000), Fee: '200000', Destination: DESTINATION,
      DestinationTag: 2468 }),
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
  const envelope = (sequence, memo, operation) => {
    const body = new xdr.Transaction({
      sourceAccount: stellar.decodeAddressToMuxedAccount(source.publicKey()),
      fee: 100,
      seqNum: xdr.SequenceNumber.fromString(sequence),
      cond: xdr.Preconditions.precondNone(),
      memo: memo.toXDRObject(),
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
  const native = stellar.Asset.native();
  const cases = [
    // One byte, one past a word boundary, and the 28-byte limit.
    { kind: 'memoText', memo: 'a', asset: null, amount: '1.5', sequence: '123456789013' },
    { kind: 'memoText', memo: 'deposit 10293', asset: null, amount: '1.5', sequence: '123456789014' },
    { kind: 'memoText', memo: 'x'.repeat(28), asset: null, amount: '1.5', sequence: '123456789015' },
    { kind: 'memoId', memo: '0', asset: null, amount: '1.5', sequence: '123456789016' },
    { kind: 'memoId', memo: '18446744073709551615', asset: null, amount: '1.5', sequence: '123456789017' },
    { kind: 'memoId', memo: '1029384756', asset: `USDC:${issuer}`, amount: '12.3456789', sequence: '123456789018' },
  ];
  const memoOf = (kind, value) => (kind === 'memoText' ? stellar.Memo.text(value) : stellar.Memo.id(value));
  return {
    seed: seed.toString('hex'),
    source: source.publicKey(),
    destination,
    network_passphrase: passphrase,
    payments: cases.map(c => ({
      ...c,
      envelope_b64: envelope(c.sequence, memoOf(c.kind, c.memo),
        stellar.Operation.payment({ destination, asset: c.asset ? usdc : native, amount: c.amount })),
    })),
    account_merge: {
      kind: 'memoText', memo: 'close 77', sequence: '123456789019',
      envelope_b64: envelope('123456789019', stellar.Memo.text('close 77'), stellar.Operation.accountMerge({ destination })),
    },
  };
}

fs.writeFileSync('core/tests/fixtures/payment-memos.json', JSON.stringify({
  provenance: 'ripple-binary-codec 2.11.0, ripple-keypairs 3.1.0, @stellar/stellar-base 14.1.0',
  xrpl: xrpl(),
  stellar: stellarVectors(),
}, null, 2) + '\n');
