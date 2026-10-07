// Independent TON W5 (wallet v5r1) fixtures for core/tests/fixtures/ton-w5.json.
// Install the pinned SDKs in a temporary directory:
// npm install --prefix /tmp/spectra-ton-vectors --ignore-scripts @ton/ton@16.3.0 @ton/core@0.63.1 @ton/crypto@3.3.0
// NODE_PATH=/tmp/spectra-ton-vectors/node_modules node scripts/generate-ton-w5-vectors.cjs
//
// Addresses come from the mnemonics already in ton-mnemonics.json, so both
// files describe the same wallets; messages are signed with the 0x01 * 32 key
// the v4R2 vectors use.
const fs = require('node:fs');
const ton = require('@ton/core');
const { WalletContractV5R1 } = require('@ton/ton');
const { storeWalletIdV5R1 } = require('@ton/ton/dist/wallets/v5r1/WalletV5R1WalletId');
const { keyPairFromSeed, mnemonicToPrivateKey } = require('@ton/crypto');

const MAINNET = -239;
const TESTNET = -3;
const hex = (buffer) => Buffer.from(buffer).toString('hex');
const w5 = (publicKey, networkGlobalId) =>
  WalletContractV5R1.create({ workchain: 0, publicKey, walletId: { networkGlobalId } });
// Spectra's stored form: bounceable, URL-safe, without the testnet flag.
const friendly = (wallet) => wallet.address.toString({ bounceable: true, urlSafe: true });
const walletId = (wallet) =>
  ton.beginCell().store(storeWalletIdV5R1(wallet.walletId)).endCell().beginParse().loadUint(32);

async function main() {
  const mnemonics = JSON.parse(fs.readFileSync('core/tests/fixtures/ton-mnemonics.json', 'utf8'));
  const addresses = [];
  for (const entry of [...mnemonics.mnemonics, mnemonics.password_protected]) {
    const key = await mnemonicToPrivateKey(entry.mnemonic.split(' '), entry.password || undefined);
    addresses.push({
      mnemonic: entry.mnemonic,
      password: entry.password,
      public_key: hex(key.publicKey),
      mainnet: friendly(w5(key.publicKey, MAINNET)),
      testnet: friendly(w5(key.publicKey, TESTNET)),
    });
  }

  const key = keyPairFromSeed(Buffer.alloc(32, 1));
  const mainnet = w5(key.publicKey, MAINNET);
  const testnet = w5(key.publicKey, TESTNET);
  const destination = new ton.Address(0, Buffer.alloc(32, 0x22));
  const external = (wallet, body, init) =>
    ton.beginCell().store(ton.storeMessage(ton.external({ to: wallet.address, init, body }), { forceRef: true })).endCell();

  const transfers = [];
  for (const [name, network, seqno, comment, bounce] of [
    ['active', 'mainnet', 7, '', true],
    ['deploy', 'mainnet', 0, '', false],
    ['comment', 'mainnet', 7, 'Hello 世界', false],
    ['snake', 'mainnet', 7, 'x'.repeat(300), true],
    ['testnet', 'testnet', 7, '', true],
  ]) {
    const wallet = network === 'mainnet' ? mainnet : testnet;
    const address = destination.toString({ bounceable: bounce });
    // A deployment signs the all-ones expiry, as Spectra's v4R2 deployments do.
    const body = wallet.createTransfer({
      seqno, secretKey: key.secretKey, timeout: seqno === 0 ? 0xffffffff : 1800000000, sendMode: 3,
      messages: [ton.internal({ to: address, value: 123456789n, bounce, body: comment || undefined })],
    });
    const message = external(wallet, body, seqno === 0 ? wallet.init : undefined);
    transfers.push({ name, network, seqno, comment, address, root_hash: hex(message.hash()) });
  }

  const jettons = [];
  const source = new ton.Address(0, Buffer.alloc(32, 0x33));
  const recipient = new ton.Address(0, Buffer.alloc(32, 0x22));
  for (const amount of [123456789n, (1n << 120n) - 1n]) {
    const payload = ton.beginCell().storeUint(0x0f8a7ea5, 32).storeUint(7, 64).storeCoins(amount)
      .storeAddress(recipient).storeAddress(mainnet.address).storeBit(false).storeCoins(1n).storeBit(false).endCell();
    const body = mainnet.createTransfer({ seqno: 7, secretKey: key.secretKey, timeout: 1800000000, sendMode: 3,
      messages: [ton.internal({ to: source, value: 100000000n, bounce: true, body: payload })] });
    jettons.push({ amount: amount.toString(), root_hash: hex(external(mainnet, body).hash()) });
  }

  const fixtures = {
    provenance: '@ton/ton 16.3.0, @ton/core 0.63.1, @ton/crypto 3.3.0',
    code_hash: hex(mainnet.init.code.hash()),
    wallet_id: { mainnet: walletId(mainnet), testnet: walletId(testnet) },
    public_key: hex(key.publicKey),
    key_address: { mainnet: friendly(mainnet), testnet: friendly(testnet) },
    addresses,
    transfers,
    jettons,
  };
  fs.writeFileSync('core/tests/fixtures/ton-w5.json', JSON.stringify(fixtures, null, 2) + '\n');
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
