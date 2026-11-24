// Independent Hedera references from @hashgraph/sdk: the keys a phrase holds
// on each derivation profile, both readings of a bare 32-byte key, HIP-15
// account-id checksums on mainnet, and HBAR transfers frozen for one or more
// nodes and signed, with each node's body, signed transaction, Transaction
// bytes and SHA-384 transaction hash.
// node tests/vectors/generate-hedera-vectors.cjs
const fs = require('node:fs');
const {
  Mnemonic, PrivateKey, AccountId, TransferTransaction, TransactionId, Timestamp, Hbar, Client,
} = require('@hashgraph/sdk');

const MNEMONIC = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';

// The SDK's three phrase readings, at each profile's template.
const PROFILES = [
  ['standard', (i) => `m/44'/3030'/0'/0'/${i}'`, (m, pass, i) => m.toStandardEd25519PrivateKey(pass, i)],
  ['ecdsa', (i) => `m/44'/3030'/0'/0/${i}`, (m, pass, i) => m.toStandardECDSAsecp256k1PrivateKey(pass, i)],
  ['evm', (i) => `m/44'/60'/0'/0/${i}`,
    (m, pass, i) => m.toStandardECDSAsecp256k1PrivateKeyCustomDerivationPath(pass, `m/44'/60'/0'/0/${i}`)],
];

const hex = (bytes) => Buffer.from(bytes).toString('hex');

// Protobuf's length-delimited fields, enough to take a TransactionList and
// a SignedTransaction apart without the SDK's own decoder.
function fields(bytes) {
  const out = [];
  let at = 0;
  const varint = () => {
    let value = 0n;
    let shift = 0n;
    for (;;) {
      const byte = bytes[at++];
      value |= BigInt(byte & 0x7f) << shift;
      if (!(byte & 0x80)) return value;
      shift += 7n;
    }
  };
  while (at < bytes.length) {
    const key = Number(varint());
    const [field, wire] = [key >> 3, key & 7];
    if (wire === 2) {
      const length = Number(varint());
      out.push([field, bytes.subarray(at, at + length)]);
      at += length;
    } else if (wire === 0) {
      out.push([field, varint()]);
    } else {
      throw new Error(`wire type ${wire}`);
    }
  }
  return out;
}

async function transfer(terms) {
  const key = PrivateKey.fromStringDer(terms.private_key);
  const payer = AccountId.fromString(terms.payer);
  const tx = new TransferTransaction()
    .addHbarTransfer(payer, Hbar.fromTinybars(-BigInt(terms.amount)))
    .addHbarTransfer(AccountId.fromString(terms.recipient), Hbar.fromTinybars(BigInt(terms.amount)))
    .setTransactionId(TransactionId.withValidStart(payer, new Timestamp(terms.valid_start_seconds, terms.valid_start_nanos)))
    .setNodeAccountIds(terms.nodes.map((node) => AccountId.fromString(node)))
    .setTransactionValidDuration(terms.valid_duration)
    .setMaxTransactionFee(Hbar.fromTinybars(terms.max_fee))
    .setTransactionMemo(terms.memo)
    .freeze();
  const signed = await tx.sign(key);
  const hashes = await signed.getTransactionHashPerNode();
  const list = fields(signed.toBytes()).map(([, transaction]) => transaction);
  const bodies = list.map((transaction) => {
    const [[, signedBytes]] = fields(transaction).filter(([field]) => field === 5);
    const parts = fields(signedBytes);
    const body = parts.find(([field]) => field === 1)[1];
    const sigMap = parts.find(([field]) => field === 2)[1];
    const [[, pair]] = fields(sigMap);
    const [, signature] = fields(pair).find(([field]) => field === 3 || field === 6);
    return {
      body_bytes: hex(body),
      signature: hex(signature),
      signed_transaction: hex(signedBytes),
      transaction: hex(transaction),
    };
  });
  return {
    ...terms,
    public_key: key.publicKey.toStringDer(),
    transaction_id: signed.transactionId.toString(),
    per_node: terms.nodes.map((node, index) => ({
      node,
      ...bodies[index],
      transaction_hash: hex(hashes.get(AccountId.fromString(node))),
    })),
  };
}

(async () => {
  const mnemonic = await Mnemonic.fromString(MNEMONIC);
  const keys = [];
  for (const [profile, path, derive] of PROFILES) {
    for (const index of [0, 1]) {
      const key = await derive(mnemonic, '', index);
      keys.push({ profile, index, path: path(index), passphrase: '', private_key: key.toStringDer(), public_key: key.publicKey.toStringDer() });
    }
    const key = await derive(mnemonic, 'TREZOR', 0);
    keys.push({ profile, index: 0, path: path(0), passphrase: 'TREZOR', private_key: key.toStringDer(), public_key: key.publicKey.toStringDer() });
  }
  const RAW = '4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318';
  const raw_key = {
    hex: RAW,
    ed25519: { private_key: PrivateKey.fromStringED25519(RAW).toStringDer(), public_key: PrivateKey.fromStringED25519(RAW).publicKey.toStringDer() },
    ecdsa: { private_key: PrivateKey.fromStringECDSA(RAW).toStringDer(), public_key: PrivateKey.fromStringECDSA(RAW).publicKey.toStringDer() },
  };
  const client = Client.forMainnet();
  const checksums = ['0.0.3', '0.0.98', '0.0.123', '0.0.1234', '0.0.98765', '0.0.1000000', '0.0.4294967295']
    .map((account) => ({ account, with_checksum: AccountId.fromString(account).toStringWithChecksum(client) }));
  client.close();
  const ed25519 = keys.find((key) => key.profile === 'standard' && key.index === 0 && !key.passphrase).private_key;
  const ecdsa = keys.find((key) => key.profile === 'ecdsa' && key.index === 0 && !key.passphrase).private_key;
  const transfers = [
    await transfer({
      private_key: ed25519, payer: '0.0.1234', recipient: '0.0.98765', amount: '150000000', memo: '104857600',
      max_fee: '2000000', valid_start_seconds: 1795522566, valid_start_nanos: 435000006, valid_duration: 120,
      nodes: ['0.0.3', '0.0.4', '0.0.7'],
    }),
    // A payer above its recipient, no memo, a whole-second start and one node.
    await transfer({
      private_key: ecdsa, payer: '0.0.98765', recipient: '0.0.1234', amount: '1', memo: '',
      max_fee: '1949000', valid_start_seconds: 1795522566, valid_start_nanos: 0, valid_duration: 120,
      nodes: ['0.0.35'],
    }),
    // A 100-byte memo of multi-byte UTF-8.
    await transfer({
      private_key: ed25519, payer: '0.0.1234', recipient: '0.0.9000001', amount: '4999999999999999', memo: 'é'.repeat(50),
      max_fee: '100000000', valid_start_seconds: 1700000000, valid_start_nanos: 1, valid_duration: 120,
      nodes: ['0.0.4', '0.0.3'],
    }),
  ];
  const fixture = {
    provenance: '@hashgraph/sdk 2.81.0',
    mnemonic: MNEMONIC,
    keys,
    raw_key,
    checksums,
    transfers,
  };
  fs.writeFileSync('core/tests/fixtures/hedera-vectors.json', JSON.stringify(fixture, null, 2) + '\n');
})();
