# Architecture decisions

Spectra is one Rust core with front ends: the `spectra` CLI in this repository
and the native apps in spectra-apple, for iOS and macOS, and spectra-android,
each linking core as a package built here. Core owns both the data and the
decisions, no platform persists a second
authoritative copy, and the CLI can drive every domain operation.
[FFI-BOUNDARY.md](FFI-BOUNDARY.md) covers binding mechanics,
[CHAIN-SUPPORT.md](CHAIN-SUPPORT.md) what each network supports and
[OPEN-ITEMS.md](OPEN-ITEMS.md) remaining work.

## Workspace

| Directory | Responsibility |
|---|---|
| `core/` | Domain logic, persistence, network, crypto and UniFFI exports; built as the apps' native library |
| `cli/` | Argument parsing, password prompting and each command's report, as text or `--json` |
| `icons/crypto/` | The SVG library the apps draw coins and tokens with, each mark on a disc, one file per token artwork name in the catalog |
| `icons/chain/` | The SVG library the apps draw chains and wallets with, each mark on a rounded square, one file per mainnet named by its id and one per testnet that does not draw its mainnet's |
| `icons/appicon/` | The app icon's sources, which each app exports into its own app icon assets |
| `strings/` | The string tables both apps read, one per locale, with the manifest naming the locales |
| `scripts/` | Source-check, icon and runtime-data tools |
| `tests/` | CLI acceptance suites on one harness (`tests/acceptance/`) and independent SDK test-data generators (`tests/vectors/`) |
| `tools/uniffi-bindgen/` | Binding generation binary |
| `tools/zcash-fixture/` | Loopback lightwalletd over a synthetic Zcash chain, for `tests/acceptance/zcash_shielded.py` |
| `tools/litecoin-mweb-fixture/` | Loopback Litecoin node and Esplora indexer over a synthetic MWEB chain, for `tests/acceptance/litecoin_mweb.py`; shares no code with core |

Core stays one crate. A crate per domain would add Cargo overhead without an
external consumer that needs the isolation. There are no published library APIs
or semver commitments between these crates. Core builds and tests without Xcode.

An Apple app links core as the SpectraCore Swift package that
`scripts/build-apple-package.sh` builds: a static library per platform, bundled
as an XCFramework, and the Swift bindings generated from it. `make apple` runs
it into `target/core-apple/`, a developer copies that directory into
spectra-apple, and Xcode compiles no Rust. It builds in `target/apple/`,
not the workspace's own `target/`: its builds set the app's minimum OS
versions, which would otherwise rebuild what terminal cargo builds without
them, and terminal cargo and a package build never wait on each other's build
lock.

