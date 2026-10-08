// Independent account public keys, for core/tests/fixtures/account-keys.json:
// the abandon phrase's account 0 on each account UTXO network, written in
// each encoding its wallets read.
// Install the pinned libraries in a temporary directory:
// npm install --prefix /tmp/spectra-account-keys --ignore-scripts bip39@3.1.0 bip32@5.0.1 tiny-secp256k1@2.2.4
// NODE_PATH=/tmp/spectra-account-keys/node_modules node scripts/generate-account-key-vectors.cjs > core/tests/fixtures/account-keys.json
//
// The versions are transcribed from their sources, not from chains.toml or
// the registry: SLIP-132 (Bitcoin, Litecoin's Ltub, Mtub and ttub), Trezor's
// coin definitions (trezor-firmware common/defs/bitcoin: Litecoin's zpub and
// testnet keys, Dogecoin's dgub, Dash's drkp, Decred's dpub and testnet tpub,
// and the xpub of Bitcoin Cash, Bitcoin Gold, Peercoin and Zcash), Dash
// Core's and Dogecoin Core's chainparams (Dash's xpub, Dogecoin's testnet
// tpub), ElectrumSV's xpub on Bitcoin SV, and rusty-kaspa's
// wallet/bip32/src/prefix.rs (kpub, ktub). Each key's first receive address
// is core/tests/fixtures/derivation-profiles.json's address for the same
// network, profile and account 0.
const bip39 = require('bip39');
const ecc = require('tiny-secp256k1');
const { BIP32Factory } = require('bip32');

const bip32 = BIP32Factory(ecc);
const PHRASE = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const SEED = bip39.mnemonicToSeedSync(PHRASE);

const XPUB = 0x0488b21e, YPUB = 0x049d7cb2, ZPUB = 0x04b24746;
const TPUB = 0x043587cf, UPUB = 0x044a5262, VPUB = 0x045f1cf6;
// network: [[profile, purpose, version, prefix], ...] with the coin type.
const NETWORKS = {
  bitcoin: [0, [['legacy', 44, XPUB, 'xpub'], ['nestedSegWit', 49, YPUB, 'ypub'], ['nativeSegWit', 84, ZPUB, 'zpub']]],
  'bitcoin-testnet': [1, [['legacy', 44, TPUB, 'tpub'], ['nestedSegWit', 49, UPUB, 'upub'], ['nativeSegWit', 84, VPUB, 'vpub']]],
  'bitcoin-testnet-4': [1, [['legacy', 44, TPUB, 'tpub'], ['nestedSegWit', 49, UPUB, 'upub'], ['nativeSegWit', 84, VPUB, 'vpub']]],
  'bitcoin-signet': [1, [['legacy', 44, TPUB, 'tpub'], ['nestedSegWit', 49, UPUB, 'upub'], ['nativeSegWit', 84, VPUB, 'vpub']]],
  litecoin: [2, [['legacy', 44, 0x019da462, 'Ltub'], ['nestedSegWit', 49, 0x01b26ef6, 'Mtub'], ['nativeSegWit', 84, ZPUB, 'zpub']]],
  'litecoin-testnet': [1, [['legacy', 44, TPUB, 'tpub'], ['nestedSegWit', 49, UPUB, 'upub'], ['nativeSegWit', 84, VPUB, 'vpub'], ['legacy', 44, 0x0436f6e1, 'ttub']]],
  peercoin: [6, [['legacy', 44, XPUB, 'xpub'], ['nestedSegWit', 49, YPUB, 'ypub'], ['nativeSegWit', 84, ZPUB, 'zpub']]],
  'peercoin-testnet': [1, [['legacy', 44, TPUB, 'tpub'], ['nestedSegWit', 49, UPUB, 'upub'], ['nativeSegWit', 84, VPUB, 'vpub']]],
  'bitcoin-cash': [145, [['standard', 44, XPUB, 'xpub']]],
  'bitcoin-cash-testnet': [1, [['standard', 44, TPUB, 'tpub']]],
  'bitcoin-sv': [236, [['standard', 44, XPUB, 'xpub']]],
  'bitcoin-sv-testnet': [1, [['standard', 44, TPUB, 'tpub']]],
  'bitcoin-gold': [156, [['standard', 44, XPUB, 'xpub']]],
  zcash: [133, [['standard', 44, XPUB, 'xpub']]],
  'zcash-testnet': [1, [['standard', 44, TPUB, 'tpub']]],
  dash: [5, [['standard', 44, XPUB, 'xpub'], ['standard', 44, 0x02fe52cc, 'drkp']]],
  'dash-testnet': [1, [['standard', 44, TPUB, 'tpub']]],
  dogecoin: [3, [['standard', 44, 0x02facafd, 'dgub']]],
  'dogecoin-testnet': [1, [['standard', 44, TPUB, 'tpub']]],
  decred: [42, [['standard', 44, 0x02fda926, 'dpub']]],
  'decred-testnet': [1, [['standard', 44, 0x043587d1, 'tpub']]],
  kaspa: [111111, [['standard', 44, 0x038f332e, 'kpub']]],
  'kaspa-testnet': [111111, [['standard', 44, 0x0390a241, 'ktub']]],
};

const vectors = [];
for (const [chain, [coin, encodings]] of Object.entries(NETWORKS)) {
  for (const [profile, purpose, version, prefix] of encodings) {
    const network = { bip32: { public: version, private: 0 }, wif: 0, messagePrefix: '', pubKeyHash: 0, scriptHash: 0 };
    const account = bip32.fromSeed(SEED, network).derivePath(`m/${purpose}'/${coin}'/0'`).neutered();
    const xpub = account.toBase58();
    if (!xpub.startsWith(prefix)) throw new Error(`${chain} ${prefix} encodes as ${xpub.slice(0, 4)}`);
    vectors.push({ chain, profile, prefix, xpub });
  }
}
console.log(JSON.stringify({
  provenance: 'bip39 3.1.0, bip32 5.0.1, tiny-secp256k1 2.2.4 (scripts/generate-account-key-vectors.cjs)',
  phrase: PHRASE,
  vectors,
}, null, 2));
