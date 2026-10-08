// The CIP-19 reward (stake) address behind each Shelley address that has one,
// as cardano-serialization-lib builds them: a phrase's base address at
// accounts 0 and 1 on mainnet and Preprod, whose stake key sits at
// m/1852'/1815'/account'/2/0, and base addresses of every credential mix.
// Enterprise and pointer addresses carry no stake credential to name.
// npm install --prefix /tmp/spectra-cardano-vectors --ignore-scripts @emurgo/cardano-serialization-lib-nodejs@17.0.0 bip39@3.1.0
// NODE_PATH=/tmp/spectra-cardano-vectors/node_modules node scripts/generate-cardano-stake-address-vectors.cjs
const fs = require('node:fs');
const bip39 = require('bip39');
const CSL = require('@emurgo/cardano-serialization-lib-nodejs');

const PHRASE = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const harden = (n) => 0x80000000 + n;
const root = CSL.Bip32PrivateKey.from_bip39_entropy(Buffer.from(bip39.mnemonicToEntropy(PHRASE), 'hex'), Buffer.alloc(0));
const keyCredential = (key) => CSL.Credential.from_keyhash(key.to_public().to_raw_key().hash());
const scriptCredential = (byte) => CSL.Credential.from_scripthash(CSL.ScriptHash.from_bytes(Buffer.alloc(28, byte)));
const reward = (network, stake) => CSL.RewardAddress.new(network, stake).to_address().to_bech32();

const phrase = [];
for (const [chain, network] of [['cardano', 1], ['cardano-preprod', 0]]) {
  for (const account of [0, 1]) {
    const keys = root.derive(harden(1852)).derive(harden(1815)).derive(harden(account));
    const stake = keyCredential(keys.derive(2).derive(0));
    phrase.push({
      chain,
      account,
      address: CSL.BaseAddress.new(network, keyCredential(keys.derive(0).derive(0)), stake).to_address().to_bech32(),
      stake_address: reward(network, stake),
    });
  }
}

// Every base-address credential mix names its stake credential, a key's as
// type 14 and a script's as type 15.
const keys = root.derive(harden(1852)).derive(harden(1815)).derive(harden(0));
const paymentKey = keyCredential(keys.derive(0).derive(0));
const stakeKey = keyCredential(keys.derive(2).derive(0));
const shapes = [];
for (const network of [1, 0]) {
  for (const [payment, stake] of [[paymentKey, stakeKey], [scriptCredential(7), stakeKey],
    [paymentKey, scriptCredential(9)], [scriptCredential(7), scriptCredential(9)]]) {
    shapes.push({
      address: CSL.BaseAddress.new(network, payment, stake).to_address().to_bech32(),
      stake_address: reward(network, stake),
    });
  }
  shapes.push({ address: CSL.EnterpriseAddress.new(network, paymentKey).to_address().to_bech32(), stake_address: null });
  shapes.push({
    address: CSL.PointerAddress.new(network, paymentKey,
      CSL.Pointer.new_pointer(CSL.BigNum.from_str('2498243'), CSL.BigNum.from_str('27'), CSL.BigNum.from_str('3')))
      .to_address().to_bech32(),
    stake_address: null,
  });
}

fs.writeFileSync('core/tests/fixtures/cardano-stake-addresses.json', JSON.stringify({
  provenance: '@emurgo/cardano-serialization-lib-nodejs 17.0.0, bip39 3.1.0',
  phrase: PHRASE,
  phrase_vectors: phrase,
  address_vectors: shapes,
}, null, 2) + '\n');
