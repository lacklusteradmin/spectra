// Independent message signatures for core/tests/fixtures/message-signatures.json.
// Install the pinned SDKs in a temporary directory:
// npm install --prefix /tmp/spectra-message-vectors --ignore-scripts bitcoinjs-message@2.2.0 bip322-js@2.0.0 @bitcoinerlab/secp256k1@1.2.0 ethers@6.17.0 tronweb@6.0.4 @mysten/sui@1.38.0 @polkadot/util-crypto@13.5.6 @polkadot/keyring@13.5.6 @polkadot/util@13.5.6
// NODE_PATH=/tmp/spectra-message-vectors/node_modules node scripts/generate-message-signature-vectors.cjs > core/tests/fixtures/message-signatures.json
const crypto = require('node:crypto');
const bitcoinMessage = require('bitcoinjs-message');
const { Signer: Bip322Signer, Verifier: Bip322Verifier } = require('bip322-js');
const bitcoin = require('bitcoinjs-lib');
const { ECPairFactory } = require('ecpair');
const ecc = require('@bitcoinerlab/secp256k1');
const { ethers } = require('ethers');
const { TronWeb } = require('tronweb');
const { Ed25519Keypair } = require('@mysten/sui/keypairs/ed25519');
const { Keyring } = require('@polkadot/keyring');
const { cryptoWaitReady } = require('@polkadot/util-crypto');
const { u8aWrapBytes, u8aToHex } = require('@polkadot/util');

bitcoin.initEccLib(ecc);
const ECPair = ECPairFactory(ecc);

// A fixed key per label, so the fixtures do not change between runs.
const key = (label) => crypto.createHash('sha256').update('spectra-message-' + label).digest();
const MESSAGES = ['', 'Hello World', 'Spectra 署名 ✓\nline two'];

// Each network's signed-message magic, as its own node writes it, and the
// Base58Check P2PKH version and WIF version its addresses and keys carry.
const LEGACY = {
  'bitcoin': { magic: 'Bitcoin Signed Message:\n', p2pkh: 0x00, wif: 0x80 },
  'bitcoin-cash': { magic: 'Bitcoin Signed Message:\n', p2pkh: 0x00, wif: 0x80 },
  'bitcoin-sv': { magic: 'Bitcoin Signed Message:\n', p2pkh: 0x00, wif: 0x80 },
  'litecoin': { magic: 'Litecoin Signed Message:\n', p2pkh: 0x30, wif: 0xb0 },
  'dogecoin': { magic: 'Dogecoin Signed Message:\n', p2pkh: 0x1e, wif: 0x9e },
  'dash': { magic: 'DarkCoin Signed Message:\n', p2pkh: 0x4c, wif: 0xcc },
  'bitcoin-gold': { magic: 'Bitcoin Gold Signed Message:\n', p2pkh: 0x26, wif: 0x80 },
  'peercoin': { magic: 'Peercoin Signed Message:\n', p2pkh: 0x37, wif: 0xb7 },
};

// The prefix bitcoinjs-message takes: the magic's length as a varint, then
// the magic. A string, because `sign` reads an object in that place as its
// signing options and falls back to Bitcoin's prefix.
const prefix = (magic) => String.fromCharCode(magic.length) + magic;

