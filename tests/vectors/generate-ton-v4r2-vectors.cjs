// Independent TON v4R2 wallet transfers for core/tests/fixtures/ton-v4r2.json,
// signed with the 0x01 * 32 key: an active wallet's, a deploying one's, one
// with a comment and one whose comment runs past a cell.
// node tests/vectors/generate-ton-v4r2-vectors.cjs
const fs = require('node:fs');
const ton = require('@ton/core');
const { WalletContractV4 } = require('@ton/ton');
const { keyPairFromSeed } = require('@ton/crypto');
const key = keyPairFromSeed(Buffer.alloc(32, 1));
const wallet = WalletContractV4.create({ workchain: 0, publicKey: key.publicKey });
const destination = new ton.Address(0, Buffer.alloc(32, 0x22));
const fixtures = { provenance: '@ton/ton 16.3.0, @ton/core 0.63.1, @ton/crypto 3.3.0', public_key: key.publicKey.toString('hex'), transfers: [] };
for (const [name, seqno, comment, bounce] of [['active', 7, '', true], ['deploy', 0, '', false], ['comment', 7, 'Hello 世界', false], ['snake', 7, 'x'.repeat(300), true]]) {
  const address = destination.toString({ bounceable: bounce });
  const body = wallet.createTransfer({seqno, secretKey: key.secretKey, timeout: seqno === 0 ? 0xffffffff : 1800000000, sendMode: 3, messages: [ton.internal({to: address, value: 123456789n, bounce, body: comment || undefined})]});
  const external = ton.beginCell().store(ton.storeMessage(ton.external({to: wallet.address, init: seqno === 0 ? wallet.init : undefined, body}), {forceRef: true})).endCell();
  fixtures.transfers.push({name, seqno, comment, address, root_hash: external.hash().toString('hex'), boc: external.toBoc({idx:false, crc32:false}).toString('hex')});
}
fs.writeFileSync('core/tests/fixtures/ton-v4r2.json', JSON.stringify(fixtures, null, 2) + '\n');

// Optional: verify exported Rust BoCs, including wire decoding and signatures.
// SPECTRA_TON_V4R2_OUTPUT=/tmp/output cargo test -p spectra_core ton_messages_match_official_sdk_vectors
// node tests/vectors/generate-ton-v4r2-vectors.cjs /tmp/output
if (process.argv[2]) {
  const assert = require('node:assert/strict');
  const { signVerify } = require('@ton/crypto');
  for (const vector of fixtures.transfers) {
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
