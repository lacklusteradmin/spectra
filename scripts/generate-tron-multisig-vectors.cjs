// Independent TRON account-permission multisig vectors, for core/tests/fixtures/tron-multisig.json.
// Install the pinned library in a temporary directory:
// npm install --prefix /tmp/spectra-tron-multisig --ignore-scripts tronweb@6.0.4
// NODE_PATH=/tmp/spectra-tron-multisig/node_modules node scripts/generate-tron-multisig-vectors.cjs > core/tests/fixtures/tron-multisig.json
//
// Three BIP-39 test phrases, each at m/44'/195'/0'/0/0 (TronWeb.fromMnemonic),
// share the first phrase's account: its owner permission is 2-of-3 over all
// three keys and its active permission (id 2) is 2-of-2 over the other two,
// allowed TransferContract, TransferAssetContract and TriggerSmartContract.
// TronWeb's own protobuf (utils.transaction.txJsonToPb) serializes a TRX
// payment under the active and the owner permission and a TRC-20 transfer
// under the active one; each key signs every transaction with
// utils.crypto.signTransaction (secp256k1 over sha256(raw_data), RFC 6979,
// v 27/28). Nothing touches a node. A hand-written protobuf reader checks
// where Permission_id lands, and ethers' SigningKey re-signs every digest.
// The operations bitmap follows java-tron's
// chainbase/src/main/java/org/tron/common/utils/WalletUtil.java
// checkPermissionOperations: contract type n is bit (n % 8) of byte n / 8.
// The account is shaped as java-tron's framework JsonFormat prints a
// visible:true POST /wallet/getaccount (Util.printAccount): fields in field
// number order, proto3 defaults omitted (so the owner permission has no type
// or id), addresses in base58, operations in hex, int64 as JSON numbers.
const crypto = require('node:crypto');
const { TronWeb, utils } = require('tronweb');

const PHRASES = [
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
  'legal winner thank year wave sausage worth useful legal winner thank yellow',
  'letter advice cage absurd amount doctor acoustic avoid letter advice cage above',
];
const PATH = "m/44'/195'/0'/0/0";
const RECIPIENT = 'TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf';
const USDT = 'TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t';
const BLOCK = { number: 66000000, id: '0000000003ef1480' + '7c'.repeat(24), timestamp_ms: 1760000000000 };
const ACTIVE_ID = 2;
const ACTIVE_CONTRACT_TYPES = { TransferContract: 1, TransferAssetContract: 2, TriggerSmartContract: 31 };

function check(condition, message) {
  if (!condition) throw new Error(message);
}

const keys = PHRASES.map((phrase) => {
  const account = TronWeb.fromMnemonic(phrase, PATH);
  check(account.address === TronWeb.address.fromPrivateKey(account.privateKey.slice(2)), 'address');
  return { address: account.address, hex: TronWeb.address.toHex(account.address).toLowerCase(),
    private_key: account.privateKey.replace(/^0x/, '') };
});
const owner = keys[0];

function operations(types) {
  const bytes = Buffer.alloc(32);
  for (const type of types) bytes[type >> 3] |= 1 << (type & 7);
  return bytes.toString('hex');
}
// TRON's own worked example (developers.tron.network/docs/multi-signature):
// TransferContract, VoteWitnessContract and FreezeBalanceV2Contract.
check(operations([1, 4, 54]) === '1200000000004000000000000000000000000000000000000000000000000000', 'operations');

// The block's last two height bytes and the id's bytes 8..16.
const height = Buffer.alloc(8);
height.writeBigUInt64BE(BigInt(BLOCK.number));
check(BLOCK.id.startsWith(height.toString('hex')), 'block id');
const reference = { ref_block_bytes: height.subarray(6).toString('hex'), ref_block_hash: BLOCK.id.slice(16, 32) };

// A protobuf message's top-level fields, enough to read raw_data.
function fields(hex) {
  const bytes = Buffer.from(hex, 'hex');
  let at = 0;
  const varint = () => {
    let value = 0n;
    for (let shift = 0n; ; shift += 7n) {
      const byte = bytes[at++];
      value |= BigInt(byte & 0x7f) << shift;
      if (byte < 0x80) return value;
    }
  };
  const out = [];
  while (at < bytes.length) {
    const tag = Number(varint());
    const [number, wire] = [tag >> 3, tag & 7];
    if (wire === 0) out.push({ number, value: varint() });
    else if (wire === 2) {
      const length = Number(varint());
      out.push({ number, hex: bytes.subarray(at, at + length).toString('hex') });
      at += length;
    } else throw new Error(`wire type ${wire}`);
  }
  return out;
}

function word(hex) {
  return hex.padStart(64, '0');
}
const trc20Data = 'a9059cbb' + word(TronWeb.address.toHex(RECIPIENT).slice(2)) + word((1234567).toString(16));

const CASES = [
  { name: 'trx_active', permission_id: ACTIVE_ID, expiration: BLOCK.timestamp_ms + 3600000,
    contract: { type: 'TransferContract', value: {
      owner_address: owner.hex, to_address: TronWeb.address.toHex(RECIPIENT).toLowerCase(), amount: 1000000 } } },
  { name: 'trx_owner', permission_id: 0, expiration: BLOCK.timestamp_ms + 86400000,
    contract: { type: 'TransferContract', value: {
      owner_address: owner.hex, to_address: TronWeb.address.toHex(RECIPIENT).toLowerCase(), amount: 1000000 } } },
  { name: 'trc20_active', permission_id: ACTIVE_ID, expiration: BLOCK.timestamp_ms + 3600000, fee_limit: 100000000,
    contract: { type: 'TriggerSmartContract', value: {
      owner_address: owner.hex, contract_address: TronWeb.address.toHex(USDT).toLowerCase(), data: trc20Data } } },
];

