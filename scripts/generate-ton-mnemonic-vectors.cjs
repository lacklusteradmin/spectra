// Independent TON mnemonic fixtures for core/tests/fixtures/ton-mnemonics.json.
// Install the pinned SDKs in a temporary directory:
// npm install --prefix /tmp/spectra-ton-vectors --ignore-scripts @ton/ton@16.3.0 @ton/core@0.63.1 @ton/crypto@3.3.0
// NODE_PATH=/tmp/spectra-ton-vectors/node_modules node scripts/generate-ton-mnemonic-vectors.cjs > core/tests/fixtures/ton-mnemonics.json
const { mnemonicNew, mnemonicValidate, mnemonicToPrivateKey } = require('@ton/crypto');
const { WalletContractV4 } = require('@ton/ton');

async function vector(words, password) {
  const key = await mnemonicToPrivateKey(words, password);
  const wallet = WalletContractV4.create({ workchain: 0, publicKey: key.publicKey });
  return {
    mnemonic: words.join(' '),
    password: password || '',
    public_key: key.publicKey.toString('hex'),
    private_key: key.secretKey.subarray(0, 32).toString('hex'),
    address: wallet.address.toString({ bounceable: true, urlSafe: true }),
  };
}

(async () => {
  const plain = [];
  for (let i = 0; i < 3; i++) plain.push(await vector(await mnemonicNew(24)));
  const protectedWords = await mnemonicNew(24, 'spectra fixture');
  const bip39 = 'abandon '.repeat(23) + 'art';
  const fixtures = {
    provenance: '@ton/ton 16.3.0, @ton/core 0.63.1, @ton/crypto 3.3.0',
    mnemonics: plain,
    password_protected: await vector(protectedWords, 'spectra fixture'),
    password_protected_valid_without_password: await mnemonicValidate(protectedWords),
    bip39_phrase: bip39,
    bip39_phrase_is_ton_mnemonic: await mnemonicValidate(bip39.split(' ')),
  };
  console.log(JSON.stringify(fixtures, null, 2));
})();
