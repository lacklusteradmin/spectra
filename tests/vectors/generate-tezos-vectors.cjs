// Independent Tezos references from Taquito: tz1 accounts derived from a
// BIP-39 phrase at m/44'/1729'/{account}'/0' (Temple, Kukai and Umami's
// default), the edsk key strings they export, recipients of every kind a
// transfer may name, and reveal and transaction operations forged by
// @taquito/local-forging for the protocol mainnet runs, signed by
// @taquito/signer under the generic-operation watermark 0x03 and hashed by
// @taquito/utils. Three operations included on mainnet are forged again and
// must hash to the operation hash the chain gives them.
// node tests/vectors/generate-tezos-vectors.cjs
const fs = require('node:fs');
const bip39 = require('bip39');
const { derivePath: slip10 } = require('ed25519-hd-key');
const { LocalForger, ProtocolsHash } = require('@taquito/local-forging');
const { InMemorySigner } = require('@taquito/signer');
const {
  b58Encode, b58DecodeAndCheckPrefix, PrefixV2, encodeOpHash, verifySignature, buf2hex,
} = require('@taquito/utils');

const MNEMONIC = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const PROTOCOL = ProtocolsHash.PsUshuai9;
const forger = new LocalForger(PROTOCOL);
const WATERMARK = new Uint8Array([3]);

// A mainnet block hash, as a branch.
const BRANCH = 'BLXuuBECtE1bYReydLQTSD1FgYwmgg6VbEBSRf6nHwxrhdNvff5';

const path = (account) => `m/44'/1729'/${account}'/0'`;

async function account(index) {
  const signer = InMemorySigner.fromMnemonic({ mnemonic: MNEMONIC, derivationPath: path(index).slice(2), curve: 'ed25519' });
  // ed25519-hd-key's SLIP-10 must give the seed Taquito's own derivation did.
  const seed = Buffer.from(slip10(path(index), bip39.mnemonicToSeedSync(MNEMONIC).toString('hex')).key);
  const edskSeed = b58Encode(seed, PrefixV2.Ed25519Seed);
  const publicKey = await signer.publicKey();
  const [publicBytes] = b58DecodeAndCheckPrefix(publicKey, [PrefixV2.Ed25519PublicKey]);
  const edskSecret = b58Encode(Buffer.concat([seed, Buffer.from(publicBytes)]), PrefixV2.Ed25519SecretKey);
  const address = await signer.publicKeyHash();
  for (const key of [edskSeed, edskSecret]) {
    if (await (await InMemorySigner.fromSecretKey(key)).publicKeyHash() !== address) throw new Error(`edsk ${key}`);
  }
  return {
    account: index,
    path: path(index),
    private_key: seed.toString('hex'),
    public_key: publicKey,
    public_key_hex: buf2hex(publicBytes),
    address,
    edsk_seed: edskSeed,
    edsk_secret_key: edskSecret,
  };
}

// Recipients of every kind: a tz2 and a tz3 account from the same phrase,
// a tz4 BLS account from a fixed secret, and a mainnet contract.
async function recipients() {
  const tz2 = await InMemorySigner.fromMnemonic({ mnemonic: MNEMONIC, derivationPath: "44'/1729'/0'/0'", curve: 'secp256k1' }).publicKeyHash();
  const tz3 = await InMemorySigner.fromMnemonic({ mnemonic: MNEMONIC, derivationPath: "44'/1729'/0'/0'", curve: 'p256' }).publicKeyHash();
  const bls = b58Encode(new Uint8Array(32).fill(7), PrefixV2.BLS12_381SecretKey);
  const tz4 = await (await InMemorySigner.fromSecretKey(bls)).publicKeyHash();
  return { tz2, tz3, tz4, kt1: 'KT1H5b7LxEExkFd2Tng77TfuWbM5aPvHstPr' };
}

async function operation(name, signerAccount, contents) {
  const signer = InMemorySigner.fromMnemonic({ mnemonic: MNEMONIC, derivationPath: signerAccount.path.slice(2), curve: 'ed25519' });
  const forged = await forger.forge({ branch: BRANCH, contents });
  // Taquito reads back exactly what it wrote.
  const parsed = await forger.parse(forged);
  if (JSON.stringify(parsed.contents) !== JSON.stringify(contents)) throw new Error(`${name}: parse`);
  const signed = await signer.sign(forged, WATERMARK);
  if (!verifySignature(forged, signerAccount.public_key, signed.prefixSig, WATERMARK)) throw new Error(`${name}: verify`);
  return {
    name,
    branch: BRANCH,
    contents,
    forged,
    signature: signed.prefixSig,
    signature_hex: signed.sbytes.slice(forged.length),
    signed: signed.sbytes,
    hash: encodeOpHash(signed.sbytes),
  };
}

const transaction = (source, fields) => ({
  kind: 'transaction', source: source.address, fee: '0', counter: '0', gas_limit: '0', storage_limit: '0', amount: '0', ...fields,
});

