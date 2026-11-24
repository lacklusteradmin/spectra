// Independent Algorand references: 25-word mnemonics, keys and addresses, and
// pay transactions encoded, named and signed by algosdk; BIP-39 accounts
// along Pera's Universal Wallet derivation (ARC-52, BIP32-Ed25519 with
// Peikert's amendment, m/44'/283'/{account}'/0/0) derived and signed by the
// Algorand Foundation's xhd-wallet-api.
// node tests/vectors/generate-algorand-vectors.cjs
const fs = require('node:fs');
const crypto = require('node:crypto');
const bip39 = require('bip39');

const PHRASE = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
// The ARC-52 reference vector, from xhd-wallet-api's README.
const README_PHRASE = 'salon zoo engage submit smile frost later decide wing sight chaos renew lizard rely canal coral scene hobby scare step bus leaf tobacco slice';
const GENESIS_ID = 'mainnet-v1.0';
const GENESIS_HASH = 'wGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit8=';

const hex = (bytes) => Buffer.from(bytes).toString('hex');
const seedOf = (label) => crypto.createHash('sha256').update(label).digest();

(async () => {
  const algosdk = await import('algosdk');
  const xhd = await import('@algorandfoundation/xhd-wallet-api');
  const api = new xhd.XHDWalletAPI();
  const PEIKERT = xhd.BIP32DerivationType.Peikert;

  // ── 25-word mnemonics ──────────────────────────────────────────────────
  const mnemonics = [Buffer.alloc(32, 0), Buffer.alloc(32, 0xff), seedOf('spectra algorand 1'), seedOf('spectra algorand 2')]
    .map((seed) => {
      const mnemonic = algosdk.mnemonicFromSeed(seed);
      const account = algosdk.mnemonicToSecretKey(mnemonic);
      return {
        seed: hex(seed),
        mnemonic,
        public_key: hex(account.sk.slice(32)),
        address: account.addr.toString(),
      };
    });
  const refusedMnemonic = (words, why) => {
    let error;
    try {
      algosdk.seedFromMnemonic(words.join(' '));
    } catch (e) {
      error = e.message;
    }
    if (!error) throw new Error(`algosdk read ${why}`);
    return { mnemonic: words.join(' '), why, algosdk_error: error };
  };
  const words = mnemonics[2].mnemonic.split(' ');
  const english = bip39.wordlists.english;
  const otherWord = (word) => english[(english.indexOf(word) + 1) % 2048];
  // The 24th word carries the key's last 3 bits and 8 bits that must be zero.
  const highBits = english[(english.indexOf(words[23]) | 0x400) % 2048];
  const refused = [
    refusedMnemonic([...words.slice(0, 24), otherWord(words[24])], 'wrong checksum word'),
    refusedMnemonic([...words.slice(0, 5), otherWord(words[5]), ...words.slice(6)], 'a changed key word'),
    refusedMnemonic(words.slice(0, 24), '24 words'),
    refusedMnemonic([...words.slice(0, 23), highBits, words[24]], 'bits past the key set'),
  ];

  // ── BIP-39 phrases along ARC-52 (Peikert) ──────────────────────────────
  const arc52 = async (phrase, passphrase, account) => {
    const seed = await bip39.mnemonicToSeed(phrase, passphrase);
    const root = xhd.fromSeed(Buffer.from(seed));
    const extended = await api.deriveKey(root, [xhd.harden(44), xhd.harden(283), xhd.harden(account), 0, 0], true, PEIKERT);
    const publicKey = await api.keyGen(root, xhd.KeyContext.Address, account, 0, PEIKERT);
    return {
      root,
      row: {
        phrase,
        passphrase,
        account,
        path: `m/44'/283'/${account}'/0/0`,
        extended_key: hex(extended.slice(0, 64)),
        public_key: hex(publicKey),
        address: algosdk.encodeAddress(publicKey),
      },
    };
  };
  const bip39Accounts = [];
  for (const [phrase, passphrase, account] of [
    [PHRASE, '', 0], [PHRASE, '', 1], [PHRASE, '', 2], [PHRASE, 'TREZOR', 0], [README_PHRASE, '', 0], [README_PHRASE, '', 1],
  ]) {
    bip39Accounts.push((await arc52(phrase, passphrase, account)).row);
  }

  // ── Pay transactions ───────────────────────────────────────────────────
  const params = (fee, flatFee, minFee, firstValid) => ({
    fee: BigInt(fee), flatFee, minFee: BigInt(minFee), firstValid: BigInt(firstValid), lastValid: BigInt(firstValid) + 1000n,
    genesisID: GENESIS_ID, genesisHash: algosdk.base64ToBytes(GENESIS_HASH),
  });
  const pay = (sender, receiver, amount, suggestedParams) => algosdk.makePaymentTxnWithSuggestedParamsFromObject({
    sender, receiver, amount: BigInt(amount), suggestedParams,
  });
  const record = (txn, signed, extra) => ({
    ...extra,
    sender: txn.sender.toString(),
    receiver: txn.payment.receiver.toString(),
    amount: txn.payment.amount.toString(),
    fee: txn.fee.toString(),
    first_valid: txn.firstValid.toString(),
    last_valid: txn.lastValid.toString(),
    genesis_id: txn.genesisID,
    genesis_hash: GENESIS_HASH,
    transaction: hex(txn.toByte()),
    bytes_to_sign: hex(txn.bytesToSign()),
    txid: txn.txID(),
    signed: hex(signed),
  });
  const mnemonicAccount = (i) => algosdk.mnemonicToSecretKey(mnemonics[i].mnemonic);
  const transfers = [];
  {
    const from = mnemonicAccount(2);
    const txn = pay(from.addr, mnemonics[3].address, 1234567, params(1000, true, 1000, 67257603));
    transfers.push(record(txn, txn.signTxn(from.sk), { kind: 'mnemonic', mnemonic: mnemonics[2].mnemonic }));
  }
  {
    // A fee priced per byte, above the minimum: algosdk's estimate of the
    // signed size times the per-byte fee.
    const from = mnemonicAccount(3);
    const txn = pay(from.addr, mnemonics[2].address, 9000000000000000000n, params(25, false, 1000, 4000000000));
    transfers.push(record(txn, txn.signTxn(from.sk), { kind: 'mnemonic', mnemonic: mnemonics[3].mnemonic, fee_per_byte: '25', min_fee: '1000' }));
  }
  {
    // Fee per byte zero: the minimum fee.
    const from = mnemonicAccount(2);
    const txn = pay(from.addr, from.addr, 1, params(0, false, 1000, 100));
    transfers.push(record(txn, txn.signTxn(from.sk), { kind: 'mnemonic', mnemonic: mnemonics[2].mnemonic, fee_per_byte: '0', min_fee: '1000' }));
  }
  for (const account of [0, 1]) {
    // An ARC-52 account signs with its extended key, as Pera does.
    const { root, row } = await arc52(PHRASE, '', account);
    const txn = pay(row.address, mnemonics[2].address, 100000 + account, params(1000, true, 1000, 67257603 + account));
    const signature = await api.signAlgoTransaction(root, xhd.KeyContext.Address, account, 0, txn.bytesToSign(), PEIKERT);
    // Signed over "TX" || msgpack, which is what the network verifies.
    const verifies = crypto.verify(null, txn.bytesToSign(), {
      key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(row.public_key, 'hex')]),
      format: 'der', type: 'spki',
    }, signature);
    if (!verifies) throw new Error('ARC-52 signature does not verify');
    transfers.push(record(txn, txn.attachSignature(row.address, signature), {
      kind: 'bip39', phrase: PHRASE, account, path: row.path, extended_key: row.extended_key,
    }));
  }

  const versions = (name) => JSON.parse(fs.readFileSync(`${__dirname}/node_modules/${name}/package.json`, 'utf8')).version;
  const fixture = {
    provenance: `algosdk ${versions('algosdk')}, @algorandfoundation/xhd-wallet-api ${versions('@algorandfoundation/xhd-wallet-api')}, bip39 ${versions('bip39')}`,
    genesis_id: GENESIS_ID,
    genesis_hash: GENESIS_HASH,
    mnemonics,
    refused_mnemonics: refused,
    bip39_accounts: bip39Accounts,
    transfers,
  };
  fs.writeFileSync('core/tests/fixtures/algorand-vectors.json', JSON.stringify(fixture, null, 2) + '\n');
})();
