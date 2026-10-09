// Independent XRP Ledger multi-signing vectors, for
// core/tests/fixtures/xrp-multisig.json.
// Install the pinned libraries in a temporary directory:
// npm install --prefix /tmp/spectra-xrp-multisig --ignore-scripts xrpl@4.7.0 ripple-binary-codec@2.11.0 ripple-keypairs@2.1.0 @scure/bip32@1.7.0 @scure/bip39@1.6.0
// NODE_PATH=/tmp/spectra-xrp-multisig/node_modules node scripts/generate-xrp-multisig-vectors.cjs > core/tests/fixtures/xrp-multisig.json
//
// Three BIP-39 test phrases (empty passphrase) are the parties, each a
// secp256k1 key at BIP-44's m/44'/144'/0'/0/0 from the "Bitcoin seed" master,
// as scripts/generate-xrp-payment-vector.cjs derives them. The first phrase's
// account is multi-signed: its SignerList holds the other two (weight 1 each)
// and a third address (weight 2), quorum 2. For each transaction,
// ripple-binary-codec gives every signer's multisigning data, ripple-keypairs
// signs it (RFC 6979, low-S, DER), and the signer's single-Signer blob is
// what that signer hands over. xrpl.js combines the blobs with multisign(),
// which sorts Signers by account ID, and hashes the result with
// hashes.hashSignedTx. Each blob is checked against xrpl's own
// Wallet.sign(tx, true), and the combination against the reversed order.
// The JSON-RPC responses are samples shaped as xrpld documents them; their
// ledger hashes and PreviousTxnIDs are placeholders.
const crypto = require('node:crypto');
const codec = require('ripple-binary-codec');
const keypairs = require('ripple-keypairs');
const xrpl = require('xrpl');
const { HDKey } = require('@scure/bip32');
const { mnemonicToSeedSync } = require('@scure/bip39');

const PATH = "m/44'/144'/0'/0/0";
const PHRASES = {
  p0: 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
  p1: 'legal winner thank year wave sausage worth useful legal winner thank yellow',
  p2: 'letter advice cage absurd amount doctor acoustic avoid letter advice cage above',
};
const THIRD_SIGNER = 'rN7n7otQDd6FczFgLdSqtcsAUxDkw6fzRH';
const DESTINATION = 'rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe';
const LSF_DISABLE_MASTER = 0x00100000;
const LSF_ONE_OWNER_COUNT = 0x00010000;

const hex = bytes => Buffer.from(bytes).toString('hex').toUpperCase();
const accountId = address => hex(xrpl.decodeAccountID(address));
const placeholder = label => crypto.createHash('sha512').update(label).digest('hex').slice(0, 64).toUpperCase();
const assert = (condition, message) => {
  if (!condition) throw new Error(message);
};
// xrpld writes JSON objects from a std::map, so keys come out in byte order.
const sortKeys = value => {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (value === null || typeof value !== 'object') return value;
  return Object.fromEntries(Object.keys(value).sort().map(key => [key, sortKeys(value[key])]));
};

const keys = Object.fromEntries(Object.entries(PHRASES).map(([name, phrase]) => {
  const node = HDKey.fromMasterSeed(mnemonicToSeedSync(phrase)).derive(PATH);
  const publicKey = hex(node.publicKey);
  const address = keypairs.deriveAddress(publicKey);
  assert(xrpl.Wallet.fromMnemonic(phrase).classicAddress === address, `${name}: xrpl.Wallet disagrees`);
  return [name, {
    phrase,
    address,
    account_id: accountId(address),
    public_key: publicKey,
    private_key: hex(node.privateKey),
  }];
}));
const signingKey = name => '00' + keys[name].private_key;

const signerList = {
  quorum: 2,
  entries: [
    { account: keys.p1.address, weight: 1 },
    { account: keys.p2.address, weight: 1 },
    { account: THIRD_SIGNER, weight: 2 },
  ].map(entry => ({ ...entry, account_id: accountId(entry.account) })),
};

