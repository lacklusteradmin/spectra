// Closing empty SPL and Token-2022 accounts, compiled and signed by
// @solana/web3.js: one CloseAccount (9) per account, rent to the owner.
// npm install --prefix /tmp/spectra-solana-vectors --ignore-scripts @solana/web3.js@1.98.4
// NODE_PATH=/tmp/spectra-solana-vectors/node_modules node scripts/generate-solana-close-accounts-vector.cjs
const fs = require('node:fs');
const web3 = require('@solana/web3.js');

const owner = web3.Keypair.fromSeed(Buffer.alloc(32, 1));
const TOKEN = new web3.PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const TOKEN_2022 = new web3.PublicKey('TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb');
const accounts = [[new web3.PublicKey(Buffer.alloc(32, 0x11)), TOKEN], [new web3.PublicKey(Buffer.alloc(32, 0x22)), TOKEN_2022]];
const blockhash = new web3.PublicKey(Buffer.alloc(32, 5)).toBase58();
const tx = new web3.Transaction({ feePayer: owner.publicKey, recentBlockhash: blockhash });
for (const [account, program] of accounts) {
  tx.add(new web3.TransactionInstruction({
    programId: program,
    keys: [
      { pubkey: account, isSigner: false, isWritable: true },
      { pubkey: owner.publicKey, isSigner: false, isWritable: true },
      { pubkey: owner.publicKey, isSigner: true, isWritable: false },
    ],
    data: Buffer.from([9]),
  }));
}
tx.sign(owner);
fs.writeFileSync('core/tests/fixtures/solana-close-accounts.json', JSON.stringify({
  provenance: '@solana/web3.js 1.98.4',
  seed: Buffer.alloc(32, 1).toString('hex'),
  owner: owner.publicKey.toBase58(),
  accounts: accounts.map(([account, program]) => ({ account: account.toBase58(), program: program.toBase58() })),
  blockhash,
  message: tx.serializeMessage().toString('hex'),
  signed: tx.serialize().toString('hex'),
}, null, 2) + '\n');
