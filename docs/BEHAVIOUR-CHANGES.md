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
