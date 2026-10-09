// Independent TON multisig v2 fixtures for core/tests/fixtures/ton-multisig.json.
// Install the pinned SDKs in a temporary directory:
// npm install --prefix /tmp/spectra-ton-vectors --ignore-scripts @ton/ton@16.3.0 @ton/core@0.63.1 @ton/crypto@3.3.0
// NODE_PATH=/tmp/spectra-ton-vectors/node_modules node scripts/generate-ton-multisig-vectors.cjs > core/tests/fixtures/ton-multisig.json
//
// The contract is ton-blockchain/multisig-contract-v2 at commit
// 9a4b13df6345c9c4068ca725e434b40f9ea5ca28: build/Multisig.compiled.json and
// build/Order.compiled.json hash to the constants below, and
// contracts/auto/order_code.func embeds the order code as a library reference
// (exotic type 2 || order code hash). Cells follow wrappers/Multisig.ts and
// wrappers/Order.ts; the multisig address needs only its code's hash and depth.
//
// Signers are the mainnet W5 wallets of the three plain mnemonics in
// ton-mnemonics.json, the wallets ton-w5.json describes.
//
// toncenter v2 runGetMethod stacks follow the legacy serializer of
// toncenter/ton-http-api-cpp (ton-http-api/src/converters/runmethod.hpp) and of
// the Python service (tvm_valuetypes stack_utils.py): integers are ["num", hex],
// cells AND slices are ["cell", {bytes, object}], and a TVM null arrives as an
// empty list, because tonlib's to_tonlib_api treats null as the end of a list.
const crypto = require('node:crypto');
const fs = require('node:fs');
const ton = require('@ton/core');
const { WalletContractV5R1 } = require('@ton/ton');
const { mnemonicToPrivateKey } = require('@ton/crypto');

const SOURCE_COMMIT = '9a4b13df6345c9c4068ca725e434b40f9ea5ca28';
const MULTISIG_CODE_HASH = 'd3d14da9a627f0ec3533341829762af92b9540b21bf03665fac09c2b46eabbac';
const MULTISIG_CODE_DEPTH = 6;
const ORDER_CODE_HASH = '6305a8061c856c2ccf05dcb0df5815c71475870567cab5f049e340bcf59251f3';
const OP = { new_order: 0xf718510f, approve: 0xa762230f, send_message: 0xf1381e5b };
const MAINNET = -239;

const hex = (buffer) => Buffer.from(buffer).toString('hex');
const cell = (c) => ({ hash: hex(c.hash()), boc_hex: hex(c.toBoc()) });
const addressDict = (addresses) => {
  const dict = ton.Dictionary.empty(ton.Dictionary.Keys.Uint(8), ton.Dictionary.Values.Address());
  addresses.forEach((address, index) => dict.set(index, address));
  return dict;
};
const named = (address, bounceable) => ({ raw: address.toRawString(), address: address.toString({ bounceable, urlSafe: true }) });

// StateInit with only code and data refs: bits 00110, descriptors 0x02 0x01, data byte 0x34.
const u16 = (n) => Buffer.from([n >> 8, n & 0xff]);
const stateInitHash = (code, data) => crypto.createHash('sha256').update(Buffer.concat([
  Buffer.from([0x02, 0x01, 0x34]), u16(code.depth), u16(data.depth()), code.hash, data.hash(),
])).digest();

// Wrapper layout; deployment always starts at next_order_seqno 0.
const multisigData = (nextOrderSeqno, threshold, signers, proposers, allowArbitrary) => ton.beginCell()
  .storeUint(nextOrderSeqno, 256).storeUint(threshold, 8)
  .storeRef(ton.beginCell().storeDictDirect(addressDict(signers)))
  .storeUint(signers.length, 8).storeDict(addressDict(proposers)).storeBit(allowArbitrary).endCell();
const multisigAddress = (data) =>
  new ton.Address(0, stateInitHash({ hash: Buffer.from(MULTISIG_CODE_HASH, 'hex'), depth: MULTISIG_CODE_DEPTH }, data));

