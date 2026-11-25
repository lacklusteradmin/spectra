# Sui GraphQL evidence — 2026-11-24

Spectra read Sui through JSON-RPC until this date, from five mainnet
endpoints and one testnet endpoint. The readings below were taken on
2026-11-24 (UTC) without API keys. Every request was a read; no transaction
was executed on either network.

## The JSON-RPC endpoints

Each endpoint was asked `sui_getChainIdentifier`,
`sui_getLatestCheckpointSequenceNumber`, `suix_getBalance`,
`suix_getAllBalances`, `suix_getCoins`, `suix_getCoinMetadata`,
`suix_getReferenceGasPrice`, `suix_queryTransactionBlocks`,
`suix_getLatestSuiSystemState`, `suix_getStakes` and
`sui_dryRunTransactionBlock`: the reads behind verification, balance, token
balance and discovery, fee, history and token history, staking and
simulation. `sui_executeTransactionBlock`, the broadcast, was not sent.

| Endpoint | Network | Every method answered |
|---|---|---|
| `https://sui-rpc.publicnode.com` | mainnet | HTTP 200, error -32000 "Sui JSON-RPC endpoint has been deprecated. Please use the gRPC endpoint instead" |
| `https://sui-mainnet-endpoint.blockvision.org` | mainnet | HTTP 404, error -32601 "Sui JSON-RPC has been deprecated. Please migrate to gRPC", or HTTP 429 |
| `https://sui.blockpi.network/v1/rpc/public` | mainnet | HTTP 404, empty body |
| `https://rpc-mainnet.suiscan.xyz` | mainnet | HTTP 404, empty body |
| `https://1rpc.io/sui` | mainnet | HTTP 404, error -32603 "Internal error" |
| `https://sui-testnet-rpc.publicnode.com` | testnet | HTTP 200, error -32000, as publicnode's mainnet |

Mysten's own fullnodes (`fullnode.mainnet.sui.io`, `fullnode.testnet.sui.io`)
answer every method with -32601 "JSON-RPC on public fullnodes has been
deprecated. Please migrate to gRPC or GraphQL endpoints". Every Sui
capability the catalog claimed (balance, history, token history, token
balance, token discovery, fee, broadcast, verification and staking) failed on
both networks.

## GraphQL or gRPC

Sui's gRPC API (`sui.rpc.v2`) serves objects, balances, coin metadata,
epochs, simulation and execution, but lists no transactions by address, so
history and token history need GraphQL either way. Sui GraphQL serves all
of them in one API over HTTP and JSON, which core's HTTP transport and the
loopback fixtures already speak.

Keyless GraphQL endpoints that answered (`chainIdentifier`, `checkpoint`,
`epoch`):

| Endpoint | Network | Chain identifier |
|---|---|---|
| `https://graphql.mainnet.sui.io/graphql` | mainnet | `4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S` |
| `https://api.zan.top/public/sui-mainnet/graphql` | mainnet | `4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S` |
| `https://graphql.testnet.sui.io/graphql` | testnet | `69WiPg3DAQiwdxfncX6wYQ2siKwAe6L9BZthQea3JNMD` |
| `https://api.zan.top/public/sui-testnet/graphql` | testnet | `69WiPg3DAQiwdxfncX6wYQ2siKwAe6L9BZthQea3JNMD` |

The chain identifier is the digest of checkpoint zero, which
`checkpoint(sequenceNumber: 0)` confirmed on all four. Ankr's
`rpc.ankr.com/sui_graphql` refused without a key; BlockPi's and publicnode's
hosts served no GraphQL path tried. Sui's provider list
(docs.sui.io, "RPC and Data Providers") names the others as keyed services.

Mysten's mainnet `serviceConfig`: `maxQueryNodes` 300, `maxQueryDepth` 20,
`maxPageSize` 50 for `Address.transactions`, `Address.objects`,
`Address.balances`, `ValidatorSet.activeValidators` and
`TransactionEffects.balanceChanges` (default 20 for the last),
`queryTimeoutMs` 40,000, `mutationTimeoutMs` 74,000. Epoch 1322's reference
gas price was 100 MIST on mainnet; testnet's epoch 1294, 1000.

## Shapes read

- A `Balance` has `totalBalance`, `coinBalance` and `addressBalance`, the
  first the sum of the other two. Mainnet addresses hold funds in the address
  balance: `0x7ab9a6a7…71cec` held 365,791,576.507 SUI, of which
  4,888,007 SUI in coin objects; `0xac5bceec…3c33c` held 287,396 units of
  USDC, 2,072 in coin objects.
- `UInt53` fields (versions, sequence numbers, gas costs, epoch ids) are JSON
  numbers; `BigInt` fields (balances, amounts, the gas price) and Move `u64`
  fields in object contents (`principal`, `stake_activation_epoch`,
  `commission_rate`, `sui_balance`) are strings.
- Addresses and types come in their 64-digit form
  (`0x0000…0002::sui::SUI`); the `type` filter of `Address.objects` takes
  the short form, and `0x2::coin::Coin` matches every `Coin<T>`.
- A `TransactionEffects.timestamp` is ISO 8601 with milliseconds.
- `simulateTransaction` takes `{"bcs": {"value": <base64 TransactionData>}}`;
  an input object at a stale version is a GraphQL error (`BAD_USER_INPUT`),
  an execution failure a `FAILURE` status with `executionError.message`.
- `transactionEffects(digest:)` of an unknown digest is `null`.
- The active validator set had 123 validators on mainnet, three pages.
  `StakedSui` contents carry `pool_id`, `stake_activation_epoch` and
  `principal`, and no reward estimate.

## Reads through the CLI

Against the built-in endpoints only, in a scratch data directory, with the
binary built from this change:

| Command | Result |
|---|---|
| `endpoints probe --chain sui`, `--chain sui-testnet` | all four reachable; "Sui network verified at checkpoint 348930599" |
| `balance` of `0xac5bceec…3c33c` | 0.665407099 SUI |
| `history` of the same | SUI and token transfers, newest first, token amounts in their own decimals (CETUS, HAEDAL, USDC) |
| `token discover` of the same | its token types with balances |
| `wallet objects` of the same | coin objects per type (1,920 USDC objects) |
| `staking validators --chain sui` | 123 validators |
| `staking positions` of `0xb52b5333…408c3` | one active `StakedSui` position, its validator active, reward unknown |
| `balance`, `history`, `token discover` of testnet `0x35bc3c3d…06d7a` | 1.44101624 tSUI; token transfers |
| `send build` of 0.1 SUI from `0xac5bceec…3c33c` (watched) | a prepared transfer at gas price 100 and budget 0.01 SUI, never signed; its bytes simulated `SUCCESS` on both mainnet endpoints (computation 100,000, storage 1,976,000, rebate 987,901 MIST) |
| `send build` with only `api.zan.top/sui-testnet/graphql` configured for mainnet | refused: "Sui endpoint is on the wrong network" |
