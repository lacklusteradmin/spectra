# Behaviour changed on purpose

The log Rule 0 requires. [PLAN.md](../PLAN.md) holds the rules and the work
still open; this file holds what has already changed and why, so a decision can
be found later without reading the plan around it.

Rule 0 licenses changing behaviour, not changing it silently. One entry per
change, newest first, and each says what it was, what it is, why that side, and
how to check it without the app:

- **Before / After** — the behaviour on each side, concretely enough to tell
  whether you are looking at the old one.
- **Why** — which rule this serves. "Rule 0" alone is not a reason; name the
  inconsistency, the dead feature, or the second model being collapsed.
- **CLI check** — the `spectra` invocation that shows it, or an honest note
  that none applies and what covers it instead.
- **Verification** — the three suites at the time of the change.


## 2026-09-21 — Remove endpoint kind and its duplicate selection rules

- **Before:** Endpoint rows carried an overlapping `kind` classification alongside
  API and capabilities. CLI selectors mixed categories with abilities, and pending
  polling assembled a separate unfiltered primary/supplemental list.
- **After:** Remove `kind` from TOML, core/FFI records, probes and CLI output.
  Monero menu choices select `monero-light-wallet`. Browser links omit API and
  declare no capabilities; explorer labels enable transaction buttons, while
  unlabeled pages stay ordinary links. API rows reject link fields.
  Existing transaction explorer buttons keep their URLs. Endpoint settings show
  API names plus localized capabilities; browser rows show only their URL.
  CLI reads/sends select compatible APIs and declared capabilities; ENS uses its
  registry-selected primary API, and pending polling reuses the catalog's API
  lists. Remove category mask constants, obsolete category tests and translations.
- **Why:** The categories described different, overlapping properties. API,
  abilities and existing browser-link metadata already express each actual need.
- **CLI check:** `spectra --json endpoints --catalog` has no `kind` fields;
  API records keep their wire contracts and browser records have null API and
  empty capabilities. The offline acceptance suite checks compatible selection;
  core tests check backend/link separation and reject malformed links or old kind.
- **Verification:** `make verify` passed: formatting/clippy, workspace Rust
  tests, 423 CLI acceptance checks (including dead-code/translation checks),
  and 106 iOS simulator tests. Swift and Kotlin bindings regenerated.

## 2026-09-20 — Replace chain fetch aliases with shared API dispatch

- **Before:** API tags filtered URLs, but balance, history, transaction status
  and rebroadcast still repeated chain-to-client matches. Five Blockbook marker
  types/files existed only to attach different signers to the same REST client.
  Each native balance branch also selected its own display formatter.
- **After:** These operations select their adapter from the active endpoint's
  API. Unknown custom URLs use the registry's existing default API. Remove the
  five alias files and PhantomData/marker trait; one Blockbook client carries
  the concrete network for address normalization. Chain-specific signing stays
  in send modules, with an early network guard replacing the phantom-type
  restriction. Nine REST readers share base-path GET and fallback handling.
  API serde names use the common kebab-case convention instead of 35 repeated
  rename attributes. Native summaries format smallest units once using registry
  decimals, preserving tiny amounts formerly truncated by client display text.
  Blockbook history normalization also covers Bitcoin Gold, Dash and Zcash;
  previously their fetched entries were dropped by the chain-shape lookup.
- **Why:** Replace repeated dispatch and protocol wrappers instead of adding
  another table around them. Chain signing rules and indexer requirements remain
  explicit; an API label does not manufacture missing protocol implementations.
- **CLI check:** `python3 scripts/cli-history.py target/debug/spectra
  HistoryTests.test_blockbook_history_is_shared_across_networks` reads Dash and
  Zcash through the same local Blockbook fixture. Core mock tests exercise all
  five networks, BCH testnet prefix normalization, wrong-network signer refusal
  before requests, and a one-wei balance. Full CLI acceptance includes the new
  history test.
- **Verification:** `make verify` passed: formatting/clippy, workspace Rust
  tests, 423 CLI acceptance checks and all 106 iOS simulator tests.

## 2026-09-20 — Declare endpoint APIs and select compatible adapters

- **Before:** Endpoint kinds described broad categories but not wire formats.
  Primary fallback lists mixed incompatible APIs (for example Esplora with
  Blockchain.info, Tron HTTP with JSON-RPC, and Blockbook clients with
  Blockchair URLs). Bitcoin and TON also selected URLs by hard-coded row IDs.
