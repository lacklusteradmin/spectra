// Token-2022 transfers under a transfer fee and a transfer hook, built by
// @solana/spl-token: TransferCheckedWithFee stating the fee, and a hook's
// extra accounts resolved from its validation account by
// addExtraAccountMetasForExecute, every seed kind included.
// npm install --prefix /tmp/spectra-solana-vectors --ignore-scripts @solana/web3.js@1.98.4 @solana/spl-token@0.4.14
// NODE_PATH=/tmp/spectra-solana-vectors/node_modules node scripts/generate-solana-token-2022-vectors.cjs
const fs = require('node:fs');
const web3 = require('@solana/web3.js');
const spl = require('@solana/spl-token');

const owner = web3.Keypair.fromSeed(Buffer.alloc(32, 1));
const key = byte => new web3.PublicKey(Buffer.alloc(32, byte));
const recipient = key(0x22);
const mint = key(0x44);
const hook = key(0x77);
const fixed = key(0x55);
const external = key(0x66);
const program = spl.TOKEN_2022_PROGRAM_ID;
const source = spl.getAssociatedTokenAddressSync(mint, owner.publicKey, false, program);
const destination = spl.getAssociatedTokenAddressSync(mint, recipient, true, program);
const validation = spl.getExtraAccountMetaAddress(mint, hook);
const amount = 1000000n;
const decimals = 6;
const fee = 5000n;

// The sending account as the token program lays it out: mint, owner, amount.
const sourceData = Buffer.alloc(165);
mint.toBuffer().copy(sourceData, 0);
owner.publicKey.toBuffer().copy(sourceData, 32);
sourceData.writeBigUInt64LE(5000000n, 64);
sourceData[108] = 1;

const meta = (discriminator, config, writable) => {
  const entry = Buffer.alloc(35);
  entry[0] = discriminator;
  Buffer.from(config).copy(entry, 1);
  entry[34] = writable ? 1 : 0;
  return entry;
};
const packed = (...parts) => {
  const config = Buffer.alloc(32);
  Buffer.concat(parts).copy(config);
  return config;
};
const literal = text => Buffer.concat([Buffer.from([1, text.length]), Buffer.from(text)]);
const metas = [
  meta(0, fixed.toBuffer(), true), // 5: a fixed account the hook writes
  meta(1, packed(literal('counter'), Buffer.from([3, 1])), true), // 6: the hook's PDA of ["counter", mint]
  meta(1, packed(Buffer.from([3, 0]), Buffer.from([2, 8, 8])), false), // 7: PDA of [source, amount]
  meta(1, packed(Buffer.from([4, 0, 32, 32])), false), // 8: PDA of the source account's owner field
  meta(0, external.toBuffer(), false), // 9: another program
  meta(0x80 + 9, packed(Buffer.from([3, 3])), true), // 10: that program's PDA of [authority]
  meta(2, packed(Buffer.from([2, 0, 32])), false), // 11: the key at byte 32 of the source account
  meta(0, mint.toBuffer(), true), // 12: the mint, asked writable, kept read-only
];
const execute = Buffer.from([105, 37, 101, 197, 75, 251, 102, 26]);
const list = Buffer.concat([Buffer.from(new Uint32Array([metas.length]).buffer), ...metas]);
const validationData = Buffer.concat([execute, Buffer.from(new Uint32Array([list.length]).buffer), list]);

const accounts = new Map([
  [validation.toBase58(), { data: validationData, owner: hook }],
  [source.toBase58(), { data: sourceData, owner: program }],
]);
const connection = {
  getAccountInfo: async address => {
    const account = accounts.get(address.toBase58());
    return account ? { ...account, executable: false, lamports: 1, rentEpoch: 0 } : null;
  },
};
const json = instruction => ({
  program: instruction.programId.toBase58(),
  keys: instruction.keys.map(k => ({ pubkey: k.pubkey.toBase58(), signer: k.isSigner, writable: k.isWritable })),
  data: instruction.data.toString('hex'),
});

async function main() {
  const create = spl.createAssociatedTokenAccountIdempotentInstruction(owner.publicKey, destination, recipient, mint, program);
  const withFee = () => spl.createTransferCheckedWithFeeInstruction(source, mint, destination, owner.publicKey, amount, decimals, fee, [], program);
  const plain = () => spl.createTransferCheckedInstruction(source, mint, destination, owner.publicKey, amount, decimals, [], program);
  const hooked = async instruction => {
    await spl.addExtraAccountMetasForExecute(connection, instruction, hook, source, mint, destination, owner.publicKey, amount);
    return instruction;
  };
  const cases = {
    fee: [create, withFee()],
    hook: [create, await hooked(plain())],
    fee_and_hook: [create, await hooked(withFee())],
  };
  fs.writeFileSync('core/tests/fixtures/solana-token-2022.json', JSON.stringify({
    provenance: '@solana/web3.js 1.98.4; @solana/spl-token 0.4.14',
    seed: Buffer.alloc(32, 1).toString('hex'),
    owner: owner.publicKey.toBase58(),
    recipient: recipient.toBase58(),
    mint: mint.toBase58(),
    hook_program: hook.toBase58(),
    external_program: external.toBase58(),
    source: source.toBase58(),
    destination: destination.toBase58(),
    validation_account: validation.toBase58(),
    validation_data: validationData.toString('base64'),
    source_data: sourceData.toString('base64'),
    amount: amount.toString(),
    decimals,
    fee: fee.toString(),
    cases: Object.fromEntries(Object.entries(cases).map(([name, instructions]) => [name, instructions.map(json)])),
  }, null, 2) + '\n');
}
main().catch(error => { console.error(error); process.exitCode = 1; });
