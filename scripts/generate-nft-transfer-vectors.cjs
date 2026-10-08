// ERC-721 and ERC-1155 calls as ethers.js encodes them: the transfers a
// wallet signs, the ownership and balance reads before them, and ERC-165's
// interface checks, with token ids from one to the largest uint256. Then two
// transfers signed as EIP-1559 transactions by the key 0x01…01, with the
// fields the CLI acceptance node answers: nonce 3 then 4, a base fee of
// 1 gwei and a 2 gwei tip (max fee 2 × base + tip), and an estimate of
// 90,000 gas with Ethereum's 20% margin.
// npm install --prefix /tmp/spectra-nft-vectors --ignore-scripts ethers@6.17.0
// NODE_PATH=/tmp/spectra-nft-vectors/node_modules node scripts/generate-nft-transfer-vectors.cjs
const fs = require('node:fs');
const { Interface, Transaction, Wallet } = require('ethers');

const erc721 = new Interface([
  'function safeTransferFrom(address from, address to, uint256 tokenId)',
  'function ownerOf(uint256 tokenId) view returns (address)',
]);
const erc1155 = new Interface([
  'function safeTransferFrom(address from, address to, uint256 id, uint256 amount, bytes data)',
  'function balanceOf(address account, uint256 id) view returns (uint256)',
]);
const erc165 = new Interface(['function supportsInterface(bytes4 interfaceId) view returns (bool)']);

const from = '0x1111111111111111111111111111111111111111';
const to = '0x2222222222222222222222222222222222222222';
const ids = ['0', '1', '34454361969670104802583458346517400542074712368450903800518897101070583190115',
  ((1n << 256n) - 1n).toString()];

const signer = new Wallet('0x' + '01'.repeat(32));
const recipient = '0x2222222222222222222222222222222222222222';
const collections = { erc721: '0x5555555555555555555555555555555555555555', erc1155: '0x6666666666666666666666666666666666666666' };
const signed = (nonce, to, data) => {
  const fields = { type: 2, chainId: 1, nonce, maxPriorityFeePerGas: 2_000_000_000n, maxFeePerGas: 4_000_000_000n,
    gasLimit: 108_000n, to, value: 0n, data, accessList: [] };
  return signer.signTransaction(fields).then(raw => ({ nonce, to, data, raw, hash: Transaction.from(raw).hash }));
};

Promise.all([
  signed(3, collections.erc721, erc721.encodeFunctionData('safeTransferFrom', [signer.address, recipient, 1234])),
  signed(4, collections.erc1155, erc1155.encodeFunctionData('safeTransferFrom', [signer.address, recipient, 7, 3, '0x'])),
]).then(([signedErc721, signedErc1155]) => fs.writeFileSync('core/tests/fixtures/nft-transfer-vectors.json', JSON.stringify({
  provenance: 'ethers 6.17.0',
  from,
  to,
  erc721: ids.map(id => ({
    token_id: id,
    transfer: erc721.encodeFunctionData('safeTransferFrom', [from, to, id]),
    owner_of: erc721.encodeFunctionData('ownerOf', [id]),
  })),
  erc1155: ids.map((id, n) => ({
    token_id: id,
    quantity: ['1', '7', '1000000000000000000000', ((1n << 128n) - 1n).toString()][n],
    transfer: erc1155.encodeFunctionData('safeTransferFrom', [from, to, id, ['1', '7', '1000000000000000000000', ((1n << 128n) - 1n).toString()][n], '0x']),
    balance_of: erc1155.encodeFunctionData('balanceOf', [from, id]),
  })),
  supports: {
    erc721: erc165.encodeFunctionData('supportsInterface', ['0x80ac58cd']),
    erc1155: erc165.encodeFunctionData('supportsInterface', ['0xd9b67a26']),
  },
  signed: {
    signer: signer.address.toLowerCase(),
    recipient,
    erc721: signedErc721,
    erc1155: signedErc1155,
  },
}, null, 2) + '\n'));
