// Sui native MultiSig accounts and a 2-of-3 SUI transfer from the Sui SDK,
// for core/tests/fixtures/sui-multisig.json.
// npm install --prefix /tmp/spectra-sui-multisig --ignore-scripts @mysten/sui@1.45.2
// NODE_PATH=/tmp/spectra-sui-multisig/node_modules node scripts/generate-sui-multisig-vectors.cjs > core/tests/fixtures/sui-multisig.json
//
// Three BIP-39 test phrases (empty passphrase) are the members, one per
// signature scheme at the SDK's default path: Ed25519 m/44'/784'/0'/0'/0',
// Secp256k1 m/54'/784'/0'/0/0 and Secp256r1 m/74'/784'/0'/0/0. The policy is
// threshold 2 with weights [1, 1, 2]. The transaction is the native transfer
// core's send::sui::prepare_transfer lays out: inputs [Pure(u64 amount),
// Pure(address recipient)], SplitCoins(GasCoin, [Input(0)]) then
// TransferObjects([NestedResult(0, 0)], Input(1)), one gas coin, the
// multisig address as sender and gas owner, no expiration. Each member signs
// blake2b-256([0, 0, 0] || bytes); secp256k1/r1 hash that digest with SHA-256
// and sign RFC 6979, low-S, so every signature is deterministic. The SDK
// combines partial signatures in the order given, which must be ascending
// member index. A second policy (threshold 3, weights [1, 1, 1]) is recorded
// by address only, for refusal tests.
const { Transaction } = require('@mysten/sui/transactions');
const { Ed25519Keypair } = require('@mysten/sui/keypairs/ed25519');
const { Secp256k1Keypair } = require('@mysten/sui/keypairs/secp256k1');
const { Secp256r1Keypair } = require('@mysten/sui/keypairs/secp256r1');
const { MultiSigPublicKey } = require('@mysten/sui/multisig');
const { decodeSuiPrivateKey, messageWithIntent } = require('@mysten/sui/cryptography');
const { verifyTransactionSignature } = require('@mysten/sui/verify');
const { blake2b } = require('@noble/hashes/blake2b');

const hex = (value) => Buffer.from(value).toString('hex');
const PHRASES = [
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
  'legal winner thank year wave sausage worth useful legal winner thank yellow',
  'letter advice cage absurd amount doctor acoustic avoid letter advice cage above',
];
const MEMBERS = [
  ['ed25519', Ed25519Keypair, "m/44'/784'/0'/0'/0'", PHRASES[0], 1],
  ['secp256k1', Secp256k1Keypair, "m/54'/784'/0'/0/0", PHRASES[1], 1],
  ['secp256r1', Secp256r1Keypair, "m/74'/784'/0'/0/0", PHRASES[2], 2],
];
const THRESHOLD = 2;
const AMOUNT = 1000000;
const RECIPIENT = '0x' + '0'.repeat(61) + 'dad';
const GAS = { objectId: '0x' + '33'.repeat(32), version: '7', digest: '11111111111111111111111111111111' };
const GAS_PRICE = 1000;
const GAS_BUDGET = 10000000;

async function main() {
  const keys = MEMBERS.map(([scheme, Keypair, path, phrase, weight]) => {
    const keypair = Keypair.deriveKeypair(phrase, path);
    const publicKey = keypair.getPublicKey();
    return {
      keypair,
      weight,
      record: {
        scheme, phrase, path, weight, flag: publicKey.flag(),
        public_key: hex(publicKey.toRawBytes()),
        sui_public_key: publicKey.toSuiPublicKey(),
        address: publicKey.toSuiAddress(),
        secret_key: hex(decodeSuiPrivateKey(keypair.getSecretKey()).secretKey),
      },
    };
  });
  const multisig = MultiSigPublicKey.fromPublicKeys({
    threshold: THRESHOLD,
    publicKeys: keys.map(({ keypair, weight }) => ({ publicKey: keypair.getPublicKey(), weight })),
  });
  const sender = multisig.toSuiAddress();
  const unreachable = MultiSigPublicKey.fromPublicKeys({
    threshold: 3,
    publicKeys: keys.map(({ keypair }) => ({ publicKey: keypair.getPublicKey(), weight: 1 })),
  });

  const tx = new Transaction();
  tx.setSender(sender); tx.setGasPrice(GAS_PRICE); tx.setGasBudget(GAS_BUDGET);
  tx.setGasPayment([GAS]);
  const [coin] = tx.splitCoins(tx.gas, [AMOUNT]);
  tx.transferObjects([coin], RECIPIENT);
  const bytes = await tx.build();
  const intentDigest = blake2b(messageWithIntent('TransactionData', bytes), { dkLen: 32 });

  const partials = [];
  for (const { keypair } of keys) partials.push((await keypair.signTransaction(bytes)).signature);
  const combinations = [];
  for (const [name, members] of [['k0_k1', [0, 1]], ['k2', [2]], ['k0_k1_k2', [0, 1, 2]]]) {
    const signature = multisig.combinePartialSignatures(members.map((index) => partials[index]));
    const verified = await multisig.verifyTransaction(bytes, signature)
      && (await verifyTransactionSignature(bytes, signature, { address: sender })).toSuiAddress() === sender;
    if (!verified) throw new Error(`combined signature ${name} does not verify`);
    combinations.push({
      name, members,
      weight: members.reduce((sum, index) => sum + keys[index].weight, 0),
      bitmap: members.reduce((bits, index) => bits | (1 << index), 0),
      signature, signature_hex: hex(Buffer.from(signature, 'base64')), verified,
    });
  }

  process.stdout.write(JSON.stringify({
    provenance: '@mysten/sui 1.45.2',
    keys: keys.map(({ record }) => record),
    multisig: {
      threshold: THRESHOLD,
      weights: keys.map(({ weight }) => weight),
      address: sender,
      public_key: hex(multisig.toRawBytes()),
      public_key_base64: multisig.toBase64(),
      sui_public_key: multisig.toSuiPublicKey(),
    },
    unreachable_policy: { threshold: 3, weights: [1, 1, 1], address: unreachable.toSuiAddress() },
    transaction: {
      sender, recipient: RECIPIENT, amount: String(AMOUNT), gas_payment: [GAS],
      gas_price: String(GAS_PRICE), gas_budget: String(GAS_BUDGET),
      raw: hex(bytes), raw_base64: Buffer.from(bytes).toString('base64'),
      digest: await tx.getDigest(), intent_message_digest: hex(intentDigest),
    },
    partial_signatures: keys.map(({ record }, index) => ({ scheme: record.scheme, signature: partials[index] })),
    combined_signatures: combinations,
  }, null, 2) + '\n');
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
