// Independent offline token-send fixtures from official SDKs.
// npm install --prefix /tmp/spectra-token-sdk --ignore-scripts @mysten/sui@1.45.2 @aptos-labs/ts-sdk@1.39.0 @ton/ton@16.3.0 @ton/core@0.63.1 @ton/crypto@3.3.0
// NODE_PATH=/tmp/spectra-token-sdk/node_modules node scripts/generate-token-send-vectors.cjs
const fs = require('node:fs');
const { Transaction } = require('@mysten/sui/transactions');
const { Ed25519Keypair } = require('@mysten/sui/keypairs/ed25519');
const apt = require('@aptos-labs/ts-sdk');
const ton = require('@ton/core');
const { WalletContractV4 } = require('@ton/ton');
const { keyPairFromSeed } = require('@ton/crypto');
const hex = value => Buffer.from(value).toString('hex');
const address = n => '0x' + n.repeat(32);
async function main() {
  const seed = Buffer.alloc(32, 1);
  const fixtures = { provenance: '@mysten/sui 1.45.2; @aptos-labs/ts-sdk 1.39.0; @ton/ton 16.3.0; @ton/core 0.63.1; @ton/crypto 3.3.0', aptos: [], sui: [], ton: [] };
  const account = apt.Account.fromPrivateKey({ privateKey: new apt.Ed25519PrivateKey(hex(seed)), legacy: true });
  for (const asset of [address('44') + '::coins::USD', address('44')]) {
    const fungible = !asset.includes('::');
    const payload = new apt.TransactionPayloadEntryFunction(apt.EntryFunction.build(
      fungible ? '0x1::primary_fungible_store' : '0x1::coin', 'transfer',
      [apt.parseTypeTag(fungible ? '0x1::fungible_asset::Metadata' : asset)],
      [...(fungible ? [apt.AccountAddress.fromString(asset)] : []), apt.AccountAddress.fromString(address('22')), new apt.U64(123456789n)]));
    const raw = new apt.RawTransaction(account.accountAddress, 7n, payload, 10000n, 100n, 1800000000n, new apt.ChainId(1));
    const message = apt.generateSigningMessage(raw.bcsToBytes(), 'APTOS::RawTransaction');
    const signature = account.sign(message);
    const transaction_hash = apt.generateUserTransactionHash({ transaction: new apt.SimpleTransaction(raw),
      senderAuthenticator: new apt.AccountAuthenticatorEd25519(account.publicKey, signature) });
    fixtures.aptos.push({ asset, sender: account.accountAddress.toString(), message: hex(message), signature: signature.toString().replace(/^0x/, ''), transaction_hash });
  }
  const key = Ed25519Keypair.fromSecretKey(seed);
  const object = (n, version) => ({ objectId: address(n), version: String(version), digest: '11111111111111111111111111111111' });
  for (const count of [1, 2]) {
    const tx = new Transaction();
    tx.setSender(key.toSuiAddress()); tx.setGasPrice(1000); tx.setGasBudget(10000000);
    tx.setGasPayment([object('33', 7)]);
    const amount = tx.pure.u64(123456789); const recipient = tx.pure.address(address('22'));
    const first = tx.objectRef(object('44', 8));
    if (count === 2) tx.mergeCoins(first, [tx.objectRef(object('55', 9))]);
    const [coin] = tx.splitCoins(first, [amount]); tx.transferObjects([coin], recipient);
    const bytes = await tx.build(); const signed = await key.signTransaction(bytes);
    fixtures.sui.push({ count, sender: key.toSuiAddress(), raw: hex(bytes), signature: signed.signature, transaction_digest: await tx.getDigest() });
  }
  const pair = keyPairFromSeed(seed); const wallet = WalletContractV4.create({ workchain: 0, publicKey: pair.publicKey });
  const source = new ton.Address(0, Buffer.alloc(32, 0x33)); const recipient = new ton.Address(0, Buffer.alloc(32, 0x22));
  for (const amount of [123456789n, 1000000000000000123456789n, (1n << 120n) - 1n]) {
  const payload = ton.beginCell().storeUint(0x0f8a7ea5, 32).storeUint(7, 64).storeCoins(amount)
    .storeAddress(recipient).storeAddress(wallet.address).storeBit(false).storeCoins(1n).storeBit(false).endCell();
  const body = wallet.createTransfer({ seqno: 7, secretKey: pair.secretKey, timeout: 1800000000, sendMode: 3,
    messages: [ton.internal({ to: source, value: 100000000n, bounce: true, body: payload })] });
  const external = ton.beginCell().store(ton.storeMessage(ton.external({ to: wallet.address, body }), { forceRef: true })).endCell();
  fixtures.ton.push({ amount:amount.toString(),sender: wallet.address.toRawString(), root_hash: hex(external.hash()), body_hash: hex(payload.hash()) });
  }
  fs.writeFileSync('core/tests/fixtures/token-send-vectors.json', JSON.stringify(fixtures, null, 2) + '\n');
}
main().catch(error => { console.error(error); process.exitCode = 1; });
