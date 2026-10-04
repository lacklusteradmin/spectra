// Official TronWeb protobuf and signing, entirely offline.
// npm install --prefix /tmp/spectra-trc10-sdk --ignore-scripts tronweb@6.0.4
// NODE_PATH=/tmp/spectra-trc10-sdk/node_modules node scripts/generate-trc10-send-vectors.cjs
const fs = require('node:fs');
const { TronWeb, utils } = require('tronweb');
const key = '01'.repeat(32);
const owner = TronWeb.address.fromPrivateKey(key);
const receiver = TronWeb.address.fromPrivateKey('02'.repeat(32));
const block = { number: 12345678, id: '0000000000bc614e1122334455667788' + '99'.repeat(16), timestamp_ms: 1800000000000 };
const vectors = [];
for (const amount of [1, 123456789, 9007199254740991]) {
  const transaction = { visible: false, raw_data: {
    ref_block_bytes: '614e', ref_block_hash: '1122334455667788',
    expiration: block.timestamp_ms + 60000, timestamp: block.timestamp_ms,
    contract: [{ type: 'TransferAssetContract', parameter: {
      type_url: 'type.googleapis.com/protocol.TransferAssetContract',
      value: { asset_name: Buffer.from('1002000').toString('hex'), owner_address: TronWeb.address.toHex(owner),
        to_address: TronWeb.address.toHex(receiver), amount },
    } }],
  } };
  const protobuf = utils.transaction.txJsonToPb(transaction);
  transaction.raw_data_hex = utils.transaction.txPbToRawDataHex(protobuf).toLowerCase();
  transaction.txID = utils.transaction.txPbToTxID(protobuf).replace(/^0x/, '').toLowerCase();
  const signed = utils.crypto.signTransaction(key, transaction);
  vectors.push({ asset_id: '1002000', amount: String(amount), raw: signed.raw_data_hex, txid: signed.txID,
    signature: signed.signature[0] });
}
fs.writeFileSync('core/tests/fixtures/trc10-send-vectors.json', JSON.stringify({
  provenance: 'tronweb 6.0.4 official protobuf and crypto; generated offline',
  sources: ['https://github.com/tronprotocol/protocol/blob/master/core/contract/asset_issue_contract.proto',
    'https://github.com/tronprotocol/protocol/blob/master/core/Tron.proto',
    'https://github.com/tronprotocol/java-tron/blob/develop/chainbase/src/main/java/org/tron/core/db/BandwidthProcessor.java'],
  key, owner, receiver, block, vectors,
}, null, 2) + '\n');