async function main() {
  await cryptoWaitReady();
  const out = {
    provenance:
      'bitcoinjs-message 2.2.0, bip322-js 2.0.0, ethers 6.17.0, tronweb 6.0.4, @mysten/sui 1.38.0, @polkadot/keyring 13.5.6, node:crypto ed25519',
    messages: MESSAGES,
    legacy: [],
    bip322: [],
    evm: [],
    tron: [],
    solana: [],
    sui: [],
    substrate: [],
  };

  for (const [chain, { magic, p2pkh, wif }] of Object.entries(LEGACY)) {
    const secret = key('legacy-' + chain);
    const pair = ECPair.fromPrivateKey(secret, { compressed: true });
    const network = { ...bitcoin.networks.bitcoin, pubKeyHash: p2pkh, wif };
    const address = bitcoin.payments.p2pkh({ pubkey: Buffer.from(pair.publicKey), network }).address;
    for (const message of MESSAGES) {
      const signature = bitcoinMessage.sign(message, secret, true, prefix(magic)).toString('base64');
      if (!bitcoinMessage.verify(message, address, signature, prefix(magic))) throw new Error(chain);
      out.legacy.push({ chain, key: secret.toString('hex'), address, message, signature });
    }
  }

  // BIP-322 simple signatures on Bitcoin's SegWit and Taproot addresses.
  const bip322Secret = key('bip322');
  const bip322Pair = ECPair.fromPrivateKey(bip322Secret, { compressed: true });
  const pubkey = Buffer.from(bip322Pair.publicKey);
  const addresses = {
    nativeSegWit: bitcoin.payments.p2wpkh({ pubkey }).address,
    nestedSegWit: bitcoin.payments.p2sh({ redeem: bitcoin.payments.p2wpkh({ pubkey }) }).address,
    taproot: bitcoin.payments.p2tr({ internalPubkey: pubkey.subarray(1, 33) }).address,
  };
  for (const [script, address] of Object.entries(addresses)) {
    for (const message of MESSAGES) {
      const signature = Bip322Signer.sign(bip322Pair.toWIF(), address, message);
      if (!Bip322Verifier.verifySignature(address, message, signature)) throw new Error(script);
      out.bip322.push({ script, key: bip322Secret.toString('hex'), address, message, signature });
    }
  }

  const evmSecret = key('evm');
  const evmWallet = new ethers.Wallet('0x' + evmSecret.toString('hex'));
  for (const message of MESSAGES) {
    out.evm.push({
      key: evmSecret.toString('hex'),
      address: evmWallet.address,
      message,
      signature: await evmWallet.signMessage(message),
    });
  }

  const tronSecret = key('tron');
  const tronWeb = new TronWeb({ fullHost: 'http://127.0.0.1:1' });
  for (const message of MESSAGES) {
    out.tron.push({
      key: tronSecret.toString('hex'),
      address: TronWeb.address.fromPrivateKey(tronSecret.toString('hex')),
      message,
      signature: tronWeb.trx.signMessageV2(message, tronSecret.toString('hex')),
    });
  }

  const solanaSecret = key('solana');
  const der = Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), solanaSecret]);
  const solanaKey = crypto.createPrivateKey({ key: der, format: 'der', type: 'pkcs8' });
  const spki = crypto.createPublicKey(solanaKey).export({ format: 'der', type: 'spki' });
  for (const message of MESSAGES) {
    out.solana.push({
      key: solanaSecret.toString('hex'),
      publicKey: spki.subarray(spki.length - 32).toString('hex'),
      message,
      signature: crypto.sign(null, Buffer.from(message, 'utf8'), solanaKey).toString('hex'),
    });
  }

  const suiSecret = key('sui');
  const suiPair = Ed25519Keypair.fromSecretKey(suiSecret);
  for (const message of MESSAGES) {
    const { signature } = await suiPair.signPersonalMessage(Buffer.from(message, 'utf8'));
    out.sui.push({
      key: suiSecret.toString('hex'),
      address: suiPair.toSuiAddress(),
      message,
      signature,
    });
  }

  // sr25519 signatures are randomized: these are for verifying, not for
  // reproducing.
  for (const [chain, ss58] of [['polkadot', 0], ['bittensor', 42]]) {
    const seed = key('substrate-' + chain);
    const pair = new Keyring({ type: 'sr25519', ss58Format: ss58 }).addFromSeed(seed);
    for (const message of MESSAGES) {
      out.substrate.push({
        chain,
        key: seed.toString('hex'),
        address: pair.address,
        message,
        signature: u8aToHex(pair.sign(u8aWrapBytes(message))),
      });
    }
  }

  process.stdout.write(JSON.stringify(out, null, 2) + '\n');
}

main();
