// Independent Stellar multisig vectors, for core/tests/fixtures/stellar-multisig.json.
// Install the pinned libraries in a temporary directory:
// npm install --prefix /tmp/spectra-stellar-multisig --ignore-scripts @stellar/stellar-base@14.1.0 bip39@3.1.0 ed25519-hd-key@2.0.0
// NODE_PATH=/tmp/spectra-stellar-multisig/node_modules node scripts/generate-stellar-multisig-vectors.cjs > core/tests/fixtures/stellar-multisig.json
//
// Three BIP-39 test phrases (empty passphrase) are the parties, each an
// ed25519 key at SEP-0005's m/44'/148'/0' by SLIP-10, as
// scripts/generate-derivation-profile-vectors.cjs derives them. The first
// phrase's account is the source; its policy keeps the master key at weight 1,
// adds the other two at weight 1 each, and sets thresholds 1/2/3, so a
// payment (medium) needs two of the three. stellar-base's TransactionBuilder
// builds each payment on the test network with fixed time bounds, and each
// signer's DecoratedSignature is taken from it; ed25519 is deterministic, so
// every signature and envelope is reproducible. The Horizon response is a
// sample shaped as Horizon writes it; its ledger numbers and times are
// placeholders.
const bip39 = require('bip39');
const { derivePath } = require('ed25519-hd-key');
const stellar = require('@stellar/stellar-base');

const PATH = "m/44'/148'/0'";
const PHRASES = {
  p0: 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
  p1: 'legal winner thank year wave sausage worth useful legal winner thank yellow',
  p2: 'letter advice cage absurd amount doctor acoustic avoid letter advice cage above',
};
const DESTINATION = 'GAYOLLLUIZE4DZMBB2ZBKGBUBZLIOYU6XFLW37GBP2VZD3ABNXCW4BVA';
const PASSPHRASE = stellar.Networks.TESTNET;
const ACCOUNT_SEQUENCE = '123456789012';
const BUILDER_FEE = '200';
const TIME_BOUNDS = { minTime: 0, maxTime: 1760003600 };
const HORIZON = 'https://horizon-testnet.stellar.org';

const assert = (condition, message) => {
  if (!condition) throw new Error(message);
};

const keypairs = Object.fromEntries(Object.entries(PHRASES).map(([name, phrase]) => {
  const seed = Buffer.from(derivePath(PATH, bip39.mnemonicToSeedSync(phrase).toString('hex')).key);
  return [name, { phrase, seed, keypair: stellar.Keypair.fromRawEd25519Seed(seed) }];
}));
const keys = Object.fromEntries(Object.entries(keypairs).map(([name, { phrase, seed, keypair }]) => [name, {
  phrase,
  address: keypair.publicKey(),
  public_key: keypair.rawPublicKey().toString('hex'),
  secret_seed: seed.toString('hex'),
}]));
const keypair = name => keypairs[name].keypair;

const policy = {
  master_weight: 1,
  signers: [
    { key: keys.p0.address, weight: 1, role: 'master' },
    { key: keys.p1.address, weight: 1, role: 'signer' },
    { key: keys.p2.address, weight: 1, role: 'signer' },
  ],
  thresholds: { low: 1, medium: 2, high: 3 },
};

function payment(name, memo) {
  const builder = new stellar.TransactionBuilder(new stellar.Account(keys.p0.address, ACCOUNT_SEQUENCE), {
    fee: BUILDER_FEE,
    networkPassphrase: PASSPHRASE,
    timebounds: TIME_BOUNDS,
  }).addOperation(stellar.Operation.payment({ destination: DESTINATION, asset: stellar.Asset.native(), amount: '10' }));
  if (memo) builder.addMemo(memo);
  const tx = builder.build();
  const unsigned = tx.toEnvelope().toXDR('base64');
  const hash = tx.hash();
  const fresh = () => new stellar.Transaction(unsigned, PASSPHRASE);

  const decorated = signer => {
    const signed = fresh();
    signed.sign(keypair(signer));
    const [signature] = signed.signatures;
    assert(keypair(signer).verify(hash, signature.signature()), `${name}/${signer}: signature does not verify`);
    assert(signature.hint().equals(keypair(signer).signatureHint()), `${name}/${signer}: wrong hint`);
    return {
      signer,
      address: keys[signer].address,
      hint: signature.hint().toString('hex'),
      signature: signature.signature().toString('hex'),
      envelope_xdr: signed.toEnvelope().toXDR('base64'),
    };
  };
  const p0 = decorated('p0');
  const p1 = decorated('p1');
  const both = fresh();
  both.sign(keypair('p0'), keypair('p1'));
  const body = both.tx;
  return {
    name,
    memo: memo ? { type: 'text', value: memo.value.toString() } : { type: 'none' },
    source_account_sequence: ACCOUNT_SEQUENCE,
    seq_num: body.seqNum().toString(),
    builder_fee: BUILDER_FEE,
    operation_count: body.operations().length,
    xdr_fee: body.fee(),
    time_bounds: TIME_BOUNDS,
    payment: { destination: DESTINATION, asset: 'native', amount: '10', amount_stroops: '100000000' },
    unsigned_envelope_xdr: unsigned,
    hash: hash.toString('hex'),
    signatures: [p0, p1].map(({ signer, address, hint, signature }) => ({ signer, address, hint, signature })),
    envelope_p0_p1_xdr: both.toEnvelope().toXDR('base64'),
    envelope_p1_only_xdr: p1.envelope_xdr,
  };
}

