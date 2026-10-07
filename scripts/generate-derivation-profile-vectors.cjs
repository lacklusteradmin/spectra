// Independent addresses for every derivation profile the registry lists, at
// accounts 0 and 1, for core/tests/fixtures/derivation-profiles.json.
// Install the pinned libraries in a temporary directory:
// npm install --prefix /tmp/spectra-profile-vectors --ignore-scripts bip39@3.1.0 bip32@5.0.1 tiny-secp256k1@2.2.4 bitcoinjs-lib@7.0.2 bchaddrjs@0.5.2 ethers@6.17.0 ed25519-hd-key@2.0.0 @solana/web3.js@1.98.4 @stellar/stellar-base@14.0.1 ripple-keypairs@3.1.0 @mysten/sui@1.38.0 @aptos-labs/ts-sdk@5.1.1 near-seed-phrase@0.2.1 @dfinity/principal@3.4.3 @emurgo/cardano-serialization-lib-nodejs@17.0.0 blake-hash@2.0.0 @noble/hashes@2.4.0 bs58check@4.0.0 kaspa-wasm@0.13.0
// NODE_PATH=/tmp/spectra-profile-vectors/node_modules node scripts/generate-derivation-profile-vectors.cjs > core/tests/fixtures/derivation-profiles.json
//
// The paths here are written from each chain's wallet convention, not read
// from chains.toml: the core test holds that the catalog's profiles and these
// agree, path and address, in both directions.
const crypto = require('node:crypto');
const bip39 = require('bip39');
const ecc = require('tiny-secp256k1');
const { BIP32Factory } = require('bip32');
const bitcoin = require('bitcoinjs-lib');
const bchaddr = require('bchaddrjs');
const { ethers } = require('ethers');
const { derivePath: slip10 } = require('ed25519-hd-key');
const { Keypair: SolanaKeypair } = require('@solana/web3.js');
const { Keypair: StellarKeypair } = require('@stellar/stellar-base');
const rippleKeypairs = require('ripple-keypairs');
const { Ed25519Keypair } = require('@mysten/sui/keypairs/ed25519');
const { Account } = require('@aptos-labs/ts-sdk');
const { parseSeedPhrase } = require('near-seed-phrase');
const { Principal } = require('@dfinity/principal');
const CSL = require('@emurgo/cardano-serialization-lib-nodejs');
const blake = require('blake-hash/js');
const { ripemd160 } = require('@noble/hashes/legacy.js');
const bs58check = require('bs58check').default || require('bs58check');
const bs58 = require('bs58').default || require('bs58');
const kaspa = require('kaspa-wasm');

bitcoin.initEccLib(ecc);
const bip32 = BIP32Factory(ecc);
const PHRASE = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const SEED = bip39.mnemonicToSeedSync(PHRASE);
const ROOT = bip32.fromSeed(SEED);

const sha256 = (data) => crypto.createHash('sha256').update(data).digest();
const hash160 = (data) => Buffer.from(ripemd160(sha256(data)));
const secpKey = (path) => ROOT.derivePath(path);
const xOnly = (pub) => pub.subarray(1, 33);

