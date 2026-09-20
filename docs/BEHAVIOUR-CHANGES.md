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