// Operations included on mainnet (protocol PsUshuai…, 2026-11-24), as a
// node's RPC answers them: forged again with the signature they carry, each
// must hash to its own operation hash.
const ONCHAIN = [
  {
    hash: 'ootbVsgxdptx4MrpHNkRjLSkRTR5PbDeAH9XVVR9xc8N69UHFsm',
    level: 15953142,
    branch: 'BLXuuBECtE1bYReydLQTSD1FgYwmgg6VbEBSRf6nHwxrhdNvff5',
    contents: [
      { kind: 'reveal', source: 'tz1LHQgG3wJPBtxqUDJSbN4FX2WNPiuurj2M', fee: '291', counter: '240753239', gas_limit: '270', storage_limit: '0', public_key: 'edpkuc4t4nhvBkMHa3bDUAKL8hpzT6jQGUFfzwVa12nGKZDrykjonK' },
      { kind: 'transaction', source: 'tz1LHQgG3wJPBtxqUDJSbN4FX2WNPiuurj2M', fee: '578', counter: '240753240', gas_limit: '2201', storage_limit: '0', amount: '28419350', destination: 'tz1cvjyVyd2T5Wozmx7CTNBH8yofzyLyJpi7' },
    ],
    signature: 'siggWQwhgoDKshc5cYNrepcshE59iL9wYYykJCHrueMDpcmD14RfuP9uiywCGrVoFVXGx3CtzCDRgFBA28rEeGLTwYFFHGRs',
    public_key: 'edpkuc4t4nhvBkMHa3bDUAKL8hpzT6jQGUFfzwVa12nGKZDrykjonK',
  },
  {
    hash: 'ooUdbpEtBjmifpzxPxkT8BSotG9Wsej3n7JeqMbSNJ8cRfXfAJq',
    level: 15953458,
    branch: 'BKzRoytPVjax2ZFUJesNnMcfvJHq879X4QugapKxXV7szoRAAQx',
    contents: [
      { kind: 'transaction', source: 'tz1UKmZhi8dhUX5a5QTfCrsH9pK4dt1dVfJo', fee: '2000', counter: '226364333', gas_limit: '10600', storage_limit: '600', amount: '990457', destination: 'tz1YTqCkRmoB43A4KSe65Z8bjRt16KMqFcMK' },
    ],
    signature: 'sigTmsXkm31Gq7EW3GbZSwgKthHTWZkipdAW4DWVnNcCDxNibXyt5FqVemfDRGXUJGTx9hfmvQzKp2B9Xg5CqK5WSRQPVsz8',
  },
  {
    hash: 'oo4tJTxKbT4qJ9apJbCWLKX7EGTtKy7UJhPqmsvw3Sp1zWjfMXn',
    level: 15953458,
    branch: 'BLghfuRzu1zRtwNvfMrcbSBQeXEbYrc55K9FTdW4htq8Cbsy39i',
    contents: [
      { kind: 'transaction', source: 'tz2FFbevyoF259M2KjEB4LcSJzUa8bgWryEy', fee: '515', counter: '244942662', gas_limit: '2155', storage_limit: '257', amount: '25531312', destination: 'tz2PUxDFm8Mz7aCPoxzoRA3dv9GBiakWVxiN' },
    ],
    signature: 'sigjFgwvfzq7guFEmc5FBW3JrAMNhQd1rYYJHnrXSUvksLthqnaqz25Bs4tAMoUcwseuPgW8bWcRSaWMw8qnqjgT9gftJZkE',
  },
];

async function onchain(record) {
  const forged = await forger.forge({ branch: record.branch, contents: record.contents });
  const [signature] = b58DecodeAndCheckPrefix(record.signature, [PrefixV2.GenericSignature]);
  const signed = forged + buf2hex(signature);
  if (encodeOpHash(signed) !== record.hash) throw new Error(`on-chain ${record.hash}`);
  if (record.public_key) {
    const edsig = b58Encode(signature, PrefixV2.Ed25519Signature);
    if (!verifySignature(forged, record.public_key, edsig, WATERMARK)) throw new Error(`on-chain signature ${record.hash}`);
  }
  return { ...record, protocol: PROTOCOL, forged, signature_hex: buf2hex(signature), signed };
}

(async () => {
  const accounts = [];
  for (const index of [0, 1, 2]) accounts.push(await account(index));
  const [sender, recipient, unrevealed] = accounts;
  const others = await recipients();
  const operations = [
    await operation('transfer', sender, [
      transaction(sender, { fee: '580', counter: '7', gas_limit: '2269', storage_limit: '0', amount: '1500000', destination: recipient.address }),
    ]),
    await operation('reveal and transfer to an empty account', unrevealed, [
      { kind: 'reveal', source: unrevealed.address, fee: '274', counter: '246598708', gas_limit: '271', storage_limit: '0', public_key: unrevealed.public_key },
      transaction(unrevealed, { fee: '529', counter: '246598709', gas_limit: '2201', storage_limit: '257', amount: '123456789', destination: sender.address }),
    ]),
    await operation('transfer to tz2', sender, [
      transaction(sender, { fee: '1000', counter: '127', gas_limit: '128', storage_limit: '16383', amount: '16384', destination: others.tz2 }),
    ]),
    await operation('transfer to tz3', sender, [
      transaction(sender, { fee: '9223372036854775807', counter: '18446744073709551615', gas_limit: '1040000', storage_limit: '60000', amount: '9223372036854775807', destination: others.tz3 }),
    ]),
    await operation('transfer to tz4', sender, [
      transaction(sender, { fee: '600', counter: '1', gas_limit: '2101', storage_limit: '0', amount: '1', destination: others.tz4 }),
    ]),
    await operation('transfer to a contract', sender, [
      transaction(sender, { fee: '2500', counter: '42', gas_limit: '12000', storage_limit: '120', amount: '5000000', destination: others.kt1 }),
    ]),
  ];
  const onchainOperations = [];
  for (const record of ONCHAIN) onchainOperations.push(await onchain(record));
  const fixture = {
    provenance: '@taquito/local-forging 25.1.1, @taquito/signer 25.1.1, @taquito/utils 25.1.1, ed25519-hd-key 2.0.0, bip39 3.1.0',
    protocol: PROTOCOL,
    chain_id: 'NetXdQprcVkpaWU',
    mnemonic: MNEMONIC,
    accounts,
    recipients: others,
    operations,
    onchain: onchainOperations,
  };
  fs.writeFileSync('core/tests/fixtures/tezos-vectors.json', JSON.stringify(fixture, null, 2) + '\n');
})();