- **After:** Every API row declares a validated `api` enum; web links omit it.
  Core selects each service slot by its client's API and selects primary bases
  with balance capability, so operation-specific URL prefixes are not treated
  as bases. Bitcoin Esplora and TON v3 selection use API types, not row IDs.
  CLI catalog output includes `api` and the actual built-in `configured` lists.
  Health checks choose JSON-RPC methods from the API, not the broad `kind`.
  TronGrid HTTP explicitly declares balance, served by `/wallet/getaccount`.
  Eight clients share JSON-RPC request/error/fallback handling, preserving
  XRPL's envelope and Monero/NEAR's string request IDs. Null `error` values are
  accepted when a result exists; malformed results still fail.
- **Known gaps now explicit:** Litecoin and Bitcoin Cash currently have
  Blockbook clients but no Blockbook catalog URLs; Monero has a wallet-RPC
  client but only light-wallet catalog URLs. These primary lists are empty
  instead of sending incompatible requests. Polkadot's Subscan web link is not
  an API fallback. The HyperEVMScan homepage is corrected to a web link with no
  history claim. No new provider or unsupported API adapter is implied.
- **Custom configuration:** Known catalog URLs with incompatible APIs or
  operation-only paths are rejected before storing/replacing configuration.
  Unknown custom URLs are still interpreted using the selected service slot's
  API contract; `api` is not inferred from a hostname.
- **Why:** A capability says what can be done, not how to encode it. Explicit
  API contracts prevent invalid fallbacks and allow shared transport code
  without erasing chain-specific methods and response decoding.
- **CLI check:** `spectra --json endpoints --catalog --chain Bitcoin` shows
  `esplora` and only compatible configured bases. Repeat for TON to see v2/v3
  separated, or Litecoin to see the missing compatible configuration. Offline
  CLI acceptance checks these routes and the absence of API tags on web links;
  mock HTTP tests cover RPC dialects, errors and fallback.
- **Verification:** `make verify` passed: formatting, clippy, Rust tests, all
  423 CLI acceptance checks and iPhone simulator tests.

## 2026-09-20 — Finish the Swift shell boundary review

- **Before:** The send-authentication toggle also disabled app unlock, wallet
  deletion and data-reset authentication. Deletion/reset could proceed when
  configured device authentication was unavailable.
- **After:** Native authentication has explicit unlock, send, delete and reset
  actions. The send toggle affects only sends/rebroadcasts. Other protected
  actions require configured device authentication and fail closed if unavailable.
- **Why:** A preference for one action must not disable unrelated protection.
  LocalAuthentication and device-local preferences remain platform concerns.
- **CLI check:** Native device authentication has no CLI equivalent;
  `DeviceAuthenticationTests` covers the action-policy matrix. Core password
  protection is independently exercised by `python3 scripts/cli-send.py target/debug/spectra`.

- **Before:** Swift selected and started Tor transports; CLI commands could load
  Tor settings without activating them. Switching embedded Tor to a custom proxy
  failed, and a late bootstrap could reinstall a proxy after stop/switch.
- **After:** Registering the platform cache directory starts a core-owned runtime.
  Committed setting changes and settings reset reconcile that runtime. Reconnect
  reads stored configuration. Bootstrap tasks are canceled and late completions
  rejected; the embedded listener binds a free port before reporting ready.
  HTTP client selection shares the transport-switch lock so a kill-switch check
  cannot race with replacement by a direct client.
  CLI network services initialize stored routing and await bootstrap (up to 120s).
  Editing settings remains offline so a failed transport can be disabled.
- **Why:** Transport configuration and execution need one owner across clients.
- **CLI check:** `spectra --json tor [--reconnect]` initializes/reports configured
  transport. `python3 scripts/cli-transport.py target/debug/spectra` proves a fresh
  process routes through the saved local SOCKS proxy with remote DNS, and honors
  disabling it. `cargo test -p spectra_core --test transport_runtime` checks live
  setting changes/reset without shell callbacks; the bootstrap regression checks
  cancellation and rejection of obsolete completions without public network.

- **Before:** Swift sent previews and separately reconstructed asset metadata
  back across FFI for shortcut amounts/details, and classified destination risk
  from two booleans. Balance display rounding could classify one wei as zero.
