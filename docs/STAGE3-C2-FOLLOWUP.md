# Stage 3 / C2: six follow-up slices

These six slices extend the previous ten-item batch. They do not mark all of
Stage 3, C2, or Android complete.

| Slice | Result | CLI and meaningful regression |
|---|---|---|
| Import | Core derives signing addresses, creates IDs, writes SecretStore, then commits all wallets together; errors clean partial secrets. | `wallet import`, `wallet create`; Rust tests inject secret/database failures and retry; Swift exercises the foreign callback. |
| Wallet edits | Rename and inclusion are field intents against the latest stored wallet. Swift queues edits and adopts committed projections. | `wallet rename`, `wallet inclusion`; concurrent edits preserve each other and deleted wallets stay absent. |
| Quotes | Core owns holding/pin requests, valid-quote merging, cache, errors and refresh policy. | `price --stored`, `price --refresh`, `currency --refresh-rates`; invalid/missing results, cooldown and restart tests. |
| Maintenance | Stored history and registry poll/finality rules select work, including the scheduler. | `txs --maintenance`, `txs --poll-chain`; confirmed EVM, Dogecoin finality and empty-hash fixtures. |
| Preview | EVM preview resolves wallet network, sender, token and exact amount in core. UTXO/Dogecoin quoting no longer reveals a seed. | `send preview`; local Sepolia RPC checks exact 1.1 ETH and pre-network refusal; Swift async binding test. |
| Boundary | Removed obsolete exports and wrappers, added operations, checked callers and meaningful domain coverage. | `send review`, `diagnostics maintenance`, export scripts; new service clock/cursor scope tests. |

## Audit decisions

Removed FFI: `fetch_prices_typed`, `fetch_fiat_rates_typed`,
`refresh_fiat_rates`, `price_merge_live_updates`, `store_wallet_seed_phrase`,
`store_wallet_private_key`, `fetch_evm_send_preview_typed`. Used Rust entry
points remain plain Rust; obsolete Swift wrappers are gone.

Added FFI: `refresh_owned_prices`, `refresh_owned_fiat_rates`,
`pending_maintenance_chains`, `preview_owned_evm_send`.

Checked Rust names, generated Swift names, direct CLI/Swift callers and foreign
callback implementations before removal. The audit traces domain coverage by
family: import/state/wallet-derived views, diagnostics, transaction-derived
views, polling and routing have service tests; protocol preview/provider tests
cover decoding and failure rules. New clock and cursor service regressions
cover adapters that previously relied on lower-level tests.

Keep distinct Tron, Dogecoin, UTXO, Bitcoin HD and simple-chain preview inputs.
Input-field gating and debouncing remain UI state; signing independently
validates all amounts, destinations and identities. Keep SecretStore and
BalanceObserver callbacks. Keep generic load/save only for platform preferences;
the live-price JSON and UserDefaults paths are removed. Settings reset retains
core's last successful quotes. Export references are not runtime test coverage.

## Failure boundaries

SQLite and platform SecretStore cannot share a hardware transaction. Ordinary
secret/database failures leave no committed import and trigger idempotent
cleanup. Cleanup failures identify affected IDs. A process crash between secret
storage and SQLite commit can leave an orphan secret, but cannot commit a
signing wallet before its secret is stored. No compatibility reader is added.

Quote failures retain old values and expose error state. Partial success changes
only valid returned keys. Scheduled calls obey persisted cooldowns; explicit CLI
refresh may bypass them. Previews do not sign or broadcast. The funded broadcast
and external infrastructure items in PLAN remain open.

## Verification

Rust workspace: 798 core tests. CLI acceptance: 332 checks plus the original
and follow-up Stage 3 fixtures. iPhone 17 Pro: 81 tests, zero failures. FFI:
179 callables (previously 182), zero unreachable candidates. Git changes are
left uncommitted.
