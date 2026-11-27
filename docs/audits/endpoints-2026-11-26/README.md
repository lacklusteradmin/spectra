# Every built-in endpoint, 2026-11-26

Each endpoint in `core/data/endpoints.toml` checked twice, read-only and
without credentials: by core itself for its network, and by an independent
probe for every capability it claims. The catalog, core's routing and the
decoders that misread live answers were corrected from what was found; this
record is the state after those corrections. Nothing was signed. A broadcast
route was checked by submitting a payload that decodes to no transaction (a
single zero byte in the route's encoding), which a service refuses before it
could relay anything.

| File | What it holds |
|---|---|
| `identity.json` | `spectra endpoints probe --json` over every network: each endpoint asked its network through its API's client (`api::network::verify_network`), or, where its API cannot say, read through that API. |
| `capabilities.json` | `python3 -B docs/audits/endpoints-2026-11-26/probe`: per endpoint, each claimed capability exercised with the requests Spectra's adapter makes, checking the fields it reads, against a sample address, token and transaction derived from the network itself. |
| `baseline-probe.json` | The same command before this change, 11:18–11:24 UTC: the health probe as it was, which asked some APIs their network and others only for an answer. |
| `probe/` | The probe: `common.py` (HTTP with Spectra's user agent, results), one module per API family (`evm.py`, `utxo.py`, `accounts_a.py`, `accounts_b.py`), `__main__.py` (runs every row, two requests per host at a time). |

Run both again from the repository root:

```sh
cargo run -p spectra_cli -- --data-dir "$(mktemp -d)" --json endpoints probe > identity.json
python3 -B docs/audits/endpoints-2026-11-26/probe --out capabilities.json
```

## Results

`identity.json` (13:02 UTC): 295 endpoints on 126 networks. 227 answered as
their network, 44 answered through an API that cannot name its network, and
24 failed: 1rpc's 18 (its quota, below), BlockCypher's four and TronGrid's
node (HTTP 429), and Somnia's explorer, asked about the zero address. Five
networks have no built-in endpoint: Blast and the BCH, DOGE, DCR and DASH
test networks.

`capabilities.json` (13:04 UTC): 235 of the 295 endpoints passed every claim;
of 1,396 claimed capabilities, 1,226 passed, 151 failed and 19 were unchecked.
The 60 endpoints with a failure:

| Cause | Endpoints |
|---|---|
| 1rpc's usage limit, reached by this audit | 18 |
| gRPC (lightwalletd), Litecoin's peer-to-peer protocol and Hedera's consensus nodes: core's probe only | 9 |
| PublicNode: an archive or indexed request needs a personal token | 6 |
| BlockCypher's and TronGrid's keyless rate limits, reached by this audit | 6 |
| A pruned Cosmos node's history search (Injective, BandChain, Kava, Saga) | 4 |
| No sample the probe could use (Starknet, BNB and Optimism testnets' tokens) | 4 |
| A sample artifact: a multisig sender on Babylon, a tip a block behind on Fractal, TON's jetton listing a moment behind | 4 |
| Rate limits misread as missing pagination (NearBlocks) | 2 |
| opBNB's fee history prices a transfer at zero, which core refuses | 2 |
| Monad: "screening unavailable" to the undecodable broadcast | 2 |
| Timeouts (dcrdata's UTXO route, Allora's search) | 2 |
| ZAN's Sui testnet balances | 1 |

Rows narrowed after this run on its evidence: Solana's PublicNode (`staking`,
which needs `getProgramAccounts`) and ZAN's Sui testnet (`balance`,
`token-discovery`).

## Findings the catalog encodes

- **Removed.** Arbitrum's, Base's, Polygon's and Arbitrum Sepolia's
  Blockscout instances answered every client, Spectra's user agent and curl's
  alike, with a Cloudflare challenge (HTTP 403). Routescan answered "chain not
  supported" for Plasma and Blast. Blast's own RPC dropped the TLS handshake
  and PublicNode's answered "unsupported platform", so Blast has no built-in
  endpoint. PolkaChu's Babylon gateway answered HTTP 502 on every request over
  about two hours. xray's Koios preprod route answered 404; Koios' own
  `preprod.koios.rest` replaces it and passed every capability.
- **Claims narrowed.** Solana's PublicNode demands a personal token for
  `getTokenAccountsByOwner`. HyperEVM testnet's dRPC refuses batches of more
  than three calls on its free plan. mempool.emzy.de's testnet4 and signet
  serve no address routes (nginx 404). XRPL Labs' testnet node keeps the
  latest ~1,300 ledgers (altnet keeps all from 7). TON Center's testnet v2 has
  had no archive workers (HTTP 542) through the day. ZAN's Sui testnet errs on
  `balance`. NearBlocks' testnet answers holdings pages without pagination
  metadata.
- **`fee` where none is read.** A TON transfer's fee is the registry's and its
  jetton balances are TON Center v3's; Bitcoin Cash, Bitcoin Gold, Dash,
  Dogecoin, Peercoin, Decred and Kaspa sends pay the registry's fee, and the
  Kaspa and Insight adapters make no fee request. Those rows no longer claim
  `fee`, and those adapters no longer offer it.
- **Kept with limits.** 1rpc's free plan stopped answering this machine on
  every network ("usage limit for your current plan") after the audit's first
  hour of requests; the baseline run at 11:20 UTC passed all 18 of its
  endpoints. BlockCypher and TronGrid answered 429 once the audit's volume
  passed their keyless hourly limits, and dcrdata stopped accepting this
  machine's TLS handshakes after bursts over about three requests a second.
  PublicNode answers an archive-token error, not `null`, for the receipt of a
  hash it does not hold on Base, BNB, Polygon and Arbitrum, which a pending
  poll asks until the transaction is mined; Ethereum's answers `null`. The
  NodeStake Dymension and Saga gateways were down or 3,000 blocks behind for
  about an hour and caught up. Somnia's explorer answers `tokentx` for an
  ordinary address but not for the zero address the health probe asks. Monad's
  nodes answered the undecodable broadcast with "screening unavailable".
  Pruned Cosmos nodes (Injective's and BandChain's own) fail a history search
  whole for an address with transactions older than the blocks they keep.

## Findings core encodes

Live answers core misread, each confirmed by hand against the service and
now a test in core:

- Blockscout's and Routescan's `tokentx` rows carry no `logIndex`, which the
  decoder required: every EVM token history page failed but on Metis, Merlin
  and Somnia.
- proxyd refuses a JSON-RPC batch of more than 10 calls on Optimism's,
  Unichain's, X Layer's and Scroll's own nodes; core sent up to 50.
- Every Horizon answers a JSON submission with HTTP 415 and takes `tx`
  form-encoded: no Stellar transaction could be submitted.
- Version 1 transactions are on Solana mainnet; a node refuses one to a
  request that supports version 0 only.
- Koios 1.4 writes no `valid_contract` in `tx_info`; a transaction's phase-2
  result is per Plutus contract under `_scripts`, none for a plain transfer.
- ICP Rosetta's `/construction/preprocess` answers no `options` for a
  transfer, which core required before asking `/construction/metadata`.
- Asset Hub's `NominationPoolsApi_pending_rewards` returns a bare `Balance`
  (16 zero bytes for an account in no pool), not an `Option`.
- api.kaspa.org leaves a script version of 0 out of a UTXO.
- dcrdata writes an output's `value` as a JSON number.
- zelcore's Bitcoin Cash Blockbook refuses a bare CashAddr payload and names
  every input and output by its prefixed CashAddr, never the legacy form a
  Spectra wallet derives.
- mempool.space instances page an address's confirmed history by 50, where
  Blockstream's Esplora pages by 25.
- Ethereum Classic's and Mordor's CoreGeth nodes write no base fee in a block:
  London is not active, and only legacy transactions are taken.
- BandChain refused a simulation whose `timeout_height` was the next block.

Core also asks every endpoint its network through the client of the API it
speaks (`api::network::verify_network`); before, the health probe passed
Solana, NEAR, XRP Ledger, Stellar, Cardano, Aptos, Tron, TON, ICP Rosetta and
Bitcoin-family endpoints on any answer.

## Not settled here

- Zebra 7.0.0 nodes report Zcash consensus branch `0x77190ad9` at height
  3,613,715, which `zcash_protocol` 0.10.6 does not know: core refuses Zcash
  sends until its crates follow the upgrade (`OPEN-ITEMS.md`).
- `lightwalletd` (gRPC) and `litecoin-p2p` (Litecoin's peer-to-peer protocol)
  are checked by core's own probe only; the HTTP probe marks them unchecked.
  So are the Hedera consensus nodes behind the mirror's address book.
- opBNB's and BNB testnet's fee history sometimes prices a transfer at zero
  (a zero base fee and a zero lowest tip), which core refuses as an invalid
  EIP-1559 fee (`OPEN-ITEMS.md`).
- Starknet and Solana devnet yielded no sample the probe could use; core's
  identity check passed on both.
