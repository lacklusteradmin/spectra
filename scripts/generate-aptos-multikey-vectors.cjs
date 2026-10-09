// Aptos MultiKey (AIP-55) account and a 2-of-3 APT transfer from the Aptos
// TypeScript SDK, for core/tests/fixtures/aptos-multikey.json.
// npm install --prefix /tmp/spectra-aptos-multikey --ignore-scripts @aptos-labs/ts-sdk@1.39.0
// NODE_PATH=/tmp/spectra-aptos-multikey/node_modules node scripts/generate-aptos-multikey-vectors.cjs > core/tests/fixtures/aptos-multikey.json
//
// Three BIP-39 test phrases (empty passphrase) are the members: Ed25519 at
// m/44'/637'/0'/0'/0' for the first two, Secp256k1 at m/44'/637'/0'/0/0 for
// the third. The MultiKey is vec<AnyPublicKey> || u8 signatures_required, its
// authentication key sha3-256(bcs || 0x03), and the account address that key.
// The RawTransaction is the APT transfer core's send::aptos::prepare_transfer
// builds: 0x1::coin::transfer<0x1::aptos_coin::AptosCoin>(recipient, amount).
// Every member signs sha3-256("APTOS::RawTransaction") || raw; Secp256k1
// signs sha3-256 of that message, RFC 6979, low-S, so every signature is
// deterministic. MultiKeyAccount signs as each pair of members; the
// transaction authenticator is SingleSender(AccountAuthenticator::MultiKey),
// and generateUserTransactionHash is sha3-256(sha3-256("APTOS::Transaction")
// || 0x00 || SignedTransaction). The REST sample is the GET /accounts/{address}
// body of an account created implicitly by a transfer, whose authentication
// key is still its address.
const apt = require('@aptos-labs/ts-sdk');
const { sha3_256 } = require('@noble/hashes/sha3');

const hex = (value) => Buffer.from(value).toString('hex');
const PHRASES = [
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
  'legal winner thank year wave sausage worth useful legal winner thank yellow',
  'letter advice cage absurd amount doctor acoustic avoid letter advice cage above',
];
const MEMBERS = [
  ['ed25519', apt.Ed25519PrivateKey, "m/44'/637'/0'/0'/0'", PHRASES[0]],
  ['ed25519', apt.Ed25519PrivateKey, "m/44'/637'/0'/0'/0'", PHRASES[1]],
  ['secp256k1', apt.Secp256k1PrivateKey, "m/44'/637'/0'/0/0", PHRASES[2]],
];
const SIGNATURES_REQUIRED = 2;
const COIN = '0x1::aptos_coin::AptosCoin';
const RECIPIENT = '0x' + '0'.repeat(61) + 'dad';
const AMOUNT = 1000000n;
const SEQUENCE = 5n;
const MAX_GAS = 10000n;
const GAS_PRICE = 100n;
const EXPIRATION = 1760000600n;
const CHAIN_ID = 2;

