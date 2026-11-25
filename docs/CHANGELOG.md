# Changelog

## Beta Commit 261

- Docs: `AGENTS.md` "CHANGELOG" (many changes, many sub-items; how a
  behaviour-keeping refactor was shown goes under `Measured:`);
  `CHANGELOG.md` (every section checked against its commit: `+X −Y lines` in
  `Total:` counts the .rs files' lines in every section, wrong names and
  numbers corrected, missing changes added, packed entries split, undated
  `Measured:` lines dated, docs-only commits' `Total:` "No code changed").
- Total: No code changed.

## Beta Commit 260

- **Sui is read and written through Sui GraphQL** (`api/sui_graphql.rs`,
  `EndpointApi::SuiGraphql`, `sui-graphql`; before: `api/sui_json_rpc.rs` and
  `SuiJsonRpc`, whose every built-in endpoint now refuses JSON-RPC,
  [audit](audits/sui-graphql-2026-11-24.md))
  - The built-in endpoints are Mysten's and ZAN's GraphQL on mainnet and
    testnet, with the capabilities the JSON-RPC rows claimed
    (`core/data/endpoints.toml`; before: five mainnet JSON-RPC rows and one
    testnet row).
  - An endpoint is the network whose checkpoint-zero digest it answers as its
    chain identifier (`Chain::sui_genesis_checkpoint`; before:
    `sui_network_identity`'s four-byte identifier and checkpoint zero's
    digest, two requests). The health probe checks it too (before: the latest
    checkpoint alone).
  - A balance, SUI's or a token's, counts the coin objects and the address
    balance together; a merge's trial budget is capped by the coin objects'
    part (`SuiBalance::coin_mist`).
  - History is one listing of the transactions that affected the address
    (before: two, as sender and as recipient, under a combined cursor).
  - History times are to the second (before: milliseconds).
  - An object listing (coins, stakes) is refused when it names an object
    another address owns (before: coin listings were taken as given).
  - Coin objects are counted per type from every `Coin` object the address
    owns (before: `suix_getAllBalances`' object counts); a listing reads at
    most 100 pages of 50.
  - Staking positions are the owner's `StakedSui` objects, active from their
    activation epoch, with the validator found by pool in the active set
    (before: `suix_getStakes`' groups, each object fetched again).
  - A staking position's reward is unknown (before: `estimatedReward`).
  - A position whose validator left the active set is `Inactive` and named by
    its pool's id.
  - A digest without effects is pending (before: an unknown digest failed the
    status read).
  - Execution returns once the transaction is final (before:
    `WaitForLocalExecution`).
  - Merges and stakes are priced and checked by one simulation
    (`SuiClient::simulate`; before: `dry_run_gas` and `simulate_staking`).
- Measured: 2026-11-24, keyless, read-only: every Sui JSON-RPC method refused
  on all six built-in endpoints; Mysten's and ZAN's GraphQL answered every read
  on mainnet and testnet; a SUI transfer the CLI built simulated `SUCCESS` on
  both mainnet endpoints ([audit](audits/sui-graphql-2026-11-24.md)).
- Tests: 6 new
  - Sui GraphQL 6: a transfer misread when its balance changes span pages; a
    probe passing another network's endpoint; data accepted beside GraphQL
    errors; 64-digit types not matching the catalog's; a listing paging
    forever on a repeated cursor; coin objects miscounted across pages.
- Docs: `CHAIN-SUPPORT.md` ("Wallets and transfers": Sui GraphQL and the
  address balance; "History and confirmation": Sui history; "Staking positions
  and execution": the Sui row), `OPEN-ITEMS.md` ("Asset and privacy
  protocols": spending the Sui address balance; "Staking coverage": Sui
  rewards and departed validators), `tests/README.md` (the harness's Sui
  GraphQL answers), `audits/sui-graphql-2026-11-24.md` (new).
- Total: 31 .rs files changed (3 new, 1 deleted), +1881 −1360 lines; data and
  config files: `core/data/endpoints.toml`, `tests/acceptance/harness.py`,
  `tests/acceptance/staking.py`, `tests/acceptance/wallet_operations.py`;
  dependencies unchanged; `cargo test` passing 1682 → 1688.

## Beta Commit 259

- **The token catalog lists 44 more tokens** (`core/data/tokens.toml`,
  `crypto-wiki.toml`; `icons/crypto/`)
  - QNT, JST, SUN, RENDER, NEXO, FET, GNO, GRT, COMP, TWT, BAT, AXS, MANA, SAND,
    APE, SPX, GMX, 1INCH, IMX, SNX, CRVUSD, SAFE, GALA, HNT, CETUS, SNEK, CBETH,
    WBETH, WEETH, EIGEN, VIRTUAL, FLOKI, JTO, W, NOT, WAL, DEEP, BONK, WIF,
    PENGU, TRUMP, DOGS, ASTER and ORCA: 122 deployments on 25 networks, each
    with a wiki row. SNEK is the catalog's first Cardano token.
  - 37 icons: 27 from web3icons (MIT), and 10 from each project's own mark
    (FET draws the ASI Alliance's). BONK, WIF, PENGU, TRUMP and DOGS publish
    no vector mark, and Aster's and Orca's terms forbid altering theirs, so
    these seven name no artwork and the apps draw their letter badge.
- **Catalog tokens gain 130 deployments on more networks**
  (`core/data/tokens.toml`)
  - USDC on Stellar, the XRP Ledger and Starknet; EURC on Stellar and World
    Chain; RLUSD on the XRP Ledger and Base; PYUSD on Stellar, Polygon, X Layer
    and Stable. They are the catalog's first Stellar and XRP Ledger assets.
  - WBTC, WETH, wstETH, rETH, LINK, UNI, AAVE, ZRO, PENDLE, MORPHO, CAKE and
    LBTC on the networks their issuers or the networks' canonical bridges list
    (WBTC on 18, WETH on 13).
  - USDe, sUSDe, ENA, USDG, USD1, GHO, USDT0, XAUt0, USAT and PAXG on the
    networks their issuers list.
  - OKB, INJ, CHZ, METIS and STRK as their own ERC-20s on Ethereum.
- **Three CoinPaprika ids are corrected** (`core/data/tokens.toml`)
  - Gram is `toncoin-the-open-network` (before: `ton-tontoken`, an unrelated
    Ethereum token).
  - Dymension is `dym-dymension-iou` (before: `dym-dymension`, which
    CoinPaprika does not list).
  - USD Blast has none (before: `usdb-usdb`, inactive on CoinPaprika).
- **The balance-refresh concurrency test runs on Stellar testnet**
  (`service/tests/balance_refresh.rs`; before: Stellar, where the catalog's
  assets now add a token read per wallet).
- Measured: 2026-11-24, contract deployments read on chain at a fixed block:
  existing 278 of 283 match (5 on Blast unreachable), added 252 of 252; all 40
  differences from CoinGecko resolve in the catalog's favour
  ([record](audits/token-catalog-2026-11-24/README.md)).
- Docs: `CHAIN-SUPPORT.md` ("Fungible assets": the Starknet row); new audit
  record `docs/audits/token-catalog-2026-11-24/`.
- Total: 1 .rs file changed, +12 −3 lines; data files `core/data/tokens.toml`,
  `core/data/crypto-wiki.toml`, `icons/crypto/` (37 new); dependencies
  unchanged; `cargo test` passing 1682 → 1682.

## Beta Commit 258

- **A directory module's body is its `mod.rs`** (`api/litecoin_p2p/mod.rs`,
  `api/substrate_json_rpc/mod.rs`, `api/substrate_json_rpc/metadata/mod.rs`;
  before: `litecoin_p2p.rs`, `substrate_json_rpc.rs` and `metadata.rs` beside
  their directories, while the domains and `send/litecoin_mweb/` used
  `mod.rs`).
- **Four out-of-line test modules follow the layout the others use**
  (`derivation/tests/mod.rs`, `api/tests/`, `src/tests/mod.rs`)
  - `derivation`'s own tests are `derivation/tests/mod.rs` (before:
    `derivation/tests.rs` beside `derivation/tests/`).
  - `substrate_json_rpc`'s and its `pools` module's are
    `api/tests/substrate_json_rpc.rs` and
    `api/tests/substrate_json_rpc_pools.rs` (before:
    `substrate_json_rpc/tests.rs` and `substrate_json_rpc/pools/tests.rs`).
  - The crate root's are `src/tests/mod.rs`, declared as `mod tests` (before:
    `src/tests/app_boundary.rs`, declared as `app_boundary_tests` through
    `#[path]`).
- **The send stages and multisig are directories of `service/`**
  (`service/send_stage/`, `service/multisig/`; before: 35 files directly in
  `service/`)
  - `send_stages.rs` is `send_stage/mod.rs`, `send_stage_protocols.rs` is
    `send_stage/protocols.rs`, and each `send_stage_<protocol>.rs` and
    `send_near.rs` is a child `send_stage/<protocol>.rs`; `multisig.rs` is
    `multisig/mod.rs` and each `multisig_<scheme>.rs` is
    `multisig/<scheme>.rs`.
  - 155 `pub(super)` items in the children reach only their group (before:
    all of `service`); the 26 the rest of `service` calls are
    `pub(in crate::service)`.
  - Nine `pub(super)` helpers of the two `mod.rs` files only their children
    call are private.
  - The tests of `send_stages` and `send_near` are
    `service/tests/send_stage.rs` and `service/tests/send_stage_near.rs`.
- **`send/message/` holds one module per message-signing scheme**
  (`send/message/mod.rs`, `stellar.rs`, `cardano.rs`, `kaspa.rs`, `monero.rs`;
  before: `send/message.rs` and the four as inline modules of
  `message_schemes.rs`, mounted through `#[path]`). Their functions are
  `pub(super)` (before: `pub(in crate::send)`).
- **The UniFFI bindgen binary is `spectra-uniffi-bindgen`**
  (`tools/uniffi-bindgen/Cargo.toml`; before: `spectra_uniffi_bindgen`, the
  one bin name with underscores).
- **The source scans match a test module whose `#[path]` leaves its directory
  with `..`** (`scripts/source_scan.py`; before: such a path did not equal the
  listed file, and only a `tests/` directory in it marked the file as test
  code).
- **`README.md` "The CLI" says every operation is a command that `--help`
  lists, with `--json` for scripts** (before: that `cargo run -p
  spectra_cli` with no command opened the windowed interface, which Beta
  Commit 254 deleted).
- Docs: `README.md` ("The CLI"), `ARCHITECTURE.md` ("Signing and service
  modules": the send stages; "Core module boundaries": directory groups and
  their visibility, test placement, `api/`, multisig), `AGENTS.md` ("Docs": a
  doc's rule is no reason on its own to keep code; "Architecture": one module
  per `EndpointApi`).
- Total: 55 .rs files changed (4 new, 1 deleted, 45 moved), +585 −642 lines;
  config files: `scripts/source_scan.py`, `scripts/test-source-scan.py`,
  `tools/uniffi-bindgen/Cargo.toml`; dependencies unchanged; `cargo test`
  passing 1682 → 1682.

## Beta Commit 257

- **Thirteen Cosmos SDK mainnets join the registry** (new
  `registry/cosmos.rs`, `derivation/cosmos.rs`, `send/cosmos.rs`,
  `api/cosmos_rest.rs`, `service/send_stage_cosmos.rs`;
  `core/data/chains.toml`, `tokens.toml`, `endpoints.toml`, `explorers.toml`,
  `chain-wiki.toml`, `crypto-wiki.toml`; `icons/crypto/`)
  - Cosmos Hub, Celestia, dYdX Chain, Terra, Kava, Injective, Akash, Babylon
    Genesis, Osmosis, Dymension, Saga, Allora and BandChain: receive, balance,
    history, a native `MsgSend` with a memo (build, sign, broadcast, status),
    raw-key and watch-only import.
  - Keys along `m/44'/{coin}'/{account}'/0/0` (coin type 118; Terra 330, Kava
    459, BandChain 494, Injective and Dymension 60); Injective and Dymension
    take Ethermint `eth_secp256k1` addresses and sign over Keccak-256.
  - The fee is a simulation's gas with two fifths more at chain-registry's
    `low` gas price, or a fee market's live price and a fifth more where
    higher (Cosmos Hub, Allora, Osmosis, Injective); a transaction carries a
    `timeout_height` about ten minutes ahead and expires there.
  - Balances count vesting coins; a send and its quote use the spendable
    balance.
  - Refused: a token send, a sender with no account on the chain, a zero
    amount, a memo over the network's limit, an address of another prefix,
    case or length, and at signing a changed sequence or a passed timeout
    height.
  - New tokens ATOM, TIA, DYDX, LUNA, INJ, AKT, BABY, OSMO, DYM, SAGA, ALLO
    and BAND; KAVA gains its Cosmos-side deployment (6 decimals).
  - 35 keyless `cosmos-rest` endpoints from two to four operators per network,
    with Mintscan, ezstaking.app (Dymension) and Allora explorers.
  - Twelve icons: seven from web3icons (MIT), and Babylon's, Dymension's,
    Saga's, Allora's and BandChain's own marks from cosmos/chain-registry.
- **Kusama and Astar join the Substrate networks** (new
  `registry/substrate.rs`; `derivation/polkadot.rs`,
  `api/substrate_json_rpc/metadata.rs`, `send/polkadot.rs`, `send/message.rs`;
  `core/data/`; `icons/crypto/`)
  - Kusama on its Asset Hub, and Astar: receive, balance, a native
    `Balances.transfer_keep_alive` (build, sign, broadcast, finality), sr25519
    message signing, raw-key and watch-only import; no address history,
    staking, multisig or `Assets` tokens.
  - A network's SS58 prefix, genesis, balance width, fee extension and inert
    extensions live on `SubstrateNetwork` (before: per-chain matches and
    `Chain::Bittensor` checks); metadata naming another fee extension than the
    network's is refused.
  - An address must carry its own network's SS58 prefix (before: Polkadot and
    Westend accepted any).
  - Bittensor's derivation moved into `derivation/polkadot.rs`
    (`derivation/bittensor.rs` removed) and its unread `static_fee_units`
    removed; its health check reads genesis and metadata as Polkadot's does.
  - New tokens KSM (12 decimals) and ASTR (18); five `substrate-json-rpc`
    endpoints (three Kusama Asset Hub, two Astar) and Subscan explorers.
- **Qtum, Fractal Bitcoin, eCash and DigiByte join the account UTXO
  networks** (`registry/utxo.rs`, `registry/keys.rs`,
  `derivation/utxo_address.rs`, `send/account_utxo.rs`,
  `service/send_stage_utxo.rs`, `api/utxo.rs`, new `api/chronik.rs`,
  `api/unisat_open_api.rs`; `core/data/`; `icons/crypto/`)
  - Each: gap-scanned receive and change, balance and history over the
    account, sends through its indexers, phrase, WIF, hex, xpub and
    watch-only import.
  - Profiles: Qtum legacy, native and nested SegWit at coin type 88; Fractal
    Bitcoin's four at coin type 0; DigiByte's four at coin type 20, which also
    reads its old `3…` P2SH addresses; eCash coin type 1899 and the older 899.
  - eCash reads and pays only CashAddr addresses (the `ecash:` prefix
    optional), signs with fork id 0 and reads Chronik (protobuf); Fractal
    Bitcoin reads UniSat's Open API and an Esplora.
  - A fee rate never goes below the network's minimum, and a reviewed rate
    under it is refused: Qtum Core's 400 sat/vB, which a Qtum send pays (its
    indexers estimate none), DigiByte Core's 100 and Fractal Bitcoin's 1, and
    1,000 satoshis per started kB on eCash; dust 72,800 (Qtum), 5,460
    (DigiByte) and 546 satoshis.
  - Qtum staking rewards wait 2,000 confirmations and eCash coinbase outputs
    100; Qtum's P2PK coinstake outputs are not spent.
  - Outputs carrying an eToken, and those UniSat leaves out of its available
    list (inscriptions, runes), count in the balance and are never spent.
  - An indexer proves its network by an identity block: block 1 on Bitcoin
    and Fractal Bitcoin, 896,800 on eCash, the genesis elsewhere (before:
    Bitcoin's genesis); a Fractal send checks every indexer it read inputs
    from before building and again before signing.
  - An Esplora endpoint whose `/fee-estimates` fails is asked mempool's
    `/v1/fees/recommended` instead, on every Esplora network
    (`api/esplora.rs`).
  - A scanned code with another network's BIP-21 scheme is refused on every
    network.
  - The change dust `service/send_stage_utxo.rs` kept for Bitcoin, Zcash,
    Decred and Kaspa is on `Chain::relay_dust_threshold` with every other
    network's (before: `legacy_change_dust`, the legacy-format networks'
    alone).
  - New tokens QTUM, FB, XEC (2 decimals) and DGB; nine endpoints (Blockbook,
    Esplora, Chronik, UniSat) and an explorer each.
- **Algorand joins the registry** (new `registry/algorand.rs`,
  `derivation/algorand.rs`, `derivation/ed25519_extended.rs`,
  `send/algorand.rs`, `api/algod.rs`, `api/algorand_indexer.rs`,
  `service/send_stage_algorand.rs`)
  - Receive, balance, history, an ALGO `pay` (build, sign, broadcast,
    status) valid for 1,000 rounds, the 25-word mnemonic (new
    `WalletSecretFormat::AlgorandMnemonic`, also a created wallet's, with no
    path or passphrase), BIP-39 along ARC-52, raw-key and watch-only import.
  - Refused before signing: a rekeyed sender, an amount and fee above the
    sender's balance over its minimum balance, a recipient left under its
    minimum balance (0.1 ALGO for an empty account), another genesis, assets
    and notes.
  - Cardano's BIP32-Ed25519 child step, extended public key and extended-key
    signing moved to `derivation/ed25519_extended.rs`; a child scalar of
    2^255 or more is refused at the step that reaches it (before: summed
    mod 2^256 unchecked).
  - New token ALGO (6 decimals); two `algod` and two `algorand-indexer`
    endpoints; Allo explorer.
- **VeChainThor joins the registry** (new `registry/vechain.rs`,
  `send/vechain.rs`, `api/thor_rest.rs`, `service/send_stage_vechain.rs`)
  - Receive at `m/44'/818'/0'/0/{account}`, balances of VET, VTHO and tracked
    VIP-180 tokens (new `TokenStandard::Vip180`), history, one-clause
    dynamic-fee (type `0x51`) sends of each (build, sign, broadcast, receipt
    status, 60-block expiry), raw-key and watch-only import.
  - Fees are paid in VTHO (`Chain::fee_token_contract`, `pays_own_fee`,
    `fee_holding_template`); `SendAffordabilityInput.is_native` is
    `pays_own_fee`.
  - A path matches a derivation profile when its account segment is hardened
    as the template's is (before: only a hardened account segment matched).
  - A memo and a mixed-case address without its EIP-55 checksum are refused;
    fee delegation, several clauses and contract calls are not offered.
  - New tokens VET and VTHO; five `thor-rest` endpoints (one without
    broadcast); VeChain Explorer.
- **Filecoin joins the registry** (new `derivation/filecoin.rs`,
  `send/filecoin.rs`, `api/lotus_json_rpc.rs`, `api/filfox.rs`,
  `service/send_stage_filecoin.rs`)
  - `f1` accounts at `m/44'/461'/{account}'/0/0`: receive, balance, history,
    FIL sends (build, sign, broadcast, status) to `f0`, `f1`, `f2`, `f3` and
    `f410f` addresses, through `InvokeEVM` to an `f410f` address or an
    `f0`/`f2` actor that has one; raw-key and watch-only import.
  - Gas is the node's estimate under a 0.07 FIL cap; the network name and
    genesis are checked before a build, a signature, a status read and a
    broadcast.
  - The nonce is read again before signing and before broadcasting, and a
    signed message whose nonce the sender has used is expired; an `f0` or
    `f2` recipient must exist.
  - New token FIL (18 decimals); three `lotus-json-rpc` endpoints and Filfox
    for history; Filfox explorer.
- **Tezos joins the registry** (new `registry/tezos.rs`,
  `derivation/tezos.rs`, `send/tezos.rs`, `api/tezos_rpc.rs`, `api/tzkt.rs`,
  `service/send_stage_tezos.rs`)
  - `tz1` accounts at SLIP-10 `m/44'/1729'/{account}'/0'`: receive, spendable
    balance, history, XTZ sends forged locally for protocol Ushuaia with a
    reveal while the key is unrevealed; `edsk` and hex keys (new
    `WalletSecretFormat::TezosSecretKey`); watch-only `tz1` to `tz4` and
    `KT1`.
  - Gas and storage come from simulation, the fee from the minimal-fee rule
    and 100 mutez more; the reviewed fee includes the most storage can burn.
    An unrevealed account's first transfer is bounded by the protocol's reveal
    and transfer costs, simulated at signing, and pays only a `tz` address.
  - Refused: a node on an unlisted protocol or about to switch protocol, and
    FA1.2 and FA2 tokens.
  - A batch reserves each of its counters (`send/reservation.rs`).
  - New token XTZ (6 decimals); three `tezos-rpc` endpoints and TzKT for
    history and status; TzKT explorer.
- **Starknet joins the registry** (new `registry/starknet.rs`,
  `derivation/starknet.rs`, `send/starknet.rs`, `api/starknet_json_rpc.rs`,
  `service/send_stage_starknet.rs`)
  - Ready (Argent X) accounts of class 0.4.0, derived as Argent derives them
    or from a Stark key (new `WalletSecretFormat::StarknetPrivateKey`,
    `KeyScheme::Stark`, `AddressNormalization::HexFelt`): balances of STRK and
    ERC-20 tokens (ETH listed), v3 sends with an undeployed account's
    deployment in the same review, watch-only import; no history.
  - Resource bounds are the estimate with half again on each amount and
    price; a deployment reserves nonce 0.
  - Refused: a deployed account of another class, with a guardian or a
    changed owner, and memos.
  - The Stark public key is recorded at import
    (`WalletState.starknet_public_key`); an undeployed wallet without it
    cannot send.
  - New token STRK (18 decimals); two `starknet-json-rpc` endpoints; Voyager
    explorer.
  - `tags_agree_with_the_registry` no longer requires a layer 2 to be EVM
    (`catalog/chains.rs`).
- **Hedera joins the registry** (new `registry/hedera.rs`,
  `derivation/hedera.rs`, `send/hedera.rs`, `api/hedera_mirror.rs`,
  `api/hedera_hapi.rs`, `service/send_stage_hedera.rs`; `api/grpc.rs`
  `pinned_channel`)
  - An existing `0.0.N` account holding one key, found by that key on a
    verified mirror or named at import (CLI `--named-account` takes `0.0.N`;
    a key several accounts hold needs one named); any other account is
    refused, and no phrase is created.
  - Ed25519, ECDSA and EVM profiles (new `DerivationProfile::Ecdsa`,
    `DerivationProfile::Evm`); DER or bare hex keys (new
    `WalletSecretFormat::HederaPrivateKey`), a bare key tried as Ed25519 and
    secp256k1 and stored as the DER of the scheme the account holds;
    watch-only `0.0.N` with an optional HIP-15 checksum.
  - HBAR `CryptoTransfer` with a memo of up to 100 bytes and a $0.001 fee
    bound, signed for three nodes and sent over gRPC TLS pinned to each
    node's certificate hash; status and history from the mirror.
  - Refused: a recipient that is missing, deleted or needs its own
    signature; a send carries HBAR only.
  - An import preview reads the network for Hedera; the funds finder leaves
    Hedera out.
  - New token HBAR (8 decimals); a mirror endpoint and a `hedera-hapi`
    address-book endpoint; HashScan explorer.
- **The 25 networks take popular ranks 67 to 91** (`core/data/chains.toml`),
  with 24 new icons from each project's own marks and web3icons (Kava shares
  Kava EVM's) and VTHO's.
- **A health check proves the network of each new API but Filfox and
  UniSat** (`api/health.rs`): the genesis on algod, the Algorand indexer,
  Thor and Hedera's mirror and address book, Lotus's network name and
  genesis, the chain id on Tezos nodes, TzKT and Starknet, Chronik's identity
  block, and a Cosmos gateway's `node_info` where it claims verification,
  with its latest block.
- **A text memo is a network's own length** (`PaymentMemoKind::MemoText`,
  `PaymentMemo::text`, `Chain::memo_text_limit`; `send/payment_memo.rs`):
  Stellar's 28 bytes, a Cosmos network's `max_memo_characters` (512 on Cosmos
  Hub, Terra, Kava and BandChain, 256 elsewhere) and Hedera's 100 bytes, with
  no zero byte on Hedera (before: Stellar's 28 alone). CLI `--memo-text`
  takes any of them.
- **A gRPC gateway's HTTP 500 is an answer** (`RetryProfile::GatewayRead`,
  `api/http.rs`): the Cosmos client's reads and simulation and the Tezos
  simulation read it with its body instead of retrying it as an outage; a
  Cosmos simulation's 500 is a rejection carrying the node's message, and a
  Tezos node's a refusal naming its error ids. Cosmos broadcasts still retry
  as chain writes.
- **A chain entry names its coin's and its fees' symbols**
  (`ChainEntry.coin_symbol`, `ChainEntry.fee_symbol`; `catalog/chains.rs`):
  `gas_token_symbol` is `coin_symbol`, and `fee_symbol` is VTHO on
  VeChainThor and the coin's elsewhere.
- **The foreign interfaces' comments name both apps** (`secrets/store.rs`,
  `service/refresh_engine.rs` `RefreshObserver`) (before: Swift and a "future
  Android").
- **Tidying the CHANGELOG covers all three repositories** (`AGENTS.md`
  "CHANGELOG"): spectra-apple's and spectra-android's sections are tidied in
  the same pass as this one's.
- Measured: 2026-11-24, Cosmos endpoints
  ([cosmos-2026-11-24](audits/cosmos-2026-11-24/README.md)): all 35
  catalogued gateways through `CosmosClient`; each network's chain id,
  accounts, balances and fee market answered; a 1-unit self-send with a memo
  simulated on all thirteen (Cosmos Hub, Akash and Babylon from a funded
  address in a second run), 50,001 to 125,924 gas; Celestia's Polkachu and
  Saga's NodeStake gateways failed history searches, Numia's served no
  `node_info` and Allora's own answered 401 on the fee market, so the catalog
  claims none of those; Allora's explorer address page redirected to a 502
  page.
- Measured: 2026-11-24, Kusama Asset Hub and Astar endpoints
  ([substrate-mainnets-2026-11-24](audits/substrate-mainnets-2026-11-24/README.md)):
  genesis and metadata (statemine spec 2005003, Astar spec 2500) answered by
  every catalogued endpoint; Dwellir's Astar node served other metadata and is
  not catalogued; a transfer quoted about 0.000059 KSM and 0.0397 ASTR.
- Measured: 2026-11-24, Tezos mainnet nodes
  ([tezos-2026-11-24](audits/tezos-2026-11-24.md)): three nodes on protocol
  Ushuaia; three included operations forged again by Taquito 25.1.1 to their
  chain hashes.
- Measured: 2026-11-24, Starknet nodes
  ([starknet-2026-11-24](audits/starknet-2026-11-24.md)): two keyless nodes
  (spec 0.10.3 and 0.9.0) on `SN_MAIN`; no keyless history source.
- Tests: 194 new
  - Cosmos 16: addresses or signed `MsgSend`s diverging from CosmJS and
    ethers.js; a gateway answer misread (coins, accounts, gas prices, history
    entries, not-found answers); a refusal missed before signing.
  - Substrate 8: Kusama or Astar transfers diverging from polkadot.js on
    their own metadata; account records or extrinsic outcomes misread;
    metadata naming another network's fee extension accepted; an address
    under another network's prefix accepted; the two networks' wallet rules
    drifting; a refusal missed before building or signing.
  - Account UTXO networks 23: Chronik and UniSat answers misread; signed
    spends diverging from bitcoinjs-lib and ecash-lib; another network's
    indexer or address accepted; a new network failing to build, sign and
    broadcast; immature Qtum outputs spent; another network's BIP-21 scheme
    accepted; a network's node parameters drifting.
  - Algorand 26: keys, addresses or signed `pay` transactions diverging from
    algosdk and xhd-wallet-api; a node or indexer answer misread; an endpoint
    on another genesis passing its health check; a refusal (rekeyed sender,
    minimum balance, another genesis) missed before signing.
  - VeChainThor 21: accounts or transactions diverging from
    @vechain/sdk-core; a node answer misread; fees priced in VET instead of
    VTHO; a refusal missed before signing.
  - Filecoin 24: addresses or signed messages diverging from Zondax and
    Glif; a Lotus or Filfox answer misread; an `f410f` recipient paid
    without `InvokeEVM`; a used nonce sent again.
  - Tezos 29: keys or forged operations diverging from Taquito; an `edsk` key
    read as other than its seed; a node or TzKT answer misread; a node's
    refusal hidden; an unrevealed account's first transfer sent without its
    reveal.
  - Starknet 21: keys, addresses or transactions diverging from Argent's
    derivation and starknet.js; a node answer misread; an undeployed account
    sending before its deployment.
  - Hedera 26: keys, checksums or transfers diverging from @hashgraph/sdk; a
    mirror answer or node certificate misread; an account holding other keys
    imported; a refusing node treated as accepting.
  - CLI acceptance: a Cosmos Hub memo send across processes, a node's
    refusal kept uncertain and retried with the same bytes
    (`send_cosmos.py`); a custom Bitcoin indexer proving itself by block 1
    (`multisig_psbt.py`).
- Docs: `CHAIN-SUPPORT.md` (counts; wallets and transfers: Substrate
  networks, phrase formats, profiles, memos, a paragraph per new protocol,
  raw keys, account keys, UTXO networks and indexers, identity blocks, fees
  and dust, the new UTXO address forms; fungible assets: VeChainThor and
  Starknet; history); `OPEN-ITEMS.md` (Substrate history); `tests/README.md`
  (`send_cosmos.py`, the new generators); `ARCHITECTURE.md` ("Core module
  boundaries": the registry's protocol-family modules); `AGENTS.md`
  ("CHANGELOG"); new records
  `docs/audits/cosmos-2026-11-24/`,
  `docs/audits/substrate-mainnets-2026-11-24/`,
  `docs/audits/tezos-2026-11-24.md` and `docs/audits/starknet-2026-11-24.md`.
- Total: 179 .rs files changed (75 new, 1 deleted), +24497 −826 lines; data
  and config files: `core/data/` (6), `core/tests/fixtures/` (19, 14 new),
  `tests/vectors/` (15, 8 new, with `package.json` and `package-lock.json`),
  `tests/acceptance/` (3, 1 new), `icons/crypto/` (25 new); dependencies:
  added `starknet-crypto` 0.8.1, and `hyper` 1 for tests; `#[test]`
  functions 1489 → 1682 (one removed with `derivation/bittensor.rs`),
  `cargo test` 1682 passing.

## Beta Commit 256

- **Core builds as the package spectra-android links**
  (`scripts/build-android-package.sh`, `make android`)
  - A shared library per ABI (`arm64-v8a` and `x86_64`), linked by the NDK's
    clang for the app's minimum API level (`ANDROID_MIN_SDK`, 33 by default)
    and stripped of all but what the loader reads, and the Kotlin bindings
    generated from the unstripped library, written to
    `target/SpectraCoreAndroid/`; built in `target/android/`.
  - An NDK older than r28 is refused: it links with 4 KB pages.
- **The error field every variant called `message` is `reason`**
  (`SpectraBridgeError`'s `Network`, `Decode`, `InvalidInput`, `Failure`,
  `StoreUnreadable`; `SecretStoreError::Backend`; and the CLI's mapping of
  them) (before: `message`, which a Kotlin exception already has, so the
  Kotlin bindings did not compile).
- **A TON transfer carries a text comment** (`PaymentMemoKind::Comment`;
  `registry/send.rs` `payment_memo_kinds`, `send/payment_memo.rs`,
  `send/ton.rs`, `service/send_stage_ton.rs`; CLI `--comment`)
  - A native transfer's comment is its message body, a jetton transfer's its
    forward payload by reference; up to 123 bytes, one cell (before: TON
    payments carried none, so an exchange deposit that needs one could not be
    sent).
  - A TON multisig order carries the spend's comment
    (`service/multisig_ton.rs`; before: `comment: None` whatever the spend
    named).
  - A scanned `ton://transfer` code's `text` comes back as the comment, and a
    code with a `bin` body or `init` state is refused; a receive request
    writes the comment as `text` (`send/scanned_payment.rs`; before: `text`
    was dropped).
- **Eight stablecoin deployments join the catalog** (`core/data/tokens.toml`):
  USDT and USDC on BNB Smart Chain (Binance-Peg, 18 decimals) and on Asset Hub
  (assets 1984 and 1337), USDT0 on Plasma, USDC on Sei, USDT on Celo and USDT
  on opBNB.
- **Asset Hub `Assets` pallet assets are a token standard** (`Asset Hub
  Asset`, by asset ID; `registry/tokens.rs`, `catalog/tokens.rs`,
  `core/data/chains.toml` for Polkadot and Westend)
  - Balances and decimals are read from the asset's `Account` and `Metadata`
    entries, every tracked asset from one pinned state (new
    `api/substrate_json_rpc/assets.rs`).
  - A transfer is `Assets.transfer` from the runtime metadata, its fee in DOT
    quoted as that transfer; all of a holding may be sent
    (`send/polkadot.rs`, `service/send_stage_substrate.rs`,
    `service/send_preview.rs`, `service/owned_send.rs`).
  - Refused at build and again before signing: a frozen or destroying asset,
    a frozen or short holding, a remainder below the asset's minimum balance,
    a blocked recipient, less than the minimum to a recipient holding none,
    and an insufficient asset to a recipient without DOT; at signing, also
    decimals changed since the review.
  - The four Asset Hub endpoints claim `token-balance`, which the Substrate
    adapter now offers on a network that hosts tokens
    (`core/data/endpoints.toml`, `api/endpoint.rs`).
- **A storage key covers maps of several keys** (`metadata/pools.rs`
  `storage_key`; before: a plain value or a map of one key).
- **The pools' runtime-metadata and storage reads are the Substrate
  client's** (`api/substrate_json_rpc.rs` `runtime_metadata`,
  `storage_value`; before: `pools_metadata` and `pool_storage` in
  `pools.rs`).
- **Sixteen EVM mainnets join the registry** (`registry/mod.rs`,
  `registry/evm.rs`; `core/data/chains.toml`, `endpoints.toml`,
  `explorers.toml`, `tokens.toml`, `chain-wiki.toml`, `crypto-wiki.toml`;
  `icons/crypto/`)
  - Kaia (8217), XDC Network (50), Robinhood Chain (4663), Kava EVM (2222),
    Chiliz Chain (88888), Stable (988), Arc (5042), Manta Pacific (169), Metis
    (1088), Flare (14), Katana (747474), MegaETH (4326), Merlin Chain (4200),
    peaq (3338), Somnia (5031) and 0G (16661), each deriving along
    `m/44'/60'` with its own RPC endpoints and explorer.
  - Native coins: ETH on Robinhood Chain, Manta Pacific, Katana and MegaETH;
    BTC at 18 places on Merlin; USDT0 on Stable and USDC on Arc, both at 18
    places; KAIA, XDC, KAVA, CHZ, METIS, FLR, PEAQ, SOMI and 0G as new tokens.
  - Catalog deployments: USDT on Kaia and Kava EVM, USDC on XDC, USDG on
    Robinhood Chain, USDT0 on Flare and MegaETH, XAUt0 and STABLE on Stable,
    EURC on Arc, MANTA on Manta Pacific, KAT on Katana, MEGA and USDe on
    MegaETH, and MERL on Merlin.
  - Keyless Blockscout history and discovery on Chiliz, Manta Pacific, Metis,
    Flare, MegaETH, Merlin and Somnia; the other nine have none built in.
  - Manta Pacific, Katana and MegaETH price L1 data and operator fees from
    the Fjord oracle, Metis from Bedrock's `getL1Fee`.
  - Sixteen icons, from web3icons (MIT) and the networks' own marks.
- **Kava EVM, Merlin and Metis send legacy EIP-155 transactions**
  (`registry/evm.rs` `evm_legacy_transactions`; `send/evm.rs`;
  `api/evm_json_rpc.rs` `fetch_fee_estimate`; `send/ethereum.rs`)
  - Priced at `eth_gasPrice`, which fills both reviewed fee fields, estimated
    with `gasPrice` and signed with an EIP-155 `v`; a split fee or an access
    list is refused (before: only EIP-1559 transactions, which these nodes
    reject).
  - Their preview reads "Gas price N gwei".
- **`eth_feeHistory` asks with a hex block count** (`api/evm_json_rpc.rs`;
  before: a JSON number, which XDC and Somnia nodes refuse).
- **Chiliz and 0G tips have a floor** (`registry/evm.rs`
  `evm_min_priority_fee_wei`): 1 gwei and 4 gwei, raised from a lower
  fee-history sample (before: a zero sample priced a zero tip).
- **A coin's own ERC-20 face is not a token** (`registry/evm.rs`
  `evm_native_coin_contract`; `service/network_tokens.rs`,
  `store/token_preferences.rs`)
  - Discovery leaves Celo's CELO, Stable's USDT0, Arc's USDC, Metis's METIS
    and peaq's PEAQ faces out of an indexer's listing.
  - Adding one as a custom token is refused as a duplicate.
- **An XDC address is read with its `xdc` prefix** (`validation/address.rs`,
  `send/flow.rs`), stored and sent as `0x`.
- **A mixed-case EVM address is read with its EIP-55 checksum wherever it
  is normalized** (`send/flow.rs` `normalize_address`): one whose checksum
  fails stays as typed and is refused as a destination, an ENS answer or an
  address book entry (before: normalizing lowercased it first, so
  `is_valid_send_address` and every caller that normalized before validating
  accepted a mistyped address).
- Measured: 2026-11-24, the 16 networks probed read-only (identity, fee
  inputs, transaction types, rollup oracles, indexers, token metadata) and
  every catalogued EVM fee endpoint asked for a hex-count fee history: 86 of
  88 answered, the two Blast endpoints neither form
  ([evm-mainnets-2026-11-24](audits/evm-mainnets-2026-11-24/README.md)).
- Tests: 17 new
  - TON 1: a multisig order dropping the spend's comment; the jetton vectors
    (v4R2 and W5, @ton/core), memo rules and payment-code tests now also guard
    the comment.
  - Asset Hub 6: `Assets.transfer` and its storage keys and values diverging
    from @polkadot/types on either runtime, a preview quoting the coin's
    transfer, an unchecked build or signing, each refusal.
  - EVM fees 3: a block count a node refuses, a legacy network priced from
    the fee market, a zero tip on a network with a floor.
  - Legacy signing 3: the signing payload or signed bytes diverging from
    ethers.js on Kava EVM, Merlin and Metis; a split fee or an access list
    signed or dropped; a legacy estimate asking about fee-market fields.
  - Registry 1: the legacy, tip-floor and coin-face sets drifting, or a
    coin's face catalogued as a token.
  - Addresses 2: an `xdc` address refused on XDC, or read on another
    network; a destination with a broken EIP-55 checksum accepted on any EVM
    network.
  - Discovery 1: a coin's ERC-20 face listed as a token; the custom-token
    test now also guards adding one.
  - CLI acceptance: a jetton comment missing from the signed message
    (`send_tokens.py`); a USDT transfer across processes (`send_polkadot.py`).
- Docs: `CHAIN-SUPPORT.md` (chain and private-key counts; wallets and
  transfers: TON comments, scanned codes, EVM transfers and checksums;
  fungible assets: Asset Hub assets, the coin's ERC-20 face, the added
  deployments; history: networks without an indexer); `OPEN-ITEMS.md`
  (built-in EVM history providers); `tests/README.md` (`send_polkadot.py`,
  `send_tokens.py`, the new and extended generators); `AGENTS.md`
  ("Repositories", "Verification"); `README.md`; `ARCHITECTURE.md`
  (introduction, "Workspace", "Ownership"); `FFI-BOUNDARY.md` (introduction,
  "Export shapes", "Runtime and storage traps"); `scripts/README.md`
  ("Android package"); new records `docs/audits/token-additions-2026-11-24/`
  and `docs/audits/evm-mainnets-2026-11-24/`.
- Total: 70 .rs files changed (2 new), +2171 −252 lines; data and config
  files: `core/data/` (6), `core/tests/fixtures/` (5, 2 new), `tests/vectors/`
  (5, 2 new), `tests/acceptance/` (3), `icons/crypto/` (16 new), `Makefile`,
  `scripts/build-android-package.sh` (new); dependencies unchanged; `#[test]`
  functions 1472 → 1489, `cargo test` 1489 passing.

## Beta Commit 255

- **A change here is finished when the apps match it** (`AGENTS.md`
  "Repositories"): the same task checks spectra-apple and spectra-android for
  what depends on the change and updates them.
- **spectra-apple's checks run on the one simulator its `AGENTS.md` names**
  (`AGENTS.md` "Verification").
- **Every report's verdict sets the CLI's exit code** (`cli/src/out.rs`
  `Report::failure`): a report that found a failure — an unreachable
  endpoint, a failed self-test, a wallet that did not refresh, failed pending
  maintenance — prints, then exits 1 with what failed (before: only
  `diagnostics configured` and `txs refresh-pending` exited non-zero, with 1
  and 3; `endpoints probe`, `refresh` and `diagnostics self-test` exited 0).
- **CLI commands name an asset by its deployment id, never its ticker**.
  - `send probe` takes `--holding` and defaults to the chain's own coin
    (before: `--asset` took a symbol, with `--chain` to narrow it).
  - `wallet hide` and `show` take only a deployment id (before: a symbol only
    one holding had also matched).
  - `alert remove` takes an alert id and removes that alert (before: a symbol
    removed every alert on it).
  - `portfolio live` and `alert list` print each holding's and alert's id.
- **`portfolio live` reads balances through core's refresh sweep**
  (`cli/src/cmd/refresh.rs` `sweep`, shared with `refresh`; before: its own
  loop over every wallet).
- **The refresh observer hears every wallet's result**
  (`RefreshObserver::on_balance_refreshed`): the engine reports each wallet
  with why its balance could not be read, and `refresh` and `portfolio live`
  print it (before: `on_balance_updated` carried the updated `WalletState`
  on success and nothing on failure).
- **Validator directories are plain functions returning `ApiError`**
  (`staking/` `fetch_validators`; `StakingError` and the six one-method
  `*StakingClient` types are deleted): the dispatcher answers `None` for a
  chain that does not stake, which the service refuses.
- **The explorer lookup is a catalog function**
  (`catalog::explorers::explorer`; before: an `impl Chain` outside
  `registry/`), and the unused `From<reqwest::Error> for SpectraBridgeError`
  is deleted.
- **Fields, derives and defaults nothing reads are removed**.
  - `ChainEntry.supports_staking`, `ChainSelfTestResult.chain_label` and
    `DiagnosticsBundlePayload.schema_version` leave the FFI records.
  - `ResidentState` no longer carries `schema_version`: the database keeps
    the layout version beside the state and refuses any other, as before.
  - The price-alert evaluation types and `PriceAlertNotification` no longer
    derive `Deserialize`, and neither it nor the history cursor fills a
    missing field with a default.
- **One parser reads Solana token accounts** (`api/solana_json_rpc.rs`
  `sum_token_accounts`; `fetch_spl_balance` takes one mint).
  - The holdings listing refuses an answer without its account list and an
    account without its parsed data, mint or token amount (before: it
    skipped them, reporting what they held as nothing).
  - Listed decimals are capped at 38 as every other read's are, and
    `SplBalance.balance_raw` is a `u128` (before: a string summed by
    re-parsing it).
- **A front end mirrors a narrow committed state**
  (`store::state::CommittedState`: revision, settings, address book, token
  preferences, price alerts).
  - `apply_state_command`, `open_state` and `reset_data` return it, and the
    new `committed_state` export reads it; `discard_state` returns nothing
    (before: each returned the whole `ResidentState`, wallets, quotes and
    diagnostics included, which the app never read).
  - `ResidentState` no longer crosses the FFI: `app_state` is a Rust method
    for core and the CLI, `PortfolioSnapshot` carries the committed state
    and the quote state in `quotes`, and `AppRefreshResult` drops its
    `state`.
- **Closed sets are written once**.
  - A submitted record is `kind <> 'receive'`, spelled once by
    `wallet_db::submitted_kind!` for the pending-sender index and the two
    queries it serves; the history table no longer checks `kind` against a
    list (before: the same 13 kinds written out four times).
  - `registry::HistoryFamily` (EVM, UTXO, account) is the one three-way
    split: `Chain::history_family` replaces `history_refresh_kind` and
    `transaction_merge_strategy`, whose enums are deleted, and the UTXO
    family comes from `uses_account_utxo` alone, which the pending-status
    poll reads too (before: the family listed three times).
  - An owned address's branch is `AddressBranch` (`receive`, `change`),
    stored and compared as one (before: the strings "external" and
    "change", and 0 and 1 in two readers); `KeypoolDiagnostic` carries it.
  - A custom endpoint's `api` is an `EndpointApi` in `AppSettingUpdate`
    (before: a string matched by name).
- **Core says whether a total is complete and whether a wallet needs its
  account reserve**.
  - `QuotedTotal` carries `unread_wallet_count`, the wallets in the total
    whose balances have not been read, and `is_unread` when none has, for
    the portfolio and each wallet (before: the app derived both from each
    wallet's `balancesReadAt`).
  - The `wallet_account_reserve` export answers for a wallet: its chain's
    reserve until it holds its coin, `None` on a network without one;
    `account_reserve` by chain is a Rust method the CLI's `wallet setup`
    calls, and `ChainEntry.requires_account_reserve` goes (before: the app
    checked the chain and the native balance and then asked by chain).
- **Every network has an address hint** (`core/data/chains.toml`): the 30
  networks without one, testnets included, now have theirs, and the catalog
  refuses a chain without it (before: an empty default, which the app filled
  from its own string tables for 11 chains).
- **Core exports the reset scopes and TON wallet versions in order**
  (`reset_scopes`, `ton_wallet_versions`), so a scope or version core adds
  reaches the app's lists.
- **Every big integer in a prepared payload is an exact decimal string**
  (`send/stages.rs` `units_u128`, now with an `option` form): NEAR's amount,
  fee budget and registration deposit, a NEAR function call's deposit and
  fee budget, a key deletion's fee budget, EVM's fees, value and extra fee,
  and Polkadot's amount and fee (before: TON's amount alone; the rest were
  JSON numbers, or strings parsed back at each use). Stored artifacts change
  format.
- **One seed-phrase derivation per family, borrowing the phrase**
  (`derivation/dispatch.rs` `derive_for_chain`, each family's
  `derive_from_seed_phrase`; the 48 `derive_<network>` functions are
  deleted).
  - `derive_for_chain` borrows the phrase and takes the wanted outputs as one
    `Wants` (before: the phrase copied into a `String`, and three positional
    booleans); each family reads its network from the `Chain`, and the eleven
    secp256k1 UTXO families derive through one function and
    `Chain::encode_discovery_address`.
  - `DerivationResult` holds the address, the public key and a `Zeroizing`
    private key, and no longer derives `Debug` (before: a plain `String`, and
    `account`, `branch` and `index`, which nothing read).
  - The P2PKH-only networks' refusal of another script type is deleted
    (unreachable: dispatch always passed P2PKH), and `derivation/dogecoin.rs`,
    left empty, is deleted.
- **Asset identities live in the registry** (`registry/xrpl_amount.rs`,
  `stellar_asset.rs`, `cardano_asset.rs`; `NftStandard` in
  `registry/tokens.rs`): `catalog/` no longer imports from `api/`, a layer
  above it, and `api/evm_nft.rs` keeps only the ERC-165 and ABI call
  encoding (before: all four in `api/`).
- **Each module's tests sit in one place, named after it**
  (`core/src/{service,send,api,derivation,store,registry}/tests/`).
  - 10 test files are renamed after the module that hosts them, 7 move to
    the module they test and 13 merge into their host's one file, so that
    `tests/<module>.rs` belongs to the module declaring it as `mod tests;`
    (before: hosts such as `send_stage_protocols`, `send_stages` and
    `service/mod.rs`'s `app_boundary` loaded tests of other modules, under
    other names).
  - Inline test modules move out of line wherever the module also had a test
    file, in 13 modules, `balance_refresh`'s 540 lines among them.
  - The tests of wallet deletion, status-tracker pruning, UTXO discovery and
    `Chain::history_family` move to `service::state`, `service::transactions`,
    `service::address_discovery` and `registry`.
  - `send_stage_support`, a shared fixture, is declared in `service/mod.rs`
    beside `loopback_service` (before: in `send_stage_protocols`).
- Tests: 3 new
  - Address hints 1: a network the catalog lists without one.
  - Exact amounts 1: a prepared big integer written as a JSON number, or
    read back other than as written.
  - Unread totals 1: a total that counts read wallets as unread, or claims a
    figure while none has been read.
  - The Solana account test also refuses a listing with an account missing
    its mint and an answer for another mint; the wallet-reserve test covers
    a funded, an unfunded and a reserve-free wallet; the history-row check
    no longer refuses an unknown kind.
- Docs: `ARCHITECTURE.md` "Core module boundaries" (the registry's asset
  identities, where unit tests live, the error conversions);
  `FFI-BOUNDARY.md` "Committed state" (new) and "Versions"; `AGENTS.md`
  "Repositories" and "Verification".
- Total: 253 .rs files changed (5 new, 16 deleted, 18 renamed), +9558 −11251
  lines; data and config files: `core/data/chains.toml`,
  `tests/acceptance/__main__.py`, `tests/acceptance/endpoints.py`;
  dependencies unchanged; `#[test]` functions 1469 → 1472, `cargo test` 1472
  passing.

## Beta Commit 254

- **The CLI's windowed interface is deleted** (`cli/src/tui/`,
  `tests/acceptance/tui.py`): it ran the same subcommands in tabs and windows
  as kitty or tmux would, with no operation of its own; `spectra` with no
  command is clap's missing-subcommand error, and the help no longer
  describes the interface. Its dependencies `ratatui`, `portable-pty`,
  `vt100`, `unicode-width` and `base64` are removed from `cli/Cargo.toml`.
- **The CLI's commands follow one set of conventions** (`cli/src/`).
  - A wallet or contact name that matches nothing exits 3, as an unknown
    transaction, setting or alert does (before: 1).
  - `wallet import` with a private key and a derivation override is a usage
    error from clap, exit 2 (before: exit 3 from the CLI's own check, which
    core repeats).
  - Every recipient flag is `--to` (before: `--destination` on `send review`,
    `preview`, `quote`, `build-owned`, `owned-broadcast` and `self-check`).
  - `endpoints`, `txs`, `price`, `portfolio` and `currency` take subcommands
    instead of mode flags: `endpoints` `probe`, `list`, `add`, `replace`,
    `remove` and `custom-only`; `txs` `list`, `summary`, `show`, `endpoints`,
    `page`, `recheck`, `refresh-pending`, `maintenance`, `poll` and
    `replaceable`; `price` `spot`, `stored` and `refresh`; `portfolio` `live`,
    `stored`, `pins`, `set-pins` and `unpin`; `currency` `show`, `set`,
    `rates` and `refresh-rates`.
  - `portfolio set-pins` and `unpin` report the pins after the change
    (before: a pin flag alone fell through to a live valuation).
  - `portfolio`'s `--skip-unreachable`, which defaulted to true and could not
    be turned off, is removed with its unreachable stop-on-error branch.
  - The 28 argument structs that declared the "Wallet id, name or address"
    positional flatten `WalletArg` instead.
  - Every `--yes` gate says what the command would do, then "re-run with
    --yes" (`CliError::unconfirmed`; before: three message shapes).
- **The CLI's offline calculators and artwork lookups are deleted**: `send
  amount`, `shortcut`, `fee-units`, `assemble` and `affordability`, and `token
  artwork` and `format`, which no suite ran and whose rules core tests;
  `send scan`, `request`, `fees` and `overrides` stay. `token_artwork_name` and
  `chain_artwork_name` go with them (`store/artwork.rs`).
- **The `diagnostics_bundle_from_json` export is removed**
  (`diagnostics/export.rs`): the app no longer imports a bundle, and core's
  tests read one with serde.
- **`store/state.rs` is split by concern** (`store/settings.rs`,
  `store/address_book.rs`, `store/token_preferences.rs`; the wallet types in
  `store/wallet_domain.rs`; the reducer's tests in
  `store/tests/state_reducer.rs`): each module holds its types, helpers and
  the reducer arms for its commands, which `reduce_state_in_place` routes to
  it, and `state` re-exports them (before: 2147 lines in one file).
- **Staking preparation is one function per network**
  (`service/staking_protocols.rs` `prepare_solana_staking`,
  `prepare_sui_staking`, `prepare_aptos_staking`, `prepare_near_staking`;
  `service/staking_substrate.rs` `prepare_pool_call`),
  which rustfmt formats (before: lines of up to 227 characters inside one
  closure that rustfmt left alone); a NEAR pool call's method, arguments and
  deposit come from `near_staking_call`, which the build and its recheck share.
- **One bounded byte reader decodes every encoded transaction and wire
  message** (`derivation::primitives::ByteReader`, used by the XRP, Stellar,
  Sui and BCH PSBT multisig decoders and the Litecoin wire codec), each
  refusing with its own error; protocol-specific reads stay in each module
  (before: five readers, each with its own `take`, integers and end check).
  A BCH PSBT whose CompactSize is longer than its value needs is refused
  (before: the reader accepted it).
- **Token balance reads share one shape** (`service/network_tokens.rs`): Sui,
  Aptos and NEAR read balance and decimals through `own_decimals_balances`,
  every catalog token's row comes from `TokenBalanceResult::known`, and
  `held_tokens` chooses its service by family as `fetch_token_balances` does
  (before: a macro for Sui and Aptos, a hand copy for NEAR, and each testnet
  named in its own arm).
- **The acceptance suites share base58, SCALE compact, `block_hash` and
  sign-then-broadcast from the harness** (`tests/acceptance/harness.py`
  `b58encode`, `b58decode`, `scale_compact`, `block_hash`,
  `Spectra.sign_and_broadcast`; before: copies in six suites, one SCALE copy
  without the big-integer mode). `__main__.py` no longer checks that
  `send owned-broadcast` needs `--yes`, which `send.py` checks.
- **Stale notes and vestigial code are corrected**.
  - `core/tests/fixtures/monero-local.md` no longer says subaddress discovery
    is unsupported, which `CHAIN-SUPPORT.md` contradicts.
  - README's `make lint` row names the source-scan tests.
  - `Cargo.toml` names `scripts/build-apple-package.sh` as what runs the
    bindgen.
  - `production_text` moves from `scripts/source_scan.py` into the one file
    that calls it, `scripts/test-source-scan.py`.
  - The CLI's tokio drops the unused `macros` feature (`cli/Cargo.toml`).
  - `refresh --endpoint` and `txs recheck --endpoint` use
    `Ctx::service_with_endpoints` (before: each wrote the endpoint update
    again).
  - `PasswordArgs`' doc comment is back on `PasswordArgs` (`cli/src/ctx.rs`;
    before: fused onto `WalletArg`'s).
- **The CLI configures logging, not core** (`cli/src/main.rs`;
  `service/mod.rs` `WalletService::new`): the stderr subscriber, quiet unless
  `RUST_LOG` asks, is installed by the CLI's `main`, and core no longer
  depends on `tracing-subscriber` (before: every `WalletService` construction
  installed a process-wide subscriber, so the app could not configure its own).
- **Every id core mints is a v4 UUID, compared exactly** (`store::new_id`).
  - Contacts, price alerts and multisig sessions get UUIDs as wallets and
    transactions do (before: 32 hex characters from `new_event_id`).
  - Wallet and transaction ids are stored and compared as given: the history
    store no longer lowercases them on write or compares with `lower()`, the
    `idx_wallets_lower_id` index is gone, and the services, the history
    refresh clock and the transaction merge compare ids with `==` (before:
    case-insensitive in the history store, eight service lookups, the refresh
    clock and the merge; exact in the reducer and the keypool, multisig and
    pagination deletes). Addresses and transaction hashes keep their case
    folding.
- **The test that pinned 58 catalog rows to the 2026-10-04 audit files is
  deleted** (`catalog/tokens.rs` `pending_catalog_tests`): no
  `# TODO(verify-token)` marker is left for it to read, and a corrected
  contract failed `cargo test`; the audits stay in `docs/audits/` as dated
  evidence.
- **The toolchain is no longer pinned** (`rust-toolchain.toml` deleted,
  `README.md`, `scripts/build-apple-package.sh`): builds use the current
  stable Rust (before: 1.98.1).
- **The app's internals are documented in spectra-apple**
  (`docs/ARCHITECTURE.md`): the Mac build, the composer's requests in
  `SendFlowState`, the rules that keep the Swift shell thin, `SendSession`'s
  ownership, the `AppState` composition, and the completion and
  `AmountPresentation` paragraphs move to spectra-apple's
  `docs/ARCHITECTURE.md`, which two sentences here point to.
- Tests: 1 new, 91 removed
  - Byte reader 1: a read past the end, or a CompactSize longer than its
    value needs, accepted.
  - The removed tests checked the windowed interface (89), the audit-pinned
    catalog rows and case-insensitive transaction ids; the diagnostics and
    history-id tests no longer check the deleted import's refusals or that
    ids differing in case are one record.
- Docs: `ARCHITECTURE.md` "Ownership", "Boundary design" (the thin-shell
  rules), "Persistent send stages", "Display precision" and "Refresh and
  diagnostics" (renamed from the native-flow headings), the `cli/` row, the
  primitives list (`ByteReader`); `tests/README.md` (`tui.py`); `README.md`
  "Development" (the toolchain) and "Verification" (`make lint`);
  `core/tests/fixtures/monero-local.md`.
- Total: 137 .rs files changed (4 new, 10 deleted), +3384 −7428 lines; data
  and config files: `Cargo.toml`, `Cargo.lock`, `cli/Cargo.toml`,
  `core/Cargo.toml`, `rust-toolchain.toml` (deleted),
  `scripts/build-apple-package.sh`, `scripts/source_scan.py`,
  `scripts/test-source-scan.py`, 29 files in `tests/acceptance/` (`tui.py`
  deleted); dependencies: the CLI
  drops `ratatui`, `portable-pty`, `vt100`, `unicode-width`, `base64` and
  tokio's `macros` feature, `tracing-subscriber` moves from core to the CLI,
  and 38 crates leave `Cargo.lock`; `#[test]` functions 1559 → 1469,
  `cargo test` 1469 passing.

## Beta Commit 253

- **Per-chain constants that callers restated are read from `registry::Chain`**
  (`registry/utxo.rs`, `registry/networks.rs`, `registry/staking.rs`)
  - Dogecoin and Bitcoin Cash P2PKH versions come from
    `fixed_utxo_address_versions`; Zcash's transparent version from
    `zcash_transparent_p2pkh_version` and Kaspa's address prefix from
    `kaspa_address_hrp` (before: constants in `derivation/dogecoin.rs`,
    `bitcoin_cash.rs`, `zcash.rs` and `kaspa.rs`, and literal bytes in the
    registry's discovery encoder).
  - Native-coin decimals in the EVM preview and history, Sui history, the PSBT
    spend amount, the Zcash shielded review check and the MWEB and Zcash
    shielded balances come from `native_decimals` (before: 18, 9 and 8 written
    at each site).
  - Substrate message verification decodes with the network's `ss58_prefix`,
    and refuses a network without one (before: 0 for Polkadot and 42 for
    anything else).
  - Aptos transaction and TON message lifetimes are
    `aptos_transaction_lifetime_secs` (600) and `ton_message_lifetime_secs`
    (60).
  - `aptos_chain_id`, `aptos_max_gas_amount`, `near_token_gas_limit`,
    `near_zero_balance_storage_limit`, `aptos_delegation_minimum`,
    `sui_staking_minimum`, `sui_staking_gas_budget` and `near_staking_gas_limit`
    return a `RegistryError` for another network (before: `None`, unwrapped or
    given a message at each call site).
  - Whether a network's staking reads sign with the wallet's key is
    `staking_reads_require_authorization`, which `staking_input_rules` and the
    CLI's `staking positions`, `recheck` and `repair` read (before: the CLI
    compared the chain with ICP itself).
- **ICP and Kaspa requests are written in `api/`; `send/` signs and hands over
  bytes or a typed transaction** (`api/icp_rosetta.rs`, `api/kaspa_rest.rs`,
  `send/icp_stages.rs`, `send/kaspa.rs`, `send/payload.rs`)
  - `IcpClient::suggested_transfer_fee` makes the Rosetta preprocess and
    metadata calls, and `submit_signed_transaction` takes the signed CBOR;
    `rosetta_post` is private (before: `send::icp_stages` built both
    construction requests and the submit document).
  - A signed ICP transfer is kept as `SignedPayload::Hex` of its CBOR (before:
    the Rosetta submit document as `Json`).
  - The balance and history reads name the ledger through
    `network_identifier()` (before: the ledger id written out in each).
  - A signed Kaspa transaction is `SignedPayload::Kaspa(KaspaTransaction)`,
    which `KaspaClient::broadcast` writes as kaspa-rest's `/transactions` body
    (before: `send::kaspa` wrote that body and the client posted it as given).
- **The Sapling parameters download from `api::zcash_parameters`** (before:
  `service/zcash_shielded.rs` fetched from a URL kept in
  `send/zcash_shielded.rs`).
- **One bounded body reader, `api::http::read_limited`**, used by
  `HttpClient::get_bytes`, the ICP replica status read and Monero's RPC
  transport (before: the latter two each read chunks by hand, and `get_bytes`
  read a body without a length header whole before checking it).
- **An EVM send's preview and its build encode the transfer with one function,
  `send::evm::transfer_call`** (`service/send_preview.rs`,
  `service/send_stages.rs`)
  - `fetch_evm_send_preview` takes the `TransferCall` (before: `value_wei` and
    `data_hex` strings it parsed back, from `prepare_evm_send_assembly`'s own
    hex encoder).
  - The `prepare_evm_send_assembly` export and its `EvmSendAssemblyInput`,
    `EvmSendAssembly`, `EvmSupportedToken` and `EvmSendError` types are
    removed; `spectra send assemble` prints `transfer_call`'s answer and no
    longer takes `--from`, which decided nothing.
- **An export exists only for a call the app's Swift makes; the CLI calls core
  as plain Rust** (`docs/FFI-BOUNDARY.md` "Callers and coverage")
  - 37 functions and methods only the CLI called are no longer exported,
    among them `build_send`, `review_owned_send`, `execute_owned_send`,
    `multisig_session`, `finalize_multisig`, `refresh_now`,
    `wallet_derived_state`, `send_affordability`, `self_tests_run_all`,
    `list_token_deployments` and the three artwork-name functions; the
    methods sit in `impl` blocks without `#[uniffi::export]`.
  - `tor_status` stays exported for the app's Tor test, which has no other
    reading of the status.
  - 30 types no export reaches lose their UniFFI derives, among them
    `SendAffordability`, `MaintenancePlan`, `NormalizedHistoryItem`,
    `DerivationResult`, `SendFeeField`, `TransactionStatusTrackerState` and the
    price-alert evaluation records; `HistoryDiagnostics` and
    `HistoryDiagnosticsSource` are plain structs (before: a macro that derived
    a UniFFI record).
- **A transaction record and a wallet carry what core decides about them;
  the exports that took them back to decide are removed**
  (`store/persistence_models.rs`, `store/wallet_domain.rs`,
  `service/transaction_actions.rs`)
  - A served `TransactionRecord` carries `direction` and `submitted`, set
    beside `actions` (before: the `transaction_amount_direction` and
    `transaction_kind_is_submitted` exports, called on every record).
  - A served wallet's `derivation_profile` names the profile and account of
    its path (before: the `derivation_profile_of_path` export).
  - The history page's sent and received filters leave out a transfer of
    nothing, as `direction` does (before: they listed it by kind alone).
- **A record's history source is typed: `HistorySource::Internal` or
  `Indexer { name }`** (`TransactionRecord.history_source`, before
  `transaction_history_source`, a free string)
  - Every EVM row, native or token, names the Blockscout indexer it was read
    from (before: `etherscan` for native rows and `rust/etherscan`, shown as
    is, for token rows).
  - The `history_source` export, its `ChainProviders` case and the aliases no
    producer wrote (`rpc`, `rust.hd`, `blockchair`, `esplora` and others) are
    removed; the history search matches an indexer's name.
  - The CLI's `txs` reports `historySource` in core's form (before: its own
    `provider`, `chainProviders` or `internal`).
- **Only the user's tokens are stored; the built-in catalog joins them each
  time state is built** (`store/state.rs` `with_custom_tokens`;
  `wallet_db/state.rs`)
  - The `token_preferences` row holds the custom tokens' deployment entries
    (before: every catalog token as a preference entry, rewritten by
    `open_state` on every launch).
  - `ResidentState::default()` carries the catalog's tokens, so a service
    that has not opened a database has them too.
  - `StateCommand::MergeBuiltInTokens` and `merge_built_in_token_preferences`
    are removed.
- **A wallet is one record, `WalletState`, stored and served alike**
  (`store/state.rs`; `store/wallet_domain.rs` `WalletView` removed)
  - A wallet has one `address: Option<String>` (before: a list of
    `WalletAddress` rows, each with a kind that was always `receive` and a
    copy of the wallet's path, and a slot-keyed map in `WalletView`);
    `address_on` answers for every network sharing the wallet's address slot.
  - Discovery, send identity, Zcash shielded keys, UTXO sources and the
    keypool read the wallet's own `derivation_path` (before: three of them
    each fell back from the address row's copy differently).
  - `WalletState::served` fills each holding's id when core hands a wallet to
    a front end (before: the `to_wallet_view`/`to_wallet_state`
    conversions).
  - Import builds `WalletState` directly, and `WalletImportOutcome` and
    `PortfolioSnapshot` carry it.
  - `ChainEntry.address_slot` is removed.
  - A stored wallet row in the earlier shape (`addresses`) does not load.
  - The CLI's wallet report drops `addresses`; `address` carries the one.
- **Aptos and Tron sign and submit the bytes they built; the hand-written
  JSON beside them is removed** (`send/aptos.rs`, `send/tron.rs`,
  `send/payload.rs`)
  - `PreparedAptosTransfer` keeps its sequence, gas and expiration as fields
    and signs to SignedTransaction BCS, submitted and simulated as BCS
    (before: a JSON body, submitted as JSON and read back by field name by
    the send stages, staking and reservations).
  - `PreparedTronTransfer` keeps its `RawReference` and signs to a protobuf
    `Transaction`, submitted through `/wallet/broadcasthex`
    (`TronHttpClient::broadcast_signed`; before: a JSON body through
    `/wallet/broadcasttransaction`); a TRC-10 send is checked by encoding the
    reviewed transfer again under the same reference.
  - A Tron multisig session is submitted as protobuf with its gathered
    signatures; TronWeb's JSON is written only for the signers to exchange
    (`tron::encode_with_json`).
  - `SignedPayload::Json` is removed: a signed ICP, Kaspa, Aptos or Tron
    transaction is `Hex` (or `Kaspa`).
- **Core decides a new wallet's phrase length and scan start, a chain
  search's match and a custom token's decimals** (`validation`,
  `derivation/phrase.rs`, `catalog/chains.rs`, `store/state.rs`; the CLI's
  `wallet new`, `chains --filter` and `token add`/`edit`)
  - `created_phrase_lengths` lists the lengths a created phrase may have,
    and `generate_seed_phrase` takes the shortest when given none (before:
    each front end read the setup descriptor's format and filtered the
    lengths itself).
  - `WalletImportCommit.created` starts a created scanning wallet's scan at
    the current height (before: each front end asked
    `new_wallet_restore_height`, now no longer exported).
  - `chain_matches_search` matches a chain's name, symbol, search keywords
    and tags; the CLI's filter reads it (before: name and symbol only).
  - `AddCustomToken` and `UpdateCustomToken` take `decimals: Option<u32>`:
    the protocol's fixed places, else the ones given, else
    `TokenPreferenceRejection::DecimalsRequired` (before: the CLI looked up
    the chain's standards to pick a number, sending 0 as a placeholder).
  - The CLI's `token add` and `edit` exit 3, rejected, without the places
    such a token needs (before: 2, usage).
- **A staged transaction's status is core's: `SendProgress.status`**
  (`service/send_stages.rs` `SendStatus`): awaiting signature, signed,
  rejected, uncertain, awaiting confirmation, confirmed or failed, read from
  its record, its stage and the nodes' answers in that order (before: each
  front-end page worked it out).
- Tests: 8 new, 21 removed
  - EVM transfer call 2: a typed decimal rounded or refused differently from
    the build, and a token sent as value or the coin as a contract call.
  - History filter 1: a transfer of nothing listed as sent or received.
  - Stored tokens 1: catalog tokens written to the database or a custom one
    lost on reopening.
  - Served wallet 1: a holding without its id, a path without its profile,
    or an address answered for another family's network.
  - Token decimals 1: places guessed for a standard that fixes none.
  - Chain search 1: a search that misses a keyword or tag.
  - Send status 1: a status read other than from the record, then the stage,
    then the nodes' answers.
  - The removed tests checked the EVM assembly export, the history-source
    parser, the wallet model conversions, a fallback between one wallet's
    network addresses and the built-in token merge command.
- Docs: `ARCHITECTURE.md` "Core module boundaries" (`api/`); `FFI-BOUNDARY.md`
  "Callers and coverage".
- Total: 159 .rs files changed (2 new, 2 deleted), +2142 −2721 lines; data and
  config files: `scripts/test-source-scan.py`, `tests/acceptance/harness.py`,
  `tests/acceptance/multisig_tron.py`, `tests/acceptance/staking.py`,
  `tests/acceptance/trc10.py`; dependencies unchanged; `#[test]` functions
  1572 → 1559, `cargo test` 1559 passing.

## Beta Commit 252

- **A phrase wallet's derivation overrides are sealed beside its phrase; the
  wallet record keeps only which ones it has** (`core/src/secrets/wallet.rs`
  `load_mnemonic`, `Mnemonic`; `store::wallet_domain::WalletDerivationFlags`;
  `WalletState.derivation_flags` replaces `derivation_overrides`; see
  `ARCHITECTURE.md` "Ownership")
  - A BIP-39 passphrase or custom HMAC key is its own blob in the seed bucket,
    under the phrase's password and salt, and comes back with the phrase from
    one key derivation (before: carried in the clear on the wallet record, in
    SQLite and in the state the front ends read).
  - `WalletDerivationFlags` (`passphrase`, `hmac_key`, `uniform_expansion`)
    answers which networks a wallet may go to, Zcash's refusal of an HMAC
    override and which Polkadot keys it exports, without unlocking the wallet.
  - `uniform_expansion` reads the HMAC override trimmed, as Polkadot's
    derivation does (before: the key-export check compared it exactly, so
    `uniform` with surrounding whitespace was not read as the mode).
  - A wallet row in the earlier shape (`derivationOverrides`) does not load:
    `WalletState` refuses unknown fields.
  - The `parse_wallet_derivation_input` export and `WalletDerivationInput` are
    removed (`core/src/derivation/input.rs` deleted); import reads a blank
    override as none (`WalletDerivationOverrides::normalized`).
- **The password verifier is removed; one key derivation seals and opens all of
  a wallet's material** (`core/src/secrets/password_verifier.rs` deleted;
  `secrets::wallet::write_material`, `read_material`)
  - Every blob, the salt and scan key included, passes through the device seal
    on the way to the store (before: only the seed and private key), and that
    seal tells a wrong password from changed data.
  - An envelope that does not open under the device key is
    `WalletSecretError::Corrupt` rather than a backend error
    (`secrets/device_key.rs`).
  - Whether a wallet is sealed is read from its salt (before: from its
    verifier).
  - The Monero view key and the Litecoin MWEB scan secret are kept through
    `store_scan_key` and `load_scan_key`, device-sealed (before: written as
    plain text to the generic bucket; `service/monero_wallet.rs`,
    `litecoin_mweb.rs`, `wallet_import.rs`).
- **A wallet password shorter than 4 characters is refused when sealing, not
  only by the form** (`validation::wallet_password_is_too_short`,
  `WalletSecretError::PasswordTooShort`; `service/wallet_import.rs`): import
  refuses it before anything is planned or stored (before: only a blank password
  was refused there).
- **The CLI exits with core's classification of a `WalletSecretError`**
  (`cli/src/error.rs`): a password too short, required or not required exits
  3, rejected (before: 1, failure, from a mapping of the CLI's own).
- **Provider amounts and block numbers that do not parse are refused instead of
  read as 0** (`api/blockscout.rs`, `icp_rosetta.rs`, `kaspa_rest.rs`,
  `koios.rs`, `toncenter_v2.rs`; Solana token balances in
  `service/network_tokens.rs`)
  - Blockscout gas price, gas used, block number and log index; ICP Rosetta
    balance, block index, operation amount and fee; Kaspa UTXO amounts; Koios
    lovelace values and fees; TON balance, fees and message values; a Solana
    token account's balance.
  - An ICP Rosetta operation other than a transaction, mint, burn or fee (an
    approval) is skipped, as it states no amount.
- **A TRC-20 transfer of more than `u64::MAX` base units builds**
  (`service/send_stage_protocols.rs` `protocol_u64`; `send_stage_tron.rs`): each
  protocol narrows the amount to its own wire type (before: one `u64` was taken
  for every protocol but NEAR, Polkadot and TON and XRP tokens, so a larger
  TRC-20 amount was refused as out of range).
- **Prepared-transaction fields keep integers past `u64` exactly**
  (`send/prepared_fields.rs`, read through `serde_json::value::RawValue`): a
  yoctoNEAR or wei amount is shown as written (before: read as a float).
- **Destination warnings no longer guess a network from an address's first
  letters** (`send/flow.rs`; `cli/src/cmd/send/mod.rs`):
  `HighRiskSendWarning::NonEvmOnEvm`, `EthOnUtxo` and `ForeignAddressFormat` and
  `Chain::flags_evm_address_as_wrong_chain` are removed, another network's
  address raises only the chain address rule's `InvalidFormat`, and the
  ENS-off-Ethereum check remains (before: an ENS name beginning `a`, `d` or `t`
  on an EVM chain, or a Solana address beginning `T`, raised a warning).
- **`StateCommand` no longer carries a whole wallet or the whole state**
  (`store/state.rs`): `ReplaceState`, `UpsertWallet` and `UpdateWalletIfPresent`
  are removed from the export; import stores its wallets through
  `store::state::upsert_wallet_in_place`, and tests through the test-only
  `WalletService::upsert_wallet`.
- **`spectra balance`, `history`, `pool discover` and `send destination` read a
  network known only by a custom endpoint** (`cli/src/cmd/chain.rs`,
  `address_pool.rs`, `send/recipient.rs`): `service_for_chain`, which refused a
  network with no compatible catalog endpoint before core was asked, is
  removed.
- **`docs/CHANGELOG.md` records what each commit changed, newest first, from the
  Initial Commit on**, written from the git history; `AGENTS.md` gains a
  "CHANGELOG" section on how it is written and tidied before a commit, and its
  Git rule points there.
- Tests: 9 new, 11 removed
  - Sealed material 4: a passphrase left in the stored wallet row or in the
    state a front end reads; overrides not coming back with the phrase, or left
    behind when the phrase is stored again without them; `uniform` not flagged
    as Polkadot's expansion mode; a scan key stored in the clear.
  - Provider amounts 3: a malformed ICP Rosetta, Koios or TON amount or fee read
    as 0.
  - Wide amounts 2: a TRC-20 amount past `u64` refused or cut; a prepared
    field's integer past `u64` read as a float.
  - Removed: the verifier's 8 (`password_verifier.rs`),
    `wallet_update_if_present.rs`'s 2, and `an_empty_password_is_refused`
    (`secrets/wallet.rs`), folded with the blank-password test into
    `a_short_password_stores_nothing_rather_than_storing_unsealed`, which also
    refuses a short password.
  - Changed guards: `a_short_password_is_refused_rather_than_stored`
    (`store/tests/wallet_import.rs`) also guards against `abc` being stored;
    `derivation/input.rs`'s test, moved to `store/wallet_domain.rs` as
    `overrides_keep_their_text_and_a_blank_is_none`, also against a blank
    override read as one; the EVM destination test in `send/mod.rs`, now
    `destination_warnings_do_not_guess_a_network_from_letters`, against an ENS
    name beginning `a`, `d` or `t`, or a Solana address beginning `T`, raising
    a warning (before: a Bitcoin address on an EVM chain raising none).
  - Acceptance: a 3-character password refused (`tests/acceptance/__main__.py`);
    a balance read from an endpoint added for a network the catalog lists none
    for (`tests/acceptance/endpoints.py`).
- Docs: `CHANGELOG.md` (new); `AGENTS.md` "Git", docs table, "Keep them true",
  "CHANGELOG" (new); `ARCHITECTURE.md` "Ownership" (`SecretStore` paragraph:
  overrides sealed beside the phrase) and "Core module boundaries"
  (`wallet_db::scan_cache`: scan key device-sealed); `tests/README.md`
  `endpoints.py` row.
- Total: 97 .rs files changed (3 deleted), +1423 −1643 lines; data and config
  files: `core/Cargo.toml`, `tests/acceptance/__main__.py`,
  `tests/acceptance/endpoints.py`; dependencies: none added or removed,
  `serde_json` gains the `raw_value` feature (`core/Cargo.toml`); `#[test]`
  functions 1574 → 1572, `cargo test` 1572 passing.

## Beta Commit 251

- **`make apple` writes the SpectraCore package to `target/SpectraCore/` for a
  developer to copy into spectra-apple** (`Makefile` `apple`;
  `scripts/build-apple-package.sh`): it passes `IPHONEOS_DEPLOYMENT_TARGET` and
  `MACOSX_DEPLOYMENT_TARGET` (26.0 unless given) and `PROFILE`/`PLATFORMS` to
  the script (before: spectra-apple's `make core` ran the script against a
  checkout of this repository beside it).
- Docs: `AGENTS.md` "Repositories" (an FFI change is checked by copying the
  package into spectra-apple, with this checkout as its `SPECTRA`) and the
  verification table; `README.md` intro; `ARCHITECTURE.md` intro and the Apple
  package paragraph; `scripts/README.md` `make apple` and the
  `build-apple-package.sh` row (an unchanged FFI recompiles no app Swift only
  when the copy keeps modification times).
- Total: no .rs files changed; data and config files: `Makefile`,
  `scripts/build-apple-package.sh`; dependencies unchanged; `#[test]`
  functions unchanged at 1574.

## Beta Commit 250

- **Core builds as the SpectraCore Swift package an Apple app links** (new
  `scripts/build-apple-package.sh`; see `ARCHITECTURE.md`)
  - A static library for the iPhone, the arm64 simulator and the Mac, bundled as
    `spectra_core.xcframework`, with the FFI's C header as the `spectra_coreFFI`
    module and the UniFFI Swift bindings as the `SpectraCore` target; it builds
    in `target/apple/`, apart from the workspace's `target/`.
  - It takes the app's minimum iOS and macOS versions (each must be one
    version), `PROFILE` `dev` or `release` and `PLATFORMS` (a subset of `ios
    ios-sim macos`); a run that finds nothing changed leaves the package as it
    was, and the header and bindings are rewritten only when their text changed.
  - spectra-apple builds against a checkout of this repository beside it, and
    its `make core` runs the script (before: it took this repository as its
    `spectra` git submodule).
- **The CI workflow is deleted** (`.github/workflows/ci.yml`): the four targets
  are run by hand; `AGENTS.md`, `README.md`, the `Makefile` and
  `rust-toolchain.toml` no longer mention CI.
- Docs: `AGENTS.md` "Repositories" and verification table; `README.md` intro and
  CI badge removed; `ARCHITECTURE.md` intro and the Apple build paragraph;
  `FFI-BOUNDARY.md` intro and scans; `scripts/README.md` "Apple package" (new)
  and svgo 4.1.0; `tests/README.md` generators note.
- Total: no .rs files changed; data and config files:
  `.github/workflows/ci.yml`, `Makefile`, `rust-toolchain.toml`,
  `scripts/build-apple-package.sh`; dependencies unchanged; `#[test]`
  functions unchanged at 1574.

## Split repo

- **This repository holds the Rust core, the CLI and the crypto icon library;
  the iOS/macOS and Android apps have their own repositories** (`AGENTS.md`
  "Repositories"; `README.md`; `ARCHITECTURE.md` module table)
  - spectra-apple takes this repository as its `spectra` git submodule;
    spectra-android is not started. An FFI change is checked with
    spectra-apple's checks and the change in its `spectra/` checkout.
  - The Swift conventions, resources rules and app-icon export leave
    `AGENTS.md`, and the app's build section leaves `README.md`.
  - `.gitignore` drops the Xcode and `swift/generated/` entries.
  - `README.md`'s CI badge points at the new repository.
- **Four verification targets: `make lint`, `test`, `test-cli` and
  `check-icons`** (`Makefile`; `.github/workflows/ci.yml`)
  - `check-icons` runs `scripts/normalize-icons.sh --check` (before: `check-ui`,
    with the design-token check).
  - `test-ios`, `test-mac` and the `ios`, `iosr`, `mac`, `macr` builds are
    gone, as is `clean`'s removal of `swift/generated/`.
  - CI runs all four; its UI job is now "Icon library".
- **`make lint` runs one source scan, `uncalled-core-fns.py`, on Rust alone**
  (`scripts/source_scan.py`, `scripts/uncalled-core-fns.py`): a Swift mention no
  longer counts as a caller, and the scan skips core's UniFFI exports, which
  `source_scan.exports` lists from the Rust source; the export, string and
  Swift-literal scans are spectra-apple's.
- Docs: `AGENTS.md` (Repositories new, docs table, Architecture, Funds and keys,
  Verification, Platform constraints, Icons); `README.md`; `ARCHITECTURE.md`
  intro, module table and the Apple build paragraph; `FFI-BOUNDARY.md` intro,
  string tables and scans ("Regenerating bindings" and the Swift rows of
  "Common errors" removed, now spectra-apple's `docs/SWIFT-BINDINGS.md`);
  `OPEN-ITEMS.md` (the two Mac app items leave); `scripts/README.md`;
  `tests/README.md` (`swift/tests/` row removed); `scripts/normalize-icons.sh`
  comment.
- Total: no .rs files changed; data and config files: `scripts/` (4),
  `.github/workflows/` (1), `.gitignore`, `Cargo.toml`, `Makefile`,
  `rust-toolchain.toml`; dependencies unchanged; `#[test]` functions unchanged
  at 1574.

## Beta Commit 249

- **`make verify` is removed; the six verification targets are run by name**
  (`Makefile`; `AGENTS.md` "Verification"): `lint`, `test`, `test-cli`,
  `check-ui`, `test-ios` and `test-mac`, and the AGENTS.md table says which a
  change needs.
- Docs: `AGENTS.md` "Funds and keys" and "Verification"; `README.md` targets;
  `OPEN-ITEMS.md` endpoint-audit item; `scripts/README.md`; `tests/README.md`
  `monero_regtest.py` row and the suite's docstring.
- Total: no .rs files changed; data and config files: `Makefile`,
  `tests/acceptance/monero_regtest.py`; dependencies unchanged; `#[test]`
  functions unchanged at 1574.

## Beta Commit 248

- **Make targets for the macOS build of the app** (`Makefile` `mac`, `macr`,
  `test-mac`; the app's sources are not in this repository)
  - `test-mac` runs the app's tests on this Mac (`platform=macOS,arch=arm64`)
    and needs the app signed with its keychain access group; `make verify`
    includes it.
  - The Rust build script the docs and comments name is
    `scripts/build-apple.sh`, for the device, the arm64 simulator or the Mac
    (before: `scripts/build-ios.sh`; `Cargo.toml`, `rust-toolchain.toml`).
- **Two open items for the Mac app** (`docs/OPEN-ITEMS.md`): walk its flows by
  hand, and review its default-styled buttons.
- Docs: `AGENTS.md` docs table, Verification, Platform constraints (iOS 26 and
  macOS 26 from one target, `PlatformChrome.swift`); `README.md` "The iOS and
  macOS app" (sandbox, data protection keychain, signing) and targets;
  `ARCHITECTURE.md` intro, module table and the Mac app paragraph;
  `FFI-BOUNDARY.md` regenerating bindings; `OPEN-ITEMS.md`; `scripts/README.md`
  build and icon rows; `tests/README.md`.
- Total: no .rs files changed; data and config files:
  `.github/workflows/ci.yml`, `Cargo.toml`, `Makefile`, `rust-toolchain.toml`;
  dependencies unchanged; `#[test]` functions unchanged at 1574.

## Beta Commit 247

- **The MIR record-field scan is removed**
  (`scripts/unwritten-record-fields.py`, `record_writers.py`,
  `test-record-writers.py`, `emit-mir.sh` deleted; `Cargo.toml`
  `[profile.mir-scan]` removed): `make lint`'s only build is clippy's.
- **Debug and test builds optimize only the dependencies run time goes to**
  (`Cargo.toml`)
  - 38 named crates (key stretching, hashing, curve arithmetic, Zcash's proofs)
    at opt-level 3 and `libsqlite3-sys` at 1; every other dependency unoptimized
    and without debug info (before: every dependency at opt-level 3). Proc
    macros and build scripts build at opt-level 1.
  - `default-members` makes a bare `cargo build`, `test` and `clippy` select
    core, the CLI and the two fixtures, leaving out the bindgen; `make lint` and
    `make test` drop `--workspace`, and `tests/acceptance` builds
    `--all-targets` to share `cargo test`'s dependency build.
  - The bindgen binary is named `spectra_uniffi_bindgen`; the bindgen and
    fixtures build no test target, and core no doc tests.
- **An endpoint health retry does not wait under test**
  (`core/src/api/health.rs`; before: 600 ms there too).
- **`make test-ios` boots the first available iPhone simulator by UDID before
  testing and leaves it running** (`Makefile` `IOS_TEST_SIM` replaces
  `IOS_TEST_DEST`).
- Docs: `AGENTS.md` Verification (a row for core behaviour or persistence
  without funds, keys or FFI: `make lint test`, plus `test-cli`; `cargo test
  <filter>`); `README.md` targets table; `FFI-BOUNDARY.md` scans;
  `scripts/README.md`.
- Total: 1 .rs file changed, +3 −1 lines; data and config files: `scripts/`
  (4), `tests/acceptance/` (2), `.github/workflows/` (1), `Cargo.lock`,
  `Cargo.toml`, `Makefile`, `core/` (1), `tools/litecoin-mweb-fixture/` (1),
  `tools/uniffi-bindgen/` (1), `tools/zcash-fixture/` (1); dependencies
  unchanged; `#[test]` functions unchanged at 1574.

## Beta Commit 246

- **`spectra` alone at a terminal opens windows that run the CLI's own
  commands** (new `cli/src/tui/`: `app`, `complete`, `editor`, `hints`, `job`,
  `keys`, `layout`, `view`, `window`; `cli/src/main.rs`; see `ARCHITECTURE.md`
  and `README.md`)
  - Tabs and windows in kitty's model, in tall, fat, stack and grid layouts,
    with kitty's key table behind the `Ctrl+B` leader; `Ctrl+B` then `?` lists
    the keys.
  - Each window is a prompt that runs what is typed as a separate `spectra`
    process in a pseudo-terminal, with its own hidden password prompt, colour
    and exit code; the interface adds no operation and holds no domain state.
  - `Tab` completes commands, flags and positionals read from the clap
    definitions; the prompt has history and line editing; a drag, a double click
    or a labelled hint copies text, an address or a URL.
  - Without a command, when stdin or stdout is not a terminal, with `--json`, or
    inside one of its own windows, `spectra` exits with a usage error that says
    a command is required.
- **LEB128 reads and every plain BLAKE2b go through core's one implementation**
  (`send/leb128.rs` `read`; `derivation::primitives::blake2b`; `send/sui.rs`,
  `sui_multisig.rs`, `tron.rs`, `tron_multisig.rs`, `message.rs`,
  `message_schemes.rs`, `zcash_shielded.rs`, `derivation/sui.rs`,
  `sui_multisig.rs`): `leb128::read` refuses a number that ends early or passes
  64 bits; the call sites' own `blake2b_simd` hashing is removed.
- **CI runs `make check-ui` with svgo 4.1.0, and `make verify` includes it**
  (`.github/workflows/ci.yml`, `Makefile`).
- Tests: 92 new
  - Windowed interface 89: a key after the leader typed into the prompt, a
    closed window's output drawn or a hidden tab's not marked, completion
    offering a flag already given or failing on an unknown word, an open quote
    or a pasted newline running something, the cursor drifting from wide text, a
    layout overlapping or leaving its area, a terminal too small panicking,
    scrollback cutting lines, hints repeating labels or offering prose and
    abbreviated hashes.
  - LEB128 3: a number that ends early or passes 64 bits read silently.
  - Acceptance: `tests/acceptance/tui.py` drives the binary at a
    pseudo-terminal.
- Docs: `AGENTS.md` reorganized (Rule 0's limits, "Docs" table, "Funds and
  keys", Verification); `README.md` (running the CLI, links, five targets);
  `ARCHITECTURE.md` intro, module table, the interface paragraph, module
  boundaries, error enums, `api` dependencies, `wallet_actions`, `wallet_db`,
  asset presentation; `CHAIN-SUPPORT.md` (Peercoin account-key versions, Bitcoin
  Cash REST v2); `FFI-BOUNDARY.md`; `OPEN-ITEMS.md` intro; `scripts/README.md`;
  `tests/README.md` (where tests live, `tui.py`).
- Total: 24 .rs files changed (10 new), +3884 −127 lines; data and config
  files: `tests/acceptance/` (2), `.github/workflows/` (1), `Cargo.lock`,
  `Makefile`, `cli/` (1), `scripts/` (1); dependencies: added base64 0.23.1,
  portable-pty 0.9.0, ratatui 0.30.2, unicode-width 0.2.3, vt100 0.18.1
  (`cli/Cargo.toml`); `#[test]` functions 1482 → 1574.

## Beta Commit 245

- **A staged transaction's review is one ordered list of `ReviewWarning`s and
  one `NetworkFee`** (`send/stages.rs`; `service/send_stages.rs`; see
  `ARCHITECTURE.md`)
  - `ReviewWarning` is a risky-send reason, an EVM recipient or contract
    finding, a self-send, an asset's transfer fee, the coin a token send
    carries, a recipient registration deposit or a transfer hook (before:
    `warnings`, `recipient_warnings` and `requires_self_send_confirmation`).
  - `NetworkFee` is an exact amount, its display value and a `FeeBound`: the
    quote's estimate, exactly what is paid, or the most it can cost (before:
    `network_fee` and `network_fee_value`; staking's `fee_is_upper_bound`).
  - A wallet operation's fee is read from and validated against its prepared
    payload; the operations' own fee fields are removed (`service/wallet_*.rs`).
- **`send_progress` says what may be done next with a staged transaction**
  (`SendProgress`, `SendNextStep`): sign, broadcast, rebroadcast or done, and
  whether a rebroadcast, a staking repair or a status recheck applies, from the
  artifact and its history record read together (before: the app derived the
  step; staking's `repair_allowed` is removed).
- **An empty amount is quoted for a tenth of the holding**
  (`service/owned_send.rs`): the quote's `amount` says what it priced (before:
  the app picked the 10%).
- **Solana, Sui and Aptos share one `short_id` for an unnamed validator or
  pool** (`staking/mod.rs`): eight characters of the id after any `0x` (before:
  a copy per network, Sui's and Aptos's cutting at ten characters whatever the
  id began with).
- Tests: 9 new
  - Send progress 6: a stage with more than one next step, a history record
    speaking for an unsubmitted rebuild or another artifact, a rebroadcast
    without a submission or its record's consent, a repair offered for anything
    but an interrupted ICP stake, a recheck offered for a settled staking
    submission.
  - Review 1: an asset's transfer terms missing from the warnings.
  - Provisional quote 1: an empty amount quoted for anything but a tenth.
  - Validator names 1: an id cut at the wrong length.
- Docs: `ARCHITECTURE.md` (provisional quote, review list and fee,
  `send_progress`); `OPEN-ITEMS.md` ("Technical debt" and its iOS items
  removed).
- Total: 37 .rs files changed (1 new), +1008 −455 lines; data and config
  files: `tests/acceptance/litecoin_mweb.py`, `tests/acceptance/staking.py`,
  `tests/acceptance/zcash_shielded.py`; dependencies unchanged; `#[test]`
  functions 1473 → 1482.

## Beta Commit 244

- **Every CLI command ends in one report, printed as text or as one `--json`
  document** (`cli/src/out.rs` `Report`, `Done`, `fail`, `progress`; every
  `cli/src/cmd/` module)
  - A command's text and its JSON come from one `Serialize` value, and progress
    goes to stderr (before: about fifteen commands printed nothing in text mode
    and seven printed Rust `{:?}`).
  - A command about one wallet takes it as its first argument, and `--wallet`
    only narrows a command over many (before: positional, `--wallet` or `--from`
    by command); `spectra balance <wallet>` is one.
  - `cmd/wallet.rs` is split into `cmd/wallet/` (`account`, `change`, `create`,
    `private`, `secrets`, `setup`, `show`) and `cmd/tx.rs` into `cmd/send/`
    (`offline`, `owned`, `recipient`, `stages`) and `cmd/txs.rs`.
- **CLI acceptance runs on one Python harness** (`tests/acceptance/`:
  `__main__.py`, `harness.py` and one module per suite;
  `tests/cli-acceptance.sh`, `cli-assertions.sh` and `test-cli-assertions.sh`
  deleted)
  - `make test-cli` is `python3 -B tests/acceptance`; the shell harness's binary
    checks run in `__main__.py`, and the suites share one CLI runner, loopback
    server and fixture loading.
  - `cli-monero-regtest.py` is `tests/acceptance/monero_regtest.py`.
- **The SDK vector generators live in `tests/vectors/` with one `package.json`**
  (`tests/vectors/package.json`, `package-lock.json`; `.gitignore`
  `tests/vectors/node_modules/`): fixture provenance and core test comments name
  the new paths.
- **The source scans are Python scripts on `scripts/source_scan.py`**
  (`scripts/uncalled-core-fns.py`, `unwritten-record-fields.py` replace their
  `.sh` heredocs): the `Makefile` runs each as `python3 -B scripts/<scan>.py`.
- Docs: `AGENTS.md` and `README.md` `test-cli`; `ARCHITECTURE.md` module table,
  the CLI report paragraph and test locations; `FFI-BOUNDARY.md` scan names;
  `OPEN-ITEMS.md` ("CLI and tests" items removed); `scripts/README.md`;
  `tests/README.md`.
- Total: 51 .rs files changed (14 new, 2 deleted), +8047 −5629 lines; data and
  config files: `tests/vectors/` (46), `tests/acceptance/` (28), `core/tests/`
  (23), `tests/` (22), `scripts/` (8), `.github/workflows/` (1), `.gitignore`,
  `Cargo.lock`, `Cargo.toml`, `Makefile`, `cli/` (1),
  `tools/litecoin-mweb-fixture/` (1), `tools/zcash-fixture/` (1);
  dependencies: added serde 1 (`cli/Cargo.toml`); `#[test]` functions
  unchanged at 1473.

## Beta Commit 243

- **One implementation per primitive** (new `derivation/bip32.rs` and
  `send/leb128.rs`; `derivation::primitives::blake2b`; rust-bitcoin's `VarInt`;
  `api::evm_json_rpc::abi_u128`; see `ARCHITECTURE.md`)
  - `derivation/bip32.rs` reads every derivation path and holds the secp256k1
    extended keys on rust-bitcoin's, replacing the BIP-32 code in
    `derivation/bitcoin.rs` and `primitives.rs`: `'`, `h` and `H` are one
    hardened step, `m/` is optional, and an index at or past 2^31 is refused
    rather than wrapped.
  - The `blake2` crate is dropped; LEB128 numbers, CompactSize and uint256 ABI
    words each have one writer or reader, the last `abi_u128` for EVM's words
    and Tron's alike.
- **A history row is its `TransactionRecord`** (`wallet_db/history.rs`;
  `HistoryRecord` removed): the indexed columns (lowercased id, wallet and hash,
  network, sort key) are derived from the record when it is written.
- **A send keeps its signed transaction only in its send record**
  (`service/send_records.rs`): `TransactionRecord.signed_transaction_payload`
  and `signed_transaction_payload_format` are removed, and a rebroadcast submits
  the send record's submission (before: it fell back to the history record's
  copy).
- **Every UTXO network merges provider history by Dogecoin's rule**
  (`store/transaction_merge.rs` `TransactionMergeStrategy::Utxo`): a provider
  row matches a stored one only under the same, present wallet id (before: the
  other UTXO networks also matched two rows without one).
- **Wallet and address-book tables drop columns nothing read**
  (`wallet_db/connection.rs`, `wallets.rs`): `wallets.chain_id`, `is_watch_only`
  and `include_in_portfolio_total`, `address_book.chain_id` and `address`, and
  their indexes.
- **Diagnostics runs belong to the service** (`diagnostics::DiagnosticRuns`,
  `WalletService.diagnostic_runs`; before: a process-wide registry).
- **Removed:** the `apply_transaction_command` export and `TransactionCommand`,
  and `seed_derivation_chain_raw`, which always answered.
- **Grab-bag modules dispersed beside their callers** (`service/helpers.rs`,
  `service/types.rs`, `service/standalone.rs` deleted; `store/mod.rs` into
  `store/status_tracker.rs`, `store/price_alerts.rs`, `service/keypool.rs`,
  `service/endpoint_directory.rs`); behaviour unchanged.
- Tests: 3 new, 5 removed
  - BIP-32 2: a path that reads differently by spelling, an account key that
    does not round-trip in its own version.
  - LEB128 1: an encoding that is not seven bits a byte.
  - Removed: four transaction-store command tests and
    `testnet_derivation_metadata_names_the_concrete_network`, with what they
    tested.
- Docs: `ARCHITECTURE.md` (one implementation per primitive, history rows, send
  records); `FFI-BOUNDARY.md` timestamps; `OPEN-ITEMS.md` (duplicated
  primitives, grab-bag modules, storage leftovers, dead paths removed).
- Total: 143 .rs files changed (3 new, 3 deleted), +2503 −3505 lines; data and
  config files: `Cargo.lock`, `core/Cargo.toml`; dependencies: removed blake2
  (`core/Cargo.toml`); `#[test]` functions 1475 → 1473.

## Beta Commit 242

- **The multisig read models live in `api/`** (`api/evm_safe.rs` `SafeState`,
  `api/tron_http.rs` `TronPermissions`, `api/xrpl_json_rpc.rs` `XrpSignerList`,
  `api/horizon.rs` `StellarSigners`; Solana's `TOKEN_PROGRAMS` in
  `api/solana_json_rpc.rs`): `send` builds against them and `api` no longer
  depends on `send`; behaviour unchanged.
- **CLI acceptance starts every loopback suite at once**
  (`tests/cli-acceptance.sh`): each owns its data directory and nodes, and its
  output and exit status are reported in order after the binary checks (before:
  one after another).
- **Debug builds keep line tables only** (`Cargo.toml` `[profile.dev] debug =
  "line-tables-only"`; before: full debug info, which its comment says doubled
  core's compile time and grew the iOS static library to 1.8 GB).
- **The record-field scan emits MIR through a workspace wrapper in a profile of
  its own** (new `scripts/emit-mir.sh`; `scripts/unwritten-record-fields.sh`;
  `Cargo.toml` `[profile.mir-scan]`): emitting MIR no longer invalidates the dev
  build.
- Docs: `ARCHITECTURE.md` (`api` defines the read models it returns);
  `FFI-BOUNDARY.md` and `scripts/README.md` (`build-ios.sh` rewrites a binding
  only when its text changed; the script is not in this repository's history),
  `emit-mir.sh`; `OPEN-ITEMS.md` ("`api` depends on `send`" removed);
  `tests/README.md` `cli-acceptance.sh` row.
- Total: 16 .rs files changed, +346 −345 lines; data and config files:
  `Cargo.toml`, `scripts/emit-mir.sh`, `scripts/unwritten-record-fields.sh`,
  `tests/cli-acceptance.sh`; dependencies unchanged; `#[test]` functions
  unchanged at 1475.

## Beta Commit 241

- **One reader of a token's decimals** (`service/network_tokens.rs`
  `token_decimals`; `send_execution::token_contract_decimals` removed): a send
  asks it `MetadataRead::Fresh`, while a balance read may reuse TRC-20 metadata
  other balance reads cached.
- **A UTXO status recheck reads and writes through the pending-status poll**
  (`service/transaction_recheck.rs`, `pending_status.rs`): a pending answer
  resumes polling (before: its own copy of the read and write, with its own
  confirmation handling).
- **`wallet_actions::offers` is the one gate for what a wallet does**
  (`service/wallet_actions.rs` `WalletAction::ALL`, `action_wallet`): each
  account operation behind the page fetches its wallet through `action_wallet`,
  which asks again (before: four `*_owner` helpers and `near_account`, each
  checking the network itself).
- **Staking dispatches once on `registry::StakingModel`**
  (`registry/staking.rs`; `service/staking.rs`; `staking/service.rs` and its
  `StakingService` deleted): delegation (Solana, Sui, Aptos, NEAR), nomination
  pools (Polkadot) or neurons (ICP); `staking::fetch_validators` reads each
  network's validator directory.
- **`MultisigScheme::of` reads the registry's `multisig_policy_format`**
  (`service/multisig.rs`; before: a chain list of its own).
- Docs: `ARCHITECTURE.md` (token decimals, `offers`, staking); `OPEN-ITEMS.md`
  ("One fact, two readers" removed).
- Total: 31 .rs files changed (1 deleted), +686 −948 lines; data and config
  files: none; dependencies unchanged; `#[test]` functions unchanged at 1475.

## Beta Commit 240

- **A token standard is `registry::TokenStandard`, and a network's own coin has
  none** (`registry/tokens.rs`; `catalog/tokens.rs`; `store/wallet_domain.rs`;
  see `ARCHITECTURE.md`)
  - Fourteen standards, spelled by name (`"ERC-20"`) in the catalogs and the
    store; a holding's `token_standard` is `None` for the coin (before: the
    string `"Native"`).
  - `tokens.toml` and `testnet-tokens.toml` drop every `kind` line
    (`"native"`, `"token"`): a deployment with a `standard` and `contract` is
    a token; `catalog::tokens::TokenKind` is removed.
  - `catalog::tokens::deployment_id` is the one place a deployment id is spelled
    (`ethereum:native`, `ethereum:erc-20:0x…`).
  - `TokenStandard::fixed_decimals` replaces `fixed_token_decimals`;
    `token_standard_name` names a standard for a front end.
- Docs: `ARCHITECTURE.md` token standards; `OPEN-ITEMS.md` ("Token standards are
  strings" removed).
- Total: 44 .rs files changed, +698 −528 lines; data and config files:
  `core/data/testnet-tokens.toml`, `core/data/tokens.toml`,
  `tests/cli-send.py`; dependencies unchanged; `#[test]` functions unchanged
  at 1475.

## Beta Commit 239

- **`send_stage_protocols.rs` dispatches each account protocol to its own
  module** (new `service/send_stage_aptos.rs`, `cardano`, `evm`, `icp`,
  `solana`, `stellar`, `substrate`, `sui`, `ton`, `tron`, `xrp`; NEAR's in
  `send_near.rs`): preparation, revalidation, signing and expiry per protocol;
  behaviour unchanged.
- Docs: `ARCHITECTURE.md` persistent send stages; `OPEN-ITEMS.md` ("four chain
  ladders" removed).
- Total: 14 .rs files changed (11 new), +2323 −1636 lines; data and config
  files: none; dependencies unchanged; `#[test]` functions unchanged at 1475.

## Beta Commit 238

- **Network identity checks live in `api/`** (new `api/network.rs`
  `verify_network`; `service/endpoint_health.rs` moved to `api/health.rs`;
  `send_stage_protocols.rs`, `pending_status.rs`, `multisig_aptos.rs`,
  `diagnostics/self_tests.rs`)
  - Each client's own `verify_network` proves an endpoint is its network before
    core signs or submits through it; `verify_network` answers `false` where
    core cannot ask, and the send stages accept such an endpoint only from the
    catalog.
  - The raw RPC calls in `service/` (`isBlockhashValid`,
    `eth_getTransactionCount`, `eth_chainId`) are client methods.
- Docs: `ARCHITECTURE.md` (`api/network.rs`, `api/health.rs`); `OPEN-ITEMS.md`
  ("Network identity checks are rewritten per call site" removed).
- Total: 13 .rs files changed (1 new), +307 −205 lines; data and config files:
  none; dependencies unchanged; `#[test]` functions unchanged at 1475.

## Beta Commit 237

- **Every history adapter returns one `api::history_page::HistoryEntry`; nothing
  is serialized to be read back by field name** (new `api/history_page.rs`;
  `service/network_history.rs`; `fetch/history.rs` with `HistoryShape` and
  `RawHistoryPage` deleted)
  - Each adapter keeps its own units, timestamp scale and transfer direction;
    `network_history` gives each entry its asset identity as a
    `NormalizedHistoryItem`.
  - `fetch/` is gone as a layer: `refresh_engine`, `refresh_policy`,
    `history_store`, `history_decode` and `bitcoin_history` are in `service/`,
    the merge rules are `store/transaction_merge.rs`, and `price.rs` is in
    `api/`.
  - `diagnostics::aggregate` (`diagnostics_history_summary`) and
    `service::helpers::json_response` are removed.
- Tests: 2 new, 3 removed
  - History items 2: a local Monero transfer normalized differently from an
    adapter's row, an undated transaction given a time.
  - Removed: the JSON re-parse's and the history summary's tests, with the code.
- Docs: `AGENTS.md` (`api`'s callers); `ARCHITECTURE.md` (module paths, history
  adapters, `price`); `OPEN-ITEMS.md` ("History round-trips through a JSON
  string" removed).
- Total: 56 .rs files changed (1 new, 3 deleted), +1068 −1091 lines; data and
  config files: none; dependencies unchanged; `#[test]` functions 1476 → 1475.

## Beta Commit 236

- **Core's modules regrouped by domain** (moves only; see `ARCHITECTURE.md`
  "Core module boundaries")
  - `registry.rs` is `registry/` (`mod`, `keys`, `address`, `endpoints`,
    `tokens`, `send`, `staking`, `utxo`, `networks`, `evm`, tests in
    `registry/tests/`); the catalogs are `catalog/` (`chains`, `tokens`,
    `endpoints`, `explorers`, `donations`, `wiki`, `restore_heights`).
  - `amount/` holds `decimal`, `input` (from `send/amount_input.rs`) and
    `presentation` (from `formatting.rs`); `secrets/` holds `store`, `backends`,
    `device_key`, `kdf`, `password_verifier`, `seed_envelope` and `wallet` (from
    `store/wallet_secrets.rs`); `endpoint_api.rs` is `api/endpoint.rs`, `tor.rs`
    is `api/tor.rs`, `worker.rs` is `service/worker.rs`; `error.rs` holds
    `SpectraBridgeError` and `LocalizableMessage` (from `lib.rs`).
- **The `ffi` crate is removed; core itself is the app's static library**
  (`ffi/` deleted; workspace `Cargo.toml`).
- **A test network names its mainnet in `chains.toml`** (`mainnet = "bitcoin"`;
  every `family` and `environment` line removed; `catalog/chains.rs`): a mainnet
  writes nothing, and `list_all_chains` gives one `ChainEntry` per network
  carrying the `Chain` itself.
- **EVM membership is having an EIP-155 id** (`Chain::evm_chain_id`;
  `registry/evm.rs`).
- **Address validation takes the `Chain`**
  (`validation::address::validate_address(chain, value)`;
  `AddressValidationRequest` and its kind strings removed).
- **`execute_send` is no longer exported, and `spectra send broadcast` is
  removed** (`service/send_execution.rs`; `cli/src/cmd/tx.rs`): front ends send
  through the owned flow (`send owned-broadcast`) or the separate stages; `send
  owned-broadcast` without `--yes` exits 2, a usage error (before: 3, rejected).
- **The Android Make targets are removed** (`Makefile` `android`, `androidr`,
  `bindgen-android`, which ran `scripts/build-android.sh` and
  `bindgen-android.sh`; `.gitignore` Kotlin and Gradle entries).
- **CI runs `make lint`, `make test` and `make test-cli`**
  (`.github/workflows/ci.yml`; before: the same commands spelled out in the
  workflow).
- **`make test-ios` picks the first available iPhone simulator** (`Makefile`
  `IOS_TEST_DEST`; before: iPhone 17 Pro).
- **A technical-debt list from the 2026-10-26 code audit** (`docs/OPEN-ITEMS.md`
  "Technical debt": core, CLI and tests, iOS app).
- Tests: 1 new, 4 removed
  - Chain catalog 1: a test network naming another test network as its mainnet.
  - Removed: the address-validation-kind and hand-kept EVM list checks, and
    `either_route_asks_the_same_thing` (`execute_send`'s sign-only route); the
    registry's other tests moved to `registry/tests/mod.rs`.
- Docs: `AGENTS.md` (`swift/app/` layout); `README.md` (two front ends);
  `ARCHITECTURE.md` module table, chain rules, module boundaries, `api`, send
  stages; `FFI-BOUNDARY.md`; `OPEN-ITEMS.md` "Technical debt" (new);
  `scripts/README.md` build rows.
- Total: 284 .rs files changed (15 new, 2 deleted), +4780 −5565 lines; data
  and config files: `scripts/` (5), `tests/` (2), `.github/workflows/` (1),
  `.gitignore`, `Cargo.lock`, `Cargo.toml`, `Makefile`, `core/data/` (1),
  `docs/audits/` (1), `ffi/` (1); dependencies: removed spectra_core
  (`ffi/Cargo.toml`, with the crate); `#[test]` functions 1479 → 1476.

## Beta Commit 235

- **Internet Computer keys are the BIP-32 secp256k1 key at
  `m/44'/223'/{account}'/0/0`, as quill, dfx and DFINITY's agent libraries
  derive them** (`derivation/icp.rs`, `private_key.rs`, `import.rs`;
  `registry.rs` key scheme; `send/keys.rs` new `Secp256k1Key`;
  `send/icp_stages.rs`, `icp_staking.rs`)
  - The principal is the self-authenticating one of the key's DER encoding with
    the uncompressed point, and every ingress is signed with ECDSA over SHA-256
    with a low S (before: an Ed25519 SLIP-10 key at
    `m/44'/223'/{account}'/0'/0'`). Every phrase's ICP address changes.
  - An imported ICP private key or public key is read as secp256k1; the
    reference data is regenerated with `@dfinity/identity-secp256k1` 3.4.3
    (`derivation-profiles.json`, `icp-staking-vectors.json`).
- **A UTXO recipient amount below the network's dust threshold is refused, and
  Dogecoin's threshold is 0.01 DOGE** (`service/send_stage_utxo.rs`;
  `registry.rs`): change below it joins the fee; Dogecoin's rises from 546 to
  1,000,000 units, Dogecoin Core's `DEFAULT_DUST_LIMIT`.
- **A Kaspa spend keeps within KIP-9's storage mass limit of 100,000**
  (`send/kaspa.rs` `storage_mass`, `MAXIMUM_STANDARD_MASS`): selection takes
  more coins while its change is too small to stand, and a send no selection
  fits is refused.
- **Wiki and staking copy corrected** (`core/data/chain-wiki.toml`,
  `crypto-wiki.toml`, `staking.toml`): Polkadot's derivation, Zcash's shielded
  support, Sei as EVM-only after SIP-3, OKX's L2 as an OP Stack rollup,
  Bittensor's consensus; DOT capped at 2.1 billion, ETC near 210.7 million, CRO
  at 100 billion; Aptos unbonding about 14 days (before: 30), NEAR about 1–1.5
  days (before: 52 hours).
- Tests: 4 new
  - UTXO dust 2: an amount the network would not relay built, on account
    networks and multisig accounts; Dogecoin change under 0.01 DOGE kept as an
    output.
  - Kaspa storage mass 2: a mass that disagrees with rusty-kaspa's values, a
    selection past the standard limit signed.
- Docs: `CHAIN-SUPPORT.md` derivation paths (the Internet Computer) and fees
  (dust thresholds, Kaspa storage mass); `tests/README.md` ICP staking generator
  row.
- Total: 17 .rs files changed, +372 −110 lines; data and config files:
  `core/data/` (3), `core/tests/` (2), `tests/` (2); dependencies unchanged;
  `#[test]` functions 1475 → 1479.

## Beta Commit 234

- **A signed transaction's reservations are derived from its own content, one
  spelling per resource** (new `send/reservation.rs` `Reservation`;
  `StoredSend::reservations`; `wallet_db/sends.rs`)
  - Outpoints, account sequences, NEAR access-key nonces, Monero key images, ICP
    transfers and neurons, Solana accounts, Sui object versions and MWEB
    outputs; a txid is lowercase and a sender normalized, so one outpoint
    written in either case is reserved once and a second artifact consuming it
    is refused when stored.
  - `api::utxo::Txid` (32 bytes, parsed in either case, written lowercase) and
    `UtxoInput` replace the `(String, u32, u64, Vec<u8>)` tuple.
- **A broadcast needs the node to name the transaction it accepted**
  (`service/send_broadcast.rs`; `send/payload.rs` `SignedPayload`)
  - An answer naming no transaction is refused (before: the id was checked only
    when given); the PSBT and Safe suites' loopback nodes now answer the real
    txid and hash.
  - A signed payload is typed by form (hex, base64, document, Sui bytes and
    signature, ICP ingress calls), and one in a form the endpoint's API does not
    take is refused before it is sent.
- **Fetched history is typed** (`fetch/history.rs`, `history_decode.rs`,
  `transactions.rs`; `FetchedTransactionRecord` removed)
  - Kind and status are `TransactionKind` and `TransactionStatus`; an EVM row
    with no known status is refused (before: an unknown status was read by kind,
    a receive as pending and a send as confirmed).
  - A transfer whose amount cannot be read is no row (before: a row of 0);
    merging works on `TransactionRecord`, and status trackers carry typed
    statuses (`store/mod.rs`).
- Tests: 7 new, 6 removed
  - Reservations 2: one outpoint in two cases reserved twice, an account
    sequence keyed on an unnormalized sender.
  - Broadcast 2: a node answer naming no transaction taken as accepted, a
    payload in the wrong form sent.
  - Txid 1: a malformed txid read as some other outpoint.
  - History 2: an EVM row without a known status read as pending, an unreadable
    amount stored as 0.
  - Removed: the wire record's round-trip and status tests with the record,
    `a_bad_txid_is_refused` and the JSON field extractor's test.
- Docs: `ARCHITECTURE.md` persistent send stages (reservations).
- Total: 101 .rs files changed (2 new), +2012 −2143 lines; data and config
  files: `tests/` (8); dependencies unchanged; `#[test]` functions 1474 →
  1475.

## Beta Commit 233

- **One `SendPreview` record for every chain** (`send/preview_types.rs`
  `SendPreview`, `SendFeeTerms`, `EvmFeeTerms`; `send/preview_decode.rs`
  deleted)
  - The fourteen per-chain preview records and the `send::flow::SendPreview`
    enum become one record: fee, spendable balance, maximum, fee-rate
    description, transaction bytes and `terms` (`Flat`, `Evm`, `Utxo`); balance
    and maximum are always given.
  - Each chain's preview is built as that record (before: as JSON decoded by
    `preview_decode`); the EVM quote comes from the prepared transaction
    (`evm_send_preview`), and `Chain::simple_preview_chain` is
    `has_shared_send_preview`.
  - `OwnedSendPreview` carries the `preview`, its fee cut to the gas asset's
    precision and its balance and maximum to the sent asset's; a quote that is
    not an exact amount is refused (before: `network_fee` and
    `SendPreviewDetails`).
  - Amount shortcuts come from the quote's maximum with no fallback to the
    portfolio balance (`amount_input::send_amount_shortcut`; before:
    `flow::quoted_send_amount`, and a preview without a maximum fell back to
    `amount - fee`).
- **A rebroadcast reads only the submission core saved**
  (`service/send_records.rs`; `send/payload.rs` `SUBMISSION_FORMAT`): a record
  whose payload format is not `core.submission_json` is refused with
  "Rebroadcast is not supported for this transaction format" (before:
  `evm.raw_hex`, `evm.rust_json` and per-chain formats were decoded);
  `RebroadcastDispatch`, `PreparedBroadcastPayload`,
  `rebroadcast_prepare_payload` and the send result's `protocol_result_json` and
  `payload_format` are removed.
- **History entries have one type** (`fetch/history.rs` `NormalizedHistoryItem`
  replaces `ChainHistoryEntry` and the duplicate in `history_decode.rs`).
- **One `PasswordArgs` for every CLI command that opens a sealed wallet**
  (`cli/src/ctx.rs`; `cmd/multisig.rs`, `staking.rs`, `tx.rs`, `wallet.rs`)
  - The file, else `--password-env` (default `SPECTRA_PASSWORD`) when set, else
    the prompt; a wallet with no password is never asked for one. Staking now
    reads `SPECTRA_PASSWORD` by default (before: no default).
  - A variable that is not UTF-8 is a usage error, and the prompt's message
    names the variable.
- **`--json` prints its error object only when the command printed nothing**
  (`cli/src/out.rs` `emitted`; `CliError::reported` and `already_emitted`
  removed).
- **Two per-request loopback checks are removed** (`api/http.rs`,
  `icp_replica.rs`): the client's proxy refuses a non-loopback target under
  the guard.
- Tests: 2 new, 24 removed
  - CLI definitions 1: a subcommand's arguments inconsistent while it is never
    parsed (clap's `debug_assert`).
  - EVM quote 1: a zero gas limit or an unrepresentable nonce quoted.
  - Removed: the per-chain preview decoders and their fallbacks, the rebroadcast
    dispatch and payload preparation, `quoted_send_amount` and the EVM JSON
    preview's tests, with the code.
- Total: 61 .rs files changed (1 deleted), +982 −2651 lines; data and config
  files: `tests/cli-icp-staking.py`; dependencies unchanged; `#[test]`
  functions 1496 → 1474.

## Beta Commit 232

- **Cardano's icon mark redrawn** (`icons/crypto/cardano.svg`): a new white path
  on the same blue disc.
- Total: no .rs files changed; data and config files:
  `icons/crypto/cardano.svg`; dependencies unchanged; `#[test]` functions
  unchanged at 1496.

## Beta Commit 231

- **CLI acceptance suites and SDK generators move from `scripts/` to `tests/`**
  (`tests/cli-acceptance.sh`, `tests/cli-*.py`, `tests/generate-*`; `Makefile`,
  `.github/workflows/ci.yml`; new `tests/README.md`): `scripts/` keeps build,
  source-check and runtime-data tools; reference data stays in
  `core/tests/fixtures/`, whose provenance names the new paths.
- **`BEHAVIOUR-CHANGES.md` and `FUTURE_PLANS.md` are deleted** (`docs/`):
  AGENTS.md's Rule 0 no longer asks for a behaviour-change entry, and the docs
  that cited them drop the references.
- **Fields nothing read are removed from API records** (`api/evm_json_rpc.rs`,
  `horizon.rs`, `kaspa_rest.rs`, `solana_json_rpc.rs`, `tron_http.rs`): EVM
  `base_fee_wei`, `Erc20Balance` (an ERC-20 read answers raw units and
  decimals), the receipt's hash and status strings, Stellar `xlm_display`, Kaspa
  `block_daa_score`, Solana token `owner` and `symbol`, TRC-20 `contract` and
  `holder`; also Decred's unused `VERSION_ONLY_WITNESS` and Zcash `review`'s
  unused network argument.
- Docs: `AGENTS.md` (Rule 0 "Silence" removed, `tests/cli-acceptance.sh`);
  `README.md` (docs list, tests); `ARCHITECTURE.md` module table and tests
  paragraph; `OPEN-ITEMS.md`; `scripts/README.md`; `tests/README.md` (new);
  `docs/audits/` paths in `peercoin-2026-10-04.md`,
  `token-verification-2026-10-04/README.md` and two evidence files.
- Total: 28 .rs files changed, +38 −114 lines; data and config files: `tests/`
  (73), `core/tests/` (11), `docs/audits/` (2), `.github/workflows/` (1),
  `Makefile`, `tools/litecoin-mweb-fixture/` (1), `tools/zcash-fixture/` (1);
  dependencies unchanged; `#[test]` functions unchanged at 1496.

## Beta Commit 230

- **A dashboard row breaks its asset down by wallet**
  (`store::wallet_domain::DashboardAssetWallet`, `DashboardAssetGroup.wallets`;
  `service/state.rs`)
  - One entry per wallet holding the asset: its amount summed across networks
    and its value (`None` when any of it is unpriced), largest value first, ties
    in wallet-list order; a wallet excluded from the portfolio or a hidden
    holding is left out.
  - `WalletDerivedState.included_portfolio_holdings` is `Vec<PortfolioHolding>`,
    each with its `wallet_id`.
- **A pinned asset held nowhere has no dashboard row** (`service/state.rs`;
  `pinned_prototype` removed): pinning orders held rows (before: a zero row
  named from the catalog whenever an included wallet on one of its networks had
  read balances).
- **Price refresh asks for held assets and price alerts only**
  (`service/network_prices.rs`; `spectra price --refresh` help): a pin held
  nowhere is not quoted.
- **The asset wiki no longer lists where a coin lives** (`core/src/wiki.rs`;
  `AssetWikiEntry.lives_on` and `AssetWikiPlace` removed): Known Tokens shows a
  token's deployments.
- Tests: 1 new, 2 removed
  - Wallet breakdown 1: wrong sums or order, or an excluded wallet listed.
  - Removed: the zero pinned row's and `lives_on`'s tests, with what they
    tested.
- Docs: `BEHAVIOUR-CHANGES.md` three 2026-10-12 entries (wallet breakdown,
  "Lives On" dropped, pinned assets held nowhere); `AGENTS.md` Git rule
  shortened to "Do not git commit or push unless I said so."
- Total: 7 .rs files changed, +181 −298 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions 1497 → 1496.

## Beta Commit 229

- **CLI acceptance keeps only what the binary shows; the rules it checked are
  cargo tests in core** (`scripts/`; `core/src/service/tests/`, `store/tests/`,
  `wallet_db/tests/`, `api/`; see `BEHAVIOUR-CHANGES.md` "CLI acceptance keeps
  only what the binary shows")
  - 17 suites deleted: `cli-wallets`, `cli-portfolio`, `cli-history`,
    `cli-transport`, `cli-token-preferences`, `cli-chain-coverage`,
    `cli-receipt-fees`, `cli-finality-account-chains`, `cli-send-local-digests`,
    `cli-send-utxo`, `cli-litecoin`, `cli-peercoin`, `cli-account-utxo`,
    `cli-send-monero`, `cli-send-xrp`, `cli-payment-memos`, `cli-multisig-p2sh`;
    `cli-multisig-sui-aptos` is `cli-multisig-aptos`, and the Bittensor and Nile
    reruns are gone. 26 suites remain.
  - The remaining suites and `cli-acceptance.sh` keep state across processes and
    data directories, files on disk, exit codes and `--yes` gates, and one
    staged build, sign and broadcast-once flow per path; each suite's scope is
    in `scripts/README.md`.
  - Every suite runs under the harness's one network journal (before: eight
    pointed `SPECTRA_LOOPBACK_ONLY` at a journal nothing read); `multisig
    submit` and `discard` are checked to ask for `--yes`.
  - `spectra-zcash-fixture` always funds mainnet with fixed amounts, and the
    MWEB fixture drops its other-network and SegWit fields, with the checks that
    used them.
- **`spectra staking build --action` is a clap `ValueEnum`**
  (`cli/src/cmd/staking.rs`; before: a string list matched by hand).
- **Two refusals moved into functions the tests call** (`api/lightwalletd.rs`
  `serves_network`, `send/zcash_shielded.rs` `shielding_error`); behaviour
  unchanged.
- Measured: 2026-10-10, `make test-cli` against a prebuilt binary: 61 checks in
  83 seconds, where the deleted suites alone took about 200
  (`docs/BEHAVIOUR-CHANGES.md`).
- Tests: 169 new
  - Multisig sessions 41 (PSBT, P2SH, Safe, Tron, XRP, Stellar, Sui, Aptos,
    Cardano, Substrate, TON): signing short of a threshold, by a non-member or
    on a changed policy, sequence or deadline; used nonces; spent inputs; MWEB
    recipients.
  - Sends 52: a reviewed send signed after its inputs, nonce, fee, runtime,
    funds or destination's memo rules changed; memos and tags required or
    altered; NEAR prepayments and registration; token, NFT and TRC-10 builds the
    network would refuse; broadcasts to an unproven or other-network endpoint;
    replacements and cancellations; durable records.
  - Staking 14: stakes below the minimum or the balance, positions under another
    authority, locked withdrawals, ICP neuron setup and control, and fees raised
    before signing.
  - Wallet operations 22: trust lines, approvals, account closing, coins, NEAR
    keys and storage, NFTs, Sui coins and Solana token accounts refused where
    their prerequisites fail.
  - History and settlement 19: pages past every full page and cursor, a
    transaction settled only for its own hash and at finality, rows netted per
    address, stored history paging and identities.
  - Import, store and endpoints 21: imports per network, refused imports sealing
    nothing, custom tokens, pins, every catalog network taking a custom
    endpoint, proxy routing after reopening, a lightwalletd on another network.
- Docs: `BEHAVIOUR-CHANGES.md` entries "CLI acceptance keeps only what the
  binary shows" and "Dead code removed from core, the app and the scripts" (the
  latter recording Beta Commit 228); `scripts/README.md` suite table and runner
  note.
- Total: 100 .rs files changed (33 new), +14066 −115 lines; data and config
  files: `scripts/` (44); dependencies unchanged; `#[test]` functions 1328 →
  1497.

## Beta Commit 228

- **Unused code deleted from core** (`core/src/api/`, `core/src/service/`,
  `core/src/lib.rs`)
  - The single-page `fetch_history(address)` readers of 14 provider clients (BCH
    REST, Blockbook, BlockCypher, Horizon, ICP Rosetta, Insight, Kaspa, Koios,
    NearBlocks, Sui, TON Center v2, TronGrid, WhatsOnChain, XRPL) are deleted;
    history reads go through the paged readers.
  - Also deleted: `SecretHex` (`send/keys.rs`), `HistoryScope` and
    `reset_history` (`service/history_cursor.rs`; a history reset calls
    `history_pagination.reset_all()`), the history store's `reset`, the
    `decimals` and `log_index` fields of `EvmTokenTransferItem`, and the
    file-backed secret store's `root`.
  - Operational log entries lose their `wallet_id` and `metadata` fields, and
    `DiagnosticCommand::Reset` and the per-wallet log pruning that read them are
    deleted (`service/diagnostic_state.rs`).
  - Crate-root, `send`, `staking` and `service` re-exports nothing used are
    removed (`DonationDestination`, `SendTokenIdentity`, `FundsScan`,
    `PortfolioSnapshot`, `WalletActions`, `MoneroSyncStatus` and others).
- **Fiat icons dropped from the icon tooling and rules**
  (`scripts/normalize-icons.sh`, `scripts/svgo.config.mjs`, `AGENTS.md`): only
  `icons/crypto/` is normalized.
- **Python bytecode removed from the tree and ignored** (`scripts/__pycache__/`,
  `.gitignore`).
- **`make clean` no longer removes `build/`** (`Makefile`).
- **The Zcash fixture drops its unused `blake2b_simd` and `prost`
  dependencies** (`tools/zcash-fixture/Cargo.toml`).
- Tests: 0 new, 2 removed (a history-store reset of one entry and a
  token-preference identity check, with the code they covered).
- Docs: `UI-FIX.md` deleted; `OPEN-ITEMS.md` keeps only open items (the ticked
  single-wallet setup, wallet details, transfer correctness, asset protocol,
  wallet import and multisig items removed); `AGENTS.md` "Icons", "Platform
  constraints"; `README.md` "Verification" (four targets, `make check-ui` by
  hand); `ARCHITECTURE.md` "Chain rules" file name; `FFI-BOUNDARY.md` "Callers
  and coverage" (`RefreshObserver`); `scripts/README.md` "Icons".
- Total: 35 .rs files changed, +57 −305 lines; data and config files:
  `scripts/__pycache__/` (4), `scripts/` (2), `.gitignore`, `Cargo.lock`,
  `Makefile`, `tools/zcash-fixture/` (1); dependencies: removed blake2b_simd,
  prost (`tools/zcash-fixture/Cargo.toml`); `#[test]` functions 1330 → 1328.

## Beta Commit 227

- **A custom token is added under a named standard** (`core/src/store/state.rs`
  `AddCustomToken`, `core/src/chains.rs` `ChainEntry.token_standards`,
  `cli/src/cmd/token.rs`)
  - `ChainEntry.token_standards` lists each protocol with its own
    `identifier_prompt` and `fixed_decimals`, replacing the network-wide
    `contract_address_prompt` and `fixed_token_decimals`.
  - With no standard named, the network's only standard is used; a network with
    several refuses with `StandardRequired`, and a standard the network lacks is
    `UnsupportedStandard` (before: the standard was guessed from the
    identifier's shape, digits on Tron being TRC-10 and every BNB contract
    ERC-20).
  - `spectra token add` on a network with several standards needs `--standard`
    (exit 3 without it).
- Docs: `BEHAVIOUR-CHANGES.md` three entries (custom token standard; two
  app-only: Show Seed Phrase after Face ID, Reset Wallet as a Settings page);
  `UI-FIX.md` items 104–106.
- Total: 4 .rs files changed, +272 −121 lines; data and config files:
  `scripts/cli-token-preferences.py`; dependencies unchanged; `#[test]`
  functions unchanged at 1330.

## Beta Commit 226

- **Three app changes recorded; no code in this repository changed**
  (`docs/BEHAVIOUR-CHANGES.md`, `UI-FIX.md` items 99–103): deleting a wallet
  returns to Home, Receive's wallet list reads like Home's, and launch shows the
  logo until core has read the store.
- Docs: `BEHAVIOUR-CHANGES.md` three 2026-10-09 entries; `UI-FIX.md` sections
  8–10.
- Total: No code changed.

## Beta Commit 225

- **Send reviews carry the total, the fee and a new-destination warning**
  (`service/owned_send.rs`, `service/send_stages.rs`)
  - `OwnedSendPreview.total` and `SendArtifactReview.total` are amount plus fee
    when the coin pays its own fee (`None` for a token).
  - `RecipientCheck.is_new_to_wallet` carries the new-address warning with the
    quote (before: only after Build).
  - `SendArtifactReview.network_fee` and `network_fee_value` carry the quoted
    fee into the built artifact; `build_staking` fills them too.
  - `OwnedSendPreview.amount_refusal` refuses an amount over the balance at
    quote time (before: at Build).
- **A scanned payment code keeps its amount and memo** (new
  `send/scanned_payment.rs`, replacing `send::flow::scanned_send_address`)
  - `read_scanned_payment(chain, token_contract, payload)` returns address,
    amount and memo from BIP-21, EIP-681, Solana Pay, `ripple:`, SEP-7,
    `monero:` and `ton://transfer`, converting wei and nanotons by native
    decimals.
  - An EIP-681 token transfer pays its `address` parameter (before: the token
    contract was read as the recipient); a code pinned to another EVM chain,
    another asset, or a memo kind Spectra cannot send is refused.
  - `read_scanned_address` reads a recipient alone for address fields.
- **Receive can ask for an amount** (`Chain::payment_uri_format`,
  `payment_request_uri`; `spectra send request`): writes a payment request in
  the network's format, refusing a network with none, another network's address,
  a non-positive or over-precise amount and an unsupported memo.
- **An unreadable store can be discarded** (`service/state.rs`,
  `wallet_db/connection.rs`; `spectra settings discard --yes`)
  - `open_state` reports `SpectraBridgeError::StoreUnreadable` for a database
    this build cannot decode (before: each call failed on its own).
  - `discard_state(path)` deletes the database, the sealed secrets and Zcash
    databases of the wallets it names (ids read raw from rows), and opens a
    fresh store; it refuses a store that is open.
  - `history_clear` deletes records by id without decoding them, so a reset
    clears records this build cannot read.
  - `send_list` leaves out a stored send it cannot decode or verify instead of
    failing the list; nonce and reservation queries still refuse it.
- **Tor on holds every request until Tor is ready; the kill-switch setting is
  deleted** (`core/src/tor.rs`; `settings set tor-kill-switch` removed): HTTP,
  gRPC and TCP requests wait while Tor connects or is down (before: with the
  default switch off they went out directly), and the policy is published after
  the transport switches.
- **Custom endpoints can be replaced and removed**
  (`AppSettingUpdate::ReplaceCustomEndpoint`, `RemoveCustomEndpoint`; `spectra
  endpoints --replace OLD --add NEW`, `--remove URL`): removing a network's last
  custom endpoint turns off "use only my endpoints".
- **Portfolio and dashboard facts for the app** (`service/valuation.rs`,
  `store/state.rs`, `store/wallet_domain.rs`)
  - `QuotedTotal.test_network_count` counts test-network holdings the total
    leaves out.
  - `WalletState.balances_read_at` is set at the first committed balance read,
    even of unchanged zeros; a pinned asset held nowhere waits for a read on a
    network it lives on.
  - `DashboardAssetGroup.is_small` marks unpinned rows under $1.
  - `TransactionSnapshot.pending_count` counts pending records across wallets.
  - `transaction_amount_direction` makes a zero-amount transfer neutral;
    `spectra txs` uses it.
- **New exports for display and seed entry** (`formatting.rs`,
  `send/prepared_fields.rs`, `validation/mod.rs`, `service/wallet_import.rs`)
  - `display_address` writes EVM addresses in EIP-55.
  - `prepared_fields` flattens prepared transaction details into named rows
    with byte strings as `0x` hex.
  - `seed_word_suggestions`, `seed_phrase_words_from_text` and
    `default_wallet_name`.
- **`UI-FIX.md` added**: the 2026-10-09 UI review checklist, items ticked as
  landed.
- Tests: 30 new, 5 removed
  - Scanned payments and requests 13 (four moved from `send/flow.rs`): a token
    transfer paying its contract, another chain's code accepted, a dropped
    amount or memo, a request the network cannot express.
  - Unreadable store 3: an undecodable store opening normally, a discard leaving
    secrets behind, clearing history failing on unreadable rows.
  - Prepared fields 3: raw JSON tags, byte lists, non-JSON details lost.
  - Dashboard and balances 4: dust shown as a row, a pinned asset shown before
    any read, zeros never marked read, test-network holdings silently omitted.
  - Custom endpoints 2: an edit losing "only mine", removal leaving a network
    with nothing to ask.
  - Seed entry 2: prefix suggestions, numbered or comma-separated pastes.
  - Send total, EIP-55 display and zero-amount direction 3: the fee added to
    a token send's total, an EVM address shown unchecksummed, a zero transfer
    shown as moving.
- Docs: `BEHAVIOUR-CHANGES.md` 37 entries dated 2026-10-09 from the UI review
  (most app-only); `UI-FIX.md` (new).
- Total: 56 .rs files changed (2 new), +2486 −401 lines; data and config
  files: `scripts/cli-acceptance.sh`, `scripts/cli-portfolio.py`,
  `scripts/cli-send.py`, `scripts/cli-transport.py`, `scripts/cli-wallets.py`;
  dependencies unchanged; `#[test]` functions 1305 → 1330.

## Beta Commit 224

- **An app crash fix recorded; no code in this repository changed**
  (`docs/BEHAVIOUR-CHANGES.md`): backing out of a new wallet's phrase page no
  longer crashes, since each setup form gets its own draft.
- Docs: `BEHAVIOUR-CHANGES.md` one 2026-10-08 entry.
- Total: No code changed.

## Beta Commit 223

- **Multisig accounts on every network that has one, behind one session model**
  (new `service/multisig.rs` and `service/multisig_<scheme>.rs`;
  `wallet_db/psbt.rs` → `wallet_db/multisig.rs`; `cli/src/cmd/multisig.rs`
  replaces `cli/src/cmd/psbt.rs`; `service/psbt.rs` deleted)
  - Sessions of any scheme live in `multisig_sessions` (was `psbt_sessions`);
    exports `multisig_account`, `create_multisig`, `import_multisig`,
    `sign_multisig`, `multisig_session(s)`, `finalize_multisig`,
    `submit_multisig`, `discard_multisig`, and `spectra multisig
    account|create|import|sign|show|list|finalize|submit|discard` (was `spectra
    psbt …`).
  - `WalletState::multisig_policy` replaces `multisig_descriptor`; `wallet watch
    --multisig` takes the policy in the network's own form
    (`Chain::multisig_policy_format`).
  - Every scheme verifies signatures before counting them, binds each session to
    a review digest, re-reads the network before a signature or submission, and
    decodes outside data only when it re-encodes byte for byte.
  - `MultisigAccount` says how sessions reach the network (`AsIs`, `ByExecutor`,
    `ByApproval`) and which local wallets may sign.
- **Schemes added** (`send/p2sh_multisig.rs`, `safe.rs`, `tron_multisig.rs`,
  `xrp_multisig.rs`, `stellar_multisig.rs`, `sui_multisig.rs`,
  `aptos_multikey.rs`, `cardano_multisig.rs`, `substrate_multisig.rs`,
  `ton_multisig.rs`; policy readers in `api/evm_safe.rs`, `tron_http.rs`,
  `xrpl_json_rpc.rs`, `horizon.rs`, `substrate_json_rpc/multisig.rs`,
  `toncenter_v2.rs`)
  - Bitcoin Cash and Dogecoin P2SH `sortedmulti` (up to 15 keys) through BCHN
    PSBTs and Dogecoin Core partially signed transactions; Bitcoin Cash Schnorr
    multisig signatures and Litecoin MWEB fields are refused.
  - Safe 1.3.0 and 1.4.1 proxies on EVM networks: owners, threshold, nonce,
    modules and guard read from the node; executed by one owner wallet
    (`--executor`); delegate calls and used nonces refused.
  - Tron account permissions (`Permission_id`), XRP Ledger signer lists (fee N+1
    base fees), Stellar signers against the medium threshold.
  - Sui multisig and Aptos MultiKey offline signatures combined into one
    authenticator; Aptos `0x1::multisig_account` not supported.
  - Cardano native scripts (cardano-cli JSON) with CIP-1854 vkey witnesses from
    signer phrase wallets.
  - `pallet-multisig` on Polkadot Asset Hub and Bittensor
    (`Chain::ss58_prefix`); a threshold of one refused.
  - TON multisig v2 (code hash checked): propose, approve, execute at threshold.
- **Ordinary sends refused before signing when the wallet's key alone cannot
  authorize them** (`send/tron.rs`, `send/xrp.rs`, Stellar): Tron when no
  permission it meets alone covers the contract, XRP when the master key is
  disabled, Stellar when the master key's weight is below the threshold.
- **`format_staking_amount` renamed `decimal::format_native_amount`.**
- Tests: 31 new
  - Multisig scheme vectors and refusals 31: Safe 5, Tron 4, TON 4, Sui 3, Aptos
    3, Cardano 3, Substrate 3, P2SH 2, XRP 2, Stellar 2. They guard addresses,
    digests, signatures and finished transactions against each network's
    reference SDK, and refuse forged or changed sessions.
- Docs: `BEHAVIOUR-CHANGES.md` one 2026-10-08 entry; `CHAIN-SUPPORT.md`
  "Multisig accounts" (new), "Wallets and transfers"; `ARCHITECTURE.md` "Core
  module boundaries" (multisig modules); `OPEN-ITEMS.md` "Multisig accounts"
  (three done, three open: signer sets, tokens, Substrate cancel);
  `scripts/README.md` ten multisig CLI suites and vector generators.
- Total: 103 .rs files changed (40 new, 2 deleted), +15078 −1009 lines; data
  and config files: `scripts/` (22), `core/tests/` (12), `Cargo.lock`, `core/`
  (1); dependencies: added p256 0.13 (`core/Cargo.toml`); `#[test]` functions
  1274 → 1305.

## Beta Commit 222

- **Every UTXO network but Cardano and Monero uses one account model**
  (`Chain::uses_account_utxo`; new `send/account_utxo.rs`,
  `send/legacy_p2pkh.rs`; `service/send_stage_utxo.rs`,
  `service/address_discovery.rs`)
  - Bitcoin, BCH, BSV, Dogecoin, Zcash, Bitcoin Gold, Decred, Kaspa and Dash
    phrase wallets store their account key, scan once before first read
    (`utxo_discoveries`), rotate receive addresses, sum every owned address and
    spend from all of them with per-input keys (before: Bitcoin-family balances
    read one address and sends spent from it; ZEC, BTG, DCR, KAS and DASH held
    one address).
  - `send/bitcoin_cash.rs`, `bitcoin_gold.rs`, `bitcoin_sv.rs`, `dash.rs`,
    `dogecoin.rs`, `service/history_bitcoin.rs`, `service/network_hd.rs` and
    `derivation/xpub_walker.rs` are deleted; `has_single_owned_address`,
    `supports_deep_utxo_discovery` and `WalletSetupLimit::SingleAddress` are
    gone.
  - Fees follow size: Bitcoin's rate over vsize, static fee per started kB on
    legacy-format networks, ZIP-317 on Zcash, at least ten atoms a byte on
    Decred, mass on Kaspa (before: most paid one fixed fee).
  - Refused now: a phrase path no account profile holds (on Bitcoin too), and
    Kaspa coinbase outputs.
  - These phrase wallets export their phrase (and on Bitcoin the account key),
    not one address's key.
- **Every account UTXO network watches an account public key** (new
  `derivation/account_key.rs`; `Chain::account_key_versions`)
  - Each network reads its own encodings (SLIP-132 for BTC and LTC, `dgub`,
    `drkp`, `dpub`, `kpub`/`ktub`, `xpub`/`tpub` elsewhere) at account depth
    with a valid checksum; a sibling network's key is refused (before: Bitcoin
    only, any depth, prefix normalised to `xpub`).
  - Two encodings of one account are one account, so a phrase upgrades the
    watched wallet.
- **Bitcoin multisig accounts and PSBTs** (new `derivation/multisig.rs`,
  `send/psbt.rs`, `service/psbt.rs`, `wallet_db/psbt.rs`; `spectra psbt
  create|import|sign|show|list|finalize|broadcast|discard`, `wallet watch
  --descriptor`)
  - A `wsh(sortedmulti)` descriptor is stored canonically with its BIP-380
    checksum; other scripts, wrong network, bad origins, more than 20 keys and
    repeated keys refused.
  - PSBTs are reviewed before signing or storing (inputs and change the wallet's
    script, valid signatures, `SIGHASH_ALL` only); a cosigner's phrase imported
    with `--upgrade` signs.
  - A custom Bitcoin indexer is checked by genesis hash before broadcasting
    (`Chain::genesis_block_hash` replaces `litecoin_genesis`).
- **Monero view-only wallets and subaddresses** (`derivation/monero.rs`,
  `send/monero_local.rs`, `service/monero_wallet.rs`; `wallet watch --address A
  --view-key K`)
  - `WalletImportKind::WatchViewKey` stores the view key as `{id}.scan-key`
    outside the password; a view-only scan needs no password, reports
    `spends_known: false` and cannot send; importing the phrase upgrades it.
  - Scans watch wallet2's lookahead (50 accounts, 200 addresses) past the
    highest used; receive address rotates to the next subaddress once the shown
    one is used (before: only the primary address was scanned and shown).
- **Polkadot and Bittensor phrases derive along Substrate junction paths** (new
  `derivation/substrate_path.rs`; `--path //hard/soft`)
  - Hard and soft junctions as subkey and polkadot.js read them; `///password`,
    `0x` junctions and numbers past 64 bits refused.
  - Import, preview, signing, message signing, staking and export use the stored
    path (before: a path was dropped and the root key used);
    `resolve_derivation_path` refuses an unparsable path instead of substituting
    the default; non-`uniform` HMAC overrides refused.
- **Litecoin's signer is `send/bitcoin.rs`, including Taproot key-path**
  (`send/litecoin.rs`).
- Tests: 29 new, 20 removed
  - Account UTXO sends 12: inputs from every source, per-network sighash and fee
    rules, a key not owning its input.
  - Multisig and PSBTs 6: descriptor checksum and refusals, PSBT review and
    signing against bitcoinjs-lib.
  - Monero subaddresses and view keys 3, Substrate junctions 3, account keys 2,
    key export 2, registry 1.
  - Removed: the old fixed-fee Bitcoin and preview-decode tests, the Bitcoin HD
    history walk and address-validation tests their code took with it.
- Docs: `BEHAVIOUR-CHANGES.md` six 2026-10-08 entries; `CHAIN-SUPPORT.md`
  "Wallets and transfers"; `ARCHITECTURE.md` "Core module boundaries";
  `OPEN-ITEMS.md` six wallet-import items ticked.
- Total: 114 .rs files changed (18 new, 9 deleted), +7237 −4462 lines; data
  and config files: `scripts/` (14), `core/tests/` (5); dependencies
  unchanged; `#[test]` functions 1265 → 1274.

## Beta Commit 221

- **A NEAR wallet gets its token storage deposits back** (new
  `service/wallet_near_storage.rs`; `send/near.rs`; `spectra wallet
  token-storage`, `wallet refund-storage --contract`)
  - `wallet_token_storage` collects contracts from holdings, stored history and
    the NearBlocks inventory (now keeping emptied tokens), and lists those
    holding a deposit while the account holds none of the token.
  - `build_token_storage_refund` builds `storage_unregister` with `{}` arguments
    (never `force`), one yoctoNEAR and 30 Tgas under
    `WalletOperation::RefundTokenStorage`; deposit, empty balance, nonce, fee
    and funds re-read before signing and broadcast.
  - History records `refundTokenStorage`; a non-staking NEAR call with no
    operation is refused by the stage binding.
- **A Cardano account shows its stake address, rewards and delegation**
  (`NetworkAccount::Cardano`, `derivation::cardano::cardano_stake_address`,
  `api/koios.rs` `account_info`): reward address type 14 or 15, registration,
  rewards and pool/DRep delegation from a genesis-verified node; enterprise,
  pointer and Byron addresses have no network account (`has_network_account` now
  asks the wallet).
- Tests: 5 new
  - NEAR storage refund 3: refunding while tokens are held, a missing deposit,
    signing anything but the reviewed unregistration.
  - Cardano stake 2: stake addresses against serialization-lib, `account_info`
    parsing.
- Docs: `BEHAVIOUR-CHANGES.md` two 2026-10-08 entries; `CHAIN-SUPPORT.md`
  "Wallets and transfers", "Staking positions and execution"; `OPEN-ITEMS.md`
  two wallet-details items ticked; `scripts/README.md`.
- Total: 26 .rs files changed (3 new), +1354 −81 lines; data and config files:
  `core/tests/fixtures/cardano-stake-addresses.json`,
  `core/tests/fixtures/near-storage-unregister.json`,
  `scripts/cli-wallet-operations.py`,
  `scripts/generate-cardano-stake-address-vectors.cjs`,
  `scripts/generate-near-storage-unregister-vector.cjs`; dependencies
  unchanged; `#[test]` functions 1260 → 1265.

## Beta Commit 220

- **XRP payments carry destination tags and Stellar payments memos** (new
  `send/payment_memo.rs`; `send/xrp.rs`, `send/stellar.rs`; `--destination-tag`,
  `--memo-text`, `--memo-id`)
  - `PaymentMemo` on `SendExecutionRequest` and `SendReviewInput`, kinds per
    `Chain::payment_memo_kinds`, bound into the review digest and shown on
    `SendArtifact.memo`; refused on other networks.
  - A destination requiring a tag (`lsfRequireDestTag`) or memo (SEP-29) is
    refused without one, read before building and again before signing.
  - Account closing takes the memo and can close into such destinations (before:
    refused outright).
- **A NEAR token send registers its recipient first**
  (`send::near::nep141_transfer_calls`, `service/send_near.rs`): an unregistered
  recipient gets `storage_deposit` with the bounds' minimum before `ft_transfer`
  in one transaction, priced in the fee budget and shown as
  `AssetTransferTerms.recipient_registration`; a contract answering neither
  NEP-145 query is refused (before: the transfer failed on chain after its gas).
- **A Cardano phrase derives its base address** (`derivation/cardano.rs`):
  payment key plus the account's stake key at `…/2/0` (before: enterprise
  address, unlike Yoroi, Eternl, Lace, Daedalus and Trezor); paths must be
  CIP-1852 role 0 or 1; raw keys keep the enterprise address; a phrase wallet no
  longer exports its payment key.
- **A Decred send pays the script its recipient names**
  (`derivation::decred::parse_decred_address`, `send/decred.rs`): P2SH
  recipients get `OP_HASH160 … OP_EQUAL` (before: every recipient got P2PKH, so
  script-hash payments went to an unspendable key hash); testnet versions from
  `Chain::decred_address_versions` (before: testnet sends refused their own
  sender); both addresses decoded before any provider read; `Dk…`, `De…`, `DS…`
  refused.
- Tests: 11 new
  - Decred 4: the wrong output script for P2SH, sighash against dcrd, a
    recipient read after inputs.
  - Payment memos 4: a memo on the wrong network or out of range, encodings
    against the XRPL codec and Stellar SDK.
  - Cardano base addresses 3: addresses against CIP-19, a path with no stake
    key.
- Docs: `BEHAVIOUR-CHANGES.md` four 2026-10-08 entries; `CHAIN-SUPPORT.md`
  "Wallets and transfers", "Staking positions and execution"; `OPEN-ITEMS.md`
  four items ticked, Cardano stake address added; `scripts/README.md`
  `cli-payment-memos.py`.
- Total: 52 .rs files changed (2 new), +1706 −362 lines; data and config
  files: `scripts/` (11), `core/tests/` (3); dependencies unchanged; `#[test]`
  functions 1249 → 1260.

## Beta Commit 219

- **Litecoin MWEB: phrase wallets hold, scan and send MWEB funds** (new
  `send/litecoin_mweb/`, `service/litecoin_mweb.rs`, `api/litecoin_p2p.rs`,
  `api/litecoin_p2p/wire.rs`, `api/tcp.rs`; `spectra wallet mweb-sync`,
  `mweb-status`, `mweb-pegin`, `send-mweb`)
  - Scan key `m/1000'/0'` and spend key `m/1000'/1'` of the BIP-39 seed; core's
    own MWEB primitives (tagged BLAKE3, Pedersen and switch commitments,
    Schnorr, stealth addresses, kernels) with bulletproofs through
    `grin_secp256k1zkp`.
  - `sync_litecoin_mweb` scans bounded batches over Litecoin's peer-to-peer
    protocol, checking scrypt proof of work and retargets, the MWEB header
    through its HogEx and output pages into the output root; three mainnet DNS
    seeds and one testnet seed built in.
  - `build_litecoin_mweb_send` pays inside MWEB or by peg-out;
    `build_litecoin_mweb_pegin` moves transparent LTC in; fees as Litecoin
    Core's; transactions verified as a node does before storing.
  - An MWEB address is valid to pay but refused as a watch or wallet address; a
    custom Litecoin indexer must name Litecoin's genesis before broadcasting.
- **Zcash: phrase wallets hold, scan and send shielded funds** (new
  `service/zcash_shielded.rs`, `send/zcash_shielded.rs`, `api/lightwalletd.rs`,
  `api/grpc.rs`, `wallet_db/zcash.rs`; `spectra wallet zcash-sync`,
  `zcash-status`, `shield`, `send-shielded`)
  - The ZIP-32 account the standard path names, kept in librustzcash's own
    database beside Spectra's (viewing keys only, deleted with the wallet).
  - `ShieldTransparent` and `ShieldedPayment` operations through the send
    stages; Sapling parameters downloaded once from `download.z.cash` and used
    only when length and BLAKE2b-512 match; signing refuses a branch other than
    librustzcash's for the next block.
  - Built-in lightwalletd endpoints at zec.rocks; gRPC and TCP obey the Tor kill
    switch, SOCKS5 proxy and loopback guard.
  - Every Zcash address form validates through librustzcash; transparent sends
    pay TEX addresses and refuse shielded ones; consensus branches come from
    `BranchId::for_height`, so transparent sends work under NU6.3.
  - `monero_heights.rs` becomes `restore_heights.rs`, giving Zcash phrase
    wallets a restore height.
- **EVM ERC-721 and ERC-1155 tokens** (new `api/evm_nft.rs`,
  `service/wallet_nfts.rs`; `api/blockscout.rs`; `spectra wallet nfts`,
  `send-nft`)
  - Inventory from Blockscout; `build_nft_transfer` reads the standard by
    ERC-165 and ownership live, builds `safeTransferFrom` under
    `WalletOperation::TransferNft`, and re-reads ownership before signing.
  - History reads `tokennfttx` and `token1155tx`; ERC-20 reads refuse NFT
    contracts and `tokentx` rows with a `tokenID` are dropped.
- **XRP Ledger and Stellar issued assets as tokens** (new `api/xrpl_amount.rs`,
  `api/stellar_asset.rs`, `send/xrp_issued.rs`, `send/stellar_issued.rs`,
  `service/wallet_trust_lines.rs`; `spectra wallet trust-lines`, `trust`,
  `untrust`)
  - "Trust Line Token" (`CODE.rIssuer`, 15 places) and "Stellar Asset"
    (`CODE:ISSUER`, 7 places) with balances, discovery and history;
    `fixed_token_decimals` replaces the decimals prompt.
  - Sends refuse missing, unauthorized, frozen or full lines, non-rippling
    issuers and inexact amounts; XRPL transfer rates paid with `SendMax`.
  - Trust lines opened and removed as `WalletOperation::TrustAsset` and
    `RemoveTrustLine`; Horizon amounts read exactly (over seven places or `i64`
    refused).
- **Cardano spends token-bearing outputs and sends native assets**
  (`send/cardano.rs` `PreparedCardanoTransaction`, new `api/cardano_asset.rs`)
  - Selection takes outputs with the sent asset, then pure ADA, then other
    token-bearing outputs; change returns every asset; every output holds the
    protocol minimum ADA; fee from Koios's epoch parameters.
  - "Cardano Native Token" (`POLICY.NAME`) with decimals from CIP-68 then the
    registry; a Koios endpoint's genesis magic checked before broadcasting.
- **Solana Token-2022 transfer fees and transfer hooks** (new
  `send/solana_token.rs`): `TransferCheckedWithFee` with the current epoch's
  fee, refused within 300 slots of a fee change; hook accounts resolved and
  simulated; paused, non-transferable, frozen, memo-required, confidential-only,
  interest-bearing and unlisted extensions refused by name; terms bound into the
  review digest.
- **History rows of a listed token carry the list's name and symbol**
  (`service/history_refresh.rs`); a provider row confirms a recorded operation
  and keeps its kind, amount and counterparty.
- **One encrypted scan cache for Monero and Litecoin MWEB**
  (`wallet_db/scan_cache.rs` replaces `wallet_db/monero.rs`), keyed from the
  wallet's scan key.
- **Loopback Zcash and MWEB fixtures** (new workspace members
  `tools/zcash-fixture/`, `tools/litecoin-mweb-fixture/`; built in CI).
- Measured: 2026-10-07 and 2026-10-08, read-only checks of the new built-in
  endpoints: Koios native-asset reads
  (`docs/audits/cardano-asset-endpoints-2026-10-07.json`), XRPL and Horizon
  trust-line reads (`issued-asset-endpoints-2026-10-07.json`), Blockscout NFT
  lists and ERC-165 calls (`nft-endpoints-2026-10-07.json`), lightwalletd gRPC
  calls (`zcash-shielded-endpoints-2026-10-07.json`) and Litecoin peer
  handshakes plus a full `wallet mweb-sync` walk
  (`litecoin-mweb-nodes-2026-10-08.json`); nothing submitted.
- Tests: 87 new, 8 removed
  - Issued assets 19: XRPL amount encoding and rounding against
    ripple-binary-codec, trust-line refusals, Stellar credit payments.
  - Zcash shielded 18: receivers against reference vectors, TEX addresses,
    Sapling parameter pinning, undecodable proposals, branch schedule, restore
    heights.
  - Litecoin MWEB 16: mainnet blocks verifying, changed parts failing, fees as
    mainnet kernels paid, peg-in and peg-out balance, header chain and
    retargets.
  - NFTs 15: ids as whole numbers, a collection read as a fungible token,
    transfers against ethers.
  - Cardano assets 9, Solana Token-2022 8, other 2.
  - Removed: four moved (three `monero_heights` tests to `restore_heights`, the
    Cardano Emurgo witness test to `send/tests/cardano.rs`); two Cardano refusal
    tests and two Zcash transparent-decoder tests replaced by the new suites.
- Docs: `BEHAVIOUR-CHANGES.md` 15 entries dated 2026-10-07 and 2026-10-08;
  `CHAIN-SUPPORT.md` "Wallets and transfers", "Fungible assets", "Non-fungible
  tokens" (new), "History and confirmation"; `ARCHITECTURE.md` "Workspace"
  (fixtures), "Core module boundaries" (`tcp`, `grpc`, `litecoin_p2p`,
  `wallet_db::zcash`, `scan_cache`); `OPEN-ITEMS.md` six asset and privacy items
  ticked, NEAR storage and CIP-113 added; `scripts/README.md` new suites and
  generators.
- Total: 127 .rs files changed (47 new, 2 deleted), +21681 −1189 lines; data
  and config files: `scripts/` (18), `core/tests/` (7), `docs/audits/` (5),
  `core/data/` (2), `.github/workflows/` (1), `Cargo.lock`, `Cargo.toml`,
  `core/` (1), `tools/litecoin-mweb-fixture/` (1), `tools/zcash-fixture/` (1);
  dependencies: arti-client 0.22 → 0.41, tor-rtcompat 0.22 → 0.41, rusqlite
  0.31 → 0.37; added blake3 1, bls12_381 0.8, grin_secp256k1zkp 0.8, http 1,
  hyper-util 0.1, jubjub 0.10, orchard 0.15, prost 0.14, rustls 0.23,
  sapling-crypto 0.7, scrypt 0.11, secrecy 0.8, tokio-rustls 0.26, tonic 0.14,
  tower 0.5, webpki-roots 1, zcash_client_backend 0.24, zcash_client_sqlite
  0.22, zcash_keys 0.16, zcash_primitives 0.30, zcash_proofs 0.30,
  zcash_protocol 0.10, zcash_script 0.4, zcash_transparent 0.10, zip32 0.2
  (`core/Cargo.toml`); new manifests `tools/litecoin-mweb-fixture/Cargo.toml`
  (bip39 2, blake3 1, grin_secp256k1zkp 0.8, hex 0.4, litecoin =0.32.8-rc.1,
  num-bigint 0.4, rand 0.8, secp256k1 0.29, serde_json 1, sha2 0.10, tokio 1)
  and `tools/zcash-fixture/Cargo.toml` (bip39 2, blake2b_simd 1.0, bls12_381
  0.8, futures 0.3, hex 0.4, http 1, incrementalmerkletree 0.8, jubjub 0.10,
  orchard 0.15, prost 0.14, rand 0.8, sapling-crypto 0.7, secp256k1 0.29,
  serde_json 1, tokio 1, tokio-stream 0.1, tonic 0.14, tonic-prost 0.14, tower
  0.5, zcash_client_backend 0.24, zcash_keys 0.16, zcash_primitives 0.30,
  zcash_protocol 0.10, zcash_script 0.4, zcash_transparent 0.10, zip32 0.2);
  `#[test]` functions 1170 → 1249.

## Beta Commit 218

- **A wallet's actions come from core** (new `service/wallet_actions.rs`;
  `spectra wallet actions`): `wallet_actions` lists each `WalletAction` with its
  section and note plus the network's setup summary; a watched wallet lists only
  what needs no key; staking and the Monero scan are wallet actions.
- **Wallets sign and verify messages in their network's scheme** (new
  `send/message.rs`, `send/message_schemes.rs`, `service/wallet_messages.rs`;
  `spectra wallet sign-message`, `address verify-message`)
  - Bitcoin-family `signmessage` for P2PKH with each network's magic, BIP-322
    simple for SegWit and Taproot, EIP-191, TIP-191, Solana, Sui personal
    message, Substrate `<Bytes>`, Stellar SEP-53, Cardano CIP-8, Kaspa personal
    message and Monero `SigV2`.
  - Solana refuses a message that reads as a transaction; EVM refuses EIP-712
    typed data; a key that is not the address's is refused.
- **Keys export in their network's own encoding**
  (`key_formats::encode_private_key`, `service/wallet_keys.rs`; `spectra wallet
  export`): WIF, Solana base58 keypair, Stellar `S…`, `suiprivkey1…`, AIP-80,
  NEAR `ed25519:…`, Monero spend and view keys, and Bitcoin account
  xpub/ypub/zpub; a phrase spread over many addresses exports no single key.
- **A wallet's key can be added to another network** (new
  `service/wallet_copy.rs`): targets restore the same phrase format or share the
  key's signature scheme (`Chain::key_scheme`); core reads the secret and runs
  the target's import, sealing a standalone copy under the same password.
- **A watched wallet takes its keys from its page**
  (`WalletImportCommit::upgrade_wallet_id`, `wallet_upgrade_methods`): the
  import upgrades that wallet or stores nothing.
- **Network-specific wallet operations** (new `service/wallet_*.rs`; `spectra
  wallet …`)
  - EVM: live ERC-20 allowances from approval logs and `approve(spender, 0)`
    revocation as `WalletOperation::RevokeApproval`; Ethereum ENS primary name
    when it resolves back (`approvals`, `revoke`, `ens`).
  - Solana: close empty token accounts for their rent, at most 20 per
    transaction (`token-accounts`, `close-token-accounts`).
  - Sui: list coin objects and merge a type's objects (`objects`, `merge`).
  - NEAR: list access keys and delete function-call keys; a named import signs
    with the key its secret derives (`keys`, `delete-key`).
  - XRP and Stellar: close an account into an existing one with `AccountDelete`
    or `AccountMerge` after checking every network prerequisite (`close`).
  - Network account facts for Tron, XRP, Stellar, TON, Substrate and NEAR
    (`account`); the TRC-20 preview reports the signed fee limit.
  - UTXO addresses and coins with confirmations from the indexer tip (`coins`);
    Peercoin outputs say whether they can be spent.
  - Monero payment proof: tx key derived from the stored send plan and checked
    with `check_tx_key` before it is shown (`prove-payment`).
- **A wallet can hide a holding from its totals**
  (`WalletState::hidden_holdings`, `StateCommand::SetHoldingHidden`; `wallet
  hide`, `unhide`): it stays sendable; databases from before the change do not
  load.
- **ICP wallets record their principal** (`WalletState::icp_principal`) beside
  the account ID.
- **Test networks link their faucet** (`chains.toml` `faucet` on 37 of 40 test
  networks, `chain_faucet_url`); a mainnet or non-https faucet is refused.
- **Every explorer but Monero's has an address page** (`Explorer::address_url`,
  `address_explorer_link`; `core/data/explorers.toml`): Zcash moves to
  mainnet.zcashexplorer.app and Bitcoin Gold to btgexplorer.com.
- **One operation field for wallet-page transactions**
  (`SendArtifact::operation`, `WalletOperation`): bound into the review digest
  and checked by `StoredSend::validate`; new history kinds `CloseTokenAccounts`,
  `MergeCoins`, `DeleteAccessKey`, `RevokeApproval`.
- Measured: 2026-10-07, explorer address pages probed for an address on each
  network (`docs/audits/explorer-addresses-2026-10-07.json`; some refused by bot
  protection are marked not confirmed live) and each test-network faucet checked
  to name its exact network (`docs/audits/testnet-faucets-2026-10-07.json`;
  Dogecoin testnet, Decred testnet and Kaspa TN10 have none).
- Tests: 51 new
  - Message signing 9: signatures against each network's SDK, a signature
    covering something else, a non-message accepted.
  - Key export 7: an export not reading back as the same wallet, Monero keys,
    BIP-84 zpub, exports needing no password.
  - Account closing 6, network account 5, wallet actions 5, wallet copy 4,
    approvals 3, hidden holdings 3.
  - Faucets 2, explorers 2, Solana token accounts 2, Sui merge 1, Monero payment
    proof 1, ICP principal 1.
- Docs: `BEHAVIOUR-CHANGES.md` 19 entries dated 2026-10-07; `OPEN-ITEMS.md`
  "Wallet details" (new, items ticked as done; Cardano base addresses and
  XRP/Stellar tags added); `scripts/README.md` `cli-wallet-operations.py` and
  generators.
- Total: 91 .rs files changed (22 new), +9898 −240 lines; data and config
  files: `scripts/` (13), `core/tests/` (7), `core/data/` (2), `docs/audits/`
  (2); dependencies unchanged; `#[test]` functions 1119 → 1170.

## Beta Commit 217

- **TON wallets are W5 by default, v4R2 on request** (`TonWalletVersion`,
  `derivation/ton.rs`, `derivation/ton_cell.rs`, `send/ton.rs`; `spectra wallet
  import --ton-wallet w5|v4R2`)
  - Created wallets and imports hold the W5 (wallet v5r1) account unless v4R2 is
    named (before: v4R2 only, so Tonkeeper and MyTonWallet wallets restored
    empty).
  - Nothing stores the version: signing takes the version whose account the
    stored address is (`TonWalletVersion::of_address`) and refuses an address no
    version gives the key.
  - The used-account search reads both versions; `spectra rescan --chain ton`
    lists both.
- **A 429 waits out the rate window** (`api/http.rs`
  `RetryProfile::delay_for_attempt`): at least one second, then two, after a
  429; every retry delay carries up to 20% jitter (before: 350 ms then 700 ms,
  no jitter).
- **PBKDF2 runs through ring** (new `core/src/kdf.rs`): BIP-39, Cardano, TON,
  Polyseed, the app password and wallet sealing use `ring::pbkdf2`; the `pbkdf2`
  crate is removed. Output unchanged.
- **Dependencies are optimized in dev and test builds** (`Cargo.toml`
  `[profile.dev.package."*"] opt-level = 3`).
- **`make test-ios` skips test diagnostics collection** (`Makefile`
  `-collect-test-diagnostics never`).
- Tests: 4 new, 4 removed
  - TON W5 2: jetton transfers against @ton/ton for both versions (replacing the
    v4R2-only test), the used-account scan reading each version.
  - Retry delays 1: a 429 retried inside its window.
  - PBKDF2 1: published SHA-256 and SHA-512 vectors.
  - Removed: three v4R2 address tests superseded by the fixture vectors.
- Docs: `BEHAVIOUR-CHANGES.md` three 2026-10-07 entries (one app-only);
  `CHAIN-SUPPORT.md` "Wallets and transfers" (TON versions); `OPEN-ITEMS.md` TON
  version item ticked; `scripts/README.md` `generate-ton-w5-vectors.cjs`.
- Total: 26 .rs files changed (1 new), +854 −355 lines; data and config files:
  `scripts/` (4), `Cargo.lock`, `Cargo.toml`, `Makefile`, `core/` (1),
  `core/tests/` (1); dependencies: removed pbkdf2, added ring 0.17
  (`core/Cargo.toml`); `#[test]` functions unchanged at 1119.

## Beta Commit 216

- **An import is on one network** (`WalletImportRequest`, `WalletImportKind`;
  `derivation/import.rs`, `service/wallet_import.rs`)
  - `WalletImportRequest` names one `chain` and a kind: `Phrase`, `PrivateKey`,
    `WatchAddresses` or `WatchAccountXpub` (before: `selected_chain_ids` with
    one wallet per chain, plus `is_watch_only_import`/`is_private_key_import`
    flags).
  - `wallet new`, `import` and `watch` take one `--chain` (a second is exit 2).
- **Each network describes how a wallet is added** (new `derivation/setup.rs`;
  `wallet_setup_descriptor`; `spectra wallet methods`, `chains` `setupMethods`)
  - The descriptor lists the methods, formats, profiles and fields a network
    offers; `import_wallets` refuses anything else.
  - `ChainIdentity` loses `derives_from_private_key`,
    `supports_watch_only_import` and `accepts_account_xpub`; `spectra chains`
    drops the two import columns.
- **Derivation profiles replace path presets** (`chains.toml`,
  `derivation/path.rs`; `Chain::derivation_profiles`, `derivation_profile_path`;
  `wallet import --profile --account`)
  - Each chain lists profiles with one account segment: Bitcoin family `legacy`,
    `nestedSegWit`, `nativeSegWit`, `taproot` where its signer spends them,
    Solana and Bitcoin Cash `legacy`, `standard` elsewhere.
  - `SeedDerivationPreset` and `SeedDerivationPaths` are deleted; wallets carry
    one `derivation_path`.
  - An unparsable path and a path on Monero, TON, Polkadot or Bittensor are
    refused (before: replaced by the default or ignored); Funds Finder scans
    every profile at accounts 0–2.
  - Bitcoin's Electrum-style paths, Bitcoin Cash `m/0`, Cardano `m/44'/1815'`,
    and the unnamed XRP and Tron legacy paths are dropped from the catalog.
- **NEAR derives along its SLIP-10 path** (`derivation/near.rs`):
  `m/44'/397'/{account}'` as near-seed-phrase does (before: the BIP-39 seed's
  first 32 bytes, a different account from MyNearWallet and Meteor).
- **Monero and TON read and create their own phrases** (new
  `derivation/phrase.rs`, `monero_words.rs`, `polyseed.rs`;
  `core/data/wordlists/`)
  - Monero reads 25-word seeds in all thirteen wordlists and 16-word Polyseed in
    ten, and creates 25-word seeds; TON reads and creates ton-crypto 24-word
    mnemonics, including password-protected ones; both refuse BIP-39 (before:
    BIP-39 readings no Monero or TON wallet restores).
  - `check_seed_phrase` takes the chain and reports the format;
    `generate_seed_phrase(chain, word_count)` replaces `generate_mnemonic`;
    `wallet check-seed --chain`.
- **Private keys import in their chain's own encoding** (new
  `derivation/key_formats.rs`; `Chain::private_key_formats`,
  `is_valid_private_key`): compressed WIF with the network's version byte,
  Solana base58/JSON keypair, Stellar `S…`, `suiprivkey1…`, AIP-80, NEAR
  `ed25519:…`; uncompressed WIF, another network's WIF and mismatched keypairs
  refused; `is_private_key_hex` deleted.
- **A Monero wallet's restore height is fixed at import** (new
  `core/src/monero_heights.rs`, `core/data/monero-checkpoints.json`; `wallet
  import --restore-height`): typed, from a Polyseed birthday, or a monthly
  checkpoint near now for a created wallet; `send sync-monero --restore-height`
  removed.
- **The address an import will store is previewed** (`preview_wallet_import`;
  `wallet import --preview`, `wallet watch --preview`): the same planning as the
  import, nothing sealed or read from the network.
- **A signing import upgrades its watched wallet; other duplicates are refused**
  (`place_import`): the watch keeps its id, history, name and settings; an
  address held with keys or watched twice is refused naming the holder.
- **Used accounts found per network, and the setup summary**
  (`service/funds_scan.rs`, new `service/setup_summary.rs`; `spectra rescan`,
  `wallet capabilities`, `wallet reserve`, `endpoints --custom-only`)
  - `begin_funds_scan` with a chain scans that network's profiles at accounts
    0–2, names its endpoints first and reports `used` (funded or with history)
    or unread.
  - `wallet_setup_summary` reports balance, history and token-discovery
    coverage, staking, limits and the endpoints the first refresh reads.
  - `AppSettings.custom_endpoints_only` keeps catalog URLs out of transport and
    indexers; `endpoint_api_options(chain)` lists every network and API for
    custom endpoints.
  - `account_reserve` reads XRP's base reserve or twice Stellar's.
- **NEAR imports hold named accounts** (`WalletImportCommit.named_account`;
  `wallet import --named-account`): stored once a verified node lists the key as
  full-access.
- Tests: 48 new, 20 removed
  - Phrase formats 20: Monero seeds in every wordlist against monero-python,
    Polyseed against the reference, TON mnemonics against ton-crypto, BIP-39
    refused on both.
  - Setup descriptor and profiles 8: an offered method failing to import, a
    profile not importing at its path, a preview differing from the stored
    address.
  - Private-key encodings 8: wrong-network WIF, uncompressed WIF, mismatched
    keypairs.
  - Watched-wallet upgrade and duplicates 5, setup summary 3, Monero restore
    heights 3, used-account search 1.
  - Removed: multi-chain import plans, BIP-39 Monero and TON readings, path
    presets and hex-only key editing.
- Docs: `BEHAVIOUR-CHANGES.md` 13 entries dated 2026-10-06 (one app-only: the
  seed-phrase copy); `CHAIN-SUPPORT.md` "Wallets and transfers" (phrase formats,
  profiles, key encodings, NEAR, reserves); `OPEN-ITEMS.md` "Single-wallet
  setup" (new, items ticked; TON version open).
- Total: 59 .rs files changed (9 new), +5874 −4375 lines; data and config
  files: `core/data/` (25), `scripts/` (16), `core/tests/` (4); dependencies
  unchanged; `#[test]` functions 1091 → 1119.

## Beta Commit 215

- **Docs record the app's per-domain observable state; no code in this
  repository changed** (`docs/ARCHITECTURE.md` "Native flow lifetimes and
  display precision", `docs/BEHAVIOUR-CHANGES.md`, `docs/OPEN-ITEMS.md`):
  `AppState` composes one observable object per domain, a reset no longer clears
  an in-flight history page fetch, and the open item for the split is removed.
- Docs: `ARCHITECTURE.md` "Native flow lifetimes and display precision";
  `BEHAVIOUR-CHANGES.md` one 2026-10-06 entry; `OPEN-ITEMS.md` `AppState` item
  removed.
- Total: No code changed.

## Beta Commit 214

- **48 FFI record fields that nothing wrote are deleted** (`send/`,
  `staking/types.rs`, `service/`)
  - Account-model previews lose `estimatedTransactionBytes`,
    `selectedInputCount` and `usesChangeOutput`; XRP
    `sequence`/`lastLedgerSequence`, Stellar `sequence`, Cardano `ttlSlot`, TON
    `sequenceNumber` and Tron `simulationUsed` go.
  - `StakingValidator.apy` and `next_epoch_active` go; `spectra staking
    validators` drops the APY column and `apy` key.
  - `SendExecutionRequest.monero_priority` goes; Monero sends use normal
    priority.
  - `BitcoinHistorySnapshot` is internal; its counterparty is an explicit empty
    string.
- **A MIR scan rejects record fields production code never writes** (new
  `scripts/unwritten-record-fields.sh`, `scripts/record_writers.py`,
  `scripts/test-record-writers.py`): run in `make lint` and the CI source-scan
  step.
- **Exported types named once, without the `Core` prefix** (19 types):
  `CoreAppState` → `ResidentState`, `CoreMessage` → `LocalizableMessage`,
  `CorePersistedTransactionRecord` → `TransactionRecord`,
  `CoreTransactionRecord` → `FetchedTransactionRecord`,
  `PriceAlertEvaluationAlert` → `PriceAlertRule`, `CoreResetPlan` → `ResetPlan`,
  and `Core*` dropped from the rest. No behaviour change.
- Tests: 0 new, 1 removed (it pinned account-model previews dropping UTXO-only
  fields).
- Docs: `BEHAVIOUR-CHANGES.md` one 2026-10-06 entry; `FFI-BOUNDARY.md` "Export
  shapes" (name a type once), "Callers and coverage" (the new scan), "Runtime
  and storage traps", "Common errors"; `ARCHITECTURE.md` "Ownership"
  (`ResidentState`); `OPEN-ITEMS.md` both items removed; `scripts/README.md`
  "Code and resource checks".
- Total: 99 .rs files changed, +618 −824 lines; data and config files:
  `.github/workflows/ci.yml`, `Makefile`, `scripts/cli-icp-staking.py`,
  `scripts/record_writers.py`, `scripts/test-record-writers.py`,
  `scripts/unwritten-record-fields.sh`; dependencies unchanged; `#[test]`
  functions 1092 → 1091.

## Beta Commit 213

- **CLI acceptance tests the binary; rules are tested in core**
  (`scripts/cli-acceptance.sh`, `AGENTS.md`, `README.md`, `FFI-BOUNDARY.md`)
  - The inline suite keeps only what the binary shows: exit codes, state across
    processes, sealed secrets on disk, `--yes` gates and the loopback guard.
  - `cli-wallet-deletion.py` and `cli-diagnostics.py` are deleted; nine Python
    tests duplicating core tests and the endpoint suite's catalog pins are
    removed.
  - Solana validators with the node's minimum delegation becomes a core wiremock
    test; BIP-84's second receive address becomes a core vector.
- **Source scans run in `make lint` and a CI "Source scans" step** (`Makefile`
  `SOURCE_SCANS`, `.github/workflows/ci.yml`).
- **Core tests that tested nothing of the project's are deleted**: diagnostics
  serde round trips, a Swift-key JSON pin, front-end string checks, hand lists
  restating registry flags and chain-count pins in UTXO polling and discovery
  (now stated per chain).
- Tests: 1 new, 6 removed
  - Staking validators 1: validators and minimum delegation not read from the
    configured node.
- Docs: `BEHAVIOUR-CHANGES.md` one 2026-10-06 entry; `AGENTS.md` "Architecture",
  "Verification"; `README.md` "The CLI", "Verification"; `FFI-BOUNDARY.md`
  "Callers and coverage"; `scripts/README.md` "CLI tests", "Code and resource
  checks".
- Total: 14 .rs files changed, +98 −165 lines; data and config files:
  `scripts/` (7), `.github/workflows/` (1), `Makefile`; dependencies
  unchanged; `#[test]` functions 1097 → 1092.

## Beta Commit 212

- **Token tags are a closed list, and the stored category is gone**
  (`core/src/tokens.rs` `TokenTag`, `list_token_tags`; `store/wallet_domain.rs`)
  - `TokenTag` (`stablecoin`, `meme`) replaces free strings; `tokens.toml` with
    an unknown or repeated tag is refused at load.
  - `CoreTokenPreferenceCategory` and the stored `category` field are deleted
    (it had no reader).
  - `spectra token catalog --json` lists each deployment's `tags`.
- Docs: `BEHAVIOUR-CHANGES.md` one 2026-10-05 entry.
- Total: 10 .rs files changed, +54 −45 lines; data and config files:
  `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]` functions
  unchanged at 1097.

## Beta Commit 211

- **Network rules and presentation share one `chains.toml` row**
  (`core/data/chains.toml`, `core/data/chain-ui.toml` deleted,
  `core/src/chains.rs`): search keywords, rank, tags, color, artwork and address
  hints move onto the network row; testnets inherit rank and tags.
- **The chain wiki and picker share one tag classification**
  (`ChainWikiEntry.tags` as `ChainTag`; `core/data/chain-wiki.toml`; `spectra
  chains --wiki`)
  - Wiki rows carry the picker's resolved tags; `chain-wiki.toml` refuses tag
    fields and its free-text tags are removed.
  - `spectra chains --wiki` prints mainnet wiki rows with the catalog's filters
    and refuses `--testnets`.
- Tests: 1 new, 1 removed
  - Tag classification 1: the wiki and picker classifying a chain differently
    (replaces the presentation-join row-order test).
- Docs: `BEHAVIOUR-CHANGES.md` two 2026-10-05 entries; `ARCHITECTURE.md` "Chain
  rules".
- Total: 3 .rs files changed, +103 −123 lines; data and config files:
  `core/data/chain-ui.toml`, `core/data/chain-wiki.toml`,
  `core/data/chains.toml`, `scripts/cli-acceptance.sh`; dependencies
  unchanged; `#[test]` functions unchanged at 1097.

## Beta Commit 210

- **Peercoin mainnet and testnet** (new `derivation/peercoin.rs`,
  `send/peercoin.rs`, `service/send_stage_peercoin.rs`; `api/blockbook.rs`;
  `core/data/chains.toml`, `tokens.toml`, `testnet-tokens.toml`,
  `endpoints.toml`, `explorers.toml`, `chain-ui.toml`, `chain-wiki.toml`)
  - Catalogued with native PPC at six decimals, keyless Blockbook endpoints,
    explorers, wiki and Funds Finder; the existing icon is reused.
  - Seed accounts on BIP-44, BIP-49, BIP-84 and BIP-86 with discovery and
    receive rotation; raw-key and watch-only imports validate the network.
  - Balance, history, preview, build, sign, broadcast and pending status;
    version-3 transactions sign the actual prevout script, P2PK minting rewards
    included.
  - Rewards spendable after 500 mainnet or 60 testnet confirmations; recipient
    and change follow the 0.01 PPC minimum, smaller change joining the fee; fee
    0.01 PPC/kB over the full size with a 0.001 PPC floor; largest-first
    selection, MAX within the money bound and 100 kB.
  - Parent transactions are hash-verified; mismatched hash, value or script,
    duplicate inputs, future-dated legacy parents and wrong-network providers
    are refused.
  - Minting is not offered.
- **Litecoin and Peercoin share one owned-account source model** (new
  `service/send_utxo_sources.rs`, from `send_stage_litecoin.rs`): stored receive
  and change addresses are spendable after restart, protected wallets included.
- **UTXO reads use each chain's precision and network** (`api/blockbook.rs`,
  `fetch/bitcoin_history.rs`, `service/network_balance.rs`)
  - Blockbook fee conversion uses the chain's decimals (before: eight for every
    coin); status keeps confirmation counts and refuses a response for another
    transaction.
  - Account history resolves addresses on the wallet's concrete mainnet or
    testnet.
  - Wallet balances are checked integer sums without the per-transaction money
    cap.
- **58 of the 59 commented token deployments are active again; Avalanche PEPE is
  deleted** (`core/data/tokens.toml`, `crypto-wiki.toml`): the 58 keep their
  original fields, with all 21 identities and wiki descriptions; Avalanche PEPE
  `0xa659…be6d` is removed at the user's request.
- **Signing and state saves open SQLite write transactions immediately**
  (`wallet_db/sends.rs`, `wallet_db/state.rs`;
  `TransactionBehavior::Immediate`): a second signer waits for the writer
  instead of failing with `database is locked`, then is refused by the
  reservation check.
- Measured: 2026-10-04, Peercoin (`docs/audits/peercoin-2026-10-04.md`): both
  public Blockbooks answered without a key with six decimals, in sync, backend
  Peercoin 15.0.0; address, UTXO and fee-estimate reads succeeded; a malformed
  `sendtx` reached the backend parser; no funded transaction broadcast. Token
  reverification (`docs/audits/token-verification-2026-10-04/`): all 59
  originals pass fixed-block chain-ID, bytecode and decimals reads; 58 are tied
  to issuer or official bridge sources; Avalanche PEPE's bridge operator is
  unresolved.
- Tests: 34 new, 1 removed
  - Peercoin provider reads 11: unverified parents, maturity boundaries, wrong
    network or precision, a status for another transaction.
  - Peercoin signing and selection 9: preimages on both networks, minimum
    change, witness fee bytes, refusals before network or storage.
  - Peercoin derivation and import 8: official Cointoolkit keys, every script
    type on its network, payment URIs on the wrong network.
  - Account UTXO balances and sources 5: rotated addresses left out, overflow,
    wrong-network paths (one moved from the Litecoin suite).
  - Registry 1.
- Docs: `BEHAVIOUR-CHANGES.md` four 2026-10-04 entries; `CHAIN-SUPPORT.md`
  counts (50 mainnets, 40 test networks), "Wallets and transfers" (Peercoin),
  "Fungible assets" (reactivation); `OPEN-ITEMS.md` endpoint audit task and the
  2026-10-04 code review gaps (transfer correctness, asset and privacy
  protocols, wallet import, history and providers, staking), replacing the three
  scope items; new records `docs/audits/peercoin-2026-10-04.md` and
  `docs/audits/token-verification-2026-10-04/README.md`.
- Total: 46 .rs files changed (10 new), +3632 −366 lines; data and config
  files: `docs/audits/` (13), `core/data/` (8), `scripts/` (2); dependencies
  unchanged; `#[test]` functions 1064 → 1097.

## Beta Commit 209

- **Staking positions and transactions on Solana, Sui, Aptos, NEAR, Polkadot and
  ICP** (new `service/staking_protocols.rs`, `staking_substrate.rs`,
  `staking_icp.rs`, `send/icp_staking.rs`, `send/polkadot_pools.rs`,
  `api/icp_replica.rs`, `api/fastnear.rs`, `api/substrate_json_rpc/pools.rs`;
  `spectra staking positions|build|recheck|repair|broadcast-endpoints`)
  - `staking positions` reads owned positions with unlocking and withdrawable
    amounts (before: six clients returned an empty list without querying).
  - `staking build` prepares stake, unstake, withdraw and claim-rewards as
    durable artifacts for `send sign` and `send broadcast-signed`: Solana stake
    accounts, Sui `StakedSui`, Aptos and NEAR delegation pools, Polkadot Asset
    Hub nomination pools, ICP NNS neurons.
  - Core checks authority, amounts, unlock state, fees and prerequisites before
    signing; watch-only wallets cannot sign.
  - ICP: explicit `--lockup-seconds` dissolve delay counted from configuration
    execution (up to two years), following topics 0/4/14, a certificate verified
    against the pinned IC root key for each step, `staking repair` for
    governance-only recovery after the original funding is proved, maturity
    claims needing at least 1 ICP and fewer than ten pending payouts.
  - Validator reads move into the API adapters (`solana_json_rpc`,
    `sui_json_rpc`, `near_json_rpc`, `aptos_rest`); computed and catalog APY
    figures are removed (`core/data/staking.toml`), and the minimum delegation
    is read from the chain.
  - History distinguishes stake, unstake, withdraw and claim; staking
    submissions get pending verification.
- **Each network lists its token standards; TRC-10 is supported on Tron**
  (`Chain::token_standards`, `api/tron_http.rs`, `api/trongrid_v1.rs`,
  `send/tron.rs`; `spectra chains` `tokenStandards`)
  - Every deployment stores its actual standard; Tron accepts TRC-10 and TRC-20,
    Aptos legacy coins and AIP-21 assets (before: one default standard per
    chain, TRC-10 refused before any request).
  - TRC-10 metadata, balances, discovery, history, `TransferAssetContract`
    signing, fees and outcomes on mainnet and Nile.
  - Legacy BTTOLD (TRC-10 `1002000`) is a separate identity from redenominated
    BTT.
  - TON jetton masters are one raw account identity, so friendly aliases cannot
    duplicate a holding.
- **59 token deployments without recorded issuer or bridge evidence are
  commented out as TODO records** (`core/data/tokens.toml`, `crypto-wiki.toml`):
  fields kept in place with the 21 identities and wiki records only they used;
  the list is
  `docs/audits/chain-support-2026-10-04/removed-token-deployments.json`.
- **Token transfers on Sui, Aptos and TON; ERC-20 on Ethereum Classic and
  HyperEVM** (`send/sui.rs`, `send/aptos.rs`, `send/ton.rs`,
  `service/send_stage_protocols.rs`)
  - Sui separates token inputs from SUI gas; TON validates the owner's jetton
    wallet, budgets attached TON and signs TEP-74's 120-bit amounts.
  - Sui, Aptos and TON hashes are derived locally and checked against submission
    replies; a retry of an expired Aptos or TON signature first resolves any
    existing outcome; TON's prepared expiry matches the signed message.
- **Every native send family has a confirmation path, and age no longer fails a
  pending send** (`service/pending_status.rs`, `api/transaction_status.rs`,
  `api/toncenter_v3.rs`)
  - Account chains read the exact transaction's result, failures included; TON
    follows the external message and jetton delivery; UTXO families read status;
    Substrate resolves finalized events.
  - A pending record keeps its state until an execution result proves the
    outcome (before: age plus repeated provider failures marked it failed).
  - Custom nodes prove network identity (Horizon passphrase, XRPL network id,
    Sui genesis checkpoint, TON zero-state hashes, Tron genesis, Aptos chain
    id).
- **Cardano signs with its full extended key and spends only pure-ADA inputs**
  (`send/cardano.rs`, `api/koios.rs`): before, it signed an expanded scalar as
  an Ed25519 seed and dropped native-asset facts on inputs.
- **Bittensor shares Polkadot's metadata-driven transaction model**
  (`send/bittensor.rs` deleted; `send/substrate.rs`,
  `api/substrate_json_rpc/metadata.rs`): pallet indices and signed extensions
  come from live metadata, with genesis, runtime and fees validated at every
  stage.
- **NEAR fees, validity and hashes follow the protocol**
  (`api/near_json_rpc.rs`, `send/near.rs`, new `service/send_near.rs`)
  - Native, NEP-141 and staking budgets use live protocol fees, implicit-account
    creation and the minimum prepaid-gas price; the static token reserve is
    removed.
  - Validity follows the canonical reference block and live validity period
    (before: a fixed 120-second timeout).
  - Storage reserve, funds and budget are rechecked before submission and
    pending retries; the local hash is kept and a foreign submission identity
    rejected.
- **Raw private-key import on 48 mainnets and their test networks** (new
  `derivation/private_key.rs`; `derivation/dispatch.rs`)
  - A test network derives its own address (before: six UTXO testnets derived
    mainnet).
  - Bitcoin account xpubs keep their concrete network.
  - Stellar receive addresses are canonical uppercase.
- **History pagination persisted, with Sui, Aptos, TON and NEAR token activity**
  (new `wallet_db/history_pagination.rs`, `api/history_page.rs`,
  `api/aptos_indexer.rs`; `spectra history --save --load-more`)
  - Continuations come from raw provider rows, advance only after the merge and
    survive restart; a failed refresh keeps the old cursor (before: one page,
    marked exhausted).
  - Aptos history comes from the public Indexer, received and orderless
    transactions included; Aptos and NEAR token discovery enumerates holdings.
  - A Blockscout-compatible custom indexer can be configured on any EVM network.
- **OP Stack receipts publish the complete actual fee** (`api/evm_json_rpc.rs`):
  execution, L1 data and operator charge from the oracle at the mined block; a
  missing component leaves the total unknown.
- **Monero defaults are eligible for local scanning and Avalanche defaults read
  ERC-20** (`core/data/endpoints.toml`).
- **ICP replica health reads and validates the CBOR status**
  (`api/icp_replica.rs`, `service/endpoint_health.rs`).
- **CLI help and results describe current behaviour**
  (`cli/src/cmd/settings.rs`, `wallet.rs`, `ctx.rs`, `main.rs`)
  - `settings reset --scope` accepts only core's scopes (exit 2 otherwise);
    warning and result name the effective plan, and JSON returns `plan` instead
    of a `settings` count.
  - Raw-key import honours `--no-password`.
  - Missing-secret errors name the actual flag (`--seed-file`,
    `--private-key-file`, `--password-file`); chain selectors take names and
    registry ids only.
- **`Chain::is_testnet` reads the network declaration**
  (`core/src/registry.rs`): loading the token catalog no longer deadlocks on the
  joined chain catalog.
- **Unused exports and resources removed** (`icons/fiat/`,
  `icons/crypto/monad.svg`, new `scripts/source_scan.py`)
  - Nine internal types stop being UniFFI exports.
  - The four fiat icons are deleted.
  - Monad's icon drops its shadow filter.
  - Source scans count production code only.
- Measured: 2026-10-04, read-only evidence under
  `docs/audits/chain-support-2026-10-04/` (endpoint probes, network identities,
  OP Stack fork configurations, TON Center, NEAR receipts, TRC-10 metadata and
  fees) and `docs/audits/account-staking-evidence-2026-10-04.json`,
  `icp-staking-evidence-2026-10-04.json`,
  `solana-staking-simulation-2026-10-04.json`: five Monero daemons passed
  network, sync, fork and scan reads; three Avalanche defaults passed chain-ID
  and ERC-20 reads; Solana staking instructions simulated on mainnet with zero
  signatures; nothing broadcast.
- Tests: 92 new, 12 removed
  - Execution status 17: success or failure guessed without the exact
    transaction, TON deliveries assumed, provider failures failing a send,
    expired retries rebroadcast blindly.
  - ICP staking 16: request ids and signatures against the DFINITY SDK, forged
    or absent certificates, double-counted neuron fees, renewed pending ingress.
  - Token protocols 15: TRC-10 identity, precision and wrong network; Sui, Aptos
    and TON signing vectors; alias and BTTOLD identities.
  - Account-chain and pool staking 13: signing against official SDKs, slashed
    pool ratios, staking history.
  - Network identity 11: a custom node on another network.
  - OP Stack fees 6, NEAR fees and validity 5, other 9 (history continuation,
    Cardano inputs, Bittensor runtime, raw keys, Stellar case, endpoint
    defaults).
  - Removed: tests pinning replaced behaviour (TRC-10 refusal, stale pending
    failing by age, the NEAR token gas floor, single-page history, static
    staking copy).
- Docs: `BEHAVIOUR-CHANGES.md` eleven 2026-10-04 entries (one app-only);
  `CHAIN-SUPPORT.md` (new: wallets and transfers, fungible assets, history and
  confirmation, staking positions and execution); `OPEN-ITEMS.md` OP Stack fees,
  token provenance list, Monad history, TON aliases, staking execution and
  staking-adapter items removed, "Known limitations" replaced by "Supported
  chain scope"; `scripts/README.md` new CLI suites, vector generators and
  source-scan files.
- Total: 175 .rs files changed (18 new, 1 deleted), +19154 −3657 lines; data
  and config files: `scripts/` (26), `docs/audits/` (25), `core/tests/` (12),
  `core/data/` (5), `icons/fiat/` (4), `Cargo.lock`, `core/` (1),
  `icons/crypto/` (1); dependencies: added candid 0.10, ic-cbor 3.2.0,
  ic-certificate-verification 3.2.0, ic-certification 3.2.0, num-bigint 0.4,
  xxhash-rust 0.8 (`core/Cargo.toml`); `#[test]` functions 984 → 1064.

## Beta Commit 208

- **Plasma, Monad and World Chain join the registry as EVM mainnets**
  (`core/data/chains.toml`, `chain-ui.toml`, `endpoints.toml`, `explorers.toml`,
  `tokens.toml`, `chain-wiki.toml`, `crypto-wiki.toml`; `core/src/registry.rs`)
  - Plasma (chain id 9745, XPL), Monad (143, MON) and World Chain (480, ETH)
    derive and sign on the shared EVM paths with their own endpoints,
    deployments, history sources and artwork; World Chain uses the `worldcoin`
    artwork, its native ETH the Ethereum mark.
  - Catalog deployments: Plasma EURC and USDC, Monad CAKE and USDC, World Chain
    USDC and WLD.
  - Plasma and World Chain have keyless history and discovery; Monad has neither
    (a new open item).
- **EVM previews, construction and signing reserve one complete fee budget**
  (`core/src/send/ethereum.rs`, `send/evm.rs`, `api/evm_json_rpc.rs`,
  `service/send_preview.rs`, `service/send_stage_protocols.rs`,
  `service/send_stages.rs`)
  - Preview and construction share gas estimation and registry-owned margins:
    Monad 7.5%, an exact 21,000-gas plain transfer unpadded (before: previews
    used unbuffered estimates and construction added 20%).
  - OP Stack networks add the L1 data fee and, where deployed, the operator fee
    read from the GasPriceOracle contract.
  - The budget is stored with the reviewed transaction, and fee growth at
    construction, signing or broadcast requires another review.
  - Construction and signing check the live native balance against value plus
    the full fee, and a standard ERC-20 transfer against the calldata amount and
    the contract's live balance.
  - Missing, malformed or out-of-range RPC nonces and gas limits refuse the
    preview instead of defaulting to zero or 21,000; the local nonce is reserved
    before the payload is priced.
  - A rollup history record without its prepared fee budget cannot be
    rebroadcast.
- **OP Stack confirmed receipts omit cost details**
  (`core/src/service/network.rs`, `service/pending_status.rs`): status,
  execution failure and block remain, a finalized receipt clears earlier partial
  costs, and Ethereum keeps its execution receipt cost (before: the execution
  gas cost was shown as the whole network fee).
- **A resolved on-chain failure is stored as `ExecutionFailed`**
  (`core/src/service/transactions.rs`, `service/pending_status.rs`), replacing
  an earlier "submission outcome unknown" or retries-exhausted reason; an
  unresolved timeout keeps its uncertainty reason.
- **Polkadot and Westend move to Asset Hub and are read through runtime
  metadata** (new `core/src/api/substrate_json_rpc/metadata.rs`;
  `api/substrate_json_rpc.rs`, `send/polkadot.rs`, `send/substrate.rs`,
  `service/endpoint_health.rs`; `core/data/endpoints.toml`)
  - Asset Hub endpoints with verified genesis hashes; a Relay Chain node is
    refused before balance or nonce reads.
  - Call indexes, signed-extension order, account layout and existential deposit
    come from the node's decoded metadata, and an unsupported layout fails
    before persistence or signing; transfers use `Balances.transfer_keep_alive`
    (before: a fixed call and legacy extensions).
  - The fee comes from `payment_queryInfo` on the exact signed envelope (before:
    a fixed DOT fee); funds checks keep the existential deposit and account for
    freezes and reserves.
  - Runtime, nonce, funds and fee are rechecked before signing and on every
    selected broadcast node; Substrate broadcasts, Bittensor included, require
    the returned hash to equal the submitted extrinsic's.
  - Pending sends scan finalized blocks for the extrinsic's `System` dispatch
    event under a persisted scan cursor; saved artifacts of the older shape must
    be rebuilt.
  - `spectra txs --poll-chain` keeps the full catalog transport, so persisted
    custom nodes are used.
- **Litecoin transparent sends spend the wallet's whole recovered account** (new
  `core/src/send/litecoin_quote.rs`, `service/send_stage_litecoin.rs`;
  `derivation/litecoin.rs`, `send/litecoin.rs`, `service/send_stage_utxo.rs`)
  - BIP44 P2PKH, BIP49 P2SH-P2WPKH and BIP84 P2WPKH derive on mainnet and
    testnet with their own address versions (before: BIP49 and BIP84 were
    refused).
  - Balances include every known address; fee previews and builds share
    confirmed inputs and script-aware virtual sizes; each input keeps its
    account path, and signing derives and verifies every source key and rechecks
    outpoints and amounts.
  - A mnemonic import stores a validated account public key, so receiving and
    recovery work on a password-protected wallet after a restart; the projection
    field `bitcoin_xpub` is renamed `account_xpub`, and a Litecoin mnemonic
    record without the key must be reimported.
  - Null or duplicate outpoints, zero values, unsupported scripts, conflicting
    accounts and amounts above MAX_MONEY fail before signing; the mainnet-only
    private-key wrapper is removed.
  - Litecoin Core's dust policy (30,000 litoshi/kvB; 5,460, 5,400 and 2,940
    litoshis for P2PKH, P2SH and P2WPKH): a below-dust recipient is refused
    before provider reads, below-dust change goes to the fee, and the default
    fee grows with virtual size to at least 1 litoshi/vB (before: Bitcoin's 546
    and a fixed 1,000).
- **Litecoin MWEB sends are removed** (`core/src/send/mweb.rs` deleted;
  `validation/address.rs`, `send/preview_decode.rs`, `send/flow.rs`): MWEB
  destinations are refused, and the extension builder and its `secp256k1zkp` and
  `blake3` dependencies are gone.
- **UTXO recovery scans receive and change to a real unused gap**
  (`core/src/service/address_discovery.rs`, `service/keypool.rs`,
  `derivation/path.rs`)
  - Each branch scans to 20 consecutive unused addresses beyond its known or
    reserved floor and fails explicitly at index 999 (before: receive only, to a
    fixed window).
  - Catalog purpose and coin plus a public branch and index are required and any
    account index is accepted; an imported address raises its branch's floor;
    Base58 keeps its case; derivation and provider errors are surfaced; CLI
    discovery uses the wallet's own network.
- **EVM imports keep each network's address and derivation path**
  (`core/src/derivation/import.rs`, `service/wallet_import.rs`,
  `store/wallet_domain.rs`): import derives only the selected networks and
  persists each with its own path; copied Ethereum/ETC sibling records are
  removed; every EVM network, Ethereum Classic and testnets included, uses the
  `ethereum` address-map slot.
- **Bitcoin watch imports validate the account public key** (`core/src/store/`,
  `cli/src/cmd/wallet.rs`): Base58Check, payload length, mainnet version and the
  encoded key are checked before persistence, and an xpub given with addresses
  is refused; `spectra wallet watch --xpub` is new, exclusive with `--address`.
- **XRP payments encode canonically and require acceptance**
  (`core/src/send/xrp.rs`, `api/xrpl_json_rpc.rs`)
  - `TxnSignature` sits before the AccountID fields, matching an official SDK
    vector (`core/tests/fixtures/xrp-payment.json`, `xrp-mnemonic-payment.json`,
    from `scripts/generate-xrp-payment-vector.cjs`).
  - Amount and fee must be positive and at most 10^17 drops; a sequence above
    `u32::MAX` is a decode error (before: it wrapped).
  - Submission needs `accepted: true`, `tesSUCCESS` or `terQUEUED` and a hash; a
    refusal is an error that keeps the signed payload for a retry.
- **EVM history decodes with each network's own tokens and pages to the end**
  (`core/src/service/history_refresh.rs`, `service/network_history.rs`): a
  testnet uses its own token descriptors, and a page ends only when both raw
  provider lists are short, so filtering unknown contracts no longer stops
  pagination.
- **Litecoin, Polkadot and XRP sends get CLI suites**
  (`scripts/cli-litecoin.py`, `cli-send-polkadot.py`, `cli-send-xrp.py`), with
  Asset Hub metadata fixtures in `core/tests/fixtures/`.
- **Four `scripts/__pycache__/*.pyc` files are committed.**
- Tests: 68 new, 4 removed
  - Litecoin 30: derivation, signing and quoting disagreeing on inputs, scripts,
    dust or fees; unowned, stale, duplicate or out-of-range inputs reaching
    signing; protected wallets failing to receive or sign after restart; MWEB
    destinations accepted.
  - UTXO recovery 7: scans stopping before the gap or at the ceiling silently,
    wrong branch floors, case-mangled witness addresses, unusable paths
    accepted.
  - Polkadot Asset Hub 7: wrong calls or extensions from metadata, a Relay Chain
    node accepted, keep-alive funds miscounted, finalized events misread.
  - EVM fees and new networks 13: unpadded or incomplete budgets, oracle
    failures passing, invalid nonces or gas defaulted, a grown fee broadcast
    without review, new chains missing assets, deployments or indexer methods.
  - XRP 5: a signature that differs from the SDK codec, amounts corrupting
    flags, sequence wrap, a refusal read as acceptance.
  - Transaction status 2: an on-chain failure left as uncertain, a timeout
    claimed as an execution failure.
  - EVM import paths 2, Bitcoin xpub validation 1, EVM history descriptors 1.
  - Removed: the three MWEB extra-output fee tests and
    `max_sendable_stops_at_zero`.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-10-04 (World Chain artwork, OP
  Stack receipt costs, execution failure, Asset Hub, Litecoin account sends,
  Litecoin dust, UTXO recovery gap, MWEB removal, Plasma/Monad/World Chain, EVM
  fee budget, EVM import paths, Bitcoin xpub validation, XRP encoding, EVM
  history, and an app entry on cancelled native reads); `OPEN-ITEMS.md` adds OP
  Stack receipt fee decoding and Monad history and discovery, and drops the
  Plasma and Monad support items.
- Total: 75 .rs files changed (9 new, 1 deleted), +7236 −1327 lines; data and
  config files: `scripts/` (9), `core/data/` (7), `core/tests/` (4),
  `scripts/__pycache__/` (4), `Cargo.lock`, `core/` (1); dependencies: added
  frame-metadata 23, parity-scale-codec 3, scale-info 2, removed blake3,
  secp256k1zkp (`core/Cargo.toml`); `#[test]` functions 920 → 984.

## Beta Commit 207

- **A token's standard belongs to its deployment, not its chain**
  (`core/src/registry.rs`, `tokens.rs`, `store/state.rs`, `store/mod.rs`,
  `store/wallet_domain.rs`, `service/network_tokens.rs`,
  `service/balance_refresh.rs`; `cli/src/cmd/token.rs`)
  - A chain's standard is a default; each deployment carries its own, and core
    validates chain, standard and identifier together (before: catalog and
    holdings required the chain's single standard).
  - Preference identity is the deployment id `chain:standard:identifier`
    (before: `chain|identifier`); duplicate normalized chain/identifier pairs
    are still refused, and reopening prefers the catalog over a custom protocol
    alias.
  - A successful balance refresh replaces the identifier's previous holding and
    removes protocol aliases; unread or unsupported tokens keep their last
    balance.
  - Tron TRC-10 ids can be stored beside TRC-20 addresses; TRC-10 metadata,
    balance, preview, build and send are refused before any request.
  - `spectra token add` takes `--standard`, and `token edit` refuses to change
    it; the chain-default contract validator and its error string are deleted.
- **Aptos reserves and signs the full gas budget** (`core/src/registry.rs`,
  `service/send_preview.rs`, `service/send_stage_protocols.rs`,
  `service/send_stages.rs`, `service/send_execution.rs`)
  - The registry owns a 10,000-unit gas limit, and previews reserve `gas unit
    price × limit` (before: the unit price alone was the fee, overstating the
    sendable maximum).
  - Owned sends carry the reviewed budget into the transaction; direct CLI
    builds price and persist one; signing checks the prepared gas against it;
    zero or out-of-u64 budgets and an insufficient live balance are refused
    before an artifact exists.
- **Native history labels come from the concrete network**
  (`core/src/fetch/history.rs`, `fetch/history_decode.rs`,
  `api/solana_json_rpc.rs`, `api/trongrid_v1.rs`, `send/icp_stages.rs`)
  - EVM records derive the native name and ticker from their network (before:
    caller-supplied, with mainnet labels on Sepolia and Fuji); the input fields,
    the EVM asset helper and four unused FFI records are deleted.
  - Solana and Tron transfer JSON no longer carries a ticker; ICP construction
    and fee validation share one registry currency value.
- **Fee estimates use registry units per family** (`core/src/registry.rs`,
  `service/send_preview.rs`): NEAR's 0.001 estimate is 10^21 units on both
  networks, giving its testnet an estimate and token gas floor; XRP, Stellar and
  Aptos live fees dispatch by family, so their testnets get them; TRX's native
  preview reads the registry estimate.
- **TON's native asset is Gram (GRAM), its test coin Test Gram (tGRAM), with a
  `gram` icon; WETH gets its own `weth` icon** (`core/data/tokens.toml`,
  `testnet-tokens.toml`, `chain-ui.toml`, `chain-wiki.toml`, `crypto-wiki.toml`;
  `icons/crypto/gram.svg`, `weth.svg`); TON's network names and artwork are
  unchanged.
- **Catalog corrections** (`core/data/tokens.toml`, `testnet-tokens.toml`):
  Ethereum RLUSD moves to Ripple's proxy
  `0x8292bb45bf1ee4d140127049757c2e0ff06317ed` (18 decimals); the undeployed
  Berachain ENA and Aptos CAKE are removed; Solana LINK 8 → 9 decimals, Sui
  FDUSD 9 → 6, Westend 10 → 12; Bera USD gets CoinPaprika `honey-honey1`.
- Measured: 2026-10-03 token audit (`docs/OPEN-ITEMS.md` "Token contract
  provenance"): 59 EVM deployments had code and matching decimals by RPC read,
  58 with matching metadata and Avalanche DAI reading `DAI.e`; issuer identity
  is unproven for all 59.
- Tests: 19 new, 2 removed
  - Token protocol identity 6: unknown standards, wrong networks or identifier
    shapes accepted; a standard alias duplicating a catalog asset; reopening
    keeping duplicate aliases.
  - TRC-10 5: a TRC-10 asset routed through the TRC-20 adapter or reaching RPC,
    storage or keys.
  - Aptos gas 4: a preview reserving less than the signed budget, a re-quoted
    budget at signing, zero or unrepresentable budgets.
  - Balance refresh 3: protocol aliases double-counting funds, unsupported
    protocols becoming a zero balance or losing their last one.
  - Fee estimates 1: testnets using mainnet endpoints for live fees.
  - Removed with the EVM native asset helper: `evm_native_asset_lookup`,
    `every_evm_chain_names_its_native_asset`.
- Docs: `BEHAVIOUR-CHANGES.md` entries (Aptos gas budget, native history
  metadata, fee estimate units, deployment-owned standards with Gram and WETH,
  audited token identities, and an app entry on native forms); `ARCHITECTURE.md`
  token standards per deployment; `OPEN-ITEMS.md` adds certifying the 59 token
  contract identities (with the provenance table), canonical TON jetton master
  identity, and the TRC-10 execution limitation.
- Total: 41 .rs files changed (1 new), +1656 −423 lines; data and config
  files: `core/data/` (5), `icons/crypto/` (2), `scripts/` (2); dependencies
  unchanged; `#[test]` functions 903 → 920.

## Beta Commit 206

- **Twelve tokens gain deployments on more chains** (`core/data/tokens.toml`);
  each shares its token's identity, artwork and price-provider ids
  - AAVE on Base, Optimism and Polygon; CRV on Avalanche C, BNB, Celo, Ink,
    Mantle, Polygon, Sonic and X Layer; CAKE on Arbitrum, Linea, opBNB, ZKsync
    Era and Aptos (`oft::CakeOFT`, 8 decimals).
  - DAI on Arbitrum, Optimism, Avalanche C (DAI.e), Linea, Scroll, ZKsync Era,
    Unichain and Celo; USDS on Avalanche C, Base and Unichain; sUSDS on
    Avalanche C and Unichain (Sky's token contract, not the supplied deployment
    account).
  - ENA and USDe on ZKsync Era, Optimism, Base, Mantle, Blast, Berachain, Linea,
    Scroll and X Layer; sUSDe on the same chains except Base (already listed),
    and on Aptos (fungible asset, 6 decimals).
  - ETHFI on Optimism; EURC on Base and Cronos; USDC on Celo, Cronos, Ink,
    Optimism, Sonic, Unichain, X Layer and ZKsync Era.
- **Chains carry concrete network names** (`core/data/chains.toml`): Arbitrum
  One, Polygon PoS, Avalanche C, BNB Smart Chain, Cronos EVM, HyperEVM, ZKsync
  Era; testnets Bitcoin Testnet 3 and 4, Decred Testnet 3, Kaspa Testnet 10,
  Avalanche Fuji C, BNB Smart Chain Testnet, Cronos EVM Testnet, HyperEVM
  Testnet, ZKsync Era Sepolia.
- **Provider-id tests no longer keep per-token lists** (`core/src/tokens.rs`):
  which tokens may lack a CoinPaprika id is decided by the catalog alone.
- Measured: 2026-10-03, read-only RPC reads of the added contracts
  (`docs/BEHAVIOUR-CHANGES.md` entries of that date): chain ids, symbols and
  decimals matched the catalog; the supplied Avalanche sUSDS address had no
  bytecode.
- Tests: 0 new, 2 removed — `deliberately_unlisted_tokens_have_no_paprika_id`,
  `catalog_tokens_carry_explicit_paprika_listings`.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-10-03 (ETHFI/EURC/USDC, TON
  tsUSDe removal, USDe/sUSDe, ENA, DAI, USDS/sUSDS, CRV, CAKE, AAVE, two on
  concrete network names); `OPEN-ITEMS.md` adds Plasma (EURC, USDC) and Monad
  (CAKE) support with their contracts.
- Total: 2 .rs files changed, +2 −35 lines; data and config files:
  `core/data/chains.toml`, `core/data/tokens.toml`,
  `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]` functions 905
  → 903.

## Beta Commit 205

- **UTXO sends pay the script the recipient address names** (new
  `core/src/derivation/utxo_address.rs`; `registry.rs`, `validation/address.rs`,
  `send/bitcoin_cash.rs`, `bitcoin_gold.rs`, `bitcoin_sv.rs`, `dash.rs`,
  `dogecoin.rs`, `litecoin.rs`)
  - One typed parser validates network and checksum and yields a P2PKH, P2SH or
    supported witness script (before: Bitcoin Cash, BSV, Dogecoin, Litecoin,
    Dash and Bitcoin Gold paid P2PKH to a P2SH recipient).
  - Bitcoin Cash CashAddr decodes its version byte and payload, and equal legacy
    and CashAddr destinations produce the same output; Litecoin pays witness v0
    and v1 (32-byte program), Bitcoin Gold witness v0 only; other versions and
    malformed CashAddr are refused before inputs are fetched.
  - Sender and change addresses must be P2PKH; Dash Testnet's sender no longer
    goes through a mainnet decoder; CashAddr, witness and MWEB spellings
    canonicalize to lowercase.
  - Address facts are on `Chain` (`fixed_utxo_address_versions`,
    `fixed_utxo_segwit_hrp`, `cashaddr_prefix`, `fixed_utxo_supports_witness`);
    the per-chain derivation files lose their own encoders.
- **Removing a wallet commits before any secret is deleted**
  (`core/src/service/state.rs`, `wallet_db/state.rs`, `wallet_db/connection.rs`)
  - Removal and a secret-cleanup queue commit together in SQLite, and a database
    failure keeps the wallet and its secrets (before: secrets were deleted
    first).
  - A committed removal returns even when cleanup fails, records a warning and
    retries the queue on reopen and later writes; an uncleared wallet id cannot
    be reused; a service with a secret backend must open its database before
    deleting wallets.
- **Token balances read on the wallet's concrete network**
  (`core/src/service/network_tokens.rs`, `cli/src/cmd/token.rs`): the protocol
  is chosen by family, so Tron, Solana, NEAR, Sui and TON testnets read their
  tokens (before: mainnets only), and `spectra token discover` queries the
  wallet's own network instead of its mainnet.
- **ICP and Zcash one-shot send paths removed** (`core/src/send/icp.rs`,
  `send/zcash.rs`, `api/blockbook.rs`): Rosetta `sign_and_submit` and
  `sign_zcash_and_broadcast` with their test-only helpers.
- **`IcpStakingClient` and `PolkadotStakingClient` drop endpoint lists they
  never read** (`core/src/staking/icp.rs`, `staking/polkadot.rs`).
- **Monad and Plasma get icons; sUSDS and Raydium are redrawn**
  (`icons/crypto/`)
  - `monad.svg` and `plasma.svg` are added, Monad in direct 64×64 coordinates
    with no transform.
  - The sUSDS disc loses its outer gradient stroke.
  - The Raydium mark is enlarged.
- **UTXO sends and wallet deletion get CLI suites**
  (`scripts/cli-send-utxo.py`, `scripts/cli-wallet-deletion.py`).
- Tests: 10 new, 10 removed
  - UTXO outputs 5: a signer paying a different script type than the address
    names, CashAddr and legacy diverging, unsupported witness versions or
    wrong-network addresses accepted.
  - Wallet deletion 4: secrets deleted before the database commits, partial
    cleanup not retried, a failed acknowledgment unsafe to retry.
  - Zcash 1: preparation reading inputs or broadcasting without a chain tip.
  - Removed: the ICP one-shot signing tests, three self-test suite tests merged
    into one, the address deriver/decoder tests replaced by the parser tests,
    and two network-family token tests.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-10-02 (sUSDS icon, Monad
  artwork, wallet removal, token queries on the concrete network, UTXO recipient
  scripts, and app entries on late seed reads and Monero scan cancellation);
  `OPEN-ITEMS.md` staking item no longer asks to drop ICP and Polkadot endpoint
  fields.
- Total: 60 .rs files changed (4 new, 1 deleted), +1227 −1320 lines; data and
  config files: `icons/crypto/` (4), `scripts/` (4), `Makefile`; dependencies
  unchanged; `#[test]` functions unchanged at 905.

## Beta Commit 204

- **Four catalog tokens get their own icons**
  (`icons/crypto/lombardbitcoin.svg`, `pythnetwork.svg`, `raydium.svg`,
  `rocketpooleth.svg`; `core/data/tokens.toml`): Lombard Staked Bitcoin, Pyth
  Network, Raydium and Rocket Pool ETH name new artwork (before: an empty
  `artwork_name`); Lombard's colour orange → green.
- **Peercoin gets an icon and PEPE's is redrawn** (`icons/crypto/`)
  - `peercoin.svg` is added; no catalog entry names it.
  - `pepe.svg` is redrawn.
- **Icon normalization rescales correctly around paint servers**
  (`scripts/svgo.config.mjs`): `<defs>`, `<clipPath>`, `<mask>` and paint
  servers are skipped, and a shape filled or stroked with `url(…)` is not
  rescaled by its attributes.
- Total: no .rs files changed; data and config files: `icons/crypto/` (6),
  `core/data/` (1), `scripts/` (1); dependencies unchanged; `#[test]`
  functions unchanged at 905.

## Beta Commit 203: Migration done

- **Rule 0 moves into `AGENTS.md`, and `docs/PLAN.md` is deleted**: the rule,
  what it does not license, and the prelaunch no-migration rule head
  `AGENTS.md`; `ARCHITECTURE.md` gains "Rules for keeping the shell thin";
  `README.md`, `FFI-BOUNDARY.md`, `FUTURE_PLANS.md` and `BEHAVIOUR-CHANGES.md`
  point to `AGENTS.md` and `OPEN-ITEMS.md` instead.
- **A short wallet password carries core's minimum**
  (`core/src/validation/mod.rs`): `TooShort { min_chars }`; `spectra wallet
  check-password` prints `{"tooShort":{"minChars":4}}` (before: a bare
  `tooShort`).
- **Registry guard test messages say to record a widening in
  `BEHAVIOUR-CHANGES.md`** (`core/src/derivation/dispatch.rs`,
  `staking/service.rs`; before: `PLAN.md`).
- Docs: `AGENTS.md` "Rule 0" and "Architecture"; `ARCHITECTURE.md`
  introduction and "Boundary design"; `FUTURE_PLANS.md` links and a new
  "Android front end" item; `OPEN-ITEMS.md` introduction and `AppState` item;
  `BEHAVIOUR-CHANGES.md` password-minimum entry and an app entry on token and
  diagnostics-import failures; `README.md`, `FFI-BOUNDARY.md` links; `PLAN.md`
  deleted.
- Total: 4 .rs files changed, +25 −16 lines; data and config files:
  `scripts/cli-acceptance.sh`, `scripts/cli-wallets.py`; dependencies
  unchanged; `#[test]` functions unchanged at 905.

## Beta Commit 202

- **Core's refusals carry a translatable template** (`core/src/lib.rs`
  `CoreMessage`; `derivation/error.rs`, `send/error.rs`, `api/error.rs`,
  `wallet_db/error.rs`, most `derivation/` and `send/` files)
  - `SpectraBridgeError::InvalidInput` and `Failure` carry a `CoreMessage`: an
    English `template` with a `%@` per `args`, built by
    `SpectraBridgeError::refused`/`failed` and `DerivationError::refused`
    (before: English strings).
  - Sentences that gained values are reworded (insufficient funds, Monero seed
    length and word, derivation path segments, watch-only chains); transport,
    parser and library text carries no args.
- **Imports are refused when they would be narrowed**
  (`core/src/derivation/import.rs`, `service/wallet_import.rs`,
  `cli/src/cmd/wallet.rs`)
  - `WalletImportRequest::check_shape` runs before deriving or sealing: a
    private-key or watch-only import takes exactly one chain, watched addresses
    and an account xpub are refused outside a watch-only import or on a chain
    that does not take them, and an import cannot be both.
  - The planner refuses a signing wallet without an address on its own chain;
    `derive_private_key_import_address` takes one chain.
  - The CLI passes every `--chain` to core and no longer derives the address
    itself (before: several `--chain` with a private key imported the first and
    dropped the rest).
- **The refresh engine revalues a new display currency**
  (`core/src/fetch/refresh_engine.rs`, `service/app_refresh.rs`,
  `service/network_prices.rs`): on a currency change while the app is active it
  runs a `Revalue` refresh (prices and rates when due, then alerts and
  movement); `AppRefreshIntent::BalancesUpdated` is renamed `Revalue`; rates are
  fetched for every currency at once.
- **The display currency is an ordinary setting** (`core/src/store/state.rs`):
  `AppSettingUpdate::FiatCurrency` through `SetAppSetting`;
  `StateCommand::SetFiatCurrency` and `FiatCurrencyChanged` are removed.
- **Quote and rate failures are typed** (`core/src/store/state.rs`,
  `service/network_prices.rs`, `cli/src/cmd/market.rs`): `quotes.pricesError`
  and `fiatError` are `QuoteRefreshFailure` (`noUsableQuote` or `unreachable {
  detail }`), stored as `{"kind":…}` (before: English sentences or raw transport
  text); the CLI prints them prefixed `prices:` or `fiat rates:`.
- **`FundsScanRead.error` is a `SpectraBridgeError`**
  (`core/src/service/funds_scan.rs`; before: the error flattened to a string).
- **The chain wiki is sorted by name and drops derivation paths**
  (`core/src/chains.rs`): `ChainWikiEntry.derivation_path` is removed; paths
  stay on `ChainEntry`.
- **New export `input_bounds()`** (`core/src/store/state.rs`): the reducer's
  token-decimals maximum (30) and large-movement alert percent range, for the
  controls that set them.
- Tests: 8 new
  - Import shape 4: a private key or watch-only import on several chains,
    watched addresses or an xpub on the wrong import or chain, a signing wallet
    stored without its address.
  - Messages 1: values not rendered into a template.
  - Revalue 1: a new display currency not revalued by core.
  - Chain wiki 1: chains out of name order.
  - Input bounds 1: a control offering a bound the reducer refuses.
- Docs: `OPEN-ITEMS.md` new, holding the open items formerly in `PLAN.md` (FFI
  record fields with no writer, staking execution, staking reads through
  adapters, FFI type names, `AppState` domains); `PLAN.md` loses "Known open
  items"; `FFI-BOUNDARY.md` describes `CoreMessage`; `README.md` and
  `FUTURE_PLANS.md` link `OPEN-ITEMS.md`; `BEHAVIOUR-CHANGES.md` entries (import
  shape and translated refusals, quote failures, display-currency revaluation
  and setting, Funds Finder errors, Crypto Wiki, and app entries on decimal
  separators and saving a recipient).
- Total: 75 .rs files changed, +949 −404 lines; data and config files:
  `scripts/cli-acceptance.sh`, `scripts/cli-diagnostics.py`; dependencies
  unchanged; `#[test]` functions 897 → 905.

## Beta Commit 201

- **The refresh engine follows committed wallet state**
  (`core/src/fetch/refresh_engine.rs`, `service/app_refresh.rs`,
  `service/state.rs`)
  - Core publishes a revision for every committed state; once device conditions
    are reported, a wallet whose fetch inputs are new or changed is refreshed at
    once with the new `Wallets` intent (balances and history, then prices), and
    an idle sweep timer starts one interval later. A removal or balance-only
    change refreshes nothing.
  - `reconcile_wallets` and `start` are no longer exported (before: the app
    called `reconcileWallets()` on every projection write and also ran a full
    refresh after import).
  - The portfolio snapshot carries `wallet_identity_revision`, bumped when a
    wallet is added, removed or changed apart from its balances.
  - `TransactionStatusChange.notify` says whether a change warrants a
    notification: confirmed or failed while status notifications are on.
- **A wallet's network is fixed at import; per-family network selection is
  removed** (`core/src/store/state.rs`, `derivation/import.rs`,
  `service/keypool.rs`, `service/diagnostic_state.rs`, `wallet_db/state.rs`,
  `registry.rs`; `cli/src/cmd/network.rs`)
  - `AppSettings.selected_chain_by_family`,
    `StateCommand::SelectChainForFamily`, `SelectedChainChanged`, the switch's
    keypool and history reset, `ImportNetworks`, `NetworkChoice` and `spectra
    network` are removed.
  - Import stores only the named network's address, plus the shared EVM/ETC slot
    (before: an address for every network of each family).
  - Watch-only addresses and account xpubs are judged on the chain picked, so a
    testnet address or `tpub` for a mainnet watch import is refused; new export
    `is_valid_watch_only_address`, and `validate_address` is no longer exported.
  - Diagnostics, endpoint probes and self-tests run on the network named; the
    diagnostics bundle carries every network's document, testnets included.
- **Core seals seeds and private keys under its own device key** (new
  `core/src/store/device_key.rs`; `store/secret_store.rs`, `secret_backends.rs`,
  `seed_envelope.rs`, `wallet_secrets.rs`)
  - Material is sealed with or without a wallet password, and core owns the
    key's rules: mint only when absent, never replace an unreadable key, at most
    one mint per process.
  - `SecretStore` gains `wrap_device_key`/`unwrap_device_key` and a `DeviceKey`
    class; the CLI's file store keeps the key unwrapped beside what it seals
    (before: an unpassworded CLI wallet stored base64 of the phrase).
  - The key lives at `device_key/signing-material`; the three envelope FFI
    exports are removed; seeds stored by earlier builds cannot be opened.
- **Operational log lines are redacted when stored**
  (`core/src/service/diagnostic_state.rs`): `DiagnosticState::append` redacts
  64-digit hex keys, extended private keys and runs of 12+ BIP-39 words in
  category, message, source and metadata; `wallet_id` and `transaction_hash` are
  kept (before: only the diagnostics bundle was sanitized).
- **A history page above the cap is cut to it**
  (`core/src/service/history_query.rs`, `history_cursor.rs`): at most
  `HISTORY_PAGE_MAX` (200) rows with `hasMore` and a cursor; only a zero limit
  is refused (before: any limit above 200 was refused).
- **Send artifacts carry the asset's symbol** (`core/src/send/stages.rs`,
  `service/send_records.rs`, `service/send_stages.rs`): `SendArtifact.symbol` is
  set at build from the history record's token lookup and bound into the review
  digest; `spectra send list` shows it.
- **Display order is decided once, in core**: known tokens sort in
  `store::sort_token_preferences` (built-ins first, then symbol, token id,
  chain), and each wallet's holdings come most valuable first, unpriced after,
  ties by symbol (`service/valuation.rs` `order_holdings_by_value`), as `spectra
  token list` and `spectra portfolio` show.
- **`verify_send_destination` is no longer an FFI export**
  (`core/src/service/send_destination.rs`); the CLI and the review call it in
  Rust.
- **The FFI export counter is deleted** (`scripts/count-exports.sh`).
- Tests: 11 new, 7 removed
  - Device key 4: a second mint, an unreadable key replaced, opening that mints,
    unpassworded material stored unsealed.
  - Refresh and notifications 2: balance changes bumping the identity revision,
    status notifications while the setting is off.
  - Log redaction 1, history page cap 1, artifact symbol 1, holdings order 1,
    token list order 1.
  - Removed with per-family selection, the old master-key envelope and the
    EIP-155 id table.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-10-02 (refresh engine and
  notifications, fixed wallet network, token order, holdings order, endpoint
  probe network, single recipient resolution, artifact symbol, log redaction,
  history page cap, device key, and app entries on error sentences, secret
  entry, appearance and the send network card); `FFI-BOUNDARY.md` and `PLAN.md`
  drop `count-exports.sh`; `scripts/README.md` file count 31 → 30.
- Total: 53 .rs files changed (1 new, 1 deleted), +1119 −1123 lines; data and
  config files: `scripts/` (7); dependencies unchanged; `#[test]` functions
  893 → 897.

## Beta Commit 200

- **Seed-phrase verdicts name a typed problem** (`core/src/validation/mod.rs`;
  `cli/src/cmd/mod.rs`, `cmd/wallet.rs`): `SeedPhraseVerdict.problem:
  SeedPhraseProblem` (`NonStandardLength`, `WrongWordCount`, `InvalidChecksum`)
  replaces the English `error` and `length_warning`; a fixed non-standard length
  is named at once; the CLI words the problem itself.
- **Staking facts are a core table** (new `core/data/staking.toml`;
  `core/src/chains.rs` `list_staking_chains()`; `spectra staking chains`): APY
  estimate, minimum stake, unbonding period and explanation for the six staking
  chains; loading refuses a table that misses a staking chain or names another;
  the Solana and NEAR explanations describe the protocol instead of saying
  Spectra acts.
- Tests: 2 new — staking table 2: a table missing or adding a chain, copy
  claiming Spectra signs.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-10-02 (BIP-39 lengths and
  typed problem, staking facts, and app entries on EVM layer-2 watch imports,
  the funds finder grid and the risk summary's locale).
- Total: 6 .rs files changed, +229 −70 lines; data and config files:
  `core/data/staking.toml`, `scripts/cli-acceptance.sh`; dependencies
  unchanged; `#[test]` functions 891 → 893.

## Beta Commit 199

- **Testnet tokens are named as test coins** (`core/data/testnet-tokens.toml`,
  `core/src/tokens.rs`): every entry reads "Test …" ("Test Bitcoin", "Test USD
  Coin"), and loading refuses a testnet token whose name does not start with
  "Test " (before: the mainnet token's name).
- Docs: `BEHAVIOUR-CHANGES.md` "Testnet tokens are named as test coins".
- Total: 3 .rs files changed, +13 −12 lines; data and config files:
  `core/data/testnet-tokens.toml`, `scripts/cli-acceptance.sh`; dependencies
  unchanged; `#[test]` functions unchanged at 891.

## Beta Commit 198

- **No code changed in this repository**: `BEHAVIOUR-CHANGES.md` records an app
  change, the backdrop's drifting colour clouds.
- Docs: `BEHAVIOUR-CHANGES.md` "The backdrop's colour clouds drift".
- Total: No code changed.

## Beta Commit 197

- **Token balances come from one holdings listing where a service offers it**
  (`core/src/service/network_tokens.rs`, `service/balance_refresh.rs`,
  `api/blockscout.rs`, `api/toncenter_v3.rs`, `api/evm_json_rpc.rs`;
  `core/data/endpoints.toml`, `chains.toml`)
  - A refresh first lists what the address holds (Solana and Sui nodes,
    TronGrid, TON Center v3, and Blockscout/Routescan, whose rows now declare
    `token-discovery`); a known token takes the listed balance, an unlisted one
    is zero, an unknown listed token is ignored.
  - With no listing service or a failed listing, per-token reads are used, EVM
    ones in concurrent JSON-RPC batches of 20 (before: every token read
    separately, one after another).
  - TON reads `jetton` as an address string and matches masters by raw address
    (before: every jetton read failed); Aptos no longer claims
    `token-discovery`.
  - `enumerates_holdings` is removed from `chains.toml`; under `spectra refresh
    --endpoint` catalog indexers are not consulted; `spectra token discover`
    reports only catalog symbols.
- **Aptos balances are read through view functions**
  (`core/src/api/aptos_rest.rs`): `POST /view` with
  `primary_fungible_store::balance` and `fungible_asset::decimals` for a
  metadata address, `coin::balance<T>` and `coin::decimals<T>` for a coin type;
  APT is read as the `AptosCoin` coin and an owner with neither store reads
  zero; `aptos-testnet` tokens are read too (before: `CoinStore` resources,
  which failed for every catalog token and for migrated APT).
- **Every known token is read; the enabled flag is removed**
  (`core/data/tokens.toml`, `testnet-tokens.toml`, `core/src/store/state.rs`,
  `store/mod.rs`, `send/transfer.rs`, `service/send_preflight.rs`;
  `cli/src/cmd/token.rs`)
  - `enabled`, `StateCommand::SetTokenPreferencesEnabled`,
    `CoreTokenPreferenceKey` and `spectra token track`/`untrack` are removed;
    balance refresh, history decoding, send preflight and the transfer builder
    no longer skip a token.
  - A catalog merge replaces the stored built-ins and keeps custom rows.
- Tests: 5 new, 2 removed
  - Holdings listing 4: non-fungible Blockscout rows kept, TON masters or
    decimals misread, a refusal read as an empty list, a known token missing
    from the list keeping a stale balance.
  - Aptos 1: fungible assets or coins read through resources that do not exist.
  - Removed with the enabled flag:
    `tracking_is_a_flag_and_a_group_moves_together`,
    `new_network_deployments_inherit_the_tokens_saved_choice`.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-10-01 (Aptos view functions,
  one listing per wallet, every known token read, and an app entry on pushed
  setup pages).
- Total: 24 .rs files changed, +1018 −872 lines; data and config files:
  `core/data/` (4), `scripts/` (3); dependencies unchanged; `#[test]`
  functions 888 → 891.

## Beta Commit 196

- **The chain picker ranks every mainnet and filters by tags**
  (`core/data/chain-ui.toml`, `core/src/chains.rs`, `cli/src/cmd/chain.rs`)
  - `ChainCategory` is removed; every mainnet has a `popular_rank` 1..=46
    without gaps and `tags` from the closed `ChainTag` set; `layer-1`, `evm` and
    `testnet` are derived and refused if written; a testnet takes its mainnet's
    rank and tags plus `testnet` (before: four categories and eight ranked
    chains).
  - `spectra chains` lists in popular order, prints `tags` and takes `--tag`.
- **A seed phrase's length and wordlist are inferred from its words**
  (`core/src/validation/mod.rs`; `cli/src/cmd/wallet.rs`, `cmd/mod.rs`)
  - `SeedPhraseCheck.word_count` and `language` are optional; core takes the
    shortest BIP-39 length holding every filled slot (at least 12) and the
    wordlist holding the most words (ties to the checksum, then English); typos
    are named against that list; more than 24 words is named, not cut (before:
    any wordlist's word counted as valid).
  - The verdict reports `word_count` and `language` and whether each was
    inferred; new export `seed_phrase_languages()`.
  - New `spectra wallet check-seed`; `wallet import` names an unfinished length
    and the detected list in its refusal.
- **Cardano and Substrate take entropy from the phrase's own wordlist**
  (`core/src/derivation/primitives.rs`, `derivation/cardano.rs`): both use
  `parse_mnemonic` and `mnemonic_entropy` from the parsed word indices, and
  `resolve_bip39_language` loses its default to English (before: a non-English
  phrase was refused for Cardano, and a phrase valid in both Chinese lists
  panicked a Polkadot import with exit 101).
- Tests: 14 new
  - Seed inference 7: phrases cut short or mis-sized, a fixed length silently
    truncating, the wrong wordlist detected, mixed wordlists passing.
  - Chain ranks and tags 4: unranked mainnets, testnets restating placement,
    tags disagreeing with the registry.
  - Wordlists and entropy 3: Cardano reading a non-English phrase as English,
    the two Chinese lists disagreeing, entropy re-detection on ambiguous
    phrases.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-10-01 (Cardano/Substrate
  wordlists, inferred length and wordlist, ranked chain picker, and an app entry
  on choosing the import method first).
- Total: 8 .rs files changed, +659 −124 lines; data and config files:
  `core/data/chain-ui.toml`, `scripts/cli-acceptance.sh`; dependencies
  unchanged; `#[test]` functions 874 → 888.

## Beta Commit 195

- **No code changed in this repository**: `AGENTS.md` adds the Swift Testing
  rule, `BEHAVIOUR-CHANGES.md` records an app launch and refresh change, and
  `PLAN.md` adds two open items (name FFI types once in Rust so Swift needs no
  typealiases; give `AppState`'s domains their own observable state).
- Docs: `AGENTS.md` Swift tests; `BEHAVIOUR-CHANGES.md` "Launch reads once;
  balance ticks no longer re-run unrelated work"; `PLAN.md` open items.
- Total: No code changed.

## Beta Commit 194

- **Core errors are typed per layer** (new `core/src/api/error.rs`,
  `derivation/error.rs`, `send/error.rs`, `wallet_db/error.rs`; `registry.rs`,
  `store/seed_envelope.rs`, `store/wallet_secrets.rs`; `cli/src/error.rs`)
  - `ApiError`, `DerivationError`, `SendError` (adding `InsufficientFunds`),
    `DbError`, `RegistryError`, `EnvelopeError` and `WalletSecretError` each
    convert to the fitting `SpectraBridgeError` variant (`Network`, `Decode`,
    `InvalidInput`, `Failure`); `From<String>` and `From<&str>` are removed
    (before: about 500 functions returned `Result<_, String>`, all reaching the
    FFI as `Failure`).
  - A UTXO shortfall reads "Insufficient funds for the amount plus the network
    fee." (before: the untranslated key `utxo.insufficientFunds`); registry
    lookups name the chain.
  - CLI exit codes follow the classification: `InvalidInput` exits 3, the rest
    1.
- **HTTP has one client and one retry loop** (`core/src/api/http.rs`):
  `http_request`, `http_post_json`, `HttpError`, `HttpHeader`, `HttpResponse`,
  `HttpTextResponse` and `HttpRetryProfile` are removed; a non-2xx answer other
  than 429/5xx is `ApiError::Status` and not retried; the Koios submit uses
  `HttpClient` and the diagnostics EVM probe `EvmClient`.
- **`core/src/app_core.rs` is split into `endpoints.rs` and
  `derivation/path.rs`, with FFI renames**: `chain_endpoints` →
  `endpoint_settings` (no longer throws), `AppCoreChainEndpoints` →
  `ChainEndpointSettings` (`grouped_settings` → `groups`),
  `AppCoreGroupedSettingsEntry` → `EndpointSettingsGroup`,
  `AppCoreEndpointRecord` → `EndpointRecord`; embedded catalogs panic on first
  use when broken (before: the endpoint catalog's load error was cached and
  returned to every caller).
- **A blank wallet password is refused, not read as no password**
  (`core/src/store/wallet_secrets.rs`, import path, `validation/`)
  - Only `None` stores material unsealed; a `Some` password blank after trimming
    fails with `WalletSecretError::EmptyPassword` when storing and reading, and
    `import_wallets` refuses it as `InvalidInput` before planning (the CLI exits
    3 and stores nothing).
  - `signing()` reports protection as `password.is_some()`;
    `validate_wallet_password` treats a whitespace-only password as `TooShort`;
    `--no-password` is the CLI's only way to an unsealed wallet.
- **CBOR goes through `ciborium`** (`core/Cargo.toml`), replacing `serde_cbor`
  (RUSTSEC-2021-0127) with canonical key order and byte-identical ICP envelopes.
- **Out-of-line tests move into each domain's `tests/` directory**
  (`service/tests/`, `send/tests/audit.rs`, `wallet_db/tests/`, `src/tests/`),
  and fixtures from `core/testdata/` to `core/tests/fixtures/`.
- Tests: 5 new (14 more moved out of `app_core.rs` with their code)
  - Blank password 3: a whitespace password stored unsealed, read, or accepted
    at import.
  - HTTP 1: a client-error status retried or swallowed.
  - ICP envelope 1: the CBOR library change altering signed bytes.
- Docs: `AGENTS.md` iOS 26 and Swift 6 rules; `ARCHITECTURE.md` test layout and
  per-layer error types; `BEHAVIOUR-CHANGES.md` entries dated 2026-09-30 (blank
  password, typed errors, and app entries on the display locale, refresh-event
  order, notification failures, app lock, pasted seed phrases and the history
  diagnostic deadline).
- Total: 188 .rs files changed (6 new, 1 deleted), +4534 −3326 lines; data and
  config files: `scripts/` (4), `core/tests/` (2), `Cargo.lock`, `core/` (1);
  dependencies: added ciborium 0.2, removed serde_cbor (`core/Cargo.toml`);
  `#[test]` functions 869 → 874.

## Beta Commit 193

- **Bitcoin Signet's coin displays `sBTC`** (`core/data/chain-ui.toml`,
  `testnet-tokens.toml`) and chain search matches it on `sBTC`; Testnet and
  Testnet4 keep `tBTC`.
- **Testnet stablecoins display `tUSDC` and `tPYUSD`**
  (`core/data/testnet-tokens.toml`).
- **38 testnets, Solana and Sui link transactions to an explorer**
  (`core/data/explorers.toml`, `core/src/explorers.rs`): each testnet points at
  its own network's explorer, Solana at Solscan and Sui at Suiscan; Kaspa
  Testnet has none.
- Tests: 0 new, 1 removed —
  `testnet_native_symbols_are_visibly_distinct_from_mainnet`.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-09-30 (Signet sBTC, testnet
  stablecoin symbols, testnet, Solana and Sui explorers).
- Total: 2 .rs files changed, +7 −16 lines; data and config files:
  `core/data/chain-ui.toml`, `core/data/explorers.toml`,
  `core/data/testnet-tokens.toml`, `scripts/cli-acceptance.sh`; dependencies
  unchanged; `#[test]` functions 870 → 869.

## Beta Commit 192

- **Token-2022 mints send when their extensions leave the transfer intact**
  (`core/src/api/solana_json_rpc.rs`, `send/solana.rs`)
  - Admitted by name: metadata, group and member records, mint close authority,
    permanent delegate, confidential-transfer configs, a transfer hook with no
    program, and a transfer fee that charges nothing now and as scheduled;
    everything else is refused, naming the extension (before: any extension was
    refused, so PYUSD could not be sent).
  - A mint with the fee extension is sent with `TransferCheckedWithFee` stating
    a zero fee.
- **Testnets host tokens; testnet USDC and PYUSD ship in the catalog**
  (`core/data/chains.toml`, `testnet-tokens.toml`; `core/src/registry.rs`,
  `chains.rs`, `wiki.rs`)
  - Each testnet carries its mainnet's `token_standard`, and
    `Chain::hosts_tokens` follows the standard alone (before: testnets refused
    every token).
  - `usd-coin-testnet` on the EVM, Solana, Sui, Aptos and NEAR testnets;
    `paypal-usd-testnet` on Ethereum and Arbitrum Sepolia, Polygon Amoy, X Layer
    Testnet and Solana Devnet; the asset wiki skips testnet deployments.
- **Seven EVM testnets join the registry** (`core/data/chains.toml`,
  `chain-ui.toml`, `endpoints.toml`): Linea Sepolia (59141), Celo Sepolia
  (11142220), Cronos Testnet (338), zkSync Era Sepolia (300), Sonic Testnet
  (14601), Ink Sepolia (763373) and X Layer Testnet (1952), each with its faucet
  coin, USDC and RPC rows; Blockscout history for Celo, zkSync and Ink Sepolia.
- Measured: 2026-09-30 (`docs/BEHAVIOUR-CHANGES.md`): each new chain id read
  back with `eth_chainId` and each USDC contract answered `USDC`/6; on Solana
  Devnet, a 1 PYUSD transfer simulated as `CreateIdempotent` then
  `TransferCheckedWithFee`, and stating a fee of 1 failed with "Calculated fee
  does not match expected fee".
- Tests: 5 new — Token-2022 5: PYUSD not sendable, a withholding fee or hook
  program admitted, unlisted extensions admitted, wrong instruction layouts.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-09-30 (Token-2022 extensions,
  testnet tokens and networks).
- Total: 7 .rs files changed, +333 −41 lines; data and config files:
  `core/data/chain-ui.toml`, `core/data/chains.toml`,
  `core/data/endpoints.toml`, `core/data/testnet-tokens.toml`; dependencies
  unchanged; `#[test]` functions 865 → 870.

## Beta Commit 191

- **No float on the send path** (`core/src/decimal.rs`, `send/flow.rs`,
  `send/mod.rs`, `send/preview_decode.rs`, `send/preview_types.rs`,
  `send/ethereum.rs`, `service/send_preflight.rs`, `fetch/history*.rs`)
  - Fees, spendable balances, maxima, fallback fees, NEAR's token gas reserve,
    the Sui gas budget, Cardano fee and rate, EVM custom fees, receipt costs and
    history amounts are exact decimal strings computed in smallest units and
    converted once with `decimal::from_units` (before: `f64`).
  - A maximum subtracts the fee exactly (`decimal::sub_or_zero`) and a shortcut
    is a share of the units (before: a ULP was subtracted, so a maximum of 4.2
    offered 4.199999).
  - EVM fees are whole wei: a custom fee finer than a wei, zero or past u64 is
    refused; the replacement bump is `ceil(wei × 1.2)`, at least 0.1 gwei.
  - Core's preview JSON carries decimal strings and wei integers and refuses a
    float; five unread preview records, three Dogecoin duplicate fields,
    `estimated_fee_rate_per_kb` and
    `ResolvedPendingStatus.confirmed_network_fee` are deleted.
- **Token balances are stored with every digit; one decimal formatter**
  (`core/src/decimal.rs`, `api/*.rs`): `decimal::from_units` and
  `from_unit_digits` replace sixteen per-chain formatters, several capped at six
  places; a malformed indexer amount is an error, not 0; unread `*_display`
  fields are deleted.
- **A chain crosses every boundary as `registry::Chain`** (about 160 record
  fields, service parameters, map keys and warning payloads)
  - `Chain` serializes, stores and prints as its catalog id, so stored JSON,
    columns and CLI output keep their spelling; an unknown id fails where it is
    read; string parsing remains only in CLI arguments, catalog TOML and
    database columns.
  - The family selection is `Chain → Chain`; the foreign-address send warning
    now covers the Tron, Solana, XRP and Monero testnets.
- **One rule decides whether a holding can be sent** (`send::SendAsset`,
  `Chain::sends_tokens()`; `service/send_preflight.rs`, `cli/src/cmd/tx.rs`)
  - Every chain sends its native asset; a token only when tracked and on EVM
    (except ETC and Hyperliquid), Solana, Tron or NEAR (before: two rules that
    disagreed, letting untracked EVM and Tron tokens through the preflight).
  - `SendRule`, `route_send_asset`, the `send_asset_routing` export and unread
    `SendPreflight` fields are removed; the preflight carries `chain` and the
    exact `amount`; `spectra send review` prints only `{"preflight": …}`.
- **73 endpoint rows added** (`core/data/endpoints.toml`)
  - Zelcore's Blockbook serves Zcash, Bitcoin Gold, Dash, Dogecoin, Litecoin and
    Bitcoin Cash, Atomic Wallet's Dash; the Zcash row claims no `fee`.
  - Fifteen testnets get providers on their mainnets' APIs (Litecoin, BSV, Kaspa
    TN10, Hyperliquid, Tron Nile, Solana Devnet, XRP, Stellar, Aptos, TON, NEAR,
    Westend, Monero stagenet, ETC Mordor, Avalanche Fuji history); no testnet
    row claims `staking`.
  - PublicNode, 1RPC, Blockscout, BlockCypher, mempool.emzy.de and Xray's Koios
    are listed on more of the chains they serve, giving Dash, Cardano Preprod,
    Sui Testnet and six EVM testnets their first endpoint; a testnet may name
    its own explorer.
- Measured: 2026-09-30, every added row probed live
  (`docs/BEHAVIOUR-CHANGES.md`); left out: 1RPC Mantle and Aptos, BlockCypher
  `btc/test3` (about 150,000 blocks behind), BlockPI except Sui, and
  `testnet.dcrdata.org` (503); no provider found for the Bitcoin Cash, Dogecoin,
  Zcash and Dash testnets.
- Tests: 8 new, 23 removed
  - Send rule 5: untracked tokens or watch-only wallets sending, a chain without
    a token builder offering tokens, zero amounts outside native EVM sends.
  - Decimal 3: digits lost at any size, subtraction below zero, extra places
    accepted, a token's own decimals ignored.
  - Removed: the routing-table and preview-shape tests that went with
    `route_send_asset` and `SendRule`, tests feeding unknown chain ids to
    removed branches, and float-conversion tests.
- Docs: `ARCHITECTURE.md` chains as `Chain`; `FFI-BOUNDARY.md` common-errors row
  `Unknown network: …`; `BEHAVIOUR-CHANGES.md` entries dated 2026-09-30 (no
  float on the send path, `Chain` at every boundary, exact token balances, one
  send rule, Zcash/Bitcoin Gold/testnet providers, providers on every chain they
  serve).
- Total: 164 .rs files changed (1 deleted), +4396 −5306 lines; data and config
  files: `core/data/endpoints.toml`, `scripts/cli-acceptance.sh`,
  `scripts/cli-endpoints.py`, `scripts/cli-history.py`; dependencies
  unchanged; `#[test]` functions 880 → 865.

## Beta Commit 190

- **CLI acceptance is confined to loopback** (`core/src/api/http.rs`,
  `core/src/tor.rs`, `service/endpoint_directory.rs`;
  `scripts/cli-acceptance.sh`, `cli-endpoints.py`)
  - When `SPECTRA_LOOPBACK_ONLY` names a journal file, endpoint selection leaves
    out remote endpoints, every other remote request goes to a dead proxy, an
    embedded Tor bootstrap is refused, and each is journaled with its target
    and command line; unset, nothing changes.
  - The acceptance script sets it for the whole run except `cli-transport.py`,
    fails on a non-empty journal, and first proves with a canary that a health
    probe of the built-in Bitcoin providers is caught (before: `cli-send.py`
    read Ethereum mainnet and sent its signed transaction to public nodes).
- Tests: 1 new — loopback 1: a non-local host treated as loopback.
- Docs: `BEHAVIOUR-CHANGES.md` "CLI acceptance is confined to loopback".
- Total: 3 .rs files changed, +105 −2 lines; data and config files:
  `scripts/cli-acceptance.sh`, `scripts/cli-endpoints.py`; dependencies
  unchanged; `#[test]` functions 879 → 880.

## Beta Commit 189

- **Every chain-service request moves into `core/src/api/`, one module per
  `EndpointApi`** (from `core/src/fetch/` and `core/src/send/`)
  - Each module is named after its API's `as_str()` (`esplora`, `whatsonchain`,
    `insight`, `koios`, `horizon`, `blockscout`, `blockcypher`, `toncenter_v2`,
    `toncenter_v3`, `tron_http`, `trongrid_v1`, `near_json_rpc`, `nearblocks`,
    `substrate_json_rpc`, `monero_daemon_rpc`, …) and holds that API's requests,
    parsing and submission; `api` depends only on its transport (`http`,
    `json_rpc`), shared answer types and `time`.
  - `send/` builds and signs, its fetch-then-build flows taking a client
    (before: eighteen `send/` files added methods to fetch clients).
  - `fetch/polkadot.rs` and `fetch/bittensor.rs` become one `SubstrateClient`.
  - NEAR history races every Nearblocks endpoint (before: only the first was
    asked).
- **Every endpoint is asked at once, in every API a chain speaks**
  (`core/src/api/http.rs`, new `api/utxo.rs`, `registry.rs`
  `Chain::endpoint_apis()`)
  - `race` answers with the first success and fails only when all fail (before:
    `with_fallback` tried endpoints in order, 180 ms apart).
  - Bitcoin uses Esplora and BlockCypher; Litecoin Blockbook, Esplora and
    BlockCypher; Dogecoin BlockCypher and Blockbook; Dash Blockbook and
    BlockCypher (before: one `primary_api()`, which left Litecoin without a
    working endpoint).
  - `UtxoClient` answers in one set of types: confirmed balance plus mempool
    delta, UTXOs including the mempool's, one newest-first history shape; only
    Esplora continues history past the first page.
  - BlockCypher broadcasts to `/txs/push` and quotes fees.
  - Broadcasts, the user's selected ones included, go to every endpoint
    together.
  - `NativeBalanceSummary.utxo_count` and `utxoCount` in `spectra balance` are
    removed.
- **Polkadot and Bittensor read balances from `System.Account` storage**
  (`core/src/api/substrate_json_rpc.rs`, `registry.rs`
  `substrate_balance_bytes`, `core/data/endpoints.toml`)
  - The record is decoded at u128 for Polkadot and u64 for subtensor; any
    other length is refused.
  - The balance is `free` less what `frozen` holds beyond `reserved`.
  - Their catalog rows declare `balance`; history stays unavailable.
- **Bitcoin Cash has an indexer** (new `core/src/api/bch_rest_v2.rs`;
  `core/data/endpoints.toml`, `service/send_preview.rs`)
  - `https://rest.bch.actorforth.org/v2` serves balance, history, UTXOs,
    status and broadcast through `UtxoClient`, without a fee quote.
  - The Litecoin/BCH preview uses 1 sat/B when no endpoint quotes one (before:
    it refused with "No fee endpoint configured").
- **Tron gains `https://tron-rpc.publicnode.com`** as a second node HTTP
  endpoint (`core/data/endpoints.toml`).
- **The Arbitrum mark is drawn smaller inside its disc**
  (`icons/crypto/arbitrum.svg`).
- Measured: 2026-09-29 (`docs/BEHAVIOUR-CHANGES.md`): OnFinality's public
  Hyperliquid RPC answered "Too Many Requests" after two calls and was left out;
  the Polkadot balance and Bitcoin Cash history CLI checks ran against the live
  networks.
- Tests: 14 new, 3 removed (22 more moved with their adapters)
  - UTXO client 4: APIs answering in different terms, a non-Esplora source
    continuing history, a broadcast sent to one endpoint only, another client
    speaking several APIs.
  - Substrate 4: wrong balance widths, frozen funds offered to a send, history
    returned as an empty list.
  - Bitcoin Cash adapter 3: BCH read as satoshis wrongly, history not netted, a
    node refusal read as a txid.
  - Race 1: a slow or dead endpoint holding up one that answers.
  - Endpoint APIs per chain 1: a chain speaking several APIs without a client
    that answers in all of them.
  - Solana associated token accounts 1: addresses differing from the SPL
    vectors.
  - Removed: `balances_render_with_trailing_zeros_trimmed` and the two stub
    clients' `missing_sources_are_errors_not_empty_wallets`.
- Docs: `AGENTS.md` one `api/<api>.rs` per `EndpointApi`; `ARCHITECTURE.md`
  module layout (`api/`); `PLAN.md` staking reads through adapters (new open
  item) and the Polkadot/Bittensor status; `BEHAVIOUR-CHANGES.md` entries dated
  2026-09-29 (Substrate balances and BCH indexer, one adapter per API, every
  endpoint asked at once).
- Total: 98 .rs files changed (11 new, 3 deleted), +5269 −4558 lines; data and
  config files: `core/data/endpoints.toml`, `icons/crypto/arbitrum.svg`,
  `scripts/cli-acceptance.sh`, `scripts/cli-transport.py`; dependencies
  unchanged; `#[test]` functions 868 → 879.

## Beta Commit 188

- **Donation addresses are a core catalog** (new `core/data/donations.toml`,
  `core/src/donations.rs`; `cli/src/cmd/chain.rs`)
  - `donation_destinations()` and new `spectra donations` give one address
    per network.
  - Loading refuses an unknown or test network, a second address for a
    network, and any address not already valid and normalized.
- **The endpoint catalog claims only what an adapter implements**
  (`core/src/endpoint_api.rs`, `core/data/endpoints.toml`)
  - A row claiming a capability outside `endpoint_capability_options` for its
    API is refused at load.
  - Eight adapter-less APIs (Blockchair, blockchain.info, BCH REST v2, SoChain
    v2, XRPScan, Substrate Sidecar, Tron JSON-RPC, Tronscan), their 12 rows,
    health checks and `probe_url` are removed; Bitcoin Cash is left with no
    endpoint.
  - TON v3 declares only token balance and discovery, TronGrid v1 only
    history, token history and token discovery.
- **Endpoint slots and capability strings become types**
  (`core/src/registry.rs`, `endpoint_api.rs`, `app_core.rs`,
  `service/endpoint_directory.rs`, `service/mod.rs`; `cli/src/cmd/chain.rs`)
  - `EndpointSlot` and `"chain:slot"` keys are removed; `Chain::primary_api()`
    names a chain's API, and other services come from `api_endpoints(chain, api,
    capabilities)`, custom endpoints first.
  - `EndpointCapability` is an enum with the same kebab-case names, used by
    catalog, records, custom endpoints, probes, overrides and `spectra endpoints
    --capabilities`; the `ENDPOINT_CAPABILITY_*` bitmasks are removed.
  - Custom endpoints store capabilities in declaration order (before: sorted
    alphabetically).
  - `endpoint_capability_id` is exported for localization.
  - `ChainEndpoints` refuses an unknown network.
  - `spectra endpoints --catalog` reports one `configured` row per chain.
- **Tron history reads TronGrid** (`core/src/fetch/tron.rs`)
  - `/transactions` and `/transactions/trc20` (confirmed only) give successful
    TRX transfers and TRC-20 `Transfer` events with base58check addresses and
    exact token amounts (before: history needed a Tronscan endpoint the
    catalog no longer had).
  - A failed read is an error (before: Tronscan's client returned an empty
    history on failure).
  - The Tronscan client is removed.
- Tests: 6 new, 2 removed
  - TronGrid history 3: TRX and TRC-20 transfers unmerged or unordered, a failed
    read returned as empty, inexact token amounts.
  - Donations 2: the shipped addresses failing to load, unsafe destinations
    accepted.
  - Capabilities 1: a capability name that does not round-trip.
  - Removed: the bitmask and slot-suffix tests.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-09-28/29 (donation addresses,
  adapter-backed catalog, TronGrid history, endpoint slots and capability
  strings).
- Total: 50 .rs files changed (1 new), +1152 −1105 lines; data and config
  files: `core/data/donations.toml`, `core/data/endpoints.toml`,
  `scripts/cli-acceptance.sh`, `scripts/cli-history.py`; dependencies
  unchanged; `#[test]` functions 864 → 868.

## Beta Commit 187

- **Transaction explorers move out of the endpoint catalog** (new
  `core/data/explorers.toml`, `core/src/explorers.rs`;
  `core/data/endpoints.toml`, `app_core.rs`, `registry.rs`)
  - One row per network: `chain_id`, the explorer's `name` and a `tx_url`
    template with `{hash}`; loading checks a known network, one row each, HTTPS
    and exactly one `{hash}`.
  - `Chain::transaction_explorer`, `transaction_explorers()` and
    `transaction_explorer_link()` replace
    `AppCoreChainEndpoints.transaction_explorer`, `AppCoreExplorerEntry`,
    `explorer_label` and `tx_suffix`.
  - Every endpoint row must declare an `api` and a capability, so
    `AppCoreEndpointRecord.api` and `EndpointProbe.api` are no longer optional;
    the 44 explorer rows and the Hyperliquid web link leave `endpoints.toml`.
  - New `spectra explorers --chain … --tx …`.
- Tests: 3 new, 2 removed — explorers 3: the shipped file not loading one row
  per network, the hash misplaced, malformed files accepted; the two
  explorer-link tests of the old catalog are removed.
- Docs: `BEHAVIOUR-CHANGES.md` "Transaction explorers moved out of the endpoint
  catalog".
- Total: 16 .rs files changed (1 new), +263 −170 lines; data and config files:
  `core/data/endpoints.toml`, `core/data/explorers.toml`,
  `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]` functions 863
  → 864.

## Beta Commit 186

- **Monero wallet-RPC remnants and `SendBroadcastMode` are removed**
  (`core/src/fetch/monero.rs`, `send/monero.rs`, `endpoint_api.rs`,
  `registry.rs`, `service/network_balance.rs`, `service/network_history.rs`,
  `service/send_broadcast.rs`; `cli/src/cmd/chain.rs`)
  - `SendBroadcastMode`, `ChainIdentity.send_broadcast_mode` and the CLI's
    `sendBroadcastMode` key are gone (every chain answered
    `SignsAndBroadcasts`).
  - `EndpointApi` loses `monero-wallet-rpc` and `monero-light-wallet` with their
    balance, history and broadcast branches and the wallet-RPC client; a custom
    endpoint declaring either is refused.
- Tests: 0 new, 1 removed — `sends_are_signed_on_device`.
- Docs: `BEHAVIOUR-CHANGES.md` "Monero's wallet-RPC remnants are gone".
- Total: 13 .rs files changed (2 deleted), +5 −276 lines; data and config
  files: `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]`
  functions 864 → 863.

## Beta Commit 185

- **Fee priority is removed** (`core/src/store/state.rs`, `app_core.rs`,
  `send/preview_decode.rs`, `send/preview_types.rs`, `service/owned_send.rs`,
  `service/send_preview.rs`; `cli/src/cmd/settings.rs`)
  - `AppSettings.fee_priority_by_chain`, its reducer case, `FeePriority`, the
    CLI `fee-priority.*` keys and Dogecoin's `feePriority` preview field are
    removed (no build, sign or estimate read them).
  - A stored `feePriorityByChain` is refused as an unknown setting.
  - `AppCoreChainEndpoints.bitcoin_esplora` is removed.
- **Core records chain diagnostics and builds the bundle**
  (`core/src/diagnostics/registry.rs`, `diagnostics/export.rs`,
  `service/diagnostic_state.rs`; `cli/src/cmd/diagnostics.rs`)
  - Core records history rows, the history-run time, endpoint probes and the
    check time.
  - `WalletService.chain_diagnostics` answers for a family on its selected
    network.
  - `diagnostics_bundle` (schema 2) builds the bundle from core state plus the
    platform's version, OS, locale and time zone.
  - The document names `chainId` and `network`, gives `null` times before a
    run and marks unprobeable endpoints `checked: false`.
  - `lastRunAt` for self-tests and rescans is removed.
  - `spectra diagnostics show` reads `chain_diagnostics` (before: empty
    endpoints and no times).
  - New `spectra diagnostics bundle`.
- **"Last checked" is core's clock; balances land per wallet**
  (`core/src/service/app_refresh.rs`, `fetch/refresh_engine.rs`)
  - `AppRefreshResult.pending_checked_at_unix` carries
    `refresh_clock.pending_transactions_at`, stamped only by a failure-free
    check.
  - The engine reports each wallet as its balances commit (before: after every
    wallet).
- Tests: 1 new, 5 removed — chain diagnostics 1: a bundle reading the wrong
  network; removed: four fee-priority tests and the per-chain document test.
- Docs: `ARCHITECTURE.md` per-wallet balance reports and diagnostics ownership;
  `BEHAVIOUR-CHANGES.md` entries dated 2026-09-28 (fee priority, chain
  diagnostics, last-checked clock, and an app entry on the keychain service
  name).
- Total: 19 .rs files changed, +488 −516 lines; data and config files:
  `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]` functions 868
  → 864.

## Beta Commit 184

- **Unwired send options are removed** (`core/src/send/bitcoin.rs`,
  `send/mod.rs`, `send/flow.rs`, `send/evm_overrides.rs`; `cli/src/cmd/tx.rs`)
  - Bitcoin extra outputs (`BitcoinExtraOutput`, the
    `extra_output_overhead_bytes` export).
  - The coin-selection strategy (`CoinSelectionStrategy`).
  - Both `sign_only` fields; `spectra send overrides` loses `--sign-only` and
    `signOnly` (`send broadcast --sign-only` is unchanged).
- **Dead core code is removed**: the `SignedSubmission` trait
  (`submission_id`, `signed_payload`, `signed_payload_format`) and
  `SignedPayloadFormat`; `wallet_db` load and delete helpers for wallets,
  addresses and the keypool, `delete_wallet_data` and `history_replace_all`;
  `Chain::rpc_health_method` and `Chain::testnets`;
  `SendExecutionShape.fee_decimals` and `supports_private_key`;
  `CoreSendVerificationStatus::Verified`, `register_owned_address`, the
  `fetch_fee_rate` stubs and Monero `create_account`. `is_private_key_backed`
  and `has_signing_material` become test helpers.
- **Comments lose their migration history** (core, the CLI,
  `scripts/cli-acceptance.sh`).
- **`service/state_performance_tests.rs` is renamed
  `address_discovery_tests.rs`.**
- Tests: 3 new, 11 removed
  - Send review 1: a changed destination signing under an old review.
  - Built-in tokens 1: a built-in removed, or toggled on one network only.
  - Keypool 1: an out-of-range baseline reserving addresses.
  - Removed with the deleted code: extra-output and coin-selection tests,
    fee-decimals and testnet send-shape tests, the verification-notice test and
    four `wallet_db` delete-helper tests.
- Docs: `BEHAVIOUR-CHANGES.md` "`send overrides` no longer echoes
  `--sign-only`"; `scripts/README.md` test organisation ("Former test files"
  table removed).
- Total: 118 .rs files changed (1 deleted), +736 −2487 lines; data and config
  files: `scripts/cli-acceptance.sh`, `scripts/uncalled-core-fns.sh`;
  dependencies unchanged; `#[test]` functions 876 → 868.

## Beta Commit 183

- **Bitcoin testnets and Signet get their own marks** (new
  `icons/crypto/bitcointestnet.svg`, `bitcoinsignet.svg`;
  `core/data/chain-ui.toml`, `testnet-tokens.toml`): Testnet and Testnet4 chains
  and coins name `bitcointestnet` (green disc), Signet `bitcoinsignet` (purple
  disc), instead of the mainnet `bitcoin` mark.
- Docs: `BEHAVIOUR-CHANGES.md` app entries (deleting a wallet asks once;
  holdings and chain-breakdown headers drop their counts).
- Total: no .rs files changed; data and config files:
  `core/data/chain-ui.toml`, `core/data/testnet-tokens.toml`,
  `icons/crypto/bitcoinsignet.svg`, `icons/crypto/bitcointestnet.svg`;
  dependencies unchanged; `#[test]` functions unchanged at 876.

## Beta Commit 182

- **No code changed in this repository**: `BEHAVIOUR-CHANGES.md` records two app
  changes to the receive flow (no page header; no bottom action bar or step
  indicator).
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-09-27 on the receive pages.
- Total: No code changed.

## Beta Commit 181

- **Undated unconfirmed transactions sort first**
  (`core/src/wallet_db/history.rs`, `wallet_db/history_query.rs`)
  - The history index sorts an undated pending record as 9999-12-31, first
    newest-first and last oldest-first, while its stored time stays unknown
    (before: sorted as the oldest).
  - A wallet's earliest transaction ignores undated rows.
- **A missing time is unknown for an unconfirmed transaction and an error for a
  confirmed one** (`core/src/fetch/*.rs`, `fetch/history.rs`, `fetch/mod.rs`,
  `service/history_refresh.rs`)
  - Blockbook, BSV, Decred and Dogecoin report an unconfirmed transaction's time
    as absent by block height, Sui by checkpoint, Solana when the node lacks
    `blockTime` (before: 0, shown as 1969/1970).
  - A confirmed transaction without a readable time fails the fetch, naming it;
    sources that list only confirmed transactions require one.
  - Normalization reads `null` as undated and refuses anything else that is not
    a positive number.
  - Stellar and Dogecoin parse RFC 3339 with the shared strict parser.
  - An undated record is stored with the unknown sentinel.
- **Dogecoin history nets each transaction and includes unconfirmed ones**
  (`core/src/fetch/dogecoin.rs`)
  - Each transaction's refs net into one entry (before: the first ref of each
    transaction only).
  - `unconfirmed_txrefs` are included as undated pending entries.
- Tests: 4 new — history times 4: Dogecoin legs not netted, an uncheckpointed
  Sui block dated, an undated entry stored as 1970, undated pending rows sorting
  last.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-09-27 (unconfirmed first and
  Dogecoin netting; undated transactions and misread dates).
- Total: 22 .rs files changed, +578 −237 lines; data and config files:
  `scripts/cli-history.py`; dependencies unchanged; `#[test]` functions 872 →
  876.

## Beta Commit 180

- **History can hide small amounts** (`core/src/service/history_query.rs`,
  `wallet_db/history_query.rs`; `cli/src/cmd/tx.rs`)
  - `HistoryQuery.hide_small_amounts` leaves out transfers below
    `HISTORY_SMALL_AMOUNT_THRESHOLD` (0.00001 of the row's asset), exported as
    `history_small_amount_threshold()`.
  - A cursor carries the setting and is refused by a query with the other.
  - New `spectra txs --page --hide-small-amounts`.
- **History clients report what each transaction moved for the address, fee
  excluded on account chains, and drop transactions that moved nothing**
  (`core/src/fetch/*.rs`)
  - Sui reads the address's SUI balance change as sender and recipient (before:
    every row a 0 SUI receipt).
  - TON drops external messages (before: a 0 TON receipt beside every send).
  - XRP reads the account's `AccountRoot` change and only successful payments
    (before: issued currencies as 0 XRP, partial payments at their ceiling,
    failures as transfers).
  - Stellar reads `create_account` and issued-asset payments by their own
    fields.
  - NEAR reads Nearblocks' `/account/{id}/txns` (before: a 404 path, so no NEAR
    history).
  - Solana reads lookup-table addresses, fee-only transactions and closed
    token accounts correctly; the dead `fetch_history` is removed.
  - Aptos counts only the framework's successful APT transfers.
  - ICP nets transfers, mints and burns.
  - Cardano and Blockbook take the address's net (before: the total output).
  - Kaspa reads the API's snake_case fields and input addresses.
  - Kaspa and Decred gain a normalization shape.
  - Bitcoin SV matches inputs against the address's own outputs, and an
    unreadable transaction fails the refresh.
  - Across normalized chains an unreadable amount refuses the row (before: 0).
  - Whole base units convert exactly.
  - EVM native ends compare lowercased.
- Tests: 15 new
  - History clients 14: zero or wrong amounts from each provider's shapes
    (Sui, TON, XRP, Stellar, NEAR, Solana, Aptos, ICP, Cardano, Kaspa, BSV,
    Blockbook, exact unit conversion).
  - Small amounts 1: the filter keeping dust or dropping the threshold.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-09-27 (hide small amounts,
  chain history amounts, and app entries on the portfolio card's wallet count
  and partial totals).
- Total: 20 .rs files changed, +1639 −811 lines; data and config files:
  `scripts/cli-history.py`; dependencies unchanged; `#[test]` functions 857 →
  872.

## Beta Commit 179

- **Transfer ends name who holds them** (`core/src/service/history_query.rs`,
  `service/mod.rs`; `cli/src/cmd/address.rs`, `cmd/tx.rs`)
  - `TransactionEndpoint.holder` is an `EndpointHolder` (`wallet { name }` or
    `contact { name }`), looked up in the transacting wallet, then the user's
    other wallets on the record's network (addresses and keypool rows), then
    that network's address-book entries, in the chain's normal form; `isMine` is
    unchanged.
  - New export `address_holder(wallet_id, chain_id, address)` and `spectra
    address holder --wallet <w> <address>`.
  - `spectra txs --endpoints` prints the holder.
- Tests: 4 new — endpoint holders 4: wrong precedence between wallet and
  contact, case-sensitive matching, a holder for an unknown address, the JSON
  shape.
- Docs: `BEHAVIOUR-CHANGES.md` entries dated 2026-09-27 (transfer ends name
  holders, and app entries on the send review, status badge, transaction detail,
  app cover, history reload and asset detail); `scripts/README.md` `cli-send.py`
  row.
- Total: 4 .rs files changed, +275 −9 lines; data and config files:
  `scripts/cli-send.py`; dependencies unchanged; `#[test]` functions 853 →
  857.

## Beta Commit 178

- **No code changed in this repository**: the 2026-09-26 `BEHAVIOUR-CHANGES.md`
  entry is corrected — the app's one-action Send was reverted, and Build, Sign
  and Broadcast stay three actions.
- Docs: `BEHAVIOUR-CHANGES.md` "Exported async methods run on core's own
  runtime".
- Total: No code changed.

## Beta Commit 177

- **Exported async methods run on core's own runtime** (new
  `core/src/worker.rs`; all 73 exported async methods in `core/src/service/`,
  `fetch/refresh_engine.rs`)
  - Each body is spawned by `worker::run` on a core tokio runtime with 8 MiB
    worker stacks; the caller awaits the join, dropping it aborts the body, and
    a panic resumes in the caller (before: UniFFI polled the body on the calling
    thread, a 512 KiB stack on iOS).
  - `WalletService`, `RefreshEngine` and `FundsScan` are cheap `Clone`s for
    this.
- **Send amount shortcuts are core's list** (`core/src/service/owned_send.rs`)
  - New export `send_amount_shortcut_percentages()` (25, 50, 75, 100).
  - A share is cut to the asset's display precision; the maximum stays exact.
- **The recipient check knows funded and own addresses**
  (`core/src/service/owned_send.rs`, `send_destination.rs`, `send_preflight.rs`)
  - `RecipientCheck` is a record with optional `activity` and `is_own_address`
    (before: an enum, `Unavailable` whenever the history read failed).
  - A funded destination is known used without a history read.
  - A self-send's review drops the `NewAddress` warning.
- Tests: 6 new
  - Worker 3: a body running off the core runtime, a dropped caller leaving the
    body running, a lost panic.
  - Send preview 3: an 18-digit share amount, a funded address reported unused
    when history fails, a self-send warned as a new destination.
- Docs: `BEHAVIOUR-CHANGES.md` "Exported async methods run on core's own
  runtime; one-action Send".
- Total: 37 .rs files changed (1 new), +2993 −2353 lines; data and config
  files: none; dependencies unchanged; `#[test]` functions 847 → 853.

## Beta Commit 176

- **Manifests trimmed** (`cli/Cargo.toml`, `core/Cargo.toml`, `ffi/Cargo.toml`):
  explanatory comments, `autobins = false` and `core`'s redundant `[lib] name`
  are removed; `build/` leaves `.gitignore`; behaviour unchanged.
- Docs: `ARCHITECTURE.md` — Xcode builds the Rust core into its own DerivedData
  (`spectra-rust/cargo-target`), not the workspace `target/`.
- Total: no .rs files changed; data and config files: `.gitignore`,
  `cli/Cargo.toml`, `core/Cargo.toml`, `ffi/Cargo.toml`; dependencies
  unchanged; `#[test]` functions unchanged at 847.

## Beta Commit 175

- **Toolchain 1.94.1 → 1.98.1** (`rust-toolchain.toml`), with the edits its
  lints ask for: `sort_by_key` with `Reverse`, `as_chunks`, a tuple `let` in
  Decred's BLAKE-256.
- **Crates renamed `spectra_cli` and `spectra_uniffi_bindgen`**
  (`cli/Cargo.toml`, `tools/uniffi-bindgen/Cargo.toml`), with the release
  overflow-check profile, CI, `scripts/cli-acceptance.sh` and `README.md`
  following; the profile's comment is shortened.
- **The Makefile drops `check`, `ios-artifacts`, `ios-artifacts-release`,
  `bindgen-ios` and `clean-generated`** (`Makefile`; Xcode's "Build Rust Core"
  phase runs `scripts/build-ios.sh`, which builds the library and generates
  the Swift bindings); `clean` removes the generated bindings itself.
- Docs: `README.md` "The iOS app"; `FFI-BOUNDARY.md` "Regenerating bindings";
  `scripts/README.md` build scripts (`bindgen-ios.sh` row removed).
- Total: 9 .rs files changed, +17 −23 lines; data and config files:
  `.github/workflows/` (1), `Cargo.lock`, `Cargo.toml`, `Makefile`, `cli/`
  (1), `rust-toolchain.toml`, `scripts/` (1), `tools/uniffi-bindgen/` (1);
  dependencies unchanged; `#[test]` functions unchanged at 847.

## Beta Commit 174

- **The workspace moves to Rust 2024** (`Cargo.toml` resolver 2 → 3; `edition =
  "2024"` in `core`, `cli`, `ffi`, `tools/uniffi-bindgen`): rustfmt's 2024
  import order and `assert!` layout across 165 files, and twelve nested `if
  let`s collapsed into let chains; behaviour unchanged.
- Total: 165 .rs files changed, +1601 −1282 lines; data and config files:
  `Cargo.toml`, `cli/Cargo.toml`, `core/Cargo.toml`, `ffi/Cargo.toml`,
  `tools/uniffi-bindgen/Cargo.toml`; dependencies unchanged; `#[test]`
  functions unchanged at 847.

## Beta Commit 173

- **Four holding-merge records are removed** (`core/src/store/mod.rs`):
  `HoldingMergeExistingInput`, `HoldingMergeIncomingInput`,
  `HoldingMergeAppendPayload` and `HoldingMergeAction`, which no function took
  or returned.
- **Unused dependencies removed**: `uuid` from the CLI and `uniffi` from the FFI
  crate.
- **`uncalled-core-fns.sh` reads only files git tracks or would track** (`git
  ls-files --cached --others --exclude-standard`), so ignored generated bindings
  no longer count as callers.
- Docs: `BEHAVIOUR-CHANGES.md` entries (dead code the scans missed, and an app
  entry on one wallet setup with advanced options); `scripts/README.md` lists
  the seven newer CLI suites and the file count (24 → 31).
- Total: 1 .rs file changed, +0 −46 lines; data and config files:
  `Cargo.lock`, `cli/Cargo.toml`, `ffi/Cargo.toml`,
  `scripts/uncalled-core-fns.sh`; dependencies: removed uuid
  (`cli/Cargo.toml`), uniffi (`ffi/Cargo.toml`); `#[test]` functions unchanged
  at 847.

## Beta Commit 172

- **Assets are identified by deployment, never by ticker** (`fetch/history.rs`,
  `fetch/transactions.rs`, `fetch/solana.rs`, `send/ethereum.rs`, `tokens.rs`,
  `registry.rs`)
  - A history row's contract or mint gives its deployment; a row without one is
    the network's own coin (Tron token rows need a contract, Solana SPL rows
    without a mint are skipped) (before: native-ness was decided by ticker).
  - A contract that does not normalize refuses the row.
  - A token row is named from the catalog entry for its deployment, or shown
    as its contract (before: by ticker, `token_name_on_chain`).
  - Transaction merges compare deployment ids only; the
    `merge_identity_includes_symbol` registry flag is removed.
  - `EvmSendAssemblyInput` carries `deployment_id` instead of `symbol`: the
    native deployment is a value transfer with no contract, and any other must
    carry the contract that derives exactly that deployment.
- **The wiki looks up coins by token id** (`core/data/crypto-wiki.toml`,
  `core/src/wiki.rs`)
  - Rows use `token_id` instead of `asset = "<ticker>"`; the text shown is
    unchanged.
  - Unknown keys are refused.
- **Core runs the maintenance loop and logs its own events**
  (`fetch/refresh_engine.rs`, `service/app_refresh.rs`,
  `service/operational_events.rs`, `service/transaction_recheck.rs`,
  `service/send_stages.rs`, `service/diagnostic_state.rs`)
  - Given `DeviceConditions`, core runs the balance sweep and the maintenance
    tick while the app is active and a wallet has something to fetch, and
    hands each `AppRefreshResult` to `on_refresh_complete`
    (`BalanceRefreshEngine` / `BalanceObserver` become `RefreshEngine` /
    `RefreshObserver`).
  - `AppRefreshResult` reports `transactions_changed` and `diagnostics_changed`.
  - Core writes English operational-log lines with `source: core` for status
    changes, broadcast outcomes, rescans, self-tests, refresh and pending
    failures and failed rechecks.
- **Core pushes Tor status and history paging state** (`core/src/tor.rs`,
  `service/history_cursor.rs`, `service/history_query.rs`)
  - Every Tor state change and bootstrap step is forwarded through
    `on_tor_status_changed`.
  - `TransactionSnapshot.wallets_with_more_history` replaces the exported
    `history_cursor`.
- **`preview_owned_send` checks the recipient beside the quote**
  (`service/owned_send.rs`, `send_preview.rs`; `cli/src/cmd/tx.rs`)
  - It returns `recipient` (`checked` with activity, or `unavailable`), and a
    failed recipient read leaves the quote standing.
  - CLI `send preview` prints it.
- **`Chain::accepts_account_xpub`** (Bitcoin only) replaces a chain comparison
  in the import planner.
- **Smaller removals** (`core/src/store/state.rs`, `store/wallet_domain.rs`,
  `send/flow.rs`)
  - The `providerState` reset scope (`settings reset --scope providerState` is
    refused).
  - `CoreSeedDerivationPaths.is_custom_enabled` (never read).
  - The `normalized_send_address` export.
- **The chain catalog's `category` is a typed `ChainCategory`**
  (`core/src/chains.rs`), so a misspelt category fails at load.
- Tests: 6 new, 1 removed
  - Deployment identity 3: a history row borrowing another asset's identity
    through its ticker, two tokens sharing a ticker merged, an EVM assembly with
    a mismatched deployment and contract accepted.
  - Refresh engine 1: device conditions not starting or stopping the maintenance
    loop.
  - Operational events 2: refresh and recheck outcomes not logged by core,
    appended lines not trimmed or capped at 800.
  - Removed: the Tron symbol merge-identity test.
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-24 entries (deployment identity in
  history, merges and EVM assembly; wiki by token id; the thinner Swift shell:
  authentication errors, one state-command queue, core-run maintenance and event
  log, pushed changes, bridge, account xpub fact, Secure Enclave–wrapped master
  key, smaller removals, one string table); `AGENTS.md` "Resources" examples;
  `docs/PLAN.md` stage 3 row.
- Total: 39 .rs files changed, +1033 −275 lines; data and config files:
  `core/data/crypto-wiki.toml`, `scripts/cli-acceptance.sh`,
  `scripts/cli-diagnostics.py`; dependencies unchanged; `#[test]` functions
  842 → 847.

## Beta Commit 171

- **The CLI's live portfolio and spot price come from core's valuation**
  (`cli/src/cmd/market.rs`; `core/src/service/network_prices.rs`
  `native_spot_price`)
  - `spectra portfolio` has core refresh each wallet's balances
    (`refresh_wallet_balances`), then prices and fiat rates when due, and prints
    `portfolio_snapshot().valuation`: every held asset with its stored amount
    and value, wallet totals and the total in the display currency,
    `unpricedCount`, wallets whose balances could not be read (`unavailable`)
    and refresh `failures`; the refreshed balances and quotes are stored
    (before: the CLI fetched native balances and a USD price itself, multiplied
    floats, skipped tokens, stored nothing, and fell back to USD when the rate
    failed).
  - `spectra price <chain>` prints core's `native_spot_price` converted with the
    stored rate.
  - A missing rate or a testnet coin gives `null` (shown as `—`) instead of
    another currency's figure or zero.
  - The fixed "via CoinGecko" line is removed.
  - The CLI-only `service::fetch_prices` and `service::fetch_fiat_rates` are
    removed.
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-24 "The CLI's live portfolio and
  spot price are core's valuation".
- Total: 4 .rs files changed, +167 −193 lines; data and config files:
  `scripts/cli-portfolio.py`; dependencies unchanged; `#[test]` functions
  unchanged at 842.

## Beta Commit 170

- **Amounts are exact decimal strings** (new `core/src/decimal.rs`;
  `formatting.rs`, `send/amount_input.rs`, `store/wallet_domain.rs`,
  `service/valuation.rs`)
  - Holdings, transaction amounts, receipt and confirmed fees, preview details
    and affordability use canonical decimal strings instead of `f64`, and
    affordability compares exactly.
  - Core sums dashboard totals (`CoreDashboardAssetGroup.total_amount`).
  - `format_asset_amount` truncates to the displayed places and flags
    `below_threshold` (before: rounding to nearest, so `0.999999999` BTC read
    `1`).
  - `is_valid_amount_input` replaces `parse_amount_input`.
  - `send_amount_shortcut` and CLI `send shortcut` take the exact balance;
    fee-adjusted preview maxima stay on floats with one ULP reserved.
  - Price-alert targets are text parsed by core.
  - `price_usd` on holdings and `fee_priority_raw` on records are removed.
- **Core values everything in the display currency** (`service/valuation.rs`,
  `service/app_refresh.rs`, `service/owned_send.rs`)
  - `PortfolioValuation` carries per-holding values, unit prices and alert
    targets; dashboard groups carry `total_value` and `price`.
  - `OwnedSendPreview` carries `network_fee`, `network_fee_value`,
    `amount_value` and the quoted `amount`.
  - Notifications carry their `currency`.
  - `refresh_app` returns the price-alert and movement notifications it
    evaluated.
  - `unpriced_chain_names` is removed.
  - A testnet holding has no value.
- **Chains cross the boundary by id** (`store/state.rs`,
  `store/wallet_domain.rs`, `registry.rs`, every record and command)
  - Wallets, holdings, transactions, contacts, token commands, fee-priority
    settings, diagnostics, wiki places, funds-finder candidates and endpoint
    probes carry `chain_id` instead of a display name; names are derived where
    text is drawn (`Chain::display_name_for_id`).
  - CLI `--json` prints `chainId`.
  - `WalletState` stores the id once; a wallet's family is
    `WalletState::family()`.
  - `CoreTokenHostingChain`, `evm_seed_derivation_chain` and the unused identity
    fields `token_hosting_chain`, `send_execution_shape`, `rpc_health_method`,
    `pending_status_poll`, `seed_derivation_chain` are removed.
  - `Chain::hosts_tokens()` replaces the hosting list.
- **A wallet's signing kind is stored, and phrase reveal is one typed call**
  (`store/state.rs` `WalletSigning`, `service/wallet_import.rs`
  `reveal_seed_phrase`)
  - `WalletState.signing` is `watchOnly`, `seedPhrase { passwordProtected }` or
    `privateKey { passwordProtected }`, written at import (before:
    `is_watch_only` plus Keychain queries on each render).
  - `reveal_seed_phrase(wallet_id, password)` answers `phrase`, `notStored`,
    `passwordRequired`, `incorrectPassword` or `passwordNotRequired`; CLI
    `wallet export` uses it.
  - `wallet_secret_state`, `wallet_seed_phrase` and both secret-material
    descriptors are removed.
- **Failures and degradations are typed** (`store/persistence_models.rs`
  `TransactionFailure`, `service/diagnostic_state.rs` `ChainDegradation`)
  - `stuckAfterRetries`, `submissionOutcomeUnknown`,
    `rebroadcastOutcomeUnknown`, `reported { message }` and
    `historyRefreshFailed`, `historyPartiallyLoaded`, `failed { message }`
    replace stored English sentences.
  - `core/src/diagnostics/degraded.rs` is deleted.
- **One self-send check** (`service/owned_send.rs`
  `is_own_send_destination`, `send/flow.rs`; `cli/src/cmd/tx.rs`)
  - Ownership compares in the chain's normal form (all-caps bech32
    lowercased).
  - `self_send_confirmation` and its records are removed.
  - CLI `send self-check` prints `{"ownAddress": bool}`.
- **Address-book contacts get core-minted ids, and duplicates are exact**
  (`store/state.rs`; `cli/src/cmd/address.rs`)
  - Core mints a contact's id, named by `AddressBookEntryAdded` (before: the
    caller supplied it).
  - Duplicates compare normalized addresses exactly, so base58 addresses
    differing only in case are distinct (before: case-insensitive).
- **`transaction_endpoints(id)` reports each side of a transfer and whether it
  is the user's** (`service/history_query.rs`; CLI `txs --endpoints <id>`).
- **Unread projections are removed**: `resolved_addresses_by_wallet_id`, the
  `merge_built_in_token_preferences` export, `endpoint_tag`,
  `WalletHoldingRef` and `GroupedPortfolioHolding`.
- Tests: 12 new, 15 removed
  - Decimals 5: non-unique canonical spellings, digits lost to floats, inexact
    sums, truncation rounding up, floats not arriving in shortest form.
  - Formatting 1: a large balance losing digits or dust shown without its
    flag.
  - Shortcuts 1: a shortcut computed through a float.
  - Transaction endpoints 3: a send's direction reversed, a receive not ending
    at the wallet regardless of case, a self-send naming its address twice.
  - Valuation 1: a testnet holding valued.
  - Address book 1: case-distinct base58 addresses merged.
  - Removed: eight degraded-sentence tests with `degraded.rs`, four
    token-hosting and descriptor tests, two endpoint-tag tests, one testnet
    pricing boundary test.
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-23 entries (chain ids, wallet
  signing, exact amounts, display-currency valuation, send preview store and
  self-send, typed failures, address-book ids, colours and transaction
  endpoints).
- Total: 131 .rs files changed (1 new, 1 deleted), +3314 −3405 lines; data and
  config files: `scripts/cli-acceptance.sh`, `scripts/cli-diagnostics.py`,
  `scripts/cli-history.py`, `scripts/cli-portfolio.py`, `scripts/cli-send.py`;
  dependencies unchanged; `#[test]` functions 845 → 842.

## Beta Commit 169

- **Failed built-in providers removed and health checks probe real reads**
  (`core/data/endpoints.toml`; new `core/src/service/endpoint_health.rs`;
  `fetch/evm.rs`, `service/network.rs`)
  - Removed: Blockchair BSV (three rows), SoChain LTC, BlockCypher DOGE testnet,
    TronGrid `.pro` and `.network`, HappyStaking Koios, Exan Monero stagenet,
    Cloudflare Ethereum, OnFinality Hyperliquid, Trezor ZEC, BTG and DASH;
    Zcash, Bitcoin Gold, Dash, Dogecoin testnet and Monero stagenet now have no
    built-in API, and custom nodes can still be added.
  - Tron PublicNode uses `/jsonrpc`.
  - EVM history sources live in the shared directory with only implemented
    capabilities.
  - Routescan uses its `/etherscan` path.
  - Testnets no longer inherit mainnet history.
  - Berachain, Cronos and X Layer have no built-in history source.
  - Probes run on the endpoint's own origin and check JSON or API responses, EVM
    chain identity plus block and balance reads, history API errors and the
    Esplora and Monero protocol paths; every built-in API has a probe, and
    browser links are excluded.
  - CLI `endpoints` checks every network, testnets included, and reports probe
    errors.
- **Stored history paging uses a cursor** (`wallet_db/history_query.rs`,
  `service/history_query.rs`): `txs --page --cursor <nextCursor>` over indexed
  `(created_at, id)`, bound to wallet, filter, search and direction,
  `nextCursor: null` on the last page; the integer offset is removed.
- **History summaries count what pages show** (`wallet_db/history.rs`,
  `history_query.rs`): the pending snapshot, replacement list, count and
  earliest dates use the same current-wallet and identity-winner rule as pages,
  so a superseded pending row or an orphaned wallet's row no longer appears or
  offers replacement.
- **Automatic status polling covers pending transactions only**
  (`service/pending_status.rs`, `transaction_recheck.rs`, `service/state.rs`)
  - Dogecoin's polling of confirmed sends up to 12 confirmations, its
    in-memory stop flag and finality event are removed; a Dogecoin send stops
    at its first confirmation, also after restart.
  - An explicit recheck resumes polling on a reorg.
  - An unresolved read schedules another check.
- **Send preparation and history writes are scoped and atomic**
  (`wallet_db/sends.rs`, `wallet_db/history.rs`, `service/monero_wallet.rs`)
  - EVM nonce preparation and Monero input selection read only the chain and
    sender's pending history and signed artifacts by index; an unrelated
    undecodable artifact no longer blocks them, while a selected invalid one
    still refuses.
  - History batches reuse one prepared statement inside an immediate RAII
    transaction, so a failed insert, delete or commit rolls back.
- **Smaller core changes** (`service/state.rs`, `formatting.rs`, `tokens.rs`,
  `store/artwork.rs`)
  - Pin options list pinned assets first.
  - The portfolio snapshot carries a precision catalog per deployment.
  - The caller-fed `token_display_decimals` is no longer exported.
  - The whole-holding artwork export (`holding_artwork_name`) is removed in
    favour of `token artwork --deployment-id`.
- Measured: 2026-09-23 live endpoint audit; the 14 failing built-in provider
  records above were removed rather than kept as fallbacks (`docs/PLAN.md`
  "Endpoint availability and redundancy").
- Tests: 13 new, 3 removed
  - Endpoint health 7: a probe sent to another host or accepting HTML, EVM reads
    not checked after chain identity or a wrong network accepted, an RPC answer
    without a result or a large hex balance rejected, Esplora or Monero probed
    off their protocol path, a history API error read as success, Rosetta
    metadata not checked.
  - EVM history 1: an explicit history path extended.
  - Status polling 1: Dogecoin polling past its first confirmation or after
    restart, or losing manual recheck.
  - Storage 4: unrelated artifacts blocking a signed query, a partial history
    batch left behind or a failed commit blocking retry, the pending-sender
    query not using its index.
  - Removed: three network and state tests replaced by the endpoint health tests
    and the polling change.
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-23 entries (failed providers and
  read probes, durable broadcast completions, pinned-first options, confirmed
  Dogecoin polling, history row confirmation counts, history identity winners,
  cursor pages, scoped send preparation and atomic batches, native flow
  isolation and precision projections); `docs/ARCHITECTURE.md` "Native flow
  lifetimes and display precision"; `docs/PLAN.md` stage 3 row, endpoint
  availability and EVM history availability.
- Total: 30 .rs files changed (2 new), +1205 −612 lines; data and config
  files: `core/data/endpoints.toml`, `scripts/cli-acceptance.sh`,
  `scripts/cli-endpoints.py`, `scripts/cli-history.py`,
  `scripts/cli-portfolio.py`; dependencies unchanged; `#[test]` functions 835
  → 845.

## Beta Commit 168

- **Every request is routed by declared capability**
  (`core/src/service/endpoint_directory.rs`, `core/src/endpoint_api.rs`,
  `service/network*.rs`, `send_preview.rs`, `send_stage_protocols.rs`,
  `staking.rs`, `core/data/endpoints.toml`)
  - A custom endpoint stores its own non-empty, canonical capability set (CLI
    `endpoints --add` requires `--capabilities`).
  - Unknown capabilities and operations the adapter does not implement are
    refused.
  - Custom records no longer copy a built-in provider's metadata.
  - Balance, fees, token reads, history, UTXOs, verification and staking pick
    only providers declaring that operation, and fallbacks keep the requirement;
    an operation needing several capabilities requires all of them.
  - EVM previews and builds read balance, fee, nonce and token metadata from
    separately selected nodes.
  - Native and token history pick their sources independently.
  - Broadcast candidates must declare `broadcast`, checked again before
    verification and submission; an explicit submission cannot use an undeclared
    URL or bypass a saved endpoint's restriction.
  - `staking` is its own capability.
  - Built-in rows used for context verification, staking, Cardano UTXOs and
    TON token metadata now declare those purposes.
  - WhatsOnChain declares broadcast on its base URL and its operation-path rows
    are removed, so `/tx/raw` is not appended twice.
  - A failed automatic Bitcoin fee preview no longer reports a fabricated 5
    sat/vB quote.
- **Nine icons redrawn without transforms or negative space** (`icons/crypto/`):
  ethfi, ink, leo, ondo, polkadot, polygon, sei, sonic and wrappedbitcoin.
- Tests: 5 new
  - Endpoint routing 5: same-API endpoints sharing capabilities or requests,
    custom capabilities unvalidated or non-canonical, EVM preview reads sent to
    one node regardless of capability, fallback or submission widening
    capabilities, WhatsOnChain broadcast using an operation path as its base.
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-23 entries (large-movement threshold
  entry, alert copy removals, ETHFI, INK, LEO, ONDO, Polkadot and Polygon, SEI,
  Sonic and Wrapped Bitcoin icon changes, custom endpoint capabilities).
- Total: 45 .rs files changed, +1150 −246 lines; data and config files:
  `icons/crypto/` (9), `scripts/` (9), `core/data/` (1); dependencies
  unchanged; `#[test]` functions 830 → 835.

## Beta Commit 167

- **Custom endpoints are one typed directory used by the real clients** (new
  `core/src/service/endpoint_directory.rs`; `store/state.rs`,
  `service/network*.rs`, `fetch/tron.rs`)
  - A custom endpoint is a network, an `api` from `endpoints.toml` and a URL;
    invalid, duplicate and incompatible entries leave state unchanged.
  - Custom URLs feed the matching API clients and health probes, newest first
    and before built-in nodes, isolated per network.
  - Blockscout history accepts custom sources.
  - TronGrid token discovery uses its own typed account URLs.
  - The separate `rpc_endpoint_by_chain`, Bitcoin Esplora and Monero backend
    settings are removed (before: the Esplora and Monero values were stored but
    not used for node selection); CLI `settings` loses `rpc-endpoint.*`,
    `bitcoin-esplora-endpoints` and `monero-backend-url`.
  - New CLI `endpoints --chain <network> --api <api> --add <url>`.
  - CLI `endpoints --catalog` takes `--source built-in|custom`.
- **The `strict-rpc-only` setting is removed**: it was stored and exposed but
  read by no operation; `settings get/set strict-rpc-only` is now refused.
- **Verification guidance scales with the change** (`AGENTS.md`,
  `docs/PLAN.md`): small localized edits need targeted checks; the full suite is
  for broad, domain, persistence, funds, key or FFI changes; report which checks
  ran.
- Tests: 3 new, 3 removed
  - Endpoint directory 3: custom nodes not using the API adapters or lost on
    reopen, custom indexer APIs mixed into primary RPC lists, invalid or
    duplicate entries changing state.
  - Removed: the per-setting endpoint URL and Esplora list parsing tests,
    replaced by the directory.
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-23 entries (typed custom endpoint
  directory, Report a Problem intro, strict RPC removal, recipient icon, pricing
  and endpoints intros); `AGENTS.md` "Verification"; `docs/PLAN.md`
  verification.
- Total: 19 .rs files changed (1 new), +687 −420 lines; data and config files:
  `scripts/` (9); dependencies unchanged; `#[test]` functions unchanged at
  830.

## Beta Commit 166

- **Token discovery is chosen per token, not per deployment**
  (`core/src/store/state.rs`, `store/mod.rs`, `core/src/tokens.rs`)
  - Toggling any deployment applies to every known deployment with the same
    token id, and a catalog refresh carries the choice to new networks (before:
    per deployment, and a new deployment could reset it).
  - A custom token keeps its own identity; a matching name, symbol or provider
    id never links it to another token.
- **Custom tokens are fully editable, with independent price ids**
  (`store/state.rs`, `fetch/price.rs`, `service/network_prices.rs`; CLI `token
  edit`, `token add --coinpaprika-id`)
  - Name, symbol, precision and optional CoinGecko and CoinPaprika ids can be
    added or edited; network and contract cannot; built-ins stay read-only.
  - Provider ids are validated before storage and URLs are refused.
  - A metadata change clears that deployment's cached price.
  - CoinPaprika quotes come from the token's own CoinPaprika id, never inferred
    from its CoinGecko id.
  - The test-only provider-to-provider lookup is deleted.
  - `spectra --json token list` shows identities, choices and both provider ids.
- **Token history labels come from deployment identity**
  (`core/src/fetch/history.rs`)
  - Names and tickers resolve by network and case-sensitive mint or contract,
    so Solana SPL history shows USDC/USDT instead of the mint address; unknown
    contracts keep their address.
  - Stored records are relabelled on refresh.
- Tests: 6 new, 4 removed
  - Token discovery 1: a new network deployment ignoring the token's saved
    choice.
  - Provider ids 4: one listing claimed by two catalog tokens, ids not
    lowercased and trimmed, an unlisted token given a CoinPaprika id, a catalog
    token without an explicit CoinPaprika listing.
  - History labels 1: an SPL label resolved across networks or
    case-insensitively.
  - Removed: four price-module tests of the deleted CoinGecko-keyed lookup,
    replaced by the catalog tests.
- Docs: `docs/BEHAVIOUR-CHANGES.md` entries dated 2026-09-22 and 2026-09-23
  (token history labels, history capability label, diagnostics actions,
  token-wide discovery and token management, provider lookup removal, Known
  Tokens filters and switches).
- Total: 13 .rs files changed, +394 −231 lines; data and config files:
  `scripts/cli-acceptance.sh`, `scripts/cli-history.py`,
  `scripts/cli-token-preferences.py`; dependencies unchanged; `#[test]`
  functions 828 → 830.

## Beta Commit 165

- **Sends are built, signed and broadcast as separate, durable stages** (new
  `core/src/service/send_stages.rs`, `send_stage_protocols.rs`,
  `send_stage_utxo.rs`, `core/src/send/stages.rs`,
  `core/src/wallet_db/sends.rs`; `service/send_execution.rs`)
  - Build persists a secret-free typed artifact; signing signs its reviewed
    fingerprint as a separate action and stores the signed content; broadcast
    submits the saved payload only to explicitly selected configured endpoints,
    with no unselected fallback (before: an in-memory review expired after 120
    seconds and the next action signed and submitted).
  - Signing rechecks nonce or sequence, selected inputs or objects and protocol
    expiry, and expiry is checked again before submission.
  - SQLite compare-and-swap and input reservations stop conflicting concurrent
    signs.
  - Retries reuse the same signed bytes and keep a result per endpoint.
  - A lost or malformed response stays uncertain.
  - Node acceptance is recorded apart from on-chain status, and a confirmed
    record cannot be reset by rebroadcasting.
  - `build_owned_send` builds from a tracked holding and stores typed build-time
    advisories (recipient, first-use, self-send) in the review digest; reopening
    returns the same review.
  - An EVM same-nonce replacement must raise both fee caps by at least 10% (and
    one wei); core moves the nonce reservation and keeps both signed artifacts.
  - `execute_send` composes the same stages; the old parameter, submission and
    result dispatchers (`send_params.rs`, `send_submission.rs`,
    `send_result.rs`, `send/payload.rs`) and combined per-chain wrappers are
    removed.
  - Wallet deletion removes its artifacts and reservations.
  - Custom broadcast endpoints need a verifiable EVM or Aptos network identity;
    other adapters need a matching catalog network, API and broadcast
    capability.
  - CLI `send build`, `send build-owned`, `send list`, `send inspect`, `send
    sign --review-digest`, `send broadcast-signed --endpoint … --yes`, `send
    configured-endpoints <chain>` (offline, in service order).
- **Monero signs on the device** (new `core/src/send/monero_local.rs`,
  `core/src/service/monero_wallet.rs`, `core/src/wallet_db/monero.rs`;
  dependencies `monero-wallet`, `monero-daemon-rpc`)
  - Scanning, decoy selection and CLSAG / Bulletproof+ signing are local;
    daemons receive public queries and the signed bytes, never spend or view
    keys (before: a remote wallet RPC).
  - A network-scoped AES-GCM encrypted scan cache and frozen signing plan are
    persisted; the view key lives in `SecretStore`.
  - Signed artifacts reserve key images.
  - Restart resumes scanning and a reorg restarts from the restore height.
  - Local unlocked balance and scanned history replace wallet-RPC reads.
  - HF16 is required and unknown hard forks refuse; pre-RingCT outputs and
    subaddresses are not supported.
  - The catalog's three Edge `monero-light-wallet` rows are replaced by
    `monero-daemon-rpc` rows: Cake Wallet on mainnet, Exan on stagenet
    (`core/data/endpoints.toml`).
  - CLI `send sync-monero --from <wallet> [--restore-height H] [--once]` and
    `send monero-status`.
- **ICP derives ledger account ids and signs locally** (`derivation/icp.rs`, new
  `send/icp_stages.rs`)
  - Account identifiers are CRC32/SHA-224 of the DER Ed25519 principal.
  - Protobuf ledger arguments, signed call and read-state envelopes and the
    ledger hash are built locally; Rosetta supplies metadata and submits.
  - ICP wallets from the old derivation must be re-imported, and their
    addresses are refused.
- **Zcash transparent sends use ZIP-244 signing** (`send/zcash.rs`, new
  `send/zcash_stages.rs`, `derivation/zcash.rs`, `registry.rs`)
  - Genesis, consensus branch (upgrades through NU6.2), unspent inputs,
    ZIP-317 fees and expiry are checked; unknown future branches refuse;
    shielded sends are not supported (before: a fixed NU5 branch and an
    incorrect signature preimage).
  - Testnet P2SH validation is corrected.
  - The preview fee floor is 10,000 zatoshis (before: 1,000).
  - Blockbook broadcast parses its JSON `result` txid.
- **State responses carry a committed revision** (`service/state.rs`,
  `CoreAppState.revision`): assigned only when committed state is published, not
  advanced by failed or no-op commands, not persisted.
- **Transaction actions come from core** (new
  `service/transaction_actions.rs`)
  - History pages, snapshots and record reads include recheck and rebroadcast
    availability with reasons, checked again before execution.
  - Confirmed UTXO records can be rechecked.
  - `txs --record` and `txs --page` print `actions`.
- **Transaction timestamps are Unix seconds everywhere**
  (`store/persistence_models.rs` `createdAtUnix`; before: the payload used the
  2001 epoch), with fractional seconds and no migration.
- **Watch-only inputs are keyed by chain id** (`derivation/import.rs`
  `WalletImportWatchOnlyEntries.byChainId`): core validates addresses and builds
  storage slots, so EVM chains are no longer merged before core sees them.
- **Maestro's Bitcoin Esplora and TronScan's `accountv2` endpoints removed**
  (`core/data/endpoints.toml`).
- Measured: 2026-09-22, Monero v0.18.5.1 `monerod` (regtest, offline) accepted
  a transaction Spectra built and signed locally, without wallet RPC; the
  public fixture and its notes are `core/tests/fixtures/monero-local.json` and
  `core/tests/fixtures/monero-local.md`.
- Tests: 10 new, 20 removed
  - Staged sends 2: an invalid exact amount or fee reaching storage or keys; a
    saved signature submitted after expiry.
  - ICP 3: a wrong ledger checksum or principal domain; envelopes binding
    anything beyond the reviewed transfer and status path; the ledger hash
    differing from the official vector.
  - Zcash 1: signature or txid differing from the official Python reference.
  - Monero 2: keys leaving the device or the signed content differing from the
    daemon-accepted fixture; a scan cache crossing networks or accepting a stale
    writer.
  - State 1: failed writes advancing the revision.
  - Watch-only 1: two EVM chains merged into one storage slot.
  - Removed: eighteen tests of the deleted send-parameter dispatcher, the Monero
    wallet-RPC binding test and the typed EVM result test.
- Docs: `docs/BEHAVIOUR-CHANGES.md` entries dated 2026-09-21 and 2026-09-22
  (durable review and session isolation, staged sends, committed state versions,
  watch-only chain ids, transaction actions, Unix timestamps, currency metadata,
  Maestro and TronScan removal, local Monero, ICP and Zcash);
  `docs/ARCHITECTURE.md`; `docs/FFI-BOUNDARY.md` timestamps and versions;
  `docs/PLAN.md` staged sends, configured endpoints and local Monero/ICP/Zcash
  marked done; new `core/tests/fixtures/monero-local.md`.
- Total: 100 .rs files changed (11 new, 3 deleted), +5255 −3729 lines; data
  and config files: `scripts/` (7), `Cargo.lock`, `core/` (1), `core/data/`
  (1), `core/tests/` (1); dependencies: added monero-daemon-rpc 0.2,
  monero-wallet 0.2, serde_cbor 0.11 (`core/Cargo.toml`); `#[test]` functions
  838 → 828.

## Beta Commit 164

- **Documentation consolidated; no code changed**
  - `PLAN.md` and `FUTURE_PLANS.md` move into `docs/`; links in `AGENTS.md`,
    `README.md`, `docs/ARCHITECTURE.md` and `docs/FFI-BOUNDARY.md` follow.
  - `docs/DUPLICATION-AUDIT.md`, `ENDPOINT-APIS.md`, `ENDPOINT-CAPABILITIES.md`,
    `NAMING.md` and `TEST-AUDIT.md` are deleted, with the links to them in
    `docs/ARCHITECTURE.md` and `core/data/endpoints.toml`.
  - `docs/BEHAVIOUR-CHANGES.md` drops its recorded entries again.
  - `AGENTS.md` "Icons" is shortened to the export rule, the house-style bullets
    and the `appicon/` exception.
- Docs: `AGENTS.md` (PLAN link, "Icons"); `README.md`, `docs/ARCHITECTURE.md`
  and `docs/FFI-BOUNDARY.md` (links); `docs/PLAN.md` and
  `docs/FUTURE_PLANS.md` (moved); `docs/BEHAVIOUR-CHANGES.md` (entries
  dropped); `docs/DUPLICATION-AUDIT.md`, `ENDPOINT-APIS.md`,
  `ENDPOINT-CAPABILITIES.md`, `NAMING.md` and `TEST-AUDIT.md` (deleted).
- Total: no .rs files changed; data and config files:
  `core/data/endpoints.toml`; dependencies unchanged; `#[test]` functions
  unchanged at 838.

## Beta Commit 163

- **Bitcoin-family and Substrate chains share their primitives**
  (`core/src/derivation/bitcoin.rs`, `send/substrate.rs`; the per-chain files
  for Bitcoin Cash, Bitcoin Gold, Bitcoin SV, Dash, Decred, Dogecoin, Kaspa,
  Litecoin, Zcash, Polkadot and Bittensor call them)
  - P2PKH derivation, address and script construction and BIP-32 path parsing
    have one implementation; Polkadot and Bittensor share their wire helpers.
  - A non-P2PKH script request on a P2PKH-only chain reports "This chain only
    supports P2PKH addresses." on every chain (before: some helpers named the
    chain); addresses, keys and signed bytes are unchanged.
  - The unused `SubstrateSignError` is removed.
- Tests: 0 new, 3 removed (per-chain P2PKH script-shape tests now covered by the
  shared code).
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-21 "Share cross-chain primitives and
  remove an unused error type"; new `docs/DUPLICATION-AUDIT.md`.
- Total: 21 .rs files changed, +151 −578 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions 841 → 838.

## Beta Commit 162

- **Stored state that is not the current shape is refused instead of tolerated**
  (`wallet_db/wallets.rs`, `wallet_db/state.rs`, `wallet_db/connection.rs`,
  `store/state.rs`, `store/password_verifier.rs`, `store/seed_envelope.rs`)
  - An undecodable wallet row, unreadable or unknown metadata, a missing
    settings field, an unknown stored fee priority and an unknown state schema
    version now fail the load, and the stored bytes are left as they were
    (before: skipped or defaulted; the skip was added in Beta Commit 148).
  - A fresh database writes the complete default settings before incremental
    updates.
  - Seed and password-verifier envelopes refuse unknown fields; a verifier must
    have the exact current version and work factor.
- **API keys and authenticated providers removed** (`core/data/endpoints.toml`,
  `fetch/`, `store/state.rs`)
  - Endpoint records and settings have no API-key fields, so the Monero
    backend key and the Etherscan key are gone.
  - The Taostats, Subscan and Ethplorer adapters and catalog rows are deleted.
  - CLI `settings set etherscan-api-key` and `monero-backend-api-key` are
    refused.
  - BNB Chain, Sonic, opBNB, Sei, Linea and Hyperliquid have no history source
    (`spectra --json chains` reports `historySource: "none"`, without
    `needsApiKey`); their history fails before any request.
  - Polkadot (including Westend) and Bittensor have no balance or history source
    and fail instead of showing zero; their RPC rows keep fee and broadcast.
  - TON requests are unauthenticated.
  - The supplemental-explorer slots are removed; pending polling uses every
    configured slot for the network.
- **Cardano broadcasts raw CBOR to Koios** (`send/cardano.rs`,
  `staking/chains/cardano.rs`)
  - `POST /submittx`, HTTP 202 and a 64-hex transaction id are required
    (before: a Blockfrost-shaped request).
  - The empty Cardano staking queries are removed.
- **Balance refresh shares in-flight reads** (`service/balance_refresh.rs`,
  `fetch/refresh_engine.rs`)
  - Requested and automatic refreshes share reads by wallet, network and
    address, at most eight at once (before: requested refresh ran wallets one
    at a time).
  - A commit rechecks the address and network under the state writer,
    persists only that wallet and publishes after it is stored (before: every
    commit rebuilt the refresh list and cloned state).
- **History pages use indexes** (`wallet_db/history_query.rs`,
  `wallet_db/connection.rs`)
  - SQLite refuses invalid identity, kind or status on write.
  - Generated identity and precedence columns let each row seek its winning
    record without a temporary sort.
  - Snapshot merging uses an id set.
- **Module layout flattened; behaviour unchanged**: `derivation/chains/`,
  `fetch/chains/`, `send/chains/` and `staking/chains/` files become siblings in
  their domain (`ton/cell.rs` → `ton_cell.rs`, `tron/metadata_cache.rs` →
  `tron_metadata_cache.rs`, `refresh/{engine,policy}.rs` → `refresh_engine.rs`,
  `refresh_policy.rs`); `store/wallet_db/` → `core/src/wallet_db/`; service test
  directories become `*_tests.rs` files.
- Tests: 8 new, 19 removed
  - Storage 3: unknown metadata or schema versions accepted, a stored setting
    missing a field loaded with a default, an unknown verifier version accepted.
  - Missing sources 2: Polkadot or Bittensor reads returning an empty wallet.
  - Cardano 1: Koios sent anything but raw CBOR, credentials sent, a response
    without a transaction id accepted.
  - Balance refresh 1: unbounded or duplicated overlapping reads, a stale
    network committed.
  - History 1: a page query using a temporary sort.
  - Removed: nine provider and supplemental-slot tests with the code they
    covered, and ten registry, store and forward-compatibility tests judged
    redundant or obsolete (`docs/TEST-AUDIT.md`).
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-21 entries (prelaunch storage
  compatibility, store test cleanup, keyless providers and Cardano broadcast,
  module layout, Etherscan history removal, balance refresh, indexed history);
  new `docs/TEST-AUDIT.md`; `docs/ARCHITECTURE.md` module layout;
  `docs/ENDPOINT-CAPABILITIES.md`; `PLAN.md` known open items (EVM history
  availability, keyless provider policy).
- Total: 201 .rs files changed (2 new, 10 deleted), +1641 −2600 lines; data
  and config files: `core/Cargo.toml`, `core/data/endpoints.toml`,
  `scripts/cli-acceptance.sh`, `scripts/cli-history.py`; dependencies
  unchanged; `#[test]` functions 852 → 841.

## Beta Commit 161

- **Endpoints declare their wire API, and clients are chosen by it**
  (`core/data/endpoints.toml`; new `core/src/endpoint_api.rs`; `app_core.rs`;
  new `docs/ENDPOINT-APIS.md`)
  - Every API row declares a validated `api` enum (for example `esplora`,
    `evm-json-rpc`, `monero-light-wallet`); browser links omit it and declare no
    capabilities.
  - `explorer_label` enables a transaction link.
  - Each service slot selects bases whose API its client speaks and which carry
    `balance` (before: primary lists mixed incompatible APIs, and Bitcoin
    Esplora and TON v3 were picked by row id).
  - Litecoin and Bitcoin Cash (Blockbook) and Monero wallet-RPC have no
    compatible catalog URLs, so those primary lists are empty instead of sending
    incompatible requests.
  - The endpoint `kind`, its category masks and `provider_id` are removed.
  - `native-history` is renamed back to `history` (native coin only;
    `token-history` stays separate).
  - A custom URL matching a catalog row with an incompatible API or an
    operation-only path is refused before it is stored.
  - CLI `endpoints --catalog` prints `api` and the `configured` lists, without
    `kind` or `providerID`.
- **Shared clients for JSON-RPC, REST and Blockbook** (new
  `core/src/fetch/json_rpc.rs`; `fetch/http.rs`, `fetch/chains/*`)
  - Eight JSON-RPC clients share request, error and fallback handling, keeping
    XRPL's envelope and Monero's and NEAR's string ids; a null `error` with a
    result is accepted, a malformed result still fails.
  - Nine REST readers share base-path GET and fallback.
  - Balance, history, status and rebroadcast pick their adapter from the
    endpoint's API.
  - The five Blockbook marker types and files become one client carrying its
    network.
  - Bitcoin Gold, Dash and Zcash history is now normalized (before: fetched
    and dropped).
  - Native balance summaries format smallest units once with registry decimals,
    so sub-micro amounts are no longer truncated.
- **Core owns the Tor runtime** (`core/src/tor.rs`, new `service/transport.rs`;
  CLI `tor [--reconnect]`)
  - Registering the platform cache directory starts it; committed setting
    changes and resets reconcile it, and reconnect reads stored settings.
  - Stopped or replaced bootstraps cannot publish a late completion.
  - The embedded listener binds a free port before reporting ready.
  - Client selection shares the transport-switch lock with the kill-switch
    check.
  - CLI network commands apply stored routing and wait up to 120 s for
    bootstrap; settings editing stays offline.
- **Owned send previews carry identity, details and amount shortcuts**
  (`service/owned_send.rs`, `service/types.rs`, `service/send_destination.rs`)
  - A preview includes wallet, holding and network identity, display details and
    core-derived 25/50/75/MAX amounts (`send preview` prints `shortcuts` and
    `details`).
  - A native-fee-only preview for a token no longer reports the gas balance as
    the token's spendable or maximum.
  - Destination activity is classified in core from raw smallest units on the
    wallet's network (`send probe` prints `activity`), separating an unused
    address from an emptied one.
- **The import-draft validator is removed** (`core/src/derivation/import.rs`):
  `WalletImportDraftInput` and `core_validate_wallet_import_draft` are deleted;
  import and rename still validate before persisting.
- Tests: 9 new, 7 removed
  - Endpoint APIs 1: a known incompatible URL accepted into configuration.
  - JSON-RPC 2: protocol errors not retried or dialects changed; malformed or
    XRPL error responses read as success.
  - Blockbook 1: one adapter mishandling a network or BCH address rules.
  - Balances 1: a sub-micro native amount shown as zero.
  - Send preview 2: a native-fee quote claiming a token balance or maximum; an
    unused address confused with an emptied one.
  - Transport 2: a late bootstrap completion published; committed settings or
    reset not switching the proxy without a platform callback (new integration
    test `core/tests/transport_runtime.rs`).
  - Removed: two endpoint-kind tests and five import-draft tests, with the code
    they covered.
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-21 entries (endpoint kind,
  API-selected adapters, endpoint APIs and known gaps, authentication actions,
  core-owned transport, owned preview details, import draft removal,
  `provider_id`, `history` rename); new `docs/ENDPOINT-APIS.md`;
  `docs/ARCHITECTURE.md`, `docs/ENDPOINT-CAPABILITIES.md`, `docs/NAMING.md`;
  `PLAN.md` stage 3 row.
- Total: 61 .rs files changed (4 new, 5 deleted), +1480 −1501 lines; data and
  config files: `core/data/endpoints.toml`, `scripts/cli-acceptance.sh`,
  `scripts/cli-history.py`, `scripts/cli-send.py`, `scripts/cli-transport.py`;
  dependencies unchanged; `#[test]` functions 850 → 852.

## Beta Commit 160

- **`Chain::evm_chain_id` refuses non-EVM chains** (`core/src/registry.rs`)
  - The error is raised before an EVM client is built (before: Ethereum's `1`
    for every non-EVM chain); `spectra send assemble --chain Bitcoin …` exits
    3.
  - The unused `EvmChain` wrapper is removed.
- **Wallet views refuse an identity that does not resolve**
  (`core/src/store/wallet_domain.rs`)
  - `WalletView.selectedChain` becomes `familyName`.
  - Conversion refuses unknown networks, unknown families and family/network
    mismatches instead of substituting the family mainnet.
  - The primary address comes from the actual network.
- **The endpoint `read` capability is removed** from 110 catalog rows and the
  vocabulary (`core/data/endpoints.toml`, `core/src/app_core.rs`); nine
  capabilities remain.
- **Settings show core's whole endpoint directory** (`app_core.rs`;
  `cli/src/cmd/chain.rs`)
  - `settings_visible` and its lookup argument are removed, so explorer links
    and auxiliary APIs are listed, deduplicated and grouped by network;
    transport selection still uses kinds and capabilities.
  - CLI `endpoints --catalog --chain <chain>` includes `settingsGroups`.
- **Send checks and submission moved to their own modules**: preflight from
  `service/maintenance.rs` to new `service/send_preflight.rs`, `send_signing.rs`
  renamed `send_submission.rs`; helpers renamed after their operations and
  unused network-query wrappers and the wallet eligibility record removed;
  behaviour unchanged.
- Tests: 3 new (six preflight tests moved with their code)
  - Endpoints 1: explorer links shown in settings becoming backend choices.
  - Registry 1: a non-EVM chain with an EIP-155 id.
  - Wallet conversion 1: the primary address taken from a network other than the
    wallet's.
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-20 entries (generic read capability,
  full settings directory, EIP-155 identity, `familyName`, naming and ownership
  cleanup); `docs/ARCHITECTURE.md`; `docs/ENDPOINT-CAPABILITIES.md` drops
  `read`.
- Total: 39 .rs files changed (1 new), +902 −936 lines; data and config files:
  `core/data/endpoints.toml`, `scripts/cli-acceptance.sh`; dependencies
  unchanged; `#[test]` functions 847 → 850.

## Beta Commit 159

- **One name per identity across core, CLI, data and stored JSON** (new
  `docs/NAMING.md`), with stored shapes changed directly and no aliases
  - Chain references are `chain_id` / `chainId` (before: `network_id` /
    `networkId`) in presentation, deployments, endpoints, wallet state, probes
    and CLI output; family selection is `selected_chain_by_family`; refresh
    entries separate `holding_chain_id` from the queried `chain_id`.
  - `TokenEntry` → `TokenDeploymentEntry` with `deployment_id`, `token_id` and
    `chain_id`, for built-in and custom deployments; CoinGecko keys are
    `coingecko_id` / `coingeckoId`.
  - `chains.toml` and `chain-ui.toml` use `[[chains]]` tables (before:
    `[[networks]]`).
  - Free functions drop the `core_` and `app_core_` prefixes, structured
    functions drop `_typed`, and raw preview functions end in `_json`; artwork
    lookup is `--chain-id` (CLI) and `chain_artwork_name` (before:
    `--network-id` and `core_network_artwork_name`).
  - Endpoint kind and capability constants (`ENDPOINT_KIND_*`,
    `ENDPOINT_CAPABILITY_*`) come from core instead of the CLI.
  - The obsolete `rpc` / `explorer` filter aliases are refused.
- **Built-in tokens resolve for chains whose id differs from their name**
  (`core/src/tokens.rs`): the hosting lookup resolves the id through `Chain`
  first, so ids such as `bnb-chain` no longer omit their built-in tokens.
- **Unused secret-store trait, validation forwarding functions and the
  `wallet_core` module alias removed.**
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-20 "Unify domain names and remove
  obsolete naming layers" with the `chain_id` and `[[chains]]` entries; new
  `docs/NAMING.md`; `docs/ARCHITECTURE.md` and `docs/ENDPOINT-CAPABILITIES.md`
  use the new names.
- Total: 93 .rs files changed, +930 −1001 lines; data and config files:
  `core/data/` (5), `scripts/` (3); dependencies unchanged; `#[test]`
  functions unchanged at 847.

## Beta Commit 158

- **Testnets derive on their own paths, recorded per network**
  (`core/data/chains.toml`, `core/src/chains.rs`, `service/wallet_import.rs`,
  `service/send_identity.rs`, `store/wallet_domain.rs`)
  - Every path-based network declares its own templates: the three Bitcoin test
    networks and the Litecoin, Bitcoin Cash, Bitcoin SV, Dogecoin, Zcash, Decred
    and Dash testnets use hardened coin type 1 (Bitcoin Testnet4:
    `m/84'/1'/0'/0/0`); other testnets keep their chain's path; Monero stays
    pathless (before: testnets had no paths and used their mainnet's).
  - Default and custom paths are keyed by network id.
  - Import stores the actual path beside each derived address, and signing,
    discovery, network switching and wallet detail read that recorded path.
  - Partial overrides are completed before derivation; existing wallet paths
    are not rewritten.
  - A wallet with no address on the selected testnet no longer falls back to its
    mainnet address.
  - The CLI wallet summary shows the active network's address.
  - A pathless network is derived with an empty path instead of being skipped at
    import.
- **Chain presentation moved to new `core/data/chain-ui.toml`**
  (`core/data/chains.toml`, `core/src/chains.rs`)
  - Search terms, category, colour, artwork, popular rank and address examples
    move there, joined to `chains.toml` by `network_id`; `chains.toml` keeps
    identity, name, family, environment, token standard, holdings capability
    and derivation paths.
  - Missing, duplicate or unknown references and fields in the wrong file are
    refused.
- **EVM membership is defined in Rust** (`registry.rs` `Chain::is_evm`): the
  `is_evm` TOML flag is removed and refused; display categories cannot change
  it; the exported catalog still reports `isEvm`.
- **`native_deployment` removed from `chains.toml`**: core resolves
  `<network_id>:native` and still rejects a missing native deployment.
- **Token files use flat, id-referenced tables** (`core/data/tokens.toml`,
  `testnet-tokens.toml`, `core/src/tokens.rs`)
  - Independent `[[tokens]]` and `[[deployments]]`, each deployment naming
    `token_id` and `network_id` (before: nested `[[tokens.deployments]]` with
    `network`).
  - Dangling references, tokens without deployments, unknown fields and the
    nested shape are refused.
- **Testnet token ids and native symbols change**
  (`core/data/testnet-tokens.toml`)
  - Ids drop `-native` (`bitcoin-testnet-native` → `bitcoin-testnet`, all 32).
  - Testnet native symbols carry a `t` prefix (`tBTC`, `tETH`, `tSUI`, `tAPT`,
    …); mainnet symbols and deployment ids are unchanged.
- Tests: 13 new
  - Chain presentation 6: a join depending on row order, display categories
    changing EVM membership, duplicate, missing or unknown presentation
    references, fields in the wrong catalog.
  - Testnet symbols 1: a testnet native symbol equal to its mainnet's.
  - Testnet paths 3: derivation metadata naming the mainnet, a testnet path lost
    on reopen, switch or signing, a missing testnet address falling back to
    mainnet.
  - Token tables 3: references depending on table order, broken references or
    the legacy shape accepted, flat deployments skipping network and asset
    integrity checks.
- Docs: `docs/BEHAVIOUR-CHANGES.md` 2026-09-20 entries (testnet token ids,
  testnet derivation paths, testnet symbols, EVM membership in Rust,
  `chain-ui.toml`, derived native deployment, flat token tables);
  `docs/ARCHITECTURE.md` on presentation and EVM membership.
- Total: 23 .rs files changed, +584 −227 lines; data and config files:
  `core/data/chain-ui.toml`, `core/data/chains.toml`,
  `core/data/testnet-tokens.toml`, `core/data/tokens.toml`,
  `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]` functions 834
  → 847.

## Beta Commit 157

- **Endpoints belong to a concrete network id** (`core/data/endpoints.toml`,
  `core/src/app_core.rs`)
  - Each row names `network_id` from `chains.toml` instead of repeating
    `chain_name` with testnet ownership in `group_title`.
  - Unknown ids and the old fields are refused at load, without aliases.
  - One network-id index serves lookups; a testnet request gets only that
    testnet's endpoints.
  - A mainnet settings group includes its family, with titles derived from the
    registry.
  - EVM diagnostics list the selected network's catalog endpoints.
- **Endpoint capabilities split** (`core/data/endpoints.toml`; new
  `docs/ENDPOINT-CAPABILITIES.md`)
  - `history` is replaced by `native-history` and `token-history`.
  - `token-discovery` and `token-balance` are added, each with its own mask
    bit.
  - `balance` means the native coin.
  - NEAR RPC rows lose their incorrect history claim and keep token balance.
  - An unknown capability name fails the catalog load.
- **CLI `endpoints --catalog [--chain <network>]` lists catalog rows offline**
  (`cli/src/cmd/chain.rs`)
  - Rows carry `networkId`, kind and capabilities.
  - Live probe output also reports `networkId`.
- **Long historical doc comments trimmed** across core (`registry.rs`,
  `store/state.rs` and others); behaviour unchanged.
- **`make test-ios` accepts `IOS_TEST_DERIVED_DATA`** for a separate
  derived-data path.
- Tests: 5 new, 2 removed
  - Endpoints 2: invalid network ids or title-based ownership accepted; a token
    capability served by an API that cannot answer it.
  - Seed envelope 2: a same-length tampered envelope authenticating; a malformed
    nonce reaching decryption.
  - Tokens 1: display decimals not taken from the known deployment or the custom
    precision.
  - Removed: two app-boundary tests for amount dust and envelope tampering,
    covered by the new ones.
- Docs: `docs/BEHAVIOUR-CHANGES.md` "2026-09-20 — Endpoint ownership references
  concrete network IDs" and the capability split; new
  `docs/ENDPOINT-CAPABILITIES.md`.
- Total: 37 .rs files changed, +382 −643 lines; data and config files:
  `Makefile`, `core/data/endpoints.toml`, `scripts/cli-acceptance.sh`;
  dependencies unchanged; `#[test]` functions 831 → 834.

## Beta Commit 156

- **Documentation pruned to what is current; no code changed**
  - `docs/BEHAVIOUR-CHANGES.md` keeps only its preamble; every recorded entry
    (about 4,900 lines) is deleted.
  - `PLAN.md` drops its completed workstreams, the 2026-09-12 network and token
    identity rewrite checklist and the closure-audit references; it keeps the
    target, the rules for new work, the stages, the boundary rules, progress
    measurement and known open items, and its verification section points to
    `make verify`.
  - `scripts/README.md` is rewritten in English.
- Docs: `PLAN.md`, `docs/BEHAVIOUR-CHANGES.md`, `scripts/README.md`, as above.
- Total: No code changed.

## Beta Commit 155

- **The refresh-frequency setting is removed; core refreshes balances and
  visible prices every five minutes** (`core/src/fetch/refresh/policy.rs`
  `AUTOMATIC_REFRESH_SECONDS`, `fetch/refresh/engine.rs`, `store/state.rs`):
  `automatic_refresh_frequency_minutes` is gone from settings, the FFI and CLI
  `settings` (`refresh-frequency-minutes` is now refused); pending confirmations
  keep their own cadence.
- **Persisted floating-point values reopen with the same bits**
  (`core/Cargo.toml`: serde_json `float_roundtrip` feature; before: a stored
  `f64` could change by one bit, failing the concurrent persistence test).
- **CLI integration fixtures regrouped into five domain suites**
  (`scripts/cli-wallets.py`, `cli-portfolio.py`, `cli-history.py`,
  `cli-send.py`, `cli-diagnostics.py`, run by `cli-acceptance.sh`), replacing
  thirteen per-change scripts (`cli-balance-refresh.py`,
  `cli-bitcoin-history.py`, `cli-owned-send.py`, `cli-shell-*.py`,
  `cli-stage3*.sh` and others); new `scripts/README.md` (in Chinese) describes
  each script.
- Tests: 1 new
  - Persistence 1: a stored timestamp changing bits on reopen.
- Docs: `docs/BEHAVIOUR-CHANGES.md` entries dated 2026-09-19 (native press
  handling, Known Tokens chain names, float round-trip, wallet details, two-line
  history rows, automatic refresh without a frequency setting, CLI suites by
  feature); new `scripts/README.md`.
- Total: 7 .rs files changed, +38 −59 lines; data and config files: `scripts/`
  (19), `core/` (1); dependencies unchanged; `#[test]` functions 830 → 831.

## Beta Commit 154

- **Portfolio state crosses as one versioned snapshot** (new
  `core/src/service/valuation.rs`; `service/state.rs` `portfolio_snapshot`)
  - Wallets, capabilities, dashboard groups, pins and valuation are derived from
    one resident state under the writer and carry an ordered session revision,
    so a front end can reject an older result.
  - Totals, dashboard valuation and movement alerts share one quote policy: a
    finite positive deployment quote, no testnet valuation and no fallback to a
    price stored on a holding; a partial total carries a missing-price count,
    and a missing currency rate gives no fiat value (before: these rules were
    also computed in the app).
  - CLI `portfolio --stored` reads it.
- **History reads are bounded SQLite queries** (new
  `core/src/service/history_query.rs`,
  `core/src/store/wallet_db/history_query.rs`; `fetch/history.rs`,
  `service/history_derived.rs`)
  - `history_page` deduplicates, filters, searches (Unicode lowercasing) and
    orders in SQLite and decodes one page plus lookahead (before: the whole
    table was loaded four times per refresh).
  - Deployment identity takes part in deduplication; records without one stay
    distinct, and case-significant hashes are kept; an unclassifiable identity
    or status fails the read.
  - `transaction_snapshot` reads recent and pending records, first-activity
    dates and a count in one read transaction; a record can be read by id.
  - The localized normalization interface and its duplicate record are deleted.
  - CLI `txs --page [--limit --offset --filter all|send|receive|pending]`, `txs
    --summary`, `txs --record <id>`; `rusqlite` gains the `functions` feature.
- **Owned sends unlock password-protected wallets**
  (`core/src/service/send_review.rs`, `cli/src/cmd/tx.rs`): the review reports
  whether the wallet needs a password (`requires_wallet_password`), and the
  CLI passes one to `execute_owned_send` from its file, environment or prompt
  source (before: no password was passed); a missing or wrong password never
  broadcasts and the review must be redone.
- **Configured self-tests follow the selected network**
  (`service/diagnostic_state.rs` `run_configured_self_tests`; CLI `diagnostics
  configured --chain <chain>`): the family resolves through the stored network
  selection (an explicit testnet stays explicit), runs that network's offline
  tests and, on an EVM network, probes its first effective RPC, naming the
  network and endpoint; a failure does not fall back to another endpoint.
- **Password validation returns reasons** (`core/src/validation/mod.rs`):
  `TooShort` or `ConfirmationMismatch` instead of English sentences, and the
  four-character minimum counts Unicode scalars (before: UTF-8 bytes); blank is
  still allowed. CLI `wallet check-password`.
- **An empty pin list stays empty** (`store/state.rs`): fresh settings store the
  default four pins, removing the last pin keeps none, and resetting the
  dashboard-customization scope restores the defaults (before: core substituted
  the defaults for an empty list).
- Tests: 4 new, 2 removed
  - Valuation 2: a missing, invalid or overflowed quote counted as zero or
    replaced by a stored price; wallets, groups and valuation from different
    versions.
  - Diagnostics 1: self-tests ignoring the selected network or accepting a
    wrong-chain response.
  - Passwords 1: untyped rejections, multibyte characters counted as bytes.
  - Removed: the in-memory history dedupe test and the empty-versus-never-pinned
    test, replaced by the bounded query and the stored empty selection.
- Docs: `docs/BEHAVIOUR-CHANGES.md` entries dated 2026-09-18 and 2026-09-19
  (password-protected owned sends, configured diagnostics, preview form capture,
  password reasons, committed runtime settings, portfolio snapshots and bounded
  history, recipient probes, transaction subtitle, empty pins, history filter
  entry point); `docs/ARCHITECTURE.md` snapshot revisions, bounded history and
  committed settings; `PLAN.md` new "Swift shell projection boundary follow-up
  (2026-09-19)" and "Swift shell: five audit fixes (2026-09-19)".
- Total: 26 .rs files changed (3 new), +1256 −931 lines; data and config
  files: `core/Cargo.toml`, `scripts/cli-acceptance.sh`,
  `scripts/cli-history-corruption.py`, `scripts/cli-projection-boundary.py`,
  `scripts/cli-shell-five-fixes.py`; dependencies unchanged; `#[test]`
  functions 828 → 830.

## Beta Commit 153

- **A dashboard group names itself, so a pinned asset held nowhere has no
  holdings** (`core/src/store/wallet_domain.rs`
  `CoreDashboardAssetGroup::identity`, `core/src/service/state.rs`): `identity`
  is the largest holding, or the catalog entry when the asset is held nowhere,
  and `holdings` lists only where it is held (before: name, symbol, artwork and
  price came from `holdings.first`, and a synthesized holding stood in for an
  unheld pin).
- **Holding templates store the chain's display name** (`core/src/tokens.rs`
  `TokenEntry::holding_template`; before: the catalog's str id such as
  `bitcoin`, unlike a stored holding).
- **Twelve tokens get marks instead of letter badges** (`core/data/tokens.toml`,
  `icons/crypto/`): cake, cbbtc, crv, ens, ethfi, gho, morpho, ondo, pendle,
  pepe, susde, susds.
- **Icon normalization handles Illustrator exports**
  (`scripts/svgo.config.mjs`): `stroke-width` is rescaled with circle geometry,
  `<style>` blocks are always inlined, and `xml:space` is removed.
- Tests: 3 new
  - Dashboard groups 1: an unheld pin given a holding, or a held asset's
    identity not being its largest holding.
  - Holding templates 2: a catalog template that canonicalization would rewrite,
    native and token templates naming the chain differently.
- Docs: `docs/BEHAVIOUR-CHANGES.md` entries "A dashboard row names itself, so it
  can hold nothing", "The chain breakdown draws chains, and says so when there
  are none", "A holding template names its chain the way a stored holding does",
  "A dashboard asset row no longer names a chain", "The transaction detail and
  About carry the backdrop, and copy buttons are their own".
- Total: 4 .rs files changed, +126 −26 lines; data and config files:
  `icons/crypto/` (12), `scripts/` (2), `core/data/` (1); dependencies
  unchanged; `#[test]` functions 825 → 828.

## Beta Commit 152

- **`spectra wallet receive` prints the address core reserves**
  (`cli/src/cmd/wallet.rs`): it calls `WalletService::receive_address(reserve:
  true)` on the wallet's resolved network and reports that network; a second
  call returns the same address (before: the stored index-0 account address,
  which on a UTXO chain is not the reserved one); an account chain still gets
  its stored address.
- **`spectra pool receive` removed** (`cli/src/cmd/address_pool.rs`): it made
  the same call under the keypool namespace.
- Docs: `docs/BEHAVIOUR-CHANGES.md` entries "The address book is a list of
  contacts, not a form with a list under it", "One bottom action bar for the
  setup, send and receive flows", "A receive address has one producer, and the
  wallet step has a selection".
- Total: 2 .rs files changed, +34 −24 lines; data and config files:
  `scripts/cli-acceptance.sh`, `scripts/cli-shell-ownership.py`; dependencies
  unchanged; `#[test]` functions unchanged at 825.

## Beta Commit 151

- **A secret store that cannot be read is an error, not an unsealed wallet**
  (`core/src/store/wallet_secrets.rs`, `store/secret_backends.rs`,
  `store/secret_store.rs`)
  - `is_sealed`, `is_private_key_backed` and `has_signing_material` return
    `Result`; only `NotFound` means no (before: any store failure answered
    `false`, so a sealed wallet whose verifier could not be read was treated as
    unsealed).
  - `wallet_secret_state` fails on a store failure; `wallet_derived_state`
    treats unreadable material as unable to sign, so no send is offered while
    the portfolio still renders.
  - `SecretStore::list_keys` is removed from the trait and every store; the
    `wallet_private_key` export is deleted.
- **Imports are planned from the form, not from caller answers**
  (`core/src/derivation/import.rs`, `service/wallet_import.rs`)
  - `WalletImportRequest` carries the name, chains, mode and watch-only entries;
    core mints ids, derives addresses, reads each family's network from settings
    and gives every imported wallet its network's native holding
    (`Chain::native_holding_template`; before: the CLI imported wallets with no
    holdings).
  - `core_validate_wallet_import_draft` takes `WalletImportDraftInput` (mode,
    chains, name, phrase words and language, key text, backup-quiz result,
    watch-only entries) and checks the phrase checksum, key and addresses itself
    (before: nine booleans the caller computed).
  - `WalletView.network_chain_id` is a `String`, no longer optional.
- **Settings updates validated and exposed as a pure rule**
  (`core/src/store/state.rs`)
  - The reducer refuses an `RpcEndpoint`, `MoneroBackendBaseUrl` or
    `BitcoinEsploraEndpoints` value that is not a URL (`AppSettingRejected`); an
    empty value still clears it. CLI `settings set rpc-endpoint.<chain>
    <non-url>` exits 3.
  - `app_settings_applying` exports the reducer's rule so a front end can show
    the result (trimmed, clamped) before core answers.
- **The dashboard's pins are answered per asset** (`store/state.rs`
  `StateCommand::SetDashboardAssetPinned`): each `CoreDashboardPinOption`
  carries `is_pinned`, counting the default set; pinning an asset that is not an
  option is refused; `dashboard_default_pinned_assets` is no longer exported.
  CLI `portfolio --unpin-token <id>`.
- **String states become enums**, with stored records changed without migration:
  `FiatCurrency`, `BackgroundSyncProfile`, `ResetScope`, `DiagnosticLogLevel`,
  `PriceAlertRejection` and typed `StateEvent` variants; record fields `nonce`,
  `confirmed_network_fee`, `estimated_fee_rate_per_kb`. CLI `settings set
  background-sync-profile <unknown>` and `settings reset --scope <unknown>` exit
  3.
- **Send warnings are enums** (`send/flow.rs` `HighRiskSendWarning`,
  `EvmRecipientPreflightWarning`): each variant carries exactly its fields; four
  chain-specific address-format codes become one `foreign_address_format { chain
  }`; the serialized form keeps `code`.
- **A price-alert refusal is a reason code** (`store/price_alerts.rs`):
  `missingCurrencyRate`, `invalidTarget`, `unknownAsset`, `duplicateAlert`,
  `alertNotFound`, worded by the CLI and the app (before: English prose shown
  verbatim).
- **A history run records its own diagnostics; feed state follows commands**
  (`service/history_operation.rs`, `store/state.rs`)
  - The refresh records per-wallet rows and marks the chain degraded or healthy,
    on scheduled runs too; `diagnostics_record` is internal.
  - Removing a wallet drops its pagination and rows; a network switch or an
    Esplora change restarts the family's feed.
  - A degraded detail is stored as the English template.
- **`BalanceRefreshEngine::reconcile_wallets(app_is_active)` acts only when the
  fetch entries change** (`fetch/refresh/engine.rs`): a changed list starts the
  engine or triggers a sweep, an empty one stops it, a rename or new balance
  fetches nothing; `stop` is no longer exported.
- **Keypool diagnostics report the recorded reservation** (`service/keypool.rs`
  `keypool_diagnostics(chain_name)`): each wallet's keypool with the
  owned-address row and path recorded when the reserved index was handed out;
  `spectra --json pool show` includes `reservedReceiveAddress` and
  `reservedReceivePath`.
- **Smaller boundary changes**
  - `known_wallet_addresses` merges a wallet's addresses with its owned rows.
  - The post-send notice comes from the stored record (new
    `WalletService::send_verification_notice`);
    `verification_notice_for_status` and `verification_notice_for_last_sent`
    are no longer exported.
  - `owned_addresses_for_wallet` and `keypool_state` are internal.
  - `fiat_currency_code` is removed; `append_chain_operational_event`,
    `clear_operational_events` and `fetch_all_history_records_typed` are
    internal.
- Tests: 17 new, 2 removed
  - Import form 4: a seed import with a bad checksum, a backup check required
    outside creation, a private key on an unsupported chain or with non-hex
    text, a watch-only import with an invalid address or a rename without a
    name.
  - Feed state 3: a history run not recording rows or chain health, a removed
    wallet keeping pagination and diagnostics, a network or Esplora change not
    restarting the feed.
  - Settings 2: the front end's applied setting differing from the reducer's, a
    non-URL endpoint stored.
  - Send warnings 2: a foreign address losing its chain, a warning serialized
    without its code.
  - Keypool 2: the reservation shown with the wrong path, known addresses
    missing owned rows.
  - Secret store 1, refresh engine 1, price alerts 1, dashboard pins 1: an
    unreadable store treated as unsealed, a rename counted as a change, a
    refusal without a code, an unpin applied to the wrong set.
  - Removed: `list_keys` backend test, the old currency-selection test.
- Docs: `docs/BEHAVIOUR-CHANGES.md` 20 entries dated 2026-09-16 and 2026-09-17
  (pins, hollow signatures, localization, endpoint URLs, settings, import
  holdings, import form, history diagnostics, enums, shell records, balance
  reconcile, send verification, secret store errors, send warnings, alert codes,
  fiat pricing, transaction sheet, keypool diagnostics, stored English text, the
  unreachable-export gate); `PLAN.md` new "Swift shell: correctness follow-up
  (2026-09-16)", "Swift shell: what was not shrunk enough (2026-09-17)", "Swift
  shell: hollow, reversed and untidy (2026-09-17)".
- Total: 78 .rs files changed (1 new), +2569 −1517 lines; data and config
  files: `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]`
  functions 810 → 825.

## Beta Commit 150

- **A failed network read is an error, not a zero balance or an empty history**
  (`core/src/fetch/chains/bittensor.rs`, `polkadot.rs`, `near.rs`, `cardano.rs`,
  `kaspa.rs`, `sui.rs`)
  - Bittensor balance and history refuse without a Taostats API key and send no
    request (before: `0 TAO` and no history, the default since `api_keys` starts
    empty); a failed or unparsable balance is an error.
  - Polkadot's balance no longer substitutes `"0"` when Subscan fails, and a
    Subscan `200` with a non-zero `code` is reported with its `message`.
  - History fetches for Bittensor, NEAR, Cardano, Polkadot, Kaspa and Sui return
    the failure instead of an empty list, so the refresh is not counted as
    successful.
- **QR payloads are parsed and validated in core** (`send/flow.rs`
  `scanned_send_address(chain, payload)`; CLI `send scan --chain <chain>
  <payload>`): candidates from payment URIs (EIP-681's `@chain-id` included) are
  validated against the chain and returned in normalized form; a payload with no
  valid address yields nothing (CLI exit 3).
- **Per-chain facts added to the registry** (`core/src/registry.rs`,
  `chains.rs`)
  - `needs_etherscan_api_key` marks the six chains whose history reads Etherscan
    (BNB Chain, Hyperliquid, Linea, Sei, opBNB, Sonic).
  - `SendBroadcastMode` (`SignsAndBroadcasts`, or `PreparesWithBackend` for
    Monero and its stagenet); `spectra --json chains` reports
    `sendBroadcastMode`.
  - `Chain::is_extension_block_destination`, used by the Litecoin MWEB fee rule
    and exported as `is_extension_block_send_destination`.
  - `CatalogColor` is a `uniffi::Enum` parsed from the TOML, so an unknown
    colour fails the catalog load (the CLI's chain colours use it).
  - `parse_bitcoin_esplora_endpoints` is exported.
- **Stored shapes tightened, without migration** (`store/persistence_models.rs`,
  `store/state.rs`, `store/mod.rs`)
  - A stored transaction's `status` is required;
    `CoreTransactionStatus::from_raw` is the one reading of status names, and
    only a provider's free-string status is still read by kind.
  - The EVM receipt's gas used, effective gas price and fee (in the chain's gas
    token, renamed `receipt_network_fee`) are written when the receipt resolves
    (before: decoded and dropped).
  - The fee-priority setting is a `FeePriority` enum (`economy`, `normal`,
    `priority`); `parse_fee_priority` reads CLI and settings-file values, and an
    unknown value is stored as the default.
  - `WalletService::save_state` / `load_state` and the `state` table are
    deleted.
- **`generate_mnemonic` refuses a word count BIP-39 does not define**
  (`core/src/validation/mod.rs`, `service/standalone.rs`; before: twelve words
  for any unknown count): `seed_phrase_lengths()` exports the five lengths with
  entropy derived from the word count, and CLI `wallet new --words 18` now works
  where only 12 and 24 were accepted.
- **Core mints the seed envelope's Keychain master key**
  (`store/seed_envelope.rs` `new_seed_envelope_master_key`): `MASTER_KEY_LEN`
  bytes from `OsRng`, an error when it cannot.
- **`core_history_source` names a stored history source**
  (`fetch/transactions.rs`): `Provider`, `ChainProviders` or `Internal`, none
  for blank and `none`; `spectra --json txs` carries `historySource`.
- **`core_is_private_key_hex` answers whether a key is valid hex** without
  returning it; `core_private_key_hex` becomes internal.
- **Fiat rates come from Open ER and Fawaz Ahmed only** (`fetch/price.rs`;
  before: also ExchangeRate.host, which now needs a key and answered with an
  empty map, and Frankfurter, which lacks AED).
- **Unused code and exports deleted**: `EvmChainContext` and
  `core_evm_chain_context`; `app_core_endpoint_records_for_chain`,
  `core_endpoint_role_mask`, the export of `app_core_endpoints_for_ids` and
  three unread `AppCoreChainEndpoints` columns (diagnostics checks, broadcast
  providers, Bitcoin wallet-store URLs); `core_normalize_history`,
  `core_active_wallet_transaction_ids`, `list_builtin_tokens`,
  `price_merge_live_updates`, `merge_price_updates`, `store_wallet_private_key`,
  `derive_bitcoin_account_xpub_typed`, `fetch_evm_history_diagnostics`.
- **New gate `scripts/uncalled-core-fns.sh`**, run by CLI acceptance, fails on a
  `pub fn` in core that nothing outside tests names; acceptance also gains
  `scripts/cli-history-source.py` and checks for `send scan`, broadcast mode and
  `--words 18`.
- Tests: 21 new, 13 removed
  - Failed reads 4: a keyless, failed or unparsable Bittensor or Polkadot
    balance reported as zero; a Subscan refusal losing its reason.
  - Registry 3: the Etherscan key on the wrong chains, a non-Monero chain
    prepared by a backend, an EVM chain without its EIP-155 id.
  - QR scan 4: a URI not reduced to its address, an address returned in a form
    the store does not keep, a payload for another chain yielding an address,
    candidates tried in the wrong order.
  - Fee priority 4: an unknown stored value not opening as the default, the
    stored spelling changing, the default still stored as a choice.
  - Seed lengths 3: wrong entropy for a length, a non-standard length
    substituted rather than refused.
  - History source 1, receipt cost 1, master key 1: an emitted source with no
    name, a receipt cost from one field or the wrong decimals, a key that does
    not seal or repeats.
  - Removed: five tests pinning the deleted endpoint projections, five on
    `EvmChainContext`, three on deleted price-merge and diagnostics code.
- Docs: `docs/BEHAVIOUR-CHANGES.md` 17 entries dated 2026-09-16 (settings
  sections from the registry, EVM receipt cost, platform preferences, shell
  lookups, QR scan, network card, fee rendering, preview keys, default
  derivation path, history source, seed envelope key, transaction status, fee
  priorities, seed phrase lengths, failed reads, fiat providers, dead code and
  gates); `PLAN.md` new "Swift shell: the view layer (2026-09-16)", the Sepolia
  proof inlined; `docs/FFI-BOUNDARY.md` on `uncalled-core-fns.sh`; `AGENTS.md`
  and `README.md` link `docs/IOS-UI.md`; `docs/APP-BOUNDARY-COVERAGE.md`,
  `KNOWN-OPEN-AUDIT.md`, `STAGE3-C2-CLOSURE.md`, `STAGE3-C2-FOLLOWUP.md` and
  `SEPOLIA-BROADCAST-PROOF.json` deleted.
- Total: 56 .rs files changed, +1304 −1174 lines; data and config files:
  `docs/SEPOLIA-BROADCAST-PROOF.json`, `scripts/cli-acceptance.sh`,
  `scripts/cli-history-source.py`, `scripts/uncalled-core-fns.sh`;
  dependencies unchanged; `#[test]` functions 802 → 810.

## Beta Commit 149

- **Token catalog reshaped** (`core/data/tokens.toml`, new
  `core/data/testnet-tokens.toml`, `core/src/tokens.rs`); the catalog holds the
  same 131 tokens and 268 deployment ids.
  - Each deployment is a `[[tokens.deployments]]` under its token, without the
    `token =` back-reference (before: two flat arrays, deployments grouped by
    network).
  - A deployment's `id` is no longer written in the file; core derives
    `{network}:native` or `{network}:{standard}:{contract}`.
  - The 32 testnet tokens and their native deployments move to
    `testnet-tokens.toml`; the catalog build asserts each file's networks match
    its environment.
- **CoinLore removed as a price provider** (`core/src/fetch/price.rs`): prices
  come from CoinGecko and CoinPaprika; the ticker fetch, decoder,
  `coinlore_nameid_for`, `AssetMarketIds::coinlore_nameid` and the 67
  `coinlore_nameid` lines are gone.
- **Six tokens get marks instead of letter badges** (`icons/crypto/`): stETH,
  wstETH, Aerodrome, Lido, LayerZero and Worldcoin name new SVGs; `mantle.svg`
  is replaced with Mantle's official mark on `#092c24`.
- **Four faceted icons draw facets in solid colours instead of opacity**
  (`ethereum`, `ethereumclassic`, `steth`, `wsteth`).
- Tests: 1 new, 1 removed
  - Token catalog 1: a token with no deployment.
  - The CoinLore id-default test is removed with the provider.
- Docs: `docs/BEHAVIOUR-CHANGES.md` entries "Testnet tokens live in
  testnet-tokens.toml", "A deployment's id is derived, not declared",
  "tokens.toml nests each deployment under the token it carries", "Four faceted
  marks draw their facets in solid colour", "CoinLore is no longer a price
  provider", "Six letter badges become marks, and Mantle's mark becomes the
  official one".
- Total: 2 .rs files changed, +125 −168 lines; data and config files:
  `icons/crypto/` (9), `core/data/` (3); dependencies unchanged; `#[test]`
  functions unchanged at 802.

## Beta Commit 148

- **A wallet row core cannot decode is skipped instead of failing the whole
  load** (`core/src/store/wallet_db/wallets.rs` `wallet_load_all`)
  - The row is logged with its id and left on disk; the other wallets load, and
    import and reset keep working (before: one undecodable row failed
    `app_state_load` and `open_state`, so every call into core failed; rows
    written before Beta Commit 144's override change did this).
  - Restoring a readable shape brings the wallet back;
    `CoreWalletDerivationOverrides` keeps `deny_unknown_fields` on input.
  - CLI acceptance rewrites one stored wallet to the pre-144 shape with
    `sqlite3` and checks listing, importing and recovery.
- **`ENDPOINT_ROLE_BACKEND` gets its own bit, `1 << 10`**
  (`core/src/app_core.rs`; before: `1 << 9`, shared with
  `ENDPOINT_ROLE_INDEXER`): non-EVM primary endpoint lists no longer include
  indexer rows without `balance`, such as broadcast URLs and URL prefixes
  (Bitcoin Cash 6 → 2, Bitcoin SV 7 → 2, XRP 5 → 4, NEAR 3 → 2, Bittensor 2 →
  1).
- **Repository gates and tooling added**
  - `AGENTS.md` (with `CLAUDE.md` pointing to it), no longer git-ignored: Git,
    architecture, verification, platform, Swift, resources and icon rules.
  - `Makefile` targets `verify`, `lint` (`cargo fmt --check`, `clippy -D
    warnings`), `fmt`, `test` (`cargo test --workspace`), `test-cli`,
    `test-ios`; the `help` target is removed.
  - `.github/workflows/ci.yml` runs formatting, clippy, tests and CLI acceptance
    on Ubuntu.
  - `rust-toolchain.toml` pins Rust 1.94.1 with rustfmt and clippy.
  - `LICENSE`: GNU GPL v3.
  - Stale `core/Cargo.lock` and `tools/uniffi-bindgen/Cargo.lock` removed.
- **Workspace made rustfmt- and clippy-clean** across core and the CLI;
  behaviour unchanged.
- **Behaviour-change log moved from `PLAN.md` to new
  `docs/BEHAVIOUR-CHANGES.md`**, unchanged; new entries go there.
- Tests: 3 new
  - Endpoint roles 2: two role names sharing a bit; a chain's primary list
    holding an endpoint that is neither an RPC node, a backend nor a balance
    source.
  - Wallet store 1: one unreadable wallet row hiding the readable ones or being
    lost.
- Docs: new `AGENTS.md`, `CLAUDE.md`, `docs/BEHAVIOUR-CHANGES.md` (moved log
  plus entries for localized copy, the endpoint role bit, three iOS surfaces and
  the unreadable wallet row); `PLAN.md` Rule 0 "Silence" and a known open item
  (the CLI cannot show the endpoint table it uses); `README.md` CI badge, the
  CLI, the iOS app, verification and license.
- Total: 42 .rs files changed, +574 −290 lines; data and config files:
  `.github/workflows/` (1), `.gitignore`, `LICENSE`, `Makefile`, `core/` (1),
  `rust-toolchain.toml`, `scripts/` (1), `tools/uniffi-bindgen/` (1);
  dependencies unchanged; `#[test]` functions 799 → 802.

## Beta Commit 147

- **Icon directories renamed: `icons/cryptoicon/` → `icons/crypto/` (78 files),
  `icons/fiaticon/` → `icons/fiat/` (4)**; `scripts/normalize-icons.sh`,
  `scripts/svgo.config.mjs` and the `every_named_mark_ships_a_file` test follow;
  artwork unchanged.
- Total: 1 .rs file changed, +1 −1 lines; data and config files:
  `icons/crypto/` (78), `icons/fiat/` (4), `scripts/` (2); dependencies
  unchanged; `#[test]` functions unchanged at 799.

## Beta Commit 146

- **The setup picker's popular chains are a catalog rank**
  (`core/data/chains.toml` `popular_rank`, `ChainEntry::popular_rank`; `spectra
  --json chains` prints `popularRank`): Bitcoin 1 through Litecoin 8, other
  chains none (before: eight ids in the app's setup view).
- **`app_core_resolve_derivation_path` returns the normalized path**
  (`core/src/app_core.rs`): the four-field `AppCoreDerivationPathResolution`
  (chain, path, account index, flavor) and its account-index and flavor helpers
  are deleted.
- **Unread outputs removed from the FFI** (`core/src/service/state.rs`,
  `core/src/diagnostics/export.rs`)
  - `WalletDerivedState` drops `signing_material_wallet_ids` and
    `private_key_backed_wallet_ids`; core still uses signing availability to
    decide send and receive enablement.
  - `core_diagnostics_json` drops the two Tron last-send-error parameters, which
    no caller ever set.
- Tests: 1 new
  - Catalog 1: popular ranks with gaps, duplicates or testnets.
- Docs: `PLAN.md` new "Swift shell residue sweep (2026-09-14)" and "Behaviour
  changed on purpose" entries for the unread state and FFI columns, the Tron
  send-diagnostic notice, the catalog rank, the performance-sample buffer, the
  send Live Activity and `app_core_resolve_derivation_path`;
  `docs/STAGE3-C2-CLOSURE.md` "Residue sweep (2026-09-14)".
- Total: 10 .rs files changed, +58 −113 lines; data and config files:
  `core/data/chains.toml`, `scripts/cli-acceptance.sh`,
  `scripts/cli-stage3.sh`; dependencies unchanged; `#[test]` functions 798 →
  799.

## Beta Commit 145

- **UniFFI raised to 0.31.2** (`core/Cargo.toml`,
  `tools/uniffi-bindgen/Cargo.toml`; `docs/FFI-BOUNDARY.md` "Regenerating
  bindings")
  - 0.31.2 emits `nonisolated(unsafe)` on callback-interface `vtablePtr` statics
    itself, so generated Swift needs no post-processing for Swift 6.
  - Generated Swift lifts Rust strings with `String(decoding:as: UTF8.self)`, so
    a string from core reaches Swift byte for byte (before: a leading U+FEFF was
    dropped behind a force-unwrap).
- **Unused core functions and exports deleted** (about 2,000 lines): EVM and
  UTXO refresh-target planners (`fetch/mod.rs`), history page decoders and
  Bitcoin snapshot merging (`history_decode.rs`, `history.rs`), `http_get` and
  the JSON-RPC probe (`fetch/http.rs`), diagnostics history aggregates, JSON
  object and UTXO payload builders (`send/preview_decode.rs`), the persisted app
  snapshot and secret index (`store/mod.rs`), `fetch_next_unused_address`,
  `fetch_bitcoin_next_unused_address_typed`, `clear_reserved_receive_index`,
  `core_parse_dogecoin_derivation_index`, `core_endpoint_str_id`, history
  source-tag formatting and `NearFtBalance`; `apply_resolved_pending_statuses`
  becomes test-only.
- **`icons/fiaticon/rub.svg` removed.**
- Tests: 0 new, 37 removed (with the deleted code they covered).
- Docs: `PLAN.md` "Behaviour changed on purpose": "UniFFI floor moves to
  0.31.2"; `docs/FFI-BOUNDARY.md` opening line and "Regenerating bindings".
- Total: 44 .rs files changed (1 deleted), +46 −2000 lines; data and config
  files: `Cargo.lock`, `core/Cargo.toml`, `icons/fiaticon/rub.svg`,
  `tools/uniffi-bindgen/Cargo.toml`; dependencies: uniffi 0.31 → 0.31.2
  (`core/Cargo.toml`, `tools/uniffi-bindgen/Cargo.toml`); `#[test]` functions
  835 → 798.

## Beta Commit 144

- **Owned sends need a core-issued, single-use review before execution** (new
  `core/src/service/send_review.rs`: `WalletService::review_owned_send`,
  `execute_owned_send(review_id, input, password)`; `send_execution.rs`
  `execute_confirmed_send`)
  - `review_owned_send` quotes the send, resolves the recipient (ENS included)
    and returns an `OwnedSendReview` with an id, the request, the preview,
    high-risk and recipient-preflight warnings and whether a self-send needs
    confirming; for EVM it pins nonce, gas limit and fees into the request.
  - A review lives 120 seconds in memory (at most 32 pending), is consumed
    before signing, and is refused when the inputs differ from the reviewed
    strings (sub-float amount edits included), when it is missing, reused or
    expired, and after a restart.
  - Execution refuses with "review again" when the sending address, holding,
    token contract or decimals changed, or when an automatic EVM nonce no longer
    matches the next nonce under the sender lock.
  - A review accepts fee and nonce edits only; gas limit, calldata, access list
    and sign-only overrides are refused.
  - `quote_owned_send` is no longer exported, and the old `execute_owned_send`
    (wallet, holding, amount, destination arguments) is replaced.
  - CLI `send quote` returns the review with its warnings; `send owned-broadcast
    --yes` reviews and executes the same inputs (`cli/src/cmd/tx.rs`).
- **Core decides what an app refresh does** (new
  `core/src/service/app_refresh.rs`:
  `WalletService::refresh_app(AppRefreshIntent, DeviceConditions)` →
  `AppRefreshResult`; CLI `diagnostics refresh --intent <json> --conditions
  <json>`)
  - Intents: scheduled, foreground, balances-updated, user, one chain,
    after-send, deep rescan; core selects the work and history cooldowns and
    returns state, pending results, failures and the poll interval, under one
    refresh lock.
  - Offline, no network work is done; a deep rescan offline reports a failure,
    and a deep rescan on a chain without deep UTXO discovery is refused.
  - A foreground refresh is skipped within 120 seconds of the last successful
    full refresh.
  - A deep rescan runs discovery, receive-reservation advancement, balances,
    history and pending polling for the requested network (or its family for a
    mainnet), and reports each failure; prices are not refreshed.
  - `BalanceRefreshEngine::configure_for_device` restarts the balance loop from
    core's wallet scope and refresh-frequency setting.
- **Derivation overrides reduced to an exact passphrase and HMAC key**
  (`store/wallet_domain.rs` `CoreWalletDerivationOverrides`; new
  `core/src/derivation/input.rs` `core_parse_wallet_derivation_input`)
  - `mnemonic_wordlist`, `iteration_count`, `salt_prefix`, `curve`,
    `derivation_algorithm`, `address_algorithm`, `public_key_format` and
    `script_type` are removed; unknown JSON fields are refused, with no
    migration.
  - Passphrase and HMAC text keep their whitespace; empty text means none.
  - `validate_for_chain` refuses a passphrase or HMAC key on a chain whose
    registry entry does not support it.
  - CLI `wallet import --derivation-input-file <file>`; a private-key import
    refuses it.
- **Imports canonicalize raw secrets in core** (`service/wallet_import.rs`): the
  mnemonic is split on any whitespace and lowercased, and private-key hex is
  canonicalized, before both derivation and storage.
- **Types, fields and keys renamed to the values they carry**, with no aliases
  or migration: `WalletSummary` → `WalletState`, `CoreImportedWallet` →
  `WalletView` (`to_wallet_state`, `to_wallet_view`); `asset_name` →
  `artwork_name` in `core/data/chains.toml` and `tokens.toml` and
  `asset_display_name` in persisted transaction and price-alert records;
  `core_*_icon_asset_name` → `core_*_artwork_name`; `WalletService::new_typed` →
  `new`; `sign_and_broadcast_send` → `execute_protocol_send`;
  `EvmSendResultDecoded` → `EvmSendDetails`;
  `SendExecutionResult.rebroadcast_payload` → `protocol_result_json`.
- **CLI acceptance assertions check the exit code** (new
  `scripts/cli-assertions.sh`, `scripts/test-cli-assertions.sh`;
  `scripts/cli-acceptance.sh`)
  - `contains` and `lacks` require exit zero as well as the output; refusal
    checks state their exit code.
  - The 27 filtered `cargo test` runs inside the acceptance scripts are removed.
  - New `scripts/cli-shell-boundary.py` covers raw-versus-canonical imports,
    derivation input, deep rescan and refresh refusals.
- Tests: 7 new
  - Send review 3: an edited (including sub-float) amount passing review, a
    tampered, reused or post-restart review executing, a stale nonce
    broadcasting.
  - App refresh 2: network work done offline, invalid networks accepted,
    deep-rescan provider failures hidden or an empty scope failing.
  - Derivation input 1: unsupported override fields accepted, secret whitespace
    trimmed.
  - Import 1: a raw mnemonic stored or derived differently from its canonical
    form.
- Docs: `PLAN.md` "Behaviour changed on purpose": "Test gate cleanup", "Deep
  rescan and import boundary cleanup", "Remaining Swift boundary rewrite",
  "Domain and boundary names describe the values they carry".
- Total: 100 .rs files changed (3 new), +1525 −677 lines; data and config
  files: `scripts/` (8), `core/data/` (2); dependencies unchanged; `#[test]`
  functions 828 → 835.

## Beta Commit 143

- **Owned send operations take a wallet, a holding and user input** (new
  `core/src/service/owned_send.rs`: `preview_owned_send`, `quote_owned_send`,
  `execute_owned_send`)
  - Core resolves the stored network, addresses, token metadata, balances and
    fees and converts fees to atomic units with checks; a failed provider read
    is an error, not a guessed fee, and a watch address previews without reading
    a secret.
  - New `spectra send quote`, `send owned-broadcast --yes`; `send preview`
    covers any stored holding, not only EVM.
  - Self-send ownership is checked against every managed wallet on the asset's
    network, including discovered UTXO addresses, with core-owned acknowledgment
    expiry (`spectra send self-check`).
  - `replacement_draft` builds a speed-up or cancel from the stored pending
    transaction with the live nonce and a required fee estimate (before: a fixed
    4/2 gwei fallback and amounts truncated to eight decimals). `spectra send
    replacement <id> [--cancel]`.
- **Send results stay typed until the output boundary** (new
  `core/src/service/send_result.rs`, `SignedSubmission`): EVM integers are
  range-checked, an empty submission id is refused and a sign-only result must
  carry signed bytes (before: results went through JSON and fields were scraped
  back, with empty-string and fallback defaults).
- **Price alerts are edited by intent** (`AddPriceAlert`, `TogglePriceAlert`,
  `RemovePriceAlert`; new `core/src/store/price_alerts.rs`): core converts the
  target with its stored rate, keeps sub-cent targets and refuses exact
  duplicates, and an edit cannot overwrite another rule's trigger flag. `spectra
  alert add --target --holding --currency`, `alert toggle <id>`, `alert remove
  <id>`.
- **Movement alerts are persisted in core** (new `core/src/service/movement.rs`;
  `spectra alert movement [--active]`): core keeps the baseline across reopen,
  reports each qualifying change once, re-baselines on composition changes,
  missing quotes or foreground observation, and excludes testnets.
- **Receive returns only a validated address** (`core/src/receive.rs` deleted
  with its message and source planners): core supplies an optional reserved
  address or an error, never a message that could become a QR payload.
- **Unnamed imports are named `Wallet N` inside the import writer** against the
  latest stored names (before: the caller supplied a start index and the CLI
  chain-specific defaults).
- **Staking is query-only** (`core/src/service/staking.rs`,
  `core/src/staking/service.rs`): `build_staking_tx`, the per-chain action
  builders and previews, `polkadot_fetch_nomination_pools`,
  `cardano_is_stake_address_registered`, seven error variants and the staking
  error export are removed; validator and position queries read the service's
  current transport. `spectra staking endpoints --chain <c>`.
- **Provider history carries transaction identity**
  (`core/src/service/history_refresh.rs`, `core/src/fetch/chains/evm.rs`)
  - EVM, UTXO, Solana and Tron history derive deployment ids from the actual
    network and contract, so a refreshed row merges with the local send of the
    same hash; a ticker never invents a contract.
  - Refreshed records store their actual network (before: the mainnet family
    name for testnets), and a merge refuses a deleted or moved wallet inside the
    write transaction.
  - EVM native history keeps failed execution, and missing execution evidence
    fails the fetch (before: every transfer confirmed).
  - Automatic polling carries the record snapshot it read; a changed record
    rejects the response, while a current one may report a failure or reorg back
    to pending.
- **Opening state publishes only after seeding commits** (`open_state`,
  `core/src/service/state.rs`): a failed first open stays unbound and a failed
  rebind keeps the previous state.
- **Derived wallet maps read one `CoreAppState` snapshot**
  (`core/src/service/state.rs`).
- **The service passes its `WalletDatabase` handle directly**
  (`core/src/store/wallet_db/`): the weak path index and the second key/value
  connection pool are removed.
- **Artwork resolves by token, deployment or network id** (new
  `core/src/store/artwork.rs`, replacing `core/src/store/chain_aliases.rs`;
  `spectra token artwork --token-id | --network-id | --deployment-id`): an
  unknown contract cannot borrow a symbol's mark, and the identifier-string
  exports are removed.
- **`normalize_token_identifier` accepts registry ids as well as display
  names** (`core/src/tokens.rs`), so SPL, TON and TRC-20 identifiers keep
  their case.
- **Dead code removed**: Stellar issued-asset signing (the native XLM payment
  remains), the Ethereum error classifier and its enum, the portfolio
  composition signature, `Chain::as_evm`, `Chain::has_network_choice`,
  `probe_endpoints`, `SolanaClient::account_exists`,
  `CoreImportedWallet::total_balance` and `plan_append_chain_operational_event`.
- **Export scripts count exported `pub(crate)` methods**
  (`scripts/count-exports.sh`).
- **Internal secret, history and recipient methods leave the FFI**; the
  closure audit records 145 callable exports.
- **CLI acceptance: new fixtures** (new `scripts/cli-owned-send.py`,
  `scripts/cli-shell-ownership.py`; `scripts/cli-acceptance.sh`,
  `scripts/cli-stage3-followup.sh`).
- Measured: 2026-09-13, iPhone 17 simulator smoke: the staking tab shows
  query-only functionality, Solana returned 100 validators, and a watch-only
  import without a name became "Wallet 1" with a working receive QR (`PLAN.md`
  "Swift shell ownership follow-up"; `docs/STAGE3-C2-CLOSURE.md` "Subsequent
  shell audit").
- Tests: 15 new, 36 removed
  - Owned operations 5: a non-EVM preview needing a secret or invalid input,
    alert intents losing another rule's trigger or the stored rate, movement
    baselines lost on reopen or alerting twice, re-baselining missed on
    composition or missing quotes, and staking ignoring current settings or
    explicit overrides.
  - History and status 4: sends and refreshes not merging on the exact network,
    same-symbol tokens sharing an identity, EVM reverts dropped or unknown
    status accepted, and fresh reorgs or failed receipts unable to correct
    confirmed history.
  - State, results and naming 4: a failed open publishing, derived maps reading
    different snapshots, typed EVM results overflowing or losing reviewed
    values, and default names allocated outside the writer.
  - Artwork and Stellar 2: an unknown contract borrowing a mark, and the native
    payment envelope's amount or signature.
  - Removed: `receive.rs` planner tests (16), staking action tests (4),
    artwork-by-ticker and chain-alias tests (9), send-result decoders and hash
    classifiers (4), the receive-source test, and the portfolio-signature and
    RPC-error-classification tests.
- Docs: `PLAN.md` stage table (C2 157 callables), "Stage 3 / C2 closure after
  the Swift ownership follow-up", "Swift shell ownership follow-up
  (2026-09-13)", "Core dead-code closure (2026-09-13)", "Shell ownership:
  receive, naming, movement and staking", "Core simplification: commit order,
  snapshots, results, database ownership and artwork", "Swift awaits storage
  readiness before asynchronous core operations", "Wallet editing opens the name
  field", "Provider history uses transaction identity and checked ownership",
  "Finish Swift-owned send and alert decisions (2026-09-12)";
  `docs/STAGE3-C2-CLOSURE.md` new "Swift ownership follow-up", "Final gate
  results (2026-09-12)" and "Subsequent shell audit (2026-09-13)".
- Total: 87 .rs files changed (6 new, 2 deleted), +3092 −3569 lines; data and
  config files: `scripts/cli-acceptance.sh`, `scripts/cli-owned-send.py`,
  `scripts/cli-shell-ownership.py`, `scripts/cli-stage3-followup.sh`,
  `scripts/count-exports.sh`; dependencies unchanged; `#[test]` functions 849
  → 828.

## Beta Commit 142

- **Networks and tokens are separate, explicit catalogs**
  (`core/data/chains.toml`, `core/data/tokens.toml`, `core/src/chains.rs`,
  `core/src/tokens.rs`)
  - Every concrete network, mainnet or testnet, is its own `[[networks]]` record
    with `family`, `environment` and a `native_deployment` reference; nothing
    inherits (before: mainnets in `[[chains]]`, testnets inheriting and clearing
    fields). The chain ticker is removed, and stable network ids are kept.
  - `tokens.toml` holds token identities with their own `id`, including native
    BTC, ETH and MNT, and deployments typed `Native` or `Protocol` with
    per-deployment precision: 131 tokens and 268 deployments (78 native). Native
    symbols, precision and market ids are derived from it.
  - Loading refuses duplicate ids, invalid protocol identifiers, wrong-network
    references and priced testnets; each testnet has its own unpriced native
    token.
  - The CLI no longer resolves a chain by its coin name or symbol
    (`cli/src/cmd/mod.rs`).
- **Holdings, sends and prices use network and deployment identity, never a
  ticker**
  - Wallets store a required `networkId` (before: an optional network mode that
    followed settings); holdings are keyed by (network, protocol, identifier),
    and core refuses a missing or invalid token identifier before storing. No
    storage adapter for older data.
  - A ticker equal to the gas symbol no longer means native: an ERC-20 called
    ETH builds ERC-20 calldata, ARB pays fees from the ETH balance, a missing
    contract is refused, and a send on a network other than the wallet's
    selected one in that family is refused.
  - Portfolio grouping uses token identity with per-deployment breakdowns;
    quote-cache keys are deployment ids; dashboard pins store token ids from a
    core-owned candidate list.
  - Local sends record their deployment id; a provider row without enough
    identity stays unknown rather than merged by ticker, and native speed-up
    requires the recorded native deployment.
  - CLI holding arguments are deployment ids (`send affordability --deployment`,
    token tracking refuses ambiguous symbols); new `spectra portfolio --stored`,
    `--pin-token`, `--pin-options`.
- **History, reset, receive, quotes and transport are owned operations** (new
  `core/src/service/history_operation.rs`, `core/src/service/reset.rs`)
  - `refresh_history` selects stored wallets, protocols and pagination; an empty
    explicit scope does no work, and a failed provider does not consume the
    cooldown, which is now per (wallet, network). `spectra history --save` and
    diagnostics use it.
  - `reset_data` validates scopes, awaits secret and domain cleanup, clears
    history, quotes and refresh clocks and returns the committed projection; a
    secret-store error keeps the wallet for retry. `spectra settings reset
    --scope <s> --yes`.
  - Receive uses the stored network, path and passphrase and a durable reserved
    index; opening receive is an offline reservation, and display reads register
    nothing. New `spectra pool receive <wallet>` and `pool discover-chain
    <chain>`.
  - Alert evaluation and dashboard grouping read stored quotes; a concurrent
    evaluation fires once and persists (`spectra alert check --stored`).
  - `new_catalog` builds endpoint slots from registry facts, and requests read
    committed RPC settings; explicit CLI or test endpoints still override.
- **History status commits are atomic and network-isolated**
  (`core/src/service/transactions.rs`, `core/src/store/wallet_db/`)
  - EVM history groups include the exact network (before: equal addresses on
    mainnet and testnet shared one response).
  - Status polling reads, decides and writes in one immediate SQLite
    transaction, so concurrent metadata survives, deleted rows stay deleted and
    a late result cannot undo a confirmation; trackers publish after the commit.
  - Every status write derives its indexed row through
    `history_record_from_payload` (before: Swift reference seconds copied into
    the Unix index); wallet deletion uses the normalized lowercase id; history
    id lookups use the primary key.
- **FFI surface 178 → 162 callable exports** (per `docs/STAGE3-C2-CLOSURE.md`).
- **CLI acceptance: new fixture** (new `scripts/cli-network-token-identity.py`;
  `scripts/cli-acceptance.sh`, `scripts/cli-stage3-followup.sh`):
  mainnet/testnet parity, native BTC and ETH, both MNT forms, ETH fees for ARB,
  an ERC-20 called ETH, cross-network grouping, unpriced testnets, invalid
  identifiers, reopening, reset scopes, stored alerts and dashboards, chain
  discovery and receive.
- Measured: 2026-09-12, final gates for the Stage 3 / C2 closure: Rust workspace
  841 tests, CLI acceptance 346 checks plus fixtures, iPhone 17 Pro 84 tests,
  all passing; FFI 162 callable exports (`docs/STAGE3-C2-CLOSURE.md`).
- Tests: 18 new, 4 removed
  - Network and token identity 6: a contract called ETH treated as native,
    precision shared across deployments, a testnet sharing or pricing a native
    token, invalid identifiers stored, native and protocol MNT merged, and
    symbols or market ids identifying a holding.
  - History and status commits 7: EVM groups crossing networks, scope and failed
    clocks decided by the caller, one wallet's cooldown blocking another,
    concurrent commits losing metadata or resurrecting rows, a failed commit
    publishing trackers, index sorting and wallet-id normalization, and id
    lookups scanning the table.
  - Owned operations 5: reset with an invalid scope or before secret cleanup,
    receive derivation without the passphrase, alerts firing twice or not across
    reopen, transport ignoring saved settings or explicit overrides, and receive
    display reads mutating state.
  - Removed: catalog-decimal and lookup-key tests in `core/src/formatting.rs`,
    and the one-market-id-per-symbol test.
- Docs: `PLAN.md` stage table (Stage 3 and C2 done), "Network and token identity
  rewrite (2026-09-12)", "Stage 3 / C2 closure", "History isolation, atomic
  status commits and indexed lookups", "Explicit networks and token
  deployments", "Close history, reset, receive, quote and transport ownership";
  new `docs/STAGE3-C2-CLOSURE.md`.
- Total: 74 .rs files changed (3 new), +2947 −1551 lines; data and config
  files: `core/data/chains.toml`, `core/data/tokens.toml`,
  `scripts/cli-acceptance.sh`, `scripts/cli-network-token-identity.py`,
  `scripts/cli-stage3-followup.sh`; dependencies unchanged; `#[test]`
  functions 835 → 849.

## Beta Commit 141

- **Manual transaction recheck is one core operation**
  (`recheck_transaction_status`, new `core/src/service/transaction_recheck.rs`;
  `spectra txs --recheck <id> [--endpoint]`)
  - It takes only the stored transaction id, checks the chain's capability and a
    32-byte hex hash, and reads that transaction on its recorded network
    whatever its status or backoff (before: Swift reset a tracker and re-ran the
    chain's sweep, which skipped failed and some confirmed records).
  - The write re-reads inside a SQLite transaction: a deleted row or changed
    hash, wallet or kind refuses the result, and concurrent metadata survives.
    Provider failures and mismatched hashes are errors that leave status and
    trackers unchanged.
  - An unconfirmed read clears old block, finality and fee data and resumes
    tracking; failed-to-confirmed clears the failure reason.
  - The exported `reset_status_tracker` is deleted.
- **Core runs the whole pending-maintenance sweep**
  (`WalletService::refresh_pending_transactions`,
  `core/src/service/pending_status.rs`; `spectra txs --refresh-pending`): scope
  comes from stored records, trackers are pruned even on an empty sweep,
  recorded networks are polled concurrently, and per-chain failures are reported
  beside committed changes. The scope query and tracker pruning are no longer
  exported.
- **Addresses compare in their chain's normal form** (`core/src/send/flow.rs`):
  the high-risk check no longer lowercases on top of `AddressNormalization`, so
  a case-variant lookalike of a book address on Bitcoin or Solana raises
  `new_address`; the self-send confirmation still folds case.
- **Keypool indices stop at the last non-hardened child**
  (`core/src/service/keypool.rs`): reserving or advancing past `i32::MAX`
  refuses, and narrowing clamps instead of wrapping to a negative index.
- **An unreadable balance or fee is an error** (`summary_display_balance` in
  `core/src/service/helpers.rs`, the simple-chain preview): before, an unknown
  chain or unparsable amount read as 0.0.
- **Two helpers simplified; behaviour unchanged**
  - `make_string_result` merged into `make_result`
    (`core/src/validation/address.rs`).
  - `normalize_aptos_token_identifier` (`core/src/tokens.rs`) copies string
    slices instead of casting bytes to `char`; non-ASCII input was already
    returned untouched.
- **Service state restructured; behaviour unchanged**
  (`core/src/service/mod.rs`, `keypool.rs`, `state.rs`)
  - `keypool` and `owned_addresses` are one `Keypool` under one lock with
    `load`, `forget` and `remember_owned`; `state_db_path` and `state_database`
    are one `StateBinding`.
  - `derive_bitcoin_hd_address_strings` is deleted;
    `sign_and_broadcast_shared_utxo`, `sign_and_broadcast_token`,
    `fetch_prices_typed` and `fetch_fiat_rates_typed` are free functions;
    `derive_bitcoin_account_xpub_typed` is a free exported function (the one FFI
    change).
- **The shared HTTP client keeps no idle connections under test**
  (`pool_max_idle_per_host(0)` under `#[cfg(test)]` in
  `core/src/fetch/http.rs`), so a test no longer reuses a connection bound to a
  dropped runtime.
- **CLI acceptance: new fixture** (new `scripts/cli-transaction-recheck.py`;
  `scripts/cli-acceptance.sh`, `scripts/cli-stage3-followup.sh`) for failed,
  confirmed and pending recheck transitions and failed reads across processes.
- Tests: 16 new, 1 removed
  - Transaction recheck 5: failed or confirmed records skipped or read on the
    wrong network, finality and reorg data left stale, invalid scope mutating
    trackers, failed or mismatched reads altering state, and deleted or changed
    rows overwritten.
  - Pending maintenance 2: unopened or corrupt storage read as empty, and the
    current instead of the recorded network polled.
  - Keypool 7: the index ceiling, wrap at the end of the keypool, negative
    narrowing, deleted wallets or reset chains left in one table, duplicate
    remembered addresses, and an untouched chain.
  - Address comparison 2: a case-sensitive lookalike folded into a known
    address, and an EVM address not matching the book in any case.
  - Removed: the tracker-reset unit test, replaced by the recheck tests.
- Docs: `PLAN.md` sections "Stage 3 / C2 follow-up: pending maintenance
  ownership", "Stage 3 / C2 follow-up: manual transaction recheck", and entries
  on the test HTTP pool, service methods, the keypool type, the database
  binding, address normal forms, keypool indices, unreadable balances and
  validator helpers.
- Total: 25 .rs files changed (2 new), +1489 −484 lines; data and config
  files: `scripts/cli-acceptance.sh`, `scripts/cli-stage3-followup.sh`,
  `scripts/cli-transaction-recheck.py`; dependencies unchanged; `#[test]`
  functions 820 → 835.

## Beta Commit 140

- **EVM nonces and submissions are journaled durably**
  (`core/src/service/send_execution.rs`, new `core/src/service/send_records.rs`)
  - EVM reads the `pending` nonce, submissions per database, network and sender
    are serialized, nonces are reserved from durable pending sends across
    restarts, and an atomic SQLite check refuses a nonce another process took
    (before: `latest`, reusable by concurrent sends).
  - Every protocol writes its send to the journal after signing and before
    submission; a response lost after submission keeps the exact input,
    identifier and nonce, and a missing receipt cannot expire a journaled send
    into a guessed failure. Monero prepares with `do_not_relay`, journals, then
    calls `relay_tx`.
  - Core writes the outgoing transaction record itself, before submitting, with
    hash, actual network, nonce and signed payload; sign-only requests write
    none, and an ambiguous failure stays uncertain.
  - `rebroadcast_transaction(id)` reads the stored chain and payload, refuses
    missing or competing formats and confirmed records, and requires a node
    transaction id. New `spectra send rebroadcast <id> --yes`.
- **Pending polls use the transaction's recorded network**
  (`core/src/service/pending_status.rs`), not the current selection.
- **History merges run inside one `BEGIN IMMEDIATE` transaction**
  (`core/src/service/transactions.rs`), so concurrent refreshes keep one
  identity per wallet.
- **SPL sends validate the mint** (`core/src/fetch/chains/solana.rs`): owner
  program and precision are read from the mint, associated accounts are derived
  for that program, and basic Token-2022 transfers work; mints with extensions
  are refused (before: caller precision and the legacy Token Program only).
- **Balance refresh merges and persists in core** (new
  `core/src/service/balance_refresh.rs`): core fetches native and tracked-token
  balances for the stored wallet and network, merges by chain-specific contract
  identity and persists before notifying; an unreadable token keeps its old
  balance, and a result for a deleted wallet or changed network cannot
  overwrite. `spectra refresh --wallet <id> [--endpoint]`.
- **Wallet deletion and network changes are transactional**
  (`core/src/service/balance_refresh.rs`, `core/src/store/`)
  - Deleting a wallet removes its history, addresses and keypool in the
    wallet's transaction after deleting secrets.
  - Network selection clears the whole family's derivation tables inside the
    settings transaction, and re-selecting the same network is a no-op.
- **Wallet import commits all wallets together after storing secrets**
  (`core/src/service/wallet_import.rs`): a secret or database failure leaves no
  committed import and cleans partial secrets.
- **Rename and inclusion are field intents against the latest stored wallet**
  (`RenameWallet`; new `spectra wallet inclusion`).
- **New `spectra wallet derived`** prints core-derived portfolio and signing
  capabilities.
- **Funds Finder is a core scan session** (new
  `core/src/service/funds_scan.rs`): batches of four, with funded, zero and
  failed reads told apart by exact smallest-unit digits; `spectra rescan
  [--dry-run]` uses it.
- **Diagnostics are one typed core state** (new
  `core/src/service/diagnostic_state.rs`): append, healthy, degraded, synced and
  clear intents; the global log is capped at 800 and each chain's view at 200.
  New `spectra diagnostics state [--command <json>]`.
- **Prices and fiat rates are refreshed for owned holdings in core**
  (`refresh_owned_prices`, `refresh_owned_fiat_rates` in
  `core/src/service/network_prices.rs`): failed or invalid quotes keep the last
  value, the cache survives restart, and scheduled refreshes obey a persisted
  cooldown. `spectra price --stored`, `--refresh`.
- **EVM preview for an owned wallet** (`preview_owned_evm_send`; `spectra send
  preview`, `send review`)
  - Core resolves the wallet's network, sender, token and exact amount.
  - UTXO and Dogecoin quoting no longer reads the seed.
- **Maintenance is readable without network**
  - New `pending_maintenance_chains` (`spectra txs --maintenance`) lists the
    chains whose stored transactions still need polling.
  - New `spectra diagnostics maintenance` prints the refresh policy under
    given device conditions.
- **FFI changes**: removed `fetch_prices_typed`, `fetch_fiat_rates_typed`,
  `refresh_fiat_rates`, `price_merge_live_updates`, `store_wallet_seed_phrase`,
  `store_wallet_private_key`, `fetch_evm_send_preview_typed`; added the four
  operations above.
- **Endpoint probes corrected** (`core/src/service/network.rs`)
  - ICP Rosetta is probed with POST and must list a network.
  - An explicit probe equal to its endpoint is honoured (Bitcoin SV).
  - Diagnostics report method and HTTP status.
- **Base history is reclassified as keyless `Open`** (`core/src/registry.rs`;
  before: `EtherscanV2`).
- **Boundary rules fixed by the app-boundary audit** (new
  `core/src/app_boundary_tests.rs`, `core/src/service/app_boundary_tests.rs`)
  - Bitcoin preview and status accept Bitcoin testnets.
  - Derivation path indices must be below 2^31.
  - Negative or non-finite movement observations cannot alert.
  - Portfolio signatures use unambiguous key boundaries.
- **CLI acceptance runs new fixtures** (`scripts/cli-acceptance.sh`; new
  `scripts/cli-stage3.sh`, `scripts/cli-stage3-followup.sh`,
  `scripts/cli-balance-refresh.py`).
- Measured: 2026-09-12, Ethereum Sepolia, a 0.000001 ETH self-transfer signed by
  `spectra send broadcast --sign-only` from an isolated test wallet and
  submitted unchanged: mined in block 11686443, gas 21,000, fee
  0.000022431561432 ETH (`docs/SEPOLIA-BROADCAST-PROOF.json`,
  `docs/KNOWN-OPEN-AUDIT.md`).
- Measured: 2026-09-12, live read-only endpoint checks: ICP Rosetta POST 200;
  Bitcoin SV WhatsOnChain 200 and Blockchair 404; Zcash Trezor Blockbook 403;
  Base Blockscout history answers Spectra's client (`docs/KNOWN-OPEN-AUDIT.md`).
- Tests: 39 new
  - Send journal and records 4: nonce reuse after response loss, restart or
    concurrency, rebroadcast on the wrong network or without a node id, polling
    the current instead of the recorded network, and concurrent history merges
    splitting identity.
  - SPL 1: mint program, precision or extensions not validated.
  - Balance refresh, deletion and import 5: stale network results, settings and
    derivation cleanup not rolling back together, deletion leaving secrets or
    relations, and failed imports leaving wallets or partial secrets.
  - Quotes and maintenance 5: failed quotes replacing the last one, quote state
    lost on restart, retry policy and forced refresh, scheduler coalescing, and
    finality scope or empty hashes.
  - Funds scan, diagnostics, cursors and wallet edits 5: zero confused with
    failed reads, unfinished batches, diagnostics not persisting, pagination
    crossing wallets or chains, and field intents overwriting each other.
  - App-boundary rules 15: dust shown as zero, hardened path round-trip,
    diagnostics redaction, envelope tampering, portfolio signatures, reset
    scopes, RPC error classes, testnet pricing, Bitcoin testnet preview and
    status, Dogecoin spent outputs, HD receive gaps, movement thresholds,
    private-key input, replacement nonces and simple-chain fee subtraction.
  - Owned preview and endpoints 3: preview using another network or a secret,
    HTTP denials hidden, and Rosetta probes without a network.
  - Derivation vectors 1: Decred and Kaspa against independent reference-wallet
    vectors.
- Docs: `PLAN.md` sections "Stage 3 / C2 completion work (requested ten items)",
  "Stage 3 / C2 follow-up (requested six items)", "Send reliability audit:
  nonce, journal, network, history and SPL", "Known-open endpoint correctness",
  "Six follow-up ownership slices", "Complete the ten Stage 3 / C2 ownership
  slices", "Wiki headers use the original coin artwork once"; new
  `FUTURE_PLANS.md` (product plans), `docs/APP-BOUNDARY-COVERAGE.md`,
  `docs/KNOWN-OPEN-AUDIT.md`, `docs/STAGE3-C2-FOLLOWUP.md`.
- Total: 78 .rs files changed (6 new), +4001 −735 lines; data and config
  files: `docs/SEPOLIA-BROADCAST-PROOF.json`, `scripts/cli-acceptance.sh`,
  `scripts/cli-balance-refresh.py`, `scripts/cli-stage3-followup.sh`,
  `scripts/cli-stage3.sh`; dependencies unchanged; `#[test]` functions 781 →
  820.

## Beta Commit 139

- **Aptos and Sui receive addresses are derived as their SDKs derive them**
  (`core/src/derivation/chains/aptos.rs`, `sui.rs`): Aptos uses SHA3-256 and Sui
  Blake2b-256(flag ‖ public key) (before: Keccak-256 for both). Wallets derived
  before are refused by the send identity check and must be re-imported; no
  compatibility path.
- **Tron, Sui and Aptos transactions are built and signed locally**
  (`core/src/send/chains/tron.rs`, `sui.rs`, `aptos.rs`, new `bcs.rs`)
  - Tron reads only a block reference from the node and encodes the TRX or
    TRC-20 transfer itself, checks the key's owner and signs with a one-minute
    expiry (before: it signed the transaction and hash the node returned). The
    trigger simulation is removed.
  - Sui signs a local BCS programmable transfer that splits the exact MIST
    amount from gas, with Blake2b-256 intent signatures (before: SHA-256 and the
    node's returned bytes); coin references are paginated up to 256 gas objects,
    and a failed execution or missing digest is not success.
  - Aptos signs a local BCS APT transfer with sequence, gas, expiry and chain id
    and refuses a node on another network (before:
    `/transactions/encode_submission` supplied the signing bytes).
  - Solana, Sui and Aptos signers take the 32-byte `Ed25519Seed` derivation
    returns (before: they demanded a 64-byte keypair and failed); secret-bearing
    params are redacted and zeroizing (new `core/src/send/keys.rs`).
- **TON and NEAR wire formats corrected** (`core/src/send/chains/ton.rs`,
  `near.rs`; new `core/src/derivation/chains/ton/cell.rs`)
  - TON sends build real cells with representation-hash signatures, a complete
    external message to the V4R2 wallet, StateInit at seqno zero and
    text-comment cells up to 4,096 bytes; the same cell code computes the
    derivation StateInit. Other modes and longer comments are refused.
  - TON reads seqno from `runGetMethod` (zero only for an uninitialized
    account), submits with `sendBocReturnHash` and requires a 32-byte message
    hash; failed reads are errors.
  - NEAR key and signature tags are one-byte Borsh discriminants (before: four
    bytes); NEAR token amounts use the contract's `ft_metadata` decimals and
    refuse when it cannot be read.
- **Kaspa signing uses Kaspa's encoding** (`core/src/send/chains/kaspa.rs`):
  u64-LE lengths, transaction-ID bytes as given and a zero payload hash (before:
  Bitcoin CompactSize, reversed ids); the sender key must match and zero or
  overflowing amounts refuse.
- **Solana self-transfers merge duplicate account keys**
  (`core/src/send/chains/solana.rs`), so a SOL or SPL transfer to oneself is no
  longer rejected by account locking.
- **Address validation tightened** (`core/src/validation/address.rs`,
  `core/src/derivation/chains/bitcoin_sv.rs`)
  - TON addresses verify checksum, flags and workchain, accept both base64
    alphabets and refuse test-only addresses on mainnet (before: any 48
    alphanumerics).
  - BSV addresses belong to one network (`BsvNetwork`), and the signer refuses a
    destination and change on different networks.
  - Sui coin types (`suiCoinType`) are validated; the high-risk check uses
    `is_valid_send_address`.
- **Bitcoin history pages through a buffered pager** (new
  `core/src/fetch/bitcoin_history.rs`, `HistoryPage<T>`; new
  `core/src/service/history_bitcoin.rs`)
  - Single-address continuation uses its saved cursor, HD block cohorts merge
    across addresses before UI pagination, and a failed read or write never
    advances the cursor; repeated cursors refuse.
  - HD history uses the wallet's network and its path's BIP44/49/84/86 script.
  - `spectra history --save` adds `--pages N` and `--endpoint URL`.
- **ENS destinations are re-resolved at submit** (`verify_send_destination`):
  the resolution cache is removed, and a destination that resolves differently
  from the reviewed address is refused. `spectra send destination` adds
  `--expected`.
- **Token preferences are edited by intent** (`StateCommand::AddCustomToken`,
  `RemoveCustomToken`, `SetCustomTokenDecimals`, `SetTokenPreferencesEnabled`,
  `ResetTokenPreferences`, `MergeBuiltInTokens`, replacing
  `SetTokenPreferences`; `tokenPreferenceRejected` events)
  - The reducer upper-cases the symbol, judges the contract by
    `CoreTokenHostingChain::contract_validation_kind`, treats the same contract
    in any case as a duplicate and refuses more than 30 decimals (before:
    clamped).
  - Opening state merges the built-in catalog and persists it; tracking is
    `is_enabled` on a row that always exists.
  - New `spectra token add|remove|decimals|reset --yes|track|untrack`.
- **A network switch resets the chain's derivation state in one transaction**
  (`reset_chain_derivation_state`, replacing `delete_keypool_for_chain` and
  `delete_owned_addresses_for_chain`); `spectra network set` performs it and
  reports `clearedDerivationState`.
- **Pending-status polling keeps its schedules and reports storage failures**
  (`core/src/service/pending_status.rs`; `spectra txs --poll-chain <chain>`
  drives a poll)
  - Pruning keeps every poll shape's trackers and backoff (before: it erased
    EVM and history schedules).
  - A failed storage read fails the poll (before: a pruning error was ignored
    and an unreadable store read as no records).
- **Password verifier envelopes are bounded**
  (`core/src/store/password_verifier.rs`): rounds between `MINIMUM_ROUNDS` and
  `MAXIMUM_ROUNDS`, and salt and digest lengths checked.
- **Send amount shortcuts come from core** (`core/src/send/amount_input.rs`, new
  `spectra send shortcut --maximum --decimals [--percentage]`): the fee-adjusted
  maximum floored to the asset's precision, never above the quote.
- **`parse_amount_input` uses the bounded exact parser**
  (`core/src/send/amount_input.rs` `parse_raw_amount`).
- **Integer overflow panics in release** for `spectra_core`, `spectra_core_ffi`
  and `spectra-cli` (`[profile.release.package.*] overflow-checks = true` in
  `Cargo.toml`).
- **Service and store modules split by domain; behaviour unchanged**:
  `service/network.rs` into `network_balance`, `network_tokens`,
  `network_history`, `network_hd`, `network_prices`; `service/send.rs` into
  `send_preview`, `send_destination`, `send_signing`, `send_broadcast`;
  `service/state.rs` into `keypool`, `address_discovery`, `transactions`,
  `wallet_import`, `operational_events`; `store/wallet_db.rs` into
  `store/wallet_db/`; `store/tests.rs` into `store/tests/`;
  `send/chains/wire.rs` renamed `bitcoin_wire.rs`.
- **Independent SDK vectors** (`core/testdata/send-audit-vectors.json` from
  `scripts/generate-send-audit-vectors.cjs` for Aptos, Sui, Solana and Tron;
  `core/testdata/protocol/transactions.json` from
  `scripts/generate-protocol-vectors.cjs` for TON and NEAR).
- **`make check-ui`** added to the `Makefile` (runs
  `scripts/check-design-tokens.sh`).
- **CLI acceptance: new sections** (`scripts/cli-acceptance.sh`; new
  `scripts/cli-bitcoin-history.py` drives 61 transactions over seven UI pages
  against a local fixture): amount shortcuts, TON address integrity, token
  tracking and custom tokens, network-switch derivation reset, reviewed
  destinations with Aptos and Sui SDK addresses, Bitcoin history pagination.
- Tests: 48 new, 2 removed
  - SDK and protocol vectors 11: Aptos, Sui, Solana and Tron derivation and
    signed bytes against official SDKs, TON and NEAR wire vectors, the Kaspa
    sighash vector, Solana self-transfers, failed Sui execution, unfunded local
    builders, and stored wallets reaching submission.
  - TON sends 2: a wrong signer, network or drain mode accepted, and failed
    seqno reads or submissions read as success.
  - Address validation 6: BSV version bytes, checksums, network and deriver
    agreement, TON checksum and network, and Sui coin types.
  - Bitcoin history 5: rows repeated or dropped at page boundaries, cohorts
    split across pages, failures claiming exhaustion, and HD addresses or
    testnet scripts diverging.
  - Token preferences 7: built-in edits, contracts judged by the wrong chain,
    case-variant duplicates, reset, symbol normalization, group tracking, and
    clamped precision.
  - Send composer 8: high-risk and composer checks disagreeing, a malformed
    destination not flagged, shortcuts above the quote, gas quotes filling token
    amounts, custom fees and clamped quotes (2), a changed ENS read reusing a
    reviewed address, and secrets printed by `Debug`.
  - Storage 9: password verifier bounds (4), chain derivation reset (2), and
    pending-status backoff, storage failures and repeated EVM requests (3).
  - Removed: the ENS cache test and the decimals-clamp round trip.
- Docs: `PLAN.md` sections "Protocol encoders and Bitcoin history pagination
  (second core audit)", "Audited signing boundaries, fresh recipients and
  service ownership", "A stored password verifier is input, not a promise", "A
  BSV address belongs to one network", "The high-risk send check asks the
  address question the same way", "Integer overflow panics in release instead of
  wrapping", "Send composer and everyday navigation", "TON/NEAR wire correctness
  and pending-poll integrity", "A failed address lookup is not an empty wallet",
  "A network switch takes the chain's derivation state with it", "The
  tracked-token list is edited by intent, not replaced wholesale", "Remove the
  redundant Asset Hub card"; `docs/ARCHITECTURE.md` new "Signing and service
  modules" and "Core module boundaries".
- Total: 138 .rs files changed (63 new, 3 deleted), +16851 −14473 lines; data
  and config files: `scripts/` (5), `core/testdata/` (2), `Cargo.toml`,
  `Makefile`; dependencies unchanged; `#[test]` functions 735 → 781.

## Beta Commit 138

- **The EVM assembler takes the typed amount string and converts it exactly**
  (`EvmSendAssemblyInput.amount: String`, `core/src/send/ethereum.rs`):
  before, it took an `f64`, so `1.1` assembled 1100000000000000089 wei in the
  preview's gas estimate, ERC-20 calldata and `spectra send assemble`. The
  assembler now refuses what signing refuses (`1e3`, excess precision);
  `amount_to_smallest_unit` converts through `parse_raw_amount`, and
  `u128_str_to_hex` is removed.
- **A Bitcoin transaction never pays out more than it spends**
  (`build_unsigned_spend`, `core/src/send/chains/bitcoin.rs`): extra outputs are
  part of the selection target on both the selected and pinned branches, change
  uses `checked_sub`, UTXO sums use `checked_add`, and the builder refuses any
  layout whose outputs exceed its inputs (before: extras were appended unfunded
  and the shortfall became zero change; no production caller passes extras).
- **ISO-8601 history timestamps are parsed safely** (`parse_iso8601_timestamp`
  in `core/src/fetch/history.rs`): non-ASCII input is refused instead of
  panicking on a char boundary, `Z`, `±HH:MM`, `±HHMM` and `±HH` offsets are
  applied (before: dropped), field ranges are checked, and the result is
  `Option` (the history caller still files an unreadable stamp as 0). The time
  divisor applies only to numeric timestamps.
- **Token send previews quote the token, read at its own decimals**
  (`fetch_evm_send_preview`, the Tron preview; `is_erc20_transfer`)
  - An ERC-20 preview reads the token's `balanceOf` and `decimals` for spendable
    and max (before: the native balance, so 1 ETH showed as "1 USDC"); the case
    is taken from the calldata selector.
  - A TRC-20 preview reads `decimals()` (before: divided by 1e6).
  - A failed token read fails the preview (before: 0.0).
- **Every chain with a token standard can host tokens** (`CoreTokenHostingChain`
  in `core/src/store/wallet_domain.rs`): Berachain, Sei, Celo, Cronos, opBNB,
  zkSync Era, Sonic, Unichain, Ink and X Layer join the eighteen.
- **Token catalog additions** (`core/data/tokens.toml`,
  `core/data/crypto-wiki.toml`; icons `usat.svg`, `usdt0.svg`, `xaut0.svg`
  added)
  - Polygon's `0xc2132D05…` row is USDT0, priced from `usdt0`. New USDT0, XAUT0,
    USAT and Bera USD (BUSD) deployments from the USDT0 deployments API.
  - L2 and gas tokens: ARB, OP, LINEA, SCR, BLAST, ZK, UNI and the Ethereum
    contracts of MNT and POL.
  - 23 wrapped, staked and protocol tokens over 66 deployments (stETH, wstETH,
    WETH, cbBTC, LBTC, rETH, sUSDS, sUSDe, GHO, ONDO, MORPHO, AERO, CRV, CAKE,
    PENDLE, ETHFI, LDO, ENS, PYTH, RAY, ZRO, WLD, PEPE), each capped at four
    chains; non-dollar additions ship disabled.
  - Polygon's native coingecko id is `polygon-ecosystem-token` (before:
    `matic-network`), and Scroll's symbol is `SCR` (before: `SCRL`).
  - A coin may ship without artwork; every chain must still name a mark
    (`every_chain_names_a_mark`).
- **Fallback price providers are asked by catalog id, never by ticker**
  (`coinpaprika_id`, `coinlore_nameid` columns in `tokens.toml` and
  `chains.toml`; `core/src/fetch/price.rs`)
  - The two hand-written id tables and the symbol fallback are removed; an asset
    with no verified id goes unpriced at that provider (BUSD is deliberately
    blank at CoinPaprika). `PriceRequestCoin` loses `symbol`.
  - The ids are read through `chains::native_market_ids()` and
    `tokens::market_ids()` and are not on the FFI records.
- **One owner for a send's destination address**
  (`WalletService::resolve_send_destination`, `Chain::resolves_ens_names`; new
  `spectra send destination --chain <c> --to <input>`)
  - ENS names resolve on Ethereum only, through a service-level cache, and every
    result comes back normalized for the chain; anything that is neither a valid
    address nor a name the chain resolves is `InvalidInput`.
  - `is_ens_name_candidate` and `resolve_ens_name_typed` are no longer exported.
- **The destination probe takes a wallet, a holding and the typed destination**
  (`send_destination_risk(wallet_id, holding_key, destination_input)`; `spectra
  send probe --wallet <w> [--asset] [--chain] --to <input>`, the
  token-descriptor flags removed): core derives the token from the holding,
  refuses an unknown holding, an unvouched token or an out-of-range precision,
  and probes the resolved destination.
- **Registry and catalog answers replace chain-symbol lists**
  (`core/src/fetch/history.rs`, `core/src/receive.rs`, `core/src/registry.rs`,
  `core/src/store/state.rs`)
  - History rows are named from the token catalog for the chain they arrived on,
    and chains use the catalog's coin name (before: a four-entry Tron table and
    "Stellar Lumens" / "NEAR Protocol" overrides).
  - The receive message reads the family from the registry, so UTXO testnets get
    their family's message; `ReceiveAddressResolverKind` (25 variants) becomes
    `ReceiveAddressSource` (stored Bitcoin account address, the chain's stored
    address, or none), and `core_receive_address_source` takes the chain name
    alone.
  - A NEAR token send carries `Chain::token_send_gas_reserve` (0.001 NEAR) on
    the preflight.
  - `AppSettings::pinned_dashboard_assets` answers
    `DEFAULT_PINNED_DASHBOARD_ASSETS` (BTC, ETH, USDT, USDC) while nothing is
    pinned.
- **CLI acceptance: new assertions** (`scripts/cli-acceptance.sh`): destination
  normalization and refusals (`.eth` on Arbitrum, Base, Polygon and Bitcoin),
  probe refusals for a missing wallet, unheld asset and unknown chain, and exact
  `send assemble` amounts.
- **Icon normalization fits exported artwork to the library**
  (`scripts/svgo.config.mjs`)
  - A square viewBox of another size is rescaled onto `0 0 64 64`.
  - Exporter `data-*` attributes are removed.
  - Inlined styles become presentation attributes (`convertStyleToAttrs`).
- Tests: 33 new
  - Send amounts and previews 9: a typed decimal not converting exactly, an
    18-decimal smallest unit lost, the preview accepting what the send refuses,
    a token amount at the wrong precision, an ERC-20 preview limited by ETH, a
    native preview not netting its fee, a TRC-20 scaled by TRX, an unreadable
    TRC-20 balance quoted as zero, and a NEAR token send without its gas floor.
  - Bitcoin outputs 2: a batch the inputs cannot cover, and any layout paying
    out more than it spends.
  - Destinations 5: a valid address left unnormalized or treated as a name, an
    empty destination, a name looked up off the chain that registers it, an
    unknown chain, and a cache hit not reported.
  - Timestamps 4: one instant in five spellings disagreeing, non-ASCII input
    panicking, a non-timestamp read as 1970, and leap seconds or the epoch.
  - Market ids 6: catalogs disagreeing about a gecko id, two assets claiming one
    listing, malformed ids, an asset silently unlisted at CoinPaprika, and
    paprika and CoinLore ids not coming from the catalog.
  - Receive, pins, artwork and hosting 7: a UTXO testnet without its family's
    message, an unknown or pathless chain, a blank stored address, a ticker
    deciding the receive source, a chain without a mark, an unpinned dashboard
    not reading as the default four, and the hosting list drifting from
    `token_standard`.
- Docs: `PLAN.md` sections "One typed amount, one conversion into integer
  units", "A Bitcoin transaction pays out no more than it spends", "A provider's
  timestamp is parsed as the untrusted string it is", "A send preview quotes the
  asset the amount field moves", "Omnichain Tether tokens and the hosting-chain
  list", "One owner for 'what address is this send going to'", "The destination
  probe names an asset, not a token descriptor", "A fallback price quote names
  the asset it prices", "Chain tokens: ARB, OP and the rest of the L2 slate",
  "Held assets: wrapped, staked and protocol tokens", "The catalog names an
  asset and the registry answers for a chain", "The receive screen asks where an
  address comes from, not which chain it is".
- Total: 28 .rs files changed, +2165 −628 lines; data and config files:
  `icons/cryptoicon/` (4), `core/data/` (3), `scripts/` (2); dependencies
  unchanged; `#[test]` functions 702 → 735.

## Beta Commit 137

- **Malformed signing data is refused** (`core/src/send/chains/icp.rs`,
  `wire.rs`, `bitcoin_cash.rs`, `bitcoin_gold.rs`, `bitcoin_sv.rs`,
  `cardano.rs`; `core/src/fetch/chains/icp.rs`)
  - ICP payload arrays must be non-empty, each payload valid hex with the IC
    request domain prefix and a 32-byte request id, and the signature type
    ECDSA; the derived public key is checked against the supplied one (before:
    bad hex became an empty preimage and missing arrays empty signature lists).
  - ICP submission keeps the 32-byte transaction hash in `txid`, and a missing
    or malformed hash fails (before: parsed as a block index with a zero
    fallback).
  - The shared Bitcoin-family txid decoder, the BCH, BSV and BTG preimages and
    Cardano input encoding refuse a non-32-byte or malformed hash; Cardano also
    refuses unbalanced or dust transactions.
- **Blockbook amounts must parse** (`core/src/fetch/chains/blockbook.rs`): a
  malformed, negative or overflowing balance or UTXO amount fails the read
  (before: became zero).
- **Derived history reads return errors**
  (`core/src/service/history_derived.rs`): normalized history, earliest dates,
  active-wallet transaction ids and replaceable sends return `Result` across
  UniFFI, including for an unopened store; an open empty store is still an empty
  success. `spectra txs --replaceable` fails rather than reporting nothing.
- **The destination probe distinguishes "no history" from "unknown"**
  (`send_destination_risk`, `core/src/service/send.rs`): balance and activity
  run concurrently with one balance read, a positive EVM nonce proves activity
  without an explorer call, and a failed read is an error.
  `fetch_evm_address_probe` and `EvmAddressProbe` are removed.
- **One seed-phrase verdict** (`core_check_seed_phrase(SeedPhraseCheck) ->
  SeedPhraseVerdict` in `core/src/validation/mod.rs`), replacing
  `validate_mnemonic`, `bip39_wordlist` and
  `core_validate_seed_phrase_word_count`
  - In order: an unfinished entry says nothing, words off the list are named,
    then the count, then the checksum.
  - A mnemonic is read in its own BIP-39 language when none is named, and only
    in the named language when one is (`parse_seed_phrase`, `bip39_language` in
    `core/src/derivation/primitives.rs`); before, derivation assumed English, so
    Chinese and other non-English phrases were refused. A wordlist name that is
    not a language is an error.
  - A seed import that derives no address refuses; the CLI checks the phrase
    before sealing (`reject_bad_seed_phrase`).
- **Core derives a private-key import's address**
  (`WalletImportCommit::private_key`, `derive_private_key_import_address` in
  `core/src/derivation/import.rs`); the CLI asks it before sealing.
- **`tokens.toml`'s first table is `[[tokens]]`**, and a deployment names its
  `token` (before: `[[assets]]` and `asset`; `core/src/tokens.rs`).
- **Icons normalized to one house style** (new `scripts/normalize-icons.sh` with
  `--check`, `scripts/svgo.config.mjs`; 67 files under `icons/cryptoicon/` and 5
  under `icons/fiaticon/` rewritten, e.g. transforms folded into path
  coordinates).
- **CLI acceptance: new assertions** (`scripts/cli-acceptance.sh`; new
  `scripts/cli-history-corruption.py` injects a corrupt history row into an
  isolated database): corrupt history refused without deleting records,
  seed-phrase messages, and a Chinese mnemonic import deriving a Bitcoin
  address.
- Tests: 19 new
  - Seed phrases 12: a finished phrase given a message, an unfinished entry
    judged, off-list words hidden behind the checksum, count and checksum out of
    order, unnormalized entries, the wrong language's word list, a picker
    warning on the field, a phrase read in the wrong language, entropy instead
    of words seeding the wallet, and an unknown wordlist falling back to
    English.
  - Signing data 3: an ICP request without domain separation, malformed ICP
    payloads reaching combine or a lost transaction hash, and unbalanced or dust
    Cardano transactions.
  - Failed reads 4: malformed Blockbook amounts, corrupt history altering
    storage or reading as empty, the destination probe reading the balance twice
    or guessing after a failure, and empty history confused with unknown.
- Docs: `PLAN.md` section "Refuse malformed signing data and failed history
  reads", entries for private-key import derivation and the seed-phrase verdict,
  and the verification note on flaky concurrent tests.
- Total: 24 .rs files changed, +954 −303 lines; data and config files:
  `icons/cryptoicon/` (67), `icons/fiaticon/` (5), `scripts/` (4),
  `core/data/` (1); dependencies unchanged; `#[test]` functions 683 → 702.

## Beta Commit 136

- **Core mints the ids of the wallets an import creates**
  (`core/src/derivation/import.rs`): an empty id plan is filled by core, and a
  supplied plan must still match the planned count (before: the caller had to
  predict the count).
- **`spectra wallet watch` takes `--address` more than once**, creating one
  wallet per address (`cli/src/cmd/wallet.rs`).
- **The send preflight names the token a send moves** (`token_contract_address`,
  `token_decimals` on the preflight; `core/src/service/maintenance.rs`,
  `core/src/send/mod.rs`): matched by contract where the holding has one and by
  symbol on the chain where it does not, and absent for a native asset or an
  untracked token, so the send refuses rather than guesses (before: each
  front-end submit branch resolved it, with Tron at six decimals for every token
  and NEAR defaulting to six).
- **A UTXO fee preview includes a destination's extra output bytes**
  (`fetch_utxo_fee_preview_typed` takes the destination;
  `core/src/send/preview_decode.rs`): Litecoin MWEB peg-in bytes are priced at
  the previewed rate and reduce what is sendable, stopping at zero;
  `extra_output_overhead_bytes` is no longer exported.
- **Icon library moved from `resources/` to `icons/`** (`icons/cryptoicon/`,
  `icons/fiaticon/`), with the artwork test's path in
  `core/src/store/chain_aliases.rs` updated.
- **CLI acceptance: one new assertion** (`scripts/cli-acceptance.sh`): a
  two-address watch import creates two wallets.
- Tests: 6 new
  - Wallet ids 2: an empty id plan not minted, and a mismatched supplied plan
    accepted, filing a wallet under an id nothing else knows.
  - Extra output bytes 3: overhead not priced at the preview rate, a zero
    overhead changing the preview, and sendable going below zero.
  - Token identity 1: a token sent at the wrong decimals or a native asset given
    token identity.
- Docs: `PLAN.md` entries for minted wallet ids, extra output pricing, Tron
  preview routing, the token a send moves and fee-free generic submit.
- Total: 9 .rs files changed, +367 −17 lines; data and config files:
  `icons/cryptoicon/` (75), `icons/fiaticon/` (5), `scripts/` (1);
  dependencies unchanged; `#[test]` functions 677 → 683.

## Beta Commit 135

- **A send is signed for the network the wallet is on** (`execute_send` via
  `WalletSummary::network_chain`, `core/src/service/send_execution.rs`; CLI
  endpoints resolved the same way): before, a send with Sepolia selected signed
  chain id 1 against mainnet endpoints. The wallet's own record wins over the
  family selection, and another family's selection does not move it.
- **Sign without broadcasting** (`SendExecutionRequest::sign_only`,
  `wants_sign_only`, `SendExecutionResult::signed_payload`;
  `Chain::supports_sign_only`)
  - One request field honoured by the EVM and Bitcoin builders returns the
    signed payload as a typed field (before: a flag on the EVM overrides only,
    and hard-coded `false` for Bitcoin).
  - A chain whose builder cannot stop before broadcasting (anything outside the
    EVM family and Bitcoin) refuses before any key is read; a run that ends with
    no payload is an error.
  - `spectra send broadcast` gains `--sign-only` (no `--yes` needed),
    `--gas-limit` and `--nonce`.
- **Balance and history fetches follow the wallet's network and file under the
  family** (`RefreshEntry` with both ids, `core/src/fetch/refresh/engine.rs`;
  `core/src/service/history_refresh.rs`): before, a testnet wallet read its
  address from the mainnet endpoints and got zero. The Bitcoin xpub arm also
  follows the network (before: it compared against `"bitcoin"` and always read
  Bitcoin's endpoints).
- **Pending-status polling is one core loop** (new
  `core/src/service/pending_status.rs`, `poll_pending_transactions`): it selects
  the records each chain's poll shape tracks (UTXO status, history txids, EVM
  receipt), fetches on the wallet's network, applies the results and returns
  what changed. `transactions_due_for_status_poll`, `record_status_poll`,
  `apply_resolved_pending_statuses`, `fetch_utxo_tx_status_typed` and
  `evm_transaction_status` are no longer exported.
- **Bitcoin history refresh runs in core** (`refresh_bitcoin_history`)
  - The account xpub is derived from the stored seed with the wallet's own
    BIP-39 passphrase (before: the empty string, so a passphrase wallet walked
    an empty range); a sealed wallet falls back to its stored address.
  - An entry with no timestamp keeps the merge's sentinel instead of the refresh
    time, and a failed fetch keeps the wallet's cursor.
  - Seed material is read through `load_signing_material`, and
    `SensitiveOverrides` is shared with `send_identity`;
    `fetch_bitcoin_hd_history_page` is no longer exported. The stored cursor
    still does not page.
- **Multi-address UTXO history refresh runs in core**
  (`refresh_utxo_chain_history`): a wallet with any address that did not answer
  merges nothing and counts as failed (before: it merged a partial netting).
  `core_refresh_targets` and `history_aggregate_by_transaction` are no longer
  exported.
- **Bitcoin send previews and fee rates take the wallet's network**
  (`fetch_bitcoin_hd_send_preview_typed`, `bitcoin_fee_rate(chain)` in
  `core/src/service/network.rs` and `send.rs`); a chain outside the Bitcoin
  family is refused.
- **Crypto icons moved from `resources/coinicon/` to `resources/cryptoicon/`**,
  and issuer-prefixed names shortened to the symbol (`circleusdc` → `usdc`,
  `tetherusdt` → `usdt`, `skydai` → `dai`, and ten more), with `asset_name` in
  `core/data/tokens.toml` to match; an empty `berausd.svg` added.
- **CLI acceptance: new "sign without broadcasting" section**
  (`scripts/cli-acceptance.sh`): a chain that cannot stop is refused by name,
  and a broadcast without `--yes` is a usage error.
- Tests: 8 new
  - Send network 2: a send signing for the family mainnet, and the two sign-only
    routes disagreeing.
  - History refresh 4: a testnet wallet fetching mainnet or filing under its
    network, a UTXO wallet with no addresses not skipped, a partly answered
    wallet storing a wrong netting, and a Bitcoin wallet with nothing to fetch
    for.
  - Pending status 2: a poll shape selecting the wrong records, and an unpolled
    chain doing work.
- Docs: `PLAN.md` entries for broadcast coverage, pending-status polling,
  network-following fetches, network-following sends, sign-only, Bitcoin and
  multi-address history, previews priced on the wallet's network and the bounded
  Bitcoin diagnostics run; the testnet-balance open item removed.
- Total: 21 .rs files changed (1 new), +2013 −415 lines; data and config
  files: `resources/cryptoicon/` (75), `core/data/` (1), `scripts/` (1);
  dependencies unchanged; `#[test]` functions 669 → 677.

## Beta Commit 134

- **Signing refuses instead of guessing gas, fees and expiry**
  (`core/src/send/chains/evm.rs`, `core/src/send/ethereum.rs`,
  `core/src/send/chains/zcash.rs`, `core/src/service/send_execution.rs`)
  - EVM native and ERC-20 signing estimate gas over the final calldata, value,
    nonce, fees and access list; a failed, zero or overflowing estimate refuses
    (before: native used 21,000 and ERC-20 fell back to 65,000). An explicit
    positive gas limit still skips estimation, and the 20% buffer rounds up in
    checked integers.
  - An `eth_feeHistory` block with an empty reward list reads as a zero priority
    fee; a missing `reward` array is still an error.
  - DOGE fee rates, ADA fees and Sui budgets convert the shortest decimal form
    exactly and refuse non-positive, non-finite, over-precise and out-of-range
    values before identity reads (before: a rounding, saturating float cast).
    New `spectra send fee-units --chain <c> --amount <n>`.
  - Zcash expiry propagates a failed tip read and refuses when tip + 40 does not
    fit u32 (before: a failed read became height 0).
  - EVM and Tron token metadata failures abort the send (before: fell back to
    the caller's decimals).
- **UTXO builders share checked accounting, wire encoding and fallback fees**
  (new `core/src/send/chains/accounting.rs` and `wire.rs`;
  `core/src/send/chains/{bitcoin,bitcoin_cash,bitcoin_gold,bitcoin_sv,dash,decred,dogecoin,litecoin,zcash,cardano}.rs`)
  - BCH, BSV, BTG, DASH, DOGE, LTC, ZEC, DCR and ADA sum inputs and fee in
    checked arithmetic and refuse zero sends (before: insufficient funds
    saturated into zero change).
  - Cardano refuses change below the minimum (before: small change was dropped
    without adjusting the explicit fee, leaving an unbalanced body).
  - A fee the caller does not supply falls back to `Chain::static_fee_units`,
    the value the send screen quotes (before: Litecoin signed 10,000 against a
    1,000 quote and Bitcoin Cash 1,000 against 2,000).
  - Bitcoin legacy coin selection sizes outputs from `SpendSizing`, so selected
    and pinned coins pay the same fee (34 vbytes per P2PKH output, before 31).
  - A malformed txid is an error (before: `unwrap_or_default()` built an empty
    outpoint), and `sign_p2wpkh` parses the from-address before coin selection
    like the other signers.
- **Failed reads are errors, not zeros**
  (`core/src/fetch/chains/{evm,tron,solana}.rs`, `core/src/service/network.rs`,
  `core/src/store/wallet_db.rs`)
  - TRC-20 integers must be one complete uint256 word with no high bits and
    decimals within 38 (before: the low u128 bits and a u8 cast).
  - Token balance reads propagate provider errors; within `fetch_token_balances`
    an unreadable token is left out rather than failing the others, and
    `send_destination_risk` refuses a token it got no row for. SPL reads sum
    every account for a mint (before: the first).
  - EVM previews require valid amount input and successful nonce, gas, balance
    and fee reads.
  - Keypool reads, reservations and advancement return a failed history query as
    an error, and out-of-range indices refuse; the keypool export now throws.
  - An unreadable `token_preferences`, `price_alerts` or `fiat_rates_from_usd`
    row is logged and dropped while its bytes stay stored; unreadable settings
    or wallet rows still fail the load.
- **One Blockbook client for Litecoin, Bitcoin Cash, Bitcoin Gold, Zcash and
  Dash** (new `core/src/fetch/chains/blockbook.rs`, `BlockbookClient<N>`),
  replacing five private copies; Bitcoin Cash normalizes CashAddr for every
  read, including `has_activity`.
- **History normalization fixes** (`core/src/service/history_derived.rs`,
  `core/src/fetch/history.rs`)
  - UTXO testnets normalize through their mainnet's shape (before: empty).
  - `amount` is always a magnitude with direction in `kind`, and a row with no
    amount is kept at zero on every chain.
  - EVM native history uses the requested page and size (page ≥ 1, size 1–500),
    and either provider failing fails the read.
- **History and balance refresh run in core over the wallets it holds** (new
  `core/src/service/history_refresh.rs`; `fetch/refresh/engine.rs`)
  - `refresh_chain_history` plans, fetches, builds and merges one chain's
    history and reports wallets refreshed and failed and records added and
    updated; record ids are dashed v4 UUIDs. New `spectra history <wallet>
    --save`.
  - `refresh_evm_chain_history` does the same for the EVM family with per-wallet
    diagnostics rows; `core_evm_refresh_targets`, `history_evm_native_asset` and
    `plan_evm_transaction_records` are no longer exported.
  - `BalanceRefreshEngine::sync_entries` builds the refresh list from core's
    wallets, using `WalletSummary::active_address` for the network the wallet is
    on; the CLI no longer builds its own list.
- **A seed import stores an address for every network of the chain's family**
  (`core/src/derivation/import.rs`, `WalletSummary`)
  - Each network's address is in its own slot, so switching to a testnet is a
    lookup; the Ethereum and Ethereum Classic slots are filled from the wallet's
    own address. `core_resolve_derived_or_stored_address` and
    `DerivedAddressPostProcess` are removed.
  - Wallets imported before this change hold only their mainnet slot and are
    re-imported, not migrated.
- **Replaceable pending sends are a core answer** (`replaceable_sends` in
  `core/src/service/history_derived.rs`; new `spectra txs --replaceable
  [--wallet]`): any EVM chain's pending send with a hash; Speed Up only for a
  gas-asset send, Cancel for both.
- **Tor settings and the kill switch are core settings**
  (`AppSettings::tor_enabled`, `tor_use_custom_proxy`,
  `tor_custom_proxy_address`, `tor_kill_switch`; CLI keys `tor-enabled`,
  `tor-custom-proxy`, `tor-proxy-address`, `tor-kill-switch`; `core/src/tor.rs`,
  `core/src/fetch/http.rs`)
  - The proxy address must be a `socks5://` or `socks5h://` URL with host and
    port; an empty value restores the default.
  - While Tor is wanted, the kill switch is on and Tor is not ready,
    `fetch::http` refuses requests (before: no code read the switch).
  - `bootstrap_tor` applies the SOCKS proxy before publishing `Running`.
- **The display currency is one of twelve codes** (`FIAT_CURRENCY_CODES` in
  `core/src/store/state.rs`): `SetFiatCurrency` refuses any other with an event,
  and the CLI exits 3.
- **A settings update core cannot store reports the refusal**
  (`appSettingRejected`, `core/src/store/state.rs`).
- **Fiat cross-rates are core state** (`CoreAppState::fiat_rates_from_usd` with
  its own row; `refresh_fiat_rates`; new `spectra currency --rates` and
  `--refresh-rates`): a provider failure leaves stored rates alone;
  `price_merge_fiat_rate_updates` is removed.
- **The RPC self-test covers any EVM chain** (`self_tests_run_evm_rpc` in
  `core/src/diagnostics/self_tests.rs`, replacing the Ethereum-only
  `self_tests_run_ethereum_rpc`): it checks the node against
  `Chain::evm_chain_id` and refuses a non-EVM chain.
- **`core_diagnostics_json_shape_ok` export removed**
  (`core/src/diagnostics/export.rs`).
- **CLI acceptance: new sections** (`scripts/cli-acceptance.sh`): checked fee
  units, addresses per network, replaceable sends and Tor routing.
- Tests: 70 new
  - Signing and fees 29: gas estimates skipped or swallowed, fee-history samples
    missing or malformed, float fee conversion, Zcash expiry overflow or an
    unavailable tip, unchecked UTXO totals, Cardano change, fallback fees
    disagreeing with the quote, P2PKH sizing, Bitcoin layouts (stable
    signatures, Taproot, dust, OP_RETURN size, insufficient funds, coin
    selection, RBF), wire encoding and malformed txids, and failed token
    metadata.
  - Reads and balances 11: partial uint256 values or invalid TRC-20 decimals, an
    unreadable token failing the others or reading as an empty wallet, malformed
    SPL accounts, Blockbook clients without endpoints, EVM history pages and
    provider errors, and EVM previews without every RPC read.
  - History 12: chain shapes, testnets, signed amounts, malformed input and
    ISO-8601 parsing; history refresh targets, network selection, record ids,
    token descriptors, unserved chains, non-EVM refusal and the empty case.
  - Replaceable sends 5: a non-EVM chain offered, a token send sped up, a row
    with nothing to replace, and the store answer changing across reopen or on
    an unopened store.
  - Refresh entries, keypool and settings 12: entries for the wrong network,
    xpub, missing address or EVM family; unreadable keypool history; per-network
    import addresses; Tor settings and the kill switch; currency codes; stored
    fiat rates; and rebuildable metadata versus settings.
  - Diagnostics 1: a non-EVM chain probed for a JSON-RPC chain id.
- Docs: `PLAN.md` sections "Checked signing, fees and history pages", "Failed
  reads refuse rather than fabricate state", "Blockbook clients, history shapes
  and Bitcoin signers", "Tor routing and the display currency", "Diagnostics",
  "Existing core-state migration: Swift integration", plus entries on stored
  per-network addresses, address-resolution columns, EVM and normalized history
  refresh, balance-refresh entries, replaceable sends and EVM composer controls;
  a "Testnet balances query mainnet endpoints" open item added.
- Total: 112 .rs files changed (4 new), +6676 −4012 lines; data and config
  files: `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]`
  functions 599 → 669.

## Beta Commit 133

- **A send names a wallet, and core resolves its signing identity from storage**
  (new `core/src/service/send_identity.rs`; `SendExecutionRequest`; new `spectra
  send identity --from <wallet> [--chain]`)
  - The request carries a wallet id and an optional unlock password; core
    resolves the stored path and overrides and exactly one signing source
    (before: the caller passed chain, sender, derivation data and two optional
    secrets, and the seed won when both were given).
  - A missing or watch-only wallet, an unrelated chain, ambiguous secret
    material and an address that disagrees with the derived key are refused
    before any provider read; a private-key wallet needs no derivation path.
  - Monero checks wallet-rpc account 0 against the resolved sender and submits
    through that same endpoint; a mismatch refuses, and a failed transfer is not
    retried on another endpoint. A NEAR named account is resolved by name, while
    an implicit account id must match the derived key.
- **Keypool reads use an indexed projection of source and change paths**
  (`core/src/store/wallet_db.rs`): SQLite expression indexes scoped to wallet
  and chain, so transaction bodies are not loaded (before: every read decoded
  the wallet's whole history). An unchanged reservation is not written again; a
  newly discovered floor still is.
- **TRC-20 symbol and decimals are cached for balance reads** (new
  `core/src/fetch/chains/tron/metadata_cache.rs`): up to 256 entries for five
  minutes, keyed by chain, endpoint list and contract; concurrent requests share
  one result, failures are not kept, and send preparation bypasses the cache.
- **UTXO address discovery derives the external xpub once and probes four
  addresses at a time** (`core/src/service/state.rs`,
  `core/src/fetch/chains/{bitcoin,bitcoin_cash,bitcoin_sv,dogecoin,litecoin}.rs`)
  - Public children are derived from the account xpub on a blocking worker, and
    no mnemonic is held during network probes (before: BIP-39 and the full
    BIP-32 path per index).
  - Activity comes from confirmed and pending counters (Esplora, Blockbook,
    BlockCypher) or BSV's balance and history index, without transaction bodies;
    results return in index order.
  - A missing counter, malformed response or provider failure is an error
    instead of "unused", and a failed probe never advances a reservation.
  - `spectra pool discover` opens the selected database and secret store
    (before: a service with no wallets); `spectra pool next` prints the derived
    receive address.
- **State commands write only what changed** (`core/src/service/state.rs`,
  `core/src/store/wallet_db.rs`): an incremental write set in one transaction; a
  settings change writes only its metadata, and a no-op command writes nothing
  (before: every wallet and address-book row was replaced).
- **Database connections are owned per database**
  (`core/src/store/wallet_db.rs`): weak handles replace a process-global map
  held under one mutex; each database locks independently, the service keeps its
  connection until rebind or drop, and SQLite waits up to five seconds on file
  locks.
- **CLI acceptance: new assertions** (`scripts/cli-acceptance.sh`) for `send
  identity`, `pool next` addresses and reservation stability after reopening.
- Tests: 22 new
  - Send identity 8: an EVM chain resolving a different identity, a mismatched,
    missing or watch-only wallet accepted, ambiguous material resolved to a key,
    a private-key wallet needing a path, a wrong password unlocking, a mainnet
    not resolving from stored data, Monero submitting through an unchecked
    endpoint, and a NEAR implicit account not matching the key.
  - Discovery 4: public children differing from full derivation, probes not
    bounded or out of order, pending or spent addresses missed, and malformed
    activity advancing a reservation.
  - TRC-20 cache 3: stale or failed metadata kept, a cancelled initializer
    stranding readers, and sends or new sources reading cached values.
  - Writes and connections 7: a setting update rewriting other rows, an
    unchanged reservation written again, the keypool projection leaking across
    wallets or missing edits, incremental writes not rolling back as one, one
    locked database blocking another, and connections not shared or not
    released.
- Docs: `PLAN.md` sections "Keypool history projections and TRC-20 read
  metadata", "Bounded discovery and incremental state writes", "Stored send
  identity and database connection ownership".
- Total: 28 .rs files changed (4 new), +1915 −432 lines; data and config
  files: `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]`
  functions 577 → 599.

## Beta Commit 132

- **Persistent state writes go through one service writer and publish only after
  SQLite commits** (`core/src/service/state.rs`, `core/src/store/wallet_db.rs`)
  - App-state and event snapshots are saved in order; keypool and address writes
    and deletes leave memory unchanged when SQLite fails (before: memory changed
    first).
  - An admitted write finishes even when the caller cancels its wait.
  - Opening a database uses the same writer and loads all tables before
    publishing; malformed event data is refused instead of silently cleared.
  - Deleting a wallet's combined records is one SQLite transaction.
- **A send amount is one exact decimal string** (`SendExecutionRequest`;
  `parse_raw_amount` in new `core/src/send/amount_input.rs`;
  `core/src/service/send_execution.rs`)
  - Every native and token builder converts with checked integer arithmetic and
    refuses invalid syntax, more decimals than the asset has, and overflow
    (before: a float plus an optional string, which chains rounded, truncated or
    ignored). Precision is bounded to 38 digits for u128, and protocol u64 and
    i64 limits are checked before signing.
  - Zero stays allowed for native EVM sends; other transfers require a positive
    amount.
  - New `spectra send amount --chain <c> --amount <n>` prints the raw units;
    `decimal_str_to_raw_units` and `amount_to_raw_units_string` in
    `core/src/send/preview_decode.rs` are removed.
- **Receive-address advancement compares and reserves atomically**
  (`core/src/service/state.rs`): a probe result for an index that is no longer
  current does nothing, and the successor skips addresses discovered while the
  probe was in flight (before: a stale probe could discard a newer reservation).
- **The manual EVM nonce is parsed in core** (`parse_evm_nonce`, `EvmNonceError`
  in `core/src/send/ethereum.rs`, replacing
  `core_ethereum_manual_nonce_validation` and
  `EthereumManualNonceValidationCode`)
  - The range is the signed i64 of the preview and FFI fields (before: Int32 in
    Swift); signed, fractional, hexadecimal and overflowing text is refused.
  - Previews refuse a negative nonce from the caller or the RPC.
- **CLI acceptance: new assertions** (`scripts/cli-acceptance.sh`) for exact
  amounts and the nonce range.
- Tests: 14 new, 2 removed
  - State writes 5: concurrent commands and events disagreeing with the reopened
    database, a failed commit publishing or not retrying, failed keypool or
    address writes changing memory, a failed log commit publishing, and a
    cancelled caller interrupting a commit.
  - Receive advancement 2: concurrent probes advancing a reservation they did
    not check, and a successor ignoring addresses discovered mid-probe.
  - Send amounts 4: units lost beyond float precision, invalid precision,
    Unicode or overflow accepted, exact amounts not reaching signing params, and
    overflow at the builder.
  - EVM nonce 3: a valid nonce across the FFI range refused, a negative nonce
    reaching a preview, and malformed or overflowing input accepted.
  - Removed: the float and exact decimal-to-raw-units tests in
    `preview_decode.rs`, with the helpers they covered.
- Docs: `PLAN.md` section "Atomic writes, exact send amounts and receive
  advancement", and the manual EVM nonce entry.
- Total: 12 .rs files changed (1 new), +1118 −586 lines; data and config
  files: `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]`
  functions 565 → 577.

## Beta Commit 131

- **EVM send overrides are validated and reach both native and token sends**
  (new `core/src/send/evm_overrides.rs`, `EvmSendOverridesInput::resolve`;
  `core/src/service/send_execution.rs`)
  - The chain, numeric signs, hex bytes and access-list schema, address and
    storage-key sizes are checked; an access list is carried into the
    transaction (before: it was discarded), malformed calldata is refused
    (before: it became absent), and a negative nonce or gas value is refused
    (before: it wrapped to an unsigned integer).
  - Explicit empty calldata stays distinct from absent calldata; custom calldata
    or a non-empty access list requires an explicit gas limit.
  - Unknown chains and malformed overrides fail before key derivation or network
    reads.
  - New `spectra send overrides [--chain] [--nonce] [--gas-limit] [--calldata]
    …` resolves overrides offline; invalid input exits 3.
- **Custom EVM fees are parsed in core into a configuration or a typed error**
  (`parse_evm_custom_fees`, `EvmCustomFeeError` in `core/src/send/ethereum.rs`,
  replacing `core_ethereum_custom_fee_validation` and
  `EthereumCustomFeeValidationCode` in `core/src/store/mod.rs`)
  - Both fees must be finite, at least one wei and within the u64-wei range, and
    max must cover priority (before: infinity was accepted); preview and send
    construction enforce the same checks.
  - New `spectra send fees --max-fee <gwei> --priority-fee <gwei>`; `inf`,
    sub-wei and overflow values exit 3.
- **CLI acceptance: new assertions** (`scripts/cli-acceptance.sh`) for `send
  fees` and `send overrides`.
- Tests: 9 new
  - Custom fees 3: a fee converting to the wrong wei, an infinite, sub-wei or
    overflowing fee accepted, and a preview built from invalid fees.
  - Overrides 3: bytes dropped on resolve, a malformed field defaulted instead
    of refused, and empty calldata conflated with absent.
  - Send builder 3: invalid fees reaching the builder without the UI parser,
    overrides missing from native or token sends, and malformed overrides
    reaching keys or the network.
- Docs: `PLAN.md` entries for EVM override validation, custom EVM fees and the
  presentation catalog (Swift); two open items removed.
- Total: 6 .rs files changed (1 new), +492 −108 lines; data and config files:
  `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]` functions 556
  → 565.

## Beta Commit 130

- **Docs rewritten shorter; no code changed.**
  - `README.md`: a short product statement and direction, a "Development"
    section naming the core, CLI, iOS app and Android skeleton, links to the
    plan and docs, and the CLI help command (before: long ethos, "what Spectra
    is and is not" and security-philosophy sections).
  - `docs/ARCHITECTURE.md` reorganised into "Workspace", "Ownership", "Chain
    rules" and "Boundary design".
  - `docs/FFI-BOUNDARY.md` reorganised: "What crosses, and how" and "Traps"
    become "Export shapes", "Runtime and storage traps" and "Callers and
    coverage".
  - `PLAN.md` condensed (+259 −3910).
- **`.gitignore` trimmed**: the cloud-sync, DerivedData and Xcode per-user,
  Swift Package Manager, per-assistant (all but `AGENTS.md` and `CLAUDE.md`),
  Rust backup, log, swap and secret-file patterns removed; `**/target/` becomes
  `target/`; `.gradle/` added.
- Docs: `README.md`, `docs/ARCHITECTURE.md`, `docs/FFI-BOUNDARY.md`, `PLAN.md`,
  as above.
- Total: no .rs files changed; data and config files: `.gitignore`;
  dependencies unchanged; `#[test]` functions unchanged at 556.

## Beta Commit 129

- **`is_valid_send_address` validates the normalized address**
  (`core/src/send/flow.rs`): a 64-hex Sui address typed without `0x` is
  accepted, as `AddAddressBookEntry` already accepted it (before: the composer
  refused it).
- **Icon artwork is resolved by the coin's symbol** (`core_icon_asset_name`,
  replacing `core_normalized_icon_identifier`;
  `core/src/store/chain_aliases.rs`)
  - A token draws its own mark on every chain it lives on, a chain ticker draws
    the chain, and a gas token falls back to the chain that pays in it; an
    unknown symbol resolves to nothing (before: artwork keyed on the chain, so a
    coin held away from home drew a letter).
  - `resources/coinicon/usdb.svg` added.
- **Monero takes the shared send submit path**
  (`Chain::uses_generic_send_submit`, `core/src/registry.rs`).
- **Shared BIP-32 and SLIP-10 code moved into
  `core/src/derivation/primitives.rs`** (`ExtendedPrivateKey`, `hmac_sha512`,
  `parse_slip10_ed25519_path`, `derive_slip10_ed25519_key`), replacing
  byte-identical copies in the Aptos, Cardano, Decred, EVM, ICP, Kaspa, Monero,
  Solana, Stellar, Sui, TON, Tron and XRP derivation modules; behaviour
  unchanged.
- **`core_evm_chain_context_tag` export removed** with its four tests
  (`core/src/send/flow.rs`).
- Tests: 11 new, 4 removed
  - Send addresses 2: a Sui address without its prefix judged differently by the
    composer and the store, and any mainnet answering differently before and
    after normalizing.
  - Send builder 1: a mainnet the router offers that `build_send_params` cannot
    build, which would fail after the user authorised the send.
  - Registry 2: a UTXO testnet not polling like its mainnet, and Monero lacking
    the preview the shared path needs.
  - Icon artwork 6: a token drawing the chain's mark, a chain ticker or gas
    token without artwork, a wiki coin without artwork, a named mark with no
    file, and an unknown symbol resolving to something.
- Docs: `PLAN.md` condensed (+607 −4864), with entries for Sui address
  normalization, router and builder agreement, the shared `submitSend` tail and
  post-send refresh (Swift), and the derivation duplicates;
  `docs/FFI-BOUNDARY.md` on checking an export is unused (`with_foreign` traits)
  and on exports with no Rust coverage.
- Total: 29 .rs files changed, +556 −1131 lines; data and config files:
  `core/Cargo.toml`, `resources/coinicon/usdb.svg`; dependencies unchanged;
  `#[test]` functions 549 → 556.

## Beta Commit 128

- **The endpoint directory is `core/data/endpoints.toml`** (replacing
  `core/data/AppEndpointDirectory.json`; `TomlEndpoint` in
  `core/src/app_core.rs`)
  - Rows may omit empty fields, and the file carries comments: the two
    vocabularies, why `history` needs care, and per-row notes on what was
    verified.
  - New export `app_core_endpoint_tag(url) -> Option<AppCoreEndpointTag>` gives
    an endpoint's kind and capabilities for a settings caption; a trailing slash
    is ignored, and a URL the catalog does not list has no tag.
- **Bittensor sends are routable** (`route_send_asset` in
  `core/src/send/mod.rs`, `Chain::uses_generic_send_submit`,
  `SendPreview::Bittensor` in `core/src/send/flow.rs` and
  `preview_decode.rs`): it routes to `"bittensor"` and takes the shared submit
  path with a `SimpleChain` preview shape under its own preview tag; no chain
  is left without a send route.
- **`simple_chain_default_fee_raw` removed**
  (`core/src/send/preview_decode.rs`): a second static-fee table with no caller,
  which disagreed with `Chain::static_fee_units` on eight of its eleven rows.
- Tests: 3 new
  - Endpoint tags 3: a catalog endpoint reporting the wrong kind or
    capabilities, a trailing slash treated as a different endpoint, and an
    unlisted URL given a tag.
- Docs: `PLAN.md` entries for unread Swift root declarations, Bittensor sends,
  the dead static-fee table and the TOML endpoint directory; the Bittensor open
  item marked fixed; metrics table.
- Total: 6 .rs files changed, +190 −36 lines; data and config files:
  `core/data/AppEndpointDirectory.json`, `core/data/endpoints.toml`;
  dependencies unchanged; `#[test]` functions 546 → 549.

## Beta Commit 127

- **Endpoint records say what an endpoint is and what it is for in separate
  fields** (`core/data/AppEndpointDirectory.json`, `core/src/app_core.rs`)
  - `roles` is replaced by `kind` (such as `rpc-node`, `indexer`) and
    `capabilities` (`read`, `balance`, `history`, `fee`, `broadcast` …), plus
    `supplementsRpcList`; an EVM node no longer claims address history, and a
    web link has no capabilities.
  - Ten EVM chains' JSON-RPC nodes (Sei, Celo, Cronos, opBNB, zkSync Era, Sonic,
    Berachain, Unichain, Ink, X Layer) are probed as RPC nodes (before: they
    lacked the `rpc` role and diagnostics reported them unreachable).
  - Dead endpoints removed, among them `eth.llamarpc.com`, `rpc.ankr.com/eth`,
    `polygon.llamarpc.com`, three Ethereum Classic nodes, the Blast and Aptos
    `blastapi.io` hosts, `1rpc.io/mantle`, `rpc.ankr.com/solana`,
    `near.lava.build`, two TronGrid hosts, two Polkadot nodes and
    `fullnode.mainnet.sui.io`; `tron-rpc.publicnode.com`, `rpc.polkadot.io` and
    `etc.etcdesktop.com` added.
  - `Chain::rpc_health_method` covers Solana (`getHealth`) and Sui.
- **New `spectra endpoints [--chain]` calls every registered endpoint and
  reports which answer** (`cli/src/cmd/chain.rs`,
  `WalletService::probe_chain_endpoints`); a JSON-RPC error body counts as
  unreachable.
- **EVM history comes from a per-chain source, and fourteen of 23 EVM chains
  need no API key** (`Chain::evm_history_source() -> EvmHistorySource` with
  `Open(base)`, `EtherscanV2`, `Unavailable`; `core/src/fetch/chains/evm.rs`)
  - `Open` sources (Blockscout, Routescan) are sent the Etherscan V1 query
    without `chainid` or key; `EtherscanV2` refuses locally without a key;
    `Unavailable` is an error (before: every EVM chain used Etherscan V2, and a
    missing key read as "no transactions").
  - An explorer refusal is told apart from an empty history by the type of
    `result`.
  - `normalize_chain_history` has an EVM arm (before: EVM history normalized to
    nothing).
  - `spectra chains` prints `historySource` and `needsApiKey` for EVM chains.
- **The UTXO address walk and receive-address reservation run in core**
  (`known_utxo_addresses`, `discover_utxo_addresses`, `utxo_receive_address`,
  `advance_used_utxo_reservations` in `core/src/service/state.rs`; new `spectra
  pool discover`): activity is one question for all twelve deep-discovery
  chains (any balance, UTXO or history), and the UTXO testnets are walked like
  their mainnets.
- **A wallet can be stored without a password**
  (`core/src/store/wallet_secrets.rs`: `store_seed_phrase`, `store_private_key`,
  `load_seed_phrase`, `load_private_key`, `has_signing_material`;
  `WalletService::wallet_secret_state`, `store_wallet_seed_phrase`,
  `store_wallet_private_key`, `wallet_seed_phrase`, `wallet_private_key`,
  `delete_wallet_secrets`)
  - Sealed and unsealed are told apart by whether a verifier exists; storing
    unsealed deletes any salt and verifier, and adding a password seals what was
    stored in the clear.
  - `spectra wallet import --no-password` stores an unsealed seed; it conflicts
    with `--password-file`.
  - The secret store keys are core's: the descriptors'
    `seed_phrase_store_key`, `password_store_key` and `private_key_store_key`
    and the `create_password_verifier` and `verify_password_verifier` exports
    are removed.
- **Send affordability counts the fee in core** (`send_affordability`,
  `SendAffordabilityInput`, `SendAffordability` in `core/src/send/mod.rs`; new
  `spectra send affordability`): the verdicts are affordable, amount exceeds
  balance, amount plus fee exceeds a native balance, or fee exceeds the gas
  balance for a token send; the gas asset and fee decimals come from the
  registry.
- **The recipient check is one core call returning two booleans**
  (`WalletService::send_destination_risk`, replacing `fetch_evm_address_probe`;
  new `spectra send probe`); the sentence is built by the front end.
- **New export `evm_transaction_status`** (`core/src/service/network.rs`)
  reads a broadcast EVM transaction's receipt: `None` while pending, otherwise
  confirmed and whether it reverted.
- **CLI acceptance: new sections** (`scripts/cli-acceptance.sh`): endpoint
  kinds, EVM history source, UTXO address discovery, wallets with no password,
  send affordability and the destination probe.
- Tests: 24 new
  - Endpoint records 4: an EVM node claiming history, a record kind no reader
    understands, the two vocabularies overlapping, and a web link with
    capabilities.
  - EVM history 7: an explorer refusal read as an empty history, a populated
    result rejected for its message, an open source sent a key, Etherscan V2
    asked without one, an unserved chain reported as a missing key, and the
    family table missing a chain or testnet.
  - Affordability 6: a governance token treated as the gas asset, a fee quoted
    at the wrong decimals, a token send checked against the fee first, an
    unresolvable chain waved through, and an unknown chain given a clean
    verdict.
  - UTXO discovery 2: a testnet missing from its mainnet's set, and a chain
    without the walk doing work.
  - Wallet secrets 5: a password-less wallet not reading back, adding or
    dropping a password leaving stale material, the password asked of an
    unsealed wallet, and a private key that cannot be stored unsealed.
- Docs: `PLAN.md` entries for endpoint kinds and capabilities, dead endpoints,
  keyless EVM history, receive reservation and the address walk in core, the
  single secret layout, the EVM pending-refresh recursion (Swift), send
  affordability and the recipient check.
- Total: 21 .rs files changed, +2082 −206 lines; data and config files:
  `core/data/AppEndpointDirectory.json`, `scripts/cli-acceptance.sh`;
  dependencies unchanged; `#[test]` functions 522 → 546.

## Beta Commit 126

- **`execute_send` builds typed send parameters in one match instead of a JSON
  string parsed back by a second match** (`build_send_params` in
  `core/src/service/send_execution.rs`; `sign_and_broadcast_send` in
  `core/src/service/send.rs`; `SendParams`, `SendTokenParams` in
  `core/src/service/send_params.rs`)
  - `execute_send` resolves the `Chain` once and passes it to both steps; each
    chain's amount arithmetic is unchanged, including Tron and Solana token
    amounts staying on the f64-scaled path and EVM and NEAR on the string-exact
    one.
  - The 21 `build_*_send_payload` functions, `parse_params`,
    `read_evm_overrides`, `render_evm_overrides_fragment` and both `json_escape`
    copies are deleted (`core/src/send/payload.rs`,
    `core/src/send/ethereum.rs`).
  - The Stellar custom-asset signing path (`StellarTokenSendParams`,
    `sign_and_submit_asset`) is still unreachable from `execute_send` and is
    marked `#[allow(dead_code)]`.
- **A history merge reads only its chain's rows** (`history_fetch_for_chain`,
  `history_fetch_where` in `core/src/store/wallet_db.rs`;
  `TransactionCommand::Merge`): before, every merge read the whole history
  table. The scoped fetches share one row decoder.
- **The simple-chain send preview reads a typed balance summary**
  (`fetch_simple_chain_send_preview` via `fetch_native_balance_summary` and
  `summary_display_balance` in `core/src/service/helpers.rs`):
  `WalletService::fetch_balance`, the free 24-arm `fetch_balance`,
  `simple_chain_balance_display` and `Chain::native_balance_field` are removed.
- **Fee estimates are two typed functions** (`bitcoin_fee_rate() -> FeeRate`,
  `native_fee_estimate(chain) -> NativeFeeEstimate` in
  `core/src/service/send.rs`), replacing one JSON-returning function with an
  unreachable EVM arm; `FeePreview`, `fee_preview` and `fee_preview_str` are
  removed. A chain with no fee is a named error.
- **The receive selection picks a holding, not a chain**
  (`core_receive_selection`, `core/src/store/mod.rs`): the native holding, or
  the first token when there is none; `receive_chains_by_wallet_id` and
  `WalletTransferAvailability.receive_chains` are removed.
- **Renames**: tokio's lock re-export is `AsyncRwLock` in
  `core/src/service/mod.rs`; `XRPSendPreview`, `TONSendPreview`,
  `ICPSendPreview` are `XrpSendPreview`, `TonSendPreview`, `IcpSendPreview`.
- Tests: 19 new, 8 removed
  - Send parameters 9: each chain family scaling its amount and default fee by
    the wrong unit, EVM overrides dropped, a non-EVM testnet or a token send
    with no decimals source not refused by name, and the two token amount paths
    merged.
  - Balances and fees 6: a chain dividing by another chain's decimals, NEAR's
    yocto amount divided instead of read, an unknown chain, a static fee not
    scaled by decimals, and a chain with no fee not named.
  - History 2: a scoped fetch returning other wallets' or chains' rows, and a
    bucket collision matching the wrong record.
  - Receive selection 2: no native holding and no holdings at all.
  - Removed: the JSON payload builders' tests.
- Docs: `PLAN.md` entries for the scoped history merge, EVM address resolution
  (Swift), one-chain wallets in receive and history, the single receive screen
  (Swift), the balance and fee dispatches, and three structures found by a cold
  read.
- Total: 25 .rs files changed, +1820 −2053 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions 512 → 522.

## Beta Commit 125

- **The chain wiki's editorial fields moved out of the chain catalog** (new
  `core/data/chain-wiki.toml`; `ChainWikiEntry`, `list_chain_wiki()` in
  `core/src/chains.rs`): `tags`, `comment`, `family`, `consensus` and
  `state_model` are one row per chain in their own file, joined by `chain`,
  and reachable only through `list_chain_wiki()`; `ChainEntry` drops from 23
  fields to 17 and `list_all_chains()` no longer carries them.
- **A wiki indexed by coin** (new `core/src/wiki.rs`, `list_asset_wiki()`,
  `AssetWikiEntry`, `AssetWikiPlace`; new `core/data/crypto-wiki.toml`)
  - One row per coin, 66 in all: the distinct coins the chains run on plus the
    catalog's tokens, with CRO counted once. Each row lists `lives_on`, every
    chain and standard it exists on with a contract or none when native; native
    places sort first in catalog order.
  - Each coin has a `comment` and, for native coins, a `total_circulation_model`
    (which moved here from the chain rows); the token catalog's per-token
    `comment` is removed.
- **A dashboard row is an asset across chains** (`CoreDashboardAssetGroup { id,
  holdings, is_pinned }`, `CoreDashboardAssetHolding` in
  `core/src/store/wallet_domain.rs`; `core/src/service/state.rs`)
  - The row key is the coingecko id (`cg:<id>`), so ETH on Ethereum, Arbitrum,
    Optimism and Base is one row; `holdings` has one entry per (network,
    standard, contract), largest value first (before: one row per network).
  - A holding without a coingecko id is keyed `contract:<chain>|<contract>`, so
    an unvouched token is always its own row.
- **Market-data ids corrected** (`core/data/tokens.toml`,
  `core/src/fetch/price.rs`): CRO `cronos` → `crypto-com-chain` (Cronos's native
  coin already used it), USDS `sky-dollar` → `usds`, and USD1 and WLFI get
  `usd1-wlfi` and `world-liberty-financial` (before: empty).
- **`core_dashboard_supported_token_entries` export removed**
  (`core/src/store/mod.rs`).
- Tests: 8 new
  - Asset wiki 6: a coin listed twice, a coin native to many chains split into
    rows, a coin native on one chain and a contract on another, a token missing
    a contract per chain, a wiki row for a coin the app does not have, and the
    table drifting from the two catalogs.
  - Catalogs 2: a symbol with two market-data ids across the chain and token
    catalogs, and a chain wiki row not taking its name from the catalog.
- Docs: `PLAN.md` entries for the chain wiki split, an unread Swift revision
  counter, the coin-indexed wiki, the crypto wiki, CRO's market-data id, the
  renamed-wallet resurrection (Swift) and the per-asset dashboard row.
- Total: 11 .rs files changed (1 new), +664 −181 lines; data and config files:
  `core/data/chain-wiki.toml`, `core/data/chains.toml`,
  `core/data/crypto-wiki.toml`, `core/data/tokens.toml`; dependencies
  unchanged; `#[test]` functions 504 → 512.

## Beta Commit 124

- **The chain catalog is two tables: `[[chains]]` for what a chain is,
  `[[networks]]` for each testnet** (`core/data/chains.toml`,
  `core/src/chains.rs`)
  - A network inherits every column from its chain except `token_standard`,
    `native_coingecko_id`, `address_prefix_hint` and the wiki block, and states
    its own id, name, path and other differences (before: each of 32 testnet
    rows restated its mainnet's columns, including a coingecko id that had to be
    overridden).
  - `primary_use` (identical to `comment` on all 78 rows) is removed;
    `contract_address_prompt` is derived from `token_standard` and `is_evm` from
    `category`, which no longer doubles as a testnet flag.
- **A dashboard row is keyed by (network, standard, contract) and holds one
  holding** (`CoreDashboardAssetGroup` in `core/src/store/wallet_domain.rs`,
  `dashboard_asset_grouping_key` in `core/src/formatting.rs`)
  - The group is `{ id, coin, value_usd, is_pinned }`;
    `CoreDashboardAssetChainEntry`, `chain_entries`, `representative_coin`,
    `total_amount` and `total_value_usd` are removed.
  - Two tokens without a coingecko id on one chain are two rows (before: the key
    fell back to the symbol and merged them).
- **Known-token balances report the contract's own decimals**
  (`fetch_token_balances` in `core/src/service/network.rs`): EVM via
  `fetch_erc20_metadata`, Sui and Aptos via new `fetch_coin_decimals`, NEAR via
  `fetch_ft_metadata`, TON via new `fetch_jetton_decimals`; the catalog's value
  is the fallback when the read fails (before: the caller's value on every
  family but Tron and Solana).
- **Where a chain's supplemental explorer endpoints go is a registry column**
  (`Chain::supplemental_endpoint_slot()`, exposed on the chain identity;
  `core/src/app_core.rs`): Polkadot and Internet Computer use `Secondary`, every
  other chain `Explorer`.
- Tests: 12 new
  - Chain catalog 6: the two tables not covering each other, a network not
    inheriting its chain's facts, a network inheriting a price, token standard,
    address hint or wiki block, and derived columns disagreeing with their
    source.
  - Supplemental endpoints 3: Hyperliquid's supplement lost, a chain other than
    Polkadot and ICP using `Secondary`, and a supplement overwriting a chain's
    primary endpoints.
  - Token balance decimals 2: an unreadable contract reported as zero, and an
    empty token list making a request.
  - Dashboard 1: two contracts on one chain merged into one row.
- Docs: `PLAN.md` entries for the two-table chain catalog, the dashboard row,
  the launch-time wallet merge (Swift), balance decimals and supplemental
  endpoint slots; two "Known open items" marked fixed.
- Total: 12 .rs files changed, +701 −246 lines; data and config files:
  `core/data/chains.toml`; dependencies unchanged; `#[test]` functions 492 →
  504.

## Beta Commit 123

- **The token catalog is two tables: `[[assets]]` for what a token is,
  `[[deployments]]` for where it lives** (`core/data/tokens.toml`,
  `core/src/tokens.rs`)
  - Name, coingecko id, colour, artwork, tags and comment are written once per
    token; contract, decimals, standard and `enabled` once per (token, chain). A
    deployment naming an unknown asset panics at load.
  - DAI's two names and colours on different chains became one.
  - `CoreTokenPreferenceEntry` holds a `TokenEntry` (`token`) instead of copying
    its fields, with `id()` as `chain|contract`; a token added from the CLI
    takes its category from the catalog's tags (before: always `Custom`).
- **Prices and fiat rates are asked of every provider at once and merged**
  (`fetch_prices`, `fetch_fiat_rates`, `merge_in_preference_order` in
  `core/src/fetch/price.rs`)
  - Providers run concurrently; the first answer per key in provider order wins
    (CoinGecko, CoinPaprika, CoinLore; Open ER, ExchangeRate.host, Frankfurter,
    Fawaz Ahmed); one failing provider no longer loses the others' answers, and
    only all failing is an error.
  - The `pricing_provider` and `fiat_rate_provider` settings and their CLI keys
    are removed (before: one chosen provider, and its failure meant no prices).
  - No price is pinned to $1.00 any more: `USD_STABLECOINS`,
    `is_usd_stablecoin`, `stablecoin_fallback_price_usd` and its export are
    gone, and an unquoted coin is absent from the map.
- **Staking is one export: `build_staking_tx(StakingActionRequest)`**
  (`core/src/staking/service.rs`, `core/src/staking/types.rs`)
  - The request is chain id, `StakingAction` (stake, join pool, unstake,
    withdraw, claim rewards, deregister, extend lockup), wallet address, amount
    as a decimal string, targets and months; it replaces 23
    `<chain>_build_<action>_tx` methods.
  - An action a chain does not have is refused with `UnsupportedAction`, and a
    bad field with `MalformedRequest`.
- **Diagnostics have one history record, `HistoryDiagnostics`**
  (`core/src/diagnostics/types.rs`, `export.rs`, `aggregate.rs`), with optional
  scanned count, cursor and error and `per_source` for multi-backend chains;
  `undecoded_count()` and `decoding_completeness()` are methods.
  `Chain::diagnostics_shape` and `DiagnosticsShape` are removed.
- **Chain self-tests are generated for every chain in the catalog**
  (`run_for_chain` in `core/src/diagnostics/self_tests.rs`, replacing the
  per-chain spec table and the Dogecoin and Ethereum special cases).
- **Address validators fixed** (`core/src/validation/address.rs`,
  `derivation/chains/{zcash,decred,dash}.rs`)
  - Zcash, Decred and Dash validators take the network, so a derived testnet
    address passes its own validator (before: the mainnet decoder refused it).
  - Solana addresses are base58-decoded and must be 32 bytes (before: length and
    alphabet only); Cardano addresses are bech32-decoded, so the checksum is
    checked.
  - `core_validate_string_identifier` and its records are merged into
    `core_validate_address`; `aptosTokenType` is one more kind.
- **Registry columns replace chain-name lists** (`core/src/registry.rs`,
  `send/flow.rs`, `app_core.rs`, `fetch/history_decode.rs`)
  - `Chain::address_normalization()` lowercases every EVM network's addresses
    (before: seven of 23 EVM mainnets).
  - The EVM destination checks run for every EVM chain, and the ENS warning for
    every EVM chain off Ethereum (before: two name lists; Base and fifteen
    others raised no warning for a Bitcoin address).
  - `broadcast_provider_options` covers the EVM family with one `is_evm()` arm;
    `history_evm_native_asset` reads `native_asset_name` and `gas_token_symbol`
    for every EVM chain (before: nine names).
  - `Chain::has_send_preview()` and an exported `extra_output_overhead_bytes`
    (non-zero only for Litecoin MWEB destinations).
  - `chain_aliases` resolves names and native symbols through the registry
    instead of two hand-written tables.
- **Status trackers are pruned by core** (`prune_status_trackers()`, replacing
  `retain_status_trackers(ids)`; `core/src/service/state.rs`): core keeps only
  pending transactions with a hash, and confirmed ones on chains whose poll
  tracks finality.
- **One record for a holding**: `CoreCoin` and `state::AssetHolding` are merged
  into `wallet_domain::AssetHolding`; `holding_identity` is exported.
- **Exports removed or narrowed**: `fetch_evm_receipt_classification`,
  `delete_history_records`, `replace_all_history_records`,
  `wallet_display_name`, `near_parse_history_response` (with its helpers) and
  `keypool_state_for_display` removed; `upsert_history_records`,
  `fetch_evm_has_contract_code` and `derive_bitcoin_hd_address_strings` are
  `pub(crate)`. `keypool_state` no longer persists and returns no error.
- **`scripts/count-exports.sh`** counts the FFI surface: `#[uniffi::export]`
  free functions plus every `pub fn` in an exported `impl`.
- **CLI acceptance runs `unreachable-exports.sh`** (`scripts/cli-acceptance.sh`,
  new "ffi surface" section).
- Tests: 28 new, 10 removed
  - Token catalog 3: two deployments of one token disagreeing about it, and the
    two tables not covering each other.
  - Price merging 4: a provider's unique answers dropped, a contested key
    changing between refreshes, one failure losing the rest, and all-empty
    treated as failure.
  - Registry-driven send rules 6 and broadcast providers 2: an EVM chain not
    lowercased or not checked, a case-significant address folded, an EVM chain
    without a native asset name or broadcast provider, a routable chain without
    a preview, and MWEB overhead on another chain.
  - Staking 4: a staking chain refusing a stake, the supported matrix drifting,
    and an unsupported action or unknown chain reported wrongly.
  - Chain aliases 3, self-tests 2, address validation 1, tracker pruning 2,
    token preferences 1: a chain name not resolving, a chain without a suite, a
    chain refusing its own derived address, confirmed transactions kept on
    chains that do not count depth, and an unreadable token-preferences row
    losing the wallets.
  - Removed: per-shape diagnostics round-trips and export tests, the dead NEAR
    history parser's tests, `wallet_display_name`'s test.
- Docs: `PLAN.md` entries for the two-table token catalog, unreachable exports,
  address and string validation, tracker pruning, the staking export, holding
  records and self-tests, diagnostics records, merged price providers, the
  chain-list sweep and stablecoin pins.
- Total: 39 .rs files changed, +2241 −2362 lines; data and config files:
  `core/data/tokens.toml`, `scripts/cli-acceptance.sh`,
  `scripts/count-exports.sh`; dependencies unchanged; `#[test]` functions 474
  → 492.

## Beta Commit 122

- **Amounts render at six significant digits chosen per amount, not a per-chain
  decimal setting** (`asset_amount_display` and `AssetAmountDisplay` in
  `core/src/formatting.rs`, replacing `resolve_asset_decimals`,
  `AssetDecimalsRequest`, `AssetDecimalsResolution`, `display_decimal_places`,
  `native_asset_display_settings_key`)
  - Places count from the first non-zero digit, capped by the asset's decimals
    and by 8; an amount below `10^-places` is flagged `below_threshold` so it
    renders as `<threshold` instead of zero.
  - The per-token `display_decimals` field is removed from
    `CoreTokenPreferenceEntry` and from the CLI (`token track
    --display-decimals` is gone, and the token list prints only the token's
    decimals).
  - New `spectra token format <amount> --chain <chain> [--symbol]` prints what
    an amount shows, at how many places, and whether it is below the threshold.
- **Token discovery: five chains are asked what an address holds**
  (`WalletService::discover_token_balances` in `core/src/service/network.rs`;
  `enumerates_holdings` in `core/data/chains.toml` and `core/src/chains.rs`;
  `HeldToken` in `core/src/fetch/chains/mod.rs`)
  - Solana, Tron, Sui, Aptos and TON (with their testnets) enumerate holdings
    through new `fetch_all_spl_balances`, `fetch_all_trc20_balances`,
    `fetch_all_coin_balances` and `fetch_all_jetton_balances`; each holding is
    matched to the catalog by contract address and marked `is_known`, and an
    unknown one is reported by contract address with the chain's own decimals.
  - A chain without `enumerates_holdings` (the EVM family, NEAR and the rest) is
    refused with "cannot enumerate holdings" rather than returning an empty
    list.
  - New `spectra token discover --wallet <w>` prints each holding with its
    contract; an unrecognised token prints no name.
  - The known-token balance path reports the decimals and symbol the chain
    returned instead of the catalog's (Tron, Solana).
  - `CoreTokenTrackingChain` is renamed `CoreTokenHostingChain`, and the
    identity column `token_tracking_chain` is `token_hosting_chain`.
- **A token send reads its decimals from the contract before signing**
  (`token_contract_decimals` in `core/src/service/send_execution.rs`): EVM
  tokens use `fetch_erc20_metadata` and Tron tokens `fetch_trc20_metadata`;
  the caller's `token_decimals` is the fallback, and a token with neither is
  refused (before: `req.token_decimals.unwrap_or(6)`).
- **EVM addresses with mixed case must match their EIP-55 checksum**
  (`core/src/validation/address.rs`; before: any 40 hex digits passed).
  All-lower and all-upper addresses are still accepted.
- **Zcash, Bitcoin Gold, Decred, Kaspa and Dash sends are routable**
  (`route_send_asset` in `core/src/send/mod.rs`; `Chain::send_execution_shape`
  in `core/src/registry.rs`): each routes to its existing `execute_send` arm
  with a fee fallback (0.00001 for Zcash, Bitcoin Gold and Kaspa; 0.00002 for
  Decred and Dash) and private-key support (before: the preflight answered
  that transfers were not enabled). Bittensor remains the only chain without a
  send route.
- **Exports removed**: `core_transaction_explorer_url` (the Aptos
  `?network=mainnet` suffix is a `txSuffix` field on the explorer record in
  `core/data/AppEndpointDirectory.json`),
  `core_supported_private_key_chain_names` (now the `derives_from_private_key`
  identity column), and `core_simple_chain_risk_probe_config`.
- **`EthereumSendPreview` is renamed `EvmSendPreview`**
  (`core/src/send/preview_types.rs`).
- **CLI acceptance: new assertions** (`scripts/cli-acceptance.sh`): EIP-55
  accept and refuse, amount formatting for small, large, 18-decimal and dust
  amounts, and discovery refused on a chain with no enumerating RPC.
- Tests: 9 new
  - Amount display 1: a zero, negative or non-finite amount asking for places.
  - Send routing 1: a chain with a send implementation that the preflight cannot
    route.
  - Token decimals 3: a chain assumed to host one decimal count, a family
    without a metadata client attempting a call instead of using the caller's
    value, and the set of families asked drifting.
  - Token discovery 2: the `enumerates_holdings` flag and the client arms
    disagreeing, and a chain marked that has no enumerating RPC.
  - Address validation 2: a mixed-case EVM address with a broken checksum
    accepted, and a chain refusing an address it derived itself.
- Docs: `PLAN.md` entries for amount display, Tron destination-risk checks,
  known tokens and discovery, contract decimals on send, Tron token decimals,
  the EVM zero-amount rule, export classification against the C2 target, the
  destination-risk gate, private-key chains, the five unroutable chains, EIP-55,
  the UTXO preview refresh and tracked-token balances.
- Total: 38 .rs files changed, +1385 −371 lines; data and config files:
  `core/data/AppEndpointDirectory.json`, `core/data/chains.toml`,
  `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]` functions 465
  → 474.

## Beta Commit 121

- **`import_wallets` derives a seed import's addresses itself, and `spectra
  wallet import` takes `--chain` more than once**
  (`core/src/derivation/import.rs` `derive_import_addresses`,
  `WalletImportCommit::seed_phrase`; `core/src/service/state.rs`;
  `cli/src/cmd/wallet.rs`)
  - A signing import whose `resolved_addresses` is empty and that carries
    `seed_phrase` gets its addresses from core: Ethereum is added whenever an
    EVM chain is selected, each path is read through
    `CoreSeedDerivationPaths::path_for`, and a chain that does not derive is
    skipped (before: the caller derived and passed `resolved_addresses`).
  - `wallet import --chain Bitcoin --chain Ethereum --chain Solana` seals the
    seed once per wallet and creates one wallet per chain; `wallet new` refuses
    more than one `--chain` with a usage error. The CLI's own `derive_address`
    is removed.
- **Applying resolved pending statuses is done in core, which reads, decides and
  writes the records** (`WalletService::apply_resolved_pending_statuses`,
  replacing `apply_resolved_pending_transaction_statuses`;
  `ResolvedPendingStatus`, `TransactionStatusChange` in `core/src/store/mod.rs`)
  - The caller passes a chain name and the network's resolutions; core computes
    stale failures, runs the planner, upserts the changed records and returns
    one `TransactionStatusChange` per record (before: the caller sent old
    status, reason and confirmations, got decisions back and wrote the records).
  - A transaction given up on stores the code `FAILURE_REASON_STUCK`
    (`"stuckAfterRetries"`) instead of a localized sentence.
  - `stale_pending_failure_ids` takes a chain name, reads core's own
    transactions and filters by `Chain::pending_status_poll`'s
    `require_send_kind`; it is no longer exported.
  - `ResolvedPendingTransactionInput`, `ResolvedPendingStatusInput`,
    `ResolvedPendingTransactionDecision`, `FailureReasonDisposition` and
    `StalePendingFailureTransactionInput` no longer cross the FFI.
- **Bitcoin and Internet Computer send fees use 8 decimals**
  (`Chain::send_execution_shape`, `core/src/registry.rs`; before: both fell to
  the default 6). ICP's shape also allows a private key and has no fee field.
- **Multi-address history aggregation is no longer named after Dogecoin**
  (`history_aggregate_by_transaction`, `MultiAddressAggregateInput`,
  `AggregatedTransaction`, formerly `history_aggregate_dogecoin`,
  `DogecoinAggregateInput`, `DogecoinAggregatedTx`;
  `core/src/fetch/history_decode.rs`).
- **Chains carry an address prefix hint** (`address_prefix_hint` in
  `core/data/chains.toml`, e.g. `bc1q…`, `0x…`, `UQ… / EQ…`;
  `core/src/chains.rs`).
- **Three history fetches are one `fetch_history_summary`**
  (`core/src/service/network.rs`), returning `HistorySummary { entry_count,
  confirmed_txids }` in place of `fetch_history_has_activity`,
  `fetch_history_entry_count` and `fetch_history_confirmed_txids`.
- **Dead functions removed**: `core_address_validation_kind`,
  `core_send_execution_shape`, `core_pending_status_poll`,
  `core_chain_display_name`, `core_network_choices`,
  `core_chain_str_id_for_name`, `core_seed_derivation_path_key`,
  `core_address_slot`, `core_supports_deep_utxo_discovery` (unexported,
  superseded by `core_chain_identities`), the exports
  `fetch_erc20_balance_typed` and `core_has_wallet_for_chain`, and
  `wallet_secret_index_from_observations`.
- **`SWIFT_REFERENCE_EPOCH_OFFSET_SECS` is defined once**
  (`core/src/store/persistence_models.rs`; before: privately in
  `fetch/transactions.rs` and `store/wallet_db.rs`).
- **CLI acceptance: 4 new assertions** (`scripts/cli-acceptance.sh`, 140 → 144):
  a three-chain import derives each chain's address, and `wallet new` refuses
  two chains.
- Tests: 3 new
  - History aggregation 1: two legs of one Litecoin transaction left as two
    records or netted wrongly.
  - Fee decimals 1: a UTXO or e8s chain falling back to 6 fee decimals.
  - Pending status apply 1: a resolution decided but never written to the
    database.
- Docs: `PLAN.md` entries for multi-address UTXO history, chain-name dispatch
  169 → 98, the zero-amount EVM send gate, core-side import derivation, the
  unified derivation loop, nine leftover `core_*` functions, dead Swift
  functions, the status-poll apply, `stale_pending_failure_ids` and
  `fetch_history_summary`.
- Total: 15 .rs files changed, +616 −239 lines; data and config files:
  `core/data/chains.toml`, `scripts/cli-acceptance.sh`; dependencies
  unchanged; `#[test]` functions 462 → 465.

## Beta Commit 120

- **Comments that recorded history, divider banners naming the item below them,
  and doc comments restating a signature removed across `core/` and `cli/`**
  (among them `registry.rs`, `store/`, `service/`, `derivation/chains/`,
  `send/flow.rs`, `cli/src/out.rs`); no code changed.
- Docs: `PLAN.md` entries for the comment sweep and for the multi-line
  chain-list sweep (Swift); metrics tables.
- Total: 33 .rs files changed, +3 −183 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions unchanged at 462.

## Beta Commit 119

- **`spectra network list` and `spectra network set <chain-id>` read and select
  which network of a family a chain is on** (new `cli/src/cmd/network.rs`):
  `list` prints every mainnet that has a network choice, the chain it is on,
  whether that is a testnet and its choices; `set` takes a registry id,
  refuses one the registry does not know, and a family's own id selects its
  mainnet (before: `network_chain_by_family` had no CLI command).
- **`spectra settings reset --yes` puts every core setting back to its default**
  (`StateCommand::ResetAppSettings`, `core/src/store/state.rs`;
  `cli/src/cmd/settings.rs`)
  - The reducer sets `settings = AppSettings::default()` and emits
    `appSettingChanged` only when something changed.
  - Without `--yes` the command exits 2 naming what would be discarded.
- **Every chain can have a custom RPC endpoint, not only Ethereum**
  (`AppSettings::rpc_endpoint_by_chain`, `AppSettingUpdate::RpcEndpoint { chain,
  value }`)
  - `ethereum_rpc_endpoint` and `AppSettingUpdate::EthereumRpcEndpoint` are
    replaced by a map keyed by chain display name; an unknown chain is refused
    and an empty value removes the override.
  - The CLI key is `rpc-endpoint.<Chain>` (before: `ethereum-rpc-endpoint`);
    keyed settings are one `CHAIN_KEYED` table of prefix, read, update and
    stored-keys, which `fee-priority.` also uses now.
  - `EndpointField::EthereumRpc` is renamed `EvmRpc`; the rule (a valid http(s)
    URL) is unchanged.
- **CLI acceptance: 18 new assertions** (`scripts/cli-acceptance.sh`, 122 →
  140): network list and set, reset with and without `--yes`, two families on
  testnets returned to mainnet by `settings reset`, and per-chain RPC endpoints.
- Tests: 1 new
  - Settings reset 1: a field added to `AppSettings` that the reset leaves at
    its old value, and a second reset that still emits an event.
- Docs: `PLAN.md` entries for duplicate Swift helpers, `spectra network`, the
  settings reset moving to core, the Litecoin recheck gate, the network-family
  reset and per-chain RPC endpoints; the per-chain RPC item removed from "Known
  open items"; metrics tables.
- Total: 8 .rs files changed (1 new), +314 −44 lines; data and config files:
  `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]` functions 461
  → 462.

## Beta Commit 118

- **`spectra send` split into `send broadcast` and `send assemble`**
  (`cli/src/cmd/tx.rs`, `cli/src/main.rs`)
  - `send broadcast` is the former `send`: it signs and broadcasts behind
    `--yes`.
  - `send assemble --chain --from --to --amount [--symbol --contract
    --decimals]` prints the `value_wei`, `to` and `data` an EVM send would sign,
    with no key, network or store; malformed addresses, a non-EVM chain and a
    partial token description are refused with exit code 3 or 2.
- **Every EVM mainnet can assemble a send, and ARB and OP assemble as token
  calls** (`core/src/send/ethereum.rs`, `core/src/send/mod.rs`,
  `prepare_evm_send_assembly`)
  - Supported is `Chain::is_evm` (before: `is_supported_evm_chain` named seven
    chains, so Base, Polygon, Linea and thirteen more EVM mainnets returned
    `UnsupportedChain` and the iOS send preview could not estimate a fee).
  - An asset is native when its symbol is the chain's `gas_token_symbol`
    (before: nine `(chain, symbol)` pairs that included `("Arbitrum", "ARB")`
    and `("Optimism", "OP")`, so those tokens assembled as a plain ETH
    transfer).
  - The native amount uses `Chain::native_decimals` instead of a literal 18;
    `native_evm_symbol_for_chain` reads the registry.
- **Which sends take the shared submit path is a registry fact**
  (`Chain::uses_generic_send_submit`,
  `SendSubmitPreflightPlan.uses_generic_submit`): replaces two chain-name lists
  on the iOS submit path; NEAR qualifies for its native asset only.
- **Fee priority is one per-chain map** (`AppSettings.fee_priority_by_chain`,
  `AppSettingUpdate::FeePriority { chain, value }`; `cli/src/cmd/settings.rs`)
  - Replaces the `bitcoin_fee_priority` and `dogecoin_fee_priority` fields; an
    absent entry means `normal`.
  - A chain the registry does not know is refused, and a value other than
    `economy`, `normal` or `priority` is stored as `normal`.
  - `spectra settings get|set fee-priority.<Chain>`; `settings list` prints only
    chains that have a preference.
- **Amount decimals come from the catalog for all 78 chains**
  (`supported_decimal_places` in `core/src/formatting.rs`; before: a 22-row
  `SUPPORTED_DECIMAL_CHAINS` table with a fallback of 6, so sixteen EVM
  mainnets, Zcash, Bitcoin Gold, Decred, Kaspa, Dash, Internet Computer,
  Bittensor and every testnet formatted at 6 places).
- **Every EVM chain shares Ethereum's display-decimals setting**
  (`native_asset_display_settings_key`; before: Arbitrum and Optimism only).
- **One token-identifier normalizer**
  (`normalize_token_identifier(contract_address, chain_name)` in
  `core/src/tokens.rs`): replaces `normalize_dashboard_contract_address` and
  `store::normalize_tracked_token_identifier`; a TON jetton address keeps its
  case (before: lowercased on one of the two paths). The Aptos and Sui helpers
  lost their export attribute.
- **A degraded-sync detail is classified in one call**
  (`diagnostics_classify_degraded_detail` in
  `core/src/diagnostics/degraded.rs`): replaces three exports and matches the
  template against the normalized detail, so a detail ending in " Last good
  sync: …" localizes on both callers' paths.
- **Endpoint diagnostics probe RPC endpoints from the catalog**
  (`core/src/app_core.rs`, `Chain::rpc_health_method`)
  - `diagnostics_checks` includes an RPC-role record on a chain with a health
    method, without needing a `probe_url`; mainnets with no checks 12 → 1
    (Monero).
  - `AppCoreDiagnosticsCheck.rpc_probe_method` carries `eth_chainId` (EVM),
    `status` (NEAR) or `chain_getHeader` (Polkadot); other endpoints are probed
    over HTTP.
- **Self-tests read the registry** (`core/src/diagnostics/self_tests.rs`)
  - The XRP suite is keyed `"XRP Ledger"` (before: `"XRP"`, which no lookup
    could reach).
  - The `chain_label`, `address_kind`, `derivation_chain` and `derivation_path`
    columns are gone; derivation coverage comes from the registry, so Monero
    runs its seed-derivation check (66 → 67 checks).
- **Status polls, diagnostics and history pages are written in one call each**
  (`core/src/service/state.rs`, `core/src/diagnostics/registry.rs`,
  `core/src/service/history_cursor.rs`)
  - `record_status_poll(transaction_id, outcome)` over `StatusPollOutcome`
    (`Confirmed`, `Pending`, `Unresolved`, `Failed`) replaces the
    success/failure pair; `clear_status_trackers` removed.
  - `diagnostics_record(chain_name, wallet_id, entry)` over
    `HistoryDiagnosticsEntry` replaces five per-shape writers.
  - `set_history_page(page, is_exhausted)` replaces two setters;
    `clear_all_history_records` removed.
- **One refresh-target planner for the non-EVM families**
  (`core_refresh_targets`, `RefreshTargetsRequest`, `RefreshWalletInput`,
  `RefreshWalletTarget` in `core/src/fetch/mod.rs`): replaces the Dogecoin and
  normalized planners and their records; the unread `index` field is gone. EVM
  keeps `core_evm_refresh_targets`.
- **One endpoint validator** (`endpoint_validation_error(field, raw)` over
  `EndpointField` in `core/src/tokens.rs`): replaces the Ethereum RPC, Monero
  backend and Bitcoin Esplora validators.
- **Removed: the backend table, static resources and two catalog flags**
  - `app_core.rs`'s 78-row `chain_backends()` table (every row `live`),
    `AppCoreChainBackend`, `AppCoreChainIntegrationState`,
    `app_core_chain_backends` and `live_chain_names`; `wallet_derived_state`
    treats every registry chain as sendable and receivable.
  - `core/src/resources.rs` and `core_static_resource_json`, whose tables were
    empty.
  - `supports_diagnostics` and `supports_endpoint_catalog` from `chains.toml`
    and `ChainEntry`.
- Measured: 2026-08-24, as `PLAN.md` records it: exported functions and
  methods 216 → 189; `cli-acceptance.sh` assertions 107 → 122; self-tests 67
  checks, 0 failed.
- Tests: 8 new, 5 removed
  - Endpoint diagnostics 3: a JSON-RPC node probed with a GET; a mainnet with
    endpoints left with nothing to check.
  - Degraded detail 2: a detail with its sync suffix missing its template; a
    partial success not reported as one.
  - Decimals 1: display decimals disagreeing with the catalog on any chain.
  - EVM assembly 2: an EVM mainnet refused for sending; a governance token
    assembled as the gas asset.
  - Removed: `all_chain_count`, the normalized refresh-target filter test, two
    static-resource tests, `aptos_package_id_splits_first_colon`.
- Docs: `FFI-BOUNDARY.md` "Traps" (an exported `async fn` block needs
  `async_runtime = "tokio"`; an export's callers are in Swift) and "Regenerating
  bindings" (the Xcode phase calls `scripts/bindgen-ios.sh`); `PLAN.md`
  "Behaviour changed on purpose" and Stage C.
- Total: 25 .rs files changed (1 deleted), +1084 −934 lines; data and config
  files: `core/data/chains.toml`, `scripts/cli-acceptance.sh`; dependencies
  unchanged; `#[test]` functions 458 → 461.

## Beta Commit 117

- **Staking works from the app** (`core/src/staking/service.rs`): the exported
  `StakingService` block declares `async_runtime = "tokio"` (before: every
  staking call from Swift failed with "there is no reactor running"; the CLI and
  Rust tests run their own runtime and passed).
- **`ChainIdentity` carries `token_tracking_chain`** (`core/src/registry.rs`):
  the `CoreTokenTrackingChain` for each of the 18 chains that host tracked
  tokens.
- Measured: 2026-08-23, as `PLAN.md` records it: after the fix the iOS staking
  tab's Solana page loads 100 live validators.
- Tests: 1 new
  - Token tracking 1: two chains claiming one tracking chain, or the count
    drifting from 18.
  - `chain_id_mapping` became `every_chain_can_be_paged`, which walks all 78
    chains instead of two.
- Docs: `PLAN.md` "Behaviour changed on purpose" (staking runtime, send-preview
  in-flight guard, diagnostics reset, owned-address lists, "Load more" on every
  chain).
- Total: 3 .rs files changed, +53 −10 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions 457 → 458.

## Beta Commit 116

- **Monero can be imported from a seed phrase** (`default_path_from_catalog` in
  `core/src/app_core.rs`, `Chain::uses_derivation_path`): a registry chain with
  an empty catalog path list answers `Ok("")` instead of an error, so `spectra
  wallet import --chain Monero` derives an address (before: "Missing default
  derivation path for Monero." on both front ends); an unknown chain still
  errors.
- **Staking support is a registry fact** (`Chain::supports_staking`,
  `StakingService::staking_chain` in `core/src/staking/service.rs`)
  - `fetch_validators` and `fetch_positions` route through one gate instead of
    matching seven `CHAIN_*` string constants.
  - Mainnets only: a testnet is refused rather than routed to a mainnet client.
  - `spectra staking validators --chain Bitcoin` reports that Bitcoin has no
    protocol-native staking (before: "no endpoints registered").
- **The send-preview shape is a registry fact** (`Chain::simple_preview_chain`;
  `core/src/service/send.rs`, `core/src/send/preview_decode.rs`):
  `fetch_simple_chain_send_preview_typed` derives it from the chain id, and
  `SimpleChain` no longer crosses the FFI.
- **Watch-only and staking flags reach the front ends**
  (`ChainIdentity.supports_watch_only_import`, `supports_staking`;
  `cli/src/cmd/chain.rs`, `cli/src/cmd/wallet.rs`)
  - `ChainIdentity.mainnet_counterpart` replaces `seed_derivation_path_key`, the
    same value.
  - `spectra chains --json` carries `watchOnlyImport` and `staking`; `spectra
    wallet watch` refuses a chain that cannot be watched without its keys.
- Measured: 2026-08-21, as `PLAN.md` records it: chain-name literals in the
  Swift root 743 → 530.
- Tests: 7 new
  - Monero derivation 1: a chain with no catalog path refused, or an unknown
    chain accepted.
  - Watch-only 1: a chain other than Monero excluded.
  - Preview shapes 2: a routing kind with no preview shape; a shape no chain
    routes to.
  - Staking 3: the flag and the dispatch disagreeing; a testnet routed to a
    mainnet client; a non-staking chain routed.
- Docs: `PLAN.md` "Behaviour changed on purpose" (EVM destination-risk warning,
  Litecoin recheck, Monero import, watch-only rows) and Stage C slices.
- Total: 10 .rs files changed, +372 −49 lines; data and config files:
  `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]` functions 450
  → 457.

## Beta Commit 115

- **Private-key import is offered exactly where a key derives an address**
  (`Chain::derives_from_private_key`; `core/src/derivation/import.rs`,
  `core/src/receive.rs`)
  - 28 mainnets; added Polygon, Base, Linea, Scroll, Blast and Mantle; removed
    Bitcoin SV, Tron, Solana, Cardano, Stellar, XRP Ledger, Sui, Aptos,
    TON, Internet Computer, NEAR, Polkadot, Zcash, Bitcoin Gold, Kaspa, Dash and
    Bittensor, each of which was offered and then failed to derive.
  - The import gate and `core_supported_private_key_chain_names` read the flag
    in place of the deleted `PRIVATE_KEY_SUPPORTED_CHAINS`, and the picker
    offers mainnets only; `chain_supports_private_key_import` deleted.
- **Settings are core state** (`AppSettings`, `StateCommand::SetAppSetting {
  update }`; new `cli/src/cmd/settings.rs`)
  - Eighteen fields of the former `PersistedAppSettings` blob, one command
    variant per field.
  - The reducer trims strings and bounds numbers: Bitcoin stop gap 1–200,
    refresh interval 5–60 minutes, and the two alert thresholds.
  - `spectra settings list|get|set`.
- **The CLI imports and exports private keys** (`spectra wallet import
  --private-key-file`; `core/src/store/wallet_secrets.rs`)
  - The key is sealed in a private-key envelope with the seed envelope's
    AES-GCM, KDF, salt and verifier.
  - A chain without private-key derivation is refused before anything is
    written; `wallet export` returns a private key; deleting a wallet removes
    it.
- **Refresh scheduling is in core** (new `core/src/service/maintenance.rs`,
  `core/src/fetch/refresh/policy.rs`):
  `WalletService::maintenance_plan(DeviceConditions)` over an in-memory
  `RefreshClock` replaces five exports; `core_chain_refresh_plans` (no caller)
  deleted.
- **Send preflight and routing derive from core state**
  (`core/src/service/maintenance.rs`)
  - `send_submit_preflight(wallet_id, holding_key, destination, amount)`
    resolves the wallet, holding, balance, EVM membership and token support
    itself; `holding_key` is `"<chain>|<symbol>"`.
  - `send_asset_routing(wallet_id, holding_key)` gives the preview the same
    route; `plan_send_preview_routing` deleted.
  - Solana routes SOL always and a token only when its mint is tracked; NEAR
    routes tracked tokens and not NEAR itself.
  - `evm_recipient_preflight` makes the contract-code probes; the high-risk-send
    reasons read the address book and history from core.
- **Transaction derived state reads core's store** (new
  `core/src/service/history_derived.rs`): `normalize_history`,
  `earliest_transaction_dates` and `active_wallet_transaction_ids` are
  `WalletService` methods; `core_normalized_history_signature` and six request
  records removed.
- **History store methods use the bound database**
  (`core/src/service/history_cursor.rs`)
  - Twelve methods read the path bound by `open_state` instead of taking
    `db_path`; an unopened store errors.
  - `history_cursor` returns the whole position; `reset_history(scope)` replaces
    four resets.
  - The status-poll config is a default and core reads the clock itself.
- **Export surface 291 → 216**
  - 33 `derive_<evm chain>` functions → `derive_evm`; `derive_for_chain_name`
    matches on `Chain`; the self-tests call the dispatcher.
  - Ten per-chain lookups (`core_address_slot`, `core_is_evm_chain`,
    `core_send_execution_shape`, `core_pending_status_poll`,
    `core_network_choices`, …) became columns of `core_chain_identities`.
  - `fetch_token_balances` absorbs `fetch_evm_token_balances_batch_typed`.
  - Endpoint catalog 20 → 10 exports: `app_core_chain_endpoints()` returns every
    chain's endpoints, and role names replace `core_endpoint_role_mask`.
  - Diagnostics: `diagnostics_record_*` write one row, `diagnostics_run_summary`
    and `diagnostics_forget_wallet` added, the read-all/replace-all pairs gone.
- Measured: 2026-08-21, as `PLAN.md` records it: exported functions and
  methods 216; Swift root vs `views/` 16,337 vs 10,785 lines; chain-name
  literals in the Swift root 743.
- Tests: 19 new, 1 removed
  - Private-key import 3: the flag and dispatcher disagreeing; the picker
    offering a chain the gate refuses; testnets offered.
  - Refresh policy 3: a stamped clock not waiting; heavy work on a flat battery;
    history replanned within its interval.
  - Send routing 6: a routing kind outside the known set; the Solana and NEAR
    rules; an unknown wallet accepted; a holding resolved by the wrong key.
  - History store 2: records landing in another database; derived views reading
    Swift reference time.
  - Settings 2: a field lost on round trip; an out-of-range value stored.
  - Private-key sealing 2: the raw key unsealed or left after deletion.
  - Diagnostics 1: the run summary miscounting.
  - Removed: `private_key_import_support_table`.
- Docs: `PLAN.md` "Behaviour changed on purpose" (private-key chains, the
  biometric gate on twelve chains, untracked NEAR tokens, settings, the
  launch-time transaction prune) and Stage C (target ~60 exports, slices).
- Total: 34 .rs files changed (3 new), +2721 −1146 lines; data and config
  files: `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]`
  functions 432 → 450.

## Beta Commit 114

- **`registry::Chain` crosses the FFI** (`uniffi::Enum`;
  `core_chain_identities()` returns chain, id, name, `is_testnet` and
  `diagnostics_shape` for every catalog row): `app_core_app_chain_descriptors`
  and `AppCoreAppChainDescriptor` removed.
- **One private-key derivation export**
  (`core_derive_from_private_key(chain_name, …)` in
  `core/src/derivation/dispatch.rs`): the EVM family plus Bitcoin, Bitcoin Cash,
  Litecoin, Dogecoin and Decred, dispatched through `mainnet_counterpart`; the
  six per-chain exports became internal.
- **Send execution shape on the registry** (`Chain::send_execution_shape`,
  `SendExecutionShape`, `SendFeeField`, `core_send_execution_shape`): for twelve
  chains, the fee-shortfall display decimals, whether a private-key-only wallet
  can sign, how the fee enters the request, and the default fee.
- **Every send-preview record names its fee `estimatedNetworkFee`**
  (`core/src/send/preview_types.rs`, `core/src/send/preview_decode.rs`; before:
  fifteen chain-specific spellings such as `estimatedNetworkFeeEth`).
- Measured: 2026-08-19, as `PLAN.md` records it: Swift 27,216 non-generated
  lines; Swift enums restating the chain list 4 → 0; chain-name literals in
  the Swift root 973.
- Tests: 3 new
  - Private-key derivation 1: coverage drifting from the EVM family and five
    UTXO chains.
  - Send execution shape 2: a shape differing from the call sites it replaced; a
    testnet differing from its mainnet.
- Docs: `PLAN.md` "Behaviour changed on purpose" (one Swift chain type,
  diagnostics on every mainnet, the private-key dispatcher, endpoints for
  sixteen more mainnets) and known open items.
- Total: 13 .rs files changed, +334 −92 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions 429 → 432.

## Beta Commit 113

- **Network-mode enums removed** (`CoreBitcoinNetworkMode`,
  `CoreDogecoinNetworkMode` in `core/src/store/wallet_domain.rs`)
  - `AddressValidationRequest.network_mode`, which nothing read, is gone; the
    `kind` string decides the network.
  - A wallet's network is a registry chain id
    (`CoreImportedWallet.network_chain_id`, `WalletSummary.network_mode`, the
    import's `network_chain_by_family`), as is the network
    `BitcoinSendParams`, `app_core_bitcoin_esplora_base_urls` and
    `app_core_bitcoin_wallet_store_default_base_urls` take (before:
    `mainnet`, `testnet`, `testnet4` or `signet`).
- **More chain facts on the registry** (`core/src/registry.rs`)
  - `Chain::bitcoin_network` replaces `bitcoin_network_for_mode`;
    `Chain::pending_status_poll` (`PendingStatusPoll`) added.
  - New exports `core_address_validation_kind`, `core_pending_status_poll`,
    `core_chain_display_name`, `core_network_choices`; they replace
    `ImportNetworks`' per-family matches and a 20-row chain-to-kind table in the
    send-flow scanner.
- **Dashboard rows are computed in core**
  (`WalletService::dashboard_asset_groups` in `core/src/service/state.rs`)
  - One row per (chain, asset), summed across wallets and ordered by value, with
    pinned symbols first in pin order; a live price beats the stored one and a
    testnet row has no value.
  - Only live prices cross; `formatting_dashboard_asset_grouping_key` export
    removed.
- Measured: 2026-08-16, as `PLAN.md` records it: Swift root vs `views/` 16,934
  vs 11,128 lines; 1,124 chain-name literals in the Swift root; 292 exported
  functions and methods.
- Tests: 4 new
  - Dashboard rows 4: rows merged across chains; a stored price beating a live
    one; a testnet row valued; pinned rows out of order.
- Docs: `PLAN.md` "Behaviour changed on purpose" (`network_mode` removed), Stage
  3 measurements, and the open question on per-chain dashboard rows.
- Total: 24 .rs files changed, +1164 −563 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions 425 → 429.

## Beta Commit 112

- **`service.rs` split by owner into `core/src/service/`** (`mod.rs`,
  `state.rs`, `network.rs`, `send.rs`, `helpers.rs`, `types.rs`,
  `standalone.rs`, and `send_params.rs` from `service_send_params.rs`); the
  export surface is unchanged.
- **Module moves**: `derivation/validation.rs` → `validation/address.rs`,
  `validation.rs` → `validation/mod.rs`, `fetch/refresh.rs` →
  `fetch/refresh/policy.rs`, `fetch/refresh_engine.rs` →
  `fetch/refresh/engine.rs`; the `addressing` and `utxo_hd` aliases removed.
- **The `Chain` enum is an index into `chains.toml`** (`Chain::entry()`):
  `str_id`, `chain_display_name`, `coin_symbol` and `from_str_id` read the
  catalog instead of 78-arm matches.
- **Network selection is a chain id** (`AppSettings.network_chain_by_family`,
  `StateCommand::SelectNetworkChain`): replaces the `NetworkModes` record and
  three settings fields; choosing the mainnet clears the entry;
  `wallet_derived_state` lost its network-mode argument.
- **No testnet balance is priced** (`core_unpriced_chain_names(settings)`): the
  rule is `!selected_network.is_testnet()` (before: only Bitcoin and Ethereum
  testnets were excluded, in two places, so a Dogecoin testnet balance was
  quoted at DOGE prices).
- **Every EVM chain gates non-native sends on token support**
  (`Chain::send_rule`; before: Ethereum, BNB Chain and Avalanche only); Ethereum
  Classic and Hyperliquid stay native-only.
- **The dashboard rebuild decision is removed** (its export,
  `DashboardRebuildDecisionRequest` and planner); the dashboard rebuilds on
  every price change.
- **Operational log in core** (`WalletService::append_chain_operational_event`):
  core stamps the id and time, keeps the newest 200 per chain, writes through,
  and loads it in `open_state`.
- **Price alerts and built-in tokens are evaluated over core state**
  (`WalletService::evaluate_price_alerts(prices)`,
  `built_in_token_preferences()`): only live prices cross; a built-in token's id
  is `builtin:<chain>:<contract>`; the price-alert KV store methods and
  `CorePersistedPriceAlertStore` removed.
- **Owned addresses live in core and core computes the keypool baseline**
  (`chain_keypool_baseline`, `app_core::utxo_discovery_index`)
  - `keypool_state`, `reserve_receive_index` and `reserve_change_index` take no
    baseline argument.
  - `delete_wallet_relational_data` also clears the in-memory keypool rows.
- **Planner pairs merged, dead planners deleted** (`core/src/store/mod.rs`,
  `core/src/send/transfer.rs`)
  - Sixteen `plan_x`/`core_x` pairs became one exported function each.
  - `plan_store_derived_state`, `plan_transfer_availability`, `can_send_holding`
    and their request records deleted (`send/transfer.rs` 286 → 57 lines).
  - `CoreTokenTrackingChain::chain_name` is the one chain mapping; its copies in
    the token merge and dashboard planners are gone.
- **Ten dead exports** deleted or un-exported: three `WalletService` methods
  including the address-book KV pair, `http_probe`, `http_get` and five
  `derive_<chain>` functions; `CorePersistedAddressBookStore` removed.
- **`wallet_db` test databases are keyed on process, thread and a counter**
  (`core/src/store/wallet_db.rs`; before: on `subsec_nanos()`, so two parallel
  tests could share a file).
- Measured: 2026-08-16, as `PLAN.md` records it: `core_plan_*` exports 8 → 0;
  290 exported functions counted from the Rust side (552 of the earlier 765
  were generated converter pairs); largest core file 4,781 → 2,501 lines; 69
  acceptance assertions.
- Tests: 9 new, 5 removed
  - Operational log 3: events lost on reopen; a log out of order or unbounded;
    clearing one chain clearing others.
  - Token preferences 3: duplicate built-in ids; catalog chain names that do not
    resolve; a merge overriding the user's choice.
  - Owned addresses 2: a recorded address not raising the baseline; owned
    addresses lost on reopen.
  - Network selection 1: choosing mainnet storing an entry.
  - Removed: two index-planner tests in `send/transfer.rs`,
    `plans_store_derived_state_with_stable_grouping`, and the two KV-store round
    trips in `persistence_models.rs`.
- Docs: `PLAN.md` Rule 0 (new), "Behaviour changed on purpose" (network mode,
  testnet pricing, dashboard rebuild, EVM send gating), Stage 2 complete, Stage
  C1 done, progress table.
- Total: 40 .rs files changed (8 new, 1 deleted), +5809 −6087 lines; data and
  config files: none; dependencies unchanged; `#[test]` functions 421 → 425.

## Beta Commit 111

- **CLI: `rescan`, `pool` and `alert`** (new `cli/src/cmd/rescan.rs`,
  `address_pool.rs`, `alert.rs`)
  - `rescan` derives the funds-finder candidates (55 addresses) from a seed read
    from a file or environment variable and checks their balances; `--no-check`
    lists them only.
  - `pool` shows and reserves a wallet's receive and change indices through
    core's keypool.
  - `alert` lists, adds and removes price alerts and reports which fire against
    live prices.
- **Price alerts are core state** (`CoreAppState.price_alerts`,
  `StateCommand::SetPriceAlerts` in `core/src/store/state.rs`; `wallet_db`): an
  alert with a non-positive target or empty holding key is dropped; a change
  emits `priceAlertsChanged`.
- **Comment banners and module docs shortened** (core and the CLI); no
  behaviour change.
- Tests: 3 new
  - Price alerts 2: an alert lost on reopen; an alert that cannot fire stored.
  - Resident state 1: a `CoreAppState` collection not surviving save and reload.
- Docs: `PLAN.md` Stage C ("The CLI gap is closed").
- Total: 64 .rs files changed (3 new), +935 −751 lines; data and config files:
  `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]` functions 418
  → 421.

## Beta Commit 110

- **`ARCHITECTURE.md` records decisions only**: "Why the workspace is small",
  "Design principles", "What this is NOT"; the file-level module map, FFI
  surface, iOS and Android notes, build commands and "Known divergence" are
  removed.
- **`FFI-BOUNDARY.md` rewritten** as "What crosses, and how", "Traps",
  "Regenerating bindings" and "Common errors".
- **`docs/SUMMARY.md` (the Swift file map) deleted**; `PLAN.md` and `README.md`
  describe `ARCHITECTURE.md` as the decisions record.
- Docs: `docs/ARCHITECTURE.md`, `docs/FFI-BOUNDARY.md`, `docs/SUMMARY.md`
  (deleted), `PLAN.md` header, `README.md` "Architecture".
- Total: No code changed.

## Beta Commit 109

- **CLI: `diagnostics`, `refresh`, `staking` and `token`** (new
  `cli/src/cmd/diagnostics.rs`, `refresh.rs`, `staking.rs`, `token.rs`)
  - `diagnostics self-test|show`, `staking validators|positions`, `token
    catalog|list|track|untrack`.
  - `refresh` sweeps balances through `BalanceRefreshEngine` with a CLI
    observer.
- **One derivation export** (`core_derive_for_chain` in
  `core/src/derivation/dispatch.rs`): replaces fifty per-chain `derive_<chain>`
  exports, forty of which keep their bodies without the attribute.
- **Diagnostics surface narrowed** (`core/src/diagnostics/`)
  - `core_diagnostics_json(chain_name)` builds the document from core's own
    registry; the shape a chain reports is `Chain::diagnostics_shape` (before:
    five builders fed by the caller).
  - The per-chain registries macro (120 generated get/set/remove/all/replace
    exports) became keyed registries with two operations each, ten exports.
  - Ten identical `<Chain>HistoryDiagnostics` records →
    `SimpleHistoryDiagnostics`; `EndpointHealthRow` and `EvmEndpointHealthRow` →
    one record (bundle JSON unchanged); an unknown chain reads empty.
- **`BalanceRefreshEngine::refresh_now`** awaits one cycle (`trigger_immediate`
  only spawns it).
- **Core logs to stderr at `warn`** unless `RUST_LOG` says otherwise
  (`core/src/service.rs`; before: stdout at `debug` in debug builds and `info`
  in release, interleaved with `--json` output).
- **Self-test fixtures are core-derived addresses**
  (`core/src/diagnostics/self_tests.rs`): seven address-validation checks
  (Bitcoin, Bitcoin Cash, Litecoin, Monero, Polkadot, Stellar, Internet
  Computer) failed on hand-typed fixtures with invalid checksums.
- **Tracked tokens persist** (`token_preferences` meta row in
  `core/src/store/wallet_db.rs`; before: `app_state_save` never wrote them and
  every launch loaded an empty list).
- **Internet Computer's registry display name is "Internet Computer"**
  (`core/src/registry.rs`; before: "ICP", disagreeing with `chains.toml`);
  `coin_symbol` stays "ICP" and `from_display_name` loses its special case.
- **Sixteen exports with no Swift caller**: ten deleted, six un-exported.
- Measured: 2026-08-14, as `PLAN.md` records it: core 59,564 lines and 234
  exported functions at the start of Stage C; generated exports 950 → 765;
  source-level exports 231 → 198; self-tests 59/66 → 66/66.
- Tests: 12 new
  - Derivation 1: a registry chain that does not derive through the one export.
  - Diagnostics 5: a chain with no document; shapes not covering their families;
    an unknown chain trapping; a failing self-test; a suite that checks nothing.
  - Registry 2: display names disagreeing with the catalog; a catalog name that
    does not resolve.
  - Token tracking 4: a tracked token or its decimals clamp lost on reopen; a
    tracking chain not round-tripping through the registry.
- Docs: `PLAN.md` Stage C (new: C1 skeleton, C2 surface, slices), the
  diagnostics collapse notes, the ICP open item closed.
- Total: 50 .rs files changed (4 new), +1541 −352 lines; data and config
  files: `scripts/cli-acceptance.sh`; dependencies unchanged; `#[test]`
  functions 406 → 418.

## Beta Commit 108

- **`spectra` is a clap subcommand CLI instead of a REPL** (`cli/src/main.rs`;
  new `cli/src/cmd/{address,chain,market,tx,wallet}.rs`, `ctx.rs`, `error.rs`,
  `out.rs`)
  - Commands `wallet`, `address`, `chains`, `balance`, `history`, `txs`, `send`,
    `price`, `portfolio`, `currency`.
  - `--json` on every command and `--data-dir` for a scratch store; exit code 3
    when core refuses, 2 for a usage error, 1 for a failure.
  - Delete, send and printing a seed take `--yes`; secrets come from a file, the
    environment or a prompt, never an argument.
  - One chain resolver accepts a display name, registry id, coin name or symbol;
    chain colours come from `chains.toml`.
  - Imports go through `WalletService::import_wallets` and mutations through
    `StateCommand` (before: the CLI built `WalletSummary` values and saved state
    itself).
  - `scripts/cli-acceptance.sh`: 38 assertions on exit codes and JSON against a
    scratch directory, without network.
- **Import addresses are validated for every chain** (`validated_addresses`,
  `validated_watch_only_entries` in `core/src/derivation/import.rs`)
  - Every supplied address and watch-only entry is checked with the chain's
    `address_validation_kind` and stored normalised (before: 21 chains kept the
    typed value unvalidated, and watch-only entries were not validated at all).
  - Refused addresses are returned in `WalletImportOutcome.rejected_addresses`;
    valid entries survive a bad one; an import left with nothing returns
    `InvalidInput` naming what was refused.
  - A Bitcoin watch xpub must start `xpub`, `ypub`, `zpub` or testnet `tpub`,
    `upub`, `vpub`.
  - Typed watch addresses are judged on the selected network and derived
    addresses on mainnet.
- **`wallet_derived_state()` returns resolved coins** (`core/src/service.rs`,
  `core/src/send/transfer.rs`): portfolio grouping, totals and send/receive
  availability from core's own wallets, replacing the index-returning planners
  on the app path; a testnet coin is not quoted and groups separately;
  `can_send_coin` reads tracked tokens from `CoreAppState`.
- **Seed sealing is core policy** (new `core/src/store/wallet_secrets.rs`): the
  KDF, the three blobs (envelope, salt, verifier), their keys and encoding moved
  from the CLI; a reseal replaces every blob, a corrupt blob is not reported as
  a wrong password, an empty password is refused.
- **Every EVM mainnet has a chain context** (`core_evm_chain_context` in
  `core/src/send/flow.rs`), including Sei, Celo, Cronos, opBNB, zkSync Era,
  Sonic, Berachain, Unichain, Ink and X Layer.
- **Endpoint lookup per network and per chain** (`core/src/app_core.rs`): a
  testnet resolves its own RPC endpoints, and a mainnet's RPC list no longer
  includes its testnets'; the settings index keeps a chain and its testnets
  together.
- Measured: 2026-08-14, as `PLAN.md` records it: `core_plan_*` exports 10 → 8;
  Swift root vs `views/` 18,613 vs 10,969 lines; CLI commands drivable without
  a TTY 0 of 24 → all.
- Tests: 37 new
  - Derived state 5: portfolio sums, exclusion, testnet quoting and send gating
    going wrong.
  - Address resolution 1: a chain the Swift switch named no longer resolving.
  - Import validation 14: a malformed address stored; import and send
    normalisers disagreeing; testnet watch addresses refused on testnet or
    accepted on mainnet; one bad address discarding the rest.
  - Endpoint index 4: a testnet without its own RPC list; testnet endpoints in a
    mainnet list; settings groups split.
  - EVM context 5: an EVM chain without a context; Ethereum Classic on the wrong
    coin type.
  - Seed sealing 8: wrong-password and corrupt-blob reports, shared salts, stale
    blobs after reseal or delete.
- Docs: `PLAN.md` "Behaviour changed on purpose" (new), Stage 3 progress,
  progress table, known open items (testnet endpoints and `EVMChainContext`
  fixed).
- Total: 18 .rs files changed (10 new), +3589 −2766 lines; data and config
  files: `Cargo.lock`, `cli/Cargo.toml`, `scripts/cli-acceptance.sh`;
  dependencies: added clap 4, serde_json 1; removed base64, bip39, dialoguer,
  getrandom, pbkdf2, sha2 (`cli/Cargo.toml`); `#[test]` functions 369 → 406.

## Beta Commit 107

- **`WalletService` owns the resident app state** (`open_state`,
  `apply_state_command`, `StateTransition`; `core/src/service.rs`,
  `core/src/store/state.rs`)
  - `open_state(db)` loads `CoreAppState` from `wallet_db` and binds the path;
    reopening the same database keeps newer in-memory writes (before: a second
    `open_state` reverted them).
  - `apply_state_command` persists before returning; a command that changes
    nothing emits no events and writes nothing.
  - `AppSettings` holds the fiat currency (its three earlier fields were unused)
    and pinned dashboard symbols (upper-cased, de-duplicated, in pin order);
    `CoreAppState` holds token preferences, with display decimals clamped to the
    token's.
  - Fields added to stored structs default when absent, so older state still
    loads.
- **The address book is core state** (`StateCommand` add, rename, remove;
  `address_book` table)
  - Trimming, address normalisation, chain validity and the duplicate check are
    reducer rules; a refusal returns an `addressBookRejected` event with an
    `AddressBookRejection` (`emptyName`, `invalidAddress`, `duplicateAddress`).
  - Entries carry a core-assigned string id.
- **Transactions are a core-owned store** (`apply_transaction_command`,
  `transactions()` over `history_records`; `core/src/fetch/transactions.rs`)
  - A merge reads core's own records, so only the incoming page crosses, and
    writes only the records it changed.
  - The wire ↔ stored conversion, including Swift reference date → Unix time for
    `created_at`, is in core.
- **Wallets are core state** (`import_wallets`,
  `StateCommand::UpdateWalletIfPresent`; `CoreImportedWallet::to_summary`,
  `WalletSummary::to_imported_wallet`)
  - `import_wallets` plans, builds and stores the wallets and returns the
    Keychain instructions; it replaces `core_plan_wallet_import`.
  - A summary keeps only the derivation path and network mode its own chain
    uses; `CoreImportedWallet` remains as a view model.
  - Balance refresh updates only a wallet that still exists (before: a refresh
    landing after deletion re-created it).
  - A testnet resolves its mainnet's derivation path (before: an error, which
    iOS turned into a crash).
- **Keypool reservation is atomic in core** (`reserve_receive_index`,
  `reserve_change_index`, `wallet_keypool`): one write lock across
  read-modify-write, loaded by `open_state`; deleting a wallet's or chain's
  keypool clears the in-memory rows too.
- **Confirmation-poll tracking moved into `WalletService`**: five planners
  became intents over core-held trackers.
- **Per-chain send and merge rules on the registry** (`Chain::send_rule`,
  `Chain::transaction_merge_strategy`, `coin_symbol` for EVM native assets):
  `TransactionCommand::Merge` no longer takes `strategy` or
  `include_symbol_in_identity`; testnets follow their mainnet.
- **`core_plan_*` exports 42 → 10** (`core/src/store/mod.rs`): pure calculations
  dropped the prefix (`core_icon_identifier`, `core_ethereum_send_error_code`,
  `core_send_submit_preflight`, …); five planners with no caller and
  `core/src/send/utxo.rs`, an unused second UTXO selector, deleted.
- **CLI commands `currency`, `addressbook` (`ab`) and `txs`**
  (`cli/src/main.rs`) drive the same state through `WalletService`.
- Measured: 2026-08-14, as `PLAN.md` records it: Swift 30,879 non-generated
  lines, 64% of them outside `views/`; 42 `core_plan_*` exports; 0 Swift calls
  into `StateCommand`.
- Tests: 68 new, 8 removed
  - Resident state 5: a command not persisted; a no-op emitting events; currency
    codes not normalised.
  - Address book 8: empty names, invalid or duplicate addresses stored; order
    lost on restart.
  - Transactions 15: added/updated misreported; unchanged records rewritten;
    `created_at` left in Swift time; the wire conversion losing a field.
  - Wallet model and state 14: per-wallet path and network mode not
    round-tripping; a deleted wallet resurrected; a reopen reverting writes.
  - Status polling 5, import 2, keypool 4, pins 3, forward compatibility 2:
    polling schedule, imported wallets missing from state, an index handed out
    twice, pins unnormalised, older state failing to load.
  - Registry rules 10: merge strategy, EVM native symbols, send rules and
    testnet derivation paths diverging from what they replaced or from the
    mainnet.
  - Removed: five with `send/utxo.rs`, three in `fetch/mod.rs`.
- Docs: `PLAN.md` (new: target, domain vs view state, stages 0–4, progress
  metrics, rules for new work); `README.md` "Architecture" (new);
  `ARCHITECTURE.md` "Known divergence: Swift owns the domain state";
  `FFI-BOUNDARY.md` regenerating bindings, which shape a new export takes, "Do
  not add a `core_plan_*`"; `SUMMARY.md`.
- Total: 30 .rs files changed (1 deleted), +3743 −931 lines; data and config
  files: none; dependencies unchanged; `#[test]` functions 309 → 369.

## Beta Commit 106

- **Per-chain facts on `registry::Chain`** (`address_slot`,
  `address_validation_kind`, `supports_watch_only_import`; exports
  `core_address_slot`, `core_supports_watch_only_import`)
  - Send-address validation reads `Chain::address_validation_kind` (before:
    `send/flow.rs`'s `chain_kind` omitted 22 mainnets, so
    `is_valid_send_address` rejected every address on Base, Polygon, Zcash,
    Kaspa, Dash, Bittensor and the newer rollups).
  - `core_evm_chain_context_tag` recognises Base and Polygon mainnets (before:
    only their testnets).
- **Wide per-chain records became chain-keyed maps**
  (`core/src/derivation/import.rs`, `app_core.rs`, `diagnostics/export.rs`,
  `store/wallet_domain.rs`, `send/flow.rs`)
  - `WalletImportAddresses`, `WalletImportWatchOnlyEntries` and
    `CoreImportedWallet` addresses keyed by `Chain::address_slot`.
  - `CoreSeedDerivationPaths` (45 fields) keyed by `Chain::str_id`, testnets
    folding onto their mainnet.
  - `DiagnosticsBundlePayload`'s 21 per-chain JSON fields → one map.
  - `SendPreviewsInput` (18 optional previews) → a `SendPreview` tagged enum.
- **Native `SecretStore` backends** (new `core/src/store/secret_backends.rs`):
  `InMemorySecretStore` for tests and `FileSecretStore` for the CLI, writing
  owner-only files and tightening a loose root.
- **`wallet_db` persists the app state** (`app_state_save`/`app_state_load`,
  `wallet_upsert`/`wallet_load_all`): wallet order kept, removed wallets pruned,
  and deleting a wallet removes its history.
- **The CLI runs on core's store and secrets** (`cli/src/main.rs`): wallets are
  a `CoreAppState` in `wallet_db` under `~/.spectra`, secrets in
  `FileSecretStore` at `~/.spectra/secrets`; its own JSON wallet store and
  SQLite code are gone.
- **Tor fails closed and resolves DNS through the proxy**
  (`core/src/fetch/http.rs`, `core/src/tor.rs`)
  - A proxy that cannot be applied, or a proxied client that fails to build,
    yields a client routed to a dead loopback port (before: a direct client).
  - `socks5://` URLs are upgraded to `socks5h://`; the built-in Tor proxy is
    `socks5h://127.0.0.1:19050`.
- **History-cursor helpers moved into `core/src/service/history_cursor.rs`**
  (from `core/src/service.rs`).
- Tests: 36 new
  - Derivation paths 1: a catalog chain or testnet without a resolved path.
  - Import 7: EVM chains not sharing one address slot; Ethereum Classic's own
    slot; the xpub on a non-Bitcoin wallet; watch-only expansion and refusals; a
    testnet borrowing its mainnet's address.
  - Registry 4: validation kind disagreeing with `is_evm`; a kind
    `validate_address` does not know; slots shared outside the EVM family;
    watch-only on Monero or testnets.
  - Send flow 9: UTXO preview fallback and clamping; UTXO fields on account
    previews; EVM tags and send validation for the chains the old tables
    omitted.
  - Secret backends 7: store conformance, keys escaping the bucket, foreign
    files listed, loose permissions.
  - App state storage 6: round trip, order, pruning, upsert, history removed
    with the wallet.
  - Tor 2: a leaky `socks5://` URL kept.
- Docs: `ARCHITECTURE.md` module map (`app_core.rs`, `registry.rs`, derivation
  modules, `state.rs`, `wallet_db.rs`, `secret_backends.rs`), Swift test-target
  note, new "Known divergence: iOS bypasses the core state model";
  `FFI-BOUNDARY.md` "Secret material" (`SecretStore` backends) and file paths.
- Total: 18 .rs files changed (2 new), +2600 −1480 lines; data and config
  files: `Cargo.lock`, `cli/Cargo.toml`; dependencies: added base64 0.22;
  removed rusqlite, serde, serde_json (`cli/Cargo.toml`); `#[test]` functions
  273 → 309.

## Beta Commit 104

- **Send execution moved to its own module and scrubs secrets** (new
  `core/src/service/send_execution.rs`; `core/src/send/mod.rs`,
  `core/src/store/wallet_domain.rs`)
  - `execute_send` moved out of `service.rs`.
  - After `execute_send`, the request's seed phrase, private key, passphrase,
    HMAC key and salt prefix are zeroized
    (`SendExecutionRequest::zeroize_sensitive_fields`).
  - `SendExecutionResult.result_json` renamed `rebroadcast_payload` and
    documented as opaque.
- Tests: 1 new
  - Send request 1: a secret field left intact after scrubbing.
- Docs: `ARCHITECTURE.md` workspace layout (core holds the UniFFI exports;
  `ffi/` is a re-export shim); `FFI-BOUNDARY.md` "Send-result payloads" and
  "Secret material" (new).
- Total: 4 .rs files changed (1 new), +493 −403 lines; data and config files:
  none; dependencies unchanged; `#[test]` functions 272 → 273.

## Beta Commit 103

- **`make ios` and `make iosr` build the app through Xcode** (`Makefile`): the
  Rust-library-and-bindings targets are now `ios-artifacts` and
  `ios-artifacts-release`; `make help` is the default; `clean-generated` removes
  generated Swift and Kotlin bindings and JNI libraries.
- **The CLI offers every catalog derivation path** (`cli/src/main.rs`): each
  tagged entry, with `{account}` set to 0, is a choice (before: default and
  alternate); the CLI builds again after Beta Commit 102's catalog change.
- **`.gitignore` ignores the Kotlin bindings and JNI libraries**
  (`kotlin/app/src/main/kotlin/uniffi/`, `kotlin/app/src/main/jniLibs/`)
- Docs: `SUMMARY.md` views list (Liquid Glass notes → `iosUI.md`).
- Total: 1 .rs file changed, +21 −17 lines; data and config files:
  `.gitignore`, `Makefile`; dependencies unchanged; `#[test]` functions
  unchanged at 272.

## Beta Commit 102

- **Each chain lists its derivation paths with tags** (`core/data/chains.toml`;
  `ChainDerivationPathEntry` in `core/src/chains.rs`)
  - `derivation_path` is a list of `{ tag, path, is_default, note }` with
    `{account}` templates; `alt_derivation_path` removed.
  - Bitcoin lists native SegWit (default), legacy, nested SegWit, Taproot and
    two Electrum paths; Litecoin, Bitcoin Cash, Solana, XRP Ledger, Tron and
    Cardano list legacy alternatives; Monero's list is empty.
  - Cardano's catalog default is `m/1852'/1815'/{account}'/0/0` (before:
    `m/44'/1815'/0'/0/0`).
- **Default paths and path flavours come from the catalog**
  (`core/src/app_core.rs`)
  - The per-account path set reads `default_path_from_catalog_for_account` for
    every chain (before: a hand-written builder); Bitcoin SV follows the account
    (before: always account 0).
  - `resolved_flavor` returns the tag of the catalog entry the path matches
    (before: a hand-written match).
- **The CLI does not build** (`cli/src/main.rs` still reads the removed
  `alt_derivation_path`).
- Total: 4 .rs files changed, +131 −105 lines; data and config files:
  `core/data/chains.toml`; dependencies unchanged; `#[test]` functions
  unchanged at 272.

## Beta Commit 101

- **Catalog files reformatted, with descriptions** (`core/data/chains.toml`,
  `core/data/tokens.toml`, `core/src/chains.rs`, `core/src/tokens.rs`)
  - Single-space `key = value`; the header says rows follow `Chain::all()` and
    the string `id` is the key.
  - `color_name` renamed `color` on chains and tokens.
  - A one-line `comment` on every chain and token.
  - `slip44_coin_type` and `notable_details` removed from chains.
- Total: 3 .rs files changed, +31 −32 lines; data and config files:
  `core/data/chains.toml`, `core/data/tokens.toml`; dependencies unchanged;
  `#[test]` functions unchanged at 272.

## Beta Commit 100

- **Nested SegWit sends are signed as nested SegWit** (`sign_p2sh_p2wpkh` in
  `core/src/send/chains/bitcoin.rs`): a P2SH `from` address is checked against
  the key and spent with a redeem script and witness (before: signed through the
  P2PKH path).
- **A UTXO with a malformed txid is an error**
  (`core/src/send/chains/bitcoin.rs`; before: replaced by an all-zero txid).
- **Send parameter records moved to `core/src/service_send_params.rs`** (new):
  the typed per-chain send params out of `service.rs`.
- **EVM derivation exports come from a macro** (`evm_deriver!` in
  `core/src/derivation/chains/evm.rs`): the same 33 `derive_<chain>` exports and
  signatures.
- **`.gitignore` covers agent and editor files** (`AGENTS.md`, `CLAUDE.md`,
  `.cursor/`, …)
- Docs: `ARCHITECTURE.md` `service.rs` and `service_send_params.rs` entries,
  vertical-slicing and FFI principles; `FFI-BOUNDARY.md` UniFFI 0.29 → 0.31,
  exports live in their owning module (no `core/src/ffi.rs`), string chain ids,
  typed send params.
- Total: 5 .rs files changed (1 new), +1006 −1185 lines; data and config
  files: `.gitignore`; dependencies unchanged; `#[test]` functions unchanged
  at 272.

## Beta Commit 99

- **`rustfmt` over core and the CLI**: most of the 127 files change formatting
  only (the Monero word list becomes one word per line).
- **Shared derivation primitives and one dispatch** (new
  `core/src/derivation/primitives.rs`, `core/src/derivation/dispatch.rs`)
  - BIP-39 language resolution and seed, BIP-32 path parsing, substrate sr25519
    material and SS58 encode/decode moved out of the per-chain files.
  - `derive_for_chain_name` maps a chain name to its `derive_<chain>` function;
    signing in `service.rs` and the CLI call it.
  - The CLI builds again (it had called the removed `derive_for_chain` since
    Beta Commit 91).
- **UTXO-family address validation checks checksums**
  (`core/src/derivation/validation.rs`)
  - Legacy addresses must decode as base58check with an allowed version byte,
    SegWit addresses with an allowed HRP, and Bitcoin Cash CashAddr must pass
    its polymod (before: prefix checks).
  - Litecoin MWEB addresses validate through `parse_mweb_address`.
- Tests: 2 new
  - Address validation 2: an address with a mutated checksum accepted; a UTXO
    address judged by prefix instead of its decoded network.
- Total: 127 .rs files changed (2 new), +10651 −5074 lines; data and config
  files: none; dependencies unchanged; `#[test]` functions 270 → 272.

## Beta Commit 98

- **Send amounts reach raw units through the typed decimal string**
  (`core/src/send/mod.rs`, `core/src/send/preview_decode.rs`,
  `core/src/send/payload.rs`, `core/src/service.rs`)
  - `SendSubmitPreflightPlan.amount_str` carries the trimmed input, and
    `SendExecutionRequest.amount_str` is converted with
    `decimal_str_to_raw_units` (string arithmetic, extra places truncated);
    without it the f64 path remains.
  - The NEAR, Polkadot, Bittensor and token-transfer payload builders take
    raw-unit strings instead of f64 amounts.
- **EVM history uses an explorer only where one is configured**
  (`Chain::evm_explorer_api_base`): Ethereum Classic, Hyperliquid and their
  testnets return "does not support Etherscan history" (before: every EVM chain
  was queried at `api.etherscan.io`); `EvmChainConfig` deleted.
- **`service.rs` fetch helpers became free functions** (the `ChainClient` enum
  is deleted).
- Tests: 1 new
  - Decimal conversion 1: f64 rounding in raw-unit amounts.
- Total: 6 .rs files changed, +367 −431 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions 269 → 270.

## Beta Commit 97

- **Cardano reads from Koios** (`core/src/fetch/chains/cardano.rs`): balance,
  unspent UTXOs, history and tip through `/address_info`, `/address_utxos`,
  `/address_txs` and `/tip`, without an API key (before: Blockfrost with a
  `project_id` header).
- **Dogecoin reads from BlockCypher** (`core/src/fetch/chains/dogecoin.rs`):
  `/addrs/{address}/balance`, `/addrs/{address}?unspentOnly=true` and
  `/txs/{txid}` (before: a Blockbook v2 API).
- **A refresh triggered mid-cycle re-runs the cycle** (`pending_trigger` in
  `core/src/fetch/refresh_engine.rs`; before: the trigger was dropped and
  entries added during a cycle waited for the timer).
- Total: 3 .rs files changed, +279 −232 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions unchanged at 269.

## Beta Commit 96

- **Monero derives from 25-word Electrum seeds** (`decode_monero_electrum_seed`
  in `core/src/derivation/chains/monero.rs`): 25 words decode through the
  1,626-word English list with the CRC-32 checksum word and no PBKDF2; other
  lengths stay BIP-39.
- **Monero's `private_key_hex` is spend ‖ view** (64 bytes; before: the spend
  key only).
- **`bip39_wordlist(language)` export** (`core/src/service.rs`): a BIP-39 word
  list, newline-separated, English for unknown codes.
- Tests: 3 new
  - Monero Electrum 3: a known vector decoding wrong; a bad checksum word
    accepted; a 12-word BIP-39 phrase parsed as Electrum.
- Total: 3 .rs files changed, +361 −20 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions 266 → 269.

## Beta Commit 95

- **Funds-finder candidates** (new `core/src/derivation/funds_finder.rs`,
  `core_generate_funds_finder_candidates`): derives addresses for 14 chains over
  standard and legacy paths and several accounts — Bitcoin's four script types ×
  accounts 0–2, Ethereum accounts and indexes 0–2 plus three truncated legacy
  paths, Solana standard and legacy — and returns chain id, path, label and
  address; no network, and a failed candidate is skipped.
- **Unused tuple-returning `derive_from_seed_phrase` helpers removed** from
  the seven P2PKH chain files (`core/src/derivation/chains/`).
- Total: 12 .rs files changed (1 new), +309 −181 lines; data and config files:
  none; dependencies unchanged; `#[test]` functions unchanged at 266.

## Beta Commit 94

- **Staking validators are fetched for all seven chains**
  (`core/src/staking/chains/*.rs`, `core/src/staking/service.rs`)
  - `fetch_validators` queries each chain's endpoints (Solana `getVoteAccounts`,
    Sui `suix_getLatestSuiSystemState`, …) and returns an empty list without
    endpoints or on failure (before: `NotYetImplemented`).
  - Action builders return `StakingActionPreview`s with fixed fee estimates,
    unbonding periods and notes; `fetch_positions` returns an empty list.
  - ICP gains `icp_build_claim_maturity_tx`; the Cardano client takes the
    configured API key; an unknown chain returns `NotYetImplemented`.
- **The diagnostics bundle is a core record** (`DiagnosticsBundlePayload`,
  `DiagnosticsEnvironmentMetadata`, `diagnostics_bundle_to_json`,
  `diagnostics_bundle_from_json` in `core/src/diagnostics/`).
- **Logging through `tracing`** (`core/src/service.rs`,
  `core/src/fetch/refresh_engine.rs`): `WalletService` installs a
  `tracing-subscriber` once, at `debug` in debug builds and `info` in release
  unless `RUST_LOG` is set; `eprintln!` logs replaced.
- **Token `category` became `tags`** (`core/data/tokens.toml`,
  `core/src/tokens.rs`): `"stablecoin"` and `"meme"` → one-element lists,
  `"custom"` → `[]`.
- **The `history_store` tests use string chain ids**
  (`core/src/fetch/history_store.rs`); they did not compile after Beta Commit
  93.
- Total: 14 .rs files changed, +1030 −164 lines; data and config files:
  `Cargo.lock`, `core/Cargo.toml`, `core/data/tokens.toml`; dependencies:
  added tracing 0.1, tracing-subscriber 0.3 (`core/Cargo.toml`); `#[test]`
  functions unchanged at 266.

## Beta Commit 93

- **Chains are identified by a string id** (`Chain::str_id`, `from_str_id`,
  `endpoint_str_id` in `core/src/registry.rs`; exports
  `core_chain_str_id_for_name`, `core_endpoint_str_id`)
  - The `Chain` enum loses its `u32` discriminants; `id()` and `from_id`
    removed.
  - Endpoint ids are `"<id>"`, `"<id>:secondary"` and `"<id>:explorer"` (before:
    `id`, `id + 100`, `id + 200`).
  - `WalletService` methods, the history pagination store and `list_tokens` take
    string chain ids; `list_tokens("")` returns every token.
- **`chain_id` and `short_label` removed from the catalog**
  (`core/data/chains.toml`, `core/data/tokens.toml`, `core/src/chains.rs`)
  - `symbol` takes the former short label (Base `BASE`, Linea `LINEA`, opBNB
    `opBNB`, …); `gas_token_symbol` is unchanged.
  - `chain_by_id` → `chain_by_str_id`; ids `opBNB` → `opbnb` and
    `bitcoin-testnet4` → `bitcoin-testnet-4`.
- **The CLI resolves every registry chain by name** (before: ids 0–29 only).
- **Doc comments on the derivation chain files**
  (`core/src/derivation/chains/*.rs`)
- Tests: 1 new
  - Registry 1: the chain count drifting from 78.
- Total: 37 .rs files changed, +902 −658 lines; data and config files:
  `core/data/chains.toml`, `core/data/tokens.toml`; dependencies unchanged;
  `#[test]` functions 265 → 266.

## Beta Commit 92

- **Duplicate address validators removed from the derivation chain files**
  (`core/src/derivation/chains/*.rs`): seventeen `validate_<chain>_address`
  functions and the Aptos, ICP and Sui watch-only encoders
  (`pubkey_to_aptos_address`, `pubkey_der_to_icp_address`,
  `pubkey_to_sui_address`), copies of what `core/src/derivation/validation.rs`
  holds.
- Total: 17 .rs files changed, +0 −189 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions unchanged at 265.

## Beta Commit 91

- **Derivation exports live in the chain files**
  (`core/src/derivation/chains/*.rs`; `core/src/derivation/api/` deleted,
  `api/types.rs` → `derivation/types.rs`)
  - The `derive_<chain>`, testnet and `_from_private_key` exports moved from
    `api/` into each chain's file; `chain_dispatch::derive_for_chain` deleted.
  - Signing in `service.rs` and the self-tests match the chain name to its
    `derive_<chain>` function: passphrase on every chain but Monero, HMAC key on
    Solana, Stellar and Polkadot, script type on Bitcoin and Litecoin.
  - The CLI still calls the removed `derive_for_chain` and does not build until
    Beta Commit 99.
- **`StakingService` UniFFI object** (new `core/src/staking/service.rs`): holds
  the Solana, Cardano, Sui, Aptos, NEAR, Polkadot and ICP clients and dispatches
  `fetch_validators`, `fetch_positions` and the per-chain transaction builders
  to them.
- Docs: `ARCHITECTURE.md` moved to `docs/ARCHITECTURE.md`; `docs/SUMMARY.md`
  added (a file-by-file map of the Swift layer).
- Total: 55 .rs files changed (1 new, 16 deleted), +2098 −1539 lines; data and
  config files: none; dependencies unchanged; `#[test]` functions unchanged at
  265.

## Beta Commit 90

- **Chain catalog in `core/data/chains.toml`** (new `core/src/chains.rs`,
  `list_all_chains`, `ChainEntry`, `core_resolve_chain_id`)
  - 78 rows: name, symbols, category, EVM flag, colour and asset name, token
    standard, wiki text, default and alternate derivation paths, native
    CoinGecko id and decimals.
  - Replaces `app_chain_descriptors()` in `app_core.rs` and the
    `ChainWikiEntries.json` alias lookup; `Chain::coin_name`, `native_decimals`
    and `coin_gecko_id` read it.
  - Default derivation paths come from the catalog
    (`default_path_from_catalog`).
- **Named per-chain derivation exports replace the request engine** (new
  `core/src/derivation/api/`; `derivation/engine.rs` and `derivation/presets.rs`
  deleted)
  - `derive_bitcoin`, `derive_ethereum`, `derive_solana`, … take a seed phrase,
    path, passphrase and `want_*` flags and return a `DerivationResult`
    (address, public key, private key, account/branch/index);
    `BitcoinScriptType` selects the UTXO address encoding.
  - Removed exports: `derivation_derive`, `derivation_derive_from_private_key`,
    `derivation_build_material`, `derivation_build_material_from_private_key`,
    `derivation_derive_all_addresses`, `app_core_chain_presets`,
    `app_core_request_compilation_presets`, `core_compile_script_type`.
  - Removed data: `DerivationPresets.json`,
    `DerivationRequestCompilationPresets.json`, `derivation_presets.toml`.
  - Signing and the CLI derive through `chain_dispatch::derive_for_chain`, which
    honours only passphrase, HMAC key and script type; iteration count,
    wordlist, salt prefix and algorithm overrides are no longer applied.
  - Bitcoin Cash, Bitcoin SV, Bitcoin Gold, Litecoin, Dogecoin, Dash and Zcash
    derive in `api/bitcoin.rs`; their chain files keep address validation only.
- **Setup-form validators** (new `core/src/validation.rs`):
  `core_validate_seed_phrase_word_count` (below 12 refused, non-standard lengths
  advised) and `core_validate_wallet_password` (empty allowed, otherwise at
  least 4 characters and matching).
- **`tokens.toml` regrouped by chain** (`core/data/tokens.toml`; before: by
  symbol)
- Tests: 0 new, 6 removed
  - Removed: two chain-inference tests, the override-beats-inference test,
    request parsing, NEAR ignoring the path, and the presets catalog load.
- Docs: `core/data/DERIVATION_WORKFLOW.md` and
  `core/src/derivation/chains/REFACTOR_NOTES.md` deleted.
- Total: 56 .rs files changed (19 new, 2 deleted), +2848 −6673 lines; data and
  config files: `core/data/DerivationPresets.json`,
  `core/data/DerivationRequestCompilationPresets.json`,
  `core/data/chains.toml`, `core/data/derivation_presets.toml`,
  `core/data/tokens.toml`; dependencies unchanged; `#[test]` functions 271 →
  265.

## Beta Commit 89

- **Sei icon gets a white full-disc background** (`resources/coinicon/sei.svg`)
- Total: no .rs files changed; data and config files:
  `resources/coinicon/sei.svg`; dependencies unchanged; `#[test]` functions
  unchanged at 271.

## Beta Commit 88

- **Icons split into `resources/coinicon/` and `resources/fiaticon/`** (from
  `resources/icons/` and `resources/icons/fiat/`)
  - The 73 coin, token and chain icons move to `resources/coinicon/`; the five
    fiat icons (CNY, EUR, GBP, RUB, USD) to `resources/fiaticon/`.
  - `bitcoingold.svg`, `ink.svg` and `sonic.svg` gain a white full-disc
    background circle.
- Total: no .rs files changed; data and config files: `resources/coinicon/`
  (73), `resources/fiaticon/` (5); dependencies unchanged; `#[test]` functions
  unchanged at 271.

## Beta Commit 84

- **UniFFI 0.29 → 0.31** (`core/Cargo.toml`, `ffi/Cargo.toml`,
  `tools/uniffi-bindgen/Cargo.toml`)
- **CLI chain list comes from `app_core_chain_presets`** (`cli/src/main.rs`
  `supported_chains`): each preset's default derivation path (before:
  `spectra_core::platform::catalog::core_bootstrap`, which Beta Commit 73
  deleted).
- **The `render_evm_overrides_fragment` test builds again**
  (`core/src/send/ethereum.rs`): its `EvmSendOverridesInput` literal fills the
  fields added in Beta Commit 80 with `..Default::default()`.
- Total: 2 .rs files changed, +10 −9 lines; data and config files:
  `Cargo.lock`, `core/Cargo.toml`, `ffi/Cargo.toml`,
  `tools/uniffi-bindgen/Cargo.toml`; dependencies: uniffi 0.29 → 0.31
  (`core/Cargo.toml`, `ffi/Cargo.toml`, `tools/uniffi-bindgen/Cargo.toml`);
  `#[test]` functions unchanged at 271.

## Beta Commit 82

- **`make iosr` and `make androidr` build the mobile libraries in release mode,
  then regenerate bindings** (`Makefile`; `scripts/build-ios.sh --release`,
  `scripts/build-android.sh --release`); `make ios` and `make android` stay
  debug.
- Total: 1 .rs file changed, +2 −2 lines; data and config files: `Makefile`;
  dependencies unchanged; `#[test]` functions unchanged at 271.

## Beta Commit 81

- **Embedded Tor client (Arti) can route all core HTTP through a local SOCKS5
  proxy** (new `core/src/tor.rs`; exports `tor_start`, `tor_stop`, `tor_status`,
  `tor_activate_custom_proxy`; `core/src/fetch/http.rs` `HttpClient::set_proxy`)
  - `tor_start(data_dir)` returns at once; a background task bootstraps Arti,
    listens on `127.0.0.1:19050` and swaps the shared reqwest client to that
    proxy.
  - `tor_status` reports `Stopped`, `Bootstrapping { percent }`, `Ready` or
    `Error { message }`; `tor_stop` tears the proxy down and goes back to direct
    connections.
  - `tor_activate_custom_proxy(socks5_url)` uses a user-supplied SOCKS5 proxy
    (e.g. Orbot) without starting Arti.
  - Each SOCKS5 connection gets its own Tor circuit (Arti's default stream
    isolation).
  - The shared `HttpClient` keeps its reqwest client behind a lock so it can be
    rebuilt with or without a proxy; reqwest gains the `socks` feature.
- **Litecoin sends to `ltcmweb1`/`tmweb1` addresses go out as MWEB peg-ins**
  (new `core/src/send/chains/mweb.rs`; `send/chains/litecoin.rs`
  `sign_and_broadcast_mweb_peg_in`; `derivation/chains/litecoin.rs`
  `parse_mweb_address`, `is_mweb_address`)
  - The Litecoin transaction carries a HogEx output (witness v8 + Pedersen
    commitment) for the amount plus optional P2PKH change; an MWEB extension
    block (one output, one peg-in kernel) follows the locktime.
  - Stealth-address keys follow DKSAP with BLAKE3 domain tags from Litecoin
    Core's libmw; commitments and Bulletproof range proofs come from
    `grin_secp256k1zkp`.
  - The fee is the caller's, floored at 1017 sats
    (`MWEB_PEGIN_OVERHEAD_BYTES`, the extension block's size at 1 sat/vbyte).
- Total: 7 .rs files changed (2 new), +803 −34 lines; data and config files:
  `Cargo.lock`, `core/Cargo.toml`; dependencies: added arti-client 0.22,
  blake3 1, secp256k1zkp 0.7 (package `grin_secp256k1zkp`), tokio-util 0.7,
  tor-rtcompat 0.22 (`core/Cargo.toml`); `#[test]` functions unchanged at 271.

## Beta Commit 80

- **EVM transactions are RLP-encoded with `alloy-rlp` and take more send
  options** (`core/src/send/chains/evm.rs`; `core/src/send/ethereum.rs`
  `EvmSendOverridesInput`)
  - The hand-written RLP encoder for the EIP-1559 signing payload and signed
    transaction is replaced by `alloy-rlp`.
  - `gas_limit` pins the gas limit (defaults: 21 000 for plain ETH, the node
    estimate otherwise).
  - `calldata_hex` attaches data to a native send, or replaces the ERC-20
    `transfer` calldata for an arbitrary contract call.
  - `access_list_json` supplies an EIP-2930 access list (before: always empty).
  - `sign_only` signs without broadcasting and returns the raw transaction hex
    with an empty txid.
  - The buffer on `eth_estimateGas` is a parameter, default 20%.
  - `fetch_erc20_metadata` asks for decimals and symbol in one JSON-RPC batch
    (`EvmClient::call_batch` in `core/src/fetch/chains/evm.rs`; before: two
    requests).
  - `encode_erc20_approve`, `encode_erc20_transfer_from`,
    `EvmClient::fetch_erc20_allowance` and `fetch_nonce_and_fee` added, with
    no caller.
- **Bitcoin sends gain coin control** (`core/src/send/chains/bitcoin.rs`;
  `CoinSelectionStrategy`, `BitcoinExtraOutput` in `core/src/staking/types.rs`)
  - Coin selection is largest-first or smallest-first; `pinned_utxos` bypasses
    selection.
  - `extra_outputs` adds further recipients or an OP_RETURN payload of up to 80
    bytes.
  - The change dust threshold (default 546 sats) and sign-only are parameters.
- **Hardcoded per-chain send constants become optional parameters, defaulting to
  the old values** (`core/src/send/chains/*.rs`)
  - Change dust threshold for Litecoin, Dogecoin, Bitcoin Cash, Bitcoin SV,
    Bitcoin Gold, Dash (546) and Decred (6 030 atoms); Kaspa minimum fee and
    dust threshold (1 000 sompi); Cardano TTL (7 200 slots) and minimum change
    (1 ADA).
  - NEAR token-transfer gas (30 TGas); Polkadot era (immortal) and tip (0);
    Stellar network passphrase (public network); TON subwallet id (698983191),
    message expiry (60 s) and send mode.
  - Zcash's NU5 version-group and branch ids move into a `ZcashNetworkUpgrade`
    value passed to the signer, with a dust-threshold parameter.
- **Polkadot sr25519 keys can expand in schnorrkel's Uniform mode**
  (`core/src/derivation/chains/polkadot.rs`): selected when the request's
  `hmac_key` is `"uniform"`; the default stays Ed25519 expansion.
- **Keccak-256 comes from `sha3`** (EVM, Tron, Sui, Aptos, Monero derivation and
  `derivation/engine.rs`; before: `tiny-keccak`, now dropped).
- **`service.rs` token-balance fan-out shares a `coin_token_balances!` macro**
  and EVM chain checks go through `chain_for_evm_id`; behaviour unchanged.
- Total: 28 .rs files changed, +1141 −638 lines; data and config files:
  `Cargo.lock`, `core/Cargo.toml`, `resources/icons/okb.svg`,
  `resources/icons/sonic.svg`, `resources/icons/zksyncera.svg`; dependencies:
  added alloy-rlp 0.3, removed tiny-keccak (`core/Cargo.toml`); `#[test]`
  functions unchanged at 271.

## Beta Commit 79

- **Balance refresh no longer writes into a Rust-side wallet state**
  (`core/src/fetch/refresh_engine.rs`, `core/src/service.rs`)
  - Each fetched native balance becomes a one-holding `WalletSummary` built from
    `native_coin_template` and is handed to
    `BalanceObserver::on_balance_updated` for the app to merge (before: core
    upserted the holding into `WalletService.wallet_state` and sent that wallet,
    or `None` when the wallet was not loaded).
  - `apply_native_amount_typed`, `apply_native_balance_summary` and
    `upsert_asset_holding` are deleted.
- **Icons for Berachain, Bittensor, Kaspa, OKB, Sonic and zkSync Era**
  (`resources/icons/`); Dogecoin, Monero and Solana redrawn.
- Total: 2 .rs files changed, +25 −65 lines; data and config files:
  `resources/icons/` (9); dependencies unchanged; `#[test]` functions
  unchanged at 271.

## Beta Commit 78

- **Cronos, Dogecoin and Sky USDS icons redrawn** (`resources/icons/`)
- Total: no .rs files changed; data and config files:
  `resources/icons/cronos.svg`, `resources/icons/dogecoin.svg`,
  `resources/icons/skyusds.svg`; dependencies unchanged; `#[test]` functions
  unchanged at 271.

## Beta Commit 77

- **Token icon names follow issuer + symbol** (`core/data/tokens.toml`
  `asset_name`; `resources/icons/`)
  - 21 `asset_name` values change to match the files, e.g. `usdc` →
    `circleusdc`, `tether` → `tetherusdt`, `dai` → `skydai`, `link` →
    `chainlink`, `pyusd` → `paypalpyusd`, `fdusd` → `firstdigitalfdusd`.
  - Icon files renamed to match: `link` → `chainlink`, `pyusd` → `paypalpyusd`,
    `dai` → `skydai`, `zec` → `zcash`.
- **Icons added** (`resources/icons/`): chains Base, Blast, Celo, Cronos,
  Decred, Hyperliquid, Mantle, Polkadot, Polygon, Scroll; tokens FDUSD, PAXG,
  Pump.fun, Shiba Inu; fiat CNY, EUR, GBP, RUB, USD under
  `resources/icons/fiat/`. Aptos redrawn.
- Total: no .rs files changed; data and config files: `resources/icons/` (24),
  `core/data/` (1); dependencies unchanged; `#[test]` functions unchanged at
  271.

## Beta Commit 76

- **Icon set extended and redrawn** (`resources/icons/`)
  - 25 added: chains Arbitrum, Avalanche (replacing `avax.svg`), Internet
    Computer, NEAR, Sui, TON, Tron; tokens Circle EURC and USDC, Ethena and
    USDe, Jupiter, LEO, Paxos USDG, Ripple RLUSD, Sky and USDS, Tether USDT and
    XAUt, TUSD, Uniswap, USDD, WLFI and USD1, Wrapped Bitcoin.
  - 22 redrawn, among them AAVE, BNB, Bitcoin, Cardano, Ethereum, Litecoin,
    Stellar, XRP and Zcash.
- Total: no .rs files changed; data and config files: `resources/icons/` (48);
  dependencies unchanged; `#[test]` functions unchanged at 271.

## Beta Commit 75

- **`ARCHITECTURE.md` describes the layout as it is** (before: a "Target
  Architecture" proposal)
  - Workspace members now include `cli`; a file-level map of `core/src/` covers
    the root files and `derivation/`, `fetch/`, `send/`, `store/`, `staking/`,
    `diagnostics/`.
  - New sections on the FFI surface, build commands and design principles; the
    migration plan ("What changes from the current codebase", "What stays the
    same") is removed.
- Docs: `ARCHITECTURE.md` rewritten throughout.
- Total: No code changed.

## Beta Commit 74

- **Derivation requests accept override names as strings**
  (`core/src/derivation/engine.rs` `UniFFIDerivationRequest`): optional
  `curve_override_name`, `derivation_algorithm_override_name`,
  `address_algorithm_override_name`, `public_key_format_override_name` and
  `script_type_override_name` are resolved through `presets::*_wire_value` and
  replace the matching numeric field when set.
- **Dogecoin-only transaction fields removed** (`core/src/fetch/transactions.rs`
  `CoreTransactionRecord`, `core/src/store/persistence_models.rs`,
  `core/src/send/verification.rs`)
  - `dogecoin_confirmations`, `dogecoin_fee_priority_raw`,
    `dogecoin_used_change_output` and `dogecoin_raw_transaction_hex` are gone
    from the merge and persisted records.
  - Send verification reads `confirmation_count` (before:
    `dogecoin_confirmations`).
- **`.gitignore` ignores `swift/generated/`** (UniFFI output)
- Total: 4 .rs files changed, +67 −52 lines; data and config files:
  `.gitignore`; dependencies unchanged; `#[test]` functions unchanged at 271.

## Beta Commit 73

- **Module layout consolidated; the set of exported functions is unchanged**
  (`core/src/`)
  - `app_core/` back to `app_core.rs`; `service/` (`mod`, `chain_client`,
    `derivation_methods`, `helpers`, `pricing_methods`, `standalone`,
    `state_methods`, `types`) back to `service.rs`; `registry/mod.rs` →
    `registry.rs`.
  - `ffi.rs` is dissolved: each `#[uniffi::export]` wrapper moves next to its
    implementation (`fetch/history.rs`, `fetch/refresh.rs`, `send/flow.rs`,
    `store/mod.rs`, …).
  - `send/flow_helpers.rs` merges into `send/flow.rs`, `fetch/http_ffi.rs` into
    `fetch/http.rs`, `fetch/balance_observer.rs` into `fetch/refresh_engine.rs`,
    `registry/tokens.rs` and `store/app_state/token_helpers.rs` into
    `tokens.rs`.
  - `store/wallet_core.rs` → `send/preview_types.rs` (re-exported as
    `wallet_core`), `store/persistence/models.rs` →
    `store/persistence_models.rs`, `platform/formatting.rs` and
    `platform/resources.rs` → crate root.
- **Chain-catalog bootstrap and embedded localization deleted**
  (`platform/catalog.rs` `core_bootstrap`, `platform/localization.rs`,
  `platform/types.rs`): the Base/en/zh-Hans localization tables and the
  `CoreCapabilities`/bootstrap types go with them; none was exported.
  `cli/src/main.rs` still calls
  `spectra_core::platform::catalog::core_bootstrap`, which no longer exists.
- Tests: 0 new, 3 removed
  - Catalog 1 and localization 2, deleted with their modules.
- Total: 47 .rs files changed (1 new, 21 deleted), +4630 −5321 lines; data and
  config files: none; dependencies unchanged; `#[test]` functions 274 → 271.

## Beta Commit 72

- **Derivation rebuilt as one self-contained file per chain** (new
  `core/src/derivation/engine.rs`; `derivation/chains/*.rs`;
  `derivation/runtime/mod.rs` deleted)
  - Each `chains/<chain>.rs` holds its own BIP-39 handling, curve walk and
    address encoder; `engine.rs` holds the request and response types,
    validation, the per-chain dispatch table and the UniFFI entry points
    (`derivation_derive`, `derivation_build_material`, …).
  - Testnets dispatch through `Chain::mainnet_counterpart()` to their mainnet's
    file, which picks version bytes, HRP or SS58 prefix from the requested
    chain.
  - `bitcoin_primitives.rs` is deleted and its BIP-32, xpub and address helpers
    live in `chains/bitcoin.rs`; `addressing.rs` → `validation.rs`, `utxo_hd.rs`
    → `xpub_walker.rs` (old names kept as re-export aliases); `runtime/tests.rs`
    → `derivation/tests.rs`.
- **Icons for DAI, Ink, KCS and LINK** (`resources/icons/`)
- Measured: 2026-04-30, `core/src/derivation/chains/REFACTOR_NOTES.md` records
  `cargo test --lib -p spectra_core` after the refactor: 283 passed, 0 failed.
- Docs: `core/src/derivation/chains/REFACTOR_NOTES.md` (new): layout, testnets,
  what was deleted, open follow-ups (the Sui seed-phrase path hashes with
  Keccak-256 while the watch-only encoder uses Blake2b-256;
  `derive_from_private_key` still dispatches in `engine.rs`).
- Total: 32 .rs files changed (1 new, 2 deleted), +7989 −3840 lines; data and
  config files: `resources/icons/dai.svg`, `resources/icons/ink.svg`,
  `resources/icons/kcs.svg`, `resources/icons/link.svg`; dependencies
  unchanged; `#[test]` functions unchanged at 274.

## Beta Commit 71

- **Testnets become chains of their own** (`core/src/registry/mod.rs` `Chain`
  ids 46–77, `is_testnet`, `mainnet_counterpart`, `mainnets()`, `testnets()`)
  - 32 testnets: Bitcoin Testnet, Testnet4 and Signet; Litecoin, Bitcoin Cash,
    Bitcoin SV, Dogecoin, Zcash, Decred, Kaspa and Dash testnets; Ethereum
    Sepolia and Hoodi, Arbitrum, Optimism and Base Sepolia, BNB Chain Testnet,
    Avalanche Fuji, Polygon Amoy, Hyperliquid Testnet, Ethereum Classic Mordor;
    Tron Nile, Solana Devnet, XRP, Stellar, Sui, Aptos, TON and NEAR testnets,
    Cardano Preprod, Polkadot Westend, Monero Stagenet.
  - Each has its own derivation preset row (`core/data/derivation_presets.toml`,
    `core/data/DerivationPresets.json`) and a live chain backend with its
    mainnet's symbols (`core/src/app_core/registry_data.rs`).
  - Derivation loses its network parameter: presets drop their `networks` list,
    `runtime/mod.rs` its `NETWORK_*` constants, and Bitcoin-family network
    parameters come from the chain (`bitcoin_network_params_for_chain`).
  - `core_evm_chain_context_tag` ignores its `ethereum_network_mode` argument;
    the chain name alone selects `ethereum_sepolia`, `polygon_amoy`, …
    (`send/flow_helpers.rs`).
- **Address validation has a kind per testnet**
  (`core/src/derivation/addressing.rs`, `core/src/send/flow.rs`)
  - Bitcoin testnets, Bitcoin Cash `bchtest:` and legacy testnet, Litecoin
    `tltc1`/`m`/`n`/`Q`/`2`, Dogecoin `n`/`2` and Monero stagenet `5`/`7`
    addresses validate under their own kinds; the other testnets reuse their
    mainnet validator.
  - `network_mode` on the request no longer selects the network for Bitcoin and
    Dogecoin (before: `testnet`/`testnet4`/`signet` on the mainnet kind).
  - Watch-only Bitcoin import still accepts a testnet address on the "Bitcoin"
    row, validating it as `bitcoinTestnet` (before: testnet or signet mode).
- **Icons added for BGB, Bitcoin Gold, Bitcoin SV, BNB, BTT, Cardano, Monero,
  PYUSD, Solana, Stellar and XRP** (`resources/icons/`); 14 others redrawn.
- Tests: 2 new
  - Testnet registry 2: mainnet and testnet counts not adding up to all chains;
    a testnet whose counterpart is not a mainnet.
- Total: 10 .rs files changed, +1000 −241 lines; data and config files:
  `resources/icons/` (25), `core/data/` (2); dependencies unchanged; `#[test]`
  functions 272 → 274.

## Beta Commit 70

- **Six non-EVM chains: Zcash, Bitcoin Gold, Decred, Kaspa, Dash, Bittensor**
  (`core/src/registry/mod.rs` ids 30–33, 43, 45; one file each in
  `derivation/chains/`, `fetch/chains/`, `send/chains/`)
  - Zcash: transparent-only V5 transactions signed per ZIP-244 (personalised
    BLAKE2b, NU5 ids), via Trezor Blockbook `zec1.trezor.io`.
  - Bitcoin Gold: BIP143 P2PKH with SIGHASH_FORKID and fork id 79, via Blockbook
    `btg1.trezor.io`.
  - Decred: BLAKE-256 sighash over the split prefix/witness serialization, via
    the dcrdata Insight API.
  - Kaspa: Schnorr signatures over a keyed BLAKE2b sighash, transactions posted
    as JSON to `api.kaspa.org`.
  - Dash: legacy P2PKH, via Blockbook `dash1.trezor.io`.
  - Bittensor: sr25519-signed `Balances.transfer_keep_alive` extrinsics over the
    OpenTensor Finney RPC.
  - Each has a derivation preset, address validation, watch-only import
    addresses, a receive-address resolver kind, endpoint entries and a live
    chain backend.
- **Ten EVM chains: Sei, Celo, Cronos, opBNB, zkSync Era, Sonic, Berachain,
  Unichain, Ink, X Layer** (ids 34–42 and 44; EVM chain ids 1329, 42220, 25,
  204, 324, 146, 80094, 130, 57073, 196), each with one RPC endpoint, a
  transaction-explorer entry, an RPC broadcast option and a live chain backend.
- **Polkadot extrinsic call indexes come from a shared table** (new
  `core/src/send/chains/substrate.rs` `RuntimeCallIndex`), used by Bittensor
  too; Polkadot and Bittensor send params are typed structs
  (`service/types.rs`).
- **Network failures surface as `SpectraBridgeError::Network`, body-decode
  failures as `Decode`** (`core/src/lib.rs` `From<reqwest::Error>`; before: both
  `Failure`).
- **Per-chain facts move onto `Chain`** (`supports_deep_utxo_discovery`,
  `flags_evm_address_as_wrong_chain`, `as_evm` returning an `EvmChain` newtype),
  replacing chain-name lists in `send/flow_helpers.rs` and `ffi.rs`.
- **Built-in tokens regrouped by symbol; PEPE (Ethereum, disabled) and BONK
  (Solana) removed** (`core/data/tokens.toml`, 93 → 91 entries).
- **Icons for Dash and Sei** (`resources/icons/`)
- Tests: 15 new
  - Address codecs for the new chains 15 (Bitcoin Gold 2, Bittensor 1, Dash 2,
    Decred 3, Kaspa 4, Zcash 3): a Bitcoin P2PKH address accepted as BTG, Dash
    or Zcash; garbage or a corrupted checksum accepted; a malformed P2PKH
    script; BLAKE-256 drifting from known vectors; Kaspa bech32 or Schnorr
    round-trips breaking.
- Docs: `docs/FFI-BOUNDARY.md` (new): typed (`ffi.rs`) vs `chain_id`-dispatched
  (`WalletService`) endpoints, what can and cannot cross UniFFI 0.29,
  regenerating bindings, boundary-breaking changes, JSON conventions, Rust vs
  Swift state, common errors.
- Total: 63 .rs files changed (19 new), +5607 −143 lines; data and config
  files: `core/data/` (3), `resources/icons/` (2), `Cargo.lock`, `core/` (1);
  dependencies: added blake2b_simd 1.0 (`core/Cargo.toml`); `#[test]`
  functions 257 → 272.

## Beta Commit 69

- **CLI shell gains wallet, balance, history, send, price and portfolio
  commands** (`cli/src/main.rs`; a tokio runtime drives `WalletService`)
  - `show`/`info`, `rename`/`mv` and `receive`/`recv`: wallet details, renaming,
    the receive address.
  - `export`: after typing `yes` and the wallet password, prints the decrypted
    seed phrase; refused for watch-only wallets.
  - `send`/`tx`: native-coin transfer from a seed wallet through `execute_send`,
    after a typed `yes` and the password check; prints the transaction hash.
  - `balance`/`bal`, `history`/`hist`, `price` and `portfolio`/`p`: live
    balance, history, price, and balances × CoinGecko prices summed in USD.
  - `staking`/`stake`: lists the stakeable chains (Solana, Sui, Aptos, Polkadot,
    ICP, NEAR, Cardano) and says staking is not wired up.
  - Output uses a per-chain truecolor palette and new banner and section
    styling.
- **EVM send overrides are typed** (`core/src/send/ethereum.rs`
  `EvmSendOverridesInput`; `SendExecutionRequest.evm_overrides`,
  `SendExecutionResult.evm`)
  - Nonce and custom fees arrive as a record (before: a JSON fragment from the
    exported `build_evm_overrides_json_fragment`).
  - `execute_send` returns the decoded EVM result (nonce, raw tx hex, gas limit)
    for EVM chains; `decode_evm_send_result` is no longer exported.
- **History records hold a typed payload, stored as JSON text**
  (`core/src/store/wallet_db.rs` `HistoryRecord.payload`)
  - `CorePersistedTransactionRecord` crosses the FFI typed and core serializes
    it for the SQLite column (before: a base64 string of the JSON); a row in the
    old form makes `history_fetch_all` fail.
  - SQLite runs with `synchronous = NORMAL` and `temp_store = MEMORY`.
- **Balance refresh is typed end to end** (`core/src/fetch/refresh_engine.rs`):
  `fetch_native_balance_summary_auto` then `apply_native_balance_summary`
  (before: balance JSON re-parsed by `update_native_balance_typed`); cycle logs
  print in debug builds only.
- **JSON bridges removed** (`core/src/ffi.rs`): encode/decode of the persisted
  price-alert and address-book stores, `HistoryRecordEncodeInput`;
  `core_static_resource_json` returns a `String` (before: bytes).
- **`SpectraBridgeError` gains `Network`, `Decode` and `InvalidInput`**
  (`core/src/lib.rs`): JSON and hex decode errors map to `Decode` (before:
  `Failure`, the only variant).
- **`service/mod.rs` splits further** into `chain_client.rs`,
  `derivation_methods.rs`, `pricing_methods.rs` and `state_methods.rs`; `lib.rs`
  re-exports trimmed.
- **Icons added for AVAX, Bitcoin Cash, Ethereum, Ethereum Classic, Litecoin,
  Optimism and Zcash** (`resources/icons/`); AAVE, Aptos, Bitcoin, Dogecoin and
  Linea redrawn.
- Total: 48 .rs files changed (4 new), +2339 −1549 lines; data and config
  files: `resources/icons/` (12), `Cargo.lock`, `cli/` (1); dependencies:
  added tokio 1 (`cli/Cargo.toml`); `#[test]` functions unchanged at 257.

## Beta Commit 68

- **Staking scaffolding: shared types and one client per chain, every operation
  unimplemented** (new `core/src/staking/`)
  - Shared UniFFI types: `StakingValidator`, `StakingPosition`,
    `StakingActionPreview`, `StakingActionKind`, `StakingPositionStatus`,
    `StakingError`.
  - Clients for Aptos, Cardano, ICP, NEAR, Polkadot, Solana and Sui name each
    chain's own operations (e.g. Solana create-and-delegate, deactivate,
    withdraw); every method returns `StakingError::NotYetImplemented` and none
    is exported.
- Total: 10 .rs files changed (9 new), +680 −0 lines; data and config files:
  none; dependencies unchanged; `#[test]` functions unchanged at 257.

## Beta Commit 67

- **Five EVM chains: Polygon, Linea, Scroll, Blast, Mantle; Base becomes an app
  chain** (`core/src/registry/mod.rs` ids 25–29, EVM chain ids 137, 59144,
  534352, 81457, 5000; `core/data/AppEndpointDirectory.json`,
  `core/data/DerivationPresets.json`, `core/src/app_core/registry_data.rs`,
  `core/src/store/wallet_domain.rs`)
  - Each of the six, Base included, gets three RPC endpoints and a
    transaction-explorer entry, a derivation preset, a seed-derivation-path
    field and a live chain backend; natives are POL on Polygon, MNT on Mantle,
    ETH elsewhere.
  - Built-in tokens: USDC on Base, Linea, Mantle, Polygon and Scroll; USDT and
    DAI on Polygon; DAI on Base; USDB on Blast (`core/data/tokens.toml`).
- **Large files split by concern** (behaviour unchanged): `app_core.rs` →
  `app_core/{mod,derivation_paths,registry_data}.rs`; `service.rs` →
  `service/{mod,helpers,standalone,types}.rs`; `derivation/runtime.rs` tests →
  `runtime/tests.rs`; chain aliases and tests out of `store/mod.rs` into
  `store/chain_aliases.rs` and `store/tests.rs`.
- **Unused client fields removed** (`fetch/chains/*.rs`): `BitcoinClient::new`
  drops its network mode, `IcpClient::new` its IC gateway endpoints, and
  never-read Dogecoin Blockbook fields go; JSON-RPC bodies are shared across
  endpoint fallbacks instead of cloned per attempt.
- **Icons for Aptos and Linea** (`resources/icons/`); AAVE, Bitcoin and Dogecoin
  redrawn.
- Total: 35 .rs files changed (9 new, 1 deleted), +3280 −3627 lines; data and
  config files: `resources/icons/` (5), `core/data/` (3), `.vscode/` (1);
  dependencies unchanged; `#[test]` functions unchanged at 257.

## Beta Commit 66

- **Per-chain code regrouped by direction** (`core/src/derivation/chains/`,
  `fetch/chains/`, `send/chains/`, one `<chain>.rs` each, replacing
  `chains/<chain>/{mod,derive,fetch,send}.rs`; each `mod.rs`'s client and shared
  types merge into the fetch file)
- **Shared modules moved into folders** (`core/src/`):
  `chains/registry.rs` → `registry/mod.rs`, `tokens.rs` →
  `registry/tokens.rs`; `catalog`, `formatting`, `localization`, `resources`,
  `types` → `platform/`; `diagnostics_sanitizer.rs` →
  `diagnostics/sanitizer.rs`, `self_tests.rs` → `diagnostics/self_tests.rs`.
- **CoinMarketCap ids removed** (`market_id` in `core/data/tokens.toml` and
  `TokenEntry`; `market_data_id` on `CoreCoin`, `AssetHolding`,
  `CoreTokenPreferenceEntry`, the holdings-merge records and
  `Chain::market_data_id`); CoinGecko ids remain.
- **Balance refresh cycles no longer overlap**
  (`core/src/fetch/refresh_engine.rs`): a cycle that starts while another runs
  (timer tick or `trigger_immediate`) is skipped; each cycle logs start, end,
  counts and elapsed time to stderr.
- **`Chain::is_bitcoin_utxo` deleted** (`core/src/registry/mod.rs`)
- **Icons for AAVE, Bitcoin and Dogecoin** (new `resources/icons/`);
  `.vscode/settings.json` sets the SVG preview background.
- Tests: 2 new, 1 removed
  - Refresh in-flight gate 2: two concurrent cycles both running; the gate not
    released after a cycle.
  - Removed `bitcoin_utxo_group_matches_legacy` with `is_bitcoin_utxo`.
- Total: 104 .rs files changed (7 new, 22 deleted), +3098 −3124 lines; data
  and config files: `.vscode/settings.json`, `core/data/tokens.toml`,
  `resources/icons/aave.svg`, `resources/icons/bitcoin.svg`,
  `resources/icons/dogecoin.svg`; dependencies unchanged; `#[test]` functions
  256 → 257.

## Beta Commit 65

- **Each chain module split into `mod.rs`, `derive.rs`, `fetch.rs` and
  `send.rs`** (`core/src/chains/<chain>/` for Aptos, Bitcoin, Bitcoin Cash,
  Bitcoin SV, Cardano, Dogecoin, EVM, ICP, Litecoin, Monero, NEAR, Polkadot,
  Solana, Stellar, Sui, TON, Tron, XRP); code moved and reformatted, behaviour
  unchanged.
- Total: 88 .rs files changed (70 new, 16 deleted), +9873 −9596 lines; data
  and config files: none; dependencies unchanged; `#[test]` functions
  unchanged at 256.

## Beta Commit 64

- **TLS roots come from `webpki-roots`** (`core/Cargo.toml` reqwest feature
  `rustls-tls-webpki-roots`; before: `rustls-tls-native-roots`)
- **EVM history uses the Etherscan V2 API with one key for every chain**
  (`core/src/chains/evm.rs`; `core/src/service.rs` `set_etherscan_api_key`)
  - `txlist` and `tokentx` go to `https://api.etherscan.io/v2/api?chainid=<id>`
    for every EVM chain (before: each chain's explorer base, with `chainid` only
    on token transfers).
  - `WalletService::set_etherscan_api_key` sets the key once (before: a
    per-chain key from the endpoint table).
- **Price providers trimmed to public endpoints** (`core/src/fetch/price.rs`,
  `service.rs` `fetch_prices_typed`)
  - Binance and Coinbase Exchange removed; CoinGecko, CoinPaprika and CoinLore
    remain.
  - CoinGecko takes no API key and no longer tries the pro endpoint first;
    `fetch_prices_typed` loses its `api_key` argument.
  - An empty CoinGecko response returns an empty map (before: an error).
  - Price and fiat-rate calls log provider, counts and failures to stderr.
- **HTTP errors carry the full error chain** (`core/src/fetch/http.rs`
  `format_reqwest_error`): source errors plus
  timeout/connect/request/body/decode flags.
- **NEAR history parsing moved into `core/src/chains/near.rs`**
  (`near_parse_history_response`, `NearHistoryParsedSnapshot` and their two
  tests, from `core/src/fetch/balance_decoder.rs`, which is deleted).
- **Tron's TronScan transaction-list history deleted** (`chains/tron.rs`
  `fetch_history`, `TronHistoryEntry`); the unified TRX/TRC-20 history stays.
- Total: 9 .rs files changed (1 deleted), +320 −473 lines; data and config
  files: `Cargo.lock`, `core/Cargo.toml`; dependencies unchanged; `#[test]`
  functions unchanged at 256.

## Beta Commit 63

- **Embedded data files move to `core/data/`**: `AppEndpointDirectory.json`,
  `DerivationPresets.json`, `DerivationRequestCompilationPresets.json` from
  `core/embedded/`; `derivation_presets.toml`, `tokens.toml`,
  `DERIVATION_WORKFLOW.md` from `core/`.
- **Visual registries and the BIP-39 word list are no longer static resources**
  (`core/src/resources.rs`)
  - `TokenVisualRegistry.json`, `ChainVisualRegistry.json`,
    `BuyCryptoProviders.json` and `BIP39EnglishWordList.txt` are deleted, so
    `core_static_resource_json` and `core_static_text_resource_utf8` find
    nothing; `bip39_english_wordlist` (from the `bip39` crate) stays.
  - The two resource tests now check that an unknown name returns `None`.
- **Token visuals live in `tokens.toml`** (`color_name`, `asset_name` per entry,
  carried on `TokenEntry`)
- **The two-letter `mark` is dropped** from `CoreCoin`, `CoreDashboardPinOption`
  and the holdings-merge append payload (before: core derived it from the symbol
  for new holdings).
- Docs: `DERIVATION_WORKFLOW.md` moved to `core/data/`, content unchanged.
- Total: 8 .rs files changed, +27 −49 lines; data and config files:
  `core/data/` (5), `core/embedded/` (4); dependencies unchanged; `#[test]`
  functions unchanged at 256.

## Beta Commit 60

- **Per-chain derivation defaults read from `core/derivation_presets.toml`**
  (new `core/src/derivation/presets.rs`): curve, derivation algorithm, address
  algorithm, public-key format, script type and supported networks per chain,
  replacing match statements in `derivation/runtime.rs`.
- **Wallets carry optional derivation overrides, reused at signing**
  (`core/src/store/wallet_domain.rs` `CoreWalletDerivationOverrides` on
  `CoreImportedWallet.derivation_overrides`; `core/src/send/mod.rs`
  `SendExecutionRequest.derivation_overrides`; `service.rs` `execute_send`)
  - Overrides: passphrase, mnemonic word list, iteration count, salt prefix,
    HMAC key, curve, derivation and address algorithm, public-key format, script
    type, as strings named like the presets file; `None` keeps the preset
    default.
  - `execute_send` re-derives the signing key with the request's overrides
    (`derive_key_material_for_chain_with_overrides`).
  - Wallets stored without the field load with no overrides
    (`#[serde(default)]`).
- Total: 6 .rs files changed (1 new), +490 −214 lines; data and config files:
  `core/derivation_presets.toml`; dependencies unchanged; `#[test]` functions
  unchanged at 256.

## Beta Commit 59

- **Swift-side planning logic ported to core exports** (`core/src/store/mod.rs`,
  `core/src/ffi.rs` `core_plan_*`)
  - Built-in token preference merge; price-alert evaluation; whether a live
    price change needs a dashboard rebuild.
  - Ethereum custom-fee and manual-nonce validation codes; Ethereum send-error
    classification (`EthereumSendErrorCode`, the app supplies the text).
  - Chain operational event log append; transaction-status polling (whether to
    poll, success and failure backoff); stale pending failures; applying
    resolved pending statuses.
  - Keypool state baseline and merge; holdings merge from a balance summary; EVM
    recipient preflight warnings.
- **`SendStateMachine` deleted** (`core/src/send/machine.rs`): the JSON-driven
  send-flow state object exported to Swift.
- **Bitcoin HD history is one call** (`service.rs`
  `fetch_bitcoin_hd_history_page`): derives 20 receive and 10 change addresses
  from the xpub, fetches their histories four at a time and returns a merged,
  deduplicated page; `fetch_bitcoin_history_snapshots` and
  `core_merge_bitcoin_history_snapshots` are no longer exported.
- **`WalletService` exports only its FFI methods** (`service.rs`): internal
  helpers (chain dispatch behind `execute_send`, balance and history helpers)
  move out of the `#[uniffi::export]` impl block, which exported them regardless
  of `pub(crate)`.
- **The last JSON field decoders are deleted** (`fetch/balance_decoder.rs`:
  `balance_decoder_i64_field`, `_f64_field`, `_first_element_string_field`)
- Total: 11 .rs files changed (1 deleted), +3421 −2922 lines; data and config
  files: none; dependencies unchanged; `#[test]` functions unchanged at 256.

## Beta Commit 58

- **`WalletService` drops its remaining JSON-string methods**
  (`core/src/service.rs`)
  - Removed: the JSON constructor `new(endpoints_json)` and `update_endpoints`,
    `fetch_token_balance`, `derive_bitcoin_account_xpub`,
    `derive_bitcoin_hd_addresses`, `fetch_bitcoin_next_unused_address`,
    `fetch_prices`, `fetch_fiat_rates`, `resolve_ens_name`, `fetch_evm_code`,
    `fetch_evm_tx_nonce`, `fetch_utxo_tx_status` and the JSON owned-address
    save/load; the typed versions remain.
  - `broadcast_raw`, `fetch_tron_send_preview` and `fetch_utxo_fee_preview`
    become `pub(crate)`.
- **Built-in token list is typed** (`core/src/tokens.rs` `list_tokens`; export
  `list_builtin_tokens` returns `Vec<TokenEntry>`; before: a cached JSON string
  from `list_builtin_tokens_json`).
- Total: 2 .rs files changed, +23 −388 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions unchanged at 256.

## Beta Commit 57

- **Exports that passed JSON strings replaced by typed records**
  (`core/src/app_core.rs`, `service.rs`, `derivation/runtime.rs`,
  `self_tests.rs`, `diagnostics/aggregate.rs`, `fetch/`)
  - The 18 `app_core_*_json` catalog exports are removed (all but
    `app_core_endpoint_records_json` had a typed twin); preset fields become
    enums (`AppCoreDerivationAlgorithm`, `AppCoreAddressAlgorithm`,
    `AppCorePublicKeyFormat`, `AppCoreScriptPolicy`, `AppCoreScriptType`) and
    chain-backend state `AppCoreChainIntegrationState` (before: strings).
  - `derivation_derive_json`, `derivation_build_material_json`, their
    private-key variants, `derivation_derive_all_addresses_json`,
    `self_tests_run_chain_json`/`_run_all_json` and the JSON balance and history
    decoder exports are removed.
  - `WalletService` gains typed fetches: `fetch_native_balance_summary`
    (smallest unit, display amount, UTXO count), history activity, entry count
    and confirmed txids, typed send previews for EVM, Tron, UTXO, Dogecoin,
    Bitcoin HD and simple chains, `fetch_evm_address_probe` (nonce and balance),
    EVM receipt classification.
  - Its JSON `fetch_balance`, `fetch_history`, `sign_and_send`,
    `sign_and_send_token` and `fetch_fee_estimate` become `pub(crate)`; the JSON
    wallet-snapshot, settings, keypool and wallet-state methods are removed.
  - `BalanceObserver::on_balance_updated` receives the `WalletSummary` after
    core applies the balance to its wallet state (before: raw balance JSON).
- **Rust-side balance and history caches deleted** (`fetch/balance_cache.rs`,
  `fetch/history_cache.rs`, `WalletService` `cached_*` methods).
- **Helper exports added** (`ffi.rs`, `send/flow_helpers.rs`,
  `diagnostics/export.rs`)
  - `core_chain_id_for_name` (also accepts "Internet Computer"),
    `core_endpoint_id` for a chain's primary, secondary or explorer slot,
    `core_evm_seed_derivation_chain_name` (BNB Chain derives on Ethereum's
    path).
  - `core_transaction_explorer_url` (Aptos appends `?network=mainnet`),
    `core_rebroadcast_prepare_payload`, `core_diagnostics_evm_json_shape_ok`;
    `core_compile_script_type` returns `AppCoreScriptType`.
- Tests: 6 new, 6 removed
  - Rebroadcast payload preparation 6: Sui signed JSON not remapped, a malformed
    Sui payload not passed through, wrap-key or extract-field payloads built
    wrong, an unknown format not refused.
  - Removed 6 with the JSON balance and history decoders.
- Total: 20 .rs files changed (2 deleted), +1097 −1513 lines; data and config
  files: none; dependencies unchanged; `#[test]` functions unchanged at 256.

## Beta Commit 56

- **Endpoint reliability ordering removed**
  (`core/src/fetch/endpoint_reliability.rs`; exports
  `core_order_endpoints_by_reliability`, `core_record_endpoint_attempt`)
- **`app_core_endpoint_records_for_chain` returns typed
  `AppCoreEndpointRecord`s** (`core/src/app_core.rs`; before:
  `app_core_endpoint_records_for_chain_json`)
- **New export `core_compile_script_type`** (`core/src/ffi.rs`, `app_core.rs`
  `compile_script_type`): a preset's Bitcoin script type, from the derivation
  path's purpose via `bitcoinPurposeScriptMap` or the preset's fixed type.
- Tests: 0 new, 2 removed
  - Endpoint reliability 2, deleted with the module.
- Total: 5 .rs files changed (1 deleted), +57 −177 lines; data and config
  files: none; dependencies unchanged; `#[test]` functions 258 → 256.

## Beta Commit 55

- **Rust mirrors of Swift `AppState` deleted**
  (`core/src/store/app_shell_state.rs`,
  `store/app_state/{caches,events,store}.rs`): the scalar-field mirror, the
  cache dictionaries, the wallets/transactions/address-book store and the event
  bus that pushed changes to a Swift observer.
- **User-facing message formatting removed from core**
  (`core/src/send/flow_helpers.rs`): Ethereum and Tron send-error text,
  address-book validation message, network and chain display titles,
  destination-risk probe messages, fiat currency display names (added in Beta
  Commit 54), send-balance validation, insufficient-funds messages, EVM
  preflight reasons.
- **Exports removed** (`core/src/ffi.rs`): `core_bootstrap_json`,
  `core_localization_document_json`, `core_build_persisted_snapshot_json`,
  `core_wallet_secret_index_json`, `core_encode_history_records_from_persisted`,
  `core_plan_display_mark`.
- **Icon and chain identifiers no longer take a localized chain id**
  (`core/src/store/mod.rs` `plan_canonical_chain_component`,
  `plan_icon_identifier`, `plan_normalized_icon_identifier`): core looks the
  chain id up by name in the embedded
  `resources/strings/base/ChainWikiEntries.json`.
- **Self-test results carry an outcome code** (`core/src/self_tests.rs`
  `ChainSelfTestOutcome` and `chain_label` on `ChainSelfTestResult`; before: an
  English `message`).
- Tests: 0 new, 29 removed
  - Deleted with the removed formatters 19, AppState caches 4, the AppState
    store 4, the event bus 2.
- Total: 10 .rs files changed (4 deleted), +178 −2044 lines; data and config
  files: none; dependencies unchanged; `#[test]` functions 287 → 258.

## Beta Commit 54

- **Two exports for the send flow and settings**
  (`core/src/send/flow_helpers.rs`)
  - `core_simple_chain_risk_probe_config`: balance field, divisor, display name
    and label for the destination-risk probe on Litecoin, Dogecoin, Solana, XRP,
    Monero, Sui and Aptos.
  - `core_fiat_currency_display_name`: display names for 12 fiat codes, the code
    itself otherwise.
- **`app_core_derivation_paths_for_preset` returns `CoreSeedDerivationPaths`**
  (`core/src/app_core.rs`; the duplicate `AppCoreSeedDerivationPaths` record is
  deleted).
- Total: 2 .rs files changed, +62 −31 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions unchanged at 287.

## Beta Commit 53

- **A chain registry replaces the numeric chain-id ladders in `WalletService`**
  (new `core/src/chains/registry.rs`: `Chain`, `EndpointSlot`;
  `core/src/service.rs`, `core/src/fetch/history.rs`)
  - `Chain` carries the frozen service chain ids 0–24 (Bitcoin 0 … Hyperliquid
    24) as discriminants; `service.rs` dispatches on it instead of `match
    chain_id { 1 | 11 | 12 | … }` ladders, `evm_chain_id_for` /
    `send_chain_from_chain_id` and `SUBSCAN_OFFSET + chain_id`-style arithmetic.
  - Endpoint lists resolve through `EndpointSlot`: primary at the chain id,
    secondary (Subscan, ICP v2, TonCenter v3) at id + 100, explorer at id + 200.
  - An id with no handler is refused with "unsupported chain: …" (before:
    "unknown chain_id: …"); EVM-only calls such as `fetch_token_balance_typed`
    refuse a non-EVM id.
  - History rows take their asset name, symbol and chain name from the registry.
- **The catalog JSON is embedded from `core/embedded/`**
  (`AppEndpointDirectory.json`, `DerivationPresets.json`,
  `DerivationRequestCompilationPresets.json`; `app_core.rs`, `catalog.rs`;
  before: read from `swift/derivation/` and `swift/rustbridge/` in the app tree)
- **Store planning exports lifted from Swift** (`core/src/store/mod.rs`,
  `core_plan_*` in `core/src/ffi.rs`)
  - Priced chain, active-wallet transaction ids, normalized history signature,
    earliest transaction dates, has-wallet-for-chain, canonical chain component,
    icon identifier, display mark, and derived-or-stored address resolution.
  - `core_plan_reset_dispatch` turns reset scopes into a plan; resetting wallets
    and secrets also resets history and cache and clears network caches.
- **`core_encode_history_records_from_persisted`** (`core/src/ffi.rs`): converts
  persisted transaction records, whose `created_at` is in Swift reference-date
  seconds, into history records in Unix seconds.
- **Send-flow helpers** (`core/src/send/flow.rs`, `flow_helpers.rs`)
  - `active_pending_refresh_interval_for_profile`: 30 s for "conservative", 10 s
    for "aggressive", otherwise the balanced interval.
  - `portfolio_composition_signature`: sorted holding keys joined by `|`.
  - `core_plan_receive_address_resolver` routes `(symbol, chain, is_evm)` to a
    receive-address resolver kind.
  - `core_seed_derivation_chain_raw` returns chain display names ("Bitcoin
    Cash") (before: camel-case keys such as `bitcoinCash`).
- **The CLI reads chain presets from core** (`cli/src/main.rs`,
  `app_core_chain_presets`; before: its own parse of
  `swift/derivation/DerivationPresets.json`)
- **The CLI prompt and section banners are restyled** (`cli/src/main.rs`)
- Tests: 5 new
  - Chain registry 5: an id that does not round-trip, and EVM or Bitcoin-UTXO
    grouping, EVM chain ids or endpoint slot offsets diverging from the tables
    they replace.
- Total: 11 .rs files changed (1 new), +1749 −773 lines; data and config
  files: `core/embedded/AppEndpointDirectory.json`,
  `core/embedded/DerivationPresets.json`,
  `core/embedded/DerivationRequestCompilationPresets.json`; dependencies
  unchanged; `#[test]` functions 282 → 287.

## Beta Commit 52

- **Degraded-chain-sync message rules move to core** (new
  `core/src/diagnostics/degraded.rs`): normalizes a degraded detail (strips the
  "last good sync" and "no prior sync" suffixes, trims), detects live-success
  phrases, and maps a detail to its `localizedStoreFormat` template key; Swift
  keeps the localized formatting.
- **EVM history page projected onto wallets**
  (`core/src/fetch/history_decode.rs`, `plan_evm_transaction_records`): one
  record per wallet and transfer that touches the wallet's normalized address as
  sender or receiver.
- **Refresh targets filtered by chain and non-empty address**
  (`core/src/fetch/mod.rs`, `core_plan_normalized_refresh_targets`)
- **Price and fiat-rate merge policies** (`core/src/fetch/price.rs`,
  `price_merge_live_updates`, `price_merge_fiat_rate_updates`)
  - Fetched prices overwrite existing ones, keys absent from the fetch are kept,
    and the result reports whether any value changed.
  - Fiat rates prefer the fetched value, fall back to the existing one, and drop
    zero rates.
- **Formatting rules** (`core/src/formatting.rs`): fiat amount rules per
  currency code (JPY differs), minimum visible amount for a decimal count,
  USD-stablecoin detection with a fallback USD price, and the dashboard asset
  grouping key.
- **Dashboard token entries** (`core/src/store/mod.rs`
  `plan_dashboard_supported_token_entries`, `store/app_state/token_helpers.rs`
  `normalize_dashboard_contract_address`): contract addresses use the canonical
  Sui/Aptos form, otherwise lowercased; empty input gives none.
- **Embedded preset and endpoint paths follow the app's renames**
  (`swift/derivation/`, `swift/rustbridge/AppEndpointDirectory.json`; also the
  CLI)
- Tests: 16 new
  - Degraded messages 6: a sync suffix left on the detail, live success not
    recognized, an unknown detail given a template.
  - EVM record projection 2: a transfer missing from its wallet, an unrelated
    transfer attributed to a wallet.
  - Refresh targets 1: another chain's or an empty address kept.
  - Price merge 4: an unchanged price reported as a change, missing keys
    dropped, a zero fiat rate kept.
  - Formatting 2: JPY rules applied to other currencies, a wrong minimum visible
    amount at zero decimals.
  - Dashboard contracts 1: Sui/Aptos contracts not canonicalized.
- Docs: `ARCHITECTURE.md` "iOS — Liquid Glass": `DashboardViews.swift` named,
  `AppLocalization.swift` dropped.
- Total: 12 .rs files changed (1 new), +735 −16 lines; data and config files:
  none; dependencies unchanged; `#[test]` functions 266 → 282.

## Beta Commit 51

- **More Swift helpers become typed exports** (`core/src/ffi.rs`;
  `DerivationPathSegment` in `app_core.rs` is now a UniFFI record)
  - Derivation paths: parse, format, normalize with a fallback, read a segment
    value, replace the last two segments.
  - Private-key hex: normalize, and check whether a string is likely one.
  - `core_endpoint_role_mask`: role names to the endpoint role bitmask.
  - `core_evaluate_large_movement`: alerts when the USD change reaches both the
    USD threshold and the percent threshold; never alerts when the previous
    total is 0 or less.
- **Balance fetches run concurrently** (`core/src/derivation/utxo_hd.rs`,
  `core/src/fetch/refresh_engine.rs`)
  - The xpub balance scan fetches up to 5 addresses at once (before: one after
    another).
  - A refresh cycle fetches up to 8 entries at once and reads the observer once
    per cycle (before: one entry at a time).
- **History cache and pagination store keyed by `(chain_id, id)` tuples**
  (`fetch/history_cache.rs`, `fetch/history_store.rs`; before:
  `"{chain_id}:{id}"` strings); behaviour unchanged.
- **The `wallet_db` connection pool uses `parking_lot::Mutex`**
  (`store/wallet_db.rs`); behaviour unchanged.
- Total: 7 .rs files changed, +198 −69 lines; data and config files:
  `Cargo.lock`, `core/Cargo.toml`; dependencies: added parking_lot 0.12
  (`core/Cargo.toml`); `#[test]` functions unchanged at 266.

## Beta Commit 50

- **The CLI becomes an interactive wallet shell with encrypted seed storage**
  (`cli/src/main.rs`; clap and the `addresses` subcommand removed)
  - `spectra` opens a `spectra>` prompt with `import`, `nw`/`newwallet`,
    `si`/`simport [--chain] [--name]`, `ai`/`advimport` (network and derivation
    path), `wi`/`wimport` (watch-only by address), `list`, `delete`, `about`,
    `help`, `quit`.
  - Wallets are stored in `~/.spectra/wallets.json` (`version: 1`, one chain and
    address per wallet); each seed is written to `~/.spectra/secrets/<id>.seed`
    as an AES-GCM envelope under a key derived from the wallet password
    (PBKDF2-HMAC-SHA256, 210,000 rounds, `<id>.salt`), with a password verifier
    in `<id>.password`.
- **Seed envelope encryption and password verifiers in core** (new
  `core/src/store/seed_envelope.rs`, `core/src/store/password_verifier.rs`)
  - AES-256-GCM envelope `{"version":1,"ciphertext","nonce"}` in the Swift
    `SeedMaterialEnvelope` layout; a key that is not 32 bytes is refused.
  - PBKDF2-HMAC-SHA256 verifier `{"version":1,"salt","rounds":210000,"digest"}`
    in the Swift `PasswordVerifierEnvelope` layout, compared in constant time;
    an empty password is refused and surrounding whitespace is trimmed.
- **Wallets hold one chain each** (`core/src/store/state.rs` `WalletSummary`,
  `schema_version` 1 → 2): `selected_chain`, the Bitcoin and Dogecoin network
  modes, `bitcoin_xpub` and the `derivation_paths` map become `chain_name`,
  `network_mode`, `xpub` and `derivation_path`.
- **Legacy wallet-store migration removed** (`core/src/store/migration.rs`
  deleted; `core_migrate_legacy_wallet_store_json`,
  `core_export_legacy_wallet_store_json` gone)
- **`WalletService::execute_send` derives, builds the payload, signs and
  broadcasts in one call** (`core/src/service.rs`,
  `send::SendExecutionRequest`): takes either a seed phrase with chain name and
  path or a private key, routes token sends by contract address, and returns the
  hash and payload format from `classify_send_broadcast_result` (before: Swift
  derived the key and called back into Rust to sign).
- **Typed UniFFI variants alongside the JSON exports** (`service.rs` `*_typed`,
  `new_typed`, `init_wallet_state_direct`, `upsert_wallet_direct`; `app_core.rs`
  `app_core_*` without `_json`; `derivation/runtime.rs` `derivation_derive`,
  `derivation_build_material`, `derivation_derive_all_addresses` and the
  private-key forms; keypool, owned-address and settings records): typed records
  in place of JSON strings; endpoints indexed by chain id.
- **`core_evaluate_high_risk_send_reasons` is typed** (`core/src/ffi.rs`;
  replaces the `_json` form); the `core_reduce_state` export is removed, and so
  are the JSON duplicates of address and identifier validation and endpoint
  ordering and attempt recording (the typed forms remain).
- **`core_validate_wallet_import_draft`** (`core/src/derivation/import.rs`):
  decides whether an import form can proceed; watch-only refuses Monero and more
  than one chain, private-key import needs exactly one supported chain, create
  mode needs a valid seed and a completed backup check.
- **`core_validate_send_balance`** (`core/src/send/flow_helpers.rs`): one
  amount-plus-fee check for native and token sends, with the user-facing
  insufficient-funds or insufficient-fee message.
- **`balance_decoder_*` field accessors exported for Swift**
  (`core/src/fetch/balance_decoder.rs`)
- **Per-call connections and allocations removed; behaviour unchanged**
  - `wallet_db` keeps one SQLite connection per database path
    (`core/src/store/wallet_db.rs`; before: one opened per call).
  - `reduce_state` mutates in place (`store/state.rs`
    `reduce_state_in_place`).
  - Localization lookup, the balance cache and the token catalog avoid
    per-call allocation (`localization.rs`, `fetch/balance_cache.rs`,
    `tokens.rs`).
- Tests: 11 new, 1 removed
  - Send balance 5: a send allowed when amount or fee exceeds the balance, a
    token send allowed without native balance for the fee.
  - Password verifier 3: a correct password rejected, whitespace changing the
    result, an empty password accepted.
  - Seed envelope 3: decryption with the wrong key succeeding, a short key
    accepted.
  - Removed: the legacy wallet-store round trip, with the migration.
- Total: 41 .rs files changed (2 new, 1 deleted), +3861 −1658 lines; data and
  config files: `Cargo.lock`, `cli/Cargo.toml`, `core/Cargo.toml`;
  dependencies: added aes-gcm 0.10, removed once_cell (`core/Cargo.toml`);
  added bip39 2, colored 2, dialoguer 0.11, dirs 5, getrandom 0.2, pbkdf2
  0.12, rpassword 7, rusqlite 0.31, serde 1, sha2 0.10, uuid 1, removed clap
  (`cli/Cargo.toml`); `#[test]` functions 256 → 266.

## Beta Commit 49

- **Static resources embedded from `core/embedded/`** (`core/src/resources.rs`;
  before: `Resources/` in the app tree)
- **Localization tables embedded from `resources/strings/<locale>/`**
  (`core/src/localization.rs`; before: `Resources/Localization/`)
- **Unused helpers removed** (`derivation/bitcoin_primitives.rs`,
  `runtime.rs`: `double_sha256`, xprv version constants, the unused witness
  version and version byte in parsed addresses); behaviour unchanged.
- Total: 4 .rs files changed, +39 −61 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions unchanged at 256.

## Beta Commit 48

- **Copies of the embedded resources added under `core/embedded/`**:
  `BIP39EnglishWordList.txt` (2048 words), `BuyCryptoProviders.json`,
  `ChainVisualRegistry.json`, `TokenVisualRegistry.json`; not yet referenced by
  code.
- Total: no .rs files changed; data and config files:
  `core/embedded/BIP39EnglishWordList.txt`,
  `core/embedded/BuyCryptoProviders.json`,
  `core/embedded/ChainVisualRegistry.json`,
  `core/embedded/TokenVisualRegistry.json`; dependencies unchanged; `#[test]`
  functions unchanged at 256.

## Beta Commit 47

- **Hand-rolled encoders replaced by crates**
  (`core/src/derivation/bitcoin_primitives.rs`, `runtime.rs`): Base58Check
  through `bs58` `with_check`, CRC16-XMODEM through `crc`, RFC 4648 Base32
  through `data-encoding`; outputs unchanged.
- **`chains/bitcoin_hd.rs` moved to `derivation/utxo_hd.rs`**
- **`core/Cargo.toml` regrouped by derivation stage** with a comment per
  dependency.
- **Deep UTXO discovery also covers Bitcoin and Dogecoin**
  (`core_supports_deep_utxo_discovery`; before: Bitcoin Cash, Bitcoin SV,
  Litecoin).
- **Bitcoin SV history is no longer capped at 25 entries**
  (`core/src/chains/bitcoin_sv.rs`)
- **Litecoin retry profiles removed** (`core/src/fetch/http.rs`, `http_ffi.rs`):
  Litecoin reads use `ChainRead` (before: `LitecoinRead`, 4 attempts with
  gentler backoff).
- Total: 11 .rs files changed, +24 −88 lines; data and config files:
  `Cargo.lock`, `core/Cargo.toml`; dependencies: added crc 3, data-encoding 2
  (`core/Cargo.toml`); `#[test]` functions unchanged at 256.

## Beta Commit 46

- **Chain-native derivation schemes** (`core/src/derivation/runtime.rs`;
  derivation algorithms 3–7, curve `SR25519` 2, chain `MONERO` 22)
  - NEAR direct seed: the first 32 bytes of the BIP-39 seed are the ed25519 key;
    the path is ignored.
  - TON mnemonic: HMAC-SHA512 entropy, then PBKDF2 with salt "TON default seed"
    and 100,000 rounds; salt prefix and iteration count still apply.
  - Cardano Icarus (BIP32-Ed25519) with Shelley enterprise addresses.
  - Substrate sr25519 from the BIP-39 mini-secret through `schnorrkel`, with
    SS58 addresses; a path with junctions is refused.
  - Monero from the reduced BIP-39 seed, view key = Keccak of the spend key,
    standard main address.
  - TON v4R2 wallet address from the embedded v4R2 code cell and default wallet
    id 698983191.
- **Per-chain address algorithms; chain inferred from the address algorithm**
  (`ADDRESS_*` 4–19, `chain_from_address_algorithm`): the request's chain id is
  deprecated; when omitted the chain follows from the address algorithm, an
  explicit chain wins, and `Auto` is refused.
- **Derivation no longer uses the `bitcoin` crate** (new
  `core/src/derivation/bitcoin_primitives.rs`; `chains/bitcoin_hd.rs`,
  `derivation/addressing.rs`): hand-written BIP-32 CKDpriv/CKDpub, xpub/xprv
  serialization with y/zpub version swapping, Hash160, Base58Check and P2PKH,
  P2SH-P2WPKH, P2WPKH and P2TR encoders; Bitcoin signing in `chains/bitcoin.rs`
  still uses the crate.
- Tests: 30 new
  - Golden vectors 2: addresses diverging from BIP-84, BIP-86 and
    trezor-firmware fixtures (Bitcoin, Ethereum, Cardano, Solana, Stellar, Tron,
    XRP), and EVM replica chains diverging from Ethereum.
  - NEAR 2, TON mnemonic 3, Cardano Icarus 4, Substrate 5, Monero 5, TON v4R2 4:
    a scheme collapsing into plain SLIP-0010 or BIP-39, a passphrase or
    iteration count with no effect, a malformed address.
  - Checksums 1: a wrong CRC16-XMODEM.
  - Chain inference 4: an address algorithm with no chain, `Auto` accepted, an
    explicit chain overridden.
- Docs: `core/DERIVATION_WORKFLOW.md` new section "Derivation Variables —
  Complete Inventory".
- Total: 5 .rs files changed (1 new), +2586 −271 lines; data and config files:
  `Cargo.lock`, `core/Cargo.toml`; dependencies: added ripemd 0.1, schnorrkel
  0.11 (`core/Cargo.toml`); `#[test]` functions 226 → 256.

## Beta Commit 45

- **Every BIP-39 and master-key constant is caller-tunable**
  (`core/src/derivation/runtime.rs`; full description in
  `core/DERIVATION_WORKFLOW.md`)
  - Any HMAC master key for BIP-32 and SLIP-0010 (before: only "Bitcoin seed"
    and "ed25519 seed" accepted); the master node and the SLIP-0010 walk are
    hand-written in place of `Xpriv::new_master` and the `slip10` crate.
  - All 10 BIP-39 wordlists (`bip39` `all-languages`; before: English only); an
    unknown wordlist is refused.
  - New `salt_prefix` (default "mnemonic") in the PBKDF2 salt.
- **`SecretStore` callbacks are classed and return errors**
  (`core/src/store/secret_store.rs`): each call carries a `SecretClass` (`Seed`,
  `PrivateKey`, `Generic`) and returns `SecretStoreError` (`NotFound`,
  `Unavailable`, `Backend`) (before: `Option`/`bool`); delete is idempotent.
- **`WalletService` pass-through secret methods removed** (`load_secret`,
  `save_secret`, `delete_secret`, `list_secret_keys`); only Rust calls the
  registered store.
- **Async exports run on tokio** (`#[uniffi::export(async_runtime = "tokio")]`
  in `service.rs`, `http_ffi.rs`, `refresh_engine.rs`, `app_state/events.rs`)
- Tests: 6 new
  - Derivation knobs 6: a custom HMAC key, salt prefix or iteration count with
    no effect, the default HMAC key not matching the standard seed, an unknown
    wordlist accepted.
- Docs: `core/DERIVATION_WORKFLOW.md` (new): entry points, caller-tunable
  fields, stages from mnemonic to address; `core/README.md` deleted.
- Total: 6 .rs files changed, +334 −127 lines; data and config files:
  `Cargo.lock`, `core/Cargo.toml`; dependencies: added hmac 0.12, removed
  slip10 (`core/Cargo.toml`); `#[test]` functions 220 → 226.

## Beta Commit 44

- **`.gitignore` ignores cloud-sync placeholder files** (`.!*!*`)
- Total: no .rs files changed; data and config files: `.gitignore`;
  dependencies unchanged; `#[test]` functions unchanged at 220.

## Beta Commit 43

- **`Core/` renamed `core/` and modules grouped into `derivation/`, `fetch/`,
  `send/` and `store/`** (e.g. `main.rs` → `derivation/runtime.rs`,
  `ethereum_send.rs` → `send/ethereum.rs`, `send_machine.rs` →
  `send/machine.rs`, `wallet_db.rs` → `store/wallet_db.rs`); behaviour
  unchanged.
- **Embedded preset and endpoint paths follow the app's `swift/` folder**
  (`core/src/app_core.rs`, `catalog.rs`)
- Docs: `Core/README.md` moved to `core/README.md`.
- Total: 83 .rs files changed (1 new), +73 −52 lines; data and config files:
  `core/Cargo.lock`, `core/Cargo.toml`, `core/tokens.toml`; dependencies: the 29
  entries of `Core/Cargo.toml` moved unchanged to `core/Cargo.toml`; `#[test]`
  functions unchanged at 220.

## Beta Commit 42

- **New `spectra` CLI binary** (new crate `cli/`, `spectra-cli`; workspace
  member): `spectra addresses "<seed>" [--chains …]` prints derived addresses
  through `derivation_derive_all_addresses_json`, using fixed paths for 13
  chains and Bitcoin, Ethereum and Solana by default; an unknown chain filter
  exits with code 2.
- **Embedded resources and localization tables read from the app's top-level
  `Resources/`** (`Core/src/resources.rs`, `localization.rs`; before:
  `Swift/Resources/`)
- Docs: `ARCHITECTURE.md` "Summary judgement on the workspace model" and "What
  this is NOT" reworded without the rust-bitcoin comparison.
- Total: 3 .rs files changed (1 new), +138 −32 lines; data and config files:
  `Cargo.lock`, `Cargo.toml`, `cli/Cargo.toml`; dependencies: added clap 4,
  serde_json 1, spectra_core (path) (`cli/Cargo.toml`); `#[test]` functions
  unchanged at 220.

## Beta Commit 41

- **AppState event bus** (new `Core/src/app_state/events.rs`): every write to
  the wallets, transactions and address-book collections publishes a typed
  event; a registered Swift `AppStateObserver` receives them through a pump
  task, and unregistering stops it.
- **Per-chain diagnostics JSON builders** (new
  `Core/src/diagnostics/export.rs`): the export-bundle shapes from Swift
  `StoreDiagnosticsExport.swift` (EVM, UTXO with optional network mode, Tron
  send errors, Solana RPC count, simple-address chains).
- **HTTP for Swift call sites** (`Core/src/http_ffi.rs`): `http_get` and
  `http_post_json` single-shot text wrappers, and `diagnostics_probe_jsonrpc`
  doing transport and parse in one call.
- **Shared HTTP client settings** (`Core/src/http.rs`)
  - HTTPS is no longer enforced by the client (before: `https_only`).
  - Request timeout 20 s → 30 s, with a 10 s connect timeout.
  - User agent `spectra-core/<version>` (before: `Spectra/1.0`).
- **EVM send preflight and fee bump** (`Core/src/send_flow_helpers.rs`):
  contract-recipient and missing-token detection, probe-failure reporting,
  replacement fee bump with defaults and a floor, insufficient-funds and
  insufficient-fee messages.
- **`.gitignore` ignores `.claude/`**
- Tests: 23 new
  - Event bus 2: an event kind not delivered, the pump running after unregister.
  - Diagnostics export 6: a field missing from or wrongly present in a chain's
    JSON.
  - HTTP 6 (against a `wiremock` server): body, headers or 4xx responses lost,
    POST without its body or content type, a closed port not reported, a
    JSON-RPC error body counted as reachable.
  - EVM preflight and fees 9: a contract recipient or missing token not
    reported, a bump below the floor, wrong insufficient-funds wording.
- Docs: `temp.md` deleted.
- Total: 8 .rs files changed (2 new), +1175 −19 lines; data and config files:
  `.gitignore`, `Cargo.lock`, `Core/Cargo.toml`; dependencies: added wiremock
  0.6 as a dev-dependency (`Core/Cargo.toml`); `#[test]` functions 197 → 220.

## Beta Commit 40

- **App-state logic lifted from Swift `AppState` into core** (27 new files under
  `Core/src/`; plan in `temp.md`)
  - Canonical wallets, transactions and address-book collections
    (`app_state/store.rs`), derived caches (`app_state/caches.rs`), scalar shell
    state (`app_shell_state.rs`), token and endpoint normalization
    (`app_state/token_helpers.rs`).
  - EVM send: destination and amount validation, native or ERC-20 request
    assembly, preview decoding with custom fee and nonce overrides
    (`ethereum_send.rs`).
  - Send previews of the 17 non-EVM chains decoded from fetched JSON
    (`send_preview_decode.rs`); per-chain send payloads with decimal-to-raw-unit
    conversion and broadcast-result classification (`send_payload.rs`).
  - Per-chain send-preview record types (`wallet_core.rs`).
  - Send helpers: ENS-name detection, Ethereum and Tron error messages,
    address-book validation, EVM chain context, rebroadcast dispatch, risk-probe
    messages (`send_flow.rs`, `send_flow_helpers.rs`); broadcast verification
    notices (`send_verification.rs`).
  - Receive-address messages per chain, wallet display names and private-key
    import support (`receive.rs`); amount input as a positive decimal with a
    maximum fractional digit count (`amount_input.rs`).
  - Diagnostics: record types matching the Swift export bundle, a per-wallet
    registry per chain, aggregation of probe and history JSON (`diagnostics/`);
    redaction of hex private keys, extended private keys and seed-word runs
    (`diagnostics_sanitizer.rs`).
  - History and balance decoders (`history_decode.rs`, `balance_decoder.rs`),
    asset-decimal and status formatting (`formatting.rs`), persisted-model and
    domain types matching the Swift `Codable` shapes (`persistence/models.rs`,
    `wallet_domain.rs`).
- **HTTP for Swift** (new `Core/src/http_ffi.rs`): `http_request` with retry
  profiles and `http_probe` single-shot reachability, replacing Swift's
  URLSession retry stack.
- **Chain self-tests** (new `Core/src/self_tests.rs`,
  `self_tests_run_chain_json`, `self_tests_run_all_json`): per chain, address
  accept and reject cases and derivation checks.
- **JSON exports added for Swift call sites** (`Core/src/ffi.rs`)
  - Address and identifier validation, endpoint reliability ordering and
    attempt recording, as JSON twins of the typed exports.
  - `core_encode_history_records_json`, and encode/decode of persisted
    price-alert, address-book and transaction records.
- **History and transaction types renamed with a `Core` prefix**
  (`CoreNormalizedHistoryEntry`, `CoreBitcoinHistorySnapshot`, …)
- Tests: 137 new
  - Receive 15: wrong receive message per chain and seed or watch-only state,
    wrong wallet names.
  - Send helpers 16: unmapped Ethereum or Tron errors, wrong EVM context or
    chain titles, rebroadcast to the wrong chain, ENS false positives.
  - Send previews and payloads 17: a preview field misread, max sendable not
    computed, wrong raw units, payload JSON not escaped.
  - EVM send 12: wrong native or ERC-20 assembly, an invalid destination
    accepted, overrides lost.
  - Send verification 9: a stale or missing notice, a failure without the
    warning flag.
  - History decode 11, balance decoder 6: wrong amounts from wei or raw units,
    Dogecoin amounts not netted, NEAR history misparsed.
  - Diagnostics 18: miscounted or misclassified probe and history results,
    registry buckets mixing chains, export shapes not round-tripping, a secret
    not redacted.
  - App state 14: collections or caches not round-tripping or not cleared, token
    and endpoint normalization errors.
  - Amount input 6, formatting 5, persistence and domain 6, self-tests 2:
    invalid amounts accepted, wrong decimals, Swift field names not
    round-tripping, a chain without self-tests.
- Docs: `temp.md` (new): Phase 5 inventory of Swift `Shell/` files and which
  move to Rust.
- Total: 33 .rs files changed (27 new), +7932 −52 lines; data and config
  files: `Cargo.lock`, `Core/Cargo.toml`; dependencies: removed async-trait,
  borsh, chrono, elliptic-curve, getrandom, hmac, ripemd, secrecy, uuid
  (`Core/Cargo.toml`); `#[test]` functions 60 → 197.

## Beta Commit 39

- **Embedded preset and endpoint JSON paths follow the app's moves**
  (`Swift/Derivation/DerivationPresets.json`,
  `Swift/ProviderCatalog/AppEndpointDirectory.json`); behaviour unchanged.
- Total: 2 .rs files changed, +5 −5 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions unchanged at 60.

## Beta Commit 38

- **`Core/src/core/*` flattened into `Core/src/`** (`lib.rs` declares each
  module; `crate::core::` paths become `crate::`); behaviour unchanged.
- **The nested `Core/tools/uniffi-bindgen/` removed** in favour of the
  workspace one.
- Total: 53 .rs files changed (2 deleted), +140 −145 lines; data and config
  files: `Core/tools/uniffi-bindgen/Cargo.lock`,
  `Core/tools/uniffi-bindgen/Cargo.toml`; dependencies: removed uniffi
  (`Core/tools/uniffi-bindgen/Cargo.toml`); `#[test]` functions unchanged at
  60.

## Beta Commit 37

- **Cargo workspace with a separate FFI crate** (root `Cargo.toml` with members
  `core`, `ffi`, `tools/uniffi-bindgen`; new `ffi/` crate `spectra_core_ffi`,
  library `spectra_core`, staticlib and cdylib, re-exporting core;
  `tools/uniffi-bindgen/`)
  - `Core/Cargo.toml` no longer builds static or dynamic libraries, and the
    release profile (LTO, one codegen unit, stripped) moves to the workspace.
  - New root `Makefile`: `ios`, `android`, `bindgen-ios`, `bindgen-android`
    (calling `scripts/*.sh`), `test` (`cargo test -p spectra_core`), `check`,
    `clean`.
- **Planning exports become typed** (`Core/src/core/ffi.rs`): each
  `core_*_json(String) -> String` becomes `core_*` taking and returning
  `uniffi::Record` types, with `usize` fields as `u64`; persisted state and
  high-risk send evaluation stay JSON.
- **Embedded catalog, resource and localization paths follow the app's move
  into `Swift/`** (`Core/src/app_core.rs`, `core/catalog.rs`,
  `core/resources.rs`, `core/localization.rs`)
- Docs: `ARCHITECTURE.md` (new): target three-crate workspace, native iOS
  (Liquid Glass) and Android (Material You) apps over one Rust core, planned
  moves (`Core/` → `core/`, Swift into `ios/`, `android/` scaffold, `scripts/`);
  `filesystem.md` deleted.
- Total: 21 .rs files changed (2 new), +406 −364 lines; data and config files:
  `tools/uniffi-bindgen/` (2), `Cargo.lock`, `Cargo.toml`, `Core/` (1),
  `Makefile`, `ffi/` (1); dependencies: added spectra_core (path), uniffi 0.29
  (`ffi/Cargo.toml`); uniffi 0.29 (`tools/uniffi-bindgen/Cargo.toml`);
  `#[test]` functions unchanged at 60.

## Beta Commit 36

- **`WalletService::fetch_normalized_history_json`**
  (`Core/src/core/service.rs`, `history::normalize_chain_history`): fetches and
  normalizes history into `ChainHistoryEntry` rows for the 17 non-EVM chains;
  EVM chains keep the paged `fetch_evm_history_page`.
- **`WalletService::upsert_asset_holdings`**: holdings match by chain and
  contract address for tokens, or chain and symbol for native coins, and the
  whole holding is replaced (before: matched by symbol and chain, only the
  amount updated).
- **NEAR token sends route to the NEAR flow** (`Core/src/core/send.rs`,
  `supports_near_token_send`)
- Total: 3 .rs files changed, +429 −7 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions unchanged at 60.

## Beta Commit 35

- **Crate renamed `spectra_derivation` → `spectra_core`** (`Core/Cargo.toml`,
  package and library)
- **High-risk send warnings** (`core_evaluate_high_risk_send_reasons_json` in
  `Core/src/core/ffi.rs`): codes `invalid_format`, `new_address`,
  `ens_resolved`, `large_send`, `chain_mismatch`, `eth_on_utxo`,
  `non_evm_on_evm`, `non_solana`, `non_tron`, `non_xrp`, `non_monero`, from the
  destination, address book, past transaction addresses and amounts.
- **`WalletService::fetch_simple_chain_send_preview`**: fee, balance and max
  sendable for Solana, XRP, Stellar, Cardano, Polkadot, Sui, Aptos, TON, NEAR,
  ICP and Monero.
- **Transaction history persisted in SQLite** (`Core/src/core/wallet_db.rs`, new
  `history_records` table; `upsert_history_records`,
  `fetch_all_history_records`, `delete_history_records`,
  `replace_all_history_records`, `delete_history_records_for_wallet`,
  `clear_all_history_records`): rows carry id, wallet, chain, hash, creation
  time and the Swift record as a base64 payload.
- Total: 3 .rs files changed, +565 −1 lines; data and config files:
  `Core/Cargo.lock`, `Core/Cargo.toml`; dependencies unchanged; `#[test]`
  functions unchanged at 60.

## Beta Commit 33

- **Generated Swift bindings removed from the core tree**
  (`Core/Derivation/Core/Generated/` deleted)
- **`WalletService` holds the wallet list and native holdings**
  (`Core/src/core/service.rs`): `init_wallet_state`, `list_wallets_json`,
  `upsert_wallet_json`, `remove_wallet_json`; `update_native_balance` decodes a
  chain's balance JSON into the native holding, `set_native_balance` sets it
  directly.
- Total: 1 .rs file changed, +207 −0 lines; data and config files:
  `Core/Derivation/Core/Generated/module.modulemap`,
  `Core/Derivation/Core/Generated/spectra_derivation.swift`,
  `Core/Derivation/Core/Generated/spectra_derivationFFI.h`,
  `Core/Derivation/Core/Generated/spectra_derivationFFI.modulemap`;
  dependencies unchanged; `#[test]` functions unchanged at 60.

## Beta Commit 32

- **Rust-driven balance refresh** (new `Core/src/core/refresh_engine.rs`
  `BalanceRefreshEngine`, `balance_observer.rs` `BalanceObserver`): Swift
  registers an observer and the entries, Rust runs the timer loop and reports
  each balance and each completed cycle; `fetch_balance_auto` accepts an
  xpub/ypub/zpub as the address.
- **History pagination state in core** (new `Core/src/core/history_store.rs`;
  `WalletService` cursor, page, exhausted and reset methods): per chain and
  wallet, replacing the Swift cursor variables.
- **Keypool and owned addresses in SQLite** (new `Core/src/core/wallet_db.rs`,
  tables `wallet_keypool` and `wallet_owned_addresses`; save, load and delete
  methods on `WalletService`): replace four Swift UserDefaults snapshots.
- **`derivation_derive_all_addresses_json`** (`Core/src/main.rs`): derives one
  address per chain from a chain-name → path map with each chain's default
  algorithms; the Bitcoin script type follows the path purpose (44, 49, 84, 86;
  P2PKH otherwise); an unknown chain or failed derivation gives `null`.
- Tests: 12 new
  - Pagination 6: a cursor or page not advancing, exhaustion not set, a reset
    clearing other wallets or chains.
  - Wallet database 6: keypool or address rows not round-tripping, upsert
    duplicating, wallet deletion leaving rows.
- Total: 7 .rs files changed (4 new), +1503 −0 lines; data and config files:
  none; dependencies unchanged; `#[test]` functions 48 → 60.

## Beta Commit 31

- **Send flow state machine** (new `Core/src/core/send_machine.rs`,
  `SendStateMachine`): Swift applies events (asset, address, amount, fee
  request, confirm, result) and gets the new state and the effects to run, from
  idle through fee preview and confirmation to submitting.
- **Keychain access through a callback** (new `Core/src/core/secret_store.rs`
  `SecretStore`; `WalletService::set_secret_store`, `load_secret`,
  `save_secret`, `delete_secret`, `list_secret_keys`)
- **Balance and history caches** (new `balance_cache.rs`, 30 s TTL;
  `history_cache.rs`, 5 min TTL; `fetch_balance_cached`, `fetch_history_cached`
  and cache get, set and invalidate methods)
- **Wallet snapshot and app settings persisted** (`save_wallet_snapshot`,
  `load_wallet_snapshot` under key `wallets.snapshot.v1`; `save_app_settings`,
  `load_app_settings`)
- **Token balances for Aptos and Sui coin types and TON jettons**
  (`chains/aptos.rs`, `sui.rs` `fetch_coin_balance`; `chains/ton.rs`
  `fetch_jetton_balances` through TonCenter v3)
- Total: 9 .rs files changed (4 new), +959 −0 lines; data and config files:
  none; dependencies unchanged; `#[test]` functions unchanged at 48.

## Beta Commit 30

- **Built-in token registry in `Core/tokens.toml`** (84 tokens over 12 chains,
  Ethereum 28 and Solana 15 the largest; new `Core/src/core/tokens.rs`,
  `list_builtin_tokens_json`, `WalletService::list_builtin_tokens`):
  `BuiltInTokenRegistry.json` is no longer embedded.
- **Unified native-and-token history** (`chains/solana.rs` SOL and SPL;
  `chains/tron.rs` TRX and TRC-20 from TronScan)
- **Balances for a list of tokens in one call**
  (`WalletService::fetch_token_balances`): TRC-20 and NEP-141 fetched in
  parallel, SPL in one batch (`fetch_spl_balances`).
- **Transaction status for UTXO chains** (`fetch_tx_status` in Bitcoin, Bitcoin
  Cash, Bitcoin SV, Dogecoin, Litecoin; `WalletService::fetch_utxo_tx_status`)
- **Generated Swift bindings committed** under `Core/Derivation/Core/Generated/`
  (`spectra_derivation.swift`, FFI header, module maps).
- Total: 12 .rs files changed (1 new), +961 −20 lines; data and config files:
  `Core/Derivation/` (4), `Core/` (3); dependencies: added futures 0.3, toml
  0.8 (`Core/Cargo.toml`); `#[test]` functions unchanged at 48.

## Beta Commit 29

- **Token transfers** (`Core/src/core/chains/`;
  `WalletService::fetch_token_balance`, `sign_and_send_token`): ERC-20 on EVM
  chains, SPL on Solana (associated token accounts, `TransferChecked`), TRC-20
  on Tron, NEP-141 on NEAR, issued assets on Stellar.
- **EVM additions** (`chains/evm.rs`): fee-bumped replacement sends with
  overrides, ENS resolution, receipts, contract code, transaction nonce, token
  metadata and transfer history, paged history, batch token balances and send
  preview.
- **Bitcoin HD wallets from an extended public key** (new
  `chains/bitcoin_hd.rs`; `derive_bitcoin_account_xpub`,
  `derive_bitcoin_hd_addresses`, `fetch_bitcoin_xpub_balance`,
  `fetch_bitcoin_next_unused_address`): xpub, ypub and zpub, receive and change
  legs, balance summed over Esplora.
- **Bitcoin SV client** (new `chains/bitcoin_sv.rs`, WhatsOnChain; service chain
  id 22, BNB Chain 23); fee rates and history for Bitcoin Cash and Litecoin.
- **Prices and fiat rates** (new `Core/src/core/price.rs`; `fetch_prices`,
  `fetch_fiat_rates`): providers CoinGecko, Binance, Coinbase Exchange,
  CoinPaprika, CoinLore, and fiat providers `OpenER`, `ExchangeRateHost`,
  `Frankfurter`, `FawazAhmed` (ported from Swift `LivePriceService` and
  `FiatRateService`).
- **SQLite state blobs** (`WalletService::load_state`, `save_state`: a JSON
  value per key in a database file).
- **Mnemonic helpers** (`generate_mnemonic` for 12–24 words, 12 by default;
  `validate_mnemonic`; `bip39_english_wordlist`)
- **Raw broadcast and send previews on `WalletService`** (`broadcast_raw`,
  `fetch_utxo_fee_preview`, `fetch_evm_send_preview`,
  `fetch_tron_send_preview`)
- Total: 23 .rs files changed (3 new), +5321 −84 lines; data and config files:
  `Core/Cargo.lock`, `Core/Cargo.toml`; dependencies: added rusqlite 0.31
  (`Core/Cargo.toml`); `#[test]` functions unchanged at 48.

## Beta Commit 28

- **`WalletService` fetches, estimates fees, signs and broadcasts for 22
  chains** (new `Core/src/core/service.rs`, a UniFFI object; per-chain clients
  in new `Core/src/core/chains/`)
  - Methods `new(endpoints_json)`, `update_endpoints`, `fetch_balance`,
    `fetch_history`, `fetch_fee_estimate`, `sign_and_send`, returning JSON;
    async on tokio.
  - Its own frozen chain-id table (0 Bitcoin … 21 Ethereum Classic, with Monero
    19 and Base 20), distinct from the derivation ids.
  - The service holds no secrets: the private key is passed in per call.
  - Bitcoin through Esplora (P2PKH, P2SH-P2WPKH, P2WPKH, P2TR); Litecoin,
    Dogecoin and Bitcoin Cash (`SIGHASH_FORKID`) through Blockbook; EVM EIP-1559
    over JSON-RPC; Solana, XRP, Stellar (Horizon, XDR), Tron, Sui, Aptos (BCS),
    NEAR (Borsh), TON (toncenter v2, wallet v4R2), ICP (Rosetta).
  - Cardano through Blockfrost with a minimal CBOR encoder for ADA-only
    transfers; Polkadot through Subscan and RPC, signed with ed25519 instead of
    sr25519; Monero through `monero-wallet-rpc` `transfer`.
- **Shared HTTP client with retry profiles** (new `Core/src/core/http.rs`): one
  `reqwest` client, retry and backoff per profile, with fallback across a
  chain's endpoints.
- **Owned-address, receive and self-send planning** (`Core/src/core/store.rs`;
  `core_aggregate_owned_addresses_json`, `core_plan_receive_selection_json`,
  `core_plan_self_send_confirmation_json`)
- Tests: 3 new
  - Store planning 3: owned addresses duplicated or reordered, a non-native
    receive holding preferred, a pending self-send confirmation not consumed.
- Total: 24 .rs files changed (20 new), +8216 −6 lines; data and config files:
  `Core/Cargo.lock`, `Core/Cargo.toml`; dependencies: added async-trait 0.1,
  base64 0.22, blake2 0.10, borsh 1, chrono 0.4, getrandom 0.2, once_cell 1,
  rand 0.8, reqwest 0.12, ripemd 0.1, sha3 0.10, tokio 1, uuid 1
  (`Core/Cargo.toml`); `#[test]` functions 45 → 48.

## Beta Commit 27

- **The C ABI is removed; Swift calls core only through UniFFI**
  (`Core/include/spectra_derivation.h` deleted; every `extern "C"` function and
  response struct removed from `main.rs`, `app_core.rs`, `core/ffi.rs`)
- **Signing material export** (`derivation_build_material_json`,
  `derivation_build_material_from_private_key_json` in `Core/src/main.rs`):
  address, private key hex, path, and account, branch and index parsed from the
  path.
- **Address and identifier validation** (new `Core/src/core/addressing.rs`;
  `core_validate_address_json`, `core_validate_string_identifier_json`):
  per-chain validators and normalization for 18 address kinds (the EVM chains
  share one), Monero included; Aptos token types validated.
- **Endpoint reliability ordering** (new
  `Core/src/core/endpoint_reliability.rs`;
  `core_order_endpoints_by_reliability_json`,
  `core_record_endpoint_attempt_json`): endpoints sorted by success ratio, then
  name.
- **More planning exports** (`core_plan_balance_refresh_health_json`,
  `core_merge_bitcoin_history_snapshots_json`,
  `core_plan_store_derived_state_json`): refresh health for partial results,
  Bitcoin snapshots merged into net entries, portfolio grouping.
- Tests: 9 new
  - Addressing 4: EVM or Aptos addresses not normalized, an invalid NEAR address
    or Aptos token type accepted.
  - Endpoint reliability 2: wrong ordering, an attempt not counted.
  - Planning 3: partial refresh results misreported, snapshots not netted,
    unstable grouping.
- Docs: `Core/README.md` rewritten as "Rust Core": UniFFI boundary, generated
  bindings under `Derivation/Core/Generated/`, the Swift bridge adapters,
  ownership split.
- Total: 11 .rs files changed (2 new), +1331 −1185 lines; data and config
  files: `Core/include/spectra_derivation.h`; dependencies unchanged;
  `#[test]` functions 36 → 45.

## Beta Commit 26

- **UniFFI 0.29 exports beside the C ABI** (`Core/src/lib.rs`
  `uniffi::setup_scaffolding!`, `SpectraBridgeError`; `Core/src/app_core.rs`,
  `Core/src/core/ffi.rs`)
  - Each `spectra_app_core_*_json` and `spectra_core_*_json` C function gets a
    `#[uniffi::export]` counterpart returning `Result<String,
    SpectraBridgeError>`.
  - `derivation_derive_json` and `derivation_derive_from_private_key_json` take
    the derivation request as JSON, seed phrase included; zero or unknown output
    flags are refused.
- **The crate builds as staticlib and cdylib** (`Core/Cargo.toml`; before:
  staticlib only)
- **New `Core/tools/uniffi-bindgen/` binary** runs `uniffi_bindgen_main`.
- Docs: `filesystem.md` (new, one line naming `Core`).
- Total: 5 .rs files changed (1 new), +560 −8 lines; data and config files:
  `Core/Cargo.lock`, `Core/Cargo.toml`,
  `Core/tools/uniffi-bindgen/Cargo.lock`,
  `Core/tools/uniffi-bindgen/Cargo.toml`; dependencies: added thiserror 2,
  uniffi 0.29 (`Core/Cargo.toml`); uniffi 0.29
  (`Core/tools/uniffi-bindgen/Cargo.toml`); `#[test]` functions unchanged at
  36.

## Beta Commit 25

- **Localization and static resource paths follow the app's move of
  `Views/Resources/` to `Resources/`** (`Core/src/core/localization.rs`,
  `resources.rs`); behaviour unchanged.
- Total: 2 .rs files changed, +33 −33 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions unchanged at 36.

## Beta Commit 24

- **The Rust crate moves from `Spectra/Derivation/Rust/` to `Core/`**; embedded
  catalog paths updated.
- **Embedded static resources** (new `Core/src/core/resources.rs`;
  `spectra_core_static_resource_json`,
  `spectra_core_static_text_resource_utf8`): token and chain visual registries,
  buy-crypto providers, built-in token registry, BIP-39 English wordlist.
- **Persisted snapshot and secret index** (`Core/src/core/store.rs`;
  `spectra_core_build_persisted_snapshot_json`,
  `spectra_core_wallet_secret_index_json`): which wallets have signing material,
  a private key or a password; a bare core-state JSON loads as a snapshot with
  no secrets.
- **Refresh target planning** (`Core/src/core/fetch.rs`; EVM targets grouped by
  normalized address, Dogecoin targets, wallet balance refresh needing an xpub
  or EVM address)
- **Send planning** (new `transfer.rs`, `send.rs`; transfer availability, asset
  routing to a chain's preview and submit, preview routing, submit preflight): a
  live chain is not sendable without signing material; preflight refuses a
  missing wallet, asset, destination or valid amount, or an amount above the
  balance, but allows a zero native EVM amount.
- **UTXO fee preview and coin selection** (new `Core/src/core/utxo.rs`;
  `spectra_core_plan_utxo_preview_json`, `spectra_core_plan_utxo_spend_json`):
  sat/vB and per-kB fee models with relay minimum and absolute minimum fee,
  fewest inputs with change when above dust; refuses an amount below dust, a fee
  below relay policy, an oversized transaction and insufficient funds.
- Tests: 21 new
  - Refresh targets 5: EVM targets not grouped or lost in load-more mode,
    ineligible Dogecoin wallets kept, a refresh planned without an xpub or EVM
    address.
  - Resources 2: an embedded resource missing.
  - Send routing and preflight 4: wrong routing for EVM native or Solana assets,
    a zero amount accepted on non-EVM chains.
  - Snapshot and secret index 3: wallets missing from the secret catalog or
    index, older state not upgraded.
  - Transfer availability 2: a holding sendable without signing material.
  - UTXO 5: fees below the minimum, more inputs than needed, a dust target
    accepted, insufficient funds not reported.
- Docs: `Core/README.md` (moved from `Spectra/Derivation/Rust/README.md`).
- Total: 23 .rs files changed (6 new, 3 deleted), +1963 −154 lines; data and
  config files: `Core/Cargo.lock`, `Core/Cargo.toml`,
  `Core/include/spectra_derivation.h`; dependencies: the 19 entries of
  `Spectra/Derivation/Rust/Cargo.toml` moved unchanged to `Core/Cargo.toml`;
  `#[test]` functions 15 → 36.

## Beta Commit 23

- **App-core catalog queries over the C ABI** (new `src/app_core.rs`,
  `spectra_app_core_*_json`, `SpectraJsonResponse`): chain presets and
  request-compilation presets, derivation path resolution and per-account seed
  paths, endpoint records by id, chain and role bitmask, grouped settings
  entries, diagnostics checks, explorer entries, Bitcoin Esplora and EVM RPC
  endpoints, broadcast providers, chain backends and descriptors; embedded from
  the app's `Catalog/DerivationPresets.json`,
  `Catalog/DerivationRequestCompilationPresets.json` and
  `ProviderCatalog/Registry/AppEndpointDirectory.json`.
- **Core domain module** (new `src/core/`, `spectra_core_*_json` in
  `core/ffi.rs`)
  - Bootstrap with capabilities, chains and locales (`catalog.rs`, `types.rs`).
  - Localization documents for 7 tables in Base, en, zh-Hans and zh-Hant,
    falling back from a regional locale to its base and to Base
    (`localization.rs`).
  - State reducer over wallets, selection, locale, fiat currency and diagnostics
    (`state.rs`).
  - Legacy Swift wallet store converted to and from core state (`migration.rs`).
  - Wallet import plans for multi-chain seed and watch-only imports, including a
    Bitcoin xpub (`import.rs`).
  - Maintenance, chain refresh and history refresh plans (`refresh.rs`); history
    normalization deduplicated by hash, preferring confirmed (`history.rs`);
    transaction merge per chain family (UTXO, Dogecoin, account-based, EVM)
    (`transactions.rs`).
  - Interfaces only for balance and history providers, transfer planning,
    broadcast and secret storage (`fetch.rs`, `send.rs`, `store.rs`).
- Tests: 14 new
  - Presets and paths 3: the catalog not loading, a wrong Taproot or Bitcoin SV
    default path.
  - Bootstrap 1, localization 2: missing capabilities, no fallback for a
    regional or missing locale.
  - State, migration, import, refresh 5: the first wallet not selected, the
    legacy store not round-tripping, wrong import or refresh plans.
  - History and transaction merge 3: duplicates kept, unconfirmed preferred,
    existing creation times overwritten.
- Total: 17 .rs files changed (15 new), +5507 −87 lines; data and config
  files: `Spectra/Derivation/Rust/include/spectra_derivation.h`; dependencies
  unchanged; `#[test]` functions 1 → 15.

## Beta Commit 22

- **Rust no longer fills in derivation defaults** (`src/main.rs`)
  - A derivation path is required (before: a default path per chain).
  - Derivation algorithm, address algorithm, public key format and script type
    must be explicit; `Auto` is refused.
  - The Bitcoin script type is no longer inferred from the path purpose and must
    be P2PKH, P2SH-P2WPKH, P2WPKH or P2TR.
- **The derivation runtime moves from `src/lib.rs` to `src/main.rs`**, which
  `lib.rs` includes as a module.
- **Derivation request presets parse from JSON** (`src/lib.rs`
  `DerivationRequestPreset`, `parse_derivation_request_presets_json`)
- Total: 2 .rs files changed (1 new), +1463 −1398 lines; data and config files:
  none; dependencies unchanged; `#[test]` functions unchanged at 1.

## Beta Commit 18

- **Addresses from a raw private key**
  (`spectra_derivation_derive_from_private_key`,
  `SpectraPrivateKeyDerivationRequest` in `include/spectra_derivation.h`;
  `src/lib.rs`): the key is exactly 64 hex characters and its curve must match
  the chain; addresses for all 22 chains, with P2WPKH as the Bitcoin default.
- Total: 1 .rs file changed, +254 −0 lines; data and config files:
  `Spectra/Derivation/Rust/Cargo.lock`, `Spectra/Derivation/Rust/Cargo.toml`,
  `Spectra/Derivation/Rust/include/spectra_derivation.h`; dependencies: added
  elliptic-curve 0.13 (`Spectra/Derivation/Rust/Cargo.toml`); `#[test]`
  functions unchanged at 1.

## Beta Commit 16

- **Derivation for 22 chains** (`src/lib.rs`; chain ids 3–21 in
  `include/spectra_derivation.h`)
  - secp256k1 (BIP-32): Bitcoin, Bitcoin Cash and Bitcoin SV (legacy P2PKH),
    Litecoin, Dogecoin, the EVM chains (Ethereum, Ethereum Classic, Arbitrum,
    Optimism, Avalanche, Hyperliquid), Tron, XRP.
  - ed25519 (SLIP-0010): Solana, Stellar, Cardano, Sui, Aptos, TON, Internet
    Computer, NEAR, Polkadot; Cardano, TON, ICP and Polkadot addresses are ad
    hoc encodings of the public key (e.g. Cardano `addr1` + hex SHA-256), not
    those chains' standard formats.
  - Each chain requires its curve and derivation algorithm; Bitcoin accepts all
    four networks, Dogecoin mainnet and testnet, the rest mainnet only.
- **BIP-39 seed computed directly** (PBKDF2-HMAC-SHA512 over NFKD text)
  - Any iteration count is accepted (0 means 2048; 1 is refused).
  - An HMAC key other than "Bitcoin seed" or "ed25519 seed" is refused.
- **Unknown output flags are refused** (`src/lib.rs`)
- **`.gitignore` added**
- Tests: 1 new
  - Derivation 1: a supported chain failing to derive.
- Docs: `Spectra/Derivation/Rust/README.md` rewritten as "Rust Derivation Core":
  Swift calls Rust for every chain over the C ABI, safety model, ownership
  split.
- Total: 1 .rs file changed, +622 −101 lines; data and config files:
  `.gitignore`, `Spectra/Derivation/Rust/Cargo.lock`,
  `Spectra/Derivation/Rust/Cargo.toml`,
  `Spectra/Derivation/Rust/include/spectra_derivation.h`; dependencies: added
  unicode-normalization 0.1 (`Spectra/Derivation/Rust/Cargo.toml`); `#[test]`
  functions 0 → 1.

## Beta Commit 15

- **Secrets zeroized after derivation** (`src/lib.rs`): the parsed request
  clears seed phrase, passphrase, HMAC key, wordlist and path on drop, and the
  BIP-39 seed bytes are cleared after the master key is built.
- Docs: `Spectra/Derivation/Rust/README.md` "Bridge Notes" (Swift calls the C
  symbols directly, no Swift fallback) and "Seed Safety Notes".
- Total: 1 .rs file changed, +43 −9 lines; data and config files: none;
  dependencies unchanged; `#[test]` functions unchanged at 0.

## Beta Commit 14

- **Rust derivation crate with a C ABI** (new `Spectra/Derivation/Rust/`, crate
  `spectra_derivation` as a static library; `include/spectra_derivation.h`)
  - `spectra_derivation_derive` takes chain, network, curve, output flags,
    algorithm and script-type ids plus UTF-8 buffers for seed phrase, path,
    passphrase, HMAC key and wordlist, and returns optional address, public key
    and private key hex or an error; `spectra_derivation_response_free` and
    `spectra_derivation_buffer_free` release it.
  - Bitcoin only: BIP-32 secp256k1 with P2PKH, P2SH-P2WPKH, P2WPKH or P2TR,
    inferred from the path purpose when not given (an unknown purpose is
    refused); default path `m/84'/0'/0'/0/0`; testnet, testnet4 and signet use
    testnet addresses.
  - Refused: Ethereum and Solana (ids reserved), an iteration count other than 0
    (default) or 2048, a custom HMAC key, a non-English wordlist, ed25519 or
    SLIP-0010 for Bitcoin.
- Docs: `AGENTS.md` deleted; `Spectra/Derivation/Rust/README.md` (new):
  migration plan from the Swift engine, ownership split, frozen FFI ids.
- Total: 1 .rs file changed (1 new), +595 −0 lines; data and config files:
  `Spectra/Derivation/Rust/Cargo.lock`, `Spectra/Derivation/Rust/Cargo.toml`,
  `Spectra/Derivation/Rust/include/spectra_derivation.h`; dependencies: added
  bech32 0.11, bip39 2, bitcoin 0.32, bs58 0.5, curve25519-dalek 4,
  ed25519-dalek 2, hex 0.4, hmac 0.12, pbkdf2 0.12, secp256k1 0.29, secrecy
  0.8, serde 1, serde_json 1, sha2 0.10, slip10 0.4, tiny-keccak 2, zeroize 1
  (`Spectra/Derivation/Rust/Cargo.toml`); `#[test]` functions unchanged at 0.

## Initial Commit

- **Project README and agent guidance** (`README.md`, `AGENTS.md`)
  - The README states the wallet's ethos (self-custody, payments, staking,
    recovery, diagnostics; no trading, swap funnels or card on-ramps).
  - `AGENTS.md` asks for permission before applying a change app-wide.
- Docs: `README.md` (new), `AGENTS.md` (new).
- Total: No code changed.