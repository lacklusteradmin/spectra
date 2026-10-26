# Architecture decisions

Spectra is one application with a shared Rust core and native front ends.
Core owns both the data and the decisions, no platform persists a second
authoritative copy, and the CLI can drive every domain operation.
[FFI-BOUNDARY.md](FFI-BOUNDARY.md) covers binding mechanics;
[OPEN-ITEMS.md](OPEN-ITEMS.md) holds remaining work.

## Workspace

| Directory | Responsibility |
|---|---|
| `core/` | Domain logic, persistence, network, crypto and UniFFI exports; built as the app's static library |
| `cli/` | Argument parsing, terminal output and password prompting |
| `swift/` | Native iOS UI and platform services |
| `scripts/` | Build, source-check and runtime-data tools |
| `tests/` | CLI acceptance suites, helpers and independent SDK test-data generators |
| `tools/uniffi-bindgen/` | Binding generation binary |
| `tools/zcash-fixture/` | Loopback lightwalletd over a synthetic Zcash chain, for `tests/cli-zcash-shielded.py` |
| `tools/litecoin-mweb-fixture/` | Loopback Litecoin node and Esplora indexer over a synthetic MWEB chain, for `tests/cli-litecoin-mweb.py`; shares no code with core |

Core stays one crate. A crate per domain would add Cargo overhead without an
external consumer that needs the isolation. There are no published library APIs
or semver commitments between these crates. Core builds and tests without Xcode.

Xcode builds the Rust core through `scripts/build-ios.sh` into its own
DerivedData (`spectra-rust/cargo-target`), not the workspace `target/`. Keeping
them apart means terminal cargo and Xcode never wait on each other's build lock
or invalidate each other's cache with different environments, and Clean Build
Folder clears the Rust side too. Deleting `target/` does not affect Xcode.

## Ownership

Rust owns wallets, settings, address rules, transaction history, persistence,
network policy and signing. Front ends send intents and render the results.
The CLI is the check that none of those operations depends on a phone.

Swift owns navigation, sheets, text being edited, focus, animation and rendering
caches. Keychain, biometrics, notifications, Live Activities and device signals
stay on the platform and supply inputs to core-owned policy. Losing view state
on restart may cost a redraw; losing domain state must not lose user data.

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
and tests use the same domain operations.

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
keywords, picker rank, tags, color, artwork and address hints have no separate
catalog or join key. Every network requires its own presentation fields;
testnets inherit rank and tags from their mainnet. Display classifications
do not determine protocol capabilities. EVM membership is having an EIP-155 id
(`Chain::evm_chain_id`), so the EVM list is written once; the catalog projects
`is_evm` for clients without a configurable TOML flag. Address validation
dispatches on the `Chain` itself (`validation::address::validate_address`),
not on a string naming a format.
The chain picker and wiki share `ChainTag` and the catalog's resolved tags;
`chain-wiki.toml` holds prose only and cannot restate a tag list.
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
and wrappers that differ only in a chain name. Tests over the registry should
assert complete capability coverage, rather than only test named examples.

## Boundary design

Exports live next to their implementations. Stateful operations are service
methods; genuinely stateless calculations may be free functions. Do not merge
distinct preview inputs into a wide command/response union just to lower the
export count: Bitcoin's xpub and gap bounds are different inputs from an EVM
nonce and fee configuration.

The native UIs share domain code, not a cross-platform UI framework. Core must
not exit the process or install a global logger; the executable owns logging
configuration and keeps stdout available for CLI JSON.

Rules for keeping the shell thin:

- Swift code that reads core-owned data only to send it back for a decision is
  a misplaced rule: make the owning service compute the answer instead. Audit
  views and record extensions too, not only `AppState`.
- Keep one writer per UI projection. Adopt core-derived answers
  asynchronously; local indexes and button-enabling checks may remain view
  state, with core enforcing validation on writes.
- Delete projection and cache fields when their last reader disappears.
- Keep concrete network, token and deployment identities distinct; never infer
  identity from a ticker or a price provider's id.
- Remove a dead wrapper only after checking direct FFI callers and foreign
  callback implementations. Delete tests of a removed helper only once the
  replacement's meaningful coverage is identified.
- An export that removes a Swift rule can be worthwhile; moving code only to
  lower a line count is not.

## Signing and service modules