const transactions = [
  payment('payment', null),
  payment('payment_memo_text', stellar.Memo.text('rent')),
];

// Horizon reads signers from its accounts_signers table ordered by key, the
// master key among them while its weight is above zero.
const account = keys.p0.address;
const links = Object.fromEntries([
  ['self', ''], ['transactions', '/transactions{?cursor,limit,order}'], ['operations', '/operations{?cursor,limit,order}'],
  ['payments', '/payments{?cursor,limit,order}'], ['effects', '/effects{?cursor,limit,order}'],
  ['offers', '/offers{?cursor,limit,order}'], ['trades', '/trades{?cursor,limit,order}'], ['data', '/data/{key}'],
].map(([rel, suffix]) => [rel, suffix ? { href: `${HORIZON}/accounts/${account}${suffix}`, templated: true }
  : { href: `${HORIZON}/accounts/${account}` }]));
const horizonAccount = {
  _links: links,
  id: account,
  account_id: account,
  sequence: ACCOUNT_SEQUENCE,
  sequence_ledger: 1234000,
  sequence_time: '1760000000',
  subentry_count: policy.signers.length - 1,
  last_modified_ledger: 1234000,
  last_modified_time: '2025-10-09T08:53:20Z',
  thresholds: {
    low_threshold: policy.thresholds.low,
    med_threshold: policy.thresholds.medium,
    high_threshold: policy.thresholds.high,
  },
  flags: { auth_required: false, auth_revocable: false, auth_immutable: false, auth_clawback_enabled: false },
  balances: [
    { balance: '100.0000000', buying_liabilities: '0.0000000', selling_liabilities: '0.0000000', asset_type: 'native' },
  ],
  signers: [...policy.signers]
    .sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0))
    .map(({ key, weight }) => ({ weight, key, type: 'ed25519_public_key' })),
  data: {},
  num_sponsoring: 0,
  num_sponsored: 0,
  paging_token: account,
};

const fixture = {
  provenance: '@stellar/stellar-base 14.1.0, bip39 3.1.0, ed25519-hd-key 2.0.0',
  notes: [
    'Keys: BIP-39 phrase, empty passphrase, SLIP-10 ed25519 at derivation_path (SEP-0005); secret_seed is the raw 32-byte ed25519 seed, public_key the raw 32-byte key.',
    'TransactionBuilder charges its fee option per operation: builder_fee 200 with one operation puts 200 in the XDR fee field. Stellar fees do not grow with the number of signatures.',
    'seq_num is the source account sequence plus one. unsigned_envelope_xdr is a TransactionEnvelope (ENVELOPE_TYPE_TX) with no signatures; hash is SHA-256 of the signature base for network_passphrase.',
    'A DecoratedSignature is hint (the last 4 bytes of the signer public key) and the 64-byte ed25519 signature of hash. envelope_p0_p1_xdr carries P0 then P1; envelope_p1_only_xdr carries P1 alone (weight 1, below the medium threshold 2).',
    'Thresholds and signer weights: https://developers.stellar.org/docs/learn/fundamentals/transactions/signatures-multisig',
    'Horizon account: https://developers.stellar.org/docs/data/apis/horizon/api-reference/resources/accounts/object and https://developers.stellar.org/docs/data/apis/horizon/api-reference/retrieve-an-account (example: stellar/stellar-docs openapi/horizon/components/examples/responses/Accounts/RetrieveAnAccount.yml). Field order, the string sequence and omitempty fields follow protocols/horizon Account in stellar/go-stellar-sdk; signers are ordered by key and include the master key while its weight is above zero (stellar-horizon internal/db2/history/account_signers.go, internal/resourceadapter/account_entry.go), and paging_token is the account ID there although the docs example shows "".',
    'Horizon ledger numbers, times and the balance are placeholders, not real ledger data. subentry_count 2 is the two added signers.',
  ],
  derivation_path: PATH,
  network_passphrase: PASSPHRASE,
  keys,
  source_account: keys.p0.address,
  policy,
  transactions,
  horizon: {
    account_request: `GET ${HORIZON}/accounts/${account}`,
    account_response: horizonAccount,
  },
};
process.stdout.write(JSON.stringify(fixture, null, 2) + '\n');
