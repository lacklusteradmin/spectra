# Sixteen EVM mainnets, 2026-11-24

Read-only checks behind Kaia, XDC Network, Robinhood Chain, Kava EVM, Chiliz
Chain, Stable, Arc, Manta Pacific, Metis, Flare, Katana, MegaETH, Merlin Chain,
peaq, Somnia and 0G in the registry, and behind the hex block count Spectra now
sends with `eth_feeHistory`. Nothing was signed or broadcast.

| File | What it holds |
|---|---|
| `network-probes.json` | Per catalogued RPC: `eth_chainId`; `eth_feeHistory` with a hex block count, and whether the node accepts a JSON-number one; `eth_gasPrice` and `eth_maxPriorityFeePerGas`; transaction types in the last 12 blocks; a type-2 receipt's fields; a plain transfer's `eth_estimateGas`; the OP Stack gas price oracle's code and its `getL1FeeUpperBound` and `getOperatorFee` answers, and on Metis its `getL1Fee(bytes)` answer. Per catalogued indexer: keyless `txlist`, `tokentx` and `tokenlist`. Per token: code size, `decimals()` and `symbol()` at the latest block, including each coin's ERC-20 face. |
| `fee-history-hex-count.json` | Every EVM endpoint already in the catalog with the `fee` capability, asked `eth_feeHistory` with a hex block count. |
| `token-sources.json` | For each new token deployment and each ERC-20 face, the issuer's or network's own page, fetched, and whether it names that exact contract. |
| `native-face-balances.json` | One address each on Stable, Arc and Metis: `eth_getBalance` and the ERC-20 face's `balanceOf` at the same block. |

## Findings the registry encodes

- **Hex block count.** XDC, Somnia and Metis nodes refuse a JSON-number block
  count. Every other catalogued EVM endpoint answered a hex one; the two 1rpc
  endpoints did so on a retry after rate limiting. The two Blast endpoints
  answered neither form: one returned 403 and one dropped the TLS connection.
- **Legacy-only networks.** Kava EVM's chain configuration sets no London
  block, Merlin's zkEVM decodes only legacy RLP, and Metis's l2geth has no
  typed transactions and no `eth_feeHistory`. Their recent blocks hold only
  type-0 transactions.
- **Tip floors.** Chiliz documents a 1 gwei minimum tip, and its blocks are
  mostly system transactions with a zero gas price. 0G's fee history and
  `eth_gasPrice` both put the tip at 4 gwei over a 7 wei base fee.
- **Rollup fees.** Manta Pacific, Katana and MegaETH have Fjord's
  `getL1FeeUpperBound` and an operator fee (non-zero on Katana). Metis's
  OVM-style oracle answers Bedrock's `getL1Fee(bytes)`. Robinhood Chain is an
  Arbitrum Orbit chain with no OP oracle.
- **The coin's ERC-20 face.** Stable's USDT0 at
  `0x779ded0c9e1022225f8e0630b35a9b54be713736` and Arc's USDC at
  `0x3600000000000000000000000000000000000000` are the gas coin at 6 places:
  at one block, `eth_getBalance` divided by 10^12 equals `balanceOf`. Metis's
  `0xdead…0000` returns the native balance unchanged. peaq documents its
  precompile `0x…0809` as the native balance; it has no code to read.
- **Indexers.** Keyless Blockscout indexers answer on Chiliz, Manta Pacific,
  Metis, Flare, MegaETH, Merlin and Somnia. The other nine networks have none:
  their indexers require a key or payment, refuse non-browser clients with a
  Cloudflare challenge (Robinhood Chain, Arc), or speak a variant of the
  Etherscan API (0G's `chainscan.0g.ai/open/api`).
