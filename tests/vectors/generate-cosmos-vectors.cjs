// Independent Cosmos SDK references: phrase addresses on every Cosmos network
// Spectra lists, and bank MsgSend transfers built, signed and hashed by CosmJS
// (SDK secp256k1) and by CosmJS's protobuf with ethers.js's signing key
// (Ethermint eth_secp256k1, as Injective and Dymension take it).
// node tests/vectors/generate-cosmos-vectors.cjs
const fs = require('node:fs');
const { DirectSecp256k1HdWallet, makeAuthInfoBytes, makeSignDoc, makeSignBytes, encodePubkey } = require('@cosmjs/proto-signing');
const { Slip10, Slip10Curve, Secp256k1, Bip39, EnglishMnemonic, sha256, stringToPath } = require('@cosmjs/crypto');
const { toBech32, toHex, toBase64 } = require('@cosmjs/encoding');
const { TxBody, TxRaw } = require('cosmjs-types/cosmos/tx/v1beta1/tx');
const { MsgSend } = require('cosmjs-types/cosmos/bank/v1beta1/tx');
const { PubKey } = require('cosmjs-types/cosmos/crypto/secp256k1/keys');
const { SigningKey, computeAddress, keccak256, getBytes } = require('ethers');

const MNEMONIC = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';

const NETWORKS = [
  { chain: 'cosmos-hub', hrp: 'cosmos', coinType: 118 },
  { chain: 'celestia', hrp: 'celestia', coinType: 118 },
  { chain: 'dydx', hrp: 'dydx', coinType: 118 },
  { chain: 'terra', hrp: 'terra', coinType: 330 },
  { chain: 'kava', hrp: 'kava', coinType: 459 },
  { chain: 'injective', hrp: 'inj', coinType: 60, eth: '/injective.crypto.v1beta1.ethsecp256k1.PubKey' },
  { chain: 'akash', hrp: 'akash', coinType: 118 },
  { chain: 'babylon', hrp: 'bbn', coinType: 118 },
  { chain: 'osmosis', hrp: 'osmo', coinType: 118 },
  { chain: 'dymension', hrp: 'dym', coinType: 60, eth: '/ethermint.crypto.v1.ethsecp256k1.PubKey' },
  { chain: 'saga', hrp: 'saga', coinType: 118 },
  { chain: 'allora', hrp: 'allo', coinType: 118 },
  { chain: 'band', hrp: 'band', coinType: 494 },
];

async function keyAt(path) {
  const seed = await Bip39.mnemonicToSeed(new EnglishMnemonic(MNEMONIC));
  const { privkey } = Slip10.derivePath(Slip10Curve.Secp256k1, seed, stringToPath(path));
  const { pubkey } = await Secp256k1.makeKeypair(privkey);
  return { privkey, compressed: Secp256k1.compressPubkey(pubkey) };
}

async function address(network, account) {
  const path = `m/44'/${network.coinType}'/${account}'/0/0`;
  if (network.eth) {
    const { privkey, compressed } = await keyAt(path);
    const eth = getBytes(computeAddress('0x' + toHex(privkey)));
    return { path, address: toBech32(network.hrp, eth), public_key: toHex(compressed) };
  }
  const wallet = await DirectSecp256k1HdWallet.fromMnemonic(MNEMONIC, {
    prefix: network.hrp,
    hdPaths: [stringToPath(path)],
  });
  const [{ address: bech32, pubkey }] = await wallet.getAccounts();
  return { path, address: bech32, public_key: toHex(pubkey) };
}

function body(transfer) {
  return TxBody.encode(TxBody.fromPartial({
    messages: [{
      typeUrl: '/cosmos.bank.v1beta1.MsgSend',
      value: MsgSend.encode(MsgSend.fromPartial({
        fromAddress: transfer.sender,
        toAddress: transfer.recipient,
        amount: [{ denom: transfer.denom, amount: transfer.amount }],
      })).finish(),
    }],
    memo: transfer.memo,
    timeoutHeight: BigInt(transfer.timeout_height),
  })).finish();
}

