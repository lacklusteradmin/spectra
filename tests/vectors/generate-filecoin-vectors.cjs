// Independent Filecoin references: f1 addresses along m/44'/461'/{account}'/0/0,
// the bytes of an address of every protocol a send pays, and messages
// serialized, given their CIDs and signed by Zondax's filecoin-signing-tools
// (its Rust signer built for Node), each checked against the same library's
// pure JavaScript implementation, Glif's address codec and a signature
// recovered with tiny-secp256k1. One message is mainnet's own, read from a
// Lotus node, whose CIDs the chain gave it.
// node tests/vectors/generate-filecoin-vectors.cjs
const fs = require('node:fs');
const assert = require('node:assert');
const signer = require('@zondax/filecoin-signing-tools');
const glif = require('@glif/filecoin-address');
const ecc = require('tiny-secp256k1');
const { blake2b } = require('@noble/hashes/blake2.js');

const MNEMONIC = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const INVOKE_EVM = 3844450837;

// Read from https://api.node.glif.io/rpc/v1 on 2026-11-24: the message's
// signed CID came from `Filecoin.StateSearchMsg`, and the message with its
// signature from `Filecoin.ChainGetBlockMessages` for block
// bafy2bzaceaklkwy724jocvkit4rxiylig5hfqvsgjeg4r4pwcxvydjxyty2o6 at height
// 6573567.
const MAINNET = {
  message: {
    To: 'f12lj6ctbqroce5uzftmnuotmhb3e5d3lqcendm4i',
    From: 'f1up5qywgfhygtqmxlvke3syvgldkqavicn5derri',
    Nonce: 6533,
    Value: '1000000000000',
    GasLimit: 10000000,
    GasFeeCap: '510000',
    GasPremium: '500000',
    Method: 0,
    Params: '',
  },
  signature: 'pciA6DCcznxCADwvWRFmFXJxaNY2E9c+vwF0eF9lrSIVx8SjzapMiE0djLdjFLscbu8qvleCQtOSxmljOLQq3AA=',
  cid: 'bafy2bzacechqqlkfsb2lu3cgjliyfsao2sprjk276vawiiji5f4jlhx5pvc7c',
  signed_cid: 'bafy2bzaceakqouboqyxtz6mp2up2bgd2qyd5qk6vjklcveik3v3fqf2taqq7g',
};

const hex = (bytes) => Buffer.from(bytes).toString('hex');