const libraryPrefix = ton.beginCell().storeUint(2, 8).storeBuffer(Buffer.from(ORDER_CODE_HASH, 'hex')).endCell();
const orderCode = new ton.Cell({ exotic: true, bits: libraryPrefix.bits, refs: [] });
const orderInit = (multisig, seqno) => {
  const data = ton.beginCell().storeAddress(multisig).storeUint(seqno, 256).endCell();
  const init = ton.beginCell().store(ton.storeStateInit({ code: orderCode, data })).endCell();
  const address = ton.contractAddress(0, { code: orderCode, data });
  if (!address.hash.equals(stateInitHash({ hash: orderCode.hash(), depth: orderCode.depth() }, data))) {
    throw new Error('state init hash disagrees with @ton/core');
  }
  return { seqno, ...named(address, true), state_init_hash: hex(init.hash()), state_init_boc_hex: hex(init.toBoc()) };
};

const relaxed = (to, value, bounce, body) => {
  const message = ton.beginCell().store(ton.storeMessageRelaxed(ton.internal({ to, value, bounce, body }))).endCell();
  const inRef = message.refs.length > 0 && message.refs[0].equals(body);
  return { ...cell(message), body: inRef ? 'ref' : 'inline' };
};
const newOrder = (queryId, seqno, isSigner, index, expiration, order) => ton.beginCell()
  .storeUint(OP.new_order, 32).storeUint(queryId, 64).storeUint(seqno, 256).storeBit(isSigner)
  .storeUint(index, 8).storeUint(expiration, 48).storeRef(order).endCell();
const comment = (text) => ton.beginCell().storeUint(0, 32).storeStringTail(text).endCell();

// toncenter v2 legacy stack entries.
const num = (n) => ['num', (n < 0n ? '-0x' : '0x') + (n < 0n ? -n : n).toString(16)];
const dataBytes = (bits) => {
  const out = Buffer.alloc(Math.ceil(bits.length / 8));
  for (let i = 0; i < bits.length; i++) if (bits.at(i)) out[i >> 3] |= 0x80 >> (i & 7);
  return out;
};
const cellObject = (c) => ({
  data: { b64: dataBytes(c.bits).toString('base64'), len: c.bits.length },
  refs: c.refs.map(cellObject),
  special: c.isExotic,
});
const cellEntry = (c) => ['cell', { bytes: c.toBoc().toString('base64'), object: cellObject(c) }];
const sliceEntry = (address) => cellEntry(ton.beginCell().storeAddress(address).endCell());
const NULL_ENTRY = ['list', { '@type': 'tvm.list', elements: [] }];
const runGetMethod = (address, method, stack, result) => ({
  request: { address: address.toString({ bounceable: true, urlSafe: true }), method, stack },
  response: { ok: true, result: { '@type': 'smc.runResult', exit_code: 0, stack: result } },
});