`service/state.rs` owns the serialized state writer and app projections.
`keypool`, `address_discovery`, `transactions`, `wallet_import` and
`operational_events` hold cohesive operations on that state; all persistent
mutations still use the same writer rather than creating per-file owners.

The send service is split into `send_execution` (stored identity and exact
amount conversion), `send_destination` (fresh resolution and review binding),
`send_preflight` (eligibility and recipient warnings), `send_preview` (quotes),
`send_stages` (durable build/sign/broadcast) and `send_broadcast`
(rebroadcast). `send_stage_protocols` dispatches an account protocol's
preparation, revalidation, signing and expiry to that protocol's own
`send_stage_<protocol>` sibling (NEAR's in `send_near`); the UTXO family's
live in `send_stage_utxo`, `send_stage_litecoin` and `send_stage_peercoin`.
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

Production source files normally sit at `core/src/<domain>/<topic>.rs`, with
no further directory nesting; `lib.rs` declares the domains and `error.rs`
holds `SpectraBridgeError` and `LocalizableMessage`, and nothing else sits at
the root. `registry/` is `Chain` with one `impl Chain` sibling per topic
(`keys`, `address`, `endpoints`, `tokens`, `send`, `staking`, `utxo`,
`networks`, `evm`); `catalog/` parses the embedded `core/data/` files (chains,
tokens, endpoints, explorers, donations, the wiki and restore heights);
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

Out-of-line unit tests live in their domain's `tests/` directory, the one
exception to the depth rule, named after the module they test:
`service/tests/send_execution.rs`. Declare each as a child of that module with
`#[cfg(test)]` and `#[path = "tests/send_execution.rs"]` so tests keep private
access without widening production visibility. Where a domain's tests need only
crate-visible items, `tests/mod.rs` owns them as one module instead, as in
`store/tests/` and `wallet_db/tests/`. Crate-root tests use `src/tests/`.
Integration tests that need their own process sit in `core/tests/`, with shared
test data in `core/tests/fixtures/`. CLI acceptance suites, their shared helpers
and independent SDK generators for that data live in the repository's top-level
`tests/`; their instructions are in
[`tests/README.md`](../tests/README.md).

Each layer reports failures in its own `thiserror` enum, named in its
`error.rs`: `api::error::ApiError` (transport, status, decode, rejected, no
endpoint, invalid input), `derivation::error::DerivationError`,
`send::error::SendError` (which wraps the first two and adds
`InsufficientFunds`), `wallet_db::error::DbError` and `registry::RegistryError`.
Each converts into `SpectraBridgeError` at the FFI, choosing `Network`,
`Decode`, `InvalidInput` or `Failure` where the error is raised, so the front
end branches on a variant rather than a message. There is no conversion from a
bare string: a new error names its category. Functions that take a caller's
closure, such as `WalletDatabase::with_connection`, are generic over the
caller's error type (`E: From<DbError>`). Embedded catalogs (`catalog::chains`,
`tokens`, `endpoints`, `explorers`, `donations`) are validated on first use and
panic when broken, since a broken embedded file is a build defect.

