// Independent fixtures. Install the pinned SDKs in a temporary directory:
// npm install --prefix /tmp/spectra-protocol-vectors --ignore-scripts @ton/ton@16.3.0 @ton/core@0.63.1 @ton/crypto@3.3.0 @near-js/transactions@2.5.1 @near-js/crypto@2.5.1
// NODE_PATH=/tmp/spectra-protocol-vectors/node_modules node scripts/generate-protocol-vectors.cjs
const fs = require('node:fs');
const crypto = require('node:crypto');
const ton = require('@ton/core');
const { WalletContractV4 } = require('@ton/ton');
const { keyPairFromSeed } = require('@ton/crypto');
const near = require('@near-js/transactions');
const { PublicKey } = require('@near-js/crypto');
const key = keyPairFromSeed(Buffer.alloc(32, 1));
const wallet = WalletContractV4.create({ workchain: 0, publicKey: key.publicKey });
const destination = new ton.Address(0, Buffer.alloc(32, 0x22));
const fixtures = { provenance: '@ton/ton 16.3.0, @ton/core 0.63.1, @ton/crypto 3.3.0, @near-js/transactions 2.5.1, @near-js/crypto 2.5.1', public_key: key.publicKey.toString('hex'), ton: [], near: [] };
for (const [name, seqno, comment, bounce] of [['active', 7, '', true], ['deploy', 0, '', false], ['comment', 7, 'Hello 世界', false], ['snake', 7, 'x'.repeat(300), true]]) {
  const address = destination.toString({ bounceable: bounce });
  const body = wallet.createTransfer({seqno, secretKey: key.secretKey, timeout: seqno === 0 ? 0xffffffff : 1800000000, sendMode: 3, messages: [ton.internal({to: address, value: 123456789n, bounce, body: comment || undefined})]});
  const external = ton.beginCell().store(ton.storeMessage(ton.external({to: wallet.address, init: seqno === 0 ? wallet.init : undefined, body}), {forceRef: true})).endCell();
  fixtures.ton.push({name, seqno, comment, address, root_hash: external.hash().toString('hex'), boc: external.toBoc({idx:false, crc32:false}).toString('hex')});
}
const pk = new PublicKey({keyType:0, data:key.publicKey});
const secret = crypto.createPrivateKey({key:Buffer.concat([Buffer.from('302e020100300506032b657004220420','hex'), Buffer.alloc(32,1)]), format:'der', type:'pkcs8'});
for (const [name, action] of [['transfer', near.actionCreators.transfer(123456789n)], ['function_call', near.actionCreators.functionCall('ft_transfer', Buffer.from('{"amount":"123456","receiver_id":"bob.near"}'), 30000000000000n, 1n)]]) {
  const tx = near.createTransaction('alice.near', pk, 'token.near', 42n, [action], Buffer.alloc(32, 2));
  const hash = crypto.createHash('sha256').update(near.encodeTransaction(tx)).digest();
  const sig = crypto.sign(null, hash, secret);
  const signed = new near.SignedTransaction({transaction:tx, signature:new near.Signature({keyType:0, data:sig})});
  fixtures.near.push({name, signed_hex:Buffer.from(signed.encode()).toString('hex')});
}
fs.writeFileSync('core/testdata/protocol/transactions.json', JSON.stringify(fixtures, null, 2) + '\n');

// Optional: verify exported Rust BoCs, including wire decoding and signatures.
// SPECTRA_PROTOCOL_OUTPUT=/tmp/output cargo test -p spectra_core ton_messages_match_official_sdk_vectors
// NODE_PATH=/tmp/spectra-protocol-vectors/node_modules node scripts/generate-protocol-vectors.cjs /tmp/output
if (process.argv[2]) {
  const assert = require('node:assert/strict');
  const { signVerify } = require('@ton/crypto');
  for (const vector of fixtures.ton) {
    const roots = ton.Cell.fromBoc(fs.readFileSync(`${process.argv[2]}/ton-${vector.name}.boc`));
    assert.equal(roots.length, 1);
    const cell = roots[0];
    assert.equal(cell.hash().toString('hex'), vector.root_hash);
    const msg = ton.loadMessage(cell.beginParse());
    assert.equal(msg.info.type, 'external-in');
    assert(msg.info.dest.equals(wallet.address));
    assert.equal(!!msg.init, vector.seqno === 0);
    const slice = msg.body.beginParse();
    const signature = slice.loadBuffer(64);
    assert(signVerify(slice.asCell().hash(), signature, key.publicKey));
    assert.equal(slice.loadUint(32), wallet.walletId);
    slice.loadUint(32); // expiry is compared by the full fixture hash
    assert.equal(slice.loadUint(32), vector.seqno);
    assert.equal(slice.loadUint(8), 0);
    assert.equal(slice.loadUint(8), 3);
    const transfer = ton.loadMessageRelaxed(slice.loadRef().beginParse());
    assert(transfer.info.dest.equals(destination));
    assert.equal(transfer.info.value.coins, 123456789n);
    if (vector.comment) {
      const body = transfer.body.beginParse();
      assert.equal(body.loadUint(32), 0);
      assert.equal(body.loadStringTail(), vector.comment);
    }
    console.log(`Verified Rust TON ${vector.name}: wire, sender, amount, signature, comment`);
  }
}