- **After:** An owned preview includes wallet/holding/network identity, display
  details and core-derived 25/50/75/MAX amounts. Swift rejects mismatched quote
  identities and never shares mainnet/testnet preview slots. A native-fee-only
  preview for a token no longer labels the gas balance as token spendable/MAX. Core classifies
  destination activity using validated raw smallest units on the wallet's network;
  Swift only localizes the resulting enum.
- **Why:** Presentation must not reconstruct asset identity or drive risk rules.
- **CLI check:** `spectra --json send preview --wallet <wallet> --holding <deployment>
  --amount 1` includes `shortcuts` and `details`; `spectra --json send probe
  --wallet <wallet> --asset ETH --to <address>` includes `activity`.
  `scripts/cli-send.py` verifies bound quote amounts; `scripts/cli-transport.py`
  distinguishes zero balance with history from a funded address holding one wei.

- **Before:** One wide FFI import-draft record modeled five UI modes, including
  renaming and the backup quiz, duplicating the Swift form's structure.
- **After:** Remove the draft enum/record/validator. Swift owns form completeness
  and backup navigation; it reuses core mnemonic/private-key validators. A nonempty
  watch-address form can submit and show core's validation outcome. Import/rename
  operations continue to validate before persistence regardless of UI gating.
- **Why:** Front-end form structure is not an authoritative domain model.
- **CLI check:** `python3 scripts/cli-wallets.py target/debug/spectra` and
  `scripts/cli-acceptance.sh` exercise imports, malformed secrets/addresses and
  persisted renames without the removed draft API. Core validation/import tests
  replace the deleted UI-draft tests.
- **Verification:** `make verify IOS_TEST_DERIVED_DATA=/private/tmp/spectra-shell-boundary-derived
  IOS_TEST_DEST='platform=iOS Simulator,id=E3D8C2FB-841F-45E5-9ED2-7E40EFA89FE5'`
  passed: rustfmt/Clippy, 848 Rust unit tests plus the independent transport
  integration test, 422 CLI acceptance checks and 106 iOS tests. The isolated
  build directory avoids a concurrent build's database lock.


## 2026-09-20 — Remove unused endpoint provider metadata

- **Before:** Endpoint rows required `provider_id` and exported it as `providerID`
  in JSON and `providerId` in Swift, mixing operator names and API types.
- **After:** Remove the field from TOML, core records and generated bindings.
  Endpoint identity, selection and request behavior are unchanged.
- **Why:** No business logic consumed this inconsistent descriptive metadata.
- **CLI check:** `spectra --json endpoints --catalog` no longer emits `providerID`.
- **Verification:** `make verify` passed formatting, clippy, Rust tests and CLI
  acceptance. iOS built successfully, but the 106-test suite reported two
  assertions in `testTorDoesNotActivateOrStopForAnUncommittedToggle`
  (`stopped` versus `ready`); the workspace also contains separate Tor lifecycle
  edits. Full verification remains blocked by that test.

## 2026-09-20 — Rename native-history to history

- **Before:** Native-coin address history used the `native-history` capability.
- **After:** The catalog, filters, CLI output and localization keys use `history`;
  `native-history` is no longer accepted. `token-history` remains separate, and
  `history` still means native-coin address history only.
- **Why:** Use the shorter requested name without merging native and token
  capabilities or changing which endpoints can serve them.
- **CLI check:** `spectra --json endpoints --catalog --chain Bitcoin` exposes
  `history`; `spectra --json endpoints --catalog` contains no `native-history`.
  CLI acceptance also checks the native/token distinction for Ethereum and TON.
- **Verification:** `make verify` passed: formatting, clippy, Rust tests, CLI
  acceptance and iPhone simulator tests.

## 2026-09-20 — Remove the generic read capability

- **Before:** 110 endpoint records declared `read`, a generic label shown in
  settings and diagnostics that no business operation used to select endpoints.
- **After:** Remove `read` from the catalog, accepted capability vocabulary,
  filter mapping and localized labels. Nine concrete capabilities remain.
- **Why:** The label had no defined operation and duplicated more precise
  capabilities such as balance and history.
- **CLI check:** `spectra --json endpoints --catalog` lists only the remaining
  capabilities; CLI acceptance checks the concrete capability lists for EVM,
  Solana and TON endpoints.