An Android app links core as the package `scripts/build-android-package.sh`
builds: a shared library per Android ABI, linked by the NDK's compiler for the
app's minimum API level, and the Kotlin bindings generated from it. `make
android` runs it into `target/core-android/`, a developer copies that
directory into spectra-android, and Gradle compiles no Rust. It builds in
`target/android/` for the same reasons.

## Ownership

Rust owns wallets, settings, address rules, transaction history, persistence,
network policy and signing. Front ends send intents and render the results.
The CLI is the check that none of those operations depends on a phone. Each
command ends in one report (`out::Report`): a `Serialize` value that `--json`
writes as one document and the terminal renders as text, so neither form can
go missing. A command about one wallet takes it as its first argument;
`--wallet` only narrows a command over many.

An app owns navigation, sheets, text being edited, focus, animation and
rendering caches. Keychain or Keystore, biometrics, notifications, Live
Activities and device signals stay on the platform and supply inputs to
core-owned policy. Losing view state on restart may cost a redraw; losing
domain state must not lose user data. How each app is put together is its
repository's `docs/ARCHITECTURE.md`.

`ResidentState` holds small resident collections. Unbounded history is a separate
queryable store so changing a setting does not clone every transaction.
`WalletService::open_state` binds the database; state commands persist before
returning and no-op commands emit no events or writes. UI projections have one
writer. Derived domain data is computed from core's store and adopted by the UI,
not computed by sending the projection back to core. Portfolio snapshots derive
wallets, quotes, grouping, pins and valuation from one state and carry a session
revision; front ends must reject older results. Every resident-state response also
carries a core-assigned committed-state revision; failed requests never advance
it. History pages are bounded queries,
while recent/pending activity and indexed aggregates arrive in a separate summary.
A summary is not the entire history; details and actions resolve stored IDs.

Import forms and backup-quiz navigation are platform view state. Core exposes
secret/address validators and enforces validation again on import. Watch-only
inputs name chains rather than internal address slots. Core does not
model the front end's pages or rename-form completeness. Device authentication
uses explicit action policies so the send preference cannot disable unlock,
delete or reset protection. Native authentication remains a platform operation.

Send previews carry their wallet, deployment and concrete network identity plus
core-derived shortcut amounts and display details. Clients do not return previews
with separately reconstructed metadata for reinterpretation. Destination activity
is a core enum computed from raw smallest-unit balances, never formatted amounts.
An empty amount is quoted provisionally, for a share of the holding core picks;
the quote's own `amount` says what it priced.

A staged transaction's review is one ordered list of `ReviewWarning`s and one
`NetworkFee` marked as an estimate, exactly what is paid, or the most it can
cost; a wallet operation's fee is read from, and validated against, its
prepared payload. Every page that reviews an artifact words that list and that
fee the same way. What may be done next is `send_progress`: the next step
(sign, broadcast, rebroadcast or done) and whether a rebroadcast, a staking
repair or a status recheck applies, read from the artifact and its history
record together. Front ends adopt it with every artifact they take and never
derive a step from the stage.

The platform registers a cache directory with `configure_network_runtime`.
Thereafter core reconciles Tor transports on committed settings changes and reset;
clients render status or request reconnect. CLI network services also register
and await transport readiness. Settings editing remains usable offline. Stopping
or changing transport cancels bootstrap and invalidates obsolete completions.

Settings forms may display optimistic edits. Runtime effects (Tor, notification
permission/delivery and refresh cadence) read only the last committed projection;
an uncommitted edit is never transport configuration. Send confirmation renders
core's password requirement and forwards transient native password input for
core to unlock, without keeping it in application state.

Core drives secret reads, writes and password encryption through `SecretStore`.
The platform implements storage; native file and in-memory backends let the CLI
and tests use the same domain operations. A phrase wallet's derivation
overrides — a BIP-39 passphrase, a custom HMAC key — are sealed beside its
phrase and opened with it; the wallet record, stored and sent to the front
ends, keeps only which ones it has (`WalletDerivationFlags`).

## Chain rules

`registry::Chain` is the authority for chain identity and capabilities, backed
by `core/data/chains.toml`. Adding a chain requires a catalog row and an enum
variant in matching order; one list in `registry/mod.rs` declares both the enum
and its iteration order. Registry tests check the catalog relationship
independently. A test network names its mainnet (`mainnet = "bitcoin"`); a
mainnet writes nothing, so a network's family cannot disagree with its own id.
Front ends read the whole catalog once as `list_all_chains`, one `ChainEntry`
per network carrying the `Chain` itself, its identity, the capabilities a
screen branches on and its presentation; there is no second per-chain table.
Network rules and presentation live on the same `chains.toml` row: search
keywords, picker rank, tags, color, artwork, address hints and what the chain
is for a reader (description, family, consensus, state model) have no separate
catalog or join key. Every network requires its own presentation fields;
testnets inherit rank, tags and the description from their mainnet. Display classifications
do not determine protocol capabilities. EVM membership is having an EIP-155 id
(`Chain::evm_chain_id`), so the EVM list is written once; the catalog projects
`is_evm` for clients without a configurable TOML flag. Address validation
dispatches on the `Chain` itself (`validation::address::validate_address`),
not on a string naming a format.
The chain picker filters by `ChainTag` and the catalog's resolved tags.
A token's description and supply model sit on its `tokens.toml` identity, not
on its deployments; `KnownToken` joins them to every network the token is on,
native coins included, for Known Tokens.
Address formats, derivation paths, address slots, EVM membership and routing
facts belong there rather than in caller-owned lists.

Token standards belong to individual deployments. A standard is a
`registry::TokenStandard`, spelled as its name (`"ERC-20"`) in the catalogs and
the store; a network's own coin has none, rather than a sentinel standard, and
`catalog::tokens::deployment_id` is the one place a deployment's id is spelled.
A chain's configured standard is a default for input, not a restriction to one
protocol. `Chain`
validates the supported chain/standard/identifier combinations; the token
catalog and persisted holdings use those rules. Read and send capabilities
are checked separately, so a representable protocol cannot silently be routed
through another standard's adapter.

A chain crosses every boundary as `Chain`, never as its id string. Records and
parameters carry the enum (a field may keep the name `chain_id`, which is what
it serializes as); `Chain` serializes, stores in SQLite and prints as its
catalog id, and an id the catalog does not know fails where it is read. The
only string parsing left is at the edges that receive text: the CLI's
arguments, the catalog TOML files and the database columns.

Chain-specific implementations live directly in each domain directory, for
example `derivation/bitcoin.rs`, `service/bitcoin_history.rs` and `send/bitcoin.rs`.
Keep differences that carry protocol meaning; share cryptographic primitives
and wrappers that differ only in a chain name. Each primitive has one
implementation: `derivation/bip32.rs` reads every derivation path and holds
the secp256k1 extended keys (rust-bitcoin's), `derivation::primitives::blake2b`
every plain BLAKE2b, `send/leb128.rs` every LEB128 number read or written,
rust-bitcoin's `VarInt` every CompactSize, `derivation::primitives::ByteReader`
every bounded read of an encoded transaction or wire message, and
`api::evm_json_rpc::abi_u128`
every uint256 ABI word, EVM's and Tron's alike. Tests over the registry should assert
complete capability coverage, rather than only test named examples.

## Boundary design

Exports live next to their implementations. Stateful operations are service
methods; genuinely stateless calculations may be free functions. Do not merge
distinct preview inputs into a wide command/response union just to lower the
export count: Bitcoin's xpub and gap bounds are different inputs from an EVM
nonce and fee configuration.

Front ends share domain code, not a UI framework. Core must
not exit the process or install a global logger; the executable owns logging
configuration and keeps stdout available for CLI JSON. The rules that keep
the app's Swift shell thin are in spectra-apple's `docs/ARCHITECTURE.md`.

## Signing and service modules

`service/state.rs` owns the serialized state writer and app projections.
`keypool`, `address_discovery`, `transactions`, `wallet_import` and
`operational_events` hold cohesive operations on that state; all persistent
mutations still use the same writer rather than creating per-file owners.

The send service is split into `send_execution` (stored identity and exact
amount conversion), `send_destination` (fresh resolution and review binding),
`send_preflight` (eligibility and recipient warnings), `send_preview` (quotes),
`send_stage` (durable build/sign/broadcast) and `send_broadcast`
(rebroadcast). `send_stage::protocols` dispatches an account protocol's
preparation, revalidation, signing and expiry to that protocol's own module
under `send_stage/` (NEAR's in `near`); the UTXO family's live in `utxo`,
`litecoin` and `peercoin`.
Prepared content is typed Rust data, not caller-owned JSON.
Secrets use redacted, zeroizing storage; an Ed25519 seed is a distinct type
rather than an unlabelled 32/64-byte array.

The Tron, Aptos and Sui protocols build their transaction bytes locally from
explicit transfer inputs and fetched chain metadata. Their prepared values
keep bytes private and expose offline signing; network submission accepts the
signed output. Solana's local builders likewise feed a broadcast-only stage.
Independent SDK fixtures test real mnemonic derivation through these signing
boundaries; mock-node tests exercise the stored-wallet execution route.

## Core module boundaries

Production source files normally sit at `core/src/<domain>/<topic>.rs`. A
domain nests one directory further for a group whose helpers the rest of the
domain should not see: one large protocol's parts (`api/litecoin_p2p/`,
`api/substrate_json_rpc/`, `send/litecoin_mweb/`), one module per
message-signing scheme (`send/message/`), or a dispatcher with one child per
protocol or scheme (`service/send_stage/`, `service/multisig/`). A directory
module's body is its `mod.rs`, a domain's and a group's alike. A helper only
its group calls is private to the group's `mod.rs` or `pub(super)` in a child;
one the rest of the domain calls is `pub(in crate::<domain>)`. `lib.rs` declares the
domains and `error.rs` holds `SpectraBridgeError` and `LocalizableMessage`, and
nothing else sits at the root. `registry/` is `Chain` with one `impl Chain` sibling per topic
(`keys`, `address`, `endpoints`, `tokens`, `send`, `staking`, `utxo`,
`networks`, `evm`, and one per protocol family: `substrate`, `cosmos`,
`algorand`, `vechain`, `tezos`, `starknet`, `hedera`), and the asset identities a token is named by
(`NftStandard` in `tokens`, `xrpl_amount`, `stellar_asset`,
`cardano_asset`), which `catalog/`, `api/` and `send/` share; `catalog/` parses the embedded `core/data/` files (chains
with their staking copy, tokens and test-network tokens, endpoints,
explorers, donations and restore heights);
`amount/` holds exact decimals, typed input and optional presentation;
`secrets/` holds the `SecretStore`, its backends, the device key, envelopes
and password stretching. Chain names and topic prefixes identify siblings:
`derivation/ton_cell.rs`, `api/tron_metadata_cache.rs`, `service/refresh_engine.rs`
and `service/refresh_policy.rs`. Keep cohesive modules; flattening directories is
not a reason to merge unrelated code or enlarge existing files.

Use the domain-qualified Rust path (`crate::api::http`, `crate::store::state`,
`crate::send::ethereum`). Do not add crate-root module aliases or compatibility
re-exports for moved modules. `wallet_db` is a root module owning relational
storage; `store` owns resident state and wallet-domain rules; `secrets` owns
what is sealed, not resident state.

A module's unit tests sit in one place: inline when the block is small,
otherwise in one file in their domain's `tests/` directory, named after the
module that hosts them:
`service/tests/send_execution.rs`. Declare it as a child of that module with
`#[cfg(test)]`, `#[path = "tests/send_execution.rs"]` and `mod tests;` so tests
keep private access without widening production visibility. A module never
keeps tests both inline and out of line, and a test sits with the module whose
behaviour it exercises. A module in a group's directory keeps its tests in the
domain's `tests/` too, the file named after the group and then the module:
`api/tests/substrate_json_rpc_pools.rs`, `service/tests/send_stage_evm.rs`; a
group's `mod.rs` keeps its own in a file named after the group:
`service/tests/send_stage.rs`. A domain's root module keeps
its own in `tests/mod.rs`, as `derivation`, `registry` and `send` do; where a
domain's tests need only crate-visible items, that file groups them by domain
in child files instead, as in `store/tests/` and `wallet_db/tests/`. The
service fixtures other tests share (`loopback_service`, `send_stage_support`)
are declared in `service/mod.rs`. The crate root keeps its own in
`src/tests/mod.rs`.
Integration tests that need their own process sit in `core/tests/`, with shared
test data in `core/tests/fixtures/`. The CLI acceptance suites and their shared
harness live in `tests/acceptance/`, and the independent SDK generators for that
data, with one `package.json`, in `tests/vectors/`; their instructions are in
[`tests/README.md`](../tests/README.md).

