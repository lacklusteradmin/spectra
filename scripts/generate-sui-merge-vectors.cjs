// Sui coin merges from the Sui SDK: a token type's objects merged into one
// with SUI paying gas, and SUI's own coins merged by paying gas with all of
// them and keeping the gas coin.
// npm install --prefix /tmp/spectra-sui-vectors --ignore-scripts @mysten/sui@1.45.2
// NODE_PATH=/tmp/spectra-sui-vectors/node_modules node scripts/generate-sui-merge-vectors.cjs
const fs = require('node:fs');
const { Transaction } = require('@mysten/sui/transactions');
const { Ed25519Keypair } = require('@mysten/sui/keypairs/ed25519');
const hex = (value) => Buffer.from(value).toString('hex');
const address = (n) => '0x' + n.repeat(32);
const object = (n, version) => ({ objectId: address(n), version: String(version), digest: '11111111111111111111111111111111' });

async function main() {
  const key = Ed25519Keypair.fromSecretKey(Buffer.alloc(32, 1));
  const sender = key.toSuiAddress();
  const vectors = [];
  async function push(name, tx) {
    const bytes = await tx.build();
    const signed = await key.signTransaction(bytes);
    vectors.push({ name, raw: hex(bytes), signature: signed.signature, transaction_digest: await tx.getDigest() });
  }
  const token = new Transaction();
  token.setSender(sender); token.setGasPrice(1000); token.setGasBudget(5000000);
  token.setGasPayment([object('33', 7)]);
  token.mergeCoins(token.objectRef(object('44', 8)), [token.objectRef(object('55', 9)), token.objectRef(object('66', 10))]);
  await push('token', token);
  const sui = new Transaction();
  sui.setSender(sender); sui.setGasPrice(1000); sui.setGasBudget(5000000);
  sui.setGasPayment([object('33', 7), object('44', 8), object('55', 9)]);
  sui.transferObjects([sui.gas], sui.pure.address(sender));
  await push('sui', sui);
  fs.writeFileSync('core/tests/fixtures/sui-merge.json', JSON.stringify({
    provenance: '@mysten/sui 1.45.2', seed: hex(Buffer.alloc(32, 1)), sender, vectors,
  }, null, 2) + '\n');
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