- **Verification:** `make verify` passed after clearing the project and Xcode
  Rust build caches: formatting, clippy, Rust tests, CLI acceptance and iPhone
  simulator tests.

## 2026-09-20 — Show the complete endpoint catalog in settings

- **Before:** `settings_visible` hid auxiliary APIs and transaction explorer
  links. Bitcoin and EVM views applied further independent filters, and custom
  Bitcoin or Monero endpoints could replace the displayed built-in list.
- **After:** Remove the visibility flag and lookup argument. Settings renders
  core's complete, deduplicated directory grouped by network, including web
  links. Custom endpoint controls remain alongside the full catalog.
  Transport selection still uses endpoint kinds and capabilities.
- **Why:** Visibility has no independent domain rule; maintaining a second
  catalog by hand makes the app's service directory incomplete.
- **CLI check:** `spectra --json endpoints --catalog --chain bitcoin` now includes
  `settingsGroups`, the same core projection rendered by Swift. CLI acceptance
  checks that its URLs match every catalog record for Bitcoin, Ethereum,
  Sepolia and Monero.
- **Verification:** `make verify` passed: formatting, clippy, Rust tests, CLI
  acceptance and iPhone simulator tests.

## 2026-09-20 — Unify domain names and remove obsolete naming layers

- **Before:** Wallet projections/settings mixed `network_chain` with `chain`;
  deployment records were named `TokenEntry` with ambiguous `id` and `chain`
  fields; custom deployments stored display names. CoinGecko keys, Swift ID
  spelling, free-function prefixes and raw/structured result names differed.
  Endpoint filters retained role names, obsolete aliases and CLI-owned bit values.
- **After:** Wallet chain identity uses `chain_id`; family selection uses
  `selected_chain_by_family`. Refresh entries distinguish `holding_chain_id`
  from the queried `chain_id`. `TokenDeploymentEntry` uses `deployment_id`,
  `token_id` and registry `chain_id` for both built-in and custom deployments.
  Preference keys and ordering consistently use chain IDs. Built-in hosting
  lookup resolves the ID through `Chain` before reading its display name, so
  IDs such as `bnb-chain` no longer silently omit their built-in tokens.
  CoinGecko keys use `coingecko_id` / `coingeckoId`; handwritten Swift uses `Id`.
  Functions omit `core_` / `app_core_`, structured functions omit `_typed`, and
  raw preview functions use `_json`. Endpoint kind and capability constants
  come from core; obsolete `rpc` / `explorer` filter aliases are rejected.
  The unused secret-store trait, redundant validation forwarding functions and
  misleading `wallet_core` module alias are removed. The endpoint Swift file
  is named after `AppEndpointDirectory`. Generated bindings are regenerated.
- **Why:** One name per identity or operation, explicit names for different
  identities, and one owner for endpoint filter definitions. Stored JSON shapes
  change directly without migration or compatibility aliases.
- **CLI check:** `spectra --json token catalog --chain ethereum-sepolia` emits
  `deployment_id` separately from `token_id`; `spectra --json endpoints --catalog
  --chain ethereum-sepolia` retains the concrete chain identity. Offline token
  preference checks cover custom deployment IDs, built-in token resolution,
  duplicate rejection, enable/disable, removal and persistence after reopening.
- **Verification:** `make verify` passed: formatting/clippy, 847 Rust tests,
  419 CLI acceptance checks and 102 iPhone simulator tests.

## 2026-09-20 — Use chain IDs consistently across catalogs and callers

- **Before:** Chain references in presentation, deployments, endpoints, wallet
  state and endpoint probes used `network_id` / `networkId`, despite resolving
  to registry `Chain` IDs. Artwork lookup exposed `--network-id`.
- **After:** These references use `chain_id` / `chainId`, including persisted
  wallet JSON, SQL JSON lookups, CLI output and generated Swift bindings.
  Artwork lookup uses `--chain-id` and `core_chain_artwork_name`. Stored shapes
  change directly, with no aliases or migrations. IDs and network selection
  behavior are unchanged. Protocol-defined Cardano network IDs and Rosetta
  `network_identifier` fields keep their protocol names.
- **Why:** Use one name for registry chain identity throughout its consumers.
- **CLI check:** `spectra --json endpoints --catalog --chain ethereum-sepolia`
  emits `chainId: ethereum-sepolia`; `spectra --json token artwork --chain-id base`
  returns `artworkName: base`. CLI acceptance also exercises stored wallet
  `chainId`, testnet selection and send identity after reopening state.
