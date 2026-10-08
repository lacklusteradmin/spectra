// A NEP-145 `storage_unregister` as the NEAR SDK builds and signs it: one
// FunctionCall to the token contract with empty arguments, so `force` is
// never passed, and the one yoctoNEAR the standard requires attached.
// npm install --prefix /tmp/spectra-near-vectors --ignore-scripts @near-js/transactions@2.5.1 @near-js/crypto@2.5.1 @near-js/utils@2.5.1
// NODE_PATH=/tmp/spectra-near-vectors/node_modules node scripts/generate-near-storage-unregister-vector.cjs
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
const gas = 30_000_000_000_000n;
const tx = near.createTransaction('alice.near', signer, token, 42n,
  [near.actionCreators.functionCall('storage_unregister', {}, gas, 1n)], Buffer.alloc(32, 2));
const hash = crypto.createHash('sha256').update(near.encodeTransaction(tx)).digest();
const signed = new near.SignedTransaction({
  transaction: tx, signature: new near.Signature({ keyType: 0, data: crypto.sign(null, hash, pkcs8(signerSeed)) }),
});
fs.writeFileSync('core/tests/fixtures/near-storage-unregister.json', JSON.stringify({
  provenance: '@near-js/transactions 2.5.1, @near-js/crypto 2.5.1',
  seed: signerSeed.toString('hex'),
  signer: 'alice.near',
  public_key: Buffer.from(signer.data).toString('hex'),
  token,
  gas: gas.toString(),
  nonce: 42,
  block_hash: Buffer.alloc(32, 2).toString('hex'),
  signed_hex: Buffer.from(signed.encode()).toString('hex'),
  hash: baseEncode(hash),
}, null, 2) + '\n');
