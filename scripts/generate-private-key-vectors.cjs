// Independent private-key encodings for core/tests/fixtures/private-key-formats.json.
// Install the pinned SDKs in a temporary directory:
// npm install --prefix /tmp/spectra-key-vectors --ignore-scripts wif@5.0.0 @solana/web3.js@1.98.4 @stellar/stellar-base@14.0.1 @mysten/sui@1.38.0 @aptos-labs/ts-sdk@5.1.1 @near-js/crypto@2.5.1 @near-js/utils@2.5.1
// NODE_PATH=/tmp/spectra-key-vectors/node_modules node scripts/generate-private-key-vectors.cjs > core/tests/fixtures/private-key-formats.json
const crypto = require('node:crypto');
const wif = require('wif');
const { Keypair: SolanaKeypair } = require('@solana/web3.js');
const { Keypair: StellarKeypair } = require('@stellar/stellar-base');
const { Ed25519Keypair } = require('@mysten/sui/keypairs/ed25519');
const { Account, Ed25519PrivateKey } = require('@aptos-labs/ts-sdk');
const { KeyPairEd25519 } = require('@near-js/crypto');
const { baseEncode } = require('@near-js/utils');

// A fixed key per label, so the fixtures do not change between runs.
const key = (label) => crypto.createHash('sha256').update('spectra-key-' + label).digest();

// The WIF version byte of each Base58Check-WIF network.
const WIF = {
  'bitcoin': 0x80, 'bitcoin-testnet': 0xef, 'bitcoin-testnet-4': 0xef, 'bitcoin-signet': 0xef,
  'bitcoin-cash': 0x80, 'bitcoin-cash-testnet': 0xef, 'bitcoin-sv': 0x80, 'bitcoin-sv-testnet': 0xef,
  'bitcoin-gold': 0x80, 'litecoin': 0xb0, 'litecoin-testnet': 0xef, 'dogecoin': 0x9e,
  'dogecoin-testnet': 0xf1, 'dash': 0xcc, 'dash-testnet': 0xef, 'zcash': 0x80, 'zcash-testnet': 0xef,
  'peercoin': 0xb7, 'peercoin-testnet': 0xef,
};

function ed25519Public(seed) {
  const der = Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), seed]);
  const publicDer = crypto.createPublicKey(crypto.createPrivateKey({ key: der, format: 'der', type: 'pkcs8' }))
    .export({ format: 'der', type: 'spki' });
  return publicDer.subarray(publicDer.length - 32);
}

const fixtures = {
  provenance: 'wif 5.0.0, @solana/web3.js 1.98.4, @stellar/stellar-base 14.0.1, @mysten/sui 1.38.0, @aptos-labs/ts-sdk 5.1.1, @near-js/crypto 2.5.1',
  wif: Object.entries(WIF).map(([chain, version]) => {
    const secret = key('wif-' + chain);
    return {
      chain, key: secret.toString('hex'),
      compressed: wif.encode({ version, privateKey: secret, compressed: true }),
      uncompressed: wif.encode({ version, privateKey: secret, compressed: false }),
    };
  }),
};

const solanaSeed = key('solana');
const solana = SolanaKeypair.fromSeed(solanaSeed);
fixtures.solana = {
  key: solanaSeed.toString('hex'),
  base58: require('bs58').default ? require('bs58').default.encode(solana.secretKey) : require('bs58').encode(solana.secretKey),
  json: JSON.stringify(Array.from(solana.secretKey)),
  address: solana.publicKey.toBase58(),
};

const stellarSeed = key('stellar');
const stellar = StellarKeypair.fromRawEd25519Seed(stellarSeed);
fixtures.stellar = { key: stellarSeed.toString('hex'), secret: stellar.secret(), address: stellar.publicKey() };

const suiSeed = key('sui');
const sui = Ed25519Keypair.fromSecretKey(suiSeed);
fixtures.sui = { key: suiSeed.toString('hex'), secret: sui.getSecretKey(), address: sui.toSuiAddress() };

const aptosSeed = key('aptos');
const aptosKey = new Ed25519PrivateKey('0x' + aptosSeed.toString('hex'));
fixtures.aptos = {
  key: aptosSeed.toString('hex'), secret: aptosKey.toAIP80String(),
  address: Account.fromPrivateKey({ privateKey: aptosKey }).accountAddress.toString(),
};

const nearSeed = key('near');
const near = new KeyPairEd25519(baseEncode(Buffer.concat([nearSeed, ed25519Public(nearSeed)])));
fixtures.near = {
  key: nearSeed.toString('hex'), secret: near.toString(),
  public_key: near.getPublicKey().toString(),
  implicit_account: Buffer.from(near.getPublicKey().data).toString('hex'),
};
// A NEAR key string whose public half belongs to another key.
const other = ed25519Public(key('near-other'));
fixtures.near_mismatched = 'ed25519:' + baseEncode(Buffer.concat([nearSeed, other]));

console.log(JSON.stringify(fixtures, null, 2));