Each layer reports failures in its own `thiserror` enum:
`api::error::ApiError` (transport, status, decode, rejected, no endpoint,
invalid input), `derivation::error::DerivationError`, `send::error::SendError`
(which wraps the first two and adds `InsufficientFunds`, `Invalid` and
`Internal`), `wallet_db::error::DbError`, `registry::RegistryError` and, in
`secrets/`, `SecretStoreError`, `EnvelopeError` and `WalletSecretError`. Each
converts into `SpectraBridgeError` at the FFI, choosing `Network`, `Decode`,
`InvalidInput` or `Failure` where the error is raised, so the front end branches
on a variant rather than a message. `StoreUnreadable`, the fifth variant, comes only
from `open_state`, so a front end can offer `discard_state`. There is no
conversion from a bare string: a new error names its category. Functions that
take a caller's closure, such as `WalletDatabase::with_connection`, are generic
over the caller's error type (`E: From<DbError>`). Embedded catalogs are
validated on first use and panic when broken, since a broken embedded file is a
build defect.

- `api/` owns every request to a chain service and the parsing of its answer:
  one module per `EndpointApi` (`api/esplora.rs`, `api/substrate_json_rpc/`,
  …), plus `utxo` (the UTXO family's client over several of them), the `http`
  and `json_rpc` transport, `tcp` (the connection `lightwalletd`'s `grpc`
  channel and Litecoin's peer-to-peer protocol `litecoin_p2p` dial, under the
  same Tor, proxy and loopback rules as `http`) and provider `time` parsing. `endpoint` defines `EndpointApi` and
  the capabilities an adapter offers on each network; `tor` is the embedded
  Tor client every transport routes through when it is on.
  `litecoin_p2p::wire` holds MWEB's wire format, which `send::litecoin_mweb`
  builds with. `price` reads market quotes and fiat rates from their
  keyless providers, and `zcash_parameters` downloads the Sapling proving
  parameters. A request's body is the adapter's to write: `send` hands it
  signed bytes or a typed transaction (`kaspa_rest::KaspaTransaction`), never
  a document in the service's schema. `send`, `staking` and `service` call `api`, never the
  other way round. It may use what sits below them: `registry`, `catalog`,
  `amount` and `derivation`'s address codecs and hashes; `tor` also reads the
  committed `AppSettings`.
- Every history adapter converts its own answer into one
  `api::history_page::HistoryEntry` — the units it reads, its timestamp
  scale and where it says which way a transfer went stay with the code that
  parsed them. `service/network_history.rs` picks a chain's adapter and gives
  each entry its asset identity as a `NormalizedHistoryItem`; nothing is
  serialized to be read back by field name.
- `api/network.rs` proves an endpoint is the network it is configured for,
  through the `verify_network` of the client of the API it speaks;
  `asks_network` says which APIs can be asked. The health probe in
  `api/health.rs` asks it, and reads an API's own routes only where it cannot
  be asked. Neither decides policy: `service/network.rs` runs the probes and
  records them, and the send stages ask before they submit and decide that
  only a built-in endpoint may skip a network core cannot ask. Its siblings
  `network_balance`, `network_tokens`, `network_history` and `network_prices`
  own the corresponding reads and dispatch. `network_tokens::token_decimals`
  is the one reader of a token's decimals: a send asks it
  `MetadataRead::Fresh`, while a balance read may reuse the TRC-20 metadata
  other balance reads cached.
- `service/endpoint_directory.rs` routes every request. A network's endpoints
  are `EndpointRecord`s: the user's own, then the service's built-in ones (the
  catalog's; a test's loopback nodes) unless the user chose theirs alone.
  A client is handed the records of its API that declare the capabilities the
  request needs. `WalletService::use_only_endpoints` names a network's
  endpoints for one process (the CLI's `--endpoint`): those of its own APIs
  replace every configured one, and its indexers stay as configured.
- `service/wallet_actions.rs::offers` decides what a wallet's page lists. The
  account operations behind that page (staking, NEAR keys and token storage,
  Sui coins, Solana token accounts, EVM approvals and NFTs, trust lines) fetch
  their wallet through `action_wallet`, which asks it again.
- Staking: `registry::StakingModel` says how a network stakes — delegation
  (Solana, Sui, Aptos, NEAR), nomination pools (Polkadot) or neurons (ICP).
  `service/staking.rs` dispatches each operation once on it, to
  `staking_protocols.rs`, `staking_substrate.rs` or `staking_icp.rs`;
  `staking::fetch_validators` reads each network's validator directory.
- `service/history_refresh.rs` reads an account UTXO wallet's history over
  every address its account is known to hold. `service/bitcoin_history.rs`
  owns provider pagination and buffered block-cohort aggregation; a display
  limit never means provider exhaustion.
- Multisig accounts: `derivation/multisig_policy.rs` reads a policy the
  address derives from, each network's form in its own module
  (`multisig.rs` for UTXO descriptors, `sui_multisig.rs`,
  `aptos_multikey.rs`, `cardano_script.rs`, `substrate_multisig.rs`).
  `send/<scheme>.rs` builds, decodes, signs and verifies each scheme's
  transactions without I/O (`psbt.rs`, `p2sh_multisig.rs`, `safe.rs`,
  `tron_multisig.rs`, `xrp_multisig.rs`, `stellar_multisig.rs`,
  `sui_multisig.rs`, `aptos_multikey.rs`, `cardano_multisig.rs`,
  `substrate_multisig.rs`, `ton_multisig.rs`); a policy the network keeps is
  read in `api/` (`evm_safe.rs`, `tron_http.rs`, `xrpl_json_rpc.rs`,
  `horizon.rs`, `substrate_json_rpc/multisig.rs`, `toncenter_v2.rs`), which
  defines the read model it returns (`SafeState`, `TronPermissions`,
  `XrpSignerList`, `StellarSigners`); `send` builds against it, never the
  other way round.
  `service/multisig/mod.rs` owns the records, the `multisig_sessions` table
  and the dispatch; each scheme's rules are in its
  `service/multisig/<scheme>.rs` child, which reads the network again before
  a signature or a submission.
- `send/bitcoin_wire.rs` contains only Bitcoin-format serialization.
  Other UTXO protocols must establish byte compatibility before reusing it.
  Kaspa owns its own hash preimage encoding. Solana compiles a unique account
  list across all instructions before signing.
- `wallet_db/` separates connection/schema, keypool, addresses, history (with
  `history_query` and `history_pagination`), wallets, sends, multisig, the scan
  cache and state. A history row is its `TransactionRecord`; the
  indexed columns beside it (lowercased id, wallet and hash, network, sort
  key) are derived from the record when it is written. A send made here keeps
  its signed transaction only in its send record, which a rebroadcast
  submits. `state` keeps its cross-table
  transactions; splitting files does not split commits. `store/tests/` groups
  regressions by domain. `wallet_db::zcash` is the exception to one database:
  a Zcash wallet's shielded state is librustzcash's own database
  (`zcash/<wallet id>.sqlite` beside Spectra's), which only
  `service::zcash_shielded` and `send::zcash_shielded` open, and which wallet
  removal deletes with the wallet's secrets. `wallet_db::scan_cache` holds
  what a scan with a wallet's own key finds — a Monero wallet's outputs, a
  Litecoin wallet's MWEB outputs — encrypted under a key derived from that
  scan key, which the SecretStore keeps device-sealed as
  `{wallet id}.scan-key`.

### Persistent send stages

`WalletService.build_send`, `sign_send` and `broadcast_send` own immutable
prepared content, signed bytes, and per-endpoint submission attempts.
The owned flow's confirmed send composes these operations. SQLite stores secret-free typed
artifacts with revision checks and atomic nonce/input reservations; wallet
removal also removes its artifacts. A signed artifact derives its reservations
from its own prepared content (`StoredSend::reservations`), each spelled one
way per resource, so two artifacts consuming one input or nonce cannot both be
stored. A reviewed digest commits to the exact
prepared content. Signing may validate freshness but never changes that content;
broadcast never rebuilds or signs. Unknown submission outcomes retain the same
payload for inspection and explicit retry. On-chain status remains history's
responsibility. Swift displays these records and forwards explicit actions;
its selected checkboxes and navigation are view state.

The CLI exposes `send build`, `sign`, `inspect`, `list`, `broadcast-signed` and
`configured-endpoints`. Unsupported protocol stages are refused according to
registry capabilities.

Build-time send advisories are stored with each artifact and covered by its
review digest. `build_owned_send` accepts user edits and owns quote/build
coordination; Swift does not round-trip a quote's transaction request. Core
operations already dispatched remain durable after closing.
Amount display utilities are optional presentation policy; signing review always
renders the exact artifact amount and exposes the prepared transaction details.

### Display precision

`PortfolioSnapshot.asset_precision` supplies effective precision keyed by
concrete deployment, including disabled custom tokens needed by history views.
The snapshot owns the unknown-history display fallback too. Swift does not
round-trip token preference data through a precision helper. Until the first
snapshot arrives, amounts needing that metadata show an unavailable placeholder.
The compact six-significant-digit/eight-decimal style is an optional shared
presentation utility, not domain validation or a requirement for detail views.
Signing review continues to render exact artifact amounts.

### Refresh and diagnostics

Refresh completes alert/movement evaluation before adopting the final portfolio
and history projections once. During a balance sweep the engine reports each
wallet as core commits it, and the app re-reads the portfolio at a bounded rate
so balances do not wait for the slowest chain; those reads carry no alerts.
Notification delivery does not rebuild projections.

Diagnostics runs record their results in core: history rows by family, endpoint
probes by network, and when each ran. `chain_diagnostics` and
`diagnostics_bundle` answer from that record for every front end; the platform
supplies only its own version, OS, locale and time zone, and keeps which runs
are in flight.
