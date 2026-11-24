// Independent Starknet references for core/tests/fixtures/starknet-vectors.json:
// Ready (formerly Argent X) accounts derived from BIP-39 phrases as the
// wallet's ArgentSigner derives them, their counterfactual addresses as
// @argent/x-shared computes them for account class 0.4.0, and STRK and ETH
// transfers signed by starknet.js as an INVOKE v3, with the DEPLOY_ACCOUNT v3
// an undeployed account sends first.
// node tests/vectors/generate-starknet-vectors.cjs
const crypto = require('node:crypto');
const fs = require('node:fs');
const { HDNodeWallet, Mnemonic } = require('ethers');
const { HDKey } = require('@scure/bip32');
const {
  CairoCustomEnum, CairoOption, CairoOptionVariant, CallData, Signer, constants, ec, encode, getChecksumAddress, hash, num, transaction,
} = require('starknet');

const MNEMONIC = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
// Ready's standard accounts: m/44'/9004'/0'/0/{index} under a BIP-32 master
// seeded with the phrase's Ethereum key (argent-x derivationPaths.ts).
const READY_PATH = "m/44'/9004'/0'/0";
// Argent account 0.4.0, the class @argent/x-shared 1.79.4 names as the latest
// (TXV3_ACCOUNT_CLASS_HASH) and deploys new standard accounts with.
const ACCOUNT_CLASS_HASH = '0x036078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f';
const STRK = '0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d';
const ETH = '0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7';
const CHAIN_ID = constants.StarknetChainId.SN_MAIN;

const hex = (value) => num.toHex(value);
const padded = (value) => encode.addHexPrefix(encode.removeHexPrefix(num.toHex(value)).padStart(64, '0'));

// The wallet secret is the phrase's Ethereum key, as ethers' HDNodeWallet
// reads it (argent-x WalletCryptoSharedService); ArgentSigner turns it into a
// BIP-32 seed through num.toHex, which drops its leading zero bytes.
function readyKey(mnemonic, passphrase, index) {
  const secret = HDNodeWallet.fromPhrase(mnemonic, passphrase).privateKey;
  const seedHex = encode.sanitizeBytes(encode.removeHexPrefix(num.toHex(secret)), 2);
  const child = HDKey.fromMasterSeed(Buffer.from(seedHex, 'hex')).derive(`${READY_PATH}/${index}`);
  const privateKey = encode.addHexPrefix(ec.starkCurve.grindKey(child.privateKey));
  return { secret, privateKey, publicKey: ec.starkCurve.getStarkKey(privateKey) };
}

// @argent/x-shared buildConstructorCalldata for a multi-signer class: the
// owner as a Starknet signer, no guardian; the salt is the public key and the
// deployer zero.
function constructorCalldata(publicKey) {
  return CallData.compile({
    ownerSigner: new CairoCustomEnum({
      Starknet: publicKey, Secp256k1: undefined, Secp256r1: undefined, Eip191: undefined, Webauthn: undefined,
    }),
    guardianAsOption: new CairoOption(CairoOptionVariant.None, null),
  });
}

function accountAddress(publicKey) {
  return padded(hash.calculateContractAddressFromHash(publicKey, ACCOUNT_CLASS_HASH, constructorCalldata(publicKey), 0));
}

function account(mnemonic, passphrase, index) {
  const key = readyKey(mnemonic, passphrase, index);
  return {
    mnemonic,
    passphrase,
    account: index,
    path: `${READY_PATH}/${index}`,
    ethereum_key: key.secret,
    private_key: padded(key.privateKey),
    public_key: padded(key.publicKey),
    address: accountAddress(key.publicKey),
    checksum_address: getChecksumAddress(accountAddress(key.publicKey)),
    constructor_calldata: constructorCalldata(key.publicKey).map(hex),
  };
}

