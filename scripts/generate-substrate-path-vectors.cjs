// Independent sr25519 keys along Substrate derivation paths (`//hard`,
// `/soft`), for core/tests/fixtures/substrate-paths.json.
// Install the pinned libraries in a temporary directory:
// npm install --prefix /tmp/spectra-substrate-paths --ignore-scripts @polkadot/keyring@14.0.3 @polkadot/util-crypto@14.0.3 @polkadot/util@14.0.3 @polkadot/wasm-crypto@7.5.4
// NODE_PATH=/tmp/spectra-substrate-paths/node_modules node scripts/generate-substrate-path-vectors.cjs > core/tests/fixtures/substrate-paths.json
//
// Each key is polkadot.js's reading of the secret URI `phrase + path`, with
// `///passphrase` when one is set: the key derived along the path, its public
// key and its addresses on Polkadot (SS58 prefix 0) and on Westend and
// Bittensor (prefix 42). A hard junction derives a deterministic secret key;
// a soft one a key whose nonce half is random, so only its scalar half is
// recorded.
const { cryptoWaitReady, keyExtractSuri, keyFromPath, mnemonicToMiniSecret, sr25519PairFromSeed, encodeAddress } = require('@polkadot/util-crypto');
const { u8aToHex } = require('@polkadot/util');

const PHRASE = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const CASES = [
  { path: '', passphrase: '' },
  { path: '//polkadot', passphrase: '' },
  { path: '//0', passphrase: '' },
  { path: '//1', passphrase: '' },
  { path: '/0', passphrase: '' },
  { path: '//polkadot//0', passphrase: '' },
  { path: '//polkadot//0/1', passphrase: '' },
  { path: '/soft//hard', passphrase: '' },
  { path: '//007', passphrase: '' },
  { path: '//18446744073709551615', passphrase: '' },
  { path: '//my account', passphrase: '' },
  { path: '//钱包', passphrase: '' },
  { path: '//0xabc', passphrase: '' },
  { path: '//a junction name longer than thirty-two bytes', passphrase: '' },
  { path: '//polkadot', passphrase: 'secret' },
];

(async () => {
  await cryptoWaitReady();
  const vectors = CASES.map(({ path, passphrase }) => {
    const uri = PHRASE + path + (passphrase ? `///${passphrase}` : '');
    const suri = keyExtractSuri(uri);
    const root = sr25519PairFromSeed(mnemonicToMiniSecret(suri.phrase, suri.password || ''));
    const pair = keyFromPath(root, suri.path, 'sr25519');
    const hardOnly = suri.path.every((junction) => junction.isHard);
    return {
      path,
      passphrase,
      hardOnly,
      publicKey: u8aToHex(pair.publicKey).slice(2),
      // The 64-byte secret key when every junction is hard; only its
      // 32-byte scalar half after a soft junction.
      secretKey: u8aToHex(hardOnly ? pair.secretKey : pair.secretKey.slice(0, 32)).slice(2),
      polkadot: encodeAddress(pair.publicKey, 0),
      substrate: encodeAddress(pair.publicKey, 42),
    };
  });
  console.log(JSON.stringify({
    provenance: '@polkadot/util-crypto 14.0.3, @polkadot/keyring 14.0.3, @polkadot/wasm-crypto 7.5.4 (scripts/generate-substrate-path-vectors.cjs)',
    phrase: PHRASE,
    vectors,
  }, null, 2));
})();
