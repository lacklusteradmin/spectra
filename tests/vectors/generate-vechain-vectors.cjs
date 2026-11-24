// Independent VeChainThor references from @vechain/sdk-core: phrase accounts
// along VeWorld's and Sync2's path, dynamic-fee (VIP-251, type 0x51)
// transfers of VET, VTHO and another VIP-180 token built, hashed, signed and
// identified, and two mainnet transactions as a node served them
// (`GET /transactions/{id}?raw=true`, 2026-11-24), decoded by the SDK.
// node tests/vectors/generate-vechain-vectors.cjs
const fs = require('node:fs');
const {
  ABIContract, Address, Clause, HDKey, Hex, Transaction, VET, VIP180_ABI, VTHO,
} = require('@vechain/sdk-core');

const MNEMONIC = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const WORDS = MNEMONIC.split(' ');
// The mainnet genesis id's last byte.
const CHAIN_TAG = 0x4a;

const hex = (bytes) => Buffer.from(bytes).toString('hex');

function account(index) {
  const path = `m/44'/818'/0'/0/${index}`;
  const key = HDKey.fromMnemonic(WORDS, path);
  return {
    index,
    path,
    private_key: hex(key.privateKey),
    public_key: hex(key.publicKey),
    address: Address.ofPublicKey(key.publicKey).toString(),
  };
}

const quantity = (value) => '0x' + BigInt(value).toString(16);

// What a clause transfers: VET to its address, or a VIP-180 token's
// `transfer(recipient, amount)`.
function transferOf(clause) {
  if (clause.data === '0x') {
    return { token: null, recipient: clause.to.toLowerCase(), amount: BigInt(clause.value).toString() };
  }
  if (!clause.data.startsWith('0xa9059cbb') || clause.data.length !== 2 + 8 + 128 || BigInt(clause.value) !== 0n) {
    throw new Error('not a transfer');
  }
  return {
    token: clause.to.toLowerCase(),
    recipient: '0x' + clause.data.slice(34, 74),
    amount: BigInt('0x' + clause.data.slice(74)).toString(),
  };
}

function transfer(name, from, clause, terms) {
  const body = {
    chainTag: CHAIN_TAG,
    blockRef: terms.block_ref,
    expiration: terms.expiration,
    clauses: [clause],
    maxPriorityFeePerGas: quantity(terms.max_priority_fee_per_gas),
    maxFeePerGas: quantity(terms.max_fee_per_gas),
    gas: terms.gas,
    dependsOn: null,
    nonce: quantity(terms.nonce),
  };
  const unsigned = Transaction.of(body);
  const signed = unsigned.sign(Buffer.from(from.private_key, 'hex'));
  if (signed.origin.toString() !== from.address) throw new Error(`${name}: origin`);
  return {
    name,
    private_key: from.private_key,
    sender: from.address,
    ...transferOf(clause),
    to: clause.to,
    value: BigInt(clause.value).toString(),
    data: clause.data,
    ...terms,
    intrinsic_gas: Transaction.intrinsicGas([clause]).wei.toString(),
    unsigned: hex(unsigned.encoded),
    signing_hash: hex(unsigned.getTransactionHash().bytes),
    signature: hex(signed.signature),
    raw: hex(signed.encoded),
    id: signed.id.toString(),
  };
}

