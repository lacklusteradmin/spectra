// A NEP-141 transfer to a recipient the token has not registered, as the NEAR
// SDK builds and signs it: storage_deposit with registration_only and the
// contract's minimum, then ft_transfer, in one transaction to the token.
// npm install --prefix /tmp/spectra-near-vectors --ignore-scripts @near-js/transactions@2.5.1 @near-js/crypto@2.5.1 @near-js/utils@2.5.1
// NODE_PATH=/tmp/spectra-near-vectors/node_modules node scripts/generate-near-token-registration-vector.cjs
const fs = require('node:fs');
const crypto = require('node:crypto');
const near = require('@near-js/transactions');
const { PublicKey } = require('@near-js/crypto');
const { baseEncode } = require('@near-js/utils');

const pkcs8 = (seed) => crypto.createPrivateKey({
  key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), seed]), format: 'der', type: 'pkcs8',
});
const publicOf = (seed) => crypto.createPublicKey(pkcs8(seed)).export({ format: 'der', type: 'spki' }).subarray(-32);
const signerSeed = Buffer.alloc(32, 1);
const signer = new PublicKey({ keyType: 0, data: publicOf(signerSeed) });
const token = 'usdt.tether-token.near';
const recipient = 'bob.near';
const amount = '1000000';
const gas = 30_000_000_000_000n;
const minimum = 1_250_000_000_000_000_000_000n;
// Arguments as JSON bytes, keys in the order serde_json writes them.
const json = (value) => Buffer.from(JSON.stringify(value));
const sign = (actions) => {
  const tx = near.createTransaction('alice.near', signer, token, 42n, actions, Buffer.alloc(32, 2));
  const hash = crypto.createHash('sha256').update(near.encodeTransaction(tx)).digest();
  const signed = new near.SignedTransaction({
    transaction: tx, signature: new near.Signature({ keyType: 0, data: crypto.sign(null, hash, pkcs8(signerSeed)) }),
  });
  return { signed_hex: Buffer.from(signed.encode()).toString('hex'), hash: baseEncode(hash) };
};
const transfer = near.actionCreators.functionCall('ft_transfer', json({ amount, receiver_id: recipient }), gas, 1n);
fs.writeFileSync('core/tests/fixtures/near-token-registration.json', JSON.stringify({
  provenance: '@near-js/transactions 2.5.1, @near-js/crypto 2.5.1',
  seed: signerSeed.toString('hex'),
  signer: 'alice.near',
  public_key: Buffer.from(signer.data).toString('hex'),
  token,
  recipient,
  amount,
  gas: gas.toString(),
  registration_deposit: minimum.toString(),
  nonce: 42,
  block_hash: Buffer.alloc(32, 2).toString('hex'),
  registered: sign([transfer]),
  unregistered: sign([
    near.actionCreators.functionCall('storage_deposit',
      json({ account_id: recipient, registration_only: true }), gas, minimum),
    transfer,
  ]),
}, null, 2) + '\n');