async function main() {
  const mnemonics = JSON.parse(fs.readFileSync('core/tests/fixtures/ton-mnemonics.json', 'utf8'));
  const signerEntries = [];
  for (const [index, entry] of mnemonics.mnemonics.slice(0, 3).entries()) {
    const key = await mnemonicToPrivateKey(entry.mnemonic.split(' '));
    const wallet = WalletContractV5R1.create({ workchain: 0, publicKey: key.publicKey, walletId: { networkGlobalId: MAINNET } });
    signerEntries.push({ index, public_key: hex(key.publicKey), ...named(wallet.address, true) });
  }
  const signers = signerEntries.map((entry) => ton.Address.parse(entry.raw));
  const proposers = [new ton.Address(0, Buffer.alloc(32, 0x44))];
  const threshold = 2;
  const nextOrderSeqno = 5;

  const data = multisigData(0, threshold, signers, proposers, false);
  const multisig = multisigAddress(data);
  const signersCell = data.refs[0];
  const proposersCell = ton.beginCell().storeDictDirect(addressDict(proposers)).endCell();

  // Order: one send_message action carrying a non-bounceable 1 TON transfer with comment "rent".
  const destination = new ton.Address(0, Buffer.alloc(32, 0x55));
  const transfer = ton.beginCell().store(ton.storeMessageRelaxed(ton.internal({
    to: destination, value: 1000000000n, bounce: false, body: comment('rent'),
  }))).endCell();
  const action = ton.beginCell().storeUint(OP.send_message, 32).storeUint(3, 8).storeRef(transfer).endCell();
  const actions = ton.Dictionary.empty(ton.Dictionary.Keys.Uint(8), ton.Dictionary.Values.Cell());
  actions.set(0, action);
  const order = ton.beginCell().storeDictDirect(actions).endCell();
  const expiration = 1762592000;

  const orderAddresses = [0, 5, 123].map((seqno) => orderInit(multisig, seqno));
  const orderAddress = ton.Address.parse(orderAddresses[1].raw);

  const signerNewOrder = newOrder(5, 5, true, 1, expiration, order);
  const proposerNewOrder = newOrder(5, 5, false, 0, expiration, order);
  const approve = ton.beginCell().storeUint(OP.approve, 32).storeUint(5, 64).storeUint(2, 8).endCell();
  const approveText = comment('approve');

  // The research vectors used fixed signers 0:11.., 0:22.., 0:33..; the same layout reproduces them.
  const fixedSigners = [0x11, 0x22, 0x33].map((byte) => new ton.Address(0, Buffer.alloc(32, byte)));
  const fixedData = multisigData(0, threshold, fixedSigners, proposers, false);
  const fixedMultisig = multisigAddress(fixedData);

  const fixtures = {
    provenance: `@ton/ton 16.3.0, @ton/core 0.63.1, @ton/crypto 3.3.0; ton-blockchain/multisig-contract-v2 ${SOURCE_COMMIT}`,
    network: 'mainnet',
    workchain: 0,
    multisig_code_hash: MULTISIG_CODE_HASH,
    multisig_code_depth: MULTISIG_CODE_DEPTH,
    order_code_hash: ORDER_CODE_HASH,
    order_code_library_cell: cell(orderCode),
    signers: signerEntries,
    proposers: proposers.map((address, index) => ({ index, ...named(address, true) })),
    threshold,
    allow_arbitrary_order_seqno: false,
    next_order_seqno: nextOrderSeqno,
    multisig: {
      data: cell(data),
      data_at_next_order_seqno: cell(multisigData(nextOrderSeqno, threshold, signers, proposers, false)),
      signers_cell: cell(signersCell),
      proposers_cell: cell(proposersCell),
      state_init_hash: hex(multisig.hash),
      ...named(multisig, true),
    },
    order: {
      seqno: nextOrderSeqno,
      expiration_date: expiration,
      send_mode: 3,
      destination: { ...named(destination, false), bounce: false },
      value: '1000000000',
      comment: 'rent',
      message: { ...cell(transfer), body: transfer.refs.length > 0 ? 'ref' : 'inline' },
      action: cell(action),
      cell: cell(order),
    },
    order_addresses: orderAddresses,
    new_order: {
      signer: { query_id: 5, order_seqno: 5, is_signer: true, index: 1, ...cell(signerNewOrder) },
      proposer: { query_id: 5, order_seqno: 5, is_signer: false, index: 0, ...cell(proposerNewOrder) },
    },
    approve: { query_id: 5, signer_index: 2, ...cell(approve) },
    approve_text: { comment: 'approve', ...cell(approveText) },
    wallet_messages: {
      new_order: { to: named(multisig, true).address, bounce: true, value: '200000000',
        ...relaxed(multisig, 200000000n, true, signerNewOrder) },
      approve: { to: named(orderAddress, true).address, bounce: true, value: '100000000',
        ...relaxed(orderAddress, 100000000n, true, approve) },
    },
    toncenter: {
      null_entry: NULL_ENTRY,
      get_multisig_data: runGetMethod(multisig, 'get_multisig_data', [], [
        num(BigInt(nextOrderSeqno)), num(BigInt(threshold)), cellEntry(signersCell), cellEntry(proposersCell),
      ]),
      get_order_address: runGetMethod(multisig, 'get_order_address', [['num', '5']], [sliceEntry(orderAddress)]),
      get_order_data: runGetMethod(orderAddress, 'get_order_data', [], [
        sliceEntry(multisig), num(5n), num(BigInt(threshold)), num(0n), cellEntry(signersCell),
        num(1n << 1n), num(1n), num(BigInt(expiration)), cellEntry(order),
      ]),
    },
    fixed_signers: {
      signers: fixedSigners.map((address) => address.toRawString()),
      data: cell(fixedData),
      ...named(fixedMultisig, true),
      order_addresses: [0, 123].map((seqno) => {
        const { raw, address } = orderInit(fixedMultisig, seqno);
        return { seqno, raw, address };
      }),
    },
  };
  console.log(JSON.stringify(fixtures, null, 2));
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
