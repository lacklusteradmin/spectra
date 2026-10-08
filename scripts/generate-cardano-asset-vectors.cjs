// Cardano transfers that carry native assets, built by the Cardano
// Serialization Library from explicit inputs, outputs, fee and TTL: a token
// sent from mixed inputs with its minimum ADA, an ADA send spending a
// token-bearing input whose tokens return as change, and change holding two
// policies' assets. Each records CSL's minimum ADA per output and minimum
// fee, and the transaction signed with the abandon-phrase payment key.
// npm install --prefix /tmp/spectra-cardano-vectors --ignore-scripts @emurgo/cardano-serialization-lib-nodejs@15.0.3
// NODE_PATH=/tmp/spectra-cardano-vectors/node_modules node scripts/generate-cardano-asset-vectors.cjs
const fs = require('node:fs');
const C = require('@emurgo/cardano-serialization-lib-nodejs');

const witness = JSON.parse(fs.readFileSync('core/tests/fixtures/cardano-emurgo-witness.json', 'utf8'));
const key = C.PrivateKey.from_extended_bytes(Buffer.from(witness.privateKey, 'hex'));
const sender = Buffer.from(witness.addressBytes, 'hex');
// An enterprise address for the recipient's key hash 0x22…
const recipient = Buffer.concat([Buffer.from([0x61]), Buffer.alloc(28, 0x22)]);
const POLICY_A = 'aa'.repeat(28);
const POLICY_B = '0b'.repeat(28);
const params = { feePerByte: '44', feeFixed: '155381', coinsPerByte: '4310' };

const multiasset = assets => {
  if (!assets.length) return undefined;
  const multi = C.MultiAsset.new();
  for (const { policy, name, quantity } of assets) {
    const policyId = C.ScriptHash.from_bytes(Buffer.from(policy, 'hex'));
    const existing = multi.get(policyId) || C.Assets.new();
    existing.insert(C.AssetName.new(Buffer.from(name, 'hex')), C.BigNum.from_str(quantity));
    multi.insert(policyId, existing);
  }
  return multi;
};
const value = (lovelace, assets) => {
  const v = C.Value.new(C.BigNum.from_str(lovelace));
  const multi = multiasset(assets);
  if (multi) v.set_multiasset(multi);
  return v;
};
const output = (address, lovelace, assets) =>
  C.TransactionOutput.new(C.Address.from_bytes(address), value(lovelace, assets));
const minAda = (address, assets) => C.min_ada_for_output(
  output(address, '0', assets), C.DataCost.new_coins_per_byte(C.BigNum.from_str(params.coinsPerByte))).to_str();

function build(name, inputs, outputs, fee, ttl) {
  const txInputs = C.TransactionInputs.new();
  for (const input of inputs) {
    txInputs.add(C.TransactionInput.new(C.TransactionHash.from_bytes(Buffer.from(input.tx_hash, 'hex')), input.tx_index));
  }
  const txOutputs = C.TransactionOutputs.new();
  for (const out of outputs) txOutputs.add(output(Buffer.from(out.address, 'hex'), out.lovelace, out.assets));
  const body = C.TransactionBody.new_tx_body(txInputs, txOutputs, C.BigNum.from_str(fee));
  body.set_ttl(C.BigNum.from_str(String(ttl)));
  const fixed = C.FixedTransaction.new_from_body_bytes(body.to_bytes());
  fixed.sign_and_add_vkey_signature(key);
  const transaction = C.Transaction.from_bytes(fixed.to_bytes());
  return {
    name,
    inputs,
    outputs: outputs.map(out => ({ ...out, min_ada: minAda(Buffer.from(out.address, 'hex'), out.assets) })),
    fee,
    ttl,
    body: Buffer.from(body.to_bytes()).toString('hex'),
    hash: fixed.transaction_hash().to_hex(),
    transaction: Buffer.from(fixed.to_bytes()).toString('hex'),
    min_fee: C.min_fee(transaction, C.LinearFee.new(C.BigNum.from_str(params.feePerByte), C.BigNum.from_str(params.feeFixed))).to_str(),
  };
}

const hash = byte => byte.repeat(32);
const asset = (policy, name, quantity) => ({ policy, name, quantity });
const cases = [
  // 40 of A.0102 to the recipient with its minimum ADA; the rest of A and
  // all of B come back as change with the remaining ADA.
  build('token_send', [
    { tx_hash: hash('11'), tx_index: 0, lovelace: '5000000', assets: [asset(POLICY_A, '0102', '100'), asset(POLICY_A, '', '7')] },
    { tx_hash: hash('00'), tx_index: 1, lovelace: '3000000', assets: [] },
  ], [
    { address: recipient.toString('hex'), lovelace: '1155080', assets: [asset(POLICY_A, '0102', '40')] },
    { address: sender.toString('hex'), lovelace: '6655731', assets: [asset(POLICY_A, '0102', '60'), asset(POLICY_A, '', '7')] },
  ], '189189', 100),
  // An ADA send that spends a token-bearing input: its token returns.
  build('ada_send_mixed_inputs', [
    { tx_hash: hash('22'), tx_index: 3, lovelace: '2000000', assets: [asset(POLICY_A, '0102', '100')] },
    { tx_hash: hash('33'), tx_index: 0, lovelace: '1500000', assets: [] },
  ], [
    { address: recipient.toString('hex'), lovelace: '2000000', assets: [] },
    { address: sender.toString('hex'), lovelace: '1323675', assets: [asset(POLICY_A, '0102', '100')] },
  ], '176325', 100),
  // Change with two policies, and names of different lengths.
  build('two_policies', [
    { tx_hash: hash('44'), tx_index: 0, lovelace: '10000000', assets: [
      asset(POLICY_B, 'ffff', '1'), asset(POLICY_A, 'cafe0000', '2'), asset(POLICY_A, 'ff', '3'), asset(POLICY_B, '00', '4')] },
  ], [
    { address: recipient.toString('hex'), lovelace: '3000000', assets: [] },
    { address: sender.toString('hex'), lovelace: '6800000', assets: [
      asset(POLICY_B, 'ffff', '1'), asset(POLICY_A, 'cafe0000', '2'), asset(POLICY_A, 'ff', '3'), asset(POLICY_B, '00', '4')] },
  ], '200000', 100),
];

fs.writeFileSync('core/tests/fixtures/cardano-assets.json', JSON.stringify({
  provenance: '@emurgo/cardano-serialization-lib-nodejs 15.0.3',
  params,
  sender: sender.toString('hex'),
  recipient: recipient.toString('hex'),
  cases,
}, null, 2) + '\n');