// A phrase whose Ethereum key starts with a zero byte, which ArgentSigner's
// seed leaves out.
function leadingZeroPhrase() {
  for (let i = 0; ; i += 1) {
    const entropy = crypto.createHash('sha256').update(`spectra-starknet-${i}`).digest().subarray(0, 16);
    const phrase = Mnemonic.fromEntropy(entropy).phrase;
    if (HDNodeWallet.fromPhrase(phrase).privateKey.startsWith('0x00')) return phrase;
  }
}

const bounds = (l1, l2, data) => ({
  l1_gas: { max_amount: BigInt(l1[0]), max_price_per_unit: BigInt(l1[1]) },
  l2_gas: { max_amount: BigInt(l2[0]), max_price_per_unit: BigInt(l2[1]) },
  l1_data_gas: { max_amount: BigInt(data[0]), max_price_per_unit: BigInt(data[1]) },
});
const boundsJson = (b) => Object.fromEntries(Object.entries(b).map(([name, bound]) => [name, {
  max_amount: hex(bound.max_amount), max_price_per_unit: hex(bound.max_price_per_unit),
}]));
const v3 = {
  version: '0x3', tip: 0, paymasterData: [], accountDeploymentData: [], nonceDataAvailabilityMode: 'L1', feeDataAvailabilityMode: 'L1',
};

async function transfer(sender, terms) {
  const signer = new Signer(sender.private_key);
  const calls = [{
    contractAddress: terms.token,
    entrypoint: 'transfer',
    calldata: CallData.compile({ recipient: terms.recipient, amount: { low: BigInt(terms.amount), high: 0n } }),
  }];
  const calldata = transaction.getExecuteCalldata(calls, '1').map(hex);
  const details = {
    ...v3, walletAddress: sender.address, chainId: CHAIN_ID, cairoVersion: '1', nonce: terms.nonce, resourceBounds: terms.bounds,
  };
  const invokeHash = padded(hash.calculateInvokeTransactionHash({
    senderAddress: sender.address, version: '0x3', compiledCalldata: calldata, chainId: CHAIN_ID, nonce: terms.nonce,
    accountDeploymentData: [], nonceDataAvailabilityMode: 0, feeDataAvailabilityMode: 0, resourceBounds: terms.bounds, tip: 0, paymasterData: [],
  }));
  const signature = await signer.signTransaction(calls, details);
  if (!ec.starkCurve.verify(signature, invokeHash, ec.starkCurve.getPublicKey(sender.private_key))) throw new Error('invoke signature');
  const invokeSignature = [hex(signature.r), hex(signature.s)];
  const result = {
    name: terms.name,
    private_key: sender.private_key,
    public_key: sender.public_key,
    sender: sender.address,
    token: terms.token,
    recipient: padded(terms.recipient),
    amount: terms.amount,
    nonce: terms.nonce,
    resource_bounds: boundsJson(terms.bounds),
    calldata,
    transaction_hash: invokeHash,
    signature: invokeSignature,
    // As starknet.js's RPC channel writes the request (buildTransaction),
    // which passes the sender through as given: here as `num.toHex` writes
    // a felt, without leading zeros, as the spec's FELT pattern has it.
    invoke_transaction: {
      type: 'INVOKE', sender_address: hex(sender.address), calldata, signature: invokeSignature, nonce: hex(terms.nonce),
      resource_bounds: boundsJson(terms.bounds), tip: '0x0', paymaster_data: [], nonce_data_availability_mode: 'L1',
      fee_data_availability_mode: 'L1', account_deployment_data: [], version: '0x3',
    },
    deployment: null,
  };
  if (terms.deploy) {
    const constructor = constructorCalldata(sender.public_key);
    const deployDetails = {
      ...v3, classHash: ACCOUNT_CLASS_HASH, contractAddress: sender.address, constructorCalldata: constructor,
      addressSalt: sender.public_key, chainId: CHAIN_ID, nonce: 0, resourceBounds: terms.deploy,
    };
    const deployHash = padded(hash.calculateDeployAccountTransactionHash({
      contractAddress: sender.address, classHash: ACCOUNT_CLASS_HASH, compiledConstructorCalldata: constructor,
      salt: sender.public_key, version: '0x3', chainId: CHAIN_ID, nonce: 0, nonceDataAvailabilityMode: 0, feeDataAvailabilityMode: 0,
      resourceBounds: terms.deploy, tip: 0, paymasterData: [],
    }));
    const deploySignature = await signer.signDeployAccountTransaction(deployDetails);
    if (!ec.starkCurve.verify(deploySignature, deployHash, ec.starkCurve.getPublicKey(sender.private_key))) throw new Error('deploy signature');
    const signatureHex = [hex(deploySignature.r), hex(deploySignature.s)];
    result.deployment = {
      class_hash: ACCOUNT_CLASS_HASH,
      contract_address_salt: sender.public_key,
      constructor_calldata: constructor.map(hex),
      resource_bounds: boundsJson(terms.deploy),
      transaction_hash: deployHash,
      signature: signatureHex,
      deploy_account_transaction: {
        type: 'DEPLOY_ACCOUNT', constructor_calldata: constructor.map(hex), class_hash: hex(ACCOUNT_CLASS_HASH),
        contract_address_salt: hex(sender.public_key), signature: signatureHex, nonce: '0x0',
        resource_bounds: boundsJson(terms.deploy), tip: '0x0', paymaster_data: [], nonce_data_availability_mode: 'L1',
        fee_data_availability_mode: 'L1', version: '0x3',
      },
    };
  }
  return result;
}