- **Verification:** `make verify` passed: formatting/clippy, 847 Rust tests,
  419 CLI acceptance checks and 102 iPhone simulator tests.

## 2026-09-20 — Name chain catalog tables consistently

- **Before:** `chains.toml` and `chain-ui.toml` used `[[networks]]` tables.
- **After:** Both use `[[chains]]`, with matching Rust deserialization fields.
  Mainnet and testnet records remain peers; record fields and IDs are unchanged.
- **Why:** Match the file names and the registry's `Chain` terminology without
  keeping a second spelling or compatibility alias for the same collection.
- **CLI check:** `spectra --json chains --testnets --filter sepolia` and
  `spectra --json token catalog --chain ethereum-sepolia` load the renamed tables.
- **Verification:** `make verify` passed: formatting/clippy, 847 Rust tests,
  419 CLI acceptance checks and 102 iPhone simulator tests.

## 2026-09-20 — Remove redundant native suffixes from testnet token IDs

- **Before:** Testnet token IDs used names such as `bitcoin-testnet-native`
  and `ethereum-sepolia-native`, unlike mainnet token IDs.
- **After:** All 32 testnet token IDs and their deployment references omit
  `-native`, becoming `bitcoin-testnet` and `ethereum-sepolia`, for example.
  Derived deployment IDs still use `:native` to identify native deployments.
- **Why:** Native versus contract is a deployment property; the token ID does
  not need to repeat it. Testnet identities remain distinct from mainnet ones.
- **CLI check:** `spectra --json token catalog --chain ethereum-sepolia`
  returns `token_id: ethereum-sepolia` and `id: ethereum-sepolia:native`.
- **Verification:** `make verify` passed: formatting/clippy, 847 Rust tests,
  419 CLI acceptance checks and 102 iPhone simulator tests.

## 2026-09-20 — Resolve and persist derivation paths per concrete network

- **Before:** Every testnet catalog had an empty path list and both path lookup
  and custom overrides fell back to the mainnet. Import stored the same path
  beside every network's address, and signing used one wallet-wide path.
- **After:** Every path-based network declares its own templates. Bitcoin's
  three test networks, Litecoin, BCH, BSV, Dogecoin, Zcash, Decred and Dash
  testnets use hardened coin type 1. Other testnets explicitly keep their
  existing chain-specific paths (including EVM, Sui, Aptos, Cardano and Kaspa).
  Monero remains pathless. Defaults/custom overrides are keyed by network ID.
  Import persists the actual path beside each derived address; signing,
  discovery, network switching and wallet detail read the recorded network
  path. CLI wallet summaries use the active network address; a missing testnet
  address no longer falls back to a mainnet address. Partial caller overrides
  are completed before derivation. Existing
  explicit wallet paths are not rewritten or migrated. A pathless network is
  derived with an empty path rather than silently skipped at import.
- **Why:** A network switch must not substitute a mainnet key or lose the path
  needed to reproduce its sender. Tests check all network identities and a
  sealed Bitcoin wallet after reopen, custom overrides and network switches.