const transactions = CASES.map((c) => {
  const contract = {
    type: c.contract.type,
    parameter: { value: c.contract.value, type_url: `type.googleapis.com/protocol.${c.contract.type}` },
  };
  // java-tron prints a zero Permission_id by leaving it out.
  if (c.permission_id) contract.Permission_id = c.permission_id;
  const rawData = { ...reference, expiration: c.expiration, contract: [contract], timestamp: BLOCK.timestamp_ms };
  if (c.fee_limit) rawData.fee_limit = c.fee_limit;
  const transaction = { visible: false, raw_data: rawData };
  const protobuf = utils.transaction.txJsonToPb(transaction);
  const rawHex = utils.transaction.txPbToRawDataHex(protobuf).toLowerCase();
  const txID = utils.transaction.txPbToTxID(protobuf).replace(/^0x/, '').toLowerCase();
  check(txID === crypto.createHash('sha256').update(Buffer.from(rawHex, 'hex')).digest('hex'), 'txID');

  // raw_data: 1 ref_block_bytes, 4 ref_block_hash, 8 expiration, 11 contract,
  // 14 timestamp, 18 fee_limit; Contract: 1 type, 2 parameter, 5 Permission_id.
  const raw = fields(rawHex);
  check(JSON.stringify(raw.map((f) => f.number)) === JSON.stringify(c.fee_limit ? [1, 4, 8, 11, 14, 18] : [1, 4, 8, 11, 14]),
    `${c.name}: raw_data fields`);
  check(raw[0].hex === reference.ref_block_bytes && raw[1].hex === reference.ref_block_hash, 'reference');
  check(raw[2].value === BigInt(c.expiration) && raw[4].value === BigInt(BLOCK.timestamp_ms), 'times');
  const inner = fields(raw[3].hex);
  const permission = inner.find((f) => f.number === 5);
  check(c.permission_id ? permission.value === BigInt(c.permission_id) && raw[3].hex.endsWith('28' + c.permission_id.toString(16).padStart(2, '0'))
    : permission === undefined, `${c.name}: Permission_id`);
  if (!c.permission_id) {
    const explicit = utils.transaction.txJsonToPb({ visible: false, raw_data: { ...rawData,
      contract: [{ ...contract, Permission_id: 0 }] } });
    check(utils.transaction.txPbToRawDataHex(explicit).toLowerCase() === rawHex, 'Permission_id 0 is the default');
  }

  const signatures = keys.map((key) => {
    const signed = utils.crypto.signTransaction(key.private_key, { txID, raw_data: rawData, raw_data_hex: rawHex });
    const signature = signed.signature[0].toLowerCase();
    check(signature.length === 130 && ['1b', '1c'].includes(signature.slice(-2)), 'signature shape');
    check(utils.crypto.ecRecover(txID, signature).toLowerCase() === key.hex, 'recovery');
    const again = new utils.ethersUtils.SigningKey('0x' + key.private_key).sign('0x' + txID).serialized;
    check(again.slice(2) === signature, 'ethers signature');
    return { address: key.address, signature };
  });
  return {
    name: c.name,
    permission_id: c.permission_id,
    raw_data: rawData,
    raw_data_hex: rawHex,
    txID,
    signatures,
  };
});

const activeOperations = operations(Object.values(ACTIVE_CONTRACT_TYPES));
const account = {
  address: owner.address,
  balance: 50000000,
  create_time: 1750000000000,
  owner_permission: {
    permission_name: 'owner',
    threshold: 2,
    keys: keys.map((k) => ({ address: k.address, weight: 1 })),
  },
  active_permission: [{
    type: 'Active',
    id: ACTIVE_ID,
    permission_name: 'active',
    threshold: 2,
    operations: activeOperations,
    keys: keys.slice(1).map((k) => ({ address: k.address, weight: 1 })),
  }],
};

console.log(JSON.stringify({
  provenance: 'tronweb 6.0.4 official protobuf and crypto, generated offline (scripts/generate-tron-multisig-vectors.cjs)',
  sources: [
    'https://github.com/tronprotocol/java-tron/blob/develop/protocol/src/main/protos/core/Tron.proto',
    'https://github.com/tronprotocol/java-tron/blob/develop/chainbase/src/main/java/org/tron/common/utils/WalletUtil.java',
    'https://github.com/tronprotocol/java-tron/blob/develop/chainbase/src/main/java/org/tron/core/capsule/TransactionCapsule.java',
    'https://github.com/tronprotocol/java-tron/blob/develop/framework/src/main/java/org/tron/core/services/http/JsonFormat.java',
    'https://developers.tron.network/docs/multi-signature',
  ],
  phrases: PHRASES,
  path: PATH,
  keys,
  account: owner.address,
  block: BLOCK,
  reference,
  recipient: RECIPIENT,
  trc20_contract: USDT,
  permission_id_encoding: 'Transaction.Contract field 5 (int32 Permission_id) as a varint after field 2 (parameter): tag 0x28, then the id (0x28 0x02 for 2). Proto3 omits a zero, so Permission_id 0 (owner) serializes exactly as a contract without one.',
  operations: {
    contract_types: ACTIVE_CONTRACT_TYPES,
    rule: 'contract type n is bit (n % 8), least significant first, of byte n / 8 of 32 (java-tron WalletUtil.checkPermissionOperations)',
    hex: activeOperations,
  },
  getaccount_note: 'Only the fields these vectors need; balance and create_time are illustrative. A live response carries more (account_resource, frozenV2, ...), also in field-number order.',
  getaccount: account,
  transactions,
}, null, 2));