async function transfer(network, terms) {
  const path = `m/44'/${network.coinType}'/0'/0/0`;
  const { privkey, compressed } = await keyAt(path);
  const sender = (await address(network, 0)).address;
  const recipient = (await address(network, 1)).address;
  const fixture = { chain: network.chain, derivation_path: path, private_key: toHex(privkey), sender, recipient, ...terms };
  const bodyBytes = body(fixture);
  const pubkey = network.eth
    ? { typeUrl: network.eth, value: PubKey.encode(PubKey.fromPartial({ key: compressed })).finish() }
    : encodePubkey({ type: 'tendermint/PubKeySecp256k1', value: toBase64(compressed) });
  const fee = BigInt(terms.fee) > 0n ? [{ denom: terms.denom, amount: terms.fee }] : [];
  const authInfoBytes = makeAuthInfoBytes([{ pubkey, sequence: BigInt(terms.sequence) }], fee, terms.gas_limit);
  const signDoc = makeSignDoc(bodyBytes, authInfoBytes, terms.chain_id, terms.account_number);
  const signBytes = makeSignBytes(signDoc);
  let signature;
  if (network.eth) {
    const signed = new SigningKey('0x' + toHex(privkey)).sign(keccak256(signBytes));
    signature = getBytes(signed.r + signed.s.slice(2));
  } else {
    const wallet = await DirectSecp256k1HdWallet.fromMnemonic(MNEMONIC, {
      prefix: network.hrp,
      hdPaths: [stringToPath(path)],
    });
    const { signature: direct } = await wallet.signDirect(sender, signDoc);
    signature = Buffer.from(direct.signature, 'base64');
  }
  const raw = TxRaw.encode(TxRaw.fromPartial({ bodyBytes, authInfoBytes, signatures: [signature] })).finish();
  return {
    ...fixture,
    body_bytes: toHex(bodyBytes),
    auth_info_bytes: toHex(authInfoBytes),
    sign_bytes: toHex(signBytes),
    signature: toHex(signature),
    tx_raw: toHex(raw),
    tx_hash: toHex(sha256(raw)).toUpperCase(),
  };
}

(async () => {
  const addresses = [];
  for (const network of NETWORKS) {
    for (const account of [0, 1]) {
      addresses.push({ chain: network.chain, account, ...(await address(network, account)) });
    }
  }
  const byChain = Object.fromEntries(NETWORKS.map((network) => [network.chain, network]));
  const transfers = [
    await transfer(byChain['cosmos-hub'], {
      chain_id: 'cosmoshub-4', denom: 'uatom', amount: '1234567', memo: '104857600', account_number: 789012, sequence: 42,
      gas_limit: 98765, fee: '494', timeout_height: '28500123',
    }),
    await transfer(byChain.terra, {
      chain_id: 'phoenix-1', denom: 'uluna', amount: '1', memo: '', account_number: 0, sequence: 0,
      gas_limit: 100000, fee: '0', timeout_height: '1',
    }),
    await transfer(byChain.injective, {
      chain_id: 'injective-1', denom: 'inj', amount: '1000000000000000000', memo: 'deposit-7', account_number: 31337, sequence: 9,
      gas_limit: 120000, fee: '19200000000000', timeout_height: '150000000',
    }),
    await transfer(byChain.dymension, {
      chain_id: 'dymension_1100-1', denom: 'adym', amount: '25000000000000000000', memo: '', account_number: 4, sequence: 1,
      gas_limit: 140000, fee: '2800000000000000', timeout_height: '9000000',
    }),
  ];
  const fixture = {
    provenance: '@cosmjs/proto-signing 0.39.0, @cosmjs/crypto 0.39.0, @cosmjs/encoding 0.39.0, cosmjs-types 0.11.0, ethers 6.17.0',
    mnemonic: MNEMONIC,
    addresses,
    transfers,
  };
  fs.writeFileSync('core/tests/fixtures/cosmos-vectors.json', JSON.stringify(fixture, null, 2) + '\n');
})();