function multisigned(name, baseFee, extra) {
  const signerCount = 2;
  const tx = {
    TransactionType: 'Payment',
    Flags: 0,
    Account: keys.p0.address,
    Destination: DESTINATION,
    ...extra,
    Amount: '1000000',
    Sequence: 9,
    Fee: String(baseFee * (1 + signerCount)),
    LastLedgerSequence: 95_000_000,
    SigningPubKey: '',
  };
  const signatures = ['p1', 'p2'].map(signer => {
    const { address, public_key: publicKey } = keys[signer];
    const multisigningHex = codec.encodeForMultisigning(tx, address);
    const signature = keypairs.sign(multisigningHex, signingKey(signer));
    assert(keypairs.verify(multisigningHex, signature, publicKey), `${name}/${signer}: signature does not verify`);
    const blob = codec.encode({
      ...tx,
      Signers: [{ Signer: { Account: address, SigningPubKey: publicKey, TxnSignature: signature } }],
    });
    const wallet = new xrpl.Wallet(publicKey, signingKey(signer));
    assert(wallet.sign(tx, true).tx_blob === blob, `${name}/${signer}: xrpl Wallet.sign disagrees`);
    return {
      signer,
      account: address,
      signing_pub_key: publicKey,
      multisigning_hex: multisigningHex,
      txn_signature: signature,
      single_signer_blob: blob,
    };
  });
  const blob = xrpl.multisign(signatures.map(s => s.single_signer_blob));
  assert(xrpl.multisign(signatures.map(s => s.single_signer_blob).reverse()) === blob,
    `${name}: multisign depends on input order`);
  const decoded = codec.decode(blob);
  return {
    name,
    base_fee_drops: baseFee,
    signer_count: signerCount,
    tx,
    signatures,
    multisigned: {
      blob,
      tx_json: decoded,
      hash: xrpl.hashes.hashSignedTx(blob),
      signers_order: decoded.Signers.map(({ Signer }) => ({ account: Signer.Account, account_id: accountId(Signer.Account) })),
    },
  };
}

const transactions = [
  multisigned('payment', 10, {}),
  multisigned('payment_destination_tag', 15, { DestinationTag: 4242 }),
];

// Ledger entries as xrpld stores them, round-tripped through the codec so
// every field name and type is a real one. SignerListSet stores the entries
// sorted by account ID.
const ledgerIndex = 94_999_990;
const ledgerHash = placeholder('spectra xrp multisig ledger');
const roundTrip = entry => ({ ...codec.decode(codec.encode(entry)) });
const signerListEntry = sortKeys({
  ...roundTrip({
    LedgerEntryType: 'SignerList',
    Flags: LSF_ONE_OWNER_COUNT,
    OwnerNode: '0',
    PreviousTxnID: placeholder('spectra xrp multisig SignerListSet'),
    PreviousTxnLgrSeq: 94_999_000,
    SignerEntries: [...signerList.entries]
      .sort((a, b) => a.account_id.localeCompare(b.account_id))
      .map(entry => ({ SignerEntry: { Account: entry.account, SignerWeight: entry.weight } })),
    SignerListID: 0,
    SignerQuorum: signerList.quorum,
  }),
  index: xrpl.hashes.hashSignerListId(keys.p0.address),
});
const accountRoot = sortKeys({
  ...roundTrip({
    LedgerEntryType: 'AccountRoot',
    Account: keys.p0.address,
    Balance: '25000000',
    Flags: LSF_DISABLE_MASTER,
    OwnerCount: 1,
    PreviousTxnID: placeholder('spectra xrp multisig AccountSet'),
    PreviousTxnLgrSeq: 94_999_100,
    Sequence: 9,
  }),
  index: xrpl.hashes.hashAccountRoot(keys.p0.address),
});
const accountFlags = {
  defaultRipple: false,
  depositAuth: false,
  disableMasterKey: true,
  disallowIncomingCheck: false,
  disallowIncomingNFTokenOffer: false,
  disallowIncomingPayChan: false,
  disallowIncomingTrustline: false,
  disallowIncomingXRP: false,
  globalFreeze: false,
  noFreeze: false,
  passwordSpent: false,
  requireAuthorization: false,
  requireDestinationTag: false,
};
const accountInfoParams = { account: keys.p0.address, ledger_index: 'validated', signer_lists: true };
const ledger = { ledger_hash: ledgerHash, ledger_index: ledgerIndex, status: 'success', validated: true };
const submitted = transactions[0].multisigned;

