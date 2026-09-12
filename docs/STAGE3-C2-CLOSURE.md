# Stage 3 / C2 closure audit

Scope: core owns domain state and decisions; CLI drives the same operations;
Swift holds rendering, editing, platform callbacks and projection adoption.
Android and the separately planned visible Build / Sign / Broadcast feature are
outside these migration stages. Provider availability remains the explicit
external dependency recorded in PLAN.

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

The roughly 150-export target is diagnostic. Keeping 162 preserves useful typed
protocol operations and rendering/catalog queries; collapsing unrelated protocols
into a wide union would make the boundary less clear.

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
