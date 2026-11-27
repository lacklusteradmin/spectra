// Legacy (EIP-155 type 0) transactions as ethers.js signs them, for the
// networks whose nodes take no typed envelope: a Kava EVM transfer, an ERC-20
// transfer on Merlin, a Metis transfer at a gas price past 2^32 wei and an
// Ethereum Classic transfer, each signed by the key 0x01…01. `unsigned` is the EIP-155 signing payload, `raw`
// the signed transaction.
// node tests/vectors/generate-evm-legacy-vectors.cjs
const fs = require('node:fs');
const { Interface, Transaction, Wallet } = require('ethers');

const signer = new Wallet('0x' + '01'.repeat(32));
const recipient = '0x2222222222222222222222222222222222222222';
const erc20 = new Interface(['function transfer(address to, uint256 amount)']);

const cases = [
  { name: 'kava-evm-native', chainId: 2222, nonce: 7, gasPrice: 1_000_000_000n, gasLimit: 21_000n,
    to: recipient, value: 1_500_000_000_000_000_000n, data: '0x' },
  { name: 'merlin-erc20', chainId: 4200, nonce: 0, gasPrice: 6_000_000n, gasLimit: 65_000n,
    to: '0x5c46bff4b38dc1eae09c5bac65872a1d8bc87378', value: 0n,
    data: erc20.encodeFunctionData('transfer', [recipient, 10n ** 18n]) },
  { name: 'metis-native', chainId: 1088, nonce: 300, gasPrice: 15_772_000_922n, gasLimit: 21_000n,
    to: recipient, value: 1n, data: '0x' },
  { name: 'ethereum-classic-native', chainId: 61, nonce: 12, gasPrice: 1_000_000_000n, gasLimit: 21_000n,
    to: recipient, value: 250_000_000_000_000_000n, data: '0x' },
];

Promise.all(cases.map(async (c) => {
  const fields = { type: 0, chainId: c.chainId, nonce: c.nonce, gasPrice: c.gasPrice, gasLimit: c.gasLimit,
    to: c.to, value: c.value, data: c.data };
  const raw = await signer.signTransaction(fields);
  return {
    name: c.name,
    chain_id: c.chainId,
    nonce: c.nonce,
    gas_price: c.gasPrice.toString(),
    gas_limit: Number(c.gasLimit),
    to: c.to,
    value: c.value.toString(),
    data: c.data,
    unsigned: Transaction.from(fields).unsignedSerialized,
    raw,
    hash: Transaction.from(raw).hash,
  };
})).then((transactions) => fs.writeFileSync('core/tests/fixtures/evm-legacy-transactions.json', JSON.stringify({
  provenance: 'ethers 6.17.0',
  key: '0x' + '01'.repeat(32),
  signer: signer.address.toLowerCase(),
  transactions,
}, null, 2) + '\n'));
