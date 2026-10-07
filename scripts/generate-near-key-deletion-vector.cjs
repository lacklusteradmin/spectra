// A NEAR DeleteKey transaction from the NEAR SDK, signed with a fixed key.
// npm install --prefix /tmp/spectra-near-vectors --ignore-scripts @near-js/transactions@2.5.1 @near-js/crypto@2.5.1 @near-js/utils@2.5.1
// NODE_PATH=/tmp/spectra-near-vectors/node_modules node scripts/generate-near-key-deletion-vector.cjs
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
const deleted = new PublicKey({ keyType: 0, data: publicOf(Buffer.alloc(32, 3)) });
const tx = near.createTransaction('alice.near', signer, 'alice.near', 42n, [near.actionCreators.deleteKey(deleted)],
  Buffer.alloc(32, 2));
const hash = crypto.createHash('sha256').update(near.encodeTransaction(tx)).digest();
const signed = new near.SignedTransaction({
  transaction: tx, signature: new near.Signature({ keyType: 0, data: crypto.sign(null, hash, pkcs8(signerSeed)) }),
});
fs.writeFileSync('core/tests/fixtures/near-delete-key.json', JSON.stringify({
  provenance: '@near-js/transactions 2.5.1, @near-js/crypto 2.5.1',
  seed: signerSeed.toString('hex'),
  signer: 'alice.near',
  public_key: Buffer.from(signer.data).toString('hex'),
  deleted_key: deleted.toString(),
  nonce: 42,
  block_hash: Buffer.alloc(32, 2).toString('hex'),
  signed_hex: Buffer.from(signed.encode()).toString('hex'),
  hash: baseEncode(hash),
}, null, 2) + '\n');
