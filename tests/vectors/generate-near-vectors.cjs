// NEAR transactions as the NEAR SDK builds and signs them, every one from
// one fixed signer: a NEAR transfer and an `ft_transfer` call; a DeleteKey; a
// NEP-145 `storage_unregister`, one FunctionCall to the token contract with
// empty arguments, so `force` is never passed, and the one yoctoNEAR the
// standard requires attached; and a NEP-141 transfer to a recipient the token
// has registered and to one it has not: storage_deposit with
// registration_only and the contract's minimum, then ft_transfer, in one
// transaction to the token.
// node tests/vectors/generate-near-vectors.cjs
const fs = require('node:fs');
const crypto = require('node:crypto');
const near = require('@near-js/transactions');
const { PublicKey } = require('@near-js/crypto');
const { baseEncode } = require('@near-js/utils');

const pkcs8 = (seed) => crypto.createPrivateKey({
  key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), seed]), format: 'der', type: 'pkcs8',
});
const publicOf = (seed) => crypto.createPublicKey(pkcs8(seed)).export({ format: 'der', type: 'spki' }).subarray(-32);
const seed = Buffer.alloc(32, 1);
const signer = new PublicKey({ keyType: 0, data: publicOf(seed) });
const blockHash = Buffer.alloc(32, 2);
const sign = (receiver, actions) => {
  const tx = near.createTransaction('alice.near', signer, receiver, 42n, actions, blockHash);
  const hash = crypto.createHash('sha256').update(near.encodeTransaction(tx)).digest();
  const signed = new near.SignedTransaction({
    transaction: tx, signature: new near.Signature({ keyType: 0, data: crypto.sign(null, hash, pkcs8(seed)) }),
  });
  return { signed_hex: Buffer.from(signed.encode()).toString('hex'), hash: baseEncode(hash) };
};
// Arguments as JSON bytes, keys in the order serde_json writes them.
const json = (value) => Buffer.from(JSON.stringify(value));

const token = 'usdt.tether-token.near';
const recipient = 'bob.near';
const amount = '1000000';
const gas = 30_000_000_000_000n;
const minimum = 1_250_000_000_000_000_000_000n;
const deleted = new PublicKey({ keyType: 0, data: publicOf(Buffer.alloc(32, 3)) });
const ftTransfer = near.actionCreators.functionCall('ft_transfer', json({ amount, receiver_id: recipient }), gas, 1n);

fs.writeFileSync('core/tests/fixtures/near-transactions.json', JSON.stringify({
  provenance: '@near-js/transactions 2.5.1, @near-js/crypto 2.5.1',
  seed: seed.toString('hex'),
  signer: 'alice.near',
  public_key: Buffer.from(signer.data).toString('hex'),
  nonce: 42,
  block_hash: blockHash.toString('hex'),
  transfer: sign('token.near', [near.actionCreators.transfer(123456789n)]),
  function_call: sign('token.near', [near.actionCreators.functionCall('ft_transfer',
    json({ amount: '123456', receiver_id: recipient }), gas, 1n)]),
  delete_key: { deleted_key: deleted.toString(), ...sign('alice.near', [near.actionCreators.deleteKey(deleted)]) },
  storage_unregister: {
    token,
    gas: gas.toString(),
    ...sign(token, [near.actionCreators.functionCall('storage_unregister', {}, gas, 1n)]),
  },
  token_registration: {
    token,
    recipient,
    amount,
    gas: gas.toString(),
    registration_deposit: minimum.toString(),
    registered: sign(token, [ftTransfer]),
    unregistered: sign(token, [
      near.actionCreators.functionCall('storage_deposit',
        json({ account_id: recipient, registration_only: true }), gas, minimum),
      ftTransfer,
    ]),
  },
}, null, 2) + '\n');