// Captured from mainnet.vechain.org: a VET transfer and a VTHO transfer.
const MAINNET = [
  {
    id: '0x20939143610e38593cc6a1c251c9211fdac13d84a019e007df4b5f9f66782d65',
    origin: '0x5f7a231df8b27159dd5e4599bafb102f8b3ad136',
    raw: '0x51f88c4a88019457ba019860dd20e2e1949a107a75cff525b033a3e53cadafe3d193b570ec8a32b0541d0236ad85c5a680856b1a22f800860a9ace287800827b0c8088c48e6affd1ae82dec0b841ca5986c54398084864fe62a019f9f4594218f56f13ee9881392f714373755a123978b22ca9cc7593a22b07c67943ba73ca145f21780ef6dcf6429f76c703d4e601',
  },
  {
    id: '0x5078aa3705cc4c20da1ad84a0dcaad9324619d9c1400583edc587b387c3a0b0f',
    origin: '0x3f6400f0edd707827b9220e75dc2f97d175e1ea2',
    raw: '0x51f8c54a88019457a3efd4ba3264f85ef85c940000000000000000000000000000456e6572677980b844a9059cbb0000000000000000000000009b2e496403dd86e5380ad65a6699fa55ddabfce20000000000000000000000000000000000000000000003eae3130ecc9690000085580ebc41f2860acd9c2679f282c9be80849ca5d08ec0b841f68ad4c578b2a76aa59260015f06bc3767537f2b8f0f439497d8c1c80ed7edde504840342cef4a46af3baaa22fa927353b0697b7b72c7b1c946648ecd2d9f4a000',
  },
];

function mainnet(captured) {
  const tx = Transaction.decode(Hex.of(captured.raw).bytes, true);
  if (tx.id.toString() !== captured.id) throw new Error(`${captured.id}: id`);
  if (tx.origin.toString().toLowerCase() !== captured.origin) throw new Error(`${captured.id}: origin`);
  const { body } = tx;
  const [clause] = body.clauses;
  return {
    id: captured.id,
    origin: captured.origin,
    ...transferOf(clause),
    raw: captured.raw.slice(2),
    signing_hash: hex(tx.getTransactionHash().bytes),
    chain_tag: body.chainTag,
    block_ref: body.blockRef,
    expiration: body.expiration,
    to: clause.to,
    value: BigInt(clause.value).toString(),
    data: clause.data,
    max_priority_fee_per_gas: BigInt(body.maxPriorityFeePerGas).toString(),
    max_fee_per_gas: BigInt(body.maxFeePerGas).toString(),
    gas: Number(body.gas),
    nonce: BigInt(body.nonce).toString(),
    signature: hex(tx.signature),
  };
}

const accounts = [0, 1, 2].map(account);
const [first, second, third] = accounts;
// B3TR, VeBetter's VIP-180 token on mainnet.
const B3TR = '0x5ef79995fe8a89e0812330e4378eb2660cede699';
const transfers = [
  transfer('vet', first, Clause.transferVET(Address.of(second.address), VET.of('1234.5')), {
    block_ref: '0x019457ba019860dd', expiration: 60, max_priority_fee_per_gas: '500000000000',
    max_fee_per_gas: '20500000000000', gas: 21000, nonce: '14134328910213411550',
  }),
  transfer('vtho', first, Clause.transferVTHOToken(Address.of(second.address), VTHO.of('25.5')).clause, {
    block_ref: '0x00000a0b0c0d0e0f', expiration: 32, max_priority_fee_per_gas: '1',
    max_fee_per_gas: '10000000000001', gas: 51644, nonce: '7',
  }),
  transfer('vip180', third, Clause.callFunction(
    Address.of(B3TR), ABIContract.ofAbi(VIP180_ABI).getFunction('transfer'), [first.address, 1n],
  ), {
    block_ref: '0x0194579646030c95', expiration: 720, max_priority_fee_per_gas: '0',
    max_fee_per_gas: '340282366920938463463374607431768211455', gas: 16777216, nonce: '0',
  }),
];

const fixture = {
  provenance: '@vechain/sdk-core 2.1.0; mainnet transactions from mainnet.vechain.org 2026-11-24',
  mnemonic: MNEMONIC,
  chain_tag: CHAIN_TAG,
  accounts,
  transfers,
  mainnet: MAINNET.map(mainnet),
};
fs.writeFileSync('core/tests/fixtures/vechain-vectors.json', JSON.stringify(fixture, null, 2) + '\n');