// bitcoinjs network parameters per UTXO chain.
const net = (bech32, pubKeyHash, scriptHash) => ({
  messagePrefix: '', bech32, bip32: { public: 0x0488b21e, private: 0x0488ade4 }, pubKeyHash, scriptHash, wif: 0x80,
});
const NETWORKS = {
  'bitcoin': bitcoin.networks.bitcoin,
  'bitcoin-testnet': bitcoin.networks.testnet,
  'bitcoin-testnet-4': bitcoin.networks.testnet,
  'bitcoin-signet': bitcoin.networks.testnet,
  'litecoin': net('ltc', 0x30, 0x32),
  'litecoin-testnet': net('tltc', 0x6f, 0x3a),
  'peercoin': net('pc', 0x37, 0x75),
  'peercoin-testnet': net('tpc', 0x6f, 0xc4),
  'dogecoin': net(undefined, 0x1e, 0x16),
  'dogecoin-testnet': net(undefined, 0x71, 0xc4),
  'dash': net(undefined, 0x4c, 0x10),
  'dash-testnet': net(undefined, 0x8c, 0x13),
  'bitcoin-sv': bitcoin.networks.bitcoin,
  'bitcoin-sv-testnet': bitcoin.networks.testnet,
  'bitcoin-gold': net(undefined, 0x26, 0x17),
};
function utxo(chain, script) {
  const network = NETWORKS[chain];
  return (path) => {
    const pubkey = Buffer.from(secpKey(path).publicKey);
    switch (script) {
      case 'p2pkh': return bitcoin.payments.p2pkh({ pubkey, network }).address;
      case 'p2sh-p2wpkh': return bitcoin.payments.p2sh({ redeem: bitcoin.payments.p2wpkh({ pubkey, network }), network }).address;
      case 'p2wpkh': return bitcoin.payments.p2wpkh({ pubkey, network }).address;
      case 'p2tr': return bitcoin.payments.p2tr({ internalPubkey: xOnly(pubkey), network }).address;
    }
  };
}
// Spectra stores Bitcoin Cash addresses in the legacy encoding; the CashAddr
// round trip checks that the two encode one key hash.
const bitcoinCash = (testnet) => (path) => {
  const legacy = bitcoin.payments.p2pkh({ pubkey: Buffer.from(secpKey(path).publicKey), network: testnet ? bitcoin.networks.testnet : bitcoin.networks.bitcoin }).address;
  if (bchaddr.toLegacyAddress(bchaddr.toCashAddress(legacy)) !== legacy) throw new Error('CashAddr round trip');
  return legacy;
};
const evm = (path) => ethers.HDNodeWallet.fromSeed(SEED).derivePath(path).address;
const tron = (path) => {
  const evmAddress = ethers.HDNodeWallet.fromSeed(SEED).derivePath(path).address;
  return bs58check.encode(Buffer.concat([Buffer.from([0x41]), Buffer.from(evmAddress.slice(2), 'hex')]));
};
const xrp = (path) => rippleKeypairs.deriveAddress(Buffer.from(secpKey(path).publicKey).toString('hex').toUpperCase());
const zcash = (prefix) => (path) => bs58check.encode(Buffer.concat([Buffer.from(prefix), hash160(Buffer.from(secpKey(path).publicKey))]));
const decred = (prefix) => (path) => {
  const blake256 = (data) => blake('blake256').update(data).digest();
  const payload = Buffer.concat([Buffer.from(prefix), Buffer.from(ripemd160(blake256(Buffer.from(secpKey(path).publicKey))))]);
  return bs58.encode(Buffer.concat([payload, blake256(blake256(payload)).subarray(0, 4)]));
};
const kaspaAddress = (network) => (path) =>
  new kaspa.PublicKey(Buffer.from(secpKey(path).publicKey).toString('hex')).toAddress(network).toString();
// SLIP-10 ed25519 walks only hardened segments.
const hardened = (path) => path.split('/').map((s, i) => (i === 0 || s.endsWith("'") ? s : s + "'")).join('/');
const ed25519Seed = (path) => slip10(hardened(path), SEED.toString('hex')).key;
const solana = (path) => SolanaKeypair.fromSeed(ed25519Seed(path)).publicKey.toBase58();
const stellar = (path) => StellarKeypair.fromRawEd25519Seed(ed25519Seed(path)).publicKey();
const sui = (path) => Ed25519Keypair.deriveKeypair(PHRASE, path).toSuiAddress();
const aptos = (path) => Account.fromDerivationPath({ path, mnemonic: PHRASE }).accountAddress.toString();
const near = (path) => Buffer.from(bs58.decode(parseSeedPhrase(PHRASE, path).publicKey.replace('ed25519:', ''))).toString('hex');
const icp = (path) => {
  const publicKey = crypto.createPublicKey(crypto.createPrivateKey({
    key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), ed25519Seed(path)]), format: 'der', type: 'pkcs8',
  })).export({ format: 'der', type: 'spki' });
  const principal = Principal.selfAuthenticating(new Uint8Array(publicKey)).toUint8Array();
  const hash = crypto.createHash('sha224').update(Buffer.concat([Buffer.from('\x0aaccount-id'), principal, Buffer.alloc(32)])).digest();
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(require('node:zlib').crc32(hash));
  return Buffer.concat([crc, hash]).toString('hex');
};
const cardano = (mainnet) => (path) => {
  const root = CSL.Bip32PrivateKey.from_bip39_entropy(Buffer.from(bip39.mnemonicToEntropy(PHRASE), 'hex'), Buffer.alloc(0));
  let key = root;
  for (const segment of path.split('/').slice(1)) {
    const index = parseInt(segment, 10);
    key = key.derive(segment.endsWith("'") ? (index | 0x80000000) >>> 0 : index);
  }
  const credential = CSL.Credential.from_keyhash(key.to_public().to_raw_key().hash());
  return CSL.EnterpriseAddress.new(mainnet ? 1 : 0, credential).to_address().to_bech32();
};

