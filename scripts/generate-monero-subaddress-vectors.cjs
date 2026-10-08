// Independent Monero subaddresses, for core/tests/fixtures/monero-subaddresses.json:
// wallet2's own derivation, compiled to WebAssembly by monero-ts, for the
// test phrase on mainnet and stagenet.
// Install the pinned library in a temporary directory:
// npm install --prefix /tmp/spectra-monero-ts --ignore-scripts monero-ts@0.11.15
// NODE_PATH=/tmp/spectra-monero-ts/node_modules node scripts/generate-monero-subaddress-vectors.cjs > core/tests/fixtures/monero-subaddresses.json
const moneroTs = require('monero-ts');

const PHRASE = 'syndrome portents apex vivid flippant dizzy bumper duplex enjoy deodorant bunch pigment wolf muppet tuition wept ailments kiwi roles against today morsel eternal excess wolf';
// (account, address) pairs: the primary address, the first of account 0,
// another account's first and primary, wallet2's lookahead edges (199, 49)
// and indices past one byte and two.
const INDICES = [[0, 0], [0, 1], [0, 2], [0, 199], [0, 200], [1, 0], [1, 1], [2, 7], [49, 0], [49, 199], [50, 0], [0, 300], [3, 70000]];

(async () => {
  const networks = [];
  for (const [chain, networkType] of [['monero', moneroTs.MoneroNetworkType.MAINNET], ['monero-stagenet', moneroTs.MoneroNetworkType.STAGENET]]) {
    const wallet = await moneroTs.createWalletKeys({ networkType, seed: PHRASE });
    const subaddresses = [];
    for (const [account, address] of INDICES) {
      subaddresses.push({ account, address, encoded: await wallet.getAddress(account, address) });
    }
    networks.push({
      chain,
      primary: await wallet.getPrimaryAddress(),
      public_spend_key: await wallet.getPublicSpendKey(),
      private_view_key: await wallet.getPrivateViewKey(),
      public_view_key: await wallet.getPublicViewKey(),
      subaddresses,
    });
  }
  console.log(JSON.stringify({
    provenance: 'monero-ts 0.11.15, wallet2 compiled to WebAssembly (scripts/generate-monero-subaddress-vectors.cjs)',
    phrase: PHRASE,
    networks,
  }, null, 2));
  process.exit(0);
})().catch((error) => { console.error(error); process.exit(1); });