- `api/` owns every request to a chain service and the parsing of its answer:
  one module per `EndpointApi` (`api/esplora.rs`, `api/substrate_json_rpc.rs`,
  …), plus `utxo` (the UTXO family's client over several of them), the `http`
  and `json_rpc` transport, the `tcp` connection that the `grpc` channel
  `lightwalletd` speaks over and Litecoin's peer-to-peer protocol
  (`litecoin_p2p`) dial (under the same Tor, proxy and loopback rules as
  `http`) and provider `time` parsing. `endpoint` defines `EndpointApi` and
  the capabilities an adapter offers on each network; `tor` is the embedded
  Tor client every transport routes through when it is on.
  `litecoin_p2p::wire` holds MWEB's wire format, which `send::litecoin_mweb`
  builds with. `price` reads market quotes and fiat rates from their
  keyless providers. It depends on nothing above it; `send`, `staking` and
  `service` call it.
- Every history adapter converts its own answer into one
  `api::history_page::HistoryEntry` — the units it reads, its timestamp
  scale and where it says which way a transfer went stay with the code that
  parsed them. `service/network_history.rs` picks a chain's adapter and gives
  each entry its asset identity as a `NormalizedHistoryItem`; nothing is
  serialized to be read back by field name.
- `api/network.rs` proves an endpoint is the network it is configured for,
  through each client's own `verify_network`, before core signs or submits
  through it; `api/health.rs` holds the read-only health probes. Neither
  decides policy: `service/network.rs` runs the probes and records them, and
  the send stages decide that only a catalog endpoint may skip a network
  core cannot ask. Its siblings `network_balance`, `network_tokens`,
  `network_history` and `network_prices` own the corresponding reads and
  dispatch. `network_tokens::token_decimals` is the one reader of a token's
  decimals: a send asks it `MetadataRead::Fresh`, while a balance read may
  reuse the TRC-20 metadata other balance reads cached.
- `service/wallet_actions.rs::offers` is the one gate for what a wallet
  does: its page lists the actions it answers yes to, and each operation
  behind one fetches its wallet through `action_wallet`, which asks it again.
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
  `service/multisig.rs` owns the records, the `multisig_sessions` table and
  the dispatch; each scheme's rules are in its `service/multisig_<scheme>.rs`
  sibling, which reads the network again before a signature or a
  submission.
- `send/bitcoin_wire.rs` contains only Bitcoin-format serialization.
  Other UTXO protocols must establish byte compatibility before reusing it.
  Kaspa owns its own hash preimage encoding. Solana compiles a unique account
  list across all instructions before signing.
- `wallet_db/` separates connection/schema, keypool, addresses, history,
  wallets, state and teardown. `state` and `teardown` keep their cross-table
  transactions; splitting files does not split commits. `store/tests/` groups
  regressions by domain. `wallet_db::zcash` is the exception to one database:
  a Zcash wallet's shielded state is librustzcash's own database
  (`zcash/<wallet id>.sqlite` beside Spectra's), which only
  `service::zcash_shielded` and `send::zcash_shielded` open, and which wallet
  removal deletes with the wallet's secrets. `wallet_db::scan_cache` holds
  what a scan with a wallet's own key finds — a Monero wallet's outputs, a
  Litecoin wallet's MWEB outputs — encrypted under a key derived from that
  scan key, which the SecretStore keeps as `{wallet id}.scan-key`.

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
coordination; Swift does not round-trip a quote's transaction request. Native
`SendSession` owns only workflow identity, in-flight state and displayed results.
Obsolete callbacks cannot update another session or start signing after native
authentication. Core operations already dispatched remain durable after closing.
Amount display utilities are optional presentation policy; signing review always
renders the exact artifact amount and exposes the prepared transaction details.

### Native flow lifetimes and display precision

`AppState` composes one `@MainActor @Observable` object per domain: the send
and receive flows, wallet import, the address book, token preferences, price
alerts, Tor, history paging, diagnostics and the platform's own preferences.
Each holds that domain's projection or view state — form inputs, loading and
error state, request identities, reset behavior — and the actions that need
nothing else; views read the object, and no forwarding copies of its fields
live on `AppState`. A projection of core state (contacts, tokens, alerts) has
one writer, `adopt`, fed by `AppState.applyCoreState`, and its edits go through
the shared `StateCommandQueue`. What needs the wallet projection or several
domains — the selected send holding, a wallet's signing requirement,
post-broadcast work — stays a thin adapter on `AppState`. Import and rename
completions may update only the session that submitted them. Dismissal
invalidates the session and clears secret inputs; a core commit still refreshes
the domain projection even when its form is gone.

`PortfolioSnapshot.asset_precision` supplies effective precision keyed by
concrete deployment, including disabled custom tokens needed by history views.
The snapshot owns the unknown-history display fallback too. Swift does not
round-trip token preference data through a precision helper. Until the first
snapshot arrives, amounts needing that metadata show an unavailable placeholder.
The compact six-significant-digit/eight-decimal style is an optional shared
presentation utility, not domain validation or a requirement for detail views.
Signing review continues to render exact artifact amounts.

### Native completion and presentation boundaries

A closed send composer rejects stale form updates but still returns successful
broadcast results to application handling. Post-send work resolves the completed
transaction ID, not the current composer. Recent/pending history is a summary;
Live Activities and resumed send details query the stored ID and distinguish
missing records from read failures.

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
`AmountPresentation` renders explicit projection values without AppState or
storage. Its native formatter cache is presentation-only. `AssetPresentationCatalog`
indexes core's immutable artwork and caches core-derived holding identities by
network, token standard and contract; balances and display labels do not identify
assets. Neither cache can authorize a domain mutation.