// chain → [[profile, template, address encoder]], from each chain's wallets.
const BTC_PROFILES = (coin, chain) => [
  ['nativeSegWit', `m/84'/${coin}'/{account}'/0/0`, utxo(chain, 'p2wpkh')],
  ['legacy', `m/44'/${coin}'/{account}'/0/0`, utxo(chain, 'p2pkh')],
  ['nestedSegWit', `m/49'/${coin}'/{account}'/0/0`, utxo(chain, 'p2sh-p2wpkh')],
  ['taproot', `m/86'/${coin}'/{account}'/0/0`, utxo(chain, 'p2tr')],
];
const EVM_CHAINS = [
  'ethereum', 'ethereum-sepolia', 'ethereum-hoodi', 'arbitrum', 'arbitrum-sepolia', 'optimism', 'optimism-sepolia',
  'avalanche', 'avalanche-fuji', 'base', 'base-sepolia', 'bnb', 'bnb-testnet', 'hyperliquid', 'hyperliquid-testnet',
  'polygon', 'polygon-amoy', 'linea', 'linea-sepolia', 'scroll', 'blast', 'mantle', 'sei', 'celo', 'celo-sepolia',
  'cronos', 'cronos-testnet', 'opbnb', 'zksync-era', 'zksync-era-sepolia', 'sonic', 'sonic-testnet', 'berachain',
  'unichain', 'ink', 'ink-sepolia', 'x-layer', 'x-layer-testnet', 'plasma', 'monad', 'world-chain',
];
const PROFILES = {
  'bitcoin': BTC_PROFILES(0, 'bitcoin'),
  'bitcoin-testnet': BTC_PROFILES(1, 'bitcoin-testnet'),
  'bitcoin-testnet-4': BTC_PROFILES(1, 'bitcoin-testnet-4'),
  'bitcoin-signet': BTC_PROFILES(1, 'bitcoin-signet'),
  'litecoin': [
    ['legacy', "m/44'/2'/{account}'/0/0", utxo('litecoin', 'p2pkh')],
    ['nativeSegWit', "m/84'/2'/{account}'/0/0", utxo('litecoin', 'p2wpkh')],
    ['nestedSegWit', "m/49'/2'/{account}'/0/0", utxo('litecoin', 'p2sh-p2wpkh')],
  ],
  'litecoin-testnet': [
    ['legacy', "m/44'/1'/{account}'/0/0", utxo('litecoin-testnet', 'p2pkh')],
    ['nativeSegWit', "m/84'/1'/{account}'/0/0", utxo('litecoin-testnet', 'p2wpkh')],
    ['nestedSegWit', "m/49'/1'/{account}'/0/0", utxo('litecoin-testnet', 'p2sh-p2wpkh')],
  ],
  'peercoin': [
    ['legacy', "m/44'/6'/{account}'/0/0", utxo('peercoin', 'p2pkh')],
    ['nativeSegWit', "m/84'/6'/{account}'/0/0", utxo('peercoin', 'p2wpkh')],
    ['nestedSegWit', "m/49'/6'/{account}'/0/0", utxo('peercoin', 'p2sh-p2wpkh')],
    ['taproot', "m/86'/6'/{account}'/0/0", utxo('peercoin', 'p2tr')],
  ],
  'peercoin-testnet': [
    ['legacy', "m/44'/1'/{account}'/0/0", utxo('peercoin-testnet', 'p2pkh')],
    ['nativeSegWit', "m/84'/1'/{account}'/0/0", utxo('peercoin-testnet', 'p2wpkh')],
    ['nestedSegWit', "m/49'/1'/{account}'/0/0", utxo('peercoin-testnet', 'p2sh-p2wpkh')],
    ['taproot', "m/86'/1'/{account}'/0/0", utxo('peercoin-testnet', 'p2tr')],
  ],
  'bitcoin-cash': [
    ['standard', "m/44'/145'/{account}'/0/0", bitcoinCash(false)],
    ['legacy', "m/44'/0'/{account}'/0/0", bitcoinCash(false)],
  ],
  'bitcoin-cash-testnet': [['standard', "m/44'/1'/{account}'/0/0", bitcoinCash(true)]],
  'bitcoin-sv': [['standard', "m/44'/236'/{account}'/0/0", utxo('bitcoin-sv', 'p2pkh')]],
  'bitcoin-sv-testnet': [['standard', "m/44'/1'/{account}'/0/0", utxo('bitcoin-sv-testnet', 'p2pkh')]],
  'bitcoin-gold': [['standard', "m/44'/156'/{account}'/0/0", utxo('bitcoin-gold', 'p2pkh')]],
  'dogecoin': [['standard', "m/44'/3'/{account}'/0/0", utxo('dogecoin', 'p2pkh')]],
  'dogecoin-testnet': [['standard', "m/44'/1'/{account}'/0/0", utxo('dogecoin-testnet', 'p2pkh')]],
  'dash': [['standard', "m/44'/5'/{account}'/0/0", utxo('dash', 'p2pkh')]],
  'dash-testnet': [['standard', "m/44'/1'/{account}'/0/0", utxo('dash-testnet', 'p2pkh')]],
  'zcash': [['standard', "m/44'/133'/{account}'/0/0", zcash([0x1c, 0xb8])]],
  'zcash-testnet': [['standard', "m/44'/1'/{account}'/0/0", zcash([0x1d, 0x25])]],
  'decred': [['standard', "m/44'/42'/{account}'/0/0", decred([0x07, 0x3f])]],
  'decred-testnet': [['standard', "m/44'/1'/{account}'/0/0", decred([0x0f, 0x21])]],
  'kaspa': [['standard', "m/44'/111111'/{account}'/0/0", kaspaAddress('mainnet')]],
  'kaspa-testnet': [['standard', "m/44'/111111'/{account}'/0/0", kaspaAddress('testnet-10')]],
  'xrp': [['standard', "m/44'/144'/{account}'/0/0", xrp]],
  'xrp-testnet': [['standard', "m/44'/144'/{account}'/0/0", xrp]],
  'tron': [['standard', "m/44'/195'/{account}'/0/0", tron]],
  'tron-nile': [['standard', "m/44'/195'/{account}'/0/0", tron]],
  'ethereum-classic': [['standard', "m/44'/61'/{account}'/0/0", evm]],
  'ethereum-classic-mordor': [['standard', "m/44'/61'/{account}'/0/0", evm]],
  'solana': [['standard', "m/44'/501'/{account}'/0'", solana], ['legacy', "m/44'/501'/{account}'", solana]],
  'solana-devnet': [['standard', "m/44'/501'/{account}'/0'", solana], ['legacy', "m/44'/501'/{account}'", solana]],
  'stellar': [['standard', "m/44'/148'/{account}'", stellar]],
  'stellar-testnet': [['standard', "m/44'/148'/{account}'", stellar]],
  'sui': [['standard', "m/44'/784'/{account}'/0'/0'", sui]],
  'sui-testnet': [['standard', "m/44'/784'/{account}'/0'/0'", sui]],
  'aptos': [['standard', "m/44'/637'/{account}'/0'/0'", aptos]],
  'aptos-testnet': [['standard', "m/44'/637'/{account}'/0'/0'", aptos]],
  'near': [['standard', "m/44'/397'/{account}'", near]],
  'near-testnet': [['standard', "m/44'/397'/{account}'", near]],
  'internet-computer': [['standard', "m/44'/223'/{account}'/0/0", icp]],
  'cardano': [['standard', "m/1852'/1815'/{account}'/0/0", cardano(true)]],
  'cardano-preprod': [['standard', "m/1852'/1815'/{account}'/0/0", cardano(false)]],
};
for (const chain of EVM_CHAINS) PROFILES[chain] = [['standard', "m/44'/60'/{account}'/0/0", evm]];

const vectors = [];
for (const [chain, profiles] of Object.entries(PROFILES)) {
  for (const [profile, template, encode] of profiles) {
    for (const account of [0, 1]) {
      const path = template.replace('{account}', String(account));
      vectors.push({ chain, profile, account, path, address: encode(path) });
    }
  }
}
console.log(JSON.stringify({
  provenance: 'bip39 3.1.0, bip32 5.0.1, bitcoinjs-lib 7.0.2, bchaddrjs 0.5.2, ethers 6.17.0, ed25519-hd-key 2.0.0, '
    + '@solana/web3.js 1.98.4, @stellar/stellar-base 14.0.1, ripple-keypairs 3.1.0, @mysten/sui 1.38.0, '
    + '@aptos-labs/ts-sdk 5.1.1, near-seed-phrase 0.2.1, @dfinity/principal 3.4.3, '
    + '@emurgo/cardano-serialization-lib-nodejs 17.0.0, blake-hash 2.0.0, kaspa-wasm 0.13.0',
  phrase: PHRASE,
  vectors,
}, null, 2));
