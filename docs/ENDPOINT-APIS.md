# Endpoint APIs

`api` identifies a wire contract, not an operator. Blockstream and Mempool both
speak `esplora`; an Ethereum node speaks `evm-json-rpc`. `capabilities` states which operations a row provides. There is no `kind` field.
Browser links omit `api` and declare no capabilities; `explorer_label` enables
a transaction link button.
API records cannot carry browser-link fields. Unknown APIs are rejected.

```toml
[[endpoints]]
id = "bitcoin.mainnet.blockstream"
chain_id = "bitcoin"
api = "esplora"
endpoint = "https://blockstream.info/api"
capabilities = ["balance", "history", "utxo", "fee", "broadcast", "verification"]
```

The full vocabulary is `EndpointApi` in `core/src/endpoint_api.rs`. Versions
are distinct where their contracts differ (`toncenter-v2`/`toncenter-v3`,
`trongrid-v1`). REST contracts include `esplora`, `blockbook`, `blockchair`,
`blockcypher`, `whatsonchain`, `koios`, `horizon`, `aptos-rest`, `icp-rosetta`
and provider-specific indexers. JSON-RPC contracts distinguish EVM, Solana,
Sui, NEAR, XRPL, Substrate, Tron and Monero wallet RPC.

Core's `Chain::endpoint_api` declares the API implemented by each current
service slot. `catalog_endpoints` selects matching rows, preserving order;
primary lists also require balance capability to exclude operation-specific
URL prefixes. Bitcoin's Esplora list and TON's secondary v3 list no longer use
hard-coded endpoint IDs. `spectra --json endpoints --catalog` exposes both the
full directory and the built-in `configured` lists; custom user overrides are
not part of that built-in projection.

Balance, history, UTXO status and rebroadcast select the client by the active
endpoint API. Unknown custom URLs use the registry default. Blockbook has one
network-aware client instead of five chain aliases; its chain-specific signers
remain in send modules and check their network before requests. REST readers
share path/GET/fallback handling through `HttpClient::get_path`.

A declared API does not create its implementation. Domain methods and decoding
still live in clients. Shared JSON-RPC envelope/error/fallback logic lives in
`core/src/fetch/json_rpc.rs`; XRPL has a different request envelope and nested
error shape, so it is handled explicitly. EVM batch requests remain separate.

In particular, Litecoin and Bitcoin Cash currently use Blockbook clients but
have no matching catalog bases. Monero's light-wallet servers do not implement
the current wallet-RPC client. Their built-in primary lists are therefore empty;
operations fail rather than sending the wrong protocol. An unknown custom URL
must speak the selected slot's API. A known incompatible catalog URL is rejected
before configuration changes. API tags do not attest reachability or trust.

Relevant protocol distinctions: [Tron separates HTTP and JSON-RPC APIs](https://developers.tron.network/docs/api);
[Monero wallet RPC](https://docs.getmonero.org/rpc-library/wallet-rpc/) is not the
[light-wallet REST contract](https://github.com/monero-project/meta/blob/master/api/lightwallet_rest.md).
