# Stage 3 / C2 closure audit

Scope: core owns domain state and decisions; CLI drives the same operations;
Swift holds rendering, editing, platform callbacks and projection adoption.
Android and the separately planned visible Build / Sign / Broadcast feature are
outside these migration stages. Provider availability remains the explicit
external dependency recorded in PLAN.

## Swift ownership follow-up

The earlier completion claim was withdrawn because passing tests and zero unused
exports did not prove that Swift was thin. The follow-up now removes the audited
remaining decisions; all final gates passed on 2026-09-12.

- [x] Alert add/toggle/remove are core intents; no cents rounding, fiat conversion,
  trigger mutation or whole-list alert writer remains in Swift.
- [x] One owned send operation replaces every Swift execution branch. Fees,
  fallback policy, token identity, affordability and atomic fee conversion are core.
- [x] One owned preview operation replaces protocol routing and source selection.
  Polkadot/other watch previews do not require a Swift seed read.
- [x] Core resolves display addresses for the stored network; missing selected
  addresses never use another network's slot. Import awaits projection readiness.
- [x] Core owns the all-wallet self-send set and acknowledgment expiry.
- [x] Replacement/cancel drafts use stored transactions and core fee estimates;
  no Swift zero-transfer composition, eight-decimal truncation or 4/2 gwei fallback.
- [x] Remove Swift background orphan deletion, local-first launch merging and
  production whole-record mutation adapters used only by tests.
- [x] Delete `AppState+SendRouting.swift`, obsolete preview wrappers, the unused
  Swift derivation-path builder and obsolete FFI exports; regenerate bindings.

### Remaining Swift responsibilities checked

| Files / area | What remains |
|---|---|
| `AppState+SendExecution`, `AppState+SendPreview` | User input snapshots, biometric/confirmation presentation, in-flight flags, stale-result protection, forwarding and result adoption |
| `AppState+SendFlow`, `SendPreviewTypes` | Composer selections, typed-input validation through core, preview accessors, labels and editable draft fields; no protocol execution request assembly |
| `AppState+OperationalTelemetry`, `PriceAlertsView` | Notification presentation, alert intent forwarding, transient self-send acknowledgment |
| `AppState+AddressResolution`, `WalletDerivedCache` | Read core's validated address projection; memoize stateless core input validation |
| `AppState+ReceiveFlow`, `AppState+ImportLifecycle` | Import/receive editing, platform authentication, core import/receive calls; transaction-detail address unions are read-only presentation indexes and never authorize sends |
| `AppState+CoreStateStore`, `PersistenceStore`, `StorePersistenceNormalization`, `StoreLifecycleReset` | Field intents, core reset, committed projection adoption and platform-only preferences; no app-side wallet/history upsert helpers or orphan deletion |
| `AppState+History`, `AppState+BalanceRefresh`, `AppState+PricingFiat` | Core refresh/query calls, observer callbacks and view caches |
| `AppState+DiagnosticsEndpoints`, `AppState+TorLifecycle`, `AppState+FundsFinder` | Platform lifecycle, core diagnostics/transport/scan calls, progress presentation |
| `Store+Settings`, `Store+Notifications`, `Store+Formatting`, views | Editable settings drafts, local notifications, text/number formatting and totals for the visible selection; these never supply authoritative balances or fees to a send |

No protocol constants need to move into Swift. UI layout, text, animation,
selection, input debounce and platform authentication intentionally stay there.
The audit concerns authoritative domain decisions, not eliminating all Swift
conditionals or all view-side arithmetic.

## Implemented closure

- [x] History: `refresh_history` selects stored wallets, their protocols, pagination
  and successful refresh clocks. Empty explicit scopes do no work; failed work
  does not consume the cooldown. CLI `history --save` uses this operation.
  Swift's protocol descriptors and separate history implementation are deleted.
- [x] Reset: `reset_data` validates scopes, awaits secret and domain cleanup,
  clears history, quotes and refresh clocks, and returns the committed projection.
  Errors are explicit; steps are idempotent and retryable, not one transaction
  spanning Keychain and SQLite. Swift retains authentication and device cache cleanup.
- [x] Receive/discovery: core chooses the stored network, address and derivation,
  reserves and records addresses. Swift does not read a mnemonic or construct a
  Bitcoin path. Xpub receiving remains supported; unsupported custom range
  algorithms use the validated stored address. Chain discovery selects wallets
  and networks inside core and reports each failure separately.
- [x] Quotes: alert evaluation and dashboard grouping consume core quotes.
  Trigger updates serialize with other state writes and survive reopening.
- [x] Transport: `new_catalog` constructs protocol endpoint slots from registry
  facts. Requests read committed RPC settings; Etherscan reads the owned setting.
  Explicit CLI/test endpoint overrides remain explicit and suppress catalog/settings
  fallback. Swift no longer builds or resubmits endpoint lists.
