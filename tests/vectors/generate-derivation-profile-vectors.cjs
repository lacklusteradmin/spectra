// Independent addresses for every derivation profile the registry lists, at
// accounts 0 and 1, for core/tests/fixtures/derivation-profiles.json.
// node tests/vectors/generate-derivation-profile-vectors.cjs > core/tests/fixtures/derivation-profiles.json
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
const { derivePath: slip10, getPublicKey: ed25519Public } = require('ed25519-hd-key');
const { Keypair: SolanaKeypair } = require('@solana/web3.js');
const { Keypair: StellarKeypair } = require('@stellar/stellar-base');
const rippleKeypairs = require('ripple-keypairs');
const { Ed25519Keypair } = require('@mysten/sui/keypairs/ed25519');
const { Account } = require('@aptos-labs/ts-sdk');
const { parseSeedPhrase } = require('near-seed-phrase');
const { Secp256k1KeyIdentity } = require('@dfinity/identity-secp256k1');
const CSL = require('@emurgo/cardano-serialization-lib-nodejs');
const blake = require('blake-hash/js');
const { ripemd160 } = require('@noble/hashes/legacy.js');
const bs58check = require('bs58check').default;
const bs58 = require('bs58').default;
const kaspa = require('kaspa-wasm');
const { toBech32 } = require('@cosmjs/encoding');
const ecash = require('ecash-lib');
const ecashaddr = require('ecashaddrjs');
const { Address: VechainAddress, HDKey: VechainHDKey } = require('@vechain/sdk-core');
const { newSecp256k1Address } = require('@glif/filecoin-address');
const { getPkhfromPk, b58Encode, PrefixV2 } = require('@taquito/utils');
const { HDKey } = require('@scure/bip32');
const { CallData, ec, encode, hash: starknetHash, num } = require('starknet');

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
  // Qtum Core's and DigiByte Core's chainparams; Fractal keeps Bitcoin's.
  'qtum': net('qc', 0x3a, 0x32),
  'fractal-bitcoin': bitcoin.networks.bitcoin,
  'digibyte': net('dgb', 0x1e, 0x3f),
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
// eCash: ecash-lib derives the key from the phrase on its own, and its
// CashAddr encoding must agree with ecashaddrjs's.
const ecashAddress = (path) => {
  const pubkey = ecash.HdNode.fromSeed(ecash.mnemonicToSeed(PHRASE)).derivePath(path).pubkey();
  const hash = ecash.shaRmd160(pubkey);
  const address = ecash.Address.p2pkh(hash).toString();
  if (address !== ecashaddr.encodeCashAddress('ecash', 'p2pkh', Buffer.from(hash).toString('hex'))) {
    throw new Error('eCash CashAddr mismatch');
  }
  if (Buffer.compare(Buffer.from(pubkey), Buffer.from(secpKey(path).publicKey)) !== 0) throw new Error('eCash key mismatch');
  return address;
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
// A tz1 account: Taquito's hash of the SLIP-10 key's edpk.
const tezos = (path) => getPkhfromPk(b58Encode(ed25519Public(ed25519Seed(path), false), PrefixV2.Ed25519PublicKey));
const near = (path) => Buffer.from(bs58.decode(parseSeedPhrase(PHRASE, path).publicKey.replace('ed25519:', ''))).toString('hex');
// A Hedera key names no account: what a profile derives is the public key
// an account is found by, in the DER Hedera's SDK writes. Ed25519 by SLIP-10,
// ECDSA secp256k1 by BIP-32, as @hashgraph/sdk's Mnemonic derives them.
const hederaEd25519 = (path) => '302a300506032b6570032100' + Buffer.from(ed25519Public(ed25519Seed(path), false)).toString('hex');
const hederaEcdsa = (path) => '302d300706052b8104000a032200' + Buffer.from(secpKey(path).publicKey).toString('hex');
// DFINITY's own phrase identity is the key at account 0; other accounts are
// the same BIP-32 key at their own account index.
if (Secp256k1KeyIdentity.fromSeedPhrase(PHRASE).getPrincipal().toText()
    !== Secp256k1KeyIdentity.fromSecretKey(secpKey("m/44'/223'/0'/0/0").privateKey).getPrincipal().toText()) {
  throw new Error('ICP phrase identity');
}
const icp = (path) => {
  const principal = Secp256k1KeyIdentity.fromSecretKey(secpKey(path).privateKey).getPrincipal().toUint8Array();
  const hash = crypto.createHash('sha224').update(Buffer.concat([Buffer.from('\x0aaccount-id'), principal, Buffer.alloc(32)])).digest();
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(require('node:zlib').crc32(hash));
  return Buffer.concat([crc, hash]).toString('hex');
};
// A phrase's base address: the payment key's hash, then that of the
// account's CIP-1852 stake key at role 2, index 0.
const cardano = (mainnet) => (path) => {
  const root = CSL.Bip32PrivateKey.from_bip39_entropy(Buffer.from(bip39.mnemonicToEntropy(PHRASE), 'hex'), Buffer.alloc(0));
  const credential = (keyPath) => {
    let key = root;
    for (const segment of keyPath.split('/').slice(1)) {
      const index = parseInt(segment, 10);
      key = key.derive(segment.endsWith("'") ? (index | 0x80000000) >>> 0 : index);
    }
    return CSL.Credential.from_keyhash(key.to_public().to_raw_key().hash());
  };
  const stakePath = path.split('/').slice(0, 4).concat(['2', '0']).join('/');
  return CSL.BaseAddress.new(mainnet ? 1 : 0, credential(path), credential(stakePath)).to_address().to_bech32();
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
  'polygon', 'polygon-amoy', 'linea', 'linea-sepolia', 'scroll', 'mantle', 'sei', 'celo', 'celo-sepolia',
  'cronos', 'cronos-testnet', 'opbnb', 'zksync-era', 'zksync-era-sepolia', 'sonic', 'sonic-testnet', 'berachain',
  'unichain', 'ink', 'ink-sepolia', 'x-layer', 'x-layer-testnet', 'plasma', 'monad', 'world-chain',
  'kaia', 'xdc', 'robinhood-chain', 'kava-evm', 'chiliz', 'stable', 'arc', 'manta-pacific', 'metis', 'flare',
  'katana', 'megaeth', 'merlin', 'peaq', 'somnia', '0g',
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
  // Qtum Core's descriptor wallets and Qtum Electrum derive at coin type 88.
  'qtum': [
    ['legacy', "m/44'/88'/{account}'/0/0", utxo('qtum', 'p2pkh')],
    ['nativeSegWit', "m/84'/88'/{account}'/0/0", utxo('qtum', 'p2wpkh')],
    ['nestedSegWit', "m/49'/88'/{account}'/0/0", utxo('qtum', 'p2sh-p2wpkh')],
  ],
  // UniSat derives Fractal accounts at Bitcoin's paths.
  'fractal-bitcoin': BTC_PROFILES(0, 'fractal-bitcoin'),
  // Cashtab's m/44'/1899', and m/44'/899', Electrum ABC's BIP-39 default and Trezor's.
  'ecash': [
    ['standard', "m/44'/1899'/{account}'/0/0", ecashAddress],
    ['legacy', "m/44'/899'/{account}'/0/0", ecashAddress],
  ],
  'digibyte': BTC_PROFILES(20, 'digibyte'),
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
  'tezos': [['standard', "m/44'/1729'/{account}'/0'", tezos]],
  'hedera': [
    ['standard', "m/44'/3030'/0'/0'/{account}'", hederaEd25519],
    ['ecdsa', "m/44'/3030'/0'/0/{account}", hederaEcdsa],
    ['evm', "m/44'/60'/0'/0/{account}", hederaEcdsa],
  ],
};
for (const chain of EVM_CHAINS) PROFILES[chain] = [['standard', "m/44'/60'/{account}'/0/0", evm]];
// Cosmos SDK accounts: the key's RIPEMD-160 of SHA-256 under the network's
// prefix, or (Ethermint) its Ethereum address under it, at Keplr's paths.
const cosmos = (hrp) => (path) => toBech32(hrp, hash160(secpKey(path).publicKey));
const cosmosEth = (hrp) => (path) => toBech32(hrp, Buffer.from(evm(path).slice(2), 'hex'));
const COSMOS_CHAINS = [
  ['cosmos-hub', 118, cosmos('cosmos')], ['celestia', 118, cosmos('celestia')], ['dydx', 118, cosmos('dydx')],
  ['terra', 330, cosmos('terra')], ['kava', 459, cosmos('kava')], ['injective', 60, cosmosEth('inj')],
  ['akash', 118, cosmos('akash')], ['babylon', 118, cosmos('bbn')], ['osmosis', 118, cosmos('osmo')],
  ['dymension', 60, cosmosEth('dym')], ['saga', 118, cosmos('saga')], ['allora', 118, cosmos('allo')],
  ['band', 494, cosmos('band')],
];
for (const [chain, coin, encode] of COSMOS_CHAINS) PROFILES[chain] = [['standard', `m/44'/${coin}'/{account}'/0/0`, encode]];
// A Filecoin f1 address hashes the 65-byte uncompressed key. Ledger Live's
// BIP-44 mode puts the account here; Glif's puts it in the last segment, and
// the two agree at account 0.
const filecoin = (path) => newSecp256k1Address(ecc.pointCompress(secpKey(path).publicKey, false)).toString();
PROFILES['filecoin'] = [['standard', "m/44'/461'/{account}'/0/0", filecoin]];
// Ready (formerly Argent X): the phrase's Ethereum key, its leading zero
// bytes dropped, seeds a second BIP-32 tree whose child at the path is ground
// into a Stark key; the account is Argent 0.4.0 owned by it, salted with it.
const starknet = (path) => {
  const secret = '0x' + Buffer.from(ROOT.derivePath("m/44'/60'/0'/0/0").privateKey).toString('hex');
  const seed = encode.sanitizeBytes(encode.removeHexPrefix(num.toHex(secret)), 2);
  const child = HDKey.fromMasterSeed(Buffer.from(seed, 'hex')).derive(path);
  const publicKey = ec.starkCurve.getStarkKey('0x' + ec.starkCurve.grindKey(child.privateKey));
  const address = starknetHash.calculateContractAddressFromHash(
    publicKey, '0x036078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f', CallData.compile([0, publicKey, 1]), 0,
  );
  return '0x' + BigInt(address).toString(16).padStart(64, '0');
};
PROFILES.starknet = [['standard', "m/44'/9004'/0'/0/{account}", starknet]];

// VeWorld and Sync2 derive VeChainThor accounts as addresses under
// `m/44'/818'/0'/0`, the account index being the last, unhardened segment.
const vechain = (path) => VechainAddress.ofPublicKey(VechainHDKey.fromMnemonic(PHRASE.split(' '), path).publicKey).toString();
PROFILES.vechain = [['standard', "m/44'/818'/0'/0/{account}", vechain]];

(async () => {
  // Pera's Universal Wallet: ARC-52 (BIP32-Ed25519, Peikert) through the
  // Algorand Foundation's xhd-wallet-api, whose derivation is asynchronous,
  // so its addresses are read before the synchronous walk.
  const xhd = await import('@algorandfoundation/xhd-wallet-api');
  const algosdk = await import('algosdk');
  const algorandRoot = xhd.fromSeed(SEED);
  const algorandAddresses = {};
  for (const account of [0, 1]) {
    const key = await new xhd.XHDWalletAPI().keyGen(algorandRoot, xhd.KeyContext.Address, account, 0, xhd.BIP32DerivationType.Peikert);
    algorandAddresses[`m/44'/283'/${account}'/0/0`] = algosdk.encodeAddress(key);
  }
  PROFILES['algorand'] = [['standard', "m/44'/283'/{account}'/0/0", (path) => algorandAddresses[path]]];

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
      + '@solana/web3.js 1.98.4, @stellar/stellar-base 14.1.0, ripple-keypairs 3.1.0, @mysten/sui 1.45.2, '
      + '@aptos-labs/ts-sdk 5.1.1, near-seed-phrase 0.2.1, @dfinity/identity-secp256k1 3.4.3, '
      + '@emurgo/cardano-serialization-lib-nodejs 17.0.0, blake-hash 2.0.0, kaspa-wasm 0.13.0, '
      + '@cosmjs/encoding 0.39.0, @algorandfoundation/xhd-wallet-api 2.0.0-canary.1, algosdk 3.9.0, '
      + 'ecash-lib 4.12.0, ecashaddrjs 2.0.1, @vechain/sdk-core 2.1.0, '
      + '@glif/filecoin-address 4.0.0, @taquito/utils 25.1.1, @scure/bip32 1.7.0, starknet 11.6.1',
    phrase: PHRASE,
    vectors,
  }, null, 2));
})();
