// Message signatures from Cardano's, Kaspa's and Monero's own SDKs:
// CIP-8/CIP-30 `signData` (COSE_Sign1 with the address in the protected
// header), Kaspa's `signMessage` (Schnorr over the personal-message hash)
// and Monero's `SigV2` (wallet2's spend-key signature).
// npm install --prefix /tmp/spectra-message-vectors-2 --ignore-scripts @emurgo/cardano-message-signing-nodejs@1.1.0 @emurgo/cardano-serialization-lib-nodejs@15.0.3 kaspa-wasm@0.13.0 monero-ts@0.11.20
// NODE_PATH=/tmp/spectra-message-vectors-2/node_modules node scripts/generate-more-message-signature-vectors.cjs
const fs = require('node:fs');
const msg = require('@emurgo/cardano-message-signing-nodejs');
const csl = require('@emurgo/cardano-serialization-lib-nodejs');
const kaspa = require('kaspa-wasm');
const monero = require('monero-ts');

const MESSAGES = ['Hello World', 'Spectra 证明 ✓\nline two', ''];
const witness = JSON.parse(fs.readFileSync('core/tests/fixtures/cardano-emurgo-witness.json', 'utf8'));
const MONERO_SEED = 'syndrome portents apex vivid flippant dizzy bumper duplex enjoy deodorant bunch pigment wolf muppet tuition wept ailments kiwi roles against today morsel eternal excess wolf';

function cardano(message) {
  const key = csl.PrivateKey.from_extended_bytes(Buffer.from(witness.privateKey, 'hex'));
  const headers = msg.HeaderMap.new();
  headers.set_algorithm_id(msg.Label.from_algorithm_id(msg.AlgorithmId.EdDSA));
  headers.set_header(msg.Label.new_text('address'), msg.CBORValue.new_bytes(Buffer.from(witness.addressBytes, 'hex')));
  const unprotected = msg.HeaderMap.new();
  unprotected.set_header(msg.Label.new_text('hashed'), msg.CBORValue.new_special(msg.CBORSpecial.new_bool(false)));
  const builder = msg.COSESign1Builder.new(
    msg.Headers.new(msg.ProtectedHeaderMap.new(headers), unprotected), Buffer.from(message, 'utf8'), false);
  const signature = key.sign(builder.make_data_to_sign().to_bytes());
  const sign1 = builder.build(signature.to_bytes());
  const cose = msg.COSEKey.new(msg.Label.from_key_type(msg.KeyType.OKP));
  cose.set_algorithm_id(msg.Label.from_algorithm_id(msg.AlgorithmId.EdDSA));
  cose.set_header(msg.Label.new_int(msg.Int.new_negative(msg.BigNum.from_str('1'))), msg.CBORValue.new_int(msg.Int.new_i32(6)));
  cose.set_header(msg.Label.new_int(msg.Int.new_negative(msg.BigNum.from_str('2'))), msg.CBORValue.new_bytes(key.to_public().as_bytes()));
  return {
    address: witness.address, key: witness.privateKey, message,
    signature: JSON.stringify({ signature: Buffer.from(sign1.to_bytes()).toString('hex'), key: Buffer.from(cose.to_bytes()).toString('hex') }),
  };
}

function kaspaVector(message) {
  const privateKey = new kaspa.PrivateKey('01'.repeat(32));
  const address = privateKey.toKeypair().toAddress('mainnet').toString();
  const signature = kaspa.signMessage({ message, privateKey });
  if (!kaspa.verifyMessage({ message, signature, publicKey: privateKey.toKeypair().publicKey })) throw new Error('kaspa verify');
  return { address, key: '01'.repeat(32), message, signature };
}

async function main() {
  const wallet = await monero.createWalletFull({ path: '', password: 'x', networkType: monero.MoneroNetworkType.MAINNET,
    seed: MONERO_SEED, restoreHeight: 0, proxyToWorker: false });
  const address = await wallet.getPrimaryAddress();
  const key = (await wallet.getPrivateSpendKey()) + (await wallet.getPrivateViewKey());
  const moneroVectors = [];
  for (const message of MESSAGES) {
    const signature = await wallet.signMessage(message, monero.MoneroMessageSignatureType.SIGN_WITH_SPEND_KEY, 0, 0);
    moneroVectors.push({ address, key, message, signature });
  }
  fs.writeFileSync('core/tests/fixtures/message-signatures-more.json', JSON.stringify({
    provenance: '@emurgo/cardano-message-signing-nodejs 1.1.0, @emurgo/cardano-serialization-lib-nodejs 15.0.3, kaspa-wasm 0.13.0, monero-ts 0.11.20',
    cardano: MESSAGES.map(cardano),
    kaspa: MESSAGES.map(kaspaVector),
    monero: moneroVectors,
  }, null, 2) + '\n');
  process.exit(0);
}
main().catch((error) => { console.error(error); process.exit(1); });