- [x] History diagnostics use the same owned refresh as normal history, including
  its saved result and failures; the second Swift protocol dispatcher is removed.
- [x] FFI audit: 162 callable exports (178 before this closure), zero unreachable
  candidates. Generated bindings are regenerated, never hand-edited.
- [x] Final gates passed on 2026-09-12: Rust workspace **841 tests**; offline CLI
  **346 checks plus both Stage 3 fixture batches**; iPhone 17 Pro **84 tests**.
  All commands exited 0. Both `testEthereumTestNetworksExposeExpectedContextsAndEndpoints`
  and `testOwnedClosureOperationsAcrossAsyncBinding` passed.

## Retained boundaries and why

| Boundary | Ownership and evidence |
|---|---|
| Wallet/import/settings/token/address book commands | Core validates and persists; Swift holds editable fields and adopts returned state. CLI wallet/settings/token/address commands exercise the same writers. |
| Send previews and execution | Distinct typed protocol inputs represent user edits. Routing, preflight, derivation, signing, journal, submission and status mutation are core operations; CLI send/txs cover them. Biometrics, confirmation sheets and text formatting remain device UI. |
| Maintenance and balance observer | Core owns scheduling decisions, wallet selection, balances and pending sweeps; Swift supplies activity/connectivity conditions and renders callbacks. |
| History cursor reads/reset and query projections | Read/reset intent and pagination UI need these; callers no longer manufacture page advances or successful clocks. |
| Keypool and owned-address diagnostics | Core owns reservations and address tables. Swift lists diagnostic rows and reads core results; it does not reserve on startup. |
| Price/movement notifications | Core persists price alert triggers. Notification authorization and delivery, device session movement baselines and display formatting remain platform responsibilities. |
| Generic platform preference blob | `PersistenceStore` uses the generic store for device preferences; authoritative domain collections have typed core storage and commands. |
| Endpoint/catalog rendering | Settings and diagnostics need labels, URLs and capability rows. Network dispatch/configuration is core-owned; rendering catalog data is retained. |
| SecretStore and BalanceObserver foreign protocols | Platform storage/callback implementations are intentional FFI entry points, not unused exports. |

The roughly 150-export target is diagnostic. The final boundary has 157 callable
exports and zero unreachable candidates. Typed protocol preview records remain
in core; Swift receives their existing tagged result through one owned operation.

## Reproducible verification

- `cargo test --workspace`: includes owned history scope/failure clocks, durable
  reset/refusal, passphrase derivation, concurrent alert evaluation/reopen, read-only
  receive projection and stored transport/explicit override tests.
- `./scripts/cli-acceptance.sh`: adds stored dashboard and alert evaluation,
  chain discovery, receive, invalid reset scopes and reset/reopen; retains the
  local Bitcoin fixture (61 transactions over seven UI pages and three provider
  pages), prior Stage 3 fixtures, send and status coverage.
- `cd swift && xcodebuild test -scheme Spectra -destination 'platform=iOS Simulator,name=iPhone 17 Pro'`:
  the new async binding test exercises history, alerts, discovery, receive refusal
  and reset through UniFFI/Tokio; all existing iOS tests remain required.
- `scripts/count-exports.sh` and `scripts/unreachable-exports.sh`.


## Final gate results (2026-09-12)

- Rust workspace: **851 passed, zero failed**.
- CLI acceptance: **347 passed**, including Stage 3/follow-up fixture batches and
  the new owned-send/alert/replacement loopback fixture.
- iPhone 17 Pro: **84 passed, zero failed**; the required Ethereum testnet
  contexts/endpoints test and the new subcent alert-intent bridge test passed.
- FFI: **162 → 157** callables, **zero** unreachable candidates; Xcode regenerated
  the bindings using the repository generator.
- `git diff --check`: clean. No Git staging or commit performed.

Gate logs for this run: `/tmp/spectra-clean-rust-gate.log`,
`/tmp/spectra-clean-cli-gate.log`, `/tmp/spectra-clean-ios-gate.log`.


## Subsequent shell audit (2026-09-13)

The earlier figures describe their own run. Direct caller inspection found
residual issues, addressed by PLAN's Swift shell ownership follow-up: owned
naming and durable movement baselines, current staking transport, receive
addresses separate from messages, immutable historical labels and unused
Swift/FFI removal. The retained-boundary statement about device-session movement
baselines is superseded: core now persists them. Staking UI is explicitly
read-only; unconnected transaction and position/preview controls are removed.
Core transaction builders remain internal and do not constitute a complete
end-user staking flow.

Final follow-up gates: Rust **833**, CLI **354** plus fixture batches, iPhone
17 Pro **87**, all passed. The regenerated FFI has **145** callable exports and
zero unused candidates. Design-token and diff checks pass. Runtime smoke also
verified Solana validator loading and the unnamed watch-wallet receive QR.
See PLAN for the exact behavior changes, CLI commands, test scope and logs.