- **References:** [SLIP-0044](https://github.com/satoshilabs/slips/blob/master/slip-0044.md)
  reserves coin type 1 for testnets; [BIP-84](https://github.com/bitcoin/bips/blob/master/bip-0084.mediawiki)
  defines the native SegWit path. Network-independent implementations remain
  explicit, such as [Sui](https://github.com/MystenLabs/ts-sdks/blob/main/packages/sui/src/keypairs/ed25519/keypair.ts)
  and [Kaspa](https://github.com/kaspanet/rusty-kaspa/blob/master/wallet/keys/src/derivation/gen1/hd.rs).
- **CLI check:** Import a Bitcoin seed wallet, run `spectra network set
  bitcoin-testnet-4`, inspect `spectra --json wallet show NAME`, and run
  `spectra --json send identity --from NAME --chain bitcoin-testnet-4`.
  The path is `m/84'/1'/0'/0/0` and the stored address matches the signer.
- **Verification:** `make verify` passed: formatting/clippy, 847 Rust tests,
  419 CLI acceptance checks and 102 iPhone simulator tests.

## 2026-09-20 — Distinguish testnet native coins in display symbols

- **Before:** Mainnet and testnet native assets shared display symbols such as
  BTC, ETH, SUI and APT despite having separate network/deployment identities.
- **After:** All testnet native display symbols carry a lowercase `t` prefix:
  `tBTC`, `tETH`, `tSUI`, `tAPT`, etc. Core supplies these labels to holdings,
  history and send presentation; network search aliases include them. Mainnet
  symbols and all asset/network IDs are unchanged. This is an app display
  convention, not a claim that these are every network's official tickers.
- **Why:** The amount itself identifies test coins instead of requiring the
  user to infer that only from a network label. Test coins remain unpriced.
- **CLI check:** `spectra --json token catalog --chain bitcoin-testnet-4` and
  `spectra --json token catalog --chain ethereum-sepolia` show `tBTC` and `tETH`.
- **Verification:** `make verify` passed: formatting/clippy, 847 Rust tests,
  419 CLI acceptance checks and 102 iPhone simulator tests.

## 2026-09-20 — Keep EVM membership in the Rust registry

- **Before:** Each `chains.toml` row configured `is_evm`; registry behavior and
  the exported catalog read that flag.
- **After:** `Chain::is_evm()` defines membership directly in Rust without
  reading the catalog. The TOML flag is removed and rejected if reintroduced.
  Catalog rows project the registry result using the existing enum/catalog
  ordering, with a count check and the independent registry order test. This
  avoids recursive catalog initialization. CLI/Swift keep receiving `isEvm`.
- **Why:** Supported chain implementations and routing live in the registry;
  membership belongs alongside them, not in editable network or UI metadata.
- **CLI check:** `spectra --json chains --filter Bitcoin` reports `isEvm: false`;
  `spectra --json chains --testnets --filter "Ethereum Sepolia"` reports true.
  Registry tests check membership against exhaustive address validators;
  catalog tests check the exported result and independence from UI categories.
- **Verification:** `make verify` passed: formatting/clippy, 843 Rust tests,
  406 CLI acceptance checks and 101 iPhone simulator tests.

## 2026-09-20 — Separate network presentation from operational facts

- **Before:** `chains.toml` mixed network rules with search terms, category,
  color, artwork, picker rank and address examples. The exported EVM flag came
  from display category, while `registry::Chain` maintained a separate EVM list.
- **After:** Those six presentation fields live in `chain-ui.toml`, with one
  `network_id` reference per concrete network. Core joins by ID independently
  of UI row order and rejects missing, duplicate or unknown references and
  misplaced/unknown fields. `chains.toml` retains identity, name, family,
  environment, token standard, holdings capability and multiline derivation
  paths, and explicitly owns `is_evm`. Registry decisions and exported EVM
  flags read that single fact; editing display categories cannot change it.
  The public catalog and Swift bindings keep their existing record shape.
- **Why:** Presentation can be edited without changing protocol decisions;
  network rules have one source rather than a UI classification and a Rust list.
- **CLI check:** `spectra --json chains --filter Bitcoin` exposes `isEvm: false`
  and the UI-owned `popularRank: 1`; `spectra --json chains --testnets --filter
  "Ethereum Sepolia"` exposes `isEvm: true`. Acceptance checks cover both;
  core tests cover UI reordering, category changes and invalid references.
- **Verification:** `make verify` passed: formatting/clippy, 843 Rust tests,
  406 CLI acceptance checks and 101 iPhone simulator tests.

## 2026-09-20 — Derive each network's native deployment reference

- **Before:** Every network in `chains.toml` repeated its native deployment ID
  in `native_deployment`, and core checked it against the derived deployment ID.
- **After:** The configuration field is removed. Core resolves the native
  deployment using `<network_id>:native`; missing native deployments, duplicate
  deployment IDs and incorrect network/kind associations remain rejected.
  Runtime/FFI records still expose the resolved native deployment ID.
- **Why:** The reference follows directly from network identity, so storing it
  separately adds redundant data and validation without expressing a choice.
- **CLI check:** `spectra --json chains --filter arbitrum` and
  `spectra --json chains --testnets --filter sepolia` expose the derived
  `nativeDeploymentId` for mainnets and testnets.
- **Verification:** `make verify` passed: formatting/clippy, 837 Rust tests,
  404 CLI acceptance checks and the iPhone simulator test suite.

## 2026-09-20 — Peer token and deployment tables in both token catalogs

- **Before:** `tokens.toml` and `testnet-tokens.toml` nested deployments under
  `[[tokens.deployments]]`, so their owning token depended on table position.
  Deployments stored their concrete network ID in `network`, while endpoint
  records already used `network_id`.
- **After:** Both files contain independent `[[tokens]]` and `[[deployments]]`
  tables, arranged with each token immediately followed by its deployments
  for editing. Each deployment explicitly references `tokens.id` with `token_id`
  and `chains.toml`'s concrete network ID with `network_id`. Core joins by ID,
  independent of token-definition order, and emits entries in deployment order.
  Empty/duplicate token IDs, dangling references, tokens with no deployments,
  unknown fields and the old nested shape are rejected. Existing network,
  environment, contract, precision, native identity, duplicate deployment and
  testnet pricing checks remain enforced. Deployment IDs remain derived; no
  compatibility reader or migration is added. Runtime/FFI record shapes do not
  change.
- **Why:** Token identity and deployment identity are separate entities; an
  explicit reference expresses the relationship without structural nesting or
  dependence on the nearest preceding token table.
- **CLI check:** `spectra --json token catalog --chain arbitrum` resolves native
  ETH to `token_id: ethereum`; `spectra --json token catalog --chain ethereum-sepolia` resolves `ethereum-sepolia:native` to the unpriced token
  `ethereum-sepolia-native`. The CLI acceptance suite covers both catalogs;
  core tests cover reordered definitions, invalid references, legacy schema,
  and network/asset-integrity failures.
- **Verification:** `make verify` passed: Rust formatting/clippy, 837 Rust
  tests, 404 CLI acceptance checks, and 101 iPhone simulator tests.

## 2026-09-20 — Endpoint ownership references concrete network IDs

- **Before:** Endpoint rows repeated `chain_name`; testnet ownership was hidden
  in `group_title`. Core inferred network identity from display text, maintained
  a second name-based settings index, and Swift indexed the result by name/title.
- **After:** Every endpoint references `network_id` from `chains.toml`.
  Unknown IDs and old ownership fields are refused at load time, without aliases
  or migrations. One network-ID index serves exact-network queries. Settings
  groups derive their family relation and display title from the registry;
  a testnet request includes only that testnet, while a mainnet settings section
  includes its family. Swift uses IDs for lookup and group identity. The unused
  catalog chain-name projection is removed. CLI catalog output and live probe
  output identify the network with `networkId`. EVM diagnostics now uses the
  selected network ID when listing configured catalog endpoints.
- **Why:** Display text is not network identity. The registry already owns the
  mainnet/testnet relation, and an endpoint must not switch networks because a
  title changes.
- **CLI check:** `spectra --json endpoints --catalog --chain ethereum-sepolia`
  reports `networkId: ethereum-sepolia`; the same command with `--chain ethereum`
  contains no Sepolia endpoints. `scripts/cli-acceptance.sh` also checks Testnet4
  and rejection of unknown networks. Core tests reject invalid IDs and the old
  name/title fields and verify exact ownership and family grouping across the
  registry. iOS tests verify ID-based lookup and settings groups.
- **Verification:** UniFFI bindings regenerated with `scripts/bindgen-ios.sh`;
  `make verify` passed (Rust formatting/clippy, 834 Rust tests, 398 CLI checks,
  and the full iPhone simulator suite, including ID-based Ethereum network
  lookup and grouping).

## 2026-09-20 — Explicit native and token endpoint capabilities

- **Before:** `history` did not distinguish native-coin transactions from token
  transfers, and no endpoint capability described token holdings or balances.
  NEAR RPC rows also claimed address history even though the client uses an
  indexer for that operation.
- **After:** Remove `history`; declare `native-history` and `token-history`
  separately. Add `token-discovery` and `token-balance` to audited API surfaces.
  `balance` explicitly means native-coin balance. Token history means fungible
  token transfers, not approvals. NEAR RPC retains token-balance but loses its
  incorrect history declaration. CLI native-history masks use the new name;
  all new capabilities have independent mask bits. Unknown capability names
  fail catalog loading. English, Simplified Chinese and Traditional Chinese
  endpoint summaries display the new labels. `endpoints --catalog` lists the
  registered API capabilities without network or health results; capability
  acceptance checks use this instead of accidentally probing live providers.
  No FFI shape changes are needed.
- **Why:** A node that can answer `balanceOf` cannot necessarily enumerate token
  holdings or address transfers. A capability must describe the registered API,
  independently of other APIs on the same chain. The declarations and their
  scope/evidence are in [ENDPOINT-CAPABILITIES.md](ENDPOINT-CAPABILITIES.md).
- **CLI check:** `spectra --json endpoints --catalog --chain Ethereum` shows token-balance
  on RPC rows and separate history/discovery on indexers;
  `spectra --json endpoints --catalog --chain TON` distinguishes v2 native history from
  v3 jetton capabilities. `scripts/cli-acceptance.sh` asserts these declarations
  offline and rejects any remaining `history` capability.
- **Verification:** `make verify` passed: Rust formatting/clippy, 832 Rust
  tests, 392 CLI acceptance checks, and the full iPhone simulator suite,
  including endpoint localization and Ethereum test-network endpoint tests.
  Mock servers and Xcode require running this gate outside the restricted sandbox.

## 2026-09-20 — Isolated platform tests and explicit service lifetime

- Before: AppState adapters always used the shared service, and tests cleared
  simulator-wide state. `startServices: false` skipped launch but a later wallet
  projection or import could still start automatic maintenance or refresh.
- After: AppState and diagnostics use an injected service (the shared service
  remains the application default). Disabling services also disables automatic
  work following wallet changes/imports or currency changes. Tests use a fresh database, an offline
  service and in-memory secrets, and explicitly load projections or await writes.
- Rationale: make test order and simulator history irrelevant; retain actual
  Keychain tests with unique accounts. Business matrices remain in core, with
  representative FFI checks and platform behavior in Swift. Encryption tests
  distinguish malformed envelopes from valid-length authenticated-data tampering.
- CLI check: `cargo test -p spectra_core store::seed_envelope` and
  `cargo test -p spectra_core display_decimals_use_known_deployment_or_custom_precision`;
  `make test-cli` checks the unchanged domain behavior. `make test-ios` verifies
  injection, deterministic delete/late-rename ordering and reopened persistence.
  Concurrent Xcode builds can use `make verify IOS_TEST_DERIVED_DATA=/tmp/spectra-test-audit-derived-data` to avoid sharing the build database.

## 2026-09-20 — Explicit network identity and current domain names

- **Before:** `Chain::evm_chain_id` returned Ethereum's `1` for every non-EVM
  chain. **After:** it returns an error for non-EVM chains, propagated before
  constructing an EVM client. The unused `EvmChain` wrapper was removed.
  **Why:** unsupported networks must never acquire a plausible signing identity.
  **CLI check:** `spectra send assemble --chain Bitcoin --from
  0x1111111111111111111111111111111111111111 --to
  0x1111111111111111111111111111111111111111 --amount 1` must refuse the request
  (exit 3), as covered by the offline acceptance suite. The registry test
  `non_evm_chains_have_no_eip155_identity` covers every registered non-EVM chain.
- **Before:** `WalletView.selectedChain` named a family, while `chainId` named
  the actual network. Conversion silently substituted a family mainnet for an
  invalid/mismatched network. **After:** the field is `familyName`; conversion
  refuses unknown networks, unknown families and family/network mismatches.
  Its primary address is resolved from the actual network, and that network's
  address slot sorts first when creating domain state.
  **Why:** a display label must not silently change a wallet's network.
  This directly changes the projection's serialized shape; no alias or migration
  is retained. **CLI check:** the offline acceptance suite's "testnet derivation
  identity" section imports on Testnet4, checks the stored path/address, reopens
  the signer and switches back to mainnet. Core conversion tests additionally
  cover invalid and mismatched identities.
- **Naming and ownership cleanup:** polling, keypool merging, history record
  construction, price-alert evaluation and send preflight validation now name
  their actual operations instead of historical planner wrappers. Real import
  and maintenance plans remain plans. `CoreReferenceTables` names the static
  reference cache; selection-dependent network titles are separate from wallet
  and transaction titles. Removed unused network query wrappers and the unused
  wallet eligibility record. Send checks live in `send_preflight`, refresh
  scheduling in `maintenance`, and signing/submission in `send_submission`.
  Swift network writes live with the network flow; projection reloads name
  their actual purpose. **Check:** `make verify`, including regenerated UniFFI
  bindings and iOS network/title coverage.