(async () => {
  const zeroLed = leadingZeroPhrase();
  const accounts = [
    account(MNEMONIC, '', 0),
    account(MNEMONIC, '', 1),
    account(MNEMONIC, '', 2),
    account(MNEMONIC, 'TREZOR', 0),
    account(zeroLed, '', 0),
  ];
  const [first, second, third] = accounts;
  const transfers = [
    await transfer(first, {
      name: 'strk-from-a-deployed-account', token: STRK, recipient: second.address, amount: '1500000000000000000', nonce: 7,
      bounds: bounds([0, '0x5af3107a4000'], [1302848, '0x2cb417800'], [384, '0x1b2']),
    }),
    await transfer(second, {
      name: 'eth-from-an-undeployed-account', token: ETH, recipient: '0x' + 'ab'.repeat(31), amount: '250000000000000', nonce: 1,
      bounds: bounds([0, '0x5af3107a4000'], [1450000, '0x2cb417800'], [256, '0x1b2']),
      deploy: bounds([0, '0x5af3107a4000'], [900000, '0x2cb417800'], [480, '0x1b2']),
    }),
    await transfer(third, {
      name: 'the-largest-amount-to-a-short-address', token: STRK, recipient: '0x1234', amount: '340282366920938463463374607431768211455', nonce: 4294967296,
      bounds: bounds([10, 1], [1, 1], [1, 1]),
    }),
  ];
  const selectors = Object.fromEntries(['transfer', 'balanceOf', 'balance_of', '__execute__', 'decimals'].map((name) => [name, hash.getSelectorFromName(name)]));
  const fixture = {
    provenance: 'starknet 11.6.1 (@scure/starknet 2.4.0), ethers 6.17.0, @scure/bip32 1.7.0; derivation after argent-x '
      + 'ArgentSigner and the address after @argent/x-shared 1.79.4 getAccountDeploymentPayload',
    chain_id: CHAIN_ID,
    account_class_hash: ACCOUNT_CLASS_HASH,
    strk: STRK,
    eth: ETH,
    selectors,
    accounts,
    transfers,
  };
  fs.writeFileSync('core/tests/fixtures/starknet-vectors.json', JSON.stringify(fixture, null, 2) + '\n');
})();