(async () => {
  const js = await import(require.resolve('@zondax/filecoin-signing-tools/js'));

  function account(index) {
    const path = `m/44'/461'/${index}'/0/0`;
    const key = signer.keyDerive(MNEMONIC, path, '', 'en');
    const twin = js.keyDerive(MNEMONIC, path, '');
    assert.equal(twin.address, key.address);
    assert.equal(hex(twin.publicKey), key.public_hexstring);
    const glifAddress = glif.newSecp256k1Address(Buffer.from(key.public_hexstring, 'hex')).toString();
    assert.equal(glifAddress, key.address);
    return {
      account: index,
      path,
      private_key: key.private_hexstring,
      public_key: key.public_hexstring,
      address: key.address,
      private_base64: key.private_base64,
    };
  }

  // The bytes a message carries for an address, as both codecs read it.
  // Zondax's JavaScript reads no ID longer than 16 digits; the largest ID
  // is checked against its Rust signer through a message that pays it.
  function addressBytes(address) {
    const bytes = hex(glif.newFromString(address).bytes);
    if (address.length <= 18) assert.equal(hex(js.parseAddress(address)), bytes);
    return bytes;
  }

  const accounts = [account(0), account(1)];
  const ethereum = '0x8ba1f109551bd432803012645ac136ddd64dba72';
  const recipients = [
    { protocol: 'id', address: 'f01234' },
    { protocol: 'id', address: 'f09223372036854775807' },
    { protocol: 'secp256k1', address: accounts[1].address },
    { protocol: 'actor', address: glif.newActorAddress(Buffer.from('satoshi')).toString() },
    // A mainnet BLS key: it signed messages in block 6573567.
    { protocol: 'bls', address: 'f3qkdqari4anmncqhccdewlupij3ifel2qhw5aqgqwvffymgg7bdqiitslgsporoewohph6ljdjz7xdgize47a' },
    { protocol: 'delegated', address: glif.delegatedFromEthAddress(ethereum), ethereum },
  ].map((recipient) => ({ ...recipient, bytes: addressBytes(recipient.address) }));
  assert.equal(glif.ethAddressFromDelegated(recipients[5].address).toLowerCase(), ethereum);
  const largest = recipients[1];
  const paysLargest = signer.transactionSerialize({
    To: largest.address, From: accounts[0].address, Nonce: 0, Value: '1', Method: 0, Params: '',
    GasLimit: 1, GasFeeCap: '1', GasPremium: '1',
  });
  assert.equal(paysLargest.slice(4, 6 + largest.bytes.length), (0x40 + largest.bytes.length / 2).toString(16) + largest.bytes);
  const to = (protocol) => recipients.find((recipient) => recipient.protocol === protocol).address;

  // A message as Lotus writes it, serialized, its CID, and signed with
  // `key`: the signature over BLAKE2b-256 of the CID's bytes, and the
  // signed message's CID, which is what the chain finds it by.
  function message(lotus, key) {
    const cbor = signer.transactionSerialize(signer.transactionParse(js.transactionSerialize(lotus), false));
    assert.equal(cbor, js.transactionSerialize(lotus));
    const cid = signer.getCid(signer.transactionParse(cbor, false));
    const entry = { message: lotus, cbor, cid };
    if (!key) return entry;
    const signed = signer.transactionSignLotus(signer.transactionParse(cbor, false), key.private_base64);
    const signature = Buffer.from(signed.Signature.Data, 'base64');
    assert.equal(signature.length, 65);
    assert.equal(hex(js.transactionSignRaw(lotus, Buffer.from(key.private_hexstring ?? key.private_key, 'hex'))), hex(signature));
    // The CID's bytes: version 1, DAG-CBOR, a 32-byte BLAKE2b-256 multihash.
    const cidBytes = Buffer.concat([Buffer.from('0171a0e40220', 'hex'), blake2b(Buffer.from(cbor, 'hex'), { dkLen: 32 })]);
    const digest = blake2b(cidBytes, { dkLen: 32 });
    const recovered = ecc.recover(digest, signature.subarray(0, 64), signature[64], false);
    assert.equal(hex(recovered), key.public_key);
    return {
      ...entry,
      cid_bytes: hex(cidBytes),
      signing_digest: hex(digest),
      signature: hex(signature),
      signed_cid: signer.getCid({ Message: signed.Message, Signature: signed.Signature }),
    };
  }

  const sender = accounts[0];
  const base = { From: sender.address, Params: '' };
  const messages = [
    { To: to('secp256k1'), Nonce: 0, Value: '1500000000000000000', GasLimit: 1526328, GasFeeCap: '101213', GasPremium: '100022', Method: 0 },
    // A zero premium is a big integer of no bytes.
    { To: to('id'), Nonce: 7, Value: '1', GasLimit: 7169093, GasFeeCap: '102071', GasPremium: '0', Method: 0 },
    { To: to('delegated'), Nonce: 42, Value: '25000000000000000000', GasLimit: 2261790, GasFeeCap: '102071', GasPremium: '101017', Method: INVOKE_EVM },
    // More FIL than a u64 of attoFIL holds, and a nonce of eight bytes.
    { To: to('bls'), Nonce: 9007199254740991, Value: '123456789000000000000000000', GasLimit: 10000000000, GasFeeCap: '100', GasPremium: '100', Method: 0 },
    { To: to('actor'), Nonce: 300, Value: '1000', GasLimit: 1526328, GasFeeCap: '101213', GasPremium: '100022', Method: 0 },
  ].map((fields) => ({ ...message({ ...base, ...fields }, sender), private_key: sender.private_key, sender: sender.address }));

  // Mainnet's message: the same library gives it the CIDs the chain did,
  // and its signature recovers the key its sender's address hashes.
  const onchain = message(MAINNET.message);
  assert.equal(onchain.cid, MAINNET.cid);
  const signedCid = signer.getCid({ Message: signer.transactionParse(onchain.cbor, false), Signature: { Type: 1, Data: MAINNET.signature } });
  assert.equal(signedCid, MAINNET.signed_cid);
  const mainnetSignature = Buffer.from(MAINNET.signature, 'base64');
  const mainnetCidBytes = Buffer.concat([Buffer.from('0171a0e40220', 'hex'), blake2b(Buffer.from(onchain.cbor, 'hex'), { dkLen: 32 })]);
  const mainnetKey = ecc.recover(blake2b(mainnetCidBytes, { dkLen: 32 }), mainnetSignature.subarray(0, 64), mainnetSignature[64], false);
  assert.equal(glif.newSecp256k1Address(mainnetKey).toString(), MAINNET.message.From);

  const fixture = {
    provenance: '@zondax/filecoin-signing-tools 2.4.3, @glif/filecoin-address 4.0.0, tiny-secp256k1 2.2.4, @noble/hashes 1.8.0; '
      + 'mainnet message read from api.node.glif.io on 2026-11-24',
    mnemonic: MNEMONIC,
    accounts: accounts.map(({ private_base64, ...rest }) => rest),
    recipients,
    messages,
    mainnet: { ...onchain, signature: hex(mainnetSignature), signed_cid: MAINNET.signed_cid },
  };
  fs.writeFileSync('core/tests/fixtures/filecoin-vectors.json', JSON.stringify(fixture, null, 2) + '\n');
})();
