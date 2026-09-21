# Endpoint capabilities

`core/data/endpoints.toml` owns the declarations. Each row references a concrete
`chain_id` from `chains.toml`; unknown IDs and obsolete name/title ownership
fields are rejected. Core indexes by network ID and derives settings families
and titles from the registry, including separate mainnet/testnet groups.
CLI catalog rows identify the network with `chainId`. Core validates the vocabulary,
uses distinct capability-mask bits, and exposes the same capabilities to CLI
diagnostics and the localized Endpoints screen. `api` declares the request/response
contract (see [Endpoint APIs](ENDPOINT-APIS.md)); `kind` has been removed.

| Capability | Meaning |
|---|---|
| `balance` | Native-coin balance |
| `history` | Address transaction history for the native coin |
| `token-history` | Fungible-token transfer history for an address; excludes approvals and arbitrary contract activity |
| `token-discovery` | Enumerate an address's token holdings without supplying a token list |
| `token-balance` | Query an address's balance of a specified token |
| `utxo` | Unspent transaction outputs |
| `fee` | Fee data or estimation |
| `broadcast` | Submit a signed transaction |
| `verification` | Transaction verification data |

`history` replaces `native-history`, with no alias for the old name. It still
means native-coin history. A history source does not implicitly
claim token history, and token-balance does not imply token discovery. Discovery
is about address holdings, not Spectra's bundled token catalog. Web links have
no capabilities.

These are declarations about the registered API surface, not proof that every
operation is currently reachable, keyless, unlimited, or wired into every wallet
flow. History can be paginated or provider-limited. The health probe does not
exercise every capability. Unverified token capabilities are left unclaimed;
this directory is not an exhaustive inventory of every provider feature.

## Token declarations and evidence

- EVM RPC: `token-balance`, via ERC-20 `balanceOf` in
  `core/src/fetch/evm.rs`. Ordinary nodes do not enumerate holdings or
  supply address-indexed native/token history. Reading known logs is not an
  address-history index.
- Solana RPC: token balance/discovery via `getTokenAccountsByOwner`; token
  transfers via signature/transaction reads in `core/src/fetch/solana.rs`.
- Sui and Aptos APIs: token balance/discovery via their coin balance and account
  resource queries in the respective `core/src/fetch/` clients. No new
  token-history claim is inferred from their history label.
- NEAR RPC: `token-balance` via `ft_balance_of` in the NEAR client. Address
  history belongs to the separately registered NearBlocks indexer.
- Tron account indexers: token holdings and transfer history; Tron node API
  rows: token balance via constant contract calls. See the Tron client and the
  [TRON API reference](https://developers.tron.network/docs/api), which separates
  current-state node calls from indexed account and TRC-20 history queries.
- TON v3: jetton balance/discovery and token history. v2 retains native history;
  it is not labeled as a jetton indexer. See the TON client and
  [TON Center v3 reference](https://docs.ton.org/api/v3/overview).
- Blockscout: native history (`txlist`), token transfers (`tokentx`), token
  balances (`tokenbalance`), and discovery (`tokenlist`), per its
  [account API](https://docs.blockscout.com/devs/apis/rpc/account).
- Ethplorer: address transactions, token transfer history, and holdings/balances
  via its [API](https://github.com/EverexIO/Ethplorer/wiki/ethplorer-api).

Inspect the declarations offline (`--catalog` performs no health probes):

```sh
spectra --json endpoints --catalog --chain Ethereum
spectra --json endpoints --catalog --chain Bitcoin
spectra --json endpoints --catalog --chain Solana
spectra --json endpoints --catalog --chain TON
```

The CLI acceptance suite checks the native/token split, and core tests verify
that capability masks select the appropriate API, including TON v2 versus v3.
The iOS suite checks that the core-provided new labels localize in the endpoint
summary.