function main() {
  const keys = MEMBERS.map(([scheme, PrivateKey, path, phrase]) => {
    const privateKey = PrivateKey.fromDerivationPath(path, phrase);
    const account = new apt.SingleKeyAccount({ privateKey });
    return {
      privateKey, account,
      record: {
        scheme, phrase, path,
        public_key: hex(privateKey.publicKey().toUint8Array()),
        any_public_key: hex(account.publicKey.bcsToBytes()),
        single_key_address: account.accountAddress.toStringLong(),
        private_key: hex(privateKey.toUint8Array()),
      },
    };
  });
  const multiKey = new apt.MultiKey({
    publicKeys: keys.map(({ privateKey }) => privateKey.publicKey()),
    signaturesRequired: SIGNATURES_REQUIRED,
  });
  const authenticationKey = multiKey.authKey();
  const sender = authenticationKey.derivedAddress();

  const payload = new apt.TransactionPayloadEntryFunction(apt.EntryFunction.build(
    '0x1::coin', 'transfer', [apt.parseTypeTag(COIN)],
    [apt.AccountAddress.fromString(RECIPIENT), new apt.U64(AMOUNT)]));
  const raw = new apt.RawTransaction(sender, SEQUENCE, payload, MAX_GAS, GAS_PRICE, EXPIRATION, new apt.ChainId(CHAIN_ID));
  const transaction = new apt.SimpleTransaction(raw);
  const message = apt.generateSigningMessageForTransaction(transaction);
  if (hex(message) !== hex(apt.generateSigningMessage(raw.bcsToBytes(), 'APTOS::RawTransaction'))) {
    throw new Error('signing message disagrees with RawTransaction BCS');
  }

  const signatures = keys.map(({ privateKey }) => privateKey.sign(message));
  keys.forEach(({ privateKey }, index) => {
    if (!privateKey.publicKey().verifySignature({ message, signature: signatures[index] })) {
      throw new Error(`member ${index} signature does not verify`);
    }
  });

  const signers = [];
  for (const [name, members] of [['k0_k2', [0, 2]], ['k0_k1', [0, 1]]]) {
    const account = new apt.MultiKeyAccount({ multiKey, signers: members.map((index) => keys[index].account) });
    const authenticator = account.signTransactionWithAuthenticator(transaction);
    const signature = authenticator.signatures;
    const manual = new apt.MultiKeySignature({
      signatures: members.map((index) => signatures[index]),
      bitmap: apt.MultiKeySignature.createBitmap({ bits: members }),
    });
    if (hex(signature.bcsToBytes()) !== hex(manual.bcsToBytes())) {
      throw new Error(`MultiKeyAccount and MultiKeySignature disagree for ${name}`);
    }
    const verified = multiKey.verifySignature({ message, signature });
    if (!verified) throw new Error(`MultiKey signature ${name} does not verify`);
    const signed = new apt.SignedTransaction(raw, new apt.TransactionAuthenticatorSingleSender(authenticator)).bcsToBytes();
    if (hex(signed) !== hex(apt.generateSignedTransaction({ transaction, senderAuthenticator: authenticator }))) {
      throw new Error(`generateSignedTransaction disagrees for ${name}`);
    }
    const hash = apt.generateUserTransactionHash({ transaction, senderAuthenticator: authenticator });
    const direct = '0x' + hex(sha3_256(Buffer.concat([sha3_256('APTOS::Transaction'), Buffer.from([0]), signed])));
    if (hash !== direct) throw new Error(`transaction hash disagrees for ${name}`);
    signers.push({
      name, members,
      bitmap: hex(signature.bitmap),
      multi_key_signature: hex(signature.bcsToBytes()),
      account_authenticator: hex(authenticator.bcsToBytes()),
      signed_transaction: hex(signed),
      transaction_hash: hash,
      verified,
    });
  }

  process.stdout.write(JSON.stringify({
    provenance: '@aptos-labs/ts-sdk 1.39.0',
    keys: keys.map(({ record }) => record),
    multi_key: {
      signatures_required: SIGNATURES_REQUIRED,
      bcs: hex(multiKey.bcsToBytes()),
      authentication_key: authenticationKey.toString(),
      address: sender.toStringLong(),
    },
    transaction: {
      sender: sender.toStringLong(),
      sequence_number: SEQUENCE.toString(),
      function: '0x1::coin::transfer',
      type_arguments: [COIN],
      recipient: RECIPIENT,
      amount: AMOUNT.toString(),
      max_gas_amount: MAX_GAS.toString(),
      gas_unit_price: GAS_PRICE.toString(),
      expiration_timestamp_secs: EXPIRATION.toString(),
      chain_id: CHAIN_ID,
      raw: hex(raw.bcsToBytes()),
      signing_message: hex(message),
      signing_message_sha3: hex(sha3_256(message)),
    },
    signatures: keys.map(({ record }, index) => ({ scheme: record.scheme, signature: hex(signatures[index].toUint8Array()) })),
    signers,
    rest_account: {
      path: `/v1/accounts/${sender.toStringLong()}`,
      body: { sequence_number: SEQUENCE.toString(), authentication_key: authenticationKey.toString() },
    },
  }, null, 2) + '\n');
}
try { main(); } catch (error) { console.error(error); process.exitCode = 1; }