const fixture = {
  provenance: 'xrpl 4.7.0, ripple-binary-codec 2.11.0, ripple-keypairs 2.1.0, @scure/bip32 1.7.0, @scure/bip39 1.6.0',
  notes: [
    'Keys: BIP-39 phrase, empty passphrase, secp256k1 BIP-32 from the "Bitcoin seed" master at derivation_path; private_key is the 32-byte scalar (ripple-keypairs takes it with a 00 prefix).',
    'A multi-signed transaction carries SigningPubKey "" and costs base_fee_drops * (1 + signer_count) drops: https://xrpl.org/docs/concepts/transactions/transaction-cost',
    'multisigning_hex is encodeForMultisigning(tx, signer): prefix 534D5400, the tx without signing fields, then the signer account ID. txn_signature is its secp256k1 DER signature.',
    'multisign() sorts Signers by account ID ascending, as xrpld requires; signers_order records the result.',
    'account_info: https://xrpl.org/docs/references/http-websocket-apis/public-api-methods/account-methods/account_info -- with signer_lists: true, API v1 nests signer_lists under account_data, API v2 (and Clio always) returns it at the result root. A present array has exactly one member.',
    'account_objects: https://xrpl.org/docs/references/http-websocket-apis/public-api-methods/account-methods/account_objects -- type "signer_list" is the short name of SignerList (https://xrpl.org/docs/references/http-websocket-apis/api-conventions/ledger-entry-short-names).',
    'SignerList entry: https://xrpl.org/docs/references/protocol/ledger-data/ledger-entry-types/signerlist -- lsfOneOwnerCount (0x00010000) is set on lists created since MultiSignReserve, so the owner reserve counts it once (OwnerCount 1).',
    'AccountRoot Flags 0x00100000 is lsfDisableMaster: https://xrpl.org/docs/references/protocol/ledger-data/ledger-entry-types/accountroot',
    'submit_multisigned: https://xrpl.org/docs/references/http-websocket-apis/public-api-methods/transaction-methods/submit_multisigned',
    'Ledger hashes and PreviousTxnIDs in rpc are placeholders (SHA-512 of a label), not real ledger data. Object keys are in byte order, as xrpld writes them.',
  ],
  derivation_path: PATH,
  keys,
  multisig_account: keys.p0.address,
  signer_list: signerList,
  transactions,
  rpc: {
    account_info_v1: {
      request: { method: 'account_info', params: [accountInfoParams] },
      response: { result: sortKeys({ account_data: { ...accountRoot, signer_lists: [signerListEntry] }, account_flags: accountFlags, ...ledger }) },
    },
    account_info_v2: {
      request: { method: 'account_info', params: [{ ...accountInfoParams, api_version: 2 }] },
      response: { result: sortKeys({ account_data: accountRoot, account_flags: accountFlags, signer_lists: [signerListEntry], ...ledger }) },
    },
    account_objects_signer_list: {
      request: { method: 'account_objects', params: [{ account: keys.p0.address, ledger_index: 'validated', type: 'signer_list' }] },
      response: { result: sortKeys({ account: keys.p0.address, account_objects: [signerListEntry], ...ledger }) },
    },
    submit_multisigned: {
      request: { method: 'submit_multisigned', params: [{ tx_json: submitted.tx_json }] },
      response: {
        result: sortKeys({
          engine_result: 'tesSUCCESS',
          engine_result_code: 0,
          engine_result_message: 'The transaction was applied. Only final in a validated ledger.',
          status: 'success',
          tx_blob: submitted.blob,
          tx_json: { ...submitted.tx_json, hash: submitted.hash },
        }),
      },
    },
  },
};
process.stdout.write(JSON.stringify(fixture, null, 2) + '\n');
