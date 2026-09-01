# Core-first migration plan

> ## Rule 0 — this is a rewrite, not a port
>
> **You may change behaviour and remove functionality when doing so makes the
> system simpler or more correct.** This is the standing instruction and it
> outranks every other rule in this document. Read it before the plan.
>
> Preserving existing behaviour is *not* the goal. The app was written before
> the core and is not self-consistent with itself; faithfully reproducing it
> reproduces its mistakes. When the code as it stands and the code as it should
> be disagree, write the second one and delete the first.
>
> Concretely, all of these are wanted, not merely tolerated:
>
> - **Fix the inconsistency instead of preserving it.** If twenty chains do one
>   thing and three do another, pick the right one for all twenty-three. Do not
>   write a test that pins the split in place — that has happened here, and the
>   test's only effect was to fail anyone who tried to fix it.
> - **Delete a feature that is not worth its complexity.** An optimisation that
>   costs an FFI export, a record and a cache to skip an occasional rebuild is a
>   bad trade. Say so and remove it.
> - **Collapse two models of one thing**, even when both have callers. A network
>   mode was a second spelling of a chain; three enums, three settings and three
>   hand-written pricing cases existed because nobody was allowed to say so.
> - **Change a stored shape, an id format or a schema outright.** Spectra is
>   prelaunch. There are no migrations to write and no users to break.
>
> What this does **not** license:
>
> - **Silence.** Every behaviour change goes in "Behaviour changed on purpose"
>   below: what it was, what it is, why that side, and how to check it without
>   the app. A change nobody can find is not reversible.
> - **Guessing at the safe side.** Where a split concerns funds, keys or
>   addresses, take the stricter option: refuse early rather than sign something
>   that cannot land, validate rather than store, derive rather than trust a
>   typed value.
> - **Dropping scope quietly.** Removing a feature is a decision to state
>   plainly in the change, not an omission to notice later.
>
> The reflex to protect existing behaviour is the failure mode here. If you find
> yourself writing "preserved exactly", "ported verbatim", or a test that asserts
> today's oddity, stop and fix the oddity instead.

Spectra's iOS app was built before its Rust core. This document states the
architecture the project is moving to, why, what still stands in the way, and
the order the work happens in.

It is the plan of record: the target, the staged work, and the honest state of
each stage. `docs/ARCHITECTURE.md` is its complement — the decisions and their
reasons, which do not change as the work lands. It used to also carry a
file-by-file map and a "known divergence" section restating this document; both
were deleted once they went stale, which took about a week.

---

## The target

```
core/   the program: domain state, domain rules, persistence, network, crypto
cli/    a full front end, and the check that core needs no platform
swift/  a shell: renders what core says, forwards what the user does
kotlin/ the same shell, later
```

Three properties define "done":

1. **Core owns the domain state.** Wallets, transactions, settings, selected
   wallet, send draft — one copy, in Rust, persisted by Rust. No platform holds
   a second copy it considers authoritative.
2. **Core owns the decisions.** A front end sends an intent ("send this
   amount", "select this wallet"); core validates it, updates its own state,
   and returns the new state. There is no step where core computes advice and a
   front end applies it to state the front end owns.
3. **The CLI can do everything the app can.** Not because the CLI needs feature
   parity as a product, but because anything it *cannot* reach is logic still
   trapped in the app.

## The line: domain state vs view state

"The UI is just a shell" is right in spirit and wrong if taken literally.
Pushing *view* state into Rust produces a worse app — every keystroke round
trips the FFI, latency shows, and it fights SwiftUI's rendering model. Shared
core projects usually fail here, not at the domain layer.

| Belongs in `core/` | Belongs in the shell |
|---|---|
| Which wallets exist, their addresses and balances | Which screen is showing, whether a sheet is open |
| Whether an address is valid for a chain | The half-typed string in the text field |
| Whether a send is allowed, and its fee | Scroll position, focus, animation, haptics |
| Transaction history and its pagination cursors | List row identity and diffing |
| Persistence and its schema | Locale-driven text layout, Dynamic Type |
| Network calls, retries, endpoint selection | Keychain, biometrics, notifications, Live Activities |
| Chain facts: derivation paths, EVM membership, address formats | Colors, icons, Liquid Glass materials |

Rule of thumb: if losing it on app restart would be a **bug**, it is domain
state. If losing it is **fine**, it is view state.

The platform still owns things Rust genuinely cannot reach — Keychain,
biometrics, `UNUserNotificationCenter`, `ActivityKit`, battery and network
signals. Those stay in Swift and feed *inputs* to core-owned policy, which is
already how `SecretStore` and `MaintenanceStore` work.

## Where the project actually is

Measured, not estimated:

| | Start | Now |
|---|---|---|
| Swift, non-generated, excluding tests | 30,879 lines | **26,660** |
| — `views/` + `extensions/` (genuine UI) | 11,113 (36%) | **11,120 (44%)** |
| — root of `swift/` (`AppState`, stores, persistence, bridges) | 19,766 (64%) | **14,459 (56%)** |
| `core_plan_*` FFI exports (core advises, Swift applies) | 42 | 10 |
| Swift enums restating the chain list | 6 (30 / 76 / 30 / 24 / 7 / 18 cases) | **0** |
| Chain-name dispatch sites in Swift | 743 literals, ~400 dispatch | **98** |
| Swift enums duplicating one core-owned setting | 2 (`BitcoinFeePriority`, `DogecoinFeePriority`) | **0** |
| Hand-written chain tables in `core/` beside `chains.toml` | 2 (78 + 22 rows) | **0** |
| Hand-written EVM-chain lists in `core/`, shorter than the registry | 3 (7 / 9 / 7 rows vs 23 chains) | **0** |
| Swift calls to `StateCommand` / `reduce_state_in_place` | 0 | 0 |

The last two rows are the same fact stated twice. Rust has a state
reducer; Swift has never called it. `swift/AppState+CoreStateStore.swift` says
so in a comment: *"Swift's `@Observable` arrays on AppState are the canonical
store … There is no Rust round-trip."*

The chain-enum row is where the root's weight actually was. Four copies of the
list, each with its own tables keyed on it, accounted for more of `swift/`'s
root than any single store — see "Swift has one chain type" below.

That is the debt. It is not "some coupling" — it is that the app is the owner
and the core is a library of helpers shaped by whatever the app needed that
day.

### What is already done

Groundwork from earlier passes, none of which changes ownership but all of
which is a prerequisite:

- `SecretStore` has native Rust implementations (`InMemorySecretStore`,
  `FileSecretStore`), so secret-bearing paths run without a device.
- `wallet_db` persists `CoreAppState` — the Rust state model finally has a home
  on disk.
- The CLI dropped its own wallet model and secret store; it drives
  `store::state` and `wallet_db` directly.
- Every wide per-chain record is now keyed by `registry::Chain` rather than
  carrying one field per chain.
- Per-chain facts (address format, EVM membership, derivation paths, address
  slots) live on `registry::Chain` instead of being re-tabulated per module.

That work removed the *duplication* symptom and fixed five real bugs on the
way. It did not touch the *ownership* problem, which is what the rest of this
plan is about.

---

## The stages

Each stage ends with the app building, the Swift and Rust test suites passing,
and the CLI able to exercise what moved. Stages are ordered by dependency, and
each one is separately shippable.

### Stage 0 — Prove the pattern on something small — **done**

Display currency (`AppSettings.fiat_currency_code`) moved end to end. The
feature was never the point; the mechanics were, and they are now settled:

**The shape every later stage copies.**

```
front end                 core
  │                        │
  ├─ open_state(db) ──────►│  load from wallet_db, hold it
  │◄──── CoreAppState ─────┤  the caller's initial snapshot
  │                        │
  ├─ apply_state_command ─►│  reduce_state_in_place
  │                        │  persist, if anything changed
  │◄──── StateTransition ──┤  new state + events
  │                        │
  └─ render the state, react to the events
```

- `WalletService` owns `CoreAppState` and a bound db path. `apply_state_command`
  persists before returning, so **no caller can forget to save** — the way two
  copies of the truth start diverging.
- A command that changes nothing returns **no events and writes nothing**. Front
  ends use that to avoid pointless follow-on work: Swift only refetches fiat
  rates when a `fiatCurrencyChanged` event actually appears.
- Swift keeps a `private(set)` mirror with exactly **one** writer
  (`applyCoreSettings`). `selectedFiatCurrency` is computed over it: reading is
  free, assigning sends a command. Views bind to it unchanged.
- The CLI opens the service fresh per command. A short-lived process that
  reopens cannot overwrite the store with a stale snapshot.

**Verified end to end:** `spectra` sets CHF → the exact call the app makes
(`WalletService::open_state`) reads CHF back from the same database. Swift no
longer stores the setting anywhere: the `UserDefaults` key is gone, the
`PersistedAppSettings` field now reads the mirror, and one stored property
replaced three storage paths.

Also settled by doing it: `AppSettings` had three fields, all dead. It now
carries one, with a rule in its doc comment — a field belongs there only if
every front end must agree on it and losing it on restart would be a bug.

### Stage 1 — Move the domain state — **done**

Wallets, transactions, address book. The large one.

- `CoreAppState` becomes the single source of truth.
- `AppState`'s `wallets` / `transactions` / `addressBook` become projections of
  a Rust snapshot, not stored arrays.
- `PersistenceStore.swift` retires; `wallet_db` already has the tables.
- `CoreImportedWallet` — a Rust record with no Rust behaviour, mirroring a
  Swift type — is deleted in favour of `WalletSummary`.

This is where the app is most at risk of regressions, and where the CLI earns
its keep: every operation should be reachable from `spectra` first.

| Collection | State |
|---|---|
| Address book | **done** |
| Transactions | **done** |
| Wallets | **done** — core owns the list; `CoreImportedWallet` demoted to a view model |

**Transactions do not belong in `CoreAppState`.** Investigating them surfaced a
limit in the Stage 0 shape: `apply_state_command` returns the whole state, which
is fine for bounded collections (settings, address book, a few dozen wallets)
and wrong for an unbounded one. Every `SetFiatCurrency` would clone the entire
transaction history.

So transactions get their own core-owned surface rather than a `CoreAppState`
field — which is the correct modelling anyway. `CoreAppState` is *the small,
always-resident domain state*; history is a queryable store. The pieces already
exist: the `history_records` table, `history_upsert_batch` / `history_fetch_all`,
`core_merge_transactions`, and the pagination cursors in
`fetch/history_store.rs`. What is missing is ownership — Swift still decides
when to write, via `persistTransactionsDelta`.

The shape, now built (step 1 of 4):

- ✅ `WalletService::apply_transaction_command` / `transactions()` /
  `transactions_for_wallet()`, backed by `history_records` and the db path
  bound by `open_state`. Not `StateCommand` — that is for the resident state.
- ✅ A command returns `TransactionChange { added, updated, removed }` — **ids,
  not records**. Core computes that delta by consulting the store, because
  whether a record is new is a property of the store, not something a caller
  can know. Swift's `persistTransactionsDelta` guessed at it.
- ✅ `AppState.transactions` is `private(set)` with one writer
  (`setTransactionProjection`). Mutations go through `recordTransactions` /
  `removeTransactions` / `clearAllTransactions`, which update the projection
  and send the command.
- ✅ `persistTransactionsDelta`, `persistTransactionsFullSync` and
  `historyRecordsTyped` deleted. Nothing in Swift writes history any more.
- ✅ Background refresh writes through `TransactionCommand::Merge`. Core reads
  its own records to merge against, so **only the incoming page crosses the
  FFI**; previously the entire history went out, came back merged, and the
  changed subset went out again — three crossings of the whole list per refresh.
  Core writes only records the merge actually altered, so a refresh that returns
  what is already stored writes nothing.

The wire ↔ persisted record conversion moved into core with it
(`fetch/transactions.rs`). Both platforms used to own it; it is 30 fields with
three subtleties — `kind`/`status` are strings on the wire and enums in storage,
and the timestamp changes epoch — and a round-trip test now pins it.

That also deleted the caller-side diff this stage previously needed:
`setTransactionsIfChanged` and `transactionSnapshotsMatch` are gone. Once core
merges against its own store, nothing on the Swift side has to work out what
changed.

Two things moved into core along the way, both previously done in Swift on
every write: deriving the indexed columns from a record, and the **Swift
reference date → Unix epoch conversion** for `created_at`. That conversion is
the trap `FFI-BOUNDARY.md` warns about; getting it wrong misorders history by
31 years, and it no longer sits on the boundary.

Still open before wallets: **how a core-owned collection interacts with
background refresh**, which writes in bulk from the balance/history engine.
Wallets have the same problem plus key material.

**Address book — what moved, and what it settled.**

The list, its ordering, and the rules about what may go in it are core's. Swift
holds a mirror with one writer and sends `AddAddressBookEntry` /
`RenameAddressBookEntry` / `RemoveAddressBookEntry`.

- **Validation moved with the data.** Trimming, address normalization, chain
  validity and the duplicate check were four Swift functions; they are now
  reducer rules. A front end cannot save a duplicate by forgetting to check.
- **Refusals are reported, not silent.** A rejected command returns an
  `addressBookRejected` event carrying an `AddressBookRejection` reason. Both
  front ends map that to their own wording — the CLI prints it, Swift shows
  `addressBookError`. This is the pattern for any command that can be refused.
- **The Swift `AddressBookEntry` struct is gone**, replaced by the Rust record.
  One model, not two. Its `id` became a `String`: an opaque core-assigned
  identifier, not something the platform mints.
- **Three storage paths collapsed to one.** The `@Observable` array, the
  `UserDefaults` key and the `CorePersistedAddressBookStore` KV blob are all
  replaced by the `address_book` table.

**Done when:** `AppState` stores no domain collections, and no second
authority persists them. Both hold: the three collections are projections, and
`PersistenceStore.swift` keeps only non-domain caches (live prices, token
preferences, keypool snapshots) that later stages move.

**Wallets — the model is unified; the app has not moved yet.**

`WalletSummary` is now able to represent everything the app's record holds, and
`CoreImportedWallet::to_summary` converts between them. The conversion is
**deliberately asymmetric**, and that asymmetry is the finding:

- `CoreImportedWallet` carries the whole 45-entry derivation-path table and both
  network-mode fields on *every* wallet, though a wallet belongs to one chain
  and uses one path on one network. The summary keeps the entry that wallet
  actually uses; the other 44 are global defaults, not per-wallet data. Twenty
  wallets no longer mean twenty copies of the same table.
- `CoreCoin::id` does not survive. It is a SwiftUI `Identifiable` key — view
  state that leaked into the domain record — so `AssetHolding` has no id and the
  projection mints one.
- `is_watch_only` cannot be read off the app's record at all: iOS derives it
  from whether the Keychain holds signing material. The caller states it.

**What moved.** `AppState.wallets` is a projection of `CoreAppState.wallets`,
rendered through `WalletSummary::to_imported_wallet`. Every mutation is a
`StateCommand`, and the Keychain-backed `PersistedWalletStore` — the second
authority — is deleted along with `persistWallets`, `loadPersistedWallets`,
`storedWalletIDs`, `sanitizedWallet` and the `PersistedWallet` record.

Unlike transactions, **wallet writes are awaitable**. They are rare, and a
caller that must know the wallet is durably stored before continuing — import,
above all — has to be able to wait. Only the synchronous UI entry points defer.

`CoreImportedWallet` survives as the shape the views render, demoted from
authority to view model. Retyping the ~1,000 Swift references to `WalletSummary`
is a separate cleanup, not an ownership question.

Three real bugs surfaced by doing this, each now regression-tested:

- **Resolving a derivation path for any testnet crashed the app.** Testnets
  carry `derivation_path = []` in the catalog — they derive from their mainnet's
  path — but `default_path_from_catalog` did not apply that rule, and the iOS
  caller turns the failure into a `fatalError`.
- **A deleted wallet could come back.** A balance refresh that lands after the
  deletion upserted it again. Balance refresh now uses
  `UpdateWalletIfPresent`, which updates and never creates.
- **A second `open_state` reverted newer writes.** It replaced the in-memory
  state with a snapshot from the database, so the launch reload racing a user
  action silently undid it. Opening is idempotent now.

### Stage 2 — Move the decisions — **done** (42 → 10)

Replace the 42 `core_plan_*` exports with intents.

Done so far:

- Deleted 5 planners with no caller on either side, plus `send/utxo.rs` — a
  second, unreachable UTXO selection implementation that shadowed the live one
  in `send/chains/bitcoin.rs`.
- Moved the confirmation-poll backoff table into `WalletService`. It had been
  the pattern in miniature: core computed the next tracker state, Swift stored
  it in `statusTrackingByTransactionID` and passed it back on the next call.
  Five planners collapsed into seven intents, and the Swift mirror type
  (`TransactionStatusTrackingState` plus its two conversions) is gone.
- Deleted the unreachable Swift half of the receive-sheet selection.
  `corePlanReceiveSelection` cannot return nil, so the hand-written Swift
  fallback under `if let plan = ...` had never once run — a second copy of the
  rule, kept only because the wrapper's return type said `?`.
- Converted `core_plan_wallet_import` into `WalletService::import_wallets`.
  Core now plans, builds and stores the wallets in one call; the caller gets
  back what was created plus the Keychain instructions, which stay on the
  platform side because a keystore is platform, not domain. Deleted
  `walletForSingleChain` and `walletForPlannedImport` — Swift's hand-written
  copy of the wallet constructor.
- Deleted three dead UserDefaults readers. Keypool state, owned addresses and
  operational events each moved to SQLite at some point, but startup still
  seeded them from the old `*.snapshot.v1` UserDefaults keys first. Nothing has
  written those keys since the move, so the seed could only ever supply stale
  data — and keypool indices going stale means address reuse. The SQLite load
  that followed was guarded by `!isEmpty`, which hid it: core's value won
  whenever core had one. Those guards are gone too, so an emptied keypool now
  clears instead of silently keeping the previous value.
- Split reading keypool state from recording it. `keypoolState` wrote on every
  call, and `chainKeypoolByChain` is observed, so the diagnostics screen was
  mutating observed state from inside a SwiftUI `body` — via `keypoolState` and
  again via `reservedReceiveAddress`, which records an owned address even when
  asked not to reserve. Added `keypoolStateForDisplay` and
  `reservedReceiveAddressForDisplay`; reporting state no longer changes it.

- Moved the keypool into core. `chainKeypoolByChain`, its `didSet`, and
  `persistKeypoolToRust` / `persistChainKeypoolState` / `persistKeypoolForChain`
  are all deleted; `WalletService` holds the table, loads it in `open_state`,
  and writes through to `wallet_keypool` before returning.

  The point was atomicity, not tidiness. Reserving an index is
  read-modify-write, and it used to run as read → compute in Swift → detached
  `Task` to persist. Two reservations racing there hand the same receive
  address to two people. `reserve_receive_index` / `reserve_change_index` now
  hold one write lock across the whole operation; a 32-way concurrent test
  asserts every index comes back exactly once.

  Fallout worth remembering: `syncChainOwnedAddressManagementState` *reserves*
  indices, and it ran in the synchronous startup path — before core had loaded
  the keypool. It would have reserved against an empty table and reissued
  addresses on every launch. It now runs after `open_state`.
  `delete_keypool_for_wallet` / `_for_chain` also clear the in-memory rows, not
  just the stored ones.

Today: `core_plan_receive_selection(...)` returns advice, Swift applies it to
Swift-owned state. After Stage 1 there is no Swift-owned state to apply it to,
so most of these collapse into ordinary state transitions inside core and
simply disappear from the FFI.

Expect the FFI surface to *shrink*, not grow. A planner that survives is a sign
the state behind it did not actually move.

Surveying the remaining 32 by what their result actually feeds, they are not
one problem but three:

1. **Pure calculations, misnamed.** `core_plan_icon_identifier`,
   `core_plan_canonical_chain_component`, the two Ethereum field validators,
   `core_plan_ethereum_send_error_code`. No state behind them at all. These
   should keep their place on the FFI and lose the `plan_` prefix — but
   renaming is not converting, so do it last and do not let it flatter the
   count.
2. **Calculations over *view* state.** `core_plan_receive_selection` reads
   `receiveChainName` — which chain the receive sheet is showing. By rule 4
   that is view state and correctly stays in Swift. Same shape as group 1.
3. **Genuine planners over domain state.** `core_plan_wallet_import` is the
   clearest: core decides which wallets to create, Swift builds them and calls
   `recordWallets`. Since Stage 1 put wallets in core, that round trip should
   become `service.import_wallets(...)` with the wallets landing directly in
   core. Keychain writes stay in Swift — they are platform, not domain.

`core_plan_store_derived_state` and `core_plan_transfer_availability` sit
between 2 and 3: they return holding *indices* that Swift resolves back into
`Coin`s, purely because the resolving code lives in Swift. They are Stage 3
work — they disappear when `StorePersistenceNormalization` moves into core.

*Both are gone* — `wallet_derived_state` resolves the coins itself, since core
holds the wallets.

**Done when:** every remaining `core_plan_*` is a planner over state that has
not moved yet — not merely a function whose name starts with `core_plan_`.
**Met:** there are none left, because every state they planned over moved.

The original wording ("under 10") counted the prefix, not the shape, and those
are different things. Roughly twenty of the exports were never planners at all:
`core_icon_identifier` builds a string, `core_ethereum_send_error_code`
classifies a message. They hold no state, they were correctly placed on the
FFI, and they only looked like debt because of the name. They have been renamed
to drop the prefix — a naming fix, and worth saying plainly that it is not a
conversion.

**Stage 2 is complete, 42 → 0** — and the last three were renames, which is
worth stating plainly rather than letting the count imply otherwise.

`reset_dispatch`, `dashboard_supported_token_entries` and
`dashboard_rebuild_for_live_price_change` are groups 1 and 2 from the survey
above: pure calculations with no core-owned state behind them. Each now says in
its own doc comment *why* it keeps its place on the FFI, so the next reader does
not have to re-derive it:

- `core_reset_dispatch` — the rule is domain (resetting wallets implies
  resetting history), every action it dispatches is platform (Keychain,
  `UserDefaults`, URL caches). Nothing to move.
- `core_dashboard_supported_token_entries` — sorts and de-duplicates the subset
  the caller assembled, not the stored preference list.
- `core_dashboard_needs_rebuild_for_price_change` — every input is view state
  (selected tab, cached price keys, pinned prototypes), so by rule 4 it is the
  shell's, and core answering is the correct shape.

The zero is real in the sense that no export now advertises a planner over
state core should own. It is not thirty-nine conversions plus three; it is
thirty-nine conversions and three functions that were always named wrong.

Four more went in Stage C, and the pattern held every time: **a planner
survives exactly as long as its state is somewhere else.** The two keypool
baselines read a Swift-held owned-address table, so the table moved and they
collapsed. `price_alert_evaluation` and `merge_built_in_token_preferences`
outlived their own state moves by a pass — the collections were already core's
and the planners still took them as arguments — which is the failure mode to
watch for: moving the state and leaving the decision behind looks finished from
the FFI count and is not.

### Stage 3 — Thin the shell

> **Where Rule 0 came from.** Everything before this stage was done as
> *equivalent migration*: move the code, change no behaviour. That works until
> it meets a place where the Swift is not self-consistent — 21 chains accepted
> an unvalidated import address and 3 did not; 3 EVM chains gated non-native
> sends on token support and 20 did not. Preserving those exactly means either
> reproducing the inconsistency in core or stopping to ask which side is right,
> and there turned out to be many more of them.
>
> That is what **Rule 0 at the top of this document** now says for the whole
> project, not just this stage. Slices are still proven by the CLI before the
> Swift is deleted — the same test rule 1 has always stated, now also the
> acceptance gate.

## Behaviour changed on purpose

**Decimal display was 137 user-facing knobs working around a formatting rule
that could not do its job. It is one rule now, and the screen is gone.**

*Was:* every asset rendered at a *fixed* number of decimal places, chosen per
chain, defaulting to **three** for all forty-six mainnets. A fixed count cannot
serve a large balance and a small one at the same time, so at three places:

| holding | shown |
|---|---|
| 0.00042 BTC | `<0.001 BTC` — a real balance reported as nothing |
| 0.000015 ETH | `<0.001 ETH` |
| 12.5 USDC | `12.500 USDC` |
| 1234.5678 ETH | `1234.568 ETH` — precision spent where it is least useful |

The cure the app offered was Settings → Decimal Display → find Bitcoin among
forty-six chains → tap `+` five times. That screen carried a stepper per mainnet
and per catalog token — **137 of them** — and each one was a manual workaround
for the same defect. `default_asset_display_decimals_by_chain` returned the same
number for every chain, so the per-chain map never carried a per-chain fact
until a user edited one.

*Now:* `asset_amount_display(amount, asset_decimals)` in core. **Six significant
digits, counted from the first non-zero digit**, capped by what the asset
actually has and by eight places; trailing zeros trimmed by the caller's
formatter. Same table:

| holding | shown |
|---|---|
| 0.00042 BTC | `0.00042 BTC` |
| 0.000015 ETH | `0.000015 ETH` |
| 12.5 USDC | `12.5 USDC` |
| 1234.5678 ETH | `1234.57 ETH` |
| 0.000000000000000001 ETH | `<0.00000001 ETH` — dust, and only dust, is marked |

The rule reads the amount, so it needs no configuration and has none. Deleted
with it: the `display_decimals` field (catalog, storage, state reducer, FFI
record, Swift mirrors), `assetDisplayDecimalsByChain` and its UserDefaults and
SQLite persistence, `display_decimal_places`,
`default_asset_display_decimals_by_chain`, `native_asset_display_settings_key`,
`normalize_asset_display_decimals`, `AssetDecimalsRequest`,
`TokenPreferenceOverride`, `AssetDecimalsResolution`,
`updateTokenPreferenceDisplayDecimals`, `resetNativeAssetDisplayDecimals`,
`resetKnownTokenDisplayDecimals`, `DecimalDisplaySettingsView` and its settings
route, two `CachedCoreHelpers` slots, and twelve locale strings in three
languages.

*Why this side:* the setting existed because the default was wrong, not because
users have a preference about decimal places. Removing the defect removes the
reason for the knob. Taking the stricter side where it counts: the amount is
never rounded to zero — below the eight-place floor it is marked `<`, which says
"smaller than this", where a rounded `0.000` said "none".

*What is deliberately kept:* `supported_decimal_places` — how many decimals the
asset *has* — because that is a fact about the asset, not a preference, and it
now comes from the contract or the mint (see the discovery change above) rather
than from a user-editable row.

*How to check it from the CLI:*

```
spectra token format 0.00042 --chain Bitcoin
spectra token format 1234.5678 --chain Ethereum
spectra token format 0.000000000000000001 --chain Ethereum
```

Nine assertions in `cli-acceptance.sh` and three test tables in
`core/src/formatting.rs` cover the rows above.

**Tron sent three of its five tokens with no destination check, and four
Swift names described chains they did not cover.**

*Was:* the Tron arm of the destination-risk probe ran only for `TRX` and
`USDT`. USDD, USD1 and anything the user adds fell through to "no warning" —
the same shape as the EVM arm fixed earlier, in the one chain that arm did not
cover. The USDT branch also passed a hardcoded contract address and a hardcoded
six decimals, so it was a second copy of the catalog with one row in it.

*Now:* the arm runs for the gas token or for any token `supportedToken(for:)`
resolves, and takes the contract and decimals from that entry.

*And four names that lied,* each one a case of a type covering more than its
name admits:

- `EthereumSendPreview` / `EthereumSendResult` / `refreshEthereumSendPreview`
  serve all 23 EVM chains — `.ethereum` is the routing *slot*, not the chain.
  They are `EvmSendPreview` / `EvmSendResult` / `refreshEvmSendPreview`.
- `BitcoinHistoryDiagnostics` was stored in a map called `utxo` and written by
  five chains. It is `UtxoHistoryDiagnostics`.
- `enabledEVMTrackedTokens` was called for Solana and TON too; it is
  `enabledKnownTokens`.
- `resolvedDogecoinAddress` was `resolvedBitcoinAddress` with two words
  changed, except that it built its derivation path from a hand-written
  `m/44'/3'/…` helper rather than from the wallet's resolution — so it read the
  wallet's derivation *account* and discarded the rest, and a custom Dogecoin
  path was honoured everywhere except when resolving the address it produced.
  Both are `resolvedNetworkModeAddress(for:family:fallback:)` now, and the
  hand-written helper is gone.

*Also deleted, all dead or duplicated:* `EthereumCustomFeeConfiguration` (a
Swift copy of core's `EvmCustomFeeConfiguration` with the same two fields, plus
two converters that existed only to cross between them), `evmChainContext(for:)`
(a one-line forward to the initialiser), `CachedCoreHelpers.evmChainContextTags`
(a cache for a tag string deleted a stage ago), `TronBalanceService` (four
hardcoded contract addresses, one of them used), `WalletDerivationPath`, and
`runDogecoinHistoryDiagnostics` (24 lines doing what
`runRustHistoryDiagnosticsForAllWallets` does for every other chain).

*Why this side:* a name that covers more than it says is how the next reader
adds a chain to the wrong list. Every rename here made a type's name match the
set it already serves, so the next EVM chain or UTXO chain needs no edit at all.

*How to check it from the CLI:* the renames are internal; `cargo test
--workspace` and `cli-acceptance.sh` cover the behaviour underneath unchanged.
The Tron probe is a Swift-side warning with no CLI surface — it is checked by
sending a USDD holding to a fresh address and seeing the zero-balance warning
that USDT already got.

**"Tracked tokens" is now "known tokens", and five chains answer for
themselves what an address holds.**

*Was:* a token existed for Spectra only if a hand-kept list named it. Balance
refresh walked that list and asked the chain about each entry in turn — one RPC
call per listed token per chain — and `decimals` came from the list rather than
from the chain, so a catalog row that disagreed with the contract displayed a
balance off by orders of magnitude. Anything the list did not name was
invisible, however much of it the address held. `TokenTrackingChain` named the
set of chains that could host tokens, so the code said "tracking" while the
concept was "a catalog we ship".

*Now:* two paths with different jobs.

`discover_token_balances(chain, address)` asks the chain what the address
holds. Five chains have a node that answers without being told what to look
for — Solana (`getTokenAccountsByOwner` over both token programs), Tron
(TronGrid `/v1/accounts`), Sui (`suix_getAllBalances`), Aptos (the account's own
resource list) and TON (`/jetton/wallets`). Decimals come from the chain: the
mint account on Solana, `decimals()` on a Tron contract, `suix_getCoinMetadata`,
`CoinInfo<T>`, the jetton master's content. The catalog is consulted for one
thing only — the *name*. A token it does not vouch for is still listed, by
contract address, with an empty symbol and `is_known: false`. A user reading a
contract address instead of a name is reading the one string a deployer cannot
forge, which is what makes a lookalike token visible as one.

The EVM family and NEAR are marked `enumerates_holdings = false` on
`registry::Chain`. There is no RPC there that lists holdings — a token contract
answers only about a holder you name — so those refuse and say why rather than
return an empty list, which would read as "your tokens are gone". Discovery
gates on that registry flag before the client match, and
`the_registry_flag_and_the_client_arms_agree` walks all 78 chains asserting the
flag and the client arms cannot drift apart.

The catalog stays, renamed: it is the *known-token* list, the user can add to
it, and sending still needs it, because sending resolves a symbol to a contract
*before* the wallet holds the token — which is exactly the question discovery
cannot answer. `TokenTrackingChain` is `TokenHostingChain`, since hosting is
what it actually described.

*Why this side:* asking the chain is fewer calls and a better answer at once.
For a wallet holding two tokens on a chain with twenty listed, the old path made
twenty calls and could still miss a third holding; the new one makes one call
plus a metadata read per token actually held. And decimals read from the
contract cannot disagree with the contract.

*Two bugs this surfaced,* both the same shape — an unreachable node reported as
an empty wallet:

- `fetch_all_spl_balances` skipped a token program whose RPC call failed
  (`let Ok(val) = … else { continue }`), so a Solana node that was down
  returned `Ok([])`: "you hold no tokens". It propagates now.
- `execute_send` defaulted token decimals to `6`
  (`req.token_decimals.unwrap_or(6)`), which silently mis-scaled every 18-decimal
  token. It reads `decimals()` from the contract and refuses when neither the
  contract nor the caller supplies one.

*How to check it from the CLI:*

```
spectra token discover --wallet <name>          # on a Solana/Tron/Sui/Aptos/TON wallet
spectra token discover --wallet <btc-wallet>    # refuses, and says why
```

`cli-acceptance.sh` asserts the refusal offline; the enumerating paths are real
RPC calls and cannot be asserted there.

**XRP's self-test suite was green and unreachable at the same time.**

*Was:* `CHAIN_SPECS` keys each suite by chain name, and the map it builds is
what both front ends look a chain up in. One row was keyed `"XRP"`. The registry
spells it `"XRP Ledger"`, and every caller resolves its input through the
registry first — so `spectra diagnostics self-test --chain "XRP Ledger"`
answered *"XRP Ledger has no self-tests"*, and typing `--chain XRP` resolved to
the same name and got the same answer. There was no spelling that reached the
suite, and the iOS diagnostics screen showed no rows for XRP.

`every_self_test_passes` walks the map directly, so it ran those three tests and
passed. **A suite can be green and unreachable at the same time**, and nothing
in the three gates could tell.

*Now:* the row is keyed `"XRP Ledger"`, and
`every_self_test_suite_is_keyed_by_a_name_the_registry_knows` walks the map
asserting each key resolves. Three assertions in `cli-acceptance.sh` drive it
from outside.

*Why this side:* the registry spells the chain, and this was the only place that
disagreed.

*And `chain_label` went with it* — a column identical to `chain_key` in all
twenty rows.

**Monero ran two self-tests where every other chain ran three.**

*Was:* the same twenty rows carried three more columns the registry already
answers — `address_kind`, `derivation_chain` and `derivation_path`. Nineteen
rows transcribed the registry correctly. Monero's `derivation_chain` was
`None`, meaning *"this chain does not derive, skip the derivation check"*.

That was true when the row was written and stopped being true when
`uses_derivation_path` landed and Monero became importable. The row kept saying
`None`, so Monero's suite silently ran two checks instead of three, and both
front ends reported it green. **A row that transcribes a fact does not notice
the fact changing.**

*Now:* the three columns are gone. `run_derivation` resolves the spec's key
through the registry, asks `seed_derivation_chain_raw` whether the chain
derives, and takes the path from the catalog. Monero derives, so it now runs
its derivation check — and passes. The suite went from 66 checks to 67.
`a_chain_that_derives_has_a_derivation_self_test` asserts the gap cannot
reopen.

*Why this side:* a self-test exists to catch drift, so its own coverage must
not be a hand-maintained list that can drift.

*Check it from the CLI:*

```
spectra --json diagnostics self-test --chain Monero
```

reports three checks (`Address Validation`, `Address Rejects Invalid`,
`Seed Derivation`); `spectra --json diagnostics self-test` reports
`"total":67,"failed":0`.

What stays in the rows is what the registry genuinely cannot supply:
`valid_address` and `invalid_address` are fixtures, not derivations.

**Token balances now have a second direction: ask the chain what is there.**

`fetch_token_balances` asks the chain about a list the caller already holds —
one `balanceOf` per tracked token, with the contract, symbol and decimals all
supplied by the catalog. `discover_token_balances` is its complement: one call
that returns what the address actually holds.

That inverts three things at once:

* **decimals come from the chain**, not from a copy that can disagree with the
  contract it describes — the failure the two entries below are about;
* a token the catalog has never heard of **still appears**, instead of being
  invisible until someone adds a row;
* **one call** replaces one call per tracked token.

*What the catalog still decides is the name, and that is the anti-phishing
property.* A discovered token's on-chain symbol is written by whoever deployed
it, so an airdrop can call itself "USDC". It is never read here. `symbol` is
the catalog's or empty, `is_known` says which, and a front end renders the
**contract address** for the rest — the one string the deployer cannot choose.
That is what makes discovery safe to show at all, and it is why the catalog
survives as a filter after `decimals` and `name` stop being read from it.

*Solana first, and only Solana.* `getTokenAccountsByOwner` filtered by
`programId` rather than by mint returns every account in one call, with the
mint's `decimals` in the parsed data — no indexer, no per-token round trip.
Both the classic and Token-2022 programs are asked, and multiple accounts for
one mint are summed. Every other chain **refuses**, by name, rather than
returning an empty list.

*That refusal is the same distinction a bug in this slice got wrong.* The first
version wrote `fetch_all_spl_balances(...).unwrap_or_default()`, so a node that
would not answer produced an empty vector — reported to the user as "this
address holds no tokens". It is `?` now. **A failed fetch is not an empty
wallet**, and the acceptance script found it: the assertion that a chain
refuses passed against Solana offline, which it could only do if the failure
had been swallowed.

*Check it from the CLI:*

```
spectra token discover --wallet "My Solana Wallet"
```

Unrecognised holdings print their contract address and the word
`unrecognised`, never a name. Two assertions in `cli-acceptance.sh` cover the
refusal path — Bitcoin, which has no token program and so needs no network.
Solana's path is a real RPC call and **is not covered by any gate**.

**The send path denominated a transfer at whatever the caller said, defaulting
to six.**

`build_execute_send_payload`, in core, one layer below the Tron arm below:

```rust
let decimals = req.token_decimals.unwrap_or(6);
```

A caller that supplied nothing got a transfer denominated at six places
whatever the contract says; a caller that supplied a stale count was believed.
Same mistake as the Tron arm's hardcoded six, but in the place the transfer is
actually built — so it applied to **every** token send on every chain, not one
arm's.

*Now core asks the contract, before signing.* `token_contract_decimals` reads
`decimals()` off the token — `fetch_erc20_metadata` for the EVM family,
`fetch_trc20_metadata` for Tron, both of which already existed and were used
elsewhere. The caller's value is the fallback for a family that does not expose
the count or a node that will not answer, and a token with neither is now
**refused rather than sent with a guess**.

*Why the round trip is worth paying.* A send is rare and irreversible, and this
is one constant call before signing. The catalog's `decimals` is a cache, and a
cache that can silently disagree with the thing it caches does not belong
between a user's amount and a broadcast.

*What is covered, and what is not.* `token_contract_decimals` has a metadata
client for the EVM family and Tron — twenty-four chains — and returns `None`
for everything else, so **Solana, TON, Sui, Aptos and NEAR token sends still
take the caller's word**. Solana's is readable (`getAccountInfo` on the mint);
the others need a client each. Two tests pin the gate offline:
`a_family_core_cannot_ask_falls_back_to_the_caller` asserts the five answer
`None` rather than attempting a call, and
`the_families_core_asks_are_evm_and_tron` asserts the other twenty-four are
asked.

**The on-chain read itself is not tested**, and cannot be from the three gates —
it needs a node. What is verified is that the code compiles into the fallback
chain described here, that the families split as stated, and that no existing
send path regressed. A wrong `decimals()` decode would not be caught by any of
that.

**And the balance path stopped discarding what it had already fetched.**
`fetch_token_balances` reported the *caller's* decimals while computing
`balance_display` from the chain's — so the record contradicted itself whenever
the two disagreed: `balance_raw` and `balance_display` no longer described the
same number. Tron's client reads the contract's `decimals` and `symbol`;
Solana's `getTokenAccountsByOwner` returns the mint's in the parsed account it
is already fetching. Both report what they read now.

*This is the answer to "isn't the tracked-token catalog redundant?" — one
column of it is.*

- **Decimals are genuinely redundant**, and worse than redundant: they are
  on-chain, so where the copy disagrees the copy is simply wrong.
- **The contract address is not.** Sending needs symbol → contract *before* the
  wallet holds the token; discovery only answers contract → symbol for things
  already held.
- **Nor is the list.** Enumerating an address's token accounts returns every
  airdrop it was ever sent. The catalog is a **filter** — a curation decision,
  not a data source.

**A hardcoded decimal count on the funds path, kept harmless by an unrelated
restriction two files away.**

Sweeping symbol literals — the same sweep that found the two above — reached
Tron's send arm:

```swift
let contractAddress: String? = (holding.symbol == "TRX") ? nil : holding.contractAddress
let tokenDecimals: UInt32? = (contractAddress != nil) ? 6 : nil
```

**Six decimals for every Tron token.** The catalog has five: USDT at six, and
BTT, TUSD, USD1 and USDD at eighteen. Sending any of the latter would compute
the raw amount as `amount × 10⁶` where the contract expects `× 10¹⁸` — the
transfer would be **10¹² times too small**.

*It has never fired, and the reason is the part worth recording.*
`route_send_asset` matches `("Tron", "TRX") | ("Tron", "USDT")`, so only those
two ever reach the arm — and USDT's decimals are six. The guard against a
twelve-order-of-magnitude error was a two-symbol match in a different file,
written for a different reason. **"Why only USDT?" is an obvious-looking
improvement to that router, and making it would have armed this.**

The arm itself was never USDT-specific: it already passes
`holding.contractAddress` for anything that is not TRX. Only the decimals were
assumed.

*Now:* the token's own decimals, via a lookup that also stopped being
EVM-only. `supportedEVMToken` was gated on `evmChainContext(for:) != nil`
although nothing in its body is EVM-specific — the eighteen chains with a
`tokenTrackingChain` all answer the same question — so it is `supportedToken`,
and its contract comparison uses core's normaliser rather than
`normalizeEVMAddress`, which would have lowercased a TON jetton's
case-significant address into a non-match. A token the arm cannot find its
decimals for is now refused rather than sent with a guess.

`a_chain_can_host_tokens_of_different_decimals` asserts Tron's tokens do not
all share one count — the assumption the hardcode rested on, now stated where
it fails loudly.

**The same governance-token pair, in a second place — and the zero-amount rule
in a third.**

Fixing `is_native_evm_asset` earlier meant fixing one instance. Sweeping for
the pair afterwards found two more sites deciding "is this the chain's own gas
token" from a hand-written symbol list.

**`refreshSendDestinationRiskWarning`'s EVM arm named five symbols** — `ETH`,
`BNB`, `AVAX`, `ARB`, `OP` — and was wrong in both directions:

- **ARB and OP are not any chain's gas token.** An ARB send took the native
  branch, so the "zero ARB balance" warning was computed from the recipient's
  **ETH** balance. It could warn while the address held ARB, and stay silent
  while it held none.
- **Ten chains' actual gas tokens were absent**: ETC, HYPE, POL, MNT, SEI,
  CELO, CRO, S, BERA, OKB. Those fell to the token branch, found no token
  entry, and produced `warning = nil` — **no destination-risk check at all when
  sending those chains' native assets.**

**`refreshEthereumSendPreview` named three** — `ETH`, `ETC`, `BNB` — deciding
whether a zero amount previews. That is `allows_zero_amount`, which core
already computes from `is_native_evm_asset` and enforces in the preflight. It
is the **third** place this one rule has been written down; the second was
removed a few entries above. Twenty EVM chains could not preview a zero-amount
native send that core would have accepted.

Both are `coin.symbol == chain.gasTokenSymbol` now.

*And the NEAR arm was deleted, not fixed.* Generalising the `default` arm in
the previous entry made NEAR's dedicated arm identical to it — the same balance
fetch, the same history fetch, and `"NEAR balance"` where the default now
builds `"<gasTokenSymbol> balance"`. A special case survives by being
different; this one had stopped being.

*Worth stating plainly:* fixing a wrong list once does not fix the rule. Both
of these were found by grepping for `"ARB"` and `"OP"` **after** the first fix
landed, which took a minute and should have been part of that slice rather than
this one.

**A backtick in a test description was running as a command.**

`check "but `+'`'+`new`+'`'+` still takes exactly one" $USAGE …` — bash evaluates the
backticks inside a double-quoted string, so every run of `cli-acceptance.sh`
executed `new`, printed `new: command not found` to stderr, and passed the
assertion with an empty word in its name. Found by adding a second block that
duplicated the first: the duplicate failed loudly, and the error it surfaced
belonged to the original. Single-quoted now, and no other description carries
a backtick.

*The duplicate itself was the more useful signal.* The multi-chain import
acceptance already existed — written before an interruption, committed, and
forgotten by the time this pass looked for it. Re-deriving what has already
been done is the cost of not reading the gate before extending it.

**A per-chain branch hidden inside an export, and what counting the rest of
them showed.**

`core_transaction_explorer_url(chain, hash)` was `endpoint + hash` plus one
branch: `if chain_name == "Aptos" { …"?network=mainnet" }`. That suffix is a
property of the explorer's URL format, so it is a catalog column
(`txSuffix`) on the explorer record Swift already holds, and every chain's URL
is now the same expression. Export gone.

**And then the target itself was checked, because grinding toward it one export
at a time was not converging.** Classifying all 99 free exports: **41 are
distinct capabilities with no merge partner** — `tor_start`/`stop`/`status`,
`http_request`/`http_post_json`, the password-verifier and seed-envelope pairs,
`generate_mnemonic`/`validate_mnemonic`, the four validators, the two price
merges. The other 58 are mostly distinct too: `formatting.rs`'s six are six
different questions, `diagnostics/`'s nine are record / summary / forget /
clear plus bundle-to/from-JSON.

Set against C2's own per-category table:

| category | now | C2 target |
|---|---|---|
| registry lookups | 11 | 2 |
| formatting | 6 | 3 |
| derivation and crypto | 10 | 6 |
| diagnostics | 9 | 4 |
| endpoint catalog | 9 | 1 |
| send / risk / preview | 13 | 0 |
| the rest | 41 | 4 |

**"The rest: 41 → 4" is the row that does not survive contact.** Reaching it
means deleting thirty-seven distinct operations, which is not a refactor — it
is either removing features or folding them into one wide function behind an
enum, and C2 rejects that shape explicitly ("UniFFI enums are worse to hold
than UniFFI methods").

C2's arithmetic was done when free functions stood at 153, and it assumed the
categories could keep collapsing at the rate the first passes managed. They
have not: the distribution is flat now — nine exports in the largest file, one
to four in most — so each pass removes one or two, and 121 more would be forty
rounds of that.

*This is C2's own test failing on C2's own number:* **"A target the shape cannot
reach is worse than no target, because it never reads as met."** The
category rows for registry lookups, endpoint catalog and send/risk/preview
still have real headroom — about 30 between them. The rest do not. A target
of roughly **145** is what the arithmetic supports; ~60 is not reachable
without changing the design decision C2 made deliberately.

Exports **181 → 180**.

**Two exports that were a lookup the caller already had, and a predicate over
its own sibling.**

`history_pagination_chain_id(chain_name)` was
`Chain::from_display_name(name).map(str_id)` and nothing else — both Swift call
sites had `Chain(displayName:)?.id` in scope. Gone.

`core_private_key_hex_is_likely` was `len == 64 && all hex` over
`core_private_key_hex_normalized`'s own result: a normaliser and a predicate
over it, exported separately, so a caller that wanted the key called both and
had to trust they agreed about what "normalised" meant. One
`core_private_key_hex(raw) -> Option<String>` — the normalised key, or nothing
— and the normaliser is `pub(crate)`.

*One export was left alone on purpose.* `core_unpriced_chain_names(settings)`
takes the whole `AppSettings` across the boundary to compute a filter Swift
could run against its own mirror. Moving it would be moving a **domain rule**
— which chains are unpriced because their selected network is a testnet — out
of core, which is backwards for this plan. C2's own text says what survives is
"the few genuinely pure calculations", and this is one.

Exports **183 → 181**.

**Thirteen chains sent with no destination-risk check, gated by a seven-row
table whose columns were both registry columns.**

`refreshSendDestinationRiskWarning` warns before a send to an address with no
balance and no history. Bitcoin, the EVM family, Tron and NEAR have arms of
their own; everything else went through `core_simple_chain_risk_probe_config`,
a table of seven `(display name, balance label)` pairs. **Thirty-three chains
covered, thirteen not** — including every chain whose send was enabled in the
entry above.

The probe itself is chain-agnostic: `fetch_native_balance_summary` and
`fetch_history_summary`, both keyed by chain id. Only the table limited it.

*Both of its columns are registry columns, and it disagreed with itself in two
places.* `display_chain_name` was the chain's name in six rows and **"XRP"** in
XRP Ledger's — the symbol, not the name. `balance_label` was
`"<SYMBOL> balance"` in five rows and bare **"balance"** for Litecoin and
Dogecoin. Derived from `displayName` and `gasTokenSymbol` both are consistent,
and the export and its record are gone.

*Two message changes, recorded:* XRP Ledger's probe now says "this XRP Ledger
address" rather than "this XRP address", and Litecoin's and Dogecoin's say
"currently zero LTC balance" / "zero DOGE balance" rather than "zero balance".

**And `core_supported_private_key_chain_names` was a filter made into a call.**
Its whole body was `Chain::all().filter(|c| !c.is_testnet() &&
c.derives_from_private_key()).map(display_name)`. `derives_from_private_key` is
a `ChainIdentity` column now, Swift filters the table it already reads once,
and the export, its Swift cache and the cache's stored result are gone.

Exports **185 → 183**.

**Five chains had complete, wired-up send implementations that no user could
reach.**

Probing which mainnets `route_send_asset` produces a `submit_kind` for turned
up seven that do not. One was the probe's fault — Solana routes on a
caller-supplied flag. The other six were Zcash, Bitcoin Gold, Decred, Kaspa,
Dash and Bittensor, and the app answers *"X transfers are not enabled yet."*
for all of them.

Checking what "not enabled" meant:

| | present? |
|---|---|
| `core/src/send/chains/{zcash,bitcoin_gold,decred,kaspa,dash,bittensor}.rs` | **yes — 179 to 426 lines each** |
| an arm in `service/send_execution.rs::execute_send` | **yes, all six** |
| derivation, address validation, receive | **yes** |
| a row in `route_send_asset`'s sixteen-pair table | **no** |

So roughly 1,600 lines of working send code, reachable from nowhere, because a
table of `(chain, symbol)` pairs did not name them. `plan_send_submit_preflight`
refuses on `submit_kind == None` before any of it runs.

*Five are enabled now.* They take the shared submit path —
`uses_generic_send_submit` — and their `send_execution_shape` carries
`SendFeeField::FeeSats` with the fee `execute_send` already defaults to when
the request has none: 1,000 units for Zcash, Bitcoin Gold and Kaspa, 2,000 for
Decred and Dash, in each chain's own decimals. That number is core's, not
invented: it is what the send would have used anyway, so the sheet now shows
and validates against the fee that will actually be paid.

*Bittensor is deliberately still out.* Its `execute_send` arm takes no fee
parameter and it has no shared-path preview, so the generic submit has nothing
to validate the balance against. Giving it a fallback means inventing a TAO
fee, which is not a number to invent on the funds path.

**A test was recording the symptom as a decision.**
`every_sendable_chain_has_a_routing_kind_from_the_known_set` listed all six
under a comment reading *"The chains with no send path are named, so adding one
is a decision rather than something that shows up as a dead branch."* The
intent was right and the list was wrong: five of the six had a send path, and
naming them made the gap look chosen. It names Bittensor now, with the reason.

*And a second invariant needed to grow rather than bend.*
`every_shared_path_routing_kind_has_a_preview_shape` asserted that a chain which
routes has a `simple_preview_chain`. The five have neither that nor a dedicated
preview — the fee fallback is their answer. Rather than exempt them, the
assertion now says what the rule actually is: **a chain that routes must be able
to name a fee, through a preview or through a fallback.** That is stronger than
what it checked before.

**EVM addresses were never checksum-checked, and four test fixtures were
pinning that.**

Running `address validate` against every mainnet with a malformed input found
one chain that accepted it — NEAR, correctly, because
`definitely-not-an-address` **is** a syntactically valid NEAR account id. But
reading NEAR's validator to check showed the shape: it lowercases the input on
line one and then tests `is_ascii_lowercase`, so that test can never fail.

For NEAR that is harmless — an uppercase account id cannot exist, so
lowercasing can only produce the one account the user could have meant. The
same three lines in `validate_evm_address` are not harmless.

**EIP-55 is a checksum encoded in an address's capitalisation, and it exists to
catch a mistyped or corrupted character.** The validator lowercased first and
never looked at the case pattern, so *any* forty hex digits passed. A pasted
address with one letter changed was accepted, and a send to it goes somewhere
nobody holds a key for.

`eip55_checksum` was already in `derivation/chains/evm.rs`, used to *produce*
checksummed addresses. Now the validator verifies one when the address carries
one: mixed case means a checksum and it must match; all-lowercase and
all-uppercase are the pre-EIP-55 forms, carry no checksum, and stay valid.

*Four fixtures were asserting the old behaviour.* `0x742D35CC…bC454E…` —
arbitrary mixed case, not a valid checksum — appeared in
`a_valid_address_survives_and_is_normalised`,
`one_bad_address_does_not_discard_the_good_ones`, a watch-only fixture and
`normalizes_evm_addresses`. Each of them passed only because nothing checked.
Rule 0 says fix the oddity rather than pin it: they are the all-uppercase form
now, which carries no checksum, is still valid, and still demonstrates the
trimming and lowercasing those tests are about.

*Check it from the CLI:*

```
spectra address validate --chain Ethereum 0x742d35cC6634C0532925a3b844Bc454e4438f44e
```

exits 3 — one letter's case flipped from the valid form. Three assertions in
`cli-acceptance.sh`, **144 → 147**.

**The send-preview in-flight fix reached three chains out of eighteen.**

Verifying the UTXO preview paths turned up the same shape a third time, and
this one is the previous fix half-applied.

`withSendPreviewInFlight` exists because of a bug this document already
records: the early exits called `preparingChains.remove(chainName)` **before
this call had inserted it**, so a keystroke that made the input momentarily
invalid cleared the flag guarding a request already on the network, and the
next keystroke started a second one beside it. The guard was written and
applied to **Ethereum, Dogecoin and Tron**.

`refreshSimpleChain` — which serves the other eleven chains — still had the
original code, `preparingChains.remove` on two exits above the `insert`. So did
all four UTXO previews, which had no coalescing at all: **every keystroke in
the amount field was one un-coalesced fetch**.

*Three things Dogecoin had that its own family did not.* Comparing the five
UTXO previews side by side:

| | Bitcoin | BCH / BSV | Litecoin | Dogecoin |
|---|---|---|---|---|
| request coalescing | ✗ | ✗ | ✗ | ✓ |
| destination checked before fetching | ✗ | ✗ | ✗ | ✓ |
| amount parsed at the asset's precision | ✗ | ✗ | ✗ | ✓ |

The third is why `Double(sendAmount)` accepted nine decimals of BTC, which has
eight.

*Now one `refreshUTXOChainPreview`* with all three, and the two genuinely
chain-specific pieces as parameters rather than as separate functions:
`adjust` carries Litecoin's MWEB overhead — an extension-block output costs
about a kilobyte that neither the fee nor the max-sendable reflects otherwise —
and `fetch` carries Bitcoin's xpub path, the one chain with a stored account
key, whose HD preview prices against every derived address. `refreshSimpleChain`
goes through `withSendPreviewInFlight` too, and parses at
`Chain.nativeDecimals`.

*What made this findable.* Bitcoin's and Litecoin's differences are real, so
"they have their own function" looked like the answer. The question that found
the bug was the other direction: **not what they have that the others lack, but
what the others have that they lack.**

**Five chains' tracked-token balances were fetched by nothing at all.**

Asked to look again, the same shape turned up on the balance axis.
`refreshEVMTokenBalances` and `refreshSolanaTokenBalances` were the same
function twice — filter wallets by chain, take the enabled tracked tokens,
build descriptors, call `fetch_token_balances`, apply to holdings — gated on
`isEVMChain` and on `chainName == "Solana"`.

Eighteen mainnets have a `tokenTrackingChain`: twelve EVM plus **Solana, Tron,
Sui, Aptos, TON and NEAR**. So five of them had no fetcher. Core supports all
of them — `fetch_token_balances` has arms for Sui, Aptos, TON and the rest —
and the Rust refresh engine only fetches *native* balances, so nothing else
covered the gap. A user tracking USDT on Tron saw the token and never saw a
balance.

One `refreshTrackedTokenBalances()` driven by `tokenTrackingChain` now.

*The merge is only safe because of a fix from earlier in this document.* The
two bodies differed in one line: the EVM one ran the contract address through
`normalizeEVMAddress` (lowercase), the Solana one used it raw, because a mint
address is case-significant. A naive merge would have picked one and broken the
other — and would have broken TON too, whose jetton addresses are
case-significant base64. `normalizedTrackedTokenIdentifier` is core's
`normalize_token_identifier(contract, chain)`, which already knows which chains
keep their case.

**Litecoin, Bitcoin Cash and Bitcoin SV only ever fetched the first address's
history.**

Asked why Dogecoin is special, the answer turned out to be that it is not — it
is the chain that was finished first, and the generic paths were built later
around the chains that came after. Two of its "special" pieces were a generic
function wearing a chain's name, and the third was a real capability that only
Dogecoin had been given.

**Two were nothing.** `parseDogecoinAmountInput(_:)` was
`parseAmountInput(text:maxDecimals: 8)` — a generic parser with one chain's
`native_decimals` written into it and that chain's name on the front.
`isValidDogecoinAddressForPolicy(_:wallet:)` and
`isValidUTXOAddressForPolicy(_:chainName:)` were the same function twice; the
only difference was that the Dogecoin one took a **wallet**, so it judged an
address against *that wallet's* network rather than the family's global
selection — a distinction all twenty-nine chains with a network choice have.
Merged into `isValidAddressForPolicy(_:chainName:wallet:requireDeepUTXODiscovery:)`.

**The third was a bug.** Five chains walk their addresses —
`supports_deep_utxo_discovery` says Bitcoin, Dogecoin, Litecoin, Bitcoin Cash
and Bitcoin SV — so a wallet on any of them holds many. History came in three
shapes:

| shape | who got it |
|---|---|
| xpub HD page, cursor-paginated | Bitcoin |
| **many addresses, netted per transaction** | **Dogecoin only** |
| one address per wallet | everything else, *including Litecoin, Bitcoin Cash and Bitcoin SV* |

So on those three chains, a wallet's history was fetched for its **first
address and no other**. Anything received on a discovered address beyond the
first was simply absent.

*The aggregation was never Dogecoin's.* `history_aggregate_dogecoin` groups
`NormalizedHistoryItem` by transaction hash and signs each leg against the
wallet's own addresses — netting a send against its own change output so one
transaction is one row. Nothing in the body mentions a chain. It is
`history_aggregate_by_transaction` now, over
`MultiAddressAggregateInput` / `AggregatedTransaction`, and
`refreshDogecoinTransactions` is
`refreshMultiAddressUTXOTransactions(chainName:)`.

Routing is by capability rather than by name: Bitcoin (the only chain with a
stored xpub) keeps the HD path, `supportsDeepUTXODiscovery` takes the
multi-address path, EVM its own, everything else the single-address one.
`two_legs_of_one_transaction_become_one_record` pins the netting with a
Litecoin fixture — a chain that could not reach the function before.

*Why this kept happening, stated plainly.* Every short list this document has
found was written by starting from the chains that already worked and copying
outwards. Bitcoin and Dogecoin were finished first and never folded back in, so
the generic paths were derived from the chains that came after them — and the
two originals stayed outside, taking the newer chains' features with them
whenever a capability was scoped by name instead of by column.

**Chain-name dispatch: 169 → 98, in five passes.**

Asked to remove all of it, including two things this document had recorded as
deliberate keeps. Both turned out to be worth doing; one of them found a
display bug on the way.

**`SendPreviewStore` is keyed now — 34 → 30, and four switches → none.**
Eighteen `var <chain>SendPreview` fields had the chain list written out four
times: an eighteen-arm `apply`, an eighteen-arm `taggedPreview`, an
eighteen-line `resetAll` and an eleven-arm `apply(SimpleChainPreview)`. Storage
is `previewBySlot: [String: SendPreview]` keyed by
`previewSlot(forChainNamed:)`, which asks the registry — so the EVM family
shares Ethereum's slot without anyone naming its members.

*The eighteen typed accessors are fifteen, and that is the floor.* Measuring
what callers actually read off a preview: **36 of the reads are
`estimatedNetworkFee`**, which every preview carries, and everything else is
one or two reads of a chain-specific field — Ethereum's nonce, Dogecoin's
max-sendable, Sui's gas budget. Three fields (ICP, NEAR, Polkadot) had no
reader left at all once the generic ones moved to
`estimatedFee(forChainNamed:)`. What remains names its slot twice, in a getter
and a setter, which is less than four switch arms per chain.

**The send sheet's seventeen-arm fee formatter is one line — and two of its
arms were right where the registry was wrong.** Each arm named a chain, its fee
symbol and a format specifier. Both are registry columns. Checking all
seventeen before collapsing: **Bitcoin and Internet Computer** formatted at
eight decimals while `send_execution_shape` said six. Satoshis and e8s both
need eight, so the view was right — `fee_decimals` had been transcribed from
"the ten call sites that carried these inline", and **Bitcoin's call site was
its own bespoke arm, not one of the ten**, so the UTXO chain the table's own
comment is about was missing from it. Fixed in the registry and pinned by
`utxo_and_e8s_chains_use_eight`; collapsing the view before checking would have
shipped a truncated Bitcoin fee.

**Eleven fee cards stopped passing what the chain name implies.**
`simpleFeeContent` took `isPreparing` and a `(fee, symbol, specifier)` triple at
every call site; all four follow from the chain. What a caller still supplies is
the footer sentence and whatever that chain shows beside its fee — content the
registry has no column for.

**Both address-hint tables are gone — 48 literals — and the fact went where it
belongs.** A fifteen-arm switch of terse examples ("bc1q…", "r…", one arm
naming nine EVM chains) and an eleven-entry dictionary of translated sentences
said the same thing: what an address on this chain looks like. The terse form
is a chain fact and is now `address_prefix_hint` in `chains.toml`, which the
EVM family gets from `is_evm` rather than from a nine-name arm that was missing
Base, Polygon, Linea and the rest. The sentences are **content**, so they moved
into the locale files keyed by chain id — with their existing zh-Hans and
zh-Hant translations carried across, not rewritten. A chain with no sentence
falls back to a template built from its prefix hint, so the fallback now covers
every chain that has an example rather than the eleven that had a sentence.

*This is the answer to "it is content, not migration".* It was content — and
content belongs in the content files, keyed by the thing it is about. Moving it
there is migration.

**Four more derived from columns that already existed.**
`transactionIconChainSlug` named six chains out of the eighteen
`tokenTrackingChain` knows, so a token on Polygon, Base, Sui, TON or NEAR got
no icon lookup at all; the slug is never parsed, so `chain.id` serves.
`displayNetworkName(for transaction:)` and its title twin tested
`chainName == "Bitcoin" || == "Dogecoin"` where `hasNetworkChoice` is the
column. And the receive path's four `(symbol, chain)` pairs are
`supportsDeepUTXODiscovery && symbol == gasTokenSymbol`.

**What is left, and why it is not the same thing.** 98 sites, and the two
largest groups are honest:

- **30 in `SendPreviewStore`** — fifteen accessors naming their slot, which is
  what a typed accessor over a keyed store costs.
- **20 in `AppState+SendFlow`** — Ethereum's ENS resolution, nonce and
  replacement transactions, and its self-test suite. Ethereum genuinely has
  ENS; naming it is not a list that can drift.

The rest are per-chain risk-probe messages and Dogecoin's own diagnostics —
features one chain has, not a subset of a feature every chain has. **The test
that matters is not the count: it is whether adding a chain requires touching
the display layer, and for everything above the answer is now no.**

**A zero-amount send of eleven chains' own gas token was refused by iOS after
core had permitted it.**

`submitSend`'s EVM arm opened with:

```swift
if holding.symbol != "ETH" && holding.symbol != "BNB", amount <= 0 { … refuse }
```

Whether a zero amount is allowed is `allows_zero_amount`, which
`plan_send_submit_preflight` computes as `is_native_evm_asset` and **already
refuses on** two dozen lines earlier. So the check was redundant where it
agreed and wrong where it did not: a zero-amount send of AVAX, HYPE, ETC, POL,
MNT, S, BERA, CELO, CRO, SEI or OKB reached Swift with core's blessing and was
turned away by a two-symbol list. Deleted; core's answer stands.

*Two redundant guards went with it.* `guard evmChainContext(for:) != nil`
opened the arm, but `EVMChainContext(chainName:)` is nil exactly when the chain
is not EVM, and `submitKind == "ethereum"` is core's statement that it is.
And `let rustSupportsChain = spectraEvmChainId != nil; guard rustSupportsChain,
let chainId = spectraEvmChainId` asks the same question twice in one statement.

*And `feeDecimals: 6` became `sendExecutionShape.feeDecimals`.* The registry
answers 6 for every EVM chain today, so nothing moves — but the literal was the
last thing in this arm that a chain could disagree with silently.

*Five more dead functions*, found by re-running the sweep after the import
slice: `classifySolanaDerivationPreference`, `loadResourceUncached`,
`runTimedChainRefresh`, `settingAddress`, `normalizeJettonMasterAddress` — 49
lines. Re-running the sweep after each structural change is now worth doing by
default; it has found something every time.

*What was looked at and left.* The ICP arm is the closest of the remaining
seven to `submitNativeChainSend`: same address resolver, same request shape,
no fee field. Two things stop it folding. Its `send_execution_shape` says
`supports_private_key: false` while the arm accepts a private key, and the
generic path **requires a fee estimate** where the ICP arm requires none — so
folding would refuse a send whenever the ICP preview was unavailable. The
second is a real behaviour change on a funds path with no offline test, and
`fee_fallback` would have to carry ICP's fixed fee for it to be safe. Left
until that is worth doing on its own.

**`import_wallets` derives its own addresses now, and `spectra wallet import`
takes more than one `--chain`.**

*Was:* `WalletImportCommit` carried `resolved_addresses`, so core stored what it
was handed rather than deriving what it needed. Both front ends derived first:
the CLI in Rust for one chain, iOS in Swift for many. The rule that **every EVM
chain derives from Ethereum's path** — because the family shares one address
slot — therefore lived only on the iOS side, since the CLI imported one chain
at a time and never met it. A registry fact, kept in whichever caller happened
to need it.

*Now:* the commit carries an optional `seed_phrase`, and a signing import that
leaves `resolved_addresses` empty gets them from
`derive_import_addresses(seed, selected_chains, paths, overrides)` — which adds
Ethereum whenever any EVM chain is selected, reads each path through
`CoreSeedDerivationPaths::path_for` (so a testnet resolves to its mainnet), and
skips a chain that will not derive rather than failing the import.

**`--chain` is repeatable**, which it could not be while the caller derived:
`spectra wallet import --chain Bitcoin --chain Ethereum --chain Solana` seals
the seed once per wallet and creates three. `wallet new` still refuses more
than one and says why.

*The addresses check against a fixture this document already trusted.* The
three-chain import derives
`BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX` for Solana — the same address
`cli-acceptance.sh` has asserted for that mnemonic since the single-chain path
existed. Four new assertions, **140 → 144**.

*iOS lost about seventy lines and a rule it should not have owned.* The path
table, the "add Ethereum if any EVM is selected" line and the per-chain loop
are gone; `importWallet` passes the seed and core does the rest. The CLI lost
`derive_address`, dead once nothing called it.

**Two ways to derive an import's addresses, on the key path, and neither
branch could tell you why.**

`importWallet` is the second-largest function in `swift/` at 321 lines, and its
derivation block forked:

- a **fast path** calling `WalletRustDerivationBridge.deriveAllAddresses`, and
- an **advanced path** looping per chain through
  `WalletDerivationLayer.deriveAddress` with the user's overrides,

chosen on `overrides.isEmpty`. Followed to the bottom, both reach the same
`WalletRustDerivationBridge.derive(chain:seedPhrase:derivationPath:passphrase:
hmacKey:…)`. The fast path passes `nil` for the two override fields; the
advanced path passes `overrides?.passphrase` and `overrides?.hmacKey` — which
are `nil` exactly when the overrides are empty, which is the only time the fast
path ran. **The same call, twice, with the same arguments.**

*And the error handling around it could not fire.* `deriveAllAddresses` was
declared `throws`, so the block sat in a `do/catch` that set "Wallet
initialization failed. Check the seed phrase." Its body used `try?` on every
chain, so it returned a partial map rather than throwing. The `catch`, and the
`(null)`-message normalisation inside it, were unreachable. A bad seed never
gets there anyway — `canImportWallet` requires `hasValidSeedPhraseChecksum` and
`importWallet` guards on it — so what the dead branch cost was not a bug but the
appearance of a check.

*Now:* one loop, always with the overrides, and an explicit guard — deriving
**nothing at all** now fails with that message instead of importing a wallet
with no addresses. That is the stricter side, and it is the check the dead
`catch` looked like it was making.

`deriveAllAddresses` went with it, and so did a no-op that surfaced once the
block around it was gone:

```swift
if selectedChains.contains("Bitcoin") {
    guard let bitcoinWalletID else { importError = "…"; return }
    _ = bitcoinWalletID
}
```

`selectedChains` is `Set(selectedChainNames)` and `bitcoinWalletID` is built by
zipping over `selectedChainNames`, so the guard could not fail; the binding was
discarded on the next line. Both it and the `createdWalletIDs` array that
existed only to build it are gone.

*What this did not do.* Both the CLI and iOS still derive an import's addresses
**outside** `import_wallets` and pass them in as `resolved_addresses` — the CLI
in Rust for one chain, iOS in Swift for many, with the "every EVM chain derives
from Ethereum's path" rule living only on the iOS side because the CLI imports
one chain at a time. Folding derivation into `import_wallets` would delete that
rule from Swift and give the CLI multi-chain import; it is the larger slice this
one sits inside. See "Known open items".

**Nine `core_*` functions whose own doc comment said why they should not
exist.**

The same sweep, run over Rust. It reports 493 candidates and almost all of them
are wrong — the trap this document already records twice: an
`#[uniffi::export]` function's callers are in Swift, a `#[test]` function's
caller is the harness, and a `fn` inside a `trait` block is a declaration, not
an implementation. Counting Swift by camelCase name, excluding test attributes
and trait bodies leaves twelve real ones.

Nine are the same thing: `core_address_validation_kind`,
`core_send_execution_shape`, `core_pending_status_poll`,
`core_chain_display_name`, `core_network_choices`, `core_chain_str_id_for_name`,
`core_seed_derivation_path_key`, `core_address_slot` and
`core_supports_deep_utxo_discovery`. Every one carries the line **"Not
exported: a column of `core_chain_identities` now."** The `#[uniffi::export]`
was removed when the identity table absorbed them and the function was left
behind — nine times, by nine passes, each of which wrote down exactly why the
body had no reason to remain.

Two more were **exported and had no Swift caller**:
`fetch_erc20_balance_typed` and `core_has_wallet_for_chain`. Dead FFI surface,
which is the kind that does not show up as dead anywhere: the binding
generator emits it, Swift compiles against it, nothing calls it.

Exports **187 → 185**, 138 lines of Rust.

*The rate of false positives is the finding.* Three sweeps this session —
comments, Swift functions, Rust functions — each needed a correction before its
output was usable, and in every case the first version was confidently wrong.
The pattern that works: run the detector, **verify a handful by hand**, then let
the compiler arbitrate the batch.

**Fifty-one dead functions, 417 lines, and 52 orphaned locale keys.**

Having found four dead functions in one file by hand, the same count run over
all of `swift/` found fifty-eight candidates. Two clusters stood out and both
are shapes this document has named before:

- **Six `enabled<Chain>TrackedTokens` on `AppState`** — one wrapper per chain
  over `enabledTokenPreferences(for:)`, every one unreferenced.
- **Fourteen error-message builders in `StaticContentCatalog`** —
  `invalidAmount`, `networkError`, `broadcastFailed` and the rest, each
  formatting a localized string nothing read. Removing them orphaned thirteen
  `CommonLocalizationContent` fields, which orphaned **52 keys across four
  locale files**. Dead code that carries dead translations with it.

Two more were dead because of the previous rounds' own edits:
`clearRPCEndpoints` and `clearFeePriorities` lost their only caller when
`resetSettingsAndEndpointsState` became a single core command.

*Two false positives, and both say something.* The compiler rejected the
deletion of `WalletBalanceObserver.onRefreshCycleComplete` and four
`SpectraSecretStoreAdapter` methods: they satisfy `BalanceObserver` and
`SecretStore`, **protocols declared in the generated UniFFI bindings**, which a
scan of `swift/*.swift` does not see. Restored. The rule for next time: a
protocol requirement can come from `swift/generated/`, so run the build before
believing the list — which is what happened here, and is why the deletion was
done in one batch with the compiler as the oracle rather than file by file.

*The disk again.* `cargo clean -p spectra_core -p spectra-cli` removed
**282,141 files and 182.5 GiB** — our own two crates' artifacts, rebuilt
hundreds of times this session, with third-party dependencies left alone. That
is the surgical version of the earlier `incremental` delete and worth reaching
for first.

**Four functions the previous round left dead.**

Moving the apply loop into core stranded `updatedTransaction`,
`updateTransactionStatus`, `statusMapByTransactionHash` and
`consumePendingSelfSendConfirmation` in
`AppState+OperationalTelemetry` — 65 lines with no caller anywhere, including
the tests. Found by listing every function in the file and counting call sites
across `swift/`, which is worth doing after any slice that replaces a loop:
**the compiler does not warn about an unused method on a type.**

**The status-poll apply was a full round trip through the caller. Core owns it
now.**

*Was:* six steps, five of them moving data core already had.

1. Swift read its transaction projection
2. built a `ResolvedPendingTransactionInput` per transaction — old status, old
   failure reason, old confirmations, all fields of a stored record
3. core computed a `ResolvedPendingTransactionDecision` per transaction
4. Swift applied each decision to build a new record
5. Swift upserted those back into core
6. Swift updated its projection

Only the resolutions in step 2 came from outside — they are what the network
said. Everything else went out of the store and came back to it.

*Now:* `apply_resolved_pending_statuses(chain_name, resolutions,
stale_failure_reason)`. Core reads its own transactions, computes its own stale
failures — the previous entry's method, now internal and no longer exported —
runs the same planner, **writes the results**, and returns a
`TransactionStatusChange` per record: what it was, what it is, whether that is
worth an event or a notification.

Swift builds the resolutions, makes one call, and does the two things this
platform owns: the localized text of an operational event, and the
notification. Its projection is re-read from core rather than reconstructed.

*What crossed the boundary and no longer does.*
`ResolvedPendingTransactionInput`, `ResolvedPendingStatusInput`,
`ResolvedPendingTransactionDecision`, `FailureReasonDisposition` and
`StalePendingFailureTransactionInput` all lost their `uniffi::Record` /
`uniffi::Enum` derives. Two `ResolvedPendingStatus` and
`TransactionStatusChange` replaced them: **five records and one enum out, two
records in**. Two service methods became one.

*The one thing the caller still supplies is the right one.*
`stale_failure_reason` is the text stored when a transaction is given up on,
and it is localized, which core cannot be.

*The localized-text-in-storage bug is fixed in the same place.* That reason was
being stored **as a localized sentence**, so a user who changed language kept
the old one on old records. Core stores `FAILURE_REASON_STUCK` — a code —
and `TransactionRecord.localizedFailureReason` makes the text at render. The
`stale_failure_reason` parameter is gone with it, so the caller supplies
nothing here at all.

That the previous round could not see it is the point: with the reason
travelling through Swift as a string, "the caller owns the localization" was a
reasonable-sounding conclusion. Once core wrote the record, the question became
what belongs *in* the record, and text does not.

*Now assertable, which it was not.*
`applying_a_resolution_stores_it_and_reports_the_change` writes a pending send,
applies a confirmation, and reads the record back out of the database. Nothing
on either side used to assert that what the planner decided ever reached
storage — the write happened in Swift, and the Rust tests stopped at the
decision.

**`stale_pending_failure_ids` was handed a copy of transactions core stores.**

The caller built a `StalePendingFailureTransactionInput` per transaction out of
an id, a creation time and a pending flag — three fields of
`CorePersistedTransactionRecord`, which core writes and reads. Rule 3's shape:
core computes over state the caller keeps a copy of.

Now it takes a chain name and reads its own transactions. The chain stays a
parameter on purpose — the sweep is per chain, and reading every transaction
would mark sends failed on chains the caller was not polling.

*The filter came from the registry, not from a new parameter.* Two of the three
call sites tracked sends only; the third passed `requireSendKind: false` for
Litecoin, whose explorer confirms receives on its own cadence. That flag is
already `Chain::pending_status_poll`'s `require_send_kind`, so core reads it
rather than taking it — the same fact that decides whether the Recheck button
appears.

`StalePendingFailureTransactionInput` no longer crosses the boundary; its
`uniffi::Record` derive is gone.

*Two private copies of one constant, found on the way.*
`SWIFT_REFERENCE_EPOCH_OFFSET_SECS = 978_307_200.0` was defined in both
`fetch/transactions.rs` and `store/wallet_db.rs`, each `const` and private, and
needed a third time here. It now sits once in `persistence_models.rs`, beside
the doc comment that explains why `created_at` is on the Swift reference date
at all.

*The test had to change shape, which is the point.* It used to hand the service
a synthetic input list. It now opens a store, writes a pending send, and asserts
the same rule — age alone is not failure — plus that another chain's sweep does
not pick it up. A test that could still pass a list would no longer be
exercising the path the app takes.

**Three history questions, one fetch each, asking about the same response.**

`fetch_history_has_activity`, `fetch_history_entry_count` and
`fetch_history_confirmed_txids` each ran the same `fetch_history` and applied a
different projection to the result — and the first was `entry_count > 0`, so it
could have been computed by any caller holding the second. No caller wanted two
of them at once, so this was surface area rather than a duplicated round trip,
but it is the shape C2 means by "seventeen questions".

Now one `fetch_history_summary`. It does not compute anything new:
`diagnostics_history_summary(json) -> HistorySummary { entry_count,
confirmed_txids }` already existed in `diagnostics/aggregate.rs`, exported and
unused by this path. **A first attempt defined a `HistorySummary` record in
`service/network.rs` with the same two fields and failed to compile on a
duplicate UniFFI symbol** — the boundary caught a duplicate a person had just
written, which is the one place in this codebase where that happens
automatically.

Exports **189 → 187**; the three `WalletServiceBridge` wrappers became one.

*A note on the machine this runs on.* The iOS suite failed mid-slice with
`lipo: No space left on device`. `target/debug` had reached 111 GB across
hundreds of builds this session — 98 GB of `deps` and 16 GB of rustc
incremental cache. Deleting `target/debug/incremental` freed 13 GB and cost
nothing but a slower next build. Worth knowing before a long session: the three
gates are cheap in time and expensive in disk.

**416 lines of comment removed**416 lines of comment removed: changelog entries, banners that named the
function below them, and doc comments that restated a signature.**

Three kinds, found by three sweeps.

**Changelog paragraphs — 379 lines.** Most doc comments written during this
migration open with what the code does and then continue with what it used to
be: *"It replaced `ethereumRPCEndpoint`, one String read through an accessor
that was…"*, *"Ten forwarding accessors used to name five chains twice each"*,
*"This tail was written out eight times"*. That history belongs here, in this
document, at greater length — in the source it is a diff pinned to a line that
has moved on. Removed by taking any paragraph that *opens* with history and
carries no forward-looking constraint.

*The anchoring matters.* A first pass matched paragraphs that merely
**contained** history and would have removed 709 lines — including
`networkChainByFamily`'s *"Core owns it; this is the mirror the UI binds to…
**Absent means mainnet**. It replaced three typed properties…"*, where a fact
the reader needs sits in the same paragraph as the history. Opening-anchored,
that paragraph survives. Paragraphs that say **must**, **never**, **cannot**,
**on purpose**, **stricter**, **drift** or **absent means** were kept
regardless.

**Divider banners — 28 lines.** `// ─── Dogecoin derivation index parser ───`
directly above `pub fn core_parse_dogecoin_derivation_index`. Removed where
70% or more of the banner's words appear in the declaration under it; the ones
that name a *group* of following items are untouched.

**Doc comments restating a signature — 9 lines.** *"Delete history records by
ID."* on `delete_history_records`, *"Derive Litecoin testnet keys."* on
`derive_litecoin_testnet`.

*What was kept, and why it is not the same thing.* `// XRP amount:
0x4000000000000000 | drops`, `// args: Vec<u8> (u32 len + bytes)`,
`// yoctoNEAR (1 NEAR = 10^24 yoctoNEAR)`, the CBOR shapes in
`cardano.rs` — wire formats, unit conversions and magic constants that the
code cannot state itself. A comment earns its line when a reader who has the
code still does not have the fact.

*The whitelist was protecting the worst offenders.* The first pass kept any
history paragraph containing **drift**, **wins**, **stricter**, **on purpose**
or **deliberate** — on the theory that those words mark a constraint. They do
not. They are the words a comment uses when it is **announcing a bug it
defeated**, which is the least useful thing a comment can do: the reader has
the fixed code in front of them and does not need to be told what the broken
version looked like. `settingsIconTint` kept six lines about two colours that
had disagreed; the reader needs one line saying it reads the catalog.

Narrowed to genuine imperatives — **must**, **never**, **do not**, **cannot**,
**beware**, **absent means** — a second pass removed 47 more lines across eight
files, and four bare `///` markers left where a first line had been deleted.

Density: Swift 9.5% → **8.4%**, Rust 11.2% → **11.0%**. Total removed: **463
lines**.

**The sweep itself was the bug. Fixing it found four more.**

*What was wrong with it.* The chain-list scan required an array literal on
**one line** with three or more chain names. Real lists wrap. Rewritten as a
six-line sliding window counting *distinct* registry names or ids, it found
four more production sites the single-line version had reported clean for
several passes.

**`AdvancedSettingsView.singleChainRefreshNames` was the same twenty-two names
again**, byte-identical to `AddressBookView.supportedChains` including their
ordering — one list, copied, gating two unrelated features. It decides which
chains get a "refresh this chain" button, and
`performUserInitiatedRefresh(forChain:)` takes any name, so twenty-four
mainnets simply had no button. Now `Chain.mainnets`.

**`ChainWikiEntry.accentColor` was a twenty-arm switch that could not run.**
It was the fallback for `ChainRegistryEntry.entry(id:)` returning nil. A wiki
entry is built from `listAllChains()` with `id: chain.id`, and
`ChainRegistryEntry` is built from the same call filtered on a non-empty name —
no catalog chain has an empty name, so the lookup always resolves. Dead, and
five chains short of the registry, which is what it would have been wrong about
had it ever run.

**`TokenTrackingChain.settingsIconSlug` returned its own case name.** Eighteen
arms of `case .ethereum: return "ethereum"` — the registry id, written out.
Now `chain?.id`.

**And `settingsIconTint` had actually drifted.** A second eighteen-arm switch,
this one assigning a colour per chain, where the catalog already carries
`color`. Two arms disagree: **Ethereum is `.blue` here and `purple` in
`chains.toml`; Solana is `.purple` here and `green` there.** Every other screen
reads the catalog through `ChainRegistryEntry.color`, so the same chain was
tinted differently depending on which screen you were looking at. The catalog
wins.

*Left alone: `SetupView.popularChainSelectionIDs`.* Eight ids, hand-written —
but "popular" is a curation decision, not a registry fact. A list is the right
shape for it.

**Three multi-line chain lists in `views/`, each gating a feature to a subset
of the registry.**

*How they were missed.* The chain-list sweep this document has leaned on
required the array literal to sit on **one line** with three or more chain
names on it. These three span three or four lines each, so a scan that had
reported "clean" for several passes was reporting on its own regex.

**The address book offered twenty-two chains of forty-six.** `supportedChains`
was a hand-written picker list, so a recipient on Base, Polygon, Bitcoin Cash,
Zcash or twenty others could not be saved — not because saving would fail, but
because there was no row to select. `addressBookAddressValidationMessage` and
`canSaveAddressBookEntry` would have accepted all of them, and every mainnet
has an `address_validation_kind`, so none of the twenty-four could have gone
unchecked. Now `Chain.mainnets.map(\.displayName)`.

**The decimal-display screen listed twenty-one chains of forty-six.**
`decimalExamples` was twenty-one `(symbol, chain)` pairs, so twenty-five
chains' native display precision could not be adjusted at all — including
every one of the thirty-five whose decimals this document fixed earlier. Every
symbol in the list agreed with `gasTokenSymbol`: a correct transcription,
twenty-five rows short. Now derived from `Chain.mainnets`.

**The send sheet named thirteen EVM chains among thirty.**
`networkSendChainNames` decided card-versus-`noNetworkPreviewCard`, and
`estimatedNetworkFeeText` had the same thirteen names in a `case`. All
twenty-three EVM chains share `ethereumSendPreview`, so the other ten — Sei,
Celo, Cronos, opBNB, zkSync Era, Sonic, Berachain, Unichain, Ink, X Layer —
had a preview to show and were shown the degraded card with no fee line. Both
now test `chain.isEVM`. The seventeen non-EVM names stay: they are
`SendPreviewStore`'s per-chain fields, which this document has already assessed
and kept.

*What was left.* `addressPrompt`'s terse format examples ("bc1q…", "r…") stay
incomplete. Writing one per chain is content in four locales, which is the
authoring gap already recorded — a chain without an entry gets an empty
placeholder and still validates, which is the same trade as before and now
covers more chains rather than fewer.

**Nine copies of four helpers, found by hashing function bodies rather than by
reading.**

*How they were found.* The chain-name sweep that has driven most of this
document returns nothing in production code any more, so this pass hashed every
normalised function body in `swift/` and `swift/views/` and looked for
collisions. That found four groups the eye had missed, in files nobody had
reason to open together.

**Four ways to format a localized string.** `AppLocalization.format`,
`localizedStoreFormat`, `walletFlowLocalizedFormat` and
`dashboardComponentsLocalizedFormat` — all four
`String(format:locale:arguments:)` over `AppLocalization.string(key)`, byte for
byte, with 103 / 46 / 8 / 1 callers. Now one: `AppLocalization.format`, which
already had the most callers and sits with the rest of the localization code.

**Three copies of the address-slot fold.** `CoreImportedWallet.addressMap` and
`WalletImportAddresses.slotMap` were byte-identical down to near-identical doc
comments; `WalletImportWatchOnlyEntries.slotMap` is the list-valued variant.
And `Chain(displayName:)?.addressSlot ?? ""` followed by an is-empty guard was
written five times. Now `addressSlot(forChainNamed:)` plus two overloads of
`addressSlotMap` — two rather than one because the list variant concatenates
where the scalar overwrites, which is a real difference and the only one.

**Three page headers.** `sendPageHeader`, `receivePageHeader` and `pageHeader`,
identical to the pixel. One `spectraPageHeader` in `ViewExtensions`.

**Two copies of the EVM endpoint list.** Both view files built "RPC endpoints
plus explorer supplements" privately. It is catalog access rather than view
code, so it moved to `AppEndpointDirectory.evmEndpointsWithSupplemental` beside
the two lists it reads.

*One group was left alone.* `go(to:)` in the send and receive flows normalises
to the same body, but its parameter is `SendFlowStep` in one and
`ReceiveFlowStep` in the other, and it mutates `@State` on different view
structs. A protocol and a generic to share five lines is worse than the five
lines. The detector cannot see the type difference; a person has to.

**`spectra network` — the axis that hid the reset bug now has a command.**

*Was:* nothing read or set `network_chain_by_family` outside the iOS picker.
`StateCommand::SelectNetworkChain` existed and `AppSettings::network_chain`
resolved it, but no CLI command reached either, so neither
`cli-acceptance.sh` nor any Rust test covered the axis. That is how "reset to
defaults" came to put **three** families back to mainnet where the registry has
**twenty-nine**, and stay that way.

*Now:* `network list` prints every family with a choice, what it is on, and
what it could be on; `network set <chain-id>` selects one, refusing an id the
registry does not know. A family's own id selects its mainnet, which is the
same rule the reset uses.

Nine assertions in `cli-acceptance.sh`, **131 → 140**. Three of them are the
previous bug, stated as a test: move two families onto testnets, run
`settings reset --yes`, assert both are back on mainnet. Before this command
that sequence was untypeable.

*It does not fit the `CHAIN_KEYED` settings table*, which is why it is its own
command rather than `settings set network.<family>`: that table's `update`
returns an `AppSettingUpdate`, and this is a `StateCommand` — a different
reducer arm with side effects of its own (reserved indices and discovered
addresses belong to the network they were derived on).

*One trap, hit twice now.* Two of the nine assertions failed first time because
their needles assumed JSON field order. `--json` output sorts keys, so
`"selected":"bitcoin","isTestnet":false` is never a substring — `isTestnet`
sorts first. Needles here must be key-local or written in sorted order.

**Resetting settings was iOS-only, and it worked by restating nineteen
defaults core already defines.**

*Was:* `resetSettingsAndEndpointsState` put settings back by assigning each
mirror the value it believed was the default — `.coinGecko`, `.usd`,
`.openER`, `""` four times, `10`, `.balanced`, and the network families — and
`AppUserPreferences.resetToDefaults` added seven more: strict RPC, three
notification toggles, the refresh cadence and the two large-movement
thresholds. **Nineteen literals across two Swift files, every one a second copy
of a `default_*` in `state.rs`**, and nothing on either side could compare them.
They agreed when checked; the network-family list in the middle of them did
not, which is the previous entry.

`core_reset_dispatch`'s doc comment argued the reset should stay in Swift:
*"every action it dispatches is platform (Keychain deletes, `UserDefaults`,
URL caches). There is no core-owned state behind it to move."* That was true
when written and stopped being true as settings, alerts, the address book,
token preferences and pinned assets moved into core. The dispatch rule — which
scopes imply which — is still a calculation the caller applies, and that part
of the comment stands.

*Now:* `StateCommand::ResetAppSettings` sets `settings = AppSettings::default()`,
so the defaults exist once. Swift sends the command and applies the answer;
`resetToDefaults` keeps only the five preferences this platform owns —
hiding balances, appearance, Face ID, auto-lock, biometric-gated sends — which
have no core default to be a copy of.

*Awaited, not spawned.* The seven mirrored preferences are no longer assigned
locally, so they are only correct once core's answer lands.
`resetSettingsAndEndpointsState` is `async` and awaits the round-trip rather
than firing a `Task`, which removes the window where the mirrors and core
disagree.

*Check it from the CLI:*

```
spectra settings reset --yes
```

Without `--yes` it exits 2 naming what would be discarded. Four assertions in
`cli-acceptance.sh`, **127 → 131**. In Rust,
`resetting_settings_restores_every_default` mutates *every* field, asserts the
reset returns each to `AppSettings::default()`, and asserts a second reset
emits no event — so a field added to `AppSettings` and forgotten in the reducer
fails there rather than quietly surviving a reset.

*Why this side:* rule 3. Core owning a value's default and a front end owning
its reset is the split that lets the two disagree, and this is the last place
in the settings path where they could.

**A received Litecoin transaction still could not be rechecked, because the fix
landed one layer below the gate.**

*Was:* `retryUTXOTransactionStatus` was fixed in an earlier pass to read
`Chain::pending_status_poll` instead of a five-name list, and its comment
records why: Litecoin is `require_send_kind: false` because its explorer
confirms receives on its own cadence, so a received Litecoin transaction should
be recheckable. The **context menu that offers the button** was not touched. It
read:

```swift
if row.transaction.kind == .send, row.transaction.status == .pending || .failed {
    if ["Bitcoin", "Bitcoin Cash", "Bitcoin SV", "Litecoin", "Dogecoin"].contains(…) {
```

The five names were a correct transcription — the registry has exactly those
five. The bug is the outer `kind == .send`, which is the rule the fix removed,
still enforced one layer up. **The function accepted the case; nothing could
reach it.**

*Now:* `TransactionRecord.supportsStatusRecheck` — a sibling of the existing
`supportsSignedRebroadcast` — carries the whole rule, and both the menu and the
function read it. The outer gate is `status == .pending || .failed` only;
`kind == .send` lives inside the property, for the chains whose registry row
asks for it.

*Why this side:* a predicate with two readers should have one definition. Both
copies were right when written and the fix only reached one of them, which is
the argument for not having two.

**"Reset to defaults" reset three chain families out of twenty-nine.**

*Was:* `for family in ["bitcoin", "ethereum", "dogecoin"] { selectNetworkChain(family) }`.
Twenty-nine mainnets have a network choice, so a user who had switched Solana
to devnet, XRP or Litecoin to testnet, or any of twenty-three others, kept that
selection through a full reset. Network selection decides which chain's
addresses and balances are shown, so a reset that silently leaves it is worse
than one that fails loudly.

*Now:* `for family in Array(networkChainByFamily.keys) { selectNetworkChain(family) }`.
The map is keyed by family and absent means mainnet, so its keys are exactly
the families with something to reset, and selecting a family's own id selects
its mainnet. No list, and **less** work than before: each `selectNetworkChain`
is an async round-trip to core, and the old code fired three unconditionally
where this fires none when nothing is off mainnet.

*Why this side:* the state already knows which families moved. Naming three was
a guess about which ones a user would change.

*Not checkable from the CLI.* There is no command for network selection or for
reset — see "Known open items".

**Twenty-two of twenty-three EVM mainnets could not be pointed at a private
node.**

*Was:* `AppSettings.ethereum_rpc_endpoint` was one `String`, and the accessor
that read it was
`chainName == "Ethereum" ? configuredEthereumRPCEndpointURL() : nil`. So the
setting existed for Ethereum and for nothing else — Base, Polygon, Arbitrum and
the rest were pinned to the catalog's public RPC pool with no way to override,
from any front end. `DiagnosticsViews` had a `case .ethereum:` that prepended
the override to the endpoint list, and every other EVM chain fell through to
the catalog list without one.

*Now:* `rpc_endpoint_by_chain`, on the same `HashMap` pattern
`fee_priority_by_chain` and `network_chain_by_family` already use, with
`AppSettingUpdate::RpcEndpoint { chain, value }`. An unknown chain is refused;
an empty value clears the override rather than storing a blank. Swift mirrors
the map and `configuredEVMRPCEndpointURL(for:)` reads it for any chain, so the
`case .ethereum:` folds into the EVM default.

*The UI followed, rather than being left behind.* `customRPCField(for:)` is one
field used by both the Ethereum section and the EVM default case, so all
twenty-three chains have it on the endpoints screen. Its placeholder was
`customEthereumRPCURLPlaceholder` — "Custom Ethereum RPC URL" — now
`customRPCURLPlaceholder`, generalised in all four locale files. That is a
one-word deletion per locale, not new copy.

*And the CLI's keyed-setting support stopped being a special case.* The
fee-priority slice added a single `FEE_PRIORITY_PREFIX` branch to `field()`.
A second keyed family would have been a second branch, so there is now a
`CHAIN_KEYED` table of `{prefix, read, update, stored}` and `Setting::ChainKeyed`
covers both. `list` walks the table, so a third family is a row.

*`EndpointField::EthereumRpc` was renamed `EvmRpc`.* Its rule is
`is_valid_http_url` — it was never Ethereum-specific; the name was.

*Why this side:* a setting the user picks per chain is one fact keyed by chain,
which is the third time this document has said so. One field per chain is how
the count of chains that have the feature comes to be one.

*Check it from the CLI:*

```
spectra settings set rpc-endpoint.Base https://base.internal.example
```

`settings get rpc-endpoint.Polygon` reads `""` for a chain never set; setting
`""` clears an override; `rpc-endpoint.Nonsuch` exits 3. Five assertions in
`cli-acceptance.sh`, **122 → 127**.

**Endpoint diagnostics ran down two paths over the same catalog. Now there is
one.**

*Was:* `AppCoreChainEndpoints` carries two slices of
`AppEndpointDirectory.json` — `evm_rpc` (records with the `rpc` role) and
`diagnostics_checks` (records with a `probe_url`) — and Swift probed each with
different code. `diagnostics_checks` required a `probe_url` and **no EVM RPC
record has one**, so twelve mainnets got an empty list: Arbitrum, Optimism,
Avalanche, Base, Ethereum Classic, Hyperliquid, Polygon, Linea, Scroll, Blast,
Mantle and Monero. Their screens were not blank — `evmEndpointChecks` read
`evm_rpc` instead, labelled the rows "Configured RPC" / "Fallback RPC", and
probed them with its own `probeEthereumRPC`.

Ethereum shows what that cost: the RPC path ran, so its five RPC nodes were
checked, and the two explorer endpoints that `diagnostics_checks` would have
returned were never probed. Meanwhile `ethereumExplorerProbeChecks` and
`bnbExplorerProbeChecks` hardcoded three probe URLs in Swift that are
**byte-identical** to the `probeURL` values already in the catalog.

*Now:* `diagnostics_checks` takes a record if it has a `probe_url` **or** it is
an RPC endpoint on a chain with a health method — an RPC endpoint is probed by
POSTing to itself, so it never needed one. Every chain runs
`runCatalogEndpointReachabilityDiagnostics(for:)`. Empty mainnets: **12 → 1**
(Monero, whose backend URL is a setting rather than a catalog record). Ethereum
goes from 5 rows to 7, the two new ones being the explorers it was skipping.

Deleted: `evmEndpointChecks`, `runPureEVMEndpointDiagnostics`,
`runEVMChainEndpointDiagnostics`, `runSimpleChainEndpointDiagnostics`,
`runSimpleEndpointDiagnostics`, `runSimpleEndpointReachabilityDiagnostics`,
`runLabeledEVMEndpointDiagnostics`, `runEVMExplorerEndpointDiagnostics`,
`probeEthereumRPC`, and both hardcoded explorer lists.
`AppState+DiagnosticsEndpoints`: 746 → 628 lines.

`evm_rpc` stays — it is the RPC list the fetchers use, not a diagnostics
artifact, with callers in `WalletServiceBridge`, `EndpointsViews` and
`DiagnosticsViews`. Its name is wrong (it is `ENDPOINT_ROLE_RPC`, so NEAR and
Polkadot each return three) and renaming it is its own small slice.

*On labels.* The EVM path labelled rows "Fallback RPC"; the catalog path always
passed `label: ""` and the row shows its URL. The merged path keeps `""` — an
endpoint is identified by its URL, and three rows all reading "Fallback RPC"
told the reader less than the hostnames already on screen. "Configured RPC" is
kept, on the one row that is not in the catalog. Deriving per-provider labels
was tried and dropped: `providerID` is the literal string `rpc` on every RPC
record, and the id's last segment collides within seven chains and yields
"io" / "pro" / "network" for Tron.

*Not verified in the simulator.* This changes what every endpoints screen
shows, and this plan says running the app is a fourth gate. Device access was
not granted this session, so it was not run. The residual risk is low rather
than absent: nothing new crosses the FFI asynchronously — `probeJSONRPC` and
`diagnosticsProbeJsonrpc` already carried NEAR and Polkadot — and the only new
field is a plain `Option<String>` on `ChainIdentity`. What is unverified is the
rendering, not the plumbing.

**Which endpoints get a JSON-RPC probe was decided by two hand-written lists
in Swift, beside a catalog that already carries the role.**

*Was:* `runNearEndpointReachabilityDiagnostics` and
`runPolkadotEndpointReachabilityDiagnostics` were near-copies of each other,
and each classified its endpoints itself.
`NearBalanceService.rpcEndpointCatalog()` named three endpoint ids; anything
in it got a `status` JSON-RPC probe, anything else a GET.
`PolkadotBalanceService.sidecarEndpointCatalog()` named one id and was tested
*inverted* — the sidecar got the GET and everything else got
`chain_getHeader`. A third method, `eth_chainId`, was spelled inside
`probeEthereumRPC`.

`AppEndpointDirectory.json` carries an `rpc` role on every endpoint record, and
`ENDPOINT_ROLE_RPC` already existed in `app_core.rs`. Both Swift lists agreed
with it when they were written. What they invited is the drift: a fourth NEAR
provider means editing the JSON *and* remembering `ChainTypes.swift`, and
forgetting the second probes a JSON-RPC node with a GET — which many of them
answer `405`, reported here as unreachable.

*Now:* `Chain::rpc_health_method()` holds the three methods (`eth_chainId` for
every EVM chain, `status` for NEAR, `chain_getHeader` for Polkadot), and
`AppCoreDiagnosticsCheck` gained `rpc_probe_method`, set when the record
carries the `rpc` role. The two Swift functions are one
`runCatalogEndpointReachabilityDiagnostics(for:)` that reads it, and both id
lists are deleted.

*Pinned:* `the_catalog_decides_which_endpoints_are_rpc` asserts NEAR's three
RPC nodes get `status` while its history API keeps a GET, and Polkadot's
sidecar keeps a GET while its three RPC nodes get `chain_getHeader` — the exact
classification the two Swift lists produced.
`a_chain_without_a_health_method_probes_over_http` walks every other chain.

*Why this side:* the catalog already knew, and rule 2 puts the method beside
the rest of the per-chain facts.

*Found, not fixed — there are two endpoint-diagnostics paths.* Twelve mainnets
have **no** `diagnostics_checks` at all (Arbitrum, Optimism, Avalanche, Monero,
Base, Ethereum Classic, Hyperliquid, Polygon, Linea, Scroll, Blast, Mantle),
because `diagnostics_checks` only includes records with a `probe_url` and no
EVM RPC record has one. Their screens are not blank: EVM chains are probed
through a second path entirely, `evmEndpointChecks` reading
`EVMChainContext.defaultRPCEndpoints`. So one chain's endpoints come from the
catalog and another's from a context record, with different probe logic each.
Now that an RPC endpoint is probed by *method* rather than by a GET against a
probe URL, `probe_url` is no longer required for one — the builder could
include RPC records and the two paths could become one. That is its own slice;
see "Known open items".

**Two files of view code were sitting in the root.**

*Was:* `ViewExtensions.swift` (300 lines — `extension View`, `extension
Binding`, `extension Color`, `ContentView`, `SpectraShimmer`,
`SpectraLoadingGlyph`) and `StakingViewModel.swift` (266 lines — a
`@MainActor @Observable` view model whose only reader is
`views/StakingView.swift`) were in the root of `swift/`, which this document
defines as `AppState`, stores, persistence and bridges.

*Now:* both are in `views/`. `views/` is a `PBXFileSystemSynchronizedRootGroup`,
so the move is a `git mv` plus deleting the eight explicit `PBXBuildFile` /
`PBXFileReference` / group-membership lines that would otherwise have compiled
them twice.

*What was checked and left.* Scanning the root for SwiftUI view code —
`: View`, `some View`, `@ViewBuilder` — returns `ViewExtensions` and nothing
else, so the root holds no other literal views. Three files that look like
candidates are not: `DiagnosticsState` is `@Observable` but persists to
UserDefaults, so it is domain state by rule 4; `StakingBridge` is a bridge,
which the root's definition names; `DashboardStore` imports SwiftUI but is an
`extension AppState`.

*And the reverse direction was checked*, because a ratio has two sides:
`views/` contains no `persistCodableToSQLite`, `UserDefaults`, Keychain,
`WalletServiceBridge.shared`, `storedSeedPhrase` or `storedPrivateKey` — no
domain logic to bring back out.

**Be clear about what this metric move is.** Root 15,852 → **15,286**, UI
10,736 → **11,302**, so the gap closed by 1,132 while *no domain logic moved*.
Filing two files correctly does not pay down debt; it stops the metric
mis-reporting where the debt is. The remaining gap is **3,984 lines**, and
closing it means moving domain logic into `core/`, not moving more files.

**The tail of every send arm was written out eight times.**

*Was:* each of the eight broadcast arms in `AppState+SendExecution` ended with
the same sequence — build a `TransactionRecord`, decorate it, record it, run
the post-send refresh, reset the composer clearing that chain's preview. **Six
of the eight were byte-identical.** Ethereum differed in three fields it
passes; Dogecoin genuinely differs, running its own refresh sequence instead of
`runPostSendRefreshActions`.

*Now:* `recordSuccessfulBroadcast` holds it, taking the nonce, payload format
and verification status Ethereum needs as defaulted parameters. Seven arms call
it. Dogecoin keeps its copy, because what it does is not the same thing.

*And the flag each arm sets is keyed off the chain again.* Every arm wrote
`sendingChains.contains("Bitcoin")` / `.insert(…)` / `.remove(…)` with the name
spelled three times — eighteen literals across six arms, each equal to
`holding.chainName`, which is already in scope. They read it now.

*Ethereum keeps its literal, deliberately.* It locks on `"Ethereum"` rather
than the holding's chain, so all twenty-three EVM chains share one flag while
Sui and Aptos have theirs. That is over-broad rather than wrong — there is one
send composer, so the flag exists to stop a double-tap, and a family-wide key
is the stricter side of that. Noted rather than changed: narrowing it would let
two EVM sends run at once, which nothing has asked for.

`AppState+SendExecution`: 676 → 651 lines, 44 → 25 chain literals.

*Why this side:* the six copies were identical, so there was nothing to
preserve, and the two that differ now say so by not calling the helper.

*One warning fixed in passing:* `refreshEVMTokenTransactions` bound
`guard let chain = evmChainContext(…)` and never used `chain`. It was the only
Swift warning in the build; there are none now.

**`spectra send` no longer broadcasts; `spectra send broadcast` does, and
`spectra send assemble` shows what it would sign.**

*Was:* `send` was a bare verb that signed and broadcast, and assembling the
transaction it signs was reachable from nowhere but the iOS send sheet. That is
the gap the two EVM bugs above lived in: `prepare_evm_send_assembly` has no
caller in `core/` or `cli/`, so it greps as dead from the Rust tree, and
`cli-acceptance.sh` had no way to reach the seven-chain list inside it.

*Now:* `send` is a subcommand group. `send broadcast` is the old command,
unchanged apart from its name. `send assemble --chain <name> --from <addr>
--to <addr> --amount <n> [--symbol S --contract C --decimals D]` prints the
`value_wei`, `to` and `data` an EVM send would sign, taking **no key, no
network and no store** — a pure function over its arguments, so it runs against
an empty data directory like everything else here.

Ten assertions in `cli-acceptance.sh` drive it: Base assembles (one of the
sixteen that could not), the amount lands in wei, ARB assembles as a contract
call with `valueWei == "0"` addressed to its contract, and malformed addresses,
a non-EVM chain and half a token description are each refused with the right
exit code. **107 → 122.**

*Verified the gate actually closes.* Reverting both tables to their old form
and re-running turns six of the ten red, and the ARB row prints the bug in one
line: `{"isNative":true,"to":"<recipient>","valueWei":"100000000000000000000",
"data":"0x"}` — a hundred ARB assembling as a hundred ETH. Before this command
existed, all three suites stayed green on that.

*Why `broadcast` rather than the bare verb:* the irreversible half of this tool
should take a word that says so. `send` already refused to run without `--yes`;
this is the same argument one level up, and it costs one word to type.

*Why this side generally:* rule 1 says the test is the CLI. An exported
function whose only caller is a Swift view is the case the rule exists for, and
"the CLI cannot drive it" turned out to be the load-bearing fact in how two
funds-path bugs survived every suite.

**Sixteen of twenty-three EVM mainnets could not send at all, and two
governance tokens were previewed as the gas asset.**

*Was:* `send/ethereum.rs` carried two hand-written tables, and `send/mod.rs` a
third, all answering questions the registry holds.

`is_supported_evm_chain` named **seven** chains. `prepare_evm_send_assembly`
gates on it, and it is what the send sheet calls to build the transaction it
estimates gas for. On the other sixteen EVM mainnets — Base, Polygon, Linea,
Scroll, Blast, Mantle, Sei, Celo, Cronos, opBNB, zkSync Era, Sonic, Berachain,
Unichain, Ink and X Layer — it returned `UnsupportedChain`, Swift caught it and
set `ethereumSendPreview = nil`, and `submitSend`'s EVM arm then stopped at
`guard let preview else { sendError = "Unable to estimate … network fee" }`.
**The send was blocked, and the message named the fee rather than the cause.**

`is_native_evm_asset` listed nine `(chain, symbol)` pairs, two of which named a
governance token: `("Arbitrum", "ARB")` and `("Optimism", "OP")`. Sending ARB
built `value_wei = amount × 10¹⁸`, `to = the recipient`, `data = 0x` — a plain
transfer of that many **ETH** — and discarded the ARB contract Swift had
already looked up and passed in. The broadcast path is separate and still sent
the token, so funds were not misdirected; what broke is the estimate. It priced
a 21,000-gas value transfer instead of a ~65,000-gas ERC-20 call, and simulated
it against an ETH balance the wallet need not have, so the preview could fail
outright on a send that was fine.

`native_evm_symbol_for_chain` named the same seven, so on the sixteen the
chain's own gas token did not count as native and `allows_zero_amount` was
false with it.

*Now:* all three read the registry. Supported is `chain.is_evm()`; native is
`chain.is_evm() && chain.coin_symbol() == symbol`, `coin_symbol` being the
catalog's `gas_token_symbol` — which is the question those nine pairs were
badly asking. The hardcoded `18` in the native branch reads
`chain.native_decimals()` instead; all thirty-three EVM rows are 18 today, and
a chain that was not would have been off by orders of magnitude on the funds
path.

*Why this side:* twenty-three chains are offered in the picker, given RPC
endpoints by the catalog, and priced — a list of seven deciding which of them
can actually spend was not a policy, it was a list nobody extended. And ARB is
not what Arbitrum charges gas in, which is the only thing "native" meant here.

*How this was nearly missed.* Grepping `core/` and `cli/` for callers of
`prepare_evm_send_assembly` returns nothing, and it reads as dead. Its only
caller is `AppState+SendPreview.swift:105`. **An exported function's callers are
not in the Rust tree** — the same lesson as the `list_all_builtin_tokens`
restore above, in the direction that costs more.

*Check it from the CLI:* not reachable — assembling an EVM send is behind
`prepare_evm_send_assembly`, which the CLI has no command for. Pinned in Rust
instead: `every_evm_mainnet_assembles_a_native_send` walks every EVM mainnet the
registry knows, and `a_governance_token_is_not_the_gas_asset` asserts ARB and OP
assemble as contract calls with `value_wei == "0"`. This is a gap worth closing
— see "Known open items".

**Two lists of chain names decided which sends take the shared submit path.**

*Was:* `AppState+SendExecution` had two adjacent `if` blocks with **identical
bodies** — `["sui", "aptos", "ton", "xrp", "stellar", "cardano", "polkadot"]`
plus a NEAR check, then `["bitcoinCash", "bitcoinSV", "litecoin"]` — both
calling `submitNativeChainSend` with the same five arguments. The comment above
them already said the lists should not be there. A twelfth chain joining the
shared path had to be added to whichever of the two the author was looking at.

*Now:* `Chain::uses_generic_send_submit` holds it, and
`SendSubmitPreflightPlan.uses_generic_submit` carries core's answer across. The
two blocks are one `if preflight.usesGenericSubmit`. Core decides rather than
the registry alone because NEAR qualifies for its native asset and not for a
token on it — a question about the asset, not only the chain.

*Verified against the old lists before switching:* a probe ran every chain in
the registry through `route_send_asset` and compared the two answers. Zero
disagreements, including the NEAR-token case.

*Why this side:* which chains share a submit path is a fact about the chains,
and rule 2 puts it on `registry::Chain`.

**Fee priority was one preference kept in three places, and the CLI could set
two chains out of seventy-eight.**

*Was:* `AppSettings` carried `bitcoin_fee_priority` and
`dogecoin_fee_priority` as their own String fields. The other seventy-six
chains shared `selectedFeePriorityOptionRawByChain`, a dictionary iOS persisted
to SQLite itself — domain state living in Swift, rule 4. Reading it meant going
through `feePriorityOption(for:)`, which branched on the two chain names before
falling back to the dictionary, and writing it meant the mirror branch in
`setFeePriorityOption`.

Three Swift enums spelled the same three cases: `ChainFeePriorityOption`,
`BitcoinFeePriority` and `DogecoinFeePriority`, the first two byte-identical
down to `displayName`. Four functions mapped between them, each body one
`init(rawValue:) ?? .normal`.

There was also a fourth store, already dead: `StoreLifecycleReset` seeded the
dictionary from a UserDefaults key nothing had written since the move to
SQLite. The comment four lines above it says exactly that about the *other*
keys it stopped seeding.

*Now:* one field, `fee_priority_by_chain`, on the same `HashMap` pattern
`network_chain_by_family` already uses — absent means `normal`, so the map is
empty until the user picks something. `AppSettingUpdate::FeePriority { chain,
value }` replaces the two per-chain variants. Swift mirrors the map beside
`networkChainByFamily` and diffs it into per-chain updates through the settings
path every other setting uses; both duplicate enums, all four mappers,
`bitcoinFeePriority(for:)`, `persistSelectedFeePriorityOptions` and both load
paths are gone.

*Two things got stricter on the way through.* Core now refuses a chain the
registry does not know rather than storing a preference under a name nothing
will read, and normalizes the value to one of the three the picker offers — a
send path that only understands `economy` / `normal` / `priority` should not
find `lightspeed` stored where a fee was supposedly chosen.

*And the diagnostics screen had a second picker for the same setting*, bound
to `$store.bitcoinFeePriority` while the send screen bound the shared
accessor. Both wrote the same property, so they agreed; with the property gone
it binds the shared accessor too. The control stays — it is a duplicate model,
not a duplicate feature.

*Why this side:* a preference the user sets per chain is one fact keyed by
chain. Two chains having fields of their own is the shape rule 2 names, and it
is why the CLI could set exactly those two.

*What this does not fix:* fee priority reaches a live fee estimate on Dogecoin
only — `fetchDogecoinSendPreviewTyped` and the Dogecoin broadcast are its
only consumers. On the other seventy-seven it is stored and read back and
nothing spends it, which the picker's own caption already admits
(*"Some networks still use provider-managed fee estimation in this build."*).
Left as it is: making it live is per-chain fee work, not migration, and
deleting the picker is a product call.

*Check it from the CLI:*

```
spectra settings set fee-priority.Dogecoin economy
```

`spectra --json settings get fee-priority.Solana` reads `normal` for a chain
never set; `settings set fee-priority.Solana lightspeed` stores `normal`;
`settings set fee-priority.Nonsuch economy` exits 3. `settings list` prints
only the chains that have a preference — all seventy-eight would bury the
seventeen settings that are not per-chain.

**Thirty-five chains formatted their amounts at six decimal places instead of
eight, nine, ten, eighteen or twenty-four.**

*Was:* `supported_decimal_places` looked up `SUPPORTED_DECIMAL_CHAINS`, a
hand-written table of **twenty-two** chains, and fell back to a literal `6` for
everything else. `chains.toml` carries `native_decimals` on all seventy-eight
rows.

The twenty-two agreed with the catalog exactly — it was a correct transcription
and fifty-six rows short. What fell through: **every EVM chain outside the
original thirteen** (Base, Polygon, Linea, Scroll, Blast, Mantle, Sei, Celo,
Cronos, opBNB, zkSync Era, Sonic, Berachain, Unichain, Ink, X Layer) at 6 places
instead of 18; Zcash, Bitcoin Gold, Decred, Kaspa, Dash and Internet Computer at
6 instead of 8; Bittensor at 6 instead of 9; and every testnet.

Six places on an eighteen-decimal asset is not a rounding preference — a balance
below 0.000001 displays as zero, and the field that formats a send amount
truncates what the user typed.

*Now:* the catalog is the answer, for all seventy-eight.

*Why this side:* there is no other side. The table was trying to be the catalog.

*Checkable without the app:*
`native_decimals_come_from_the_catalog_for_every_chain` walks every row and
asserts the two agree — which the twenty-two-row table could not do, and which
is why it stayed short for as long as it did.

**The same degraded-sync message localized on one path and not on the other.**

*Was:* three exports asked three questions about one detail string —
`diagnostics_detail_indicates_live_success`,
`diagnostics_normalize_degraded_detail` and
`diagnostics_degraded_detail_template_key` — and the two Swift callers asked
different subsets in different orders. `markChainDegraded` matched the template
against the **raw** detail; `localizedDegradedMessage` normalised first and
matched against that.

The templates match on a suffix, and a detail carrying `" Last good sync:
10:00 AM"` does not end with one. So the identical message resolved to
`"%@ providers are unavailable…"` and was localized on one route, and fell
through to `localizedStoreString(detail)` — the raw English, chain name
inlined — on the other.

*Now:* `diagnostics_classify_degraded_detail(detail)` returns all three facts
at once and matches the template against the normalized form, so both routes
get the same answer.

*Why this side:* matching the normalized form is what the template list was
written for — every entry is a bare suffix. The raw-form match was the one that
could only work when the caller happened to have stripped the suffix already.

*Checkable without the app:*
`a_detail_with_its_suffix_still_matches_its_template` asserts the raw form does
*not* match — the bug, stated — and that the classification does.

**Nineteen EVM mainnets did not follow Ethereum's display-decimals setting.**

*Was:* `native_asset_display_settings_key` decided which chain's decimals
setting an asset reads, and the EVM family shares Ethereum's because they share
a native asset the user configures once. It named **three** of the twenty-three
EVM mainnets — `matches!(chain_name, "Ethereum" | "Arbitrum" | "Optimism")` — so
setting ETH's display decimals moved Arbitrum and Optimism, and left Base,
Polygon, BNB Chain, Avalanche and nineteen others reading a key of their own
that nothing writes.

*Now:* `Chain::is_evm` is the membership, as it is everywhere else.

*Why this side:* the whole point of the shared key is that ETH is one asset;
having twenty of the twenty-three quietly opt out makes the setting look broken
rather than absent.

*Checkable without the app:* the rule is one `is_evm()` call now, and the same
one every other EVM arm in this document ended up reading.

**A TON jetton address was lowercased on one path and not the other.**

*Was:* two functions normalised a token's contract address for a chain.
`normalize_dashboard_contract_address` in core special-cased Sui and Aptos and
lowercased everything else; `normalizedTrackedTokenIdentifier` in `AppState`
had a twelve-name EVM arm, then Aptos, Sui, **TON** and a lowercase default. A
jetton master address is case-significant base64, so the two answers for TON
differ — one resolves and one does not, depending on which path reached it.

*Now:* one `normalize_token_identifier(contract_address, chain_name)`, keyed by
`Chain`, with TON's rule stated beside Sui's and Aptos's. The `token_standard`
argument went too: it was already `_token_standard`.

*Why this side:* the side that keeps the address usable. TON was right and the
default was wrong for it, which is why the copy that knew was the app's.

*Checkable without the app:* `token_identifier_normalisation_is_per_chain`
asserts `"  EQAbC  "` on TON comes back as `EQAbC`, case intact.

**Staking never worked on iOS. Every call failed before it reached a client.**

*Was:* `StakingService`'s exported impl block was `#[uniffi::export]` with no
`async_runtime = "tokio"`. It is the only block in the crate with `async fn`s
that lacked it. UniFFI polls the future with no reactor installed, so every
staking call from Swift returned *"there is no reactor running, must be called
from the context of a Tokio 1.x runtime"* — validators, positions, every action
preview, on all seven chains, always. The staking tab rendered its copy and then
put an error alert on top of it.

*Now:* the attribute is there. The Solana page loads a hundred live validators
with their real commissions.

*Why this side:* there is no other side. The tab was inert.

*How it was found, which is the part worth keeping:* by opening the app. Neither
gate could see it. `cli-acceptance.sh` drives `StakingService` from inside the
CLI's own runtime — `ctx.rt.block_on(...)` — so the missing attribute is
invisible there, and it is *because* the CLI supplies a runtime that the Rust
tests pass too. The iOS suite has no test that reaches the network. Three green
suites and a dead feature, which is the honest limit of "proven by the CLI": it
proves core's rules, not that the boundary is wired.

**A keystroke that made the amount invalid dropped the guard on a send preview
already in flight, and Tron's preview kept showing the previous amount's fee.**

*Was:* the three debounced previews — Ethereum, Dogecoin, Tron — each inlined
the same "one in flight at a time" bookkeeping, and each also called
`preparingChains.remove(chainName)` on **every** early exit. Five of Ethereum's
eight, three of Dogecoin's and two of Tron's run *before* the flag is set. At
those points the call has not claimed the flag, so the `remove` did not clear
its own: it cleared whatever call was actually on the network. Type a character
that makes the amount momentarily invalid and the guard protecting a live
request is gone; the next keystroke starts a second request beside it, and
whichever finishes last wins.

Tron's guard also differed from the other two: `guard !contains else { return }`
with no `pendingSendPreviewRefreshChains.insert`, so a request arriving while one
was in flight was **dropped** rather than retried — the preview then showed the
fee for the amount before the one on screen.

*Now:* `withSendPreviewInFlight(_:retry:body:)` owns the flag for all three.
Nothing outside it touches `preparingChains`, so an early exit cannot reach it,
and Tron coalesces like the others.

*Why this side:* a fee preview that does not match the amount on screen is the
kind of wrong that looks right, and this is the number a user checks before
signing. The stricter side is the one where the last request wins and the guard
means what it says.

*Checkable without the app:* not from the CLI — this is debounce state in
`AppState`. What changed structurally is that `preparingChains` has one writer.

**A diagnostics reset put back sixteen of the rows it had just cleared.**

*Was:* `resetDiagnosticsState` emptied `historyRunByChain` and
`endpointHealthByChain`, and then ran a thirty-two entry key-path list setting
`isRunning` and `isChecking` to `false` on sixteen named chains. Both subscripts
insert a default row on write — `set { map[chainName] = newValue }` over a
getter that returns `.init()` when the key is missing — so the loop did not
clear anything. It wrote sixteen default rows back into maps the two lines above
it had emptied, and left the other chains' rows genuinely gone. The block above
it carries a comment saying an earlier belt-and-suspenders version of exactly
this "said nothing it does not"; this one said something, and it was wrong.

*Now:* the loop is deleted. The two map assignments reset every chain, which is
more than the sixteen and is what the function is for.

*Why this side:* a reset that reinstates rows is not a reset, and the rows it
reinstated were the sixteen someone last remembered — a list that was already
seven chains short of the diagnostics screens.

*And ten lines naming the five UTXO chains became `utxoRescanStateByChain = [:]`,*
which likewise clears the map rather than five keys of it.

*Checkable without the app:* not from the CLI — this is `AppState`'s own reset
path. The check is that neither map is written after being cleared.

**Seven chains' stored addresses were not counted as the wallet's own, and
eight kinds of wallet showed no address at all.**

*Was:* four separate "all of this wallet's addresses" lists, each written out by
hand, each a different length, none of them twenty-four.

`knownOwnedAddresses` appended seventeen — no TON, Zcash, Bitcoin Gold, Decred,
Kaspa, Dash or Bittensor — so an address stored on one of those was not among
the addresses the app considers its own. The wallet detail row built sixteen and
took `.compactMap { $0 }.first`, which is "prefer Bitcoin, then Bitcoin Cash, …"
dressed as a fallback, and left out eight chains entirely: a Zcash or TON wallet
had no address to show. `makeAddressSnapshots` listed eighteen, six short. And
the receive view's `walletStaticAddress` switched over twenty-four arms, of
which the EVM arm named twenty-three chains and read Ethereum's slot even for
Ethereum Classic, which has its own.

*Now:* all four read `address(forChainNamed:)`, which keys on
`Chain::address_slot`. `knownOwnedAddresses` and `makeAddressSnapshots` walk the
registry in catalog order; the detail row shows the wallet's own chain's
address, which is what a per-chain wallet has; the receive view is one line and
reads Ethereum Classic's own slot.

*Why this side:* every one of the four was trying to say "the addresses this
wallet has", and the storage has been a slot-keyed map since the address book
moved. A list that has to be extended by hand when a chain is added is a list
that will be one chain short, and all four of them were.

*Checkable without the app:* `spectra wallet show` prints the addresses a wallet
carries, and the storage it prints from is the same map all four now read.

**"Load more" was offered on every chain and worked on eighteen.**

*Was:* `canLoadMoreHistory` asks `history_pagination_chain_id`, which answers
for **every** chain the registry knows — so the button appears for any wallet
whose paging is not exhausted. `loadMoreOnChainHistory`, which answers the tap,
iterated three hand-written lists: five UTXO names, twelve EVM names, and Tron.
Everything else fell through and nothing happened.

That is eleven EVM chains — Ethereum Classic, Sei, Celo, Cronos, opBNB, zkSync
Era, Sonic, Berachain, Unichain, Ink and X Layer — and every account-based
chain: Solana, XRP Ledger, Stellar, Cardano, Sui, Aptos, TON, Internet
Computer, NEAR, Polkadot, Monero, Zcash and the rest. A user on any of them
could tap "Load more" as often as they liked and see the same first page.

*Now:* the chains to page are the chains the eligible wallets are on, and the
function already had that set — `eligibleWalletIDs` — before throwing it away
for the lists. Bitcoin and Dogecoin keep their own fetch, every EVM chain pages
through the token history, and everything else goes through the normalized
path, whose own comment already claimed it covers "any future account-based
chain".

*Why this side:* the button's presence and the button's effect were answered by
two different rules, and the one the user sees was the more generous. Making
the effect match the offer is the only direction that does not take a working
control away from the eighteen.

*Checkable without the app:* `every_chain_can_be_paged` in core walks all
seventy-eight and asserts `history_pagination_chain_id` answers for each — that
is the half deciding which wallets are offered the control, and it is now
deliberately total rather than accidentally so.

**A used Bitcoin receive address could be handed out again.**

*Was:* `hasUTXOOnChainActivity` had three arms asking different questions.
Bitcoin's asked "any UTXOs, or any confirmed balance" and never looked at
history. Bitcoin Cash, Bitcoin SV, Litecoin and Dogecoin asked "any balance, or
any history" and never looked at the UTXO count.

An address that received and was then fully spent has no UTXOs and no balance,
and only history. So on Bitcoin — and only on Bitcoin — that address reported
"no activity", and the receive flow reads exactly this to decide whether a
reserved index has been used and should be stepped past. The four chains that
checked history stepped past it; Bitcoin re-issued it.

*Now:* one question for all of them — any UTXOs, any balance, or any history —
over the chain set `Chain::supports_deep_utxo_discovery` names, which brings
the testnets in with their mainnets.

*Why this side:* handing out a used receive address is an address-reuse leak
that links two payments to the same person, and the strictest reading of "has
this been used" is the union of the three signals. The widening for the other
four (they now also count UTXOs) is in the same direction.

*Checkable without the app:* not from the CLI — it takes a live address with
spent history. What is checkable is that the chain set is
`supports_deep_utxo_discovery`, which core tests, rather than three arms and a
`default`.

**Ten EVM mainnets got no destination-risk warning, and twenty threw their
token-transfer diagnostics away.**

*Was:* four hand-written EVM chain lists in the send path, none of them the
registry's twenty-three. The destination-risk probe named thirteen and everything
outside them fell to `default:`, which is `warning = nil` — so sending to a fresh
or contract address on Sei, Celo, Cronos, opBNB, zkSync Era, Sonic, Berachain,
Unichain, Ink or X Layer raised nothing at all. The two address-hint switches
named the same thirteen, so those ten got "Enter an address for the selected
chain." where the rest got their format. The ENS gate named twelve, for a rule
that is "Ethereum" — `resolveEVMRecipientAddress` refuses every other chain, so
eleven of the twelve were routed into a call that throws.

And `StoreHistoryRefresh` built an `EthereumTokenTransferHistoryDiagnostics`
record for **every** EVM chain and then decided where to file it from a
three-arm expression — Ethereum family, Arbitrum, Optimism — dropping the other
twenty on the floor. The diagnostics registry is keyed by chain and takes any of
them; there was no reason for the arm beyond nobody having generalised it.

*Now:* the probe arm, both hint arms and the diagnostics filing read
`Chain.isEVM` and `Chain.mainnetCounterpart`. The ENS gate says `== "Ethereum"`.

*Why this side:* the risk warning is a funds-safety surface — "this address has
no balance and no history, check the recipient" is the last thing between a typo
and a loss — and a feature that silently covers 13 of 23 chains is worse than
one that covers none, because the ten look checked. Ethereum's own, more
specific hint went in the collapse: keeping it would mean Arbitrum reading
"Ethereum addresses must start with 0x", and the `%@` form is accurate for the
whole family including Ethereum.

*Checkable without the app:* not from the CLI — these are UI strings and a
network probe. `testEveryEVMChainGetsAFormatSpecificAddressHint` walks every EVM
mainnet and asserts none of them gets the generic fallback, comparing against a
chain that has no arm rather than against English text, so it holds in any
locale.

**A received Litecoin transaction can be rechecked.**

*Was:* `retryUTXOTransactionStatus` gated on a five-name list,
`transaction.kind == .send` for all five, and `chainName == "Dogecoin"` for
whether to clear finality. All three are `Chain::pending_status_poll`, which
says `Utxo { tracks_finality, require_send_kind }` — and Litecoin is
`require_send_kind: false`, precisely because its explorer confirms receives on
a different cadence than the send path assumes. The blanket `.send` check
disagreed with that, so the one chain the registry singles out was the one the
recheck refused.

*Now:* the guard reads the poll policy. Bitcoin, Bitcoin Cash, Bitcoin SV and
Dogecoin still require a send; Litecoin does not; finality is cleared where the
registry says the chain tracks it. Testnets come in with their mainnets, which
they did not before.

*Why this side:* the registry's field exists to state that exception and is
tested on the core side; the Swift copy was a second opinion, and the second
opinion was wrong.

*Checkable without the app:* `pending_status_poll` is covered in core; the
recheck itself is a UI action over a live chain, so what changed is that the
guard derives from the same value core polls on.

**Monero could not be imported. Not in one mode — in every mode, on both front
ends, for as long as the catalog has had a row for it.**

*Was:* Monero's row carries `derivation_path = []`, because its spend and view
keys come from the seed directly rather than from a BIP-32 path. Every caller
read that as a broken catalog rather than as an answer.
`default_path_from_catalog` returned `Err("Missing default derivation path for
Monero.")`, so `spectra wallet import --chain Monero` exited with it, and iOS's
batch derivation skipped any chain whose path was empty — which meant the batch
produced no Monero address, which is why the import flow demanded the user
*type* one. The only field to type it into was on the watch-addresses page,
which a seed import never visits, so `typed("Monero")` was always empty and the
guard above the import refused unconditionally: *"Enter a valid Monero
address."* Watch-only was refused separately and correctly. A private-key
import excludes Monero. So there was no path to a Monero wallet at all, while
`AppState+SendExecution`, `AppState+SendPreview`, the diagnostics tables and the
receive flow all carried Monero code that nothing could reach.

Core has derived Monero from a seed phrase the whole time — `derive_monero`
takes no path and never has.

*Now:* a chain the registry knows and the catalog gives no path for answers
`Ok("")` instead of erroring, and the two callers stop treating "none" as
"broken". `spectra wallet import --chain Monero` produces
`48ZFsb…4B282s4mcDi`, which `spectra address validate --chain Monero` accepts.
The typed-address guard, the field behind it and the `record("Monero", …)` line
are deleted; Monero derives like everything else.

*Why this side:* the alternative is the one the flow was reaching for — let the
user type an address and store it against a seed the app cannot derive from.
That is the failure the receive-flow slice already named: a valid address for
the wrong key still parses, and the wallet is stored with an address its key
cannot spend. Rule 0's second limit says derive rather than trust a typed value,
and here deriving was always possible.

*Checkable without the app:* the four assertions under "a chain with no
derivation path" in `cli-acceptance.sh`, and in core
`a_chain_with_no_catalog_path_derives_without_one`, which also asserts Monero is
the only mainnet that says it and that an unknown chain still errors.
`every_registry_chain_derives_through_one_call` no longer skips chains without a
catalog path — that `continue` was covering the only chain the test needed to
catch.

**Twenty-two of the twenty-eight watchable chains had no way to enter an
address, or entered it into a slot nothing reads.**

*Was:* `watchAddressesInputsGroup` was eighteen hand-written sections and a
seven-name EVM condition, against a registry flag —
`Chain::supports_watch_only_import` — that names every mainnet except Monero. It
disagreed in four directions at once. Zcash, Bitcoin Gold, Decred, Kaspa, Dash
and Bittensor had no section, so selecting one showed a page with no field and
failed with "Enter at least one valid address to import." Sixteen EVM mainnets
— Polygon, Base, Linea, Scroll, Blast, Mantle, Sei, Celo, Cronos, opBNB, zkSync
Era, Sonic, Berachain, Unichain, Ink and X Layer — fell outside the seven the
EVM condition named, so no EVM field appeared for them either. Ethereum Classic
*did* get the shared EVM field, and that was worse than nothing: its address
slot is its own, so what the user typed landed in the `ethereum` slot and the
planner, reading `ethereumclassic`, saw an empty import. And Monero had a
section although the flag excludes it and a guard three files away refused it.

Eleven of the twenty-eight worked.

*Now:* one row per address *slot*, from the registry —
`Chain.mainnets.filter { $0.supportsWatchOnlyImport }`, first chain in catalog
order owning the row. The EVM family shares Ethereum's slot and so shares one
field; Ethereum Classic owns its own and gets its own; Monero is not in the
list. Adding a chain to `chains.toml` adds its field.

*And the validation follows the selected network.* Bitcoin resolved its address
kind through the network the family is on, Dogecoin did the same through
`isValidDogecoinAddressForPolicy`, and the other sixteen judged against mainnet
whatever network was selected — so a testnet watch address was marked invalid in
red while core went on to accept it. One rule for every chain now.

*Why this side:* the flag is what the planner enforces, so anything else is the
picker offering what the planner will refuse — the same rule the private-key
slice applied one pass earlier. Nothing was removed except Monero's field, which
had no reader.

*Checkable without the app:* `spectra wallet watch --chain "Ethereum Classic"`
and `--chain Polygon` both succeed, `--chain Monero` is refused, and
`spectra chains --json` carries `watchOnlyImport` beside `privateKeyImport`.
Five assertions in `cli-acceptance.sh`.

**Which chains accept a private-key import is one fact now, and it changed for
twenty-three of them.**

*Was:* four answers to that question, and no two of them the same.
`PRIVATE_KEY_SUPPORTED_CHAINS` in `import.rs` was a thirty-nine name array and
built the picker. `chain_supports_private_key_import` in `receive.rs` was a
separate twenty-three name `matches!` and gated the submit. Swift's
`derivePrivateKeyImportAddress` was a twenty-arm switch on chain names and
decided whether an address appeared at all. And `core_derive_from_private_key`
— the only one of the four that can actually produce an address — covers the
EVM family plus Bitcoin, Bitcoin Cash, Litecoin, Dogecoin and Decred.

Eleven chains satisfied all four. The rest failed somewhere downstream of being
offered: sixteen were in the picker and refused by the submit gate, under a
message reading *"Private key import currently supports every chain in this
build except Monero"* — which named the one exclusion it did not have.
Twelve more passed both gates and then derived nothing, failing with "Unable to
derive an address from this key" after the user had pasted a key. Decred passed
both gates and derives in core, but was missing from Swift's switch, so the app
refused an import the CLI completes. And six EVM mainnets — Polygon, Base,
Linea, Scroll, Blast, Mantle — were in neither core list, though a key derives
the same address on them as on the fifteen EVM chains that were.

*Now:* `Chain::derives_from_private_key` is the fact and all of them read it.
Twenty-eight mainnets are offered, which is exactly the set a key yields an
address for.

*Added:* Polygon, Base, Linea, Scroll, Blast, Mantle, Decred.
*Removed:* Bitcoin SV, Tron, Solana, Cardano, Stellar, XRP Ledger, Sui, Aptos,
TON, Internet Computer, NEAR, Polkadot, Zcash, Bitcoin Gold, Kaspa, Dash,
Bittensor.

*Why this side:* every chain removed is one where the import could not have
completed — the flow offered it, took the key, and failed. Offering an
operation that always fails is worse than not offering it, and this is a key
path, so the stricter side is to refuse before the key is typed rather than
after it is pasted. Every chain added derives correctly and was left out by an
omission rather than a decision. What the collapse buys beyond the twenty-three
is that widening is now derivation work instead of a list edit: implement
private-key derivation for Solana and it appears in the picker, with no second
place to remember.

*Checkable without the app:* `spectra chains --json --filter Polygon` reports
`"privateKeyImport":true`, and `wallet import --chain Polygon` yields the same
address as `--chain Ethereum`; `--chain Decred` imports; `--chain Cardano`
exits 3. Six assertions in `cli-acceptance.sh`. In core,
`the_registry_flag_and_the_dispatcher_agree_on_every_chain` walks all seventy-eight
chains and asserts the flag and the dispatcher say the same thing,
`every_offered_private_key_chain_passes_the_gate` binds the picker to the gate,
and `a_key_alone_is_not_enough_on_these_chains` names the exclusions so widening
one is a deliberate edit rather than a side effect.

**Twelve chains now ask for biometric authentication and warn about a risky
destination. They did not.**

*Was:* `submitSend` had fifteen branches, and three of them returned before the
shared gates. Sui, Aptos, TON, XRP Ledger, Stellar, Cardano, NEAR, Polkadot,
Bitcoin Cash, Bitcoin SV, Litecoin and Internet Computer all took one of those
three, so a send on any of them skipped `evaluateHighRiskSendReasons` — the
first-time-destination and wrong-network warnings — and skipped
`authenticateForSensitiveAction` entirely. A user who had turned on "require
biometrics for send actions" got it on Bitcoin, Dogecoin, Tron, Solana, Monero
and every EVM chain, and did not get it on the other twelve.

Not even consistent among the chains sharing a helper: Monero goes through
`submitNativeChainSend` too, but from a branch *below* the gates, so it was
gated and its eleven siblings were not.

*Now:* the gates run once, above the routing, so every chain passes the same
ones. `submitNativeChainSend` lost its `checkSelfSend` parameter with them —
self-send confirmation is one of the hoisted gates, and the parameter existed
because one of the three early branches needed it and the other two did not.

*Why this side:* it is a security setting that silently did nothing on twelve
chains. Rule 0's second limit — take the stricter side wherever funds are
involved — and the stricter side of "should this send ask for a fingerprint" is
yes.

*Checkable without the app:* not from the CLI, which has no biometrics. The
check is that the branch order is now gates-then-route, and that no branch
returns above the gates.

**A NEAR token the user does not track can no longer be sent.**

*Was:* the NEP-141 branch matched on `chainName == "NEAR"`, the token standard
and the presence of a contract address — its own three-part test, run after
core had already decided the send did not route anywhere. `submitKind` is
`nil` for an untracked mint, and the branch never looked at it, so core refused
the route and the caller sent the transfer regardless.

*Now:* the branch is `preflight.submitKind == "near"`, which is only set when
`supports_near_token_send` says the user tracks that contract.

*Why this side:* the same rule as the twelve chains above. A send core has
declined to route is one the caller should not make.

*Checkable without the app:* `routing_follows_the_token_list_core_holds` in
core states the Solana half of the same rule — untracked mint, no route — and
`every_sendable_chain_has_a_routing_kind_from_the_known_set` pins the kinds
`submitSend` switches on.

**The settings blob became state core owns, and four of its fields stayed on
iOS because they were never domain state.**

*Was:* `PersistedAppSettings`, a twenty-three field record. iOS built every
field from its own properties, wrote them together as one SQLite row on any
change, and read them back the same way at launch. Three consequences.

Every settings change carried a snapshot of every other setting, so two screens
editing two settings raced and the later write reinstated the earlier screen's
stale copy of everything else. The bounds — a Bitcoin stop gap of 1…200, a
refresh interval of 5…60 minutes, the two alert thresholds — were `didSet`
clamps in `AppUserPreferences`, the only copy, so a value was only in range
where someone had remembered to check. And the CLI could not read or set any of
it: which RPC to talk to, what a send's fee priority is, when an alert fires.

*Now:* eighteen fields are on `AppSettings` in `CoreAppState`, set through
`StateCommand::SetAppSetting { update }` — one variant per field, so setting one
field says one field. The reducer trims strings and bounds numbers, and the
value that comes back is the stored one. `spectra settings list|get|set` drives
the same state.

*Which four stayed, and why:* hiding balances is one front end's presentation;
Face ID, auto-lock and biometric-gated sends are one platform's capability, and
a CLI has neither the concept nor a way to honour them. They persist as a small
`PlatformPreferences` blob through the generic key-value store — a blob is the
right shape for something one front end owns entirely. The rule the header of
`AppSettings` already stated ("do not add a field here that only one front end
reads") is what decided the split; the old record simply had not been asked.

*What this deleted on the way:* a launch-time `UserDefaults` read per setting,
plus the twenty keys behind it. Nothing had written those keys since settings
moved to SQLite, so they were already dead — but not harmlessly so once core
owns the values, because each read lands in a `didSet` that commits, and every
launch would have sent core a stale seed before core's own state arrived.

*Checkable without the app:* `spectra settings set bitcoin-stop-gap 9999`
prints `200`, and a second process reads `200` back. `spectra settings set
etherscan-api-key "  KEY  "` stores `KEY`. Seven assertions in
`cli-acceptance.sh`, and `every_settings_field_round_trips` /
`a_setting_outside_its_range_is_bounded` in core.

*One thing it cost:* the iOS side grew. `Store+Settings.swift` is 192 lines of
mirror — a diff of what this front end holds against core's last answer, and an
adopt that does not overwrite an edit still in flight — where the blob writer it
replaces was about 30. Root lines went 16,347 → 16,439. The mirror is the price
of two owners becoming one, and it is the same shape `tokenPreferences` and
`priceAlerts` already pay; but "moving a subsystem into core" and "removing
Swift lines" are not the same move, and this round is a clean example of the
first without the second.

**A transaction recorded during launch is no longer deleted by the launch.**

*Was:* `reloadPersistedStateFromSQLite` read the wallet list, then the
transaction list, then called `pruneTransactionsForActiveWallets` — which
removes every transaction whose `walletID` is not in `wallets`, and persists
the removal. The two reads are a moment apart and the whole load races anything
the user does after the app is on screen, so a wallet recorded while the load
was in flight was missing from the first read; its transactions were present in
the second; and the prune deleted them for having no wallet. Permanently: the
removal goes through `removeTransactions`, which writes.

*Now:* the load adopts and rebuilds, and prunes nothing. The two prune sites
that follow a *wallet mutation* stay — there the projection is already the new
one, which is the case the prune was written for.

*Why this side:* the stricter side where funds are involved is the one that
does not delete a record of a real send. An orphaned row shows one extra
history entry until the next wallet mutation prunes it; the other failure loses
evidence that money moved. The wallet-deletion path removes a wallet's
transactions itself (`removeTransactions(forWalletID:)`), so this was cleanup
for a case that path already covers.

*How it was found:* `testTransactionStatusChangeIsPersisted` had been failing
intermittently in full runs and passing in isolation, which reads like a timing
flake. It was not — raising the poll budget from five seconds to twenty made it
fail *slower*, which is what said the record was never coming back rather than
coming back late. Recorded here rather than under "known open items" because
the fix is a behaviour change, not a repair: the load used to delete data and
now does not.

*Checkable without the app:* `spectra send broadcast` a transaction, then reopen the
store — `spectra history list` still has it. The failing version needed the
wallet write and the load to interleave, which a second process cannot
reproduce, so the test is the check.

**Swift has one chain type, and it is the registry's.**

*Was:* four Swift enums each wrote out the chain list — `SpectraChainID` (30
string ids), `SeedDerivationChain` (76 display names), `AppChainID` (30) and
`StandardDiagnosticsChain` (24) — against a catalog of 78. No two agreed.
`SeedDerivationChain` had `bnbChainTestnet` and `moneroStagenet` but neither
`BNB Chain` nor `Monero`. Around them sat the tables that keyed on them: a
24-row diagnostics dispatch of ten closures each, a 30-case endpoint-catalog
switch, a five-row UTXO action table, a 29-row `[String: SeedDerivationChain]`
map, and a 30-arm private-key derivation switch — every row differing only in a
chain's own name.

*Now:* `registry::Chain` derives `uniffi::Enum` and crosses the boundary
directly. One export, `core_chain_identities()`, hands over the catalog as
`(chain, id, name, is_testnet, diagnostics_shape)`, and
[`swift/Chain+Registry.swift`](swift/Chain+Registry.swift) builds every lookup
Swift needs from it. The tables are gone: which history record a chain reports
is `Chain::diagnostics_shape`, which endpoints it has is the endpoint catalog,
which family derives it from a private key is `Chain::is_evm` and five names.

*Why this side:* rule 2. Every one of these lists was a hand-maintained subset
of `chains.toml`, and each was wrong in a different place. There is no version
of "keep them in sync" that is cheaper than not having them.

*Checkable without the app:* `spectra chains list` is the same 78 the enum now
carries; `core_chain_identities()` is that list plus the columns, and
`chain_order_matches_the_catalog` fails the build if the enum and the TOML
drift.

**Diagnostics screens, and the diagnostics export, now cover every mainnet.**

*Was:* the hub listed the intersection of `AppChainID` (30) and
`StandardDiagnosticsChain` (24), and `diagnosticsBundleChainNames` was a third
hand-typed list of the same 24 display names. `supports_diagnostics` is `true`
for all 78 rows in the catalog, so it never selected anything — the number 24
was whatever someone had typed.

*Now:* both are `Chain.mainnets` — 46. The drivers behind the screen were
already generic over the chain name (`runEVMChainDiagnostics`,
`runUTXOChainDiagnostics`, `runSimpleChainDiagnostics` dispatch on registry
facts), so the other 22 worked all along and were simply unreachable.

*Why mainnets and not all 78:* a diagnostics screen per testnet is 32 rows of
noise for a screen that reads a wallet's history, and no wallet has to be on a
testnet for the mainnet row to be useful. This is a product judgement, not an
inconsistency being fixed, so it is recorded as one.

*Checkable without the app:* `spectra diagnostics export` writes one document
per chain in the same set; `testTheBundleListIsTheChainsTheHubOffers` fails if
one of the three lists is edited back into a copy.

**Two screens were passing "Bitcoin Diagnostics" where a chain name belongs.**

*Was:* `AppChainDescriptor` carried both `chainName` ("Bitcoin") and `title`
("Bitcoin Diagnostics"), with a comment on the enum warning that passing one
for the other "silently resolves to nothing". Two call sites did exactly that.
`StandardChainDiagnosticsView.task` asked
`chainKeypoolDiagnostics(for: chain.title)` and
`operationalEvents(for: chain.title)` — both key on the display name, so the
keypool and operational-event sections were empty on every chain's screen.
`syncChainOwnedAddressManagementState()` looped over
`diagnosticsChains.map(\.title)` and registered owned addresses under names
like "Bitcoin Diagnostics"; `resolvedAddress` returned nil for all of them, so
the whole loop was a no-op.

*Now:* `Chain` has `displayName` and no `title`. The heading is built where it
is displayed. Both sites read the chain name.

*Why this side:* nothing was choosing between the two spellings — the code
wanted the chain and got the heading. Deleting the type that carried both is
what makes the mistake unavailable rather than commented.

*What the second fix cost:* `syncChainOwnedAddressManagementState` had been
free because it did nothing. Doing it costs a reserve and a write per (wallet,
chain), and one wallet on an EVM chain resolves an address on every EVM
mainnet, since they share a slot. Two bounds: only chains that own their
address slot are visited — filing the same EVM address under twenty-five names
tells the keypool nothing Ethereum's row does not — and an address core already
holds is skipped, so the work lands on the first load after an import rather
than on every launch. The suite is faster with the loop doing real work than it
was with it doing none, because the skip removes the repeated writes.

*Checkable without the app:* `spectra diagnostics keypool --chain Bitcoin`
returns rows for a wallet that has an address; the failing version passed a
name no chain has.

**One private-key derivation dispatcher, in core.**

*Was:* `WalletRustDerivationBridge.deriveFromPrivateKey` was a thirty-arm
switch naming which chains derive by which algorithm, calling six per-chain
UniFFI exports that existed only to be called from it. Which family a chain
belongs to is a registry fact, and the switch listed `.bnbChainTestnet` under
EVM while `BNB Chain` itself was not in the enum at all.

*Now:* `core_derive_from_private_key(chain_name, …)`, matching on
`Chain::mainnet_counterpart()` — `is_evm()` plus five named UTXO chains — so
testnets fall out of the registry instead of needing a case each. The six
per-chain exports are internal helpers now. Net effect on the FFI surface is
five fewer.

*Why this side:* rule 1 — the CLI can drive private-key derivation now, and it
could not before.

*A gap this made visible:* the import gate `PRIVATE_KEY_SUPPORTED_CHAINS` listed
39 chains; derivation covers the EVM family and five UTXO chains. A private key
imported for XRP Ledger passed the gate and produced no address. This pass left
it alone — widening derivation is new work, narrowing the gate removes an import
path — and a later one closed it: the gate is
`Chain::derives_from_private_key` now, which is the same fact the dispatcher
acts on. See "Which chains accept a private-key import is one fact now" above,
and `the_registry_flag_and_the_dispatcher_agree_on_every_chain` for the test
that replaced the one this paragraph used to name.

**Sixteen mainnets now get their endpoints registered with core.**

*Was:* `buildEndpoints` listed thirty rows of `chainId: SpectraChainID.x,
chainName: "X"`, stating both halves of a fact where either determines the
other. Thirty of the catalog's forty-seven mainnets were on it. The sixteen
missing — Berachain, Bitcoin Gold, Bittensor, Celo, Cronos, Dash, Decred, Ink,
Kaspa, Sei, Sonic, Unichain, X Layer, Zcash, opBNB, zkSync Era — got no
endpoints registered at all, so any call that went through the registered list
had nothing to reach for them.

*Now:* the loop asks the catalog. A chain with no endpoint records still
contributes nothing — both payload builders already returned empty for that —
so the change is exactly "the chains that have endpoints get them".

*Why this side:* it is the same stale-partial-copy the `EVMChainContext` entry
below describes, with an overlapping list of victims. A hand-maintained subset
of a catalog is wrong by default; the question is only when someone notices.

*Checkable without the app:* the sixteen are the set difference between
`core_live_chain_names()` and the thirty names the deleted rows carried.

**A Dogecoin testnet holding is no longer priced as mainnet DOGE — in both
places the rule lived.**

*Was:* the rule existed twice. `NetworkModes::is_priced_chain` named `"Bitcoin"`
and `"Ethereum"` and let every other chain through as priced;
`plan_priced_chain` did the same thing behind `core_priced_chain(chain_name,
bitcoin_mode, ethereum_mode)`, which the render path called per coin through a
memoized wrapper. Fixing the first copy did not fix the second — the app still
quoted a Dogecoin testnet balance at mainnet DOGE prices, which is real money on
screen.

*Now:* one rule, `!selected_network.is_testnet()`, over the core-owned
selection. `core_unpriced_chain_names(settings)` hands the caller the whole set
when the state changes; the per-coin question is a set membership test.
`no_testnet_coin_is_quoted_on_any_family` covers Bitcoin, Ethereum and Dogecoin.

*Why a set and not a query:* the first attempt made it a `WalletService` method
and populated the set from a `Task`. That left the render path quoting a testnet
at mainnet prices for a runloop hop after a network switch — a smaller version
of the same bug. It is a pure function of the settings now, applied in the same
synchronous step that adopts them.

**The dashboard no longer decides whether to rebuild.**

*Was:* an FFI export, a `DashboardRebuildDecisionRequest` record, a core
planner, a Swift wrapper, a `cachedDashboardRelevantPriceKeys` cache maintained
for no other purpose, and pinned-prototype plumbing at the call site — all to
occasionally skip one in-memory pass when prices changed but no displayed price
did.

*Now:* prices change on a refresh cycle and the rebuild touches no I/O, so it
just rebuilds. Everything above is deleted.

*Why this side:* the machinery cost more to carry, and to keep correct, than the
work it avoided. This is Rule 0's second bullet.

*Was:* `NetworkModes::is_priced_chain` decided whether a coin is quoted by
naming the families with a testnet by hand — `"Bitcoin"` and `"Ethereum"`.
Dogecoin was not on the list, so a Dogecoin testnet balance was quoted at
mainnet DOGE prices and displayed as real money. The rule had been ported
verbatim from Swift, bug included.

*Now:* a coin is quoted when its selected network is not a testnet, which holds
for every family and for families added later.

*Checkable without the app:* `no_testnet_coin_is_quoted_on_any_family` walks
Bitcoin, Ethereum and Dogecoin and asserts an empty price-request set for each.

**A network mode is a chain, and the selection is core-owned.**

*Was:* three enums (`CoreBitcoinNetworkMode`, `CoreDogecoinNetworkMode`, and a
Swift-only Ethereum one) spelling out chains the registry already had as
first-class variants, persisted as three strings in the platform settings blob,
and handed to core per call as a `NetworkModes` record.

*Now:* `AppSettings.network_chain_by_family` — one map of `mainnet id ->
selected id`, behind `SelectNetworkChain`. Choosing the mainnet clears the entry
rather than storing it, so "not chosen" and "chose mainnet" are one state.
`NetworkModes` and the three settings fields are gone.

*The enums are gone now too.* `CoreBitcoinNetworkMode`,
`CoreDogecoinNetworkMode` and the Swift-only `EthereumNetworkMode` are deleted,
along with the `NetworkSelection` table that converted between them and chain
ids. What made the sixty-odd call sites tractable was finding that most of them
did not need a network at all — see below.

**`AddressValidationRequest.network_mode` was dead, and it was hiding a test.**

*Was:* the request carried a `network_mode` "retained for backwards
compatibility with stored wallets". Nothing read it — the `kind` string
(`"bitcoin"` vs `"bitcoinTestnet4"`) had always been what decided — and
prelaunch there are no stored wallets to be compatible with.

*Now:* deleted. That removed the argument from every Swift validation call at
once, which is most of what was holding the mode enums up.

*What it exposed:* `testImportingBitcoinWalletPersistsDerivedAddressOnTestnet4`
asserted that an imported testnet4 wallet's stored address was valid
*testnet4* — and passed, because it said "testnet4" through the ignored
argument while `kind` said `"bitcoin"`. It had been validating a mainnet
address as mainnet. The real behaviour is the one this document already
describes: import derives against the mainnet chain and the testnet address is
re-derived for display. The test now asserts both halves separately, and is
named for what it checks.

*Five more chain tables went with it:* `bitcoin_network_for_mode`,
`ImportNetworks`'s two per-family matches, a twenty-row chain-to-kind table in
the send-flow scanner, and a five-case one in `isValidUTXOAddressForPolicy`.
Each asked the registry a question the registry could already answer —
`Chain::bitcoin_network`, `Chain::address_validation_kind`,
`Chain::network_choices`.

**Every EVM chain gates non-native sends the same way.**

*Was:* Ethereum, BNB Chain and Avalanche required a non-native asset to be a
supported token; the other twenty EVM chains had no restriction. The same
unsupported token was refused on Ethereum and offered on Arbitrum, where it
would be accepted and then fail at submit. A test,
`send_rule_asymmetry_across_evm_chains`, pinned the three-chain list in place.

*Now:* one rule for the family. `EthereumClassic` and `Hyperliquid` stay
native-only, which is stricter still.

*Why this side:* refusing early is better than a signed transaction that cannot
land. `every_evm_chain_gates_non_native_sends_the_same_way` replaces the test
whose only function was to fail anyone fixing this.

**Import addresses are now validated for every chain.**

*Was:* the iOS import path built a `resolved<Chain>Address` local for 21 chains
— the typed value, kept whenever the chain was selected and non-empty, with no
validation — and used it in preference to the validated/derived value. Dogecoin,
Ethereum and Ethereum Classic had no such local and used only the validated
path. So whether a malformed address reached storage depended on which chain it
was typed under.

*Now:* `validated_addresses` in core runs every supplied address through
`validate_address` with the chain's own `address_validation_kind`, keeps the
normalised form, and drops what does not parse. One rule, no exceptions.

*Why this side:* a stored malformed address is worse than a missing one — it
renders as the wallet's receive address, so a user could hand it out.

*Checkable without the app:* `spectra address validate` runs the same function
the import does, and exits 3 on a refusal so a script can assert it.

```
$ spectra address validate --chain Ethereum 0x742D35CC6634C0532925A3B844bC454E4438F44E
  ✓ valid, normalised to 0x742d35cc6634c0532925a3b844bc454e4438f44e
$ spectra address validate --chain Solana not-an-address
  ✗ not a valid Solana address        # exit 3
```

*Fallout worth noting:* two existing core tests used placeholder addresses
(`"SoLaNaAddr"`, `"bc1qaddr"`) and started failing, because the rule dropped
them. They now use real ones. A fixture that a validation rule rejects was
never testing what it claimed to.

**Watch-only import addresses are validated too.**

*Was:* the rule above only ran over `resolved_addresses`. A watch-only import
reads a different input — `watch_only_entries` — so it was not validated at
all. The rule therefore covered the path whose address core derives itself, and
skipped the one where the user types it. That is backwards.

*Now:* `validated_watch_only_entries` applies the same rule to that input.
A watch-only import of a malformed address used to store a wallet with no
address; it is refused.

*Found by:* rewriting the CLI so `wallet watch` goes through `import_wallets`.
The old CLI built the `WalletSummary` itself, so it could not have found this.

**Refused import addresses are reported, not dropped.**

*Was:* `import_wallets` dropped what failed validation with a `tracing::warn!`
and carried on, so no caller could tell a validated import from a silently
emptied one.

*Now:* `WalletImportOutcome` carries `rejected_addresses`, and an import left
with nothing to plan returns `InvalidInput` naming what was refused rather than
a generic failure. iOS surfaces it through `importError`. Same shape as
`addressBookRejected`: core decides, the front end reports.

**The CLI no longer writes domain records directly.**

*Was:* its four import commands built `WalletSummary` values by hand and called
`app_state_save`; rename and delete mutated `CoreAppState` in place and saved
it. Every one of those skipped the reducer — which is why the CLI kept passing
while bypassing the validation core had just gained.

*Now:* imports go through `WalletService::import_wallets`, mutations through
`StateCommand`. The visible change: an import core would refuse now fails
instead of succeeding.

**A signing import's address comes only from derivation.**

*Was:* the import slot map read `resolved<Chain>Address ?? <chain>Address` for
23 chains, where the first is the raw typed value (kept whenever the chain is
selected and the field is non-empty, **unvalidated**) and the second is the
derived address. The typed value won.

*Now:* the 23 locals and the fallback are gone. Derivation is the only source
for a signing import; a private-key import uses a typed value only after it
validates. Bitcoin's xpub and Monero's address stay typed, because neither has
a derived counterpart.

*Why this side:* an address that is not derived from the wallet's own key is
unspendable by that wallet, and core's validation cannot catch it — a valid
address for the wrong key still parses.

*Honest scope:* this was **latent, not live**. `isWatchOnlyMode` has one writer
and it resets the draft first, so the typed fields are always empty in a signing
import. What is removed is a rule whose safety depended on an invariant two
files away, not a reachable bug. See the Stage 3 note above for the trace.

**History scalars keyed by chain; `DiagnosticsStore` is 707 → 189.**

The last uniform family: `<chain>HistoryDiagnosticsLastUpdatedAt` and
`isRunning<Chain>HistoryDiagnostics` across 24 chains — 48 stored properties and
48 forwarding pairs — onto `historyRunByChain`.

Two things the compiler caught that a careless pass would have shipped:
`StoreHistoryRefresh.swift` was not in the file list I rewrote, so it still
named the deleted properties; and a `?:` chain ending in `nil` infers a
read-only `KeyPath`, which a subscript-backed path cannot satisfy — that site
now carries a chain *name* instead of a key path, which is what it wanted all
along.

**What is left, and the finding under it.** The 62 forwards still in
`DiagnosticsStore` are the `<chain>HistoryDiagnosticsByWallet` dictionaries,
whose value types differ. Except mostly they do not: XRP, Stellar, Cardano,
Monero, Sui, Aptos, TON, ICP, NEAR and Polkadot all declare **the same four
fields** — `address`, `sourceUsed`, `transactionCount`, `error`. Swift already
says so out loud, with a `SimpleAddressHistoryDiag` protocol in
`StoreDiagnosticsExport` that exists purely to treat them as one type.

So the same move as the endpoint rows applies: ten identical uniffi records
become one, and the ten dictionaries collapse with them. Only Bitcoin
(`identifier`, `nextCursor`), Ethereum, Tron (`tronScanTxCount`,
`tronScanTrc20Count`) and Solana (`rpcCount`) genuinely differ. That is the next
slice, and it ends `DiagnosticsStore.swift`.

**Endpoint health keyed by chain — and the two row records became one.**

The blocker was not the properties, it was the types. `EvmEndpointHealthRow`
differed from `EndpointHealthRow` by a single `label` field, and two records for
one thing forced two differently-typed slots per chain, which is why this family
could not be keyed. They are one record in core now, with `label` empty for
chains whose diagnostics list endpoints without one. The bundle JSON is
unchanged: `endpoint_row_value` still omits the field and
`evm_endpoint_row_value` still emits it.

With that done, 72 stored properties and their 72 forwarding pairs collapsed
onto `endpointHealthByChain`. One wrinkle worth writing down: the generic
runners take key paths, and a key path cannot use `dict[key, default: …]` —
the subscript index has to be Hashable. So `AppState` gained a
`subscript(endpointHealthFor:)` that reads through to the table and returns the
empty state for an unknown chain, and the key paths point at that.

`DiagnosticsStore.swift`: 707 → 373. `DiagnosticsState.swift`: 500 → 436.

**The diagnostics state collapse — first family done, the pattern proven.**

Self-tests were three stored properties per chain (`<chain>SelfTestResults`,
`isRunning<Chain>SelfTests`, `<chain>SelfTestsLastRunAt`) across six chains,
each with a forwarding pair in `DiagnosticsStore`, a `run<Chain>SelfTests()`
wrapper threading three key paths, a row in the view dispatch table, and two
lines in the reset. Eighteen properties, six wrappers, five tables.

Now: one `selfTestsByChain: [String: SelfTests]`, one `runSelfTests(for:)`, and
the reset clears the table. **-102 lines**, and the per-chain abbreviation
("BTC", "BCH", …) that the wrappers carried by hand comes from the registry
descriptor, which had it all along.

The shape every remaining family copies:

1. Replace the per-chain stored properties with one keyed table on
   `WalletChainDiagnosticsState`.
2. Delete the matching forwarding pairs in `DiagnosticsStore.swift`; add one
   accessor that returns the empty state for an unknown chain.
3. Collapse the per-chain wrappers into one function taking a chain name.
4. Point the view dispatch table and the reset at the table.

What is left, in the order the win gets bigger: endpoint health (three
properties × ~22 chains, complicated only by `EndpointHealthRow` vs
`EvmEndpointHealthRow`), then the history scalars (two × ~22, fully uniform).
`DiagnosticsStore.swift` is 707 → 644 and is entirely forwarding; it ends at
zero when those two families land.

**The net for the diagnostics collapse, built and proven to bite.**

The same 24 chains are written down six times: `StandardDiagnosticsChain`,
`chainDiagDescriptors`, `DiagnosticsViews.dispatchTable`,
`diagnosticsBundleChainNames`, the `diagnosticsJSON(for:)` switch, and the 163
per-chain stored properties behind them. Collapsing that is the largest piece of
Stage 3 left, and its failure mode is a chain falling out of one list while the
others keep it.

`DiagnosticsChainTableTests` asserts the lists agree — on registry id, since the
enum spells chains as ids, the bundle as display names, and `title` as neither
("Bitcoin Diagnostics"). It says nothing about behaviour; it exists to fail when
one table is edited alone.

**A net was checked before being trusted.** Removing `"Solana"` from the bundle
list turns three of the four tests red, naming the chain. The first attempt to
verify that used a `sed` expression that silently matched nothing, so the tests
"passed" and proved exactly nothing — worth recording, because an unchecked net
and no net look identical from the outside.

**Three copies of the diagnostics chain list became one, and a test that keeps
it that way.**

*Was:* 23 near-identical `<chain>DiagnosticsJSON()` wrappers in
`StoreDiagnosticsExport`, a 24-row table below them calling all 23, and 24
closures in `DiagnosticsViews` calling them a third time.

*Now:* one `diagnosticsJSON(for:)` keyed by display name, one
`diagnosticsBundleChainNames` list, and the views pass a name.

*This one bought no lines* — 130 in, 130 out. What it bought is that the list
exists once, and `DiagnosticsBundleCoverageTests` names the chain when the
switch and the list disagree.

Collapsing the wrappers did drop **Tron and Solana** on the way — they have
their own JSON builders and did not match the shape the other 22 shared. Worth
being accurate about what that proves: `DiagnosticsBundleTests` already asserted
`chainDiagnosticsJson.count == 24`, so the drop would have failed the suite the
moment it ran. It compiled and I saw green only because I had run
`xcodebuild build` and not yet `xcodebuild test`. The existing net held; what it
would not have said is *which* two chains were missing, which is the gap the new
test fills.

*Not done, and it is the reason the switch survives:* the per-chain state is
still 163 stored properties on `WalletChainDiagnosticsState`, forwarded by 707
lines of pure pass-through in `DiagnosticsStore.swift`. Keying those by chain
deletes both and is the largest single win left in Stage 3 — roughly 1,100
lines — but it moves ~650 call sites, most of them in `views/`, and there are
no UI tests behind it. It should be its own pass, not a tail-end of this one.

**`EVMChainContext` stopped being a 15-case copy of a 23-row registry table.**

*Was:* a Swift enum with one case per EVM chain and five switches over it —
`displayName`, `tokenTrackingChain`, `expectedChainID`, the derivation path and
`isEthereumFamily`. It had cases for 15 of the 23 EVM mainnets, and
`isEVMChain` was defined as "this switch returned non-nil". So **Sei, Celo,
Cronos, opBNB, zkSync Era, Sonic, Berachain, Unichain, Ink and X Layer were not
EVM chains as far as the app was concerned**, and every EVM path skipped them
without saying so. `Chain::evm_chain_id` has had all 23 the whole time.

*Now:* a struct built from `core_evm_chain_context`, with `isEVMChain` asking
`coreIsEvmChain` directly. The named statics (`EVMChainContext.arbitrum`, …) are
kept so the call sites in `views/` read the same. Adding an EVM chain is a
registry edit.

*One deliberate non-choice:* the statics resolve through core and fall back to a
context with chain id `0` rather than force-unwrapping. A wrong chain id fails
the pre-signing check loudly; a `fatalError` in a static initialiser would take
the app down at launch. That is the same trap that made resolving a derivation
path crash every testnet.

**The import flow's three per-chain expansions collapsed onto one table.**

*Was:* `AppState+ReceiveFlow` expanded every chain four separate times — 24
`typed<Chain>Address` locals, 23 `<chain>AddressEntries` locals, a 25-optional
block unpacked from the derived map, and a 25-row slot map repacking it — plus
a 22-row watch-only copy and a hardcoded 23-name EVM set.

*Now:* `draft.watchOnlyInputsByChainName` is the one table, as `ImportDraft`
always intended; `addressByChainName` is filled by whichever branch produced
the addresses and handed to `WalletImportAddresses.slotMap` once. The EVM set
is `coreIsEvmChain`. The file is 872 → 708 lines.

*Worth stating plainly:* the raw count of chain-name literals in this file went
from 201 to 109, not to zero, and part of the remainder is new — `typed("Sui")`
appears where `typedSuiAddress` did. That is not cheating the metric so much as
showing its limit: a name used to *look up one table* is not the same debt as a
name that *declares its own variable*, but both count the same. Adding a chain
to this file is now a `ImportDraft` field plus nothing here.

**The import flow's own address validation is gone; core's is the only one.**

*Was:* `AppState+ReceiveFlow` held three more gates on top of core's — a
16-row watch-only validation table, a Bitcoin address-or-xpub guard, and a
seven-chain EVM guard — each restating per-chain address formats the registry
already holds. It also re-stated `ImportDraft.watchOnlyInputsByChainName` as a
23-row slot map, normalising each chain by hand on the way.

*Now:* all four are deleted. `draft.watchOnlyEntriesBySlot` is passed straight
through, and core validates and normalises.

*Two behaviour changes inside this, both deliberate:*

- **A bad line no longer fails the whole import.** Swift refused if any entry
  in a chain's list was invalid; core keeps the valid ones and returns the rest
  in `rejectedAddresses`. An import with nothing left still fails.
- **A malformed Bitcoin xpub is refused.** Swift checked the `xpub`/`ypub`/
  `zpub` prefix; deleting that guard without replacing it would have let any
  string be stored as an account xpub. The rule moved to core and gained the
  testnet prefixes (`tpub`/`upub`/`vpub`), which the Swift check did not have —
  so a testnet watch-xpub import works where it silently did not before.

*Two things this slice found, both now pinned by tests:*

- **Validating watch-only addresses had broken testnet watch imports.** Added
  in the previous slice, it judged every address in the `bitcoin` slot as
  mainnet — but `ImportDraft` is keyed by mainnet display name, so a testnet
  address arrives in that slot too. `ImportNetworks` now carries the mode.
- **The two inputs need different networks, and conflating them broke a
  signing import.** `resolved_addresses` holds what the caller *derived*, and
  derivation runs against the mainnet chain whatever mode is selected — the
  testnet address is re-derived for display. Judging that map by the selected
  mode dropped every address on a testnet import;
  `testImportingBitcoinWalletPersistsDerivedAddressOnTestnet4` caught it.
  Mainnet for the derived map, the selected mode for the typed one.

**`spectra` is not a REPL.**

*Was:* an interactive shell — 24 prompt-driven commands, `dialoguer` pickers,
and a `main()` that ignored argv entirely. Nothing could drive it but a person
at a keyboard, so rule 1's "if `spectra` cannot drive it" could never actually
be tested.

*Now:* a clap subcommand tree. `--json` on every command, `--data-dir` for a
scratch store, and exit codes that separate *core said no* (3) from *something
broke* (1) and *you asked wrongly* (2). Destructive actions — delete, send,
printing a seed — take `--yes` rather than a typed confirmation. Secrets arrive
by file, environment or interactive prompt, never as an argument: arguments are
visible in `ps` and land in shell history.



With state and decisions in core, most of the 19,766 lines at the root of
`swift/` are either dead or belong in Rust:

- `StoreHistoryRefresh`, `StorePersistenceNormalization`,
  `StoreDiagnosticsExport`, `DashboardStore` — move to core.
- `Store+Formatting` is the exception, and the earlier draft of this list was
  wrong to include it wholesale. Which decimals an asset shows and what
  currency a value is in are domain; turning a number into text with
  `NumberFormatter` is locale-aware platform work that Rust would only
  reimplement worse. Move the rules, leave the rendering.

Done so far:

- **Pinned dashboard assets → `AppSettings`.** They were a Swift array kept in
  `cachedPinnedDashboardAssetSymbols`, persisted through a second settings
  blob, and seeded at launch from a UserDefaults key that had no writer left.
  Core normalises (upper-case, de-duplicate, keep pin order) and stores them.
- **Tracked tokens → `CoreAppState.token_preferences`.** The clamping rule —
  a token cannot display more decimal places than it has — moved with them, so
  it is enforced once rather than at each Swift mutation site. Loading them
  from a separate SQLite blob is gone; they arrive with the rest of the state.

  Adding those two fields broke every launch on an existing database
  ("missing field `pinnedDashboardAssetSymbols`") because the stored JSON
  predates them. `#[serde(default)]` fixes it, and
  `settings_forward_compatibility` now pins that down — extending a stored
  struct must not make what is already written unreadable.

- **Transaction merge strategy → `registry::Chain`.** Eighteen Swift wrappers
  (`upsertBitcoinTransactions`, `upsertSolanaTransactions`, …) each named a
  chain and the merge strategy to use with it. That is a per-chain fact stated
  eighteen times in the shell — rule 2, and precisely the thing that makes
  adding a chain a Swift change. They collapse into one
  `upsertTransactions(_:chainName:)`; `TransactionCommand::Merge` no longer
  takes `strategy` or `include_symbol_in_identity` at all, because core reads
  both off the chain.

  `refreshNormalizedChainTransactions` also took an `upsert` closure *and* a
  `chainName`, so callers had to supply a matching pair. The closure is gone;
  the helper uses the chain name it already has.

  Writing the mapping by hand reproduced the original bug in miniature:
  `zcash-testnet` fell through to account-based because only `Chain::Zcash` was
  listed. It now resolves through `mainnet_counterpart()`, and
  `a_testnet_merges_the_same_way_as_its_mainnet` fails if any testnet ever
  diverges again.

- **EVM native assets → `registry::Chain::coin_symbol()`.** `supportedEVMToken`
  excluded a chain's native asset with six hand-written chain/symbol pairs
  (`"Ethereum"`/`"ETH"`, `"Avalanche"`/`"AVAX"`, …) against 23 EVM mainnets.
  It asks the registry now. Worth being precise: this was *not* a live bug —
  an unlisted native asset fell through to a token lookup that also missed, so
  the answer came out the same. It was a partial copy of a complete table,
  which is how the next one becomes a bug.

- **Send gating → `registry::Chain::send_rule()`.** `can_send_holding` matched
  on chain-name strings in core. Now a `SendRule` on the chain, resolved
  through `mainnet_counterpart()` so testnets cannot diverge.

  Moving it exposed an asymmetry worth a decision rather than a silent fix:
  Ethereum, BNB Chain and Avalanche require a non-native asset to be a
  supported token before offering send; every other EVM chain falls through to
  no restriction. So an unsupported token on Arbitrum or Base would be offered
  for sending and fail at submit, where the same token on Ethereum would not be
  offered. Behaviour was preserved exactly — `send_rule_asymmetry_across_evm_chains`
  pins the current list and fails if it changes, so the question stays visible
  instead of being decided by accident.

- **`wallet_derived_state()` — the indirection is gone.**
  `core_plan_store_derived_state` and `core_plan_transfer_availability`
  returned holding *indices* that Swift resolved back into `Coin`s against its
  own copy of the wallets. Core holds the wallets, so it now resolves them
  itself and returns coins. `rustStoreDerivedStatePlan`,
  `rustTransferAvailabilityPlan` and `resolveHolding` are deleted — 49 lines of
  pure index plumbing — and `_rebuildWalletDerivedStateBody` drops from ~80
  lines of ref-walking to assembling a cache from what core already resolved.

  Three inputs still cross, and they are the right three: which wallets have
  signing material or a private key (the platform keystore) and which network
  mode the user is on. `NetworkModes` carries the last one and ports
  `displayNetworkName` exactly — the network mode changes an asset's identity
  key, so a testnet BTC groups separately from a mainnet one and is not quoted.

  `can_send_coin` reads tracked tokens from `CoreAppState.token_preferences`,
  which is only possible because they moved into core earlier this stage.

- **`resolvedAddress(for:chainName:)`** was a 24-case switch mapping each chain
  to its own accessor, where nineteen of those accessors were the identical
  one-liner with a different slot. It asks core for the derivation chain now.

  Four stay explicit and the reason matters: Bitcoin and Dogecoin choose their
  derivation chain from the selected network mode, Cardano prefers a stored
  address before deriving, and Monero *only* uses a stored address. Collapsing
  all twenty-four — which is what I did first, and which compiled — would have
  made Monero attempt a derivation it has no key for. Compilation does not
  catch this class of change; only reading each implementation does.
  `every_chain_the_switch_named_still_resolves` covers the nineteen.

- **The CLI became the gate it was supposed to be.** Rule 1 has always said the
  CLI is the test — "if `spectra` cannot drive it, it is in the wrong place" —
  but the CLI was a REPL whose `main()` never read argv, so the test could only
  ever be run by hand. Rewritten as a clap front end: 2,802 lines in one
  `main.rs` became 2,083 across ten files, and `scripts/cli-acceptance.sh` runs
  38 assertions against a scratch data directory with no network — 69 today.

  What the rewrite found is the point. The old CLI *bypassed* core on every
  path it was supposed to prove: it assembled `WalletSummary` values itself and
  called `app_state_save`, so it exercised none of the import rules. The
  previous slice had added `validated_addresses_for_cli` — a core function
  existing only so the CLI could check a rule it could not otherwise reach —
  which is the shape of the problem, not a fix for it. It is deleted; `wallet
  watch` now drives `import_wallets`, and doing so immediately surfaced that
  watch-only addresses were never validated at all.

  Also deleted: the CLI's own `chain_rgb` (30 hardcoded RGB pairs against 78
  catalog chains, everything else grey — it reads `chains.toml`'s `color` now),
  `chain_id_for_name`, `chain_native_symbol` and `load_chain_presets`, all
  re-tabulating what `registry::Chain` holds. Its `bip39`, `pbkdf2`, `sha2`,
  `base64` and `getrandom` dependencies went with them: mnemonic generation was
  already core's, and seed sealing now is.

- **Seed sealing policy → `store::wallet_secrets`.** The KDF and its cost, the
  three blobs a sealed wallet consists of, their key names and their encoding
  lived in `cli/src/main.rs`. That is domain policy in a front end. It is core's
  now, with tests covering the cases the inline version had no way to state —
  that two wallets sealed from the same phrase and password differ, that a
  reseal replaces every blob rather than leaving a stale salt, and that a
  corrupt blob is not reported as a wrong password.

  Worth being precise about what was *not* duplicated: iOS seals with a random
  Keychain-held master key, not a password-derived one, and that difference is
  correct — the Keychain provides the protection the CLI has to derive. Both
  sides already shared core's `seed_envelope` cipher. What was misplaced was the
  CLI owning the policy around it.

**Where this stands, and the honest shape of what is left.**

The state and decisions have moved. What remains is bulk: roughly 8,700 lines
of Swift that exist because the shell restates per-chain facts core already
holds. The four heavy files, with how much of each is literally a hardcoded
chain name:

| File | Lines | Lines naming a chain |
|---|---|---|
| `AppState+SendFlow` | 1,566 | 106 |
| `AppState` | 1,252 | 29 |
| `AppState+ReceiveFlow` | 872 | 154 |
| `AppState+DiagnosticsEndpoints` | 859 | 64 |

`AppState+ReceiveFlow` is the densest and the next target. The import flow
expands every chain three separate times — `typed<Chain>Address` locals, then
`resolved<Chain>Address` locals, then the slot map — and `ImportDraft` already
has `watchOnlyInputsByChainName`, a single table proving the shape works. The
three expansions should collapse onto one table.

**The open question about the slot map is now traced and answered.** The
previous note said twenty-three chains kept an unvalidated `resolved<Chain>`
local that outranked the derived value, that Dogecoin / Ethereum / Ethereum
Classic did not, and that "whether the looser path is reachable depends on the
import guards upstream, which is not traced here."

Traced: **it was not reachable.** `res(wants, v)` returns the typed value when
the chain is selected and the field is non-empty, and those fields are only
filled in watch-only mode. `ImportDraft.isWatchOnlyMode` has exactly one
writer — `configureForWatchAddressesImport()` — which calls `reset()` first, so
a signing import always starts with every address field empty. The `didSet` on
the flag only refreshes selection state; nothing else in the app assigns it.

So the fallback was inert, not wrong. What it was, precisely, is a rule whose
safety lived in a different file: had a second writer of `isWatchOnlyMode` ever
appeared — a mode toggle inside the flow, a restored draft — a typed address
would have silently outranked the one derived from the user's seed, and the
wallet would have been stored with an address its key cannot spend. Core's
validation would not have caught it either: a valid address for the wrong key
still parses.

The 23 locals and the `?? ` fallbacks are deleted. A signing import's address
now comes only from derivation, and a private-key import's only from a value
that passed validation. Two chains keep an explicit typed source, and both have
a reason: Bitcoin's account **xpub** is not an address and has no derived
counterpart, and **Monero** is not part of the batch derivation, so what the
user supplied is its only source — guarded by the validity check that already
sat above it.

The three-chain asymmetry disappears with the fallback rather than needing to be
settled: Dogecoin, Ethereum and Ethereum Classic were the chains with *no*
looser path, and now no chain has one.

**One correction to an earlier note in this plan:** `walletDerivedCache` should
*not* be deleted. It is a synchronous projection in front of an async core,
which is exactly the pattern this document asks for — lose it on restart and
nothing breaks. Its *producer* moved into core (`wallet_derived_state`); its
consumers are views, and they are right to read a cache.

**Done when:** the root of `swift/` is a minority of the Swift line count, and
adding a chain requires no Swift change at all.

**Not met.** Measured now, not estimated:

| | Start | Now | Target |
|---|---|---|---|
| `swift/` root vs `views/` | 19,766 vs 11,113 | **15,931 vs 10,729** | inverted |
| Chain-name literals in root | — | **281** | 0 |

Root is 60% of the Swift line count, from 64%. Inverting it means moving
roughly 5,600 more lines — a third of what is left there.

*The literal count needs a caveat.* It greps chain names in string literals,
which catches dispatch (`case "Bitcoin":`) and localized user-facing text
("…while sending on Tron.") alike. The second kind is not the debt this metric
is about — a message naming a chain is correct — so the real dispatch figure is
lower than 281. `AppState+SendFlow`'s densest block turned out to be entirely
of the second kind. Reported as-is because a metric quietly redefined mid-way
is worse than one with a known bias, but a reader should not treat it as pure
signal.

*The enum row has moved twice.* It read 4 and then 5 and now 6, each time
because a sweep found a list scoped to one screen rather than to the app —
`StakingSupportedChain` in the staking tab, then `CoreTokenTrackingChain`'s
three hand-written members in `RegistryModels`. The number to trust is the
literal count below it, which does not depend on anyone having noticed a type.

*How it is counted*, since the figures above this pass were taken with a looser
grep and are not comparable: the display names of all seventy-eight registry
chains, as exact double-quoted literals, over `swift/*.swift` — the names come
from `spectra --json chains --testnets`, so the counter cannot drift from the
registry either.

Where they are, and what shape each is:

| File | Literals | Lines | Shape |
|---|---|---|---|
| `AppState+SendFlow` | 70 | 1,498 | the address-hint table, Ethereum self-tests, Dogecoin rebroadcast |
| `AppState+DiagnosticsEndpoints` | 21 | 628 | Bitcoin's xpub walk, and the Monero protocol probe |
| `AppState+SendExecution` | 25 | 651 | per-chain broadcast arms, each a different payload |
| `AppState+SendPreview` | 12 | 363 | three real per-chain rules: Solana's coin check, Polkadot's seed, Bitcoin's HD |
| `SendPreviewTypes` | 35 | 291 | `SendPreviewStore`'s eighteen per-chain fields |
| `AppState+ReceiveFlow` | 21 | 640 | Bitcoin's xpub forms and the UTXO receive-index path |
| `StoreHistoryRefresh` | 20 | 582 | the Bitcoin and Dogecoin fetches, which are genuinely their own |
| `CoreModels` | 13 | 806 | the five shims with a reader, and the fee-priority map |

Every one is the shape collapsed five times over now: a fact the registry
holds, restated once per chain, usually as a wrapper whose body is one call.
None is hard. There are a lot of them, and each needs its own pass through the
four suites — call it four to six more rounds at the rate these went.

*What is left in the send family is a different shape.* After these passes the
four send files hold 206 of the 530, and the EVM lists are out of all of them.
`SendPreviewStore` is the remaining bulk: eighteen `var <chain>SendPreview`
fields, two seventeen-arm name switches selecting between them, and an
eighteen-line `resetAll`. That is a slot-keyed map wearing a wide record —
except the eighteen fields have about 150 readers across the views, which is
the `CoreModels` trade below: keying the store would rewrite every reader into
something less legible than `store.bitcoinSendPreview`. Recorded as understood
rather than pending; the switches are the part worth removing, and they cannot
go while the fields stay.

*One caveat on the `CoreModels` row.* Those 24 shims are one line each and
serve ~150 readable call sites (`wallet.bitcoinAddress`). Deleting them saves
24 lines and rewrites 150 sites into `address(forChainNamed:)`. That is a
worse trade than it looks on the literal count, and it is why they are still
here — the metric would improve and the code would not.


### Stage C — Rewrite core — **started**

Stages 0-3 moved ownership *into* core without reshaping core. It shows: the
crate is 59,276 lines with **290 exported functions**, which is the plainest
statement that it is still a library of helpers rather than a program. A front
end that has 290 ways in does not have to go through the ten that matter.

Measured, not estimated:

| | Start | Now |
|---|---|---|
| Exported functions and methods | 234 | **180** |
| Largest file in `core/` | `service.rs`, 4,781 lines | `store/tests.rs`, 2,501 |
| `service.rs` | 4,781 lines, 90 functions | **nine modules, largest 1,359** |
| Chain tables | two — `chains.rs` (TOML) and `registry.rs` (enum) | **one** |
| Duplicate module pairs | three | **none** |

The export count went *up* because the 234 was measured before the CLI work
added `spectra`'s commands to the surface; it is not a regression, and the
honest baseline is the 290 above.

**C1 — the skeleton — done.** Merge the two chain tables, split `service.rs`
into the owners it actually has, collapse the duplicate module pairs. Held the
FFI surface still apart from ten dead exports, so no Swift call site moved.

**C2 — the surface.** Most exports are "core computes a value, Swift assembles
it" — those should *disappear*, not be renamed. What survives: `WalletService`
methods, `StateCommand`, and the few genuinely pure calculations. This one moves
every Swift call site, so it runs with the rest of Stage 3 rather than beside it.

**The target was ~60. It is ~150, and here is why the first number was
wrong.**

*What ~60 was.* Counted when free functions stood at 153 and service methods at
94, C2 projected each category collapsing at the rate the first passes managed:
registry lookups 21 → 2, formatting 19 → 3, endpoint catalog 18 → 1,
send/risk/preview 20 → 0, "the rest" 21 → 4. Total ~60. The reasoning was
sound for the shape those categories had then — most of their members were the
same question asked once per chain.

*What happened instead.* Those members are gone. What is left is not more of
the same: it is one function per distinct capability, and the distribution has
gone flat — nine exports in the largest file, one to four in most. Measured
across all 99 free exports, **41 have no merge partner at all**:
`tor_start`/`stop`/`status`/`activate_custom_proxy`, `http_request` and
`http_post_json`, the password-verifier and seed-envelope pairs,
`generate_mnemonic` / `validate_mnemonic` / `bip39_wordlist`, four validators
over four different inputs, two price merges. Of the other 58, most are also
distinct — `formatting.rs`'s six are six different questions;
`diagnostics/registry.rs`'s four are record / summary / forget / clear, which
is the CRUD shape C2 asked for and already met.

*The row that breaks it is "the rest: 21 → 4", now 41 → 4.* Reaching that means
deleting thirty-seven distinct operations. Not merging them — deleting them, or
folding them behind one enum-dispatched entry point, **which is the shape C2
itself rejects two paragraphs later**: "it trades static typing at the boundary
for a hundred-arm union, and UniFFI enums are worse to hold than UniFFI
methods."

*The revised arithmetic, measured rather than projected:*

| | now | reachable | what carries it |
|---|---|---|---|
| Free functions | 99 | ~85 | `is_valid_send_address` + `normalized_send_address` are one call; `core_endpoint_str_id` and `core_resolve_chain_id` are identity columns; `app_core_endpoints_for_ids` and `app_core_endpoint_records_for_chain` fold into the chain-endpoints record if it carries ids and raw records; a formatting pair and the diagnostics bundle pair |
| `WalletService` methods | 81 | ~63 | keypool's two scoped deletes become one `scope` enum, as `reset_history` already did; `replace_all_history_records` is delete-then-upsert; the status-poll surface has one or two left |
| — network fetches | 22 | 22 | **already below C2's 24**; each is a distinct chain read |
| **Total** | **180** | **~150** | |

*Why ~150 and not a rounder, braver number.* Because that is what the
candidates add up to when they are counted one by one rather than extrapolated,
and C2's own test is the reason to care: **"A target the shape cannot reach is
worse than no target, because it never reads as met."** ~60 failed that test
against the design this project deliberately chose. A number reached by
counting can be argued with; one reached by projecting a rate cannot be
checked until it has already been missed.

*What would make ~60 right.* Only a different boundary: one `call(Command) ->
Response` with a wide enum on each side. That remains a real design, and it
remains rejected — for the reason C2 gave and has not stopped being true.

*How it is counted*, so "met" is checkable: free functions in the generated
bindings excluding `FfiConverter*`, plus the methods on `WalletServiceProtocol`.
That is what `swift/generated/spectra_core.swift` shows and what the numbers in
this document are.

**On "765 exported functions", which this document reported last pass.** That
number came from `grep -c '^public func' swift/generated/`, and 552 of the 765
were `FfiConverterTypeX_lift` / `_lower` pairs — two per *record*, generated
whether or not anything calls them. They measure how many types cross the
boundary, not how many ways in there are. The callable surface was 213 then and
is **290** now, counted from the Rust side and cross-checked against the
generated `WalletServiceProtocol` and the free functions. Converter pairs are
worth tracking separately (544 today) because a record that stops crossing
removes two of them, but adding them to the API count made a 213-function
surface read as three and a half times bigger than it is.

Done so far:

- **The preview debounce: one helper for three chains, 304 → 281.**

  Written up above — the bookkeeping was inlined three times and wrong in two
  ways. `AppState+SendPreview`: 35 literals → 12.

  *`AppState+SendExecution` has the same shape and is left alone.* Seven send
  arms each write `guard !sendingChains.contains(X)`, then `insert(X)` and
  `defer remove(X)` forty lines later, with validation in the gap — twenty-one
  literals. Unlike the preview path there is no bug: no arm removes the flag on
  an early exit. Collapsing it means restructuring seven signing paths so the
  network call moves inside a closure, which is a real change to the funds path
  bought with a metric. Assessed and declined, which is a different thing from
  not looked at.

- **The diagnostics descriptors stopped spelling their own key: 46 → 21, and
  329 → 304 overall.**

  The rows are keyed by `Chain` and every closure in them wrote the chain name
  out again — `chainName: "Tron"` inside the `.tron` entry, twice or three times
  per row. The chain id and the two key paths went a pass earlier; this is the
  last column. The closures take `(AppState, Chain)` now, so a row that wants
  its own name says `chain.displayName`.

  *Two byte-identical probes became one.*
  `runEthereumEndpointReachabilityDiagnostics` and
  `runBNBEndpointReachabilityDiagnostics` differed in a chain name given three
  times each and in which explorer list they appended, which is two arguments.

  *And the dispatchers were converting a `Chain` to its name and back.*
  `Chain(displayName: chain.displayName)?.isEVM ?? false` — three of them, on a
  value that is already a `Chain`. It is `chain.isEVM`.

  What is left in the file is genuinely per-chain: Bitcoin's history diagnostics
  walk an xpub, and Monero, NEAR and Polkadot each probe a different protocol.
  Those name their chain because they *are* that chain's.

- **The address-field copy: two fourteen-arm switches saying the same fourteen
  things in a different tense. 342 → 329, and the line count went *up*.**

  `addressBookAddressValidationMessage` had one switch for "the field is empty,
  here is what a valid address looks like" and a second for "that does not
  parse, here is what to fix" — the same fourteen chains, twice, in two tenses.
  One table keyed by chain now, with the EVM family and the Sui/Aptos pair
  staying as arms because their strings take `%@`.

  *This one cost 29 lines.* A dictionary of string pairs is more verbose than
  the switch it replaces, and the metric that matters here is the duplication —
  each sentence appears once instead of its pair being spread across two
  functions' worth of arms — not the line count. Recorded rather than hidden:
  the root-vs-views number moved the wrong way for a change that is still
  right, which is the same trade the settings mirror made and the opposite of
  the dead-code sweep's.

  *And it makes a content gap visible in one place instead of two.* Ten
  mainnets have no copy and fall back to "Enter an address for the selected
  chain." — see the open item below. Filling that needs sentences in four
  locales, which is authoring rather than migration, so the table names the gap
  and leaves it.

- **And an eighth, twenty-two rows long, deciding how amounts are formatted.**

  `SUPPORTED_DECIMAL_CHAINS` — chain name to native decimals — beside
  `chains.toml`'s `native_decimals`. Written up above as a behaviour change,
  because unlike the backends table this one was *short*, and the gap showed on
  screen.

  Two copies found in one pass, in the same file as each other's neighbours, is
  the argument for the metric this document added a row for: **hand-written
  chain tables in `core/` beside `chains.toml`**. It was 1 at the start of the
  pass and 2 by the end of looking; it is 0 now. The way to keep it there is a
  test that walks `list_all_chains()`, which is what both replacements have.

- **A seventh copy of the chain list — 78 rows, in `app_core.rs`, and every row
  said the same thing.**

  `chain_backends()` was a hand-written table beside `chains.toml`: seventy-eight
  `live("Bitcoin", &["BTC"])` rows carrying an `integration_state` and four
  `supports_*` booleans. **Every row was `live(...)`**, so the enum's `Planned`
  variant had no instance and `integration_state` selected nothing — the same
  shape as `supports_diagnostics` two passes ago.

  *And the names were the registry's.* Diffed before deleting: the 78 backend
  names and the 78 `chains.toml` names are the same set, exactly. So the table
  was a second spelling of the chain list that had to be edited in step with the
  first, with no test that could say so.

  What read it:

  - `AppEndpointDirectory.liveChainNames` — the list filtered by
    `integrationState == .live`, which is every row. It is `Chain.all` now.
  - `AppEndpointDirectory.backend(for:)` — **no callers**.
  - `supportsBalanceRefresh/ReceiveAddress/Send(for:)` — three accessors over a
    field that is always `true`. One had a caller:
    `ImportDraft.unsupportedSelectedChainNames`, which filtered the selection by
    `!supportsBalanceRefresh` and could therefore only ever return `[]` — and
    *it* had no callers either.
  - `WalletService::wallet_derived_state` in core, where
    `backend.is_some_and(|b| b.supports_send)`,
    `backend.is_some_and(|b| b.supports_receive_address)` and
    `live_chains.contains(…)` were three spellings of "the registry knows this
    chain", which is what they are now.

  Gone: the table, `AppCoreChainBackend`, `AppCoreChainIntegrationState`,
  `app_core_chain_backends`, `live_chain_names`, Swift's `ChainBackendRecord`,
  `ChainIntegrationState`, `allBackends`, `backend(for:)`, the three `supports_*`
  accessors and the always-empty `unsupportedSelectedChainNames`. 189 → 188 on
  the export count, which badly understates it: a 78-row table and two records
  went with the one export.

- **Three questions about one string became one: 191 → 189.**

  The degraded-detail trio, written up above. What makes it worth a note beyond
  the count is that *asking separately is what let the callers disagree* — three
  independent questions have an order, and two call sites picked different ones.
  A classification has no order.

- **The refresh planners: three requests, three targets and a dead field.**

  `core_evm_refresh_targets`, `core_dogecoin_refresh_targets` and
  `core_normalized_refresh_targets` each took their own `…RefreshWalletInput`
  and returned their own `…RefreshWalletTarget`. Read side by side the three
  planners are one function: on the chain, in the allowed set, and having
  somewhere to look. The only real difference is `address: Option<String>`
  versus `addresses: Vec<String>` — one address and several of them.

  *And the Dogecoin one wrote the chain name into core.*
  `filter(|w| w.selected_chain == "Dogecoin")`, for a caller that only ever
  called it for Dogecoin. The chain is the request's.

  *`index` crossed the boundary twice and was never read.* Every input carried
  the caller's position in its own array so the target could carry it back, and
  all three call sites map the results by `wallet_id` instead. Three records in
  and three out, each a `u64` heavier than it needed to be.

  One `RefreshWalletInput`, one `RefreshTargetsRequest`, one
  `RefreshWalletTarget`, and `core_refresh_targets` for the two non-EVM
  families. EVM keeps its own export because its return is a *plan* — the
  grouping by normalized address is real work, not a shape difference. Six
  records became three, which is six fewer converter pairs; the generated
  bindings are down to 426 lifts.

  *A mistake worth recording.* Rewriting the tests module by truncating the file
  at its start silently deleted the `// ── FFI surface ──` block that followed
  it — the exports themselves. `cargo build` passed, because nothing in Rust
  calls them; `cargo test` passed for the same reason. Only the Swift build
  failed, with "cannot find `coreRefreshTargets` in scope". An export with no
  Rust caller is invisible to two of the three gates, which is the same lesson
  as the staking runtime one file over.

- **196 → 192, and the token normalizer turned out to have a *third* copy.**

  *In `core/src/store/mod.rs`*, `normalize_tracked_token_identifier` — a
  twelve-name EVM arm, then Aptos, Sui, TON and a lowercase default — with a doc
  comment that said outright it "mirrors the Swift
  `normalizedTrackedTokenIdentifier` dispatch". Which was itself the copy of
  `normalize_dashboard_contract_address` merged a pass earlier. Three
  transcriptions of one table, across two languages, and the doc comment naming
  one of the others is what makes it findable: **a comment that says "mirrors X"
  is a duplicate declaring itself.** All three read one function now, and the
  two chain-specific normalizers lost their export attributes with the last
  Swift caller.

  *Three endpoint validators were two identical functions and a list.*
  `ethereum_rpc_endpoint_validation_error` and
  `monero_backend_base_url_validation_error` were byte-identical but for the
  message; `bitcoin_esplora_endpoints_validation_error` ran the same check over
  a parsed list. One `endpoint_validation_error(field, raw)` over an
  `EndpointField` that decides both how the value is parsed and which message
  names it. The test asserts the last part directly — same input, different
  field, different message — which the three separate functions could not say.

- **198 → 196, and a three-state that was written as two booleans.**

  `record_status_poll_success(id, resolved_status_confirmed,
  resolved_status_pending, reported_confirmations)` beside
  `record_status_poll_failure(id)`. The two booleans encode three answers —
  confirmed, pending, neither — in a shape where "confirmed **and** pending"
  type-checks and means nothing, and the caller computed both from one
  `TransactionStatus` it already had.

  One `record_status_poll(transaction_id, outcome)` over a four-variant
  `StatusPollOutcome`: `Confirmed { confirmations }`, `Pending`, `Unresolved`,
  `Failed`. The fourth is the one the pair could not say — a poll that failed is
  not a verdict about the transaction, and it was only distinguishable before by
  which of the two methods you called.

  *And `clear_status_trackers()` was `retain_status_trackers([])`*, which one
  call site already spelled that way.

- **210 → 198: one writer for five shapes, and nine wrappers nothing called.**

  *The diagnostics writers were five calls for one question.*
  `diagnostics_record_utxo/evm/simple/tron/solana` each took a different record
  type, so every Swift call site picked between them by knowing which shape its
  chain reports — a fact `Chain::diagnostics_shape` already holds. One
  `diagnostics_record(chain_name, wallet_id, entry)` with a five-variant
  `HistoryDiagnosticsEntry`; the entry carries its own shape. The five writers
  lost the attribute rather than the body, because the maps they write are
  genuinely five typed maps.

  *`clear_all_history_records()` was `replace_all_history_records([])`.*
  `history_replace_all` deletes every row before inserting, so the two ran the
  same `DELETE` and differed only in what followed it.

  *And two pagination setters were two writes to one cursor.* `set_history_page`
  and `set_history_exhausted` were called together, in that order, at the one
  call site that used either — the page just fetched and whether it came back
  short. Two writes to a three-field record is two chances for a reader to see
  half an update, the same shape as the diagnostics read-modify-write a pass
  earlier. One `set_history_page(page, is_exhausted)`.

  *Nine bridge wrappers had no caller at all, and neither did what they fronted.*
  `refreshEndpoints`, `rustBip39Wordlist`, `fetchSolanaBalance`,
  `fetchNearBalance`, `fetchErc20Balance`, `transactionExplorerBaseURL`,
  `broadcastProviderOptions`, `allTokens`, `evmChainContextTag` — each a
  declaration with nothing referencing it, in front of a core export reachable
  only through it. Found by sweeping every `func` in the bridge files for
  references outside its own file, which is worth keeping as a check: this is
  the fourth time the pattern has turned up, after the token normalizers, the
  keypool trio and `advance_history_page`.

  *One of the nine was a false positive and the tests caught it.*
  `CachedCoreHelpers.allTokens` was dead, but `list_all_builtin_tokens` — the
  export behind it — has callers of its own in `StaticContentCatalog`. Removing
  the wrapper is right; removing the export was not, and it is restored. A dead
  wrapper does not imply a dead export, which is the thing to check next time
  rather than the thing to assume.

- **The export surface: 215 → 210, and every one of the five was a duplicate
  rather than a deletion.**

  There are no dead exports left — a sweep for exports with zero Swift callers
  returns only UniFFI's own init shim. What is left comes from collapsing, and
  this pass took the cheap ones:

  - `formatting_asset_minimum_visible_amount` was `10^-n`, crossed the boundary
    and memoized on the Swift side. A rule core owns is worth a call; an
    exponent is not.
  - `bip39_english_wordlist` was `bip39_wordlist("en")` with the argument bound,
    beside a function whose default arm is English.
  - `aptos_package_identifier`, `canonical_aptos_hex_address` and
    `normalize_sui_package_component` each had a Swift forwarder and **the
    forwarder had no caller** — three exports reachable only from a shim nothing
    called. Two are still used inside `tokens.rs` and lost the attribute; the
    third had no Rust caller either and is deleted with its test.
  - `normalize_dashboard_contract_address` and
    `normalizedTrackedTokenIdentifier` were the same function in two languages,
    and disagreed about TON — written up above.

  *What the arithmetic looks like from here.* 210, target ~60. The remaining
  groups are `diagnostics_record_*` (five writers, one per record shape, which
  is one tagged enum), the network fetches on `WalletService` (28, target ~24),
  the history store (17, target 4) and the per-chain preview fetchers, which
  C2's own note says are *not* one call. This is many slices, not one.

- **An audit pass: the tests mostly earn their keep, and `core::resources` was
  a module that could only fail.**

  Prompted by "are all those tests worth it". Counted honestly: **five of 458
  were deletable**, and the answer to the question is that the suites are lean
  rather than bloated. `all_chain_count` asserted `Chain::all().count() == 78`
  — a pure change detector that teaches people to bump a number;
  `testnet_counts_match_total` kept its real invariant and lost its literal 32;
  `ethereum_self_tests_pass` and `self_tests_cover_all_chains` were both strict
  subsets of `every_self_test_passes`. What is left is known vectors, "these are
  all of them" walks over the registry, and round trips through the store —
  which is the shape this document has been asking for.

  *The other two deletions were a module.* `core::resources` held two **empty**
  static tables, two lookups over them, an exported `core_static_resource_json`
  that could therefore only ever return an error, and two tests asserting that a
  lookup returns `None` — which passed because there was nothing to look up.
  `StaticContentCatalog` called it first on every content load, swallowed the
  throw with `try?`, and fell through to the bundle. Module, export, record and
  both Swift call sites deleted.

  *Two catalog flags went with them*, written up in "known open items" below:
  one selected nothing, and the other hid Bitcoin SV from a screen whose catalog
  has three endpoint records for it.

  *And a twenty-four arm receive dispatch.* `receiveAddress()` switched over
  `core_receive_address_resolver`'s cases and twenty of the twenty-four arms were
  `resolved<Chain>Address(for:)` — `resolvedChainAddress(for:chain:)` with the
  chain the case already names. Four arms genuinely differ; the rest is
  `resolvedAddress(for:chainName:)` through `mainnetCounterpart`, which is what
  core dispatches on. Six more per-chain resolvers had no reader afterwards.

  *The comment sweep found nothing.* Two passes looking for comments that
  restate the line below them, and for `MARK:` banners that repeat the
  declaration under them, returned zero hits in `swift/` and `core/`. The
  convention here is already why-not-what. What the sweep *did* find is the
  other kind of stale comment — five citations of symbols that no longer exist,
  including a test renamed two passes ago — and those are fixed.

- **A sixth Swift copy of the chain list, and the `RawRepresentable` trap under
  it. 420 literals → 342.**

  `CoreTokenTrackingChain` is core's enum. Swift hand-wrote its `rawValue` (18
  arms), its `init?(rawValue:)` (18 arms) and its `allCases` (18 entries), and
  `StaticContentCatalog.tokenTrackingChainFor` added a fourth (18 arms plus a
  fallback repeating the same lookup). The mapping they restate is
  `CoreTokenTrackingChain::chain_name` and `from_chain_name`, whose own doc
  comment already says it "had four copies … and `tokenTrackingChainFor` in
  Swift" — Rust collapsed its own copies and left the Swift ones standing,
  because there was no way to ask. There is now: `token_tracking_chain` is a
  column of `core_chain_identities`.

  `"bnb"` was an arm of its own in two of the four. It is BNB Chain's catalog
  id, so `Chain(id:)` answers it without the literal.

  *The interesting part is why the first attempt crashed.* Deriving `rawValue`
  from a `[CoreTokenTrackingChain: Chain]` table trapped the app in
  `_dispatch_once_wait` before the first frame. The extension declared
  `RawRepresentable`, whose default `==` and `hash(into:)` route through
  `rawValue` — and those defaults win over the conformance UniFFI generates. So
  building a dictionary keyed by this enum hashed its keys, which called
  `rawValue`, which waited on the `dispatch_once` it was already inside. The
  conformance is dropped; `rawValue` stays a plain property, every call site is
  unchanged, and hashing is the generated one again. Worth naming because the
  hazard is invisible at the declaration and only fires once `rawValue` stops
  being self-contained.

  `RegistryModels.swift`: 36 literals → 0. `StoreLifecycleReset.swift`: 42 → 0
  (see the behaviour change above). Found by the crash report rather than by a
  test, which is the honest way round to say it: the iOS suite catches a wrong
  answer, not a launch trap in a static initializer.

- **Nineteen of the twenty-four `wallet.<chain>Address` shims are gone, and the
  trade this document declined turned out to be a different trade. 499 → 420.**

  The shims were kept on the grounds that they serve "~150 readable call sites",
  and rewriting those into `address(forChainNamed:)` would improve a metric and
  not the code. That was right about reads and wrong about the population: most
  of those call sites were not reading a wallet's Bitcoin address, they were
  **switching between the shims by chain name** to find out which one to read.
  Six such switches:

  - `ReceiveFlowViews.walletStaticAddress` — twenty-four arms
  - `AppState+ReceiveFlow`'s `liveResolvers` — eighteen `(name, resolver)` pairs
    scanned linearly, where every resolver was `resolvedAddress(for:chainName:)`
  - `AppState+SendFlow.knownUTXOAddresses` — five arms, taken last pass
  - `AppState+ImportLifecycle.knownOwnedAddresses` — seventeen appends
  - `WalletFlowViews`' detail row — sixteen, then `.first`
  - `Platform.makeAddressSnapshots` — eighteen pairs

  Each is `address(forChainNamed:)` with the name it already had, and four of
  them were short — see the behaviour change above. With the dispatch gone,
  nineteen shims had no reader at all. The five that remain have a reason:
  Bitcoin falls back to its account xpub, Dogecoin has a watch address, Ethereum
  backs the EVM family, and Cardano and Monero prefer a stored address to a
  derived one.

  *And two more tables were their own keys.* `ChainAddressDescriptor` carried a
  `KeyPath` to the shim and a validation-kind string beside the `Chain` it is
  keyed by — both derivable, so nineteen rows lost two columns each and kept the
  three flags that vary. `ImportedWallet`'s convenience initializer took
  twenty-four `<chain>Address:` parameters and folded them through a
  twenty-four row table; between them they served two call sites, both tests,
  both passing one address. It takes `addresses:` now.

  `CoreModels.swift`: 56 literals → 13. `AppState+ReceiveFlow`: 45 → 21.
  `AppState+AddressResolution`: 6. Root lines under 16,000 for the first time.

- **The history paging and the UTXO activity probe: three more lists, two of
  them wrong. 530 literals → 499.**

  Both behaviour changes are above; the shape is the same one this document has
  now recorded a dozen times, and both cases had the answer already in hand.
  `loadMoreOnChainHistory` computed `eligibleWalletIDs` — the wallets that can
  page — and then iterated chain names instead of asking those wallets what
  chain they are on. `hasUTXOOnChainActivity`'s three arms are
  `supports_deep_utxo_discovery`'s membership, written out and disagreeing about
  what "activity" means.

  *And five arms picked between five shims.* `knownUTXOAddresses` switched on
  the chain name to choose between `wallet.bitcoinAddress`,
  `.bitcoinCashAddress`, `.bitcoinSvAddress`, `.litecoinAddress` and
  `.dogecoinAddress` — which is `wallet.address(forChainNamed:)`, keyed on the
  registry's address slot. This is the one place the `CoreModels` shims were
  being *dispatched between* rather than read directly, which is what made it
  worth removing where the ~150 plain reads are not.

  `StoreHistoryRefresh.swift`: 41 literals → **20**. `AppState+SendFlow.swift`:
  93 → 83.

- **Which preview shape a chain uses became a registry fact, and
  `AppState+SendRouting` went to zero chain names.**

  Swift held `[String: SimpleChain]` — eleven display names mapped to a core
  enum — and passed the result back across the boundary beside the chain id core
  derives it from. `fetch_simple_chain_send_preview_typed(chainId:address:chain:)`
  is the `endpoint_role_mask` shape one more time: a round trip whose whole
  output is the next call's input, except here the round trip went through a
  table Swift maintained.

  `Chain::simple_preview_chain` answers it, the method derives it, and
  `SimpleChain` **stopped crossing the FFI** — it lost `uniffi::Enum` and is now
  internal to `preview_decode` and the registry. Through `mainnet_counterpart`,
  because the shape is what decoding needs and a testnet decodes like its
  mainnet; which network is reached is the chain id's business.

  *And the dispatch was eleven ways of saying one thing.*
  `case .solana: await refreshSendPreview(forChainNamed: "Solana")`, eleven
  times, in a switch whose subject core had already routed. It is
  `case .some: refreshSendPreview(forChainNamed: selectedSendCoin.chainName)` —
  the chain the coin is on, which is what all eleven said.

  *`resetInactiveSendPreviews` was `SendPreviewStore`'s field list written a
  second time and `preparingChains`' contents a third.* Fifty-four lines of
  `if activePreview != .x { store.xSendPreview = nil; preparingChains.remove("X") }`,
  reasoning in preview *kinds* about a store that keys on chains — so the EVM
  family's shared slot had to be got right at every one of them. It is three
  lines now: keep the active slot's preview, reset the rest, intersect
  `preparingChains` with the slot. `SendPreviewStore.previewSlot(forChainNamed:)`
  names the rule `apply` and `taggedPreview` were already dispatching on.

  `AppState+SendRouting.swift`: 135 → 78 lines, **25 chain-name literals → 0**.

  *Two tests, in both directions.*
  `every_shared_path_routing_kind_has_a_preview_shape` walks every mainnet and
  asserts that a routing kind outside the seven with a preview path of their own
  is a chain the registry has a shape for — which is exactly what the collapsed
  `case .some` arm assumes.
  `every_preview_shape_belongs_to_a_chain_that_routes_there` is the inverse, so a
  shape nothing routes to fails rather than sitting unreachable.

- **The diagnostics descriptors stopped passing each row its own key: 92
  literals → 46.**

  `chainDiagDescriptors` is keyed by `Chain`, and every value restated that key
  four to six times. A Tron row read `chainId: Chain.tron.id`,
  `isRunningKP: \.[historyRunFor: "Tron"].isRunning`, `chainName: "Tron"`,
  `tsKP: \.[historyRunFor: "Tron"].lastUpdatedAt`, and for its endpoints
  `isCheckingKP` / `checks:` / `resultsKP` / `tsKP` on
  `endpointHealthFor: "Tron"` — eight arguments, all of them the dictionary key
  the dispatcher used to find the row.

  Every one is derivable from the chain name the call already carries:
  `Chain(displayName:)?.id` for the id, and the two subscripts for the four key
  paths. The helpers take the name and build them. What is left in a row is
  what actually differs between chains — how the address resolves, what record
  shape the chain reports, and which function records it.

  *The same shape one layer down.* `runAddressHistoryDiagnostics*` took an
  `isRunningKP` and a `markUpdated` closure; both were the chain's own row.
  `runSimpleEndpointReachabilityDiagnostics` and
  `runLabeledEVMEndpointDiagnostics` took `setResults` and `markUpdated` as two
  closures and called them one after the other at every call site — a pair, so
  they are one `publish`.

  *And `withEndpointCheck` now owns the write-back.* It took a key path to a
  `Bool`; it takes the chain and hands the probe a `publish` that stores the
  rows and stamps the time. That is what took six probe functions from naming
  their chain four to eight times each down to once or twice — and it preserves
  the one difference that mattered: Bitcoin calls `publish` after every
  endpoint, so its rows appear as they arrive, which is a property of that
  probe rather than something to normalise away.

  *One function was a copy under another name.*
  `runDogecoinEndpointReachabilityDiagnostics` in `AppState+SendFlow` was
  `runSimpleEndpointDiagnostics` with `"Dogecoin"` written in four places. The
  descriptor row calls the shared one.

  `AppState+DiagnosticsEndpoints.swift`: 742 → 736 lines but 92 → 46 literals,
  which is the honest shape of this one — it removed arguments, not lines.

- **The send family: six chain lists out of `AppState+SendFlow`, 171 literals
  → 99, and `mainnet_counterpart` became a column.**

  The behaviour half is above. What belongs here is that four of the six lists
  were the *same* list — the EVM family, written out at thirteen names, four
  times, in one file — and the fix is `Chain.isEVM` each time. `EVMChainContext`
  was deleted for exactly this a pass ago and the file kept four copies of the
  set it had held.

  *Two more were copies of an answer core already gives.* The simple-chain risk
  arm named Litecoin, Dogecoin, Solana, XRP Ledger, Monero, Sui and Aptos, which
  is the seven `core_simple_chain_risk_probe_config` matches — and it already
  answers `nil` for a chain it has no probe for, so the arm's only possible
  contribution was being the staler of the two. It is the `default:` now.
  And `retryUTXOTransactionStatus` restated three fields of
  `Chain::pending_status_poll` and got one of them wrong; its five-arm switch
  was `refreshPendingTransactions(chainName: <the same name>)` five times.

  *`seed_derivation_path_key` was `mainnet_counterpart` under a caller's name.*
  Literally: `chain.mainnet_counterpart().str_id()`. Published as the mainnet
  now, with `seedDerivationPathKey` a one-line reading of it, which is what let
  the diagnostics filing above say "this chain's own mainnet" without a fourth
  spelling.

  *Fifteen `EVMChainContext.<chain>` statics, kept "so the existing call sites
  read the same".* Those call sites were gone; what was left were two test
  assertions and three uses of `?? .ethereum` / `.bnb`. The `??` ones are worth
  naming: the fallback built a *fabricated* context with chain id 0 when the
  registry lookup missed, so the only case it could fire in — a registry with no
  Ethereum — is the one where it reports mainnet as not-mainnet and hands a
  wrong chain id to a pre-signing check. They guard instead.

  *And ten dead locals went with the pass* — `let now` in three refresh
  functions, `selectedCoin` and `walletIndex` in `SendExecution`, two in
  `DashboardStore`, and three `??` on non-optionals. The build has no
  unused-value warnings left, which is the point: they were hiding in a list too
  long to read.

- **The fifth Swift enum restating the chain list is gone, and so are the two
  copies of it in core.**

  `StakingSupportedChain` was seven cases with a `chainName` switch and a
  `chainId` switch, over ten call sites — both switches answering what `Chain`
  already answers. It survived the four-enum collapse because it is scoped to
  one tab rather than to the app, so the sweep that found `SpectraChainID`,
  `SeedDerivationChain`, `AppChainID` and `StandardDiagnosticsChain` never
  reached it. This document said the count was 0; it was 1.

  *And core held the other two.* `StakingService::fetch_validators` and
  `fetch_positions` each matched seven `const CHAIN_*: &str` spellings and fell
  through to `NotYetImplemented`, so "does this chain stake" was written three
  times across two languages, and the string spellings — `"internet-computer"`
  among them — were one typo away from a staking tab that lists no validators
  and says nothing. `Chain::supports_staking` is the fact,
  `StakingService::staking_chain` is the gate the two dispatches share, and the
  seven constants are `Chain::X.str_id()`.

  *The binding test is offline, which is why it is worth having.* Every staking
  client returns an empty list when it was given no endpoints, so a routed call
  and a refused one are distinguishable with no network:
  `the_registry_flag_and_the_dispatch_agree_on_every_chain` walks all
  seventy-eight and asserts the flag and the routing say the same thing.
  `staking_is_a_mainnet_answer` states why the flag does *not* go through
  `mainnet_counterpart` — the clients are built against mainnet endpoints, so
  Solana Devnet routing to the Solana client would report mainnet validators for
  a devnet wallet.

  *What stayed in the view, and why.* `StakingChainDescriptor` still names its
  seven chains — an APY estimate, an unbonding period, a minimum stake and a
  paragraph on the mechanics. That is editorial copy about a protocol, not a
  registry fact, and pushing it into `chains.toml` would put marketing text in
  the chain catalog. What came *out* of it is `chainName` and `symbol`, which
  the registry does answer and which could therefore disagree with every other
  screen. Three Swift tests keep the copy and the flag in step in both
  directions: every offered chain has copy, no copy names an unoffered chain,
  and the picker is mainnets only.

  Zero chain-name literals left in `StakingTypes.swift`, `StakingView.swift` and
  `StakingViewModel.swift`. `spectra chains --json` carries `staking` beside
  `privateKeyImport` and `watchOnlyImport`, and `spectra staking validators
  --chain Bitcoin` now says *"Bitcoin does not have protocol-native staking"*
  rather than "no endpoints registered for Bitcoin", which was true and about
  the wrong thing.

- **Twelve declared-and-never-referenced Swift types, and one file of them.**

  Not a collapse — a sweep, prompted by asking whether the small files in the
  root of `swift/` should be merged. They should not: the answer to a 47-line
  file is the naming rule this repo already has (one topic per file), and
  merging by size moves no line count. What the pass through them did find is
  that twelve top-level types are declared and mentioned nowhere else.

  `Identifiers.swift` was the whole file: four `Hashable`/`Codable` newtypes —
  `WalletID`, `HoldingKey`, `AssetIdentifier`, `TransactionHash` — under a
  header saying "adoption is incremental". Nothing adopted them, and
  `HoldingKey`'s doc had drifted to a format core does not use (it says
  `"<chain>:<symbol>"`; core's is `"<chain>|<symbol>"`), so the one thing the
  file offered a reader was wrong.

  `EthereumNetworkMode` is the tail of "a network mode is a chain", below —
  three cases and a display-name switch that survived the collapse with no
  caller. `WalletServiceBridgeError`, `WalletRustDerivationBridgeError` and
  `WalletRustEndpointCatalogBridgeError` are three `LocalizedError` enums with
  no `throw` site left: the failures they named are core's to report now.
  `SendPrimarySectionsView`, `DashboardDetailRow`, `BundleTokenImage`,
  `ChainToggleLabel`, `HistorySection`, `DonationDestination`,
  `WalletDerivationCurve`, `WalletRustSigningMaterialModel` and
  `BIP39EnglishWordList` are the rest.

  Root 16,233 → 16,155; `views/` 10,785 → 10,724. The `views/` half moves the
  root-vs-views metric the *wrong* way, which is the right trade: the metric is
  for finding restated per-chain facts, not a reason to keep dead code in the
  half that counts.

  *One correction on the way.* The sweep looks for a type name mentioned once,
  which finds dead types and not dead files: `SendPrimarySectionsView.swift`
  holds three live pages beside the dead composer, and deleting the file broke
  the build before the struct alone was taken. A file is dead when everything in
  it is, which is a second question.

- **The watch-address inputs became one table and one loop, and the import
  flow stopped holding per-chain text at all.**

  Two behaviour changes came out of this one and are written up above; the shape
  is that `ImportDraft` held twenty-three `var <chain>AddressInput` properties
  and a twenty-three row table transcribing them into
  `watchOnlyInputsByChainName`, and `WalletSetupViews` held eighteen sections
  naming their chains a third time. Three hand-written copies of one list, and
  all three differed: the fields had twenty-four entries (Monero's was never in
  the table, which is what made a Monero import impossible), the table had
  twenty-three, the view rendered eighteen, and `reset()` cleared eighteen of
  the twenty-three — Zcash, Bitcoin Gold, Decred, Kaspa, Dash and Bittensor were
  never cleared.

  One stored `[String: String]`, one `ForEach` over the chains the registry says
  can be watched, and `reset()` is one line that cannot be five short.
  `ImportDraft.swift`: 596 → 541. `WalletSetupViews.swift`: 1,402 → 1,339, and
  its chain-name literals **40 → 0** — the last one was a footer note that fired
  on any import naming Monero, which was noise while a Monero import was
  impossible and wrong once one derived. It reads the flag and the watch-only
  mode now, and `only_monero_is_excluded_from_watch_only_import` keeps the copy
  — which still names Monero — honest.

  *`supports_watch_only_import` is a column of `core_chain_identities` now*
  rather than a fact only Rust could see. That is what made the view's list
  derivable at all: the flag existed, the planner enforced it, and the front end
  had no way to ask.

  *And a guard that could not fire went with them.* The Cardano branch validated
  `typed("Cardano")` on the non-watch-only path, where the value is always
  empty: the per-chain fields live on the watch-addresses page, and all three
  writers of `isWatchOnlyMode` call `reset()` first. Traced rather than assumed,
  the same way the twenty-three `resolved<Chain>` fallbacks were traced last
  pass — this is the last of that family.

- **Four lists of which chains take a private key became one registry column:
  217 → 216, and the wide record in the import flow went with them.**

  The behaviour half is written up above; what belongs here is the shape. This
  is the `EVMChainContext` finding again — a hand-written per-chain list beside
  a registry that holds the fact — except there were four of them and the one
  that was right was the one nothing consulted.
  `chain_supports_private_key_import` is deleted rather than rewritten: the
  picker's list and the gate's answer are the same call now, so a second export
  asking "is this chain in that list" had nothing left to add.

  *And Swift's copy was a seventeen-field record.*
  `PrivateKeyImportAddressResolution` carried one optional per chain, was built
  by a twenty-arm switch, unpacked by twenty-six `record(…)` lines, and tested
  for emptiness by a seventeen-way `!= nil` chain — the same list written a
  fourth, fifth and sixth way. A private-key import selects exactly one chain,
  which `plan_signing_import` already refuses to exceed, so what the flow needs
  is one `String?`. `derivePrivateKeyImportAddress` is one call to the
  dispatcher with nothing to switch on. `AppState+ReceiveFlow.swift`: 707 → 652.

  *Twelve of the switch's arms were dead and one was missing.* The arms for
  Tron, Solana, XRP Ledger, Stellar, Cardano, Sui, Aptos, TON, Internet
  Computer, NEAR, Polkadot and Bitcoin SV each called the dispatcher for a chain
  it answers `None` for, so all twelve produced `nil` through a `try?`. Decred,
  which the dispatcher does derive, had no arm at all. A switch cannot state
  which of its arms are reachable; a registry flag and one test over every chain
  can.

  *The derivation also ran twice.* The guard derived an address to check that
  one came back, threw it away, and the record block derived it again a hundred
  lines further down. It is derived once and held.

  *One more per-chain fact reached the CLI.* `spectra chains --json` carries
  `privateKeyImport` beside `isEvm`, which is what makes the app's picker
  assertable from a second process — the acceptance script reads the column and
  then imports on a chain it claims.

- **Ten per-chain lookups became columns of the identity table: 230 → 218.**

  `core_address_slot`, `core_address_validation_kind`, `core_is_evm_chain`,
  `core_supports_deep_utxo_discovery`, `core_send_execution_shape`,
  `core_pending_status_poll`, `core_seed_derivation_path_key`,
  `core_seed_derivation_chain_raw`, `core_evm_seed_derivation_chain_name` and
  `core_network_choices` each took a chain name or id and re-derived from the
  registry. They are columns of `core_chain_identities`, which
  `Chain+Registry.swift` already reads once. `core_chain_display_name` and
  `core_chain_str_id_for_name` went too — they were `name` and `id`, looked up
  backwards.

  *Two of them were taking a string to parse back into a `Chain`.*
  `seed_derivation_chain_raw` and `evm_seed_derivation_chain` took a display
  name, called `Chain::from_display_name`, and matched. They take a `Chain`
  now and are internal.

- **Two token-balance fetchers with one signature: 218 → 217.**

  `fetch_token_balances` and `fetch_evm_token_balances_batch_typed` had the
  *same* parameters and the *same* return type, and covered complementary sets
  of chains — Tron, Solana, NEAR, Sui, Aptos and TON in one, the EVM family in
  the other. So a caller holding a chain had to know which family it was in to
  pick the method, which is what the chain id already says. One method with an
  EVM arm.

  *What this slice was not.* The estimate in C2 said the six
  `fetch_<family>_send_preview_typed` methods were one call, on the grounds
  that `Chain::send_execution_shape` already dispatches them. It does not:
  that shape dispatches *submission*. A preview takes what its family needs —
  an xpub and two gap counts for Bitcoin HD, a gas nonce and custom fee
  configuration for EVM, a fee priority for Dogecoin — and returns a different
  record for each. Collapsing them means a union on both sides, which buys a
  smaller number and a worse type. The 28 network fetches are closer to 24 than
  to the 12 the table estimates, and that row should be read as such.


- **The endpoint catalog: twenty exports → ten.**

  *Eight of them answered one chain's endpoints one field at a time* — the RPC
  list, the explorer supplements, the settings groups, the diagnostics checks,
  the transaction explorer, the broadcast providers, and two Bitcoin base-URL
  lookups. A settings screen showing one chain made six calls, each re-walking
  the same table, each a separate chance to be handed a different chain's
  answer. `app_core_chain_endpoints()` returns the catalog, one row per chain,
  and `AppEndpointDirectory` reads it once — the shape `core_chain_identities`
  established. It is safe to read once because the catalog is static: the doc
  on the old lookups already said a throw meant a corrupt bundle rather than a
  runtime condition.

  *`app_core_endpoint_for_id` had no caller* — only the plural did.

  *`app_core_live_chain_names` was `app_core_chain_backends` filtered by
  `integration_state`*, which the caller can do with the backends it already
  holds.

  *And `core_endpoint_role_mask` existed to be handed straight back.* The
  records lookup took a `u32` bit mask, and the only way to get one was to call
  core with role names and receive the mask — a round trip whose whole output
  was the next call's input. The boundary takes role names now; Rust callers
  that hold the mask constants use `endpoint_records_for_chain_masked`.


- **The history store: 26 methods → 20, and the database path stopped crossing.**

  *Eleven pagination methods for a three-field record.* Three getters answered
  the position one field at a time, so a caller that wanted the whole thing
  made three calls and could see it change between them — `history_cursor`
  returns it. Four resets differed only in how much to forget, which is one
  question with the answer in the method name — `reset_history(scope)` takes a
  `HistoryScope`. And `advance_history_page` had a bridge wrapper on the Swift
  side and no caller anywhere.

  *Twelve exported methods took a `db_path` the service already held.* Core is
  bound to its database by `open_state`, and every one of these asked the
  caller where that database was, on every call — nothing checked the argument
  against the binding. A caller passing a different path wrote to a different
  file and read back from the one core thought it was on, which presents as
  data silently disappearing. They read `bound_state_db_path()` now, and the
  path crosses in exactly one place: `open_state`.
  `records_land_in_the_database_the_service_was_opened_on` states it, including
  that an unopened store errors rather than guessing.


- **The confirmation-poll schedule and the clock moved to the table they
  schedule.**

  Five service methods took a `TransactionStatusPollConfig` — six numbers
  deciding how often a pending send is re-polled, how far the backoff goes, how
  many confirmations mean final, and when a pending send is given up on — and
  every one of them also took `now`. Both were the caller's: the six constants
  lived on `AppState`, so the schedule core applied was whatever the last call
  said it was, and a second front end would have had to know the same six
  numbers to behave the same way. Reading the clock from an argument meant the
  caller decided when "now" was.

  The config is a `Default` on the record, which no longer crosses the
  boundary, and each method reads `now_secs()` itself. Nine signatures lost
  arguments; six constants and a `finalityConfirmations(for:)` that returned
  one of them are deleted from `AppState`.

  *What it did to the tests.* They injected a short config and a fake clock, so
  they asserted core's arithmetic against numbers no build ever used. They
  assert the real policy now, which is testable without controlling time:
  nothing has been polled twenty seconds apart inside one test run, a
  `created_at` of zero is decades stale, and six recorded failures is the real
  threshold.


- **`spectra wallet import --private-key-file` — the last wallet operation the
  CLI could not drive.**

  Core has dispatched private-key derivation by chain since
  `core_derive_from_private_key`; what was missing was the command. The wallet
  metric reads "all" now.

  *The key is sealed, not stored.* iOS keeps a private key as a plain Keychain
  entry, which is reasonable where the operating system protects it; the CLI's
  store is files in a directory. So `wallet_secrets` gained a private-key
  envelope beside the seed one — same AES-GCM, same password-derived key, same
  salt and verifier — and `a_private_key_seals_and_unlocks_with_the_right_password`
  asserts the raw key does not appear in what lands on disk.

  *Derive before sealing.* A chain with no private-key derivation is refused
  before anything is written, so the store never holds a key for a wallet that
  could never sign with it. `spectra wallet import --chain Cardano
  --private-key-file …` exits 3 and says why.

  *And export returns it.* `wallet export` reported "no sealed secret for this
  wallet" for a private-key wallet — true about the phrase, wrong about the
  wallet. It handles both now, behind the same `--yes` gate and the same
  password, because a key the CLI can seal and never return is a lost key.


- **The transaction derived state stopped being ferried to core and back.**

  Three exports took the transaction list as an argument —
  `core_normalize_history`, `core_earliest_transaction_dates`,
  `core_active_wallet_transaction_ids` — so the caller converted its projection
  of *core's own records* into three different FFI input shapes and handed them
  back to be reduced. A fourth, `core_normalized_history_signature`, existed
  only so the caller could hash those records first and decide whether the
  round trip was worth making.

  All three are `WalletService` methods reading core's store; the signature
  function and its record are deleted, because core decides that where the data
  is. Six records stopped crossing with them — `NormalizeHistoryRequest`,
  `HistoryTransaction`, `HistoryWallet`, `TransactionEarliestInput`,
  `TransactionActivityInput`, `WalletChainInput` — which is twelve converter
  pairs, 455 → 443.

  This is the slice the previous pass unblocked by deciding the mirror does not
  change. It did not: `rebuildTransactionDerivedState` stopped recomputing from
  the projection and started adopting core's answer, which is what
  `dashboardAssetGroups` already did. `cachedTransactionByID` is an index into
  the projection and stayed local, as that decision said it would.

  *The trap in it, and the test for it.* `HistoryRecord::created_at` is Unix;
  the payload's `created_at` is in Swift reference time, and both are in scope
  at the point this code reads one. They differ by thirty-one years — reading
  the wrong one dates every history entry to 1993 and sorts the list by that.
  `derived_views_use_the_rows_unix_timestamp` writes a record from 2024 and
  asserts it does not come back from before 2023.


- **`submitSend` reads the route it was given.**

  Core routed the send in the preflight and returned `submitKind`; the function
  then re-derived the same decision from chain-name lists — `["Sui", "Aptos",
  "TON", …].contains(holding.chainName)`, `["Bitcoin Cash", "Bitcoin SV",
  "Litecoin"]`, `holding.symbol == "BTC"`, `isEVMChain(holding.chainName)` —
  fifteen branches of it. Two answers to one question, and the second one
  ignored the first.

  Every branch is `preflight.submitKind` now. What that surfaced is written up
  under "Behaviour changed on purpose": three of the fifteen returned *above*
  the shared gates, so twelve chains sent without biometric authentication or a
  high-risk destination warning, and the NEP-141 branch sent tokens core had
  declined to route.

  `every_sendable_chain_has_a_routing_kind_from_the_known_set` is the net the
  restructure needs: `submitSend` switches on eighteen strings that core
  produces, so a kind renamed in core would drop a chain into "not enabled yet"
  at the moment a user tried to send. It also names the six mainnets with no
  send path, so adding one is a decision rather than a dead branch someone
  notices later.


- **The two send-risk checks stopped being assembled by the caller.**

  *`core_evaluate_high_risk_send_reasons`* was handed the address book and
  every address this wallet had ever sent to on the chain — both core's own
  store. "Is this a first-time destination" is one of the risk signals, so the
  signal was only as complete as the caller's copy of the history. What crosses
  now is what the user actually did: the destination, what they typed, and
  whether an ENS name got them there.

  *`core_evm_recipient_preflight_warnings`* was handed the results of two
  contract-code probes, which are core's own network calls. The caller made
  both, swallowed their errors, looked up which token the holding is from the
  token list core owns, and handed the three answers back for core to judge.
  `WalletService::evm_recipient_preflight` makes the probes. A failed probe is
  still `None` rather than `false` — "we could not check" and "it is not a
  contract" are different answers and the evaluator treats them differently.

  Both are `WalletService` methods now and the two free functions lost the
  attribute. Three request records stopped crossing with them —
  `HighRiskSendRequest`, `HighRiskChainAddress`, `EvmRecipientPreflightRequest`
  — which is six converter pairs. The localisation stays on the platform: the
  codes cross, the strings do not.

  *Caught on the way:* both methods first landed in the file's *second*,
  un-exported `impl WalletService` block, because the doc comment they were
  anchored to had moved there. They compiled, the app compiled, and neither
  crossed the boundary — the failure showed up only as "the generated bindings
  do not mention them". The inverse of the `pinned_prototype` mistake, and it
  fails just as quietly.


- **Send routing became one decision, and the duplicated rule went with it.**

  The previous pass moved the submit preflight into core and left a debt in
  writing: the Solana and NEAR send-support rules existed in core *and* in
  Swift, because the preview path still had its own copy. There were three
  answers to one question — the preview assembled a `SendAssetRoutingInput`
  and asked core to route it, the submit path re-checked
  `isSupportedSolanaSendCoin` on its own side after core had already routed the
  send in the preflight, and core derived it a third time.

  `WalletService::send_asset_routing(wallet_id, holding_key)` is the one
  answer, sharing the preflight's derivation. The preview asks it; the submit
  branch reads `preflight.submitKind`, which core had already computed and the
  caller was throwing away. Both Swift predicates are deleted.

  *And `plan_send_preview_routing` turned out to be a wrapper with nothing
  left.* It read `route_send_asset().preview_kind` and discarded the rest, so
  once the preview asked for the whole route it had no caller — gone with its
  request and plan records. Its test asserted the preview kind alone; the
  replacement asserts that the preview and the submit kind are the same route,
  which is the property that was actually at risk, plus that an untracked mint
  routes nowhere rather than to Solana.

  `routing_follows_the_token_list_core_holds` states it at the service level:
  the same holding is unroutable before its mint is tracked and routes to
  Solana after, with nothing passed in.


- **Refresh scheduling moved into core, and one dead planner went with it.**

  Five exports answered one question between them —
  `core_active_maintenance_plan`, `core_should_run_background_maintenance`,
  `evaluate_heavy_refresh_gate`, `compute_background_maintenance_interval`,
  `active_pending_refresh_interval_for_profile` — each taking the piece of
  `AppState` it needed as an argument, because core held none of it.
  `WalletRefreshPlanner` was 80 lines of Swift packing five `Date?` properties
  and two dictionaries into request records and unpacking the answers: the
  `core_plan_*` shape, wearing a different name.

  `RefreshClock` is on `WalletService` now, and one
  `maintenance_plan(conditions)` answers what to do this tick and how long to
  wait for the next one. Not persisted, and that is the whole difference from
  the keypool — a restart should refresh, which is what an empty clock already
  means. The intervals it needs are settings core owns, which is why this slice
  had to follow the settings one. What crosses is `DeviceConditions`:
  reachability, power, battery, and whether a screen showing prices is in front
  of the user — the half core genuinely cannot know.

  *`core_chain_refresh_plans` had no caller.* It and `runPlannedChainRefreshes`
  were reachable only from the planner's own test — the third instance of that
  pattern in this document. Gone with two records.

  *And a fifth copy of the chain table.* `display_name_for_chain_id` was a
  24-arm match inside the policy module, accepting both `bitcoincash` and
  `bitcoin-cash` because its callers disagreed about the id format. Ordering is
  `Chain::chain_display_name` now.

  The Swift tests went with the planner: they asserted core's arithmetic
  through a Swift wrapper, and `policy.rs` states it directly — including the
  case they could not reach, that a stamped clock is the *same* clock the next
  question reads. What is left on the Swift side is the half core cannot know.

- **The send submit preflight stopped taking core's own answers as arguments.**

  `core_send_submit_preflight` was handed `walletFound`, `assetFound`, the
  available balance, and — on the funds path — `isEvmChain`,
  `supportsSolanaSendCoin` and `supportsNearTokenSend`. Core owns the wallets,
  the holdings, the registry and the token preferences all six are derived
  from, so it was trusting a caller's answer about its own state to decide
  whether a send may be made and how it is routed. Rule 0's second limit is
  exactly this: derive rather than trust a typed value.

  `WalletService::send_submit_preflight(wallet_id, holding_key, destination,
  amount)` derives them. `holding_key` is `"<chain>|<symbol>"`, which core can
  compute, so nothing about the identity crosses either. The request record
  stopped crossing with it.

  The two send-support rules are core's now and have tests that could not exist
  while they were iOS predicates: SOL always, a Solana token only when its mint
  is tracked, NEAR itself never (the native path handles it), and contract
  matching case-insensitive. The Swift copies survived this slice because the
  preview path still called them; the slice below removes that path and them.


- **The settings blob moved into `CoreAppState`: 2 methods and a 23-field
  record gone, and the CLI gained a settings surface.**

  Written up in full under "Behaviour changed on purpose" — what belongs here
  is what it did to the shape. `PersistedAppSettings` was the last typed record
  that existed only to carry one front end's state across the boundary and back
  unchanged: core stored the JSON and had no opinion about any field in it.
  Eighteen fields are `AppSettings` now, behind one `SetAppSetting` command with
  a variant per field, and the bounds that were iOS `didSet` clamps are the
  reducer's. Four fields stayed on iOS, which is the first time that header's
  rule — "do not add a field here that only one front end reads" — has been
  applied in the other direction.

  `spectra settings list|get|set` is the check: setting a stop gap of 9999
  reads back 200 in a second process, which no test could state while the
  bounds lived in a `didSet`.

  Root lines went *up* by 92. The mirror that replaces the blob writer is
  larger than the blob writer was, and the honest reading is in the write-up:
  this round moved ownership without removing Swift.


- **291 exports → 252, and the last three copies of the derivation dispatch
  became one.**

  *Thirty-three `derive<EvmChain>` exports were one function.* A macro stamped
  out `derive_ethereum`, `derive_arbitrum`, `derive_x_layer` and thirty more,
  every body the same call — an EVM address does not depend on which EVM chain
  asks for it. Nothing outside the crate called any of them: Swift went through
  `core_derive_for_chain`, whose thirty-three arms picked between thirty-three
  copies. One `derive_evm`, one arm, `c if c.is_evm()`.

  *And `derive_for_chain_name` stopped matching on strings.* Seventy-eight arms
  keyed on display names, with no way to say it had them all — a typo fell
  through to the error arm and read as an unsupported chain. It matches on
  `Chain` now, so the arms are variants and the fallthrough can only be reached
  by a chain the registry does not know.

  *`self_tests.rs` held a third copy* — eighteen arms, the smallest of the
  three, silently returning `None` for the chains nobody had extended it to. It
  calls the dispatcher. The script types it forced are the ones the dispatcher
  derives from each spec's path anyway (`m/84'` is P2WPKH, `m/44'` is P2PKH).

  Three more exports had no caller across the boundary and lost the attribute:
  `diagnostics_parse_jsonrpc_probe`, `fiat_currency_code`,
  `transactions_for_wallet`. `WalletService` has a second, un-exported `impl`
  block now, so "reachable from Rust" and "an entry point" stop being the same
  thing.

- **The diagnostics registry: read-everything / write-everything became record
  one row.**

  `diagnostics_all_*` and `diagnostics_replace_*` came in pairs, so storing one
  wallet's result read every row for that chain across the boundary, inserted
  into the copy, and sent all of them back. Two wallets refreshing at once kept
  whichever finished second. `diagnostics_record_*` writes one entry under the
  registry's own lock, and the getters are internal — the exporter still builds
  a document from them, and nothing outside the crate wanted a whole map.

  Two things Swift did with those maps moved with them.
  `diagnostics_run_summary` answers the screen's two numbers — how many wallets
  reported, and which source each used — replacing a pair of five-way switches
  on the record shape that existed to reach the two fields every shape has. And
  `diagnostics_forget_wallet` replaces twenty-seven lines naming every chain on
  every shape to drop one wallet: a wallet is gone from all of them or none.

- **Seven `<Chain>BalanceService` shells, and thirty-four one-line forwards.**

  `ChainTypes.swift` held seventeen of these enums; for seven the whole body was
  `endpointCatalog()` and `diagnosticsChecks()` forwarding to
  `AppEndpointDirectory` with the chain's own name. Six call sites remained
  after the endpoint switches collapsed, and they name the chain themselves now.
  `ChainTypes.swift`: 341 → 281, and the ten enums that keep real content —
  Tron's contracts, Monero's backends, Solana's token metadata — keep it.


- **The diagnostics descriptor table: 24 rows → 8.**

  Three fall-throughs replace the rows that only named a chain — the shared
  simple shape (8 rows), the EVM family (5), and the UTXO chains (3). What is
  left is genuinely different and says why: Bitcoin walks an xpub, Dogecoin
  counts history entries directly, Tron and Solana have their own record
  shapes, and Ethereum, BNB Chain, Monero, NEAR keep endpoint probes that parse
  JSON-RPC inline rather than just reaching the host.

  Chain-name literals in `swift/` root: 1,034 → 973.

- **Eleven preview refreshers became one, with two real exceptions named.**

  `refresh<Chain>SendPreview` existed eleven times, each building the same
  config with a chain name, a symbol, an address resolver and a message. Two of
  the eleven genuinely differed and now say so in one place: Solana's
  sendable-coin rule is its own, and Polkadot refuses to preview for a wallet
  with no seed phrase because the estimate needs the derived account.
  `AppState+SendPreview.swift`: 431 → 358.

- **Ten send arms became one, and the merge nearly broke Monero.**

  `submitSend` had ten arms calling one of two helpers, each passing the same
  four constants inline: how many decimals to show in a fee-shortfall message,
  whether a private-key-only wallet can sign, how the estimated fee enters the
  request (gas budget, explicit amount, satoshis, or nothing), and what fee to
  assume with no preview. Those are `Chain::send_execution_shape` now, and the
  two helpers are one `submitNativeChainSend`.

  The values were transcribed from the call sites rather than re-derived, and
  `the_shape_matches_what_the_call_sites_carried` pins all twelve chains —
  these decide whether a send is refused for insufficient fee and how the fee
  reaches the signer, so they are worth a test that fails loudly.
  `AppState+SendExecution.swift`: 811 → 677.

  **The merge introduced a failure the build did not catch.** The unified
  helper resolved a derivation chain and bailed if there was none. Eleven of
  the twelve chains have one; **Monero does not** — it signs from stored key
  material, and the arm being replaced passed an empty path on purpose. Every
  suite would have stayed green while Monero sends failed with "unable to
  resolve derivation path".

  Caught by checking, not by a test: after the build went green, listing which
  routed chains actually resolve a `SeedDerivationChain` showed eleven OK and
  one missing. The lesson is the ordinary one for this kind of merge — the
  thing a per-chain arm did *differently* is the thing worth enumerating before
  deleting it, and "it compiles" says nothing about it.

- **Eighteen preview records agreed to call the fee the same thing — and a
  round that cost more lines than it saved.**

  Every send-preview record carried the estimated fee under a chain-specific
  name: `estimatedNetworkFeeSui`, `…Apt`, `…Ton`, `…Xrp`, sixteen spellings of
  one concept. A caller that wanted "the fee" had to know which chain it was
  holding. They are all `estimatedNetworkFee` now, which is what let the send
  path stop caring.

  On the back of that: three identical `refresh<Chain>SendPreview` functions
  became one `refreshSendPreview(forChainNamed:)`, three identical send arms in
  `submitSend` became one, and `SendPreviewStore` gained
  `apply(_:forChainNamed:)` / `estimatedFee(forChainNamed:)`.

  **This round made the line count worse — 16,712 → 16,765 — and that is worth
  stating rather than hiding.** The scattered per-chain closures it removed
  were three or four lines each in a dozen places; the keyed switches that
  replace them are two 20-row tables in one file. Rule 2 prefers that (a
  per-chain fact stated once, where it can be found), and the field rename is
  a real fix. But "collapse a wrapper family" stopped paying in lines several
  rounds ago, and pretending otherwise would misread the next person about
  where the remaining work is.

  *What that implies for the metric.* Root-line-count moves when a subsystem
  moves into core (the dashboard rows: -73) or a file is deleted. Folding Swift
  wrappers into Swift dispatch reduces the number of places a chain is named —
  which is the second Stage 3 criterion — and barely touches the first. The two
  criteria pull in different directions at this point, and the line one is now
  the weaker signal.

- **The UTXO rescan: ten accessors and five wrappers over one keyed table.**

  `AppState` had `isRunning<Chain>Rescan` and `<chain>RescanLastRunAt` for five
  chains — ten computed properties, each four lines, each forwarding to
  `utxoRescanStateByChain[<name>]`. The table was already keyed; only the
  accessors named chains. One `[rescanFor:]` subscript replaces them.

  On top sat five `run<Chain>Rescan()` wrappers, each passing two key paths, a
  chain name and its ticker. The ticker is on the chain descriptor, the state
  is keyed, and both refreshes take a chain name — so `runUTXORescan(chainName:)`
  is the whole thing, with `supportsDeepUtxoDiscovery` as its guard.

- **Twenty-two per-chain diagnostics accessors, and 82 lines of reset that
  said nothing.**

  `DiagnosticsState` had 24 `<chain>HistoryDiagnosticsByWallet` accessors and
  `DiagnosticsStore` had 24 forwards for them — four lines each, all calling
  one of three shape-specific functions with a chain name. Two more keyed
  subscripts (`[utxoHistoryFor:]`, `[evmHistoryFor:]`) join the
  `[simpleHistoryFor:]` that already existed, and 22 of the 24 collapse. Tron
  and Solana keep theirs: their records genuinely differ.
  `DiagnosticsState.swift`: 397 → 301, `DiagnosticsStore.swift`: 158 → 118.

  *The full-reset path was 82 lines of `self[…For: "<Chain>"] = [:]`* — 22
  chains times four — immediately above a call to `diagnosticsClearAll()`,
  which empties the entire Rust-owned registry. A comment called the block
  "belt-and-suspenders" over exactly that call, which is a fair description of
  code that does nothing. It is the one call plus four lines for the two
  Swift-held tables now. `StoreLifecycleReset.swift`: 422 → 350.

  *A mistake worth recording:* the blanket deletion also took Tron's and
  Solana's accessors, because they matched the same four-line shape while
  calling different functions. The compiler caught it immediately, but the
  lesson is the same one as the `sed` that matched nothing — a pattern that
  fits the shape is not a pattern that fits the meaning.

- **Three wrapper families collapsed, and the net caught me.**

  *Eighteen `refreshPending<Chain>Transactions()`* forwarded to one of two
  generics, each naming a chain, a chain id, an address resolver and up to two
  flags. How a chain reaches finality is `Chain::pending_status_poll` now —
  UTXO polling with two flags, history-txid, EVM receipt, or none — and one
  `refreshPendingTransactions(chainName:)` dispatches on it.

  *Fifteen `refresh<Chain>Transactions(loadMore:)`* did the same for history.
  All fifteen passed a `resolved<Chain>Address` function that
  `resolvedAddress(for:chainName:)` already dispatches, so the callee looks it
  up. Bitcoin and Dogecoin keep their own: HD xpub expansion and a
  confirmed-fee path are real differences.

  *A 24-row refresh descriptor table* named every chain twice more. With both
  fetches taking a chain name and `supportsDeepUtxoDiscovery` on the registry,
  the row is the chain name and the list comes from core.
  `ChainRefreshDescriptors.swift`: 208 → 155.

  *Eight diagnostics descriptor rows* were byte-identical but for the chain
  name. A chain with no row falls through to a shared path now.

  **The last one shipped a bug that a test caught.** The fall-through passed
  `chain.title` where a chain name belongs — and `title` is
  `"Aptos Diagnostics"`, a screen heading. Eight chains' diagnostics would have
  resolved to nothing and silently done nothing. `StandardDiagnosticsChain`
  gained a `chainName` alongside `title`, with a comment on both saying which
  is which.

  Worth being precise about what saved it: not the existing net.
  `DiagnosticsChainTableTests` asserted one descriptor per chain, which this
  change deliberately breaks, so it failed for the intended reason and had to
  be rewritten. The bug was caught by the *replacement* I wrote for it — "a
  chain without a descriptor must still resolve by name" — which is the
  assertion the new shape needs and the old one could not have made.

- **The dashboard's rows moved into core.**

  `_rebuildDashboardDerivedStateBody` was ~100 lines of Swift reading five
  caches to do three domain things: group holdings into rows, order the rows by
  value, and put pinned symbols first. `WalletService::dashboard_asset_groups`
  does it now. Only the live prices cross — core already holds the holdings,
  the tracked tokens, the pinned list and the selected networks, and the
  records (`CoreDashboardAssetGroup`, `CoreDashboardAssetChainEntry`) had been
  sitting in `wallet_domain.rs` as mirrors the whole time.

  `DashboardStore.swift`: 299 → 226. `formatting_dashboard_asset_grouping_key`
  and its Swift memo went too — the key is computed where the grouping is.

  *Two things worth recording.* The first test I wrote asserted that ETH on
  Ethereum and ETH on Arbitrum are one row; it failed, and the port was right —
  `dashboard_asset_grouping_key` includes the chain, so they are two. See the
  open question below. And `pinned_prototype`, a private helper, landed inside
  an `#[uniffi::export]` block and was therefore exported; the export audit
  caught it the same pass. `lib.rs` warns about exactly this, which is not the
  same as remembering it.

- **Rule 0's first two applications.** Both are written up in full under
  "Behaviour changed on purpose"; what belongs here is what they did to the
  structure.

  *The network mode became a chain id.* The `NetworkModes` FFI record is gone
  along with its two three-chain matches, `wallet_derived_state` lost its third
  argument, and the platform settings blob lost three fields — the selection is
  `AppSettings.network_chain_by_family` now. This is the shape the earlier
  slices kept producing: a second model of something the registry already had,
  and a bug living in the gap between them.

  *The EVM send gate lost its exception list*, and with it a test that existed
  only to fail anyone who fixed the split.

- **Sixteen pairs of functions doing one function's work; two planners kept
  alive only by their own tests.**

  The `plan_x` / `core_x` split had a reason once — `core_x` was the exported
  wrapper, `plan_x` the testable inner. For sixteen of them `core_x`'s entire
  body was `plan_x(same args)` and `plan_x` had no other caller, so the split
  bought a second name to grep and nothing else. The attribute moved onto the
  definition, including the three icon helpers that were exported from a
  different module than the one they live in. `plan_*` count: 34 → 15.

  Two of the rest — `plan_store_derived_state` and `plan_transfer_availability`
  — turned out to be reachable **only from tests of themselves**. This document
  already said `wallet_derived_state` replaced them; what it did not say is
  that the originals were still compiled, still carried their five
  index-and-flags request records, and still had a passing test each, which is
  what makes dead code look alive. Both are gone with their records, their
  tests, and `can_send_holding` — the index-taking twin of `can_send_coin`,
  whose only caller was the planner. `send/transfer.rs`: 264 → 56 lines.

  *What made this checkable:* the replacements already covered the same
  ground — grouping, portfolio exclusion, send gating — against the real
  `wallet_derived_state` rather than against an index list a caller assembled.
  Deleting a test is only safe when you can name the test that now covers it.

- **Two test-isolation faults, found by changing behaviour on top of them.**

  Neither was introduced by this pass; both were latent and surfaced the moment
  a change perturbed timing or shared state. Recording them together because
  they are the same failure with different owners.

  *`awaitPendingCoreStateWrites` had nothing to wait on.* The mirror committers
  called `beginCoreStateRead()` *inside* their `Task`, so at the moment a caller
  asked "has everything settled?", no epoch had been claimed and the answer was
  yes. The epoch's own doc says "claim an epoch before awaiting core"; they now
  do, and a failed command calls `finishCoreStateRead` so a write that never
  lands cannot hang the wait forever.

  *The iOS `setUp` reset wallets but not settings.* Adding a test that switches
  to Bitcoin testnet4 left every later test on it, and the one that broke was an
  unrelated transaction-status test three cases away. Selecting each family's
  mainnet is part of the reset now.

- **A test helper that could collide with itself.**

  `wallet_db`'s `tmp_db()` named its file from `subsec_nanos()` alone.
  Thirteen tests share it and the runner is parallel, so two could take the
  same nanosecond and read each other's rows. It surfaced as
  `app_state_round_trips` failing in a full run and passing in isolation.
  Keyed on process, thread and a counter now. Worth recording because this is
  the second instance — the token-preference helper had the same fault — and
  the failure it produces looks like a bug in whatever test lost the race.

- **One chain table, found six times.**

  Mapping a chain to a `CoreTokenTrackingChain` was written out in six places
  before this pass: `from_chain_name`, its inverse `chain_name`, a private
  `chain_label` in the token merge planner, a private `chain_name` in the
  dashboard planner, `chain_display_name` in `send/transfer.rs`, and
  `tokenTrackingChainFor` in Swift. Four of them were byte-identical
  eighteen-arm matches.

  `chain_name` is the table. Everything else asks it, `from_chain_name` scans
  it case-insensitively, and the two Swift-side copies went with the planners
  they served. This is rule 2 exactly, and worth noting that no two of the six
  disagreed — the cost had not been paid yet, which is the only reason it was
  cheap to fix.

- **The operational log moved: 4 → 3, and the cap became enforceable.**

  `chainOperationalEventsByChain` was a Swift dictionary with a `didSet`
  writing a KV blob; core owned `plan_append_chain_operational_event`, which
  prepended an event and truncated to 200. The caller minted the id, read the
  clock, passed the existing list in and wrote the capped list back — so the
  bound, the ordering and the identifier were all only as correct as whichever
  caller last handled them.

  `WalletService::append_chain_operational_event` stamps the id and the time,
  applies the cap and persists, all under its own lock.
  `the_log_is_newest_first_and_bounded` pushes 205 events and asserts 200
  survive in the right order — a property the planner *stated* and could not
  enforce.

  Not in `CoreAppState`: 200 entries times every chain is too much to clone on
  an unrelated `SetFiatCurrency`, so it takes the keypool's shape — in memory,
  write-through, loaded by `open_state`.

  *A `String`-backed enum went with it.* Swift's `ChainOperationalEvent.Level`
  had a `rawValue` that one display site used; the generated uniffi enum has
  none, so the level gained a `displayName`. Worth recording because of how it
  presented: the missing `rawValue` failed inside a `ForEach`, and the error
  Swift reported was that the element did not conform to `Identifiable` — which
  it did. Two rounds went into the conformance before the actual cause.

- **Two more planners folded into the state they decide about: 6 → 4.**

  Both were the same shape, and both only became removable because the state
  had already moved.

  *`price_alert_evaluation`* took the alert list as an argument, returned
  `has_triggered` updates, and left the caller to write them back. Two callers
  did — Swift and the CLI — with near-identical loops. Now
  `WalletService::evaluate_price_alerts(prices)` reads its own alerts, records
  what changed through `SetPriceAlerts`, and returns only the notifications a
  platform must actually send. The alert list no longer crosses the boundary in
  either direction; only the live prices go out, because a live price is the
  one input core does not have.

  *`merge_built_in_token_preferences`* took *both* lists from the caller. The
  built-in half was core's own catalog: Swift called `list_all_builtin_tokens`,
  reshaped each row into a preference entry, and handed it back so core could
  merge it against a list core also held. `built_in_token_preferences()` does
  the reshaping where the catalog is, and the merge is a `WalletService` method
  that reads and stores its own preferences.

  A built-in's `id` is now `builtin:<chain>:<contract>` rather than a UUID the
  caller minted. Not a compatibility question: the caller regenerated those ids
  on every launch, so nothing depended on them being stable — and now something
  can.

  **The chain mapping this uncovered had four copies.** `tokens.toml` says
  which chain a token is on; turning that into a `CoreTokenTrackingChain` was
  written out as a match in `from_chain_name`, again inverted in
  `chain_name`, a third time as a private `chain_label` inside the merge
  planner, and a fourth time as `tokenTrackingChainFor` in Swift. `chain_name`
  is the table now; `from_chain_name` scans it case-insensitively (which also
  handles `tokens.toml` spelling BNB Chain `"bnb"`), `chain_label` is gone, and
  the Swift copy went with the planner.
  `the_catalog_chain_names_all_resolve` walks both directions.

- **The owned-address table moved, and the two keypool planners fell out.**

  `chainOwnedAddressMapByChain` was an `@Observable` dictionary on `AppState`
  with a `didSet` that wrote through to `wallet_owned_addresses`. SQLite had a
  copy, but only ever read at launch — Swift was the authority. That is what
  made `core_plan_baseline_chain_keypool_state` necessary: core could not
  compute the baseline, so Swift computed it and passed it in.

  Which meant the guarantee `reserve_receive_index` exists to provide was
  weaker than it looked. Core holds one write lock across the whole
  read-modify-write so two callers cannot be handed the same receive address —
  but the floor that reservation starts from arrived *as an argument*, computed
  from a caller's copy of three tables. The lock protects the increment, not
  the input.

  Core owns the table now: in memory, write-through, loaded by `open_state`
  beside the keypool. `chain_keypool_baseline` reads its own transactions, its
  own owned addresses and the wallet's address slot, so `keypool_state`,
  `reserve_receive_index` and `reserve_change_index` take no baseline at all.
  `a_recorded_owned_address_raises_the_baseline` states the property that shape
  could not: register index 7, and the next receive index is 8, with nobody
  passing anything in.

  Deleted on the Swift side: the dictionary and its `didSet`,
  `baselineChainKeypoolState`, `parseUTXODiscoveryIndex` (ported to
  `app_core::utxo_discovery_index`), `persistChainOwnedAddressMap`,
  `persistOwnedAddressesForChain`, `persistOwnedAddressToRust`, and the
  launch-time load. `AppState+SendFlow` is 1,531 → 1,470.

  *Two things fell out of doing it.* `discoverUTXOAddresses` computed the
  highest owned external index and took `max` against `nextExternalIndex` —
  which the baseline already folds in, so the term was dead arithmetic over a
  table it had to hold to compute. And `delete_wallet_relational_data` cleared
  SQLite but not core's in-memory keypool rows, so a deleted wallet's indices
  stayed in the baseline for the rest of the session; it clears both now.

  *One thing this cost:* `knownOwnedAddresses` is async now, so
  `TransactionDetailView` caches the one value its body needed rather than
  calling into core from a computed property. That cache is view state by rule
  4 — losing it costs a redraw — and it is the pattern this document already
  blesses for `walletDerivedCache`.

- **`service.rs` split by owner: 4,677 lines → nine files, largest 1,359.**

  It had accumulated the marks of the merges that built it — six `// ── Merged
  from service_*.rs` banners, three `#[path]` module declarations pointing at
  files in a `service/` directory that already existed, and a block of orphaned
  `// ── Phase 2.1` navigation comments whose methods had moved out from under
  them. The split is by *owner*, which is the only question a reader has:
  [`state`] holds the resident `CoreAppState` and its persistence, [`network`]
  every read that leaves the process, [`send`] fee-to-broadcast, with
  `helpers` / `types` / `standalone` under them.

  Rust allows many `impl` blocks per type and UniFFI exports them as one, so
  nothing about the boundary changed — the export count was identical before and
  after.

  *Done as a line-assignment with a coverage assertion* rather than by hand:
  every one of the 4,677 lines is assigned exactly one destination and the
  script refuses to run if the assignment double-counts or misses a line. Three
  of the merged sections turned out to carry their own `impl` header, which the
  assertion did not catch — it checks that lines are not lost, not that the
  result parses — and the compiler found all three.

- **Two chain tables became one, and the enum became an index into it.**

  `Chain::str_id`, `Chain::chain_display_name` and `Chain::coin_symbol` were
  78-arm matches restating the catalog's `id`, `name` and `gas_token_symbol`
  columns, with `from_str_id` a 78-arm reverse map on top. The enum is declared
  in `chains.toml` order, so a variant *is* an index: `Chain::entry()` returns
  the row, and those four functions become field reads. `registry.rs` loses the
  tables; adding a chain is a TOML edit plus one variant.

  **The first version of the test that guards this proved nothing.** It
  asserted `chain.str_id() == entry.id` — but `str_id` reads the catalog now,
  so the two agree by construction. Swapping two entries in `chains.toml` left
  it green. The rewrite spells the expected id from the *variant name* instead,
  which is the one source independent of the table being checked, with six
  named exceptions where the enum and the catalog legitimately differ
  (`Icp`/`internet-computer`, `BnbChain`/`bnb`, and four more). Verified by
  perturbing both sides: swapping two catalog rows and swapping two enum
  variants each fail it, naming the chain.

  Compile-time exhaustiveness survives the deletion: `address_validation_kind`
  and `static_fee_units` still match all 78 variants with no wildcard, so a new
  variant cannot be added without the build noticing.

- **Three duplicate module pairs, and a module with three names.**

  `validation.rs` (seed-phrase and password field rules) sat beside
  `derivation/validation.rs` (per-chain address rules) — same name, different
  module, and a caller had to know which. Address validation is not derivation;
  both are now `validation/`, with `address.rs` under it.

  Under that, `derivation/mod.rs` carried `pub use validation as addressing;`
  and `pub use xpub_walker as utxo_hd;` — aliases from an earlier restructure,
  so the address rules answered to *three* paths and half the call sites used
  the alias. Both aliases are gone and every caller names the one module.

  `fetch/refresh.rs` and `fetch/refresh_engine.rs` were a pair of names saying
  which was written first. They are `refresh/policy.rs` (decides whether a
  refresh is due — pure, no I/O) and `refresh/engine.rs` (does it).

- **Ten dead exports deleted or un-exported.**

  Three `WalletService` methods with no caller in Swift, the CLI or Rust
  (`delete_owned_addresses_for_wallet` and the two address-book KV methods —
  leftovers from before the address book moved into `CoreAppState`);
  `http_probe`, which had none anywhere; `http_get` and five `derive_<chain>`
  functions, which are called only from inside Rust and lost the attribute.
  `CorePersistedAddressBookStore` / `Entry` went with the KV methods.

- **Five JSON builders → one, and the caller stopped ferrying core's own data.**

  `diagnosticsJSON(for:)` was a 24-case switch picking between five builders,
  and each case read history *out of core's registry* and handed it straight
  back across the FFI so core could build a document from it. Core owns that
  storage, and which shape a chain reports is now
  `Chain::diagnostics_shape` — a registry fact, so `core_diagnostics_json`
  matches on no chain names at all. The chain name is the whole input.

  `every_chain_produces_a_document` walks the registry and asserts one comes
  back; five separate builders could not say that, and a chain with no builder
  said nothing. `StoreDiagnosticsExport.swift`: 342 → 156, and the three
  wrapper helpers plus `simpleEntries` went with the switch.

- **950 exported functions → 765.** Measured from the generated bindings, which
  is the only honest count: grepping `#[uniffi::export]` in the source says
  "231", because one macro invocation can expand to a hundred exports. That gap
  *was* the problem, not a measurement artefact.

  *`per_chain_registries!` was 120 of them.* The macro stamped out five
  exports — get / set / remove / all / replace — for each of twenty-four
  chains, over one hash map. Swift called `all` and `replace`; **`get`, `set`
  and `remove` had no caller on either side, for any chain** — seventy-two
  functions that existed to be generated. Now three registries keyed by chain
  name plus Tron and Solana unkeyed, two operations each: ten exports.

  *Ten records became one.* `simple_address_diagnostics!` stamped out ten
  structs — `XRPHistoryDiagnostics`, `StellarHistoryDiagnostics`, eight more —
  from a single field list. The macro was core admitting they were identical
  while making the callers pay for the difference: ten typed slots, ten
  dictionaries, ten forwarding pairs, and a Swift protocol whose entire job was
  to treat them as one type again. One `SimpleHistoryDiagnostics`, and the
  protocol, its ten conformances and the ten dictionaries went with it.
  `DiagnosticsStore.swift`: 707 → **158**.

- **231 source-level exports → 198, in two earlier cuts.**

  *Sixteen had no Swift caller at all.* Ten were dead outright and are deleted;
  six were used only inside Rust and lost the attribute. Nothing else changed.

  *Fifty `derive<Chain>` exports became one.* Swift called each from exactly one
  arm of a 212-line switch in `WalletRustDerivationBridge` that reproduced a
  dispatch core has always had — `derive_for_chain_name`, which the CLI already
  used. The switch is now four lines calling `core_derive_for_chain`, the bridge
  is 356 → 161 lines, and 40 exports lost the attribute while keeping their
  bodies, because the dispatcher calls them.

  The new export can state something the fifty could not:
  `every_registry_chain_derives_through_one_call` walks every chain in the
  registry and asserts an address comes back. Fifty separate functions had no
  way to say "these are all of them".

  **The distribution is the finding.** Of 231 exports, **173 have exactly one
  Swift call site**. That is what a helper library looks like from the inside:
  not an interface, a pile of one-shot favours. The remaining cuts are the same
  shape as the derive one — find the Swift switch, ask what single call it is
  standing in for.

- **The CLI gap is closed: `rescan`, `pool`, `alert`.**

  *`rescan`* drives the funds finder. Core derives the candidate matrix — 55
  addresses, twelve of them Bitcoin across four script types and three
  accounts — and its own doc said "the balance of this address is checked
  separately by Swift", which was the half with no second implementation.

  *`pool`* reserves receive and change indices. Core holds one write lock
  across the whole read-modify-write because two callers racing there hand the
  same receive address to two people; a second *process* is the only way to
  test that guarantee, and it holds — reserving a receive index repeatedly
  returns the same one, change consumes every time.

  *`alert`* needed the state to move first. Price-alert rules lived only in
  Swift (`priceAlerts.snapshot`), with core owning the evaluator and Swift
  applying its verdict — the `core_plan_*` shape this document is removing.
  They are `CoreAppState.price_alerts` now, behind `SetPriceAlerts`, which
  refuses an alert with a non-positive target because it could never fire.

  **That slice left the app behind, and for a pass it was worse than before.**
  Core gained `price_alerts`; Swift kept its own `[PriceAlertRule]` and went on
  loading and saving the `priceAlerts.snapshot` blob through
  `load_price_alert_store` / `save_price_alert_store`. So the CLI and the app
  each had a complete, authoritative, *separate* alert list — one copy of the
  truth turned into two, which is the exact debt this document exists to
  remove. Found by auditing the export list for dead entries, not by anything
  failing: no test opened one list and read the other, and there was no way for
  a user to notice from inside the app.

  Now `AppState.priceAlerts` is a mirror on the `tokenPreferences` pattern —
  assigning sends `SetPriceAlerts`, the stored list lands back through
  `applyCoreState`. The Swift `PriceAlertRule` struct is a `typealias` for
  core's record, so the `id` is core's opaque string rather than a
  platform-minted `UUID`, and `evaluatePriceAlerts` stopped converting one
  shape into the other before every call. Gone with it: two FFI methods, two
  records (`CorePersistedPriceAlertStore` / `Rule`), two bridge wrappers, the
  `priceAlerts.snapshot` key, and the `UserDefaults` line in the reset path.

  *Worth naming the pattern:* moving state into core is not done when core can
  hold it. It is done when nothing else does. A slice that adds the core-side
  half and stops has not moved anything — it has forked it.

  **The round-trip test was written before the command this time.** The token
  list shipped unpersisted for exactly as long as nobody reopened the database
  after writing one, so `every_resident_collection_round_trips` now walks the
  whole resident state rather than the newest field: adding a collection either
  joins that test or fails it.

- **`spectra refresh`, and two bugs core could not have shown on its own.**

  `BalanceRefreshEngine` is the one subsystem where Rust already owns the loop:
  it holds the timer, fetches, applies the result to Rust-owned state, and calls
  back through `BalanceObserver`. Swift's conformance was the only one, so "the
  engine needs no platform" was an assumption. It holds — a CLI observer sweeps
  three chains and reports three updates.

  *`trigger_immediate` cannot be awaited.* It spawns the cycle and returns,
  which is right for an app that will receive callbacks later and useless to a
  process about to exit: the first run printed "0 refreshed, 0 errors" while
  three fetches were still in flight. Added `refresh_now`, the same cycle
  awaited. Nothing was wrong with the engine; there was simply no way to *use*
  it from anything short-lived, and only a short-lived caller would notice.

  *Core installed a global tracing subscriber on stdout, at `debug`.*
  `WalletService::new_typed` did it in a `OnceLock`, so no caller could opt
  out, and every `spectra --json` run had core's connection logs interleaved
  through the document — `json.load` failing on "Extra data" is how it
  surfaced. A library taking the process's stdout is wrong on its own; at
  `debug` by default it is also loud. Now stderr, and `warn` unless `RUST_LOG`
  says otherwise.

  *And one in the acceptance script:* the new check ran `spectra refresh`
  against the shared directory, which by then had wallets — so the script that
  promises no network swept three chains over it. It has its own empty
  directory now.

- **Seven of core's own self-tests were failing, on fabricated fixtures.**

  `self_tests.rs` is 605 lines of derivation and address checks that only the
  iOS diagnostics screen had ever run. Run from the CLI: **59 of 66 passed**.
  All seven failures were `<chain> Address Validation`, for Bitcoin, Bitcoin
  Cash, Litecoin, Monero, Polkadot, Stellar and Internet Computer.

  The validators were right. The *fixtures* were hand-typed strings that look
  like addresses and carry invalid checksums — Bitcoin's was the BIP-173 test
  vector with the last seven characters wrong (`…c5xw7kygt080` for
  `…c5xw7kv8f3t4`), and Internet Computer's was a principal where an account
  identifier belongs. Same class as the `"SoLaNaAddr"` placeholders found
  earlier, but subtler, because these look real.

  Settled empirically rather than from memory: core rejects the fixture and
  accepts the canonical vector. The replacements are **derived by core** from
  the standard test mnemonic and each one verified through
  `spectra address validate` before use, so they are right by construction
  rather than by typing. 66/66 now.

  `every_self_test_passes` makes the self-tests themselves tested, so the next
  bad fixture fails the build instead of showing a red row on a screen nobody
  reads.

  *Two of my own bugs on the way:* `--json` printed two documents when a
  command reported results and then failed (`CliError::reported` now suppresses
  the second), and the `--chain` filter passed a registry id where core keys
  self-tests by display name — which reported "Bitcoin has no self-tests", a
  sentence that reads as a coverage gap rather than a wrong argument.

- **Tracked tokens were never persisted, and the CLI is how that surfaced.**

  This document claimed they "arrive with the rest of the state". They did not:
  `token_preferences` is a field on `CoreAppState`, and `app_state_save` wrote
  `settings`, `wallets` and the address book and never it. Every launch loaded
  an empty list.

  Nothing caught it because no test reopened the database after tracking a
  token, and the app holds them in memory for the life of a session — so the
  loss only shows on relaunch, and only to a user. `spectra token track`
  followed by `spectra token list` is two processes, so it showed immediately.

  Fixed with a `token_preferences` meta row, pinned by two tests that save and
  reload. The clamp — a token cannot display more places than it has — works
  and now survives the round trip: asking for 99 places on a 6-decimal token
  stores 6.

  *A fixture bug on the way:* the first version of the test helper keyed its
  temp database on the process id alone, so the second test read the first
  one's tokens and passed on data it never wrote. Keyed on thread id too now.

- **`spectra staking` and `spectra token` are the first CLI callers core has
  had for either.** Staking's service — validators, positions, four chains of
  transaction builders — was reachable only from `StakingBridge.swift`, and the
  token list only from Swift's settings screen. Both worked first try, which
  answers whether they were core-owned; the token *persistence* did not, which
  answers whether anyone had checked.

- **One name per chain.** The enum called Internet Computer "ICP" and
  `chains.toml` called it "Internet Computer". The cost was never the name: it
  was a special case in `from_display_name`, an id-keyed lookup in `app_core`
  with a comment explaining why, and the standing question of which spelling a
  call site meant. Both tests walking the full catalog pass, so the rename was
  the only drift — and `every_catalog_name_resolves` means the next one
  fails the build.

### Stage 4 — Android

Only meaningful once the above holds. If Kotlin can be brought up against the
same core without discovering new iOS assumptions, the migration worked.

---

## How progress is measured

Not by feel. These four numbers, checked at the end of each stage:

| Metric | Start | Now | Target |
|---|---|---|---|
| `core_plan_*` exports | 42 | **0** | 0 |
| Swift root lines vs `views/` | 19,766 vs 11,113 | 14,282 vs 10,804 | inverted |
| Domain collections stored on `AppState` | 3 | 0 | 0 |
| Domain settings owned by core | 0 | **21 fields; 4 left on iOS on purpose** | all |
| Wallet operations reachable from the CLI | partial | **all** | all |
| CLI commands drivable without a TTY | 0 of 24 | all (25 now) | all |
| Exported functions and methods | 234 | **180** (99 free + 81 methods) | ~150 (see C2) |
| Largest file in `core/` | 4,781 | 2,501 | — |

*The other unmet row was checked by the same standard and stands.* Inverting
the Swift ratio needs **3,479** lines *deleted* from the root, or **1,739**
*moved* into `views/` — moving counts twice, since it lowers one side and raises
the other. Most of the work so far has been deletion, which is why the number
has barely moved while 5,385 lines have gone. Classifying the root by
role: **6,666 lines are structurally root** — the bridges, the registry
adapters, the `@Observable` shell, the record types, platform and persistence —
and **7,854 are candidates**, every one of them orchestration over core calls.
(Classified at 14,494 root lines; the split has not been re-derived since.)
Reaching the target means moving 22% of that pool, which is demanding but has
no floor under it.

That is the difference from the export target, and the reason one number was
revised and the other was not: there, 41 of 99 free exports had **no merge
partner at all**, a hard floor sitting above the target. Here there is no such
floor.
The last row is the one that makes the others checkable. Every
earlier "proven by the CLI" claim in this document was proven by a person typing
into a prompt. `scripts/cli-acceptance.sh` replaces that with 157 assertions on
exit codes and JSON, over a scratch data directory and with no network.

*What it still cannot see.* The CLI drives core from inside its own Tokio
runtime, so a UniFFI export that needs `async_runtime = "tokio"` and does not
declare it passes every assertion here and fails on the first call from Swift —
which is how the staking tab came to be inert with all three suites green. See
the behaviour change above. Running the app is a fourth gate, not a formality.

Both iOS suites are green as of this pass — 40 tests, 0 failures, over
consecutive full runs. (Forty rather than thirty-six: three keep the staking
tab's editorial copy in step with `Chain::supports_staking` in both directions,
and one walks every EVM mainnet asserting none of them falls back to the generic
address hint.) (Thirty-six rather than thirty-nine: the refresh-planner
tests asserted core's arithmetic through a Swift wrapper, and went with the
wrapper.) (Thirty-eight rather than forty: the diagnostics table
tests that pinned six copies of the chain list against each other went with the
copies.) The
`testEthereumTestNetworksExposeExpectedContextsAndEndpoints` failure this
document told readers to expect is fixed, so a red test is now a real one.

Stage 0 built the mechanism and moved nothing, so the Swift count went *up*.
The address book is the first collection to actually move; the count starts
coming down as the paths it replaced are deleted.

## Rules for new work while this is in progress

0. **Change behaviour and delete functionality when it makes the system simpler
   or more correct** — see Rule 0 at the top of this document. This outranks
   the rules below. Record every such change under "Behaviour changed on
   purpose"; take the stricter side wherever funds, keys or addresses are
   involved.
1. New domain logic goes in `core/`. If it cannot be driven from the CLI, it is
   in the wrong place.
2. New per-chain facts go on `registry::Chain`, not in a `match` in whichever
   module needs them.
3. Do not add a `core_plan_*`. If core needs to decide something, it should own
   the state it decides about.
4. Swift may hold view state freely. It may not hold domain state.
5. Prefer deleting a Swift file over porting it. Much of the root of `swift/`
   exists only to reconcile two copies of the truth; with one copy it is not
   needed.

## Known open items

- **`fetch_token_balances` takes decimals from the caller on every family but
  Tron.** Tron reads the contract's own and reports it; EVM passes
  `token.decimals` straight through without ever calling `decimals()`, and
  Solana takes it from the descriptor although `getTokenAccountsByOwner`
  returns the mint's in the parsed account data it is already fetching. Where
  the two disagree the contract is right, so the catalog's copy is a column
  that can only be wrong. Reading it per family would also let the sending path
  stop needing a catalog entry to know how to denominate a transfer — see the
  Tron decimals entry above for what that assumption cost.

- **Is a dashboard row per asset, or per (chain, asset)?** Today it is the
  latter: `dashboard_asset_grouping_key` includes the chain, so ETH on Ethereum
  and ETH on Arbitrum are separate rows. But the record is named
  `DashboardAssetGroup` and carries `chain_entries`, which reads like
  cross-chain grouping was the intent — and with the chain in the key,
  `chain_entries` can only ever differ by token standard or contract within one
  chain. `a_row_is_per_chain_and_sums_across_wallets` pins the current answer so
  changing it is a decision rather than a regression. Not changed on the way
  past: it is visible product layout, not an inconsistency.


- ~~Six chains claim watch-only support and have nowhere to type an address.~~
  **Fixed**, and it was twenty-two rather than six once the seven-name EVM
  condition and Ethereum Classic's slot were counted — see the behaviour change
  above. The sections are a `ForEach` over the flag now.

- ~~Ten mainnets have no address-format hint.~~ **Closed, and the reasoning in
  this item was the thing that was wrong.** It said filling the gap meant
  authoring Chinese for ten address formats. It did not: the terse form
  ("bc1q…", "r…") is a fact about the chain and is `address_prefix_hint` in
  `chains.toml` now, and the eleven translated sentences are content, so they
  moved into the locale files keyed by chain id with their existing
  translations carried across. A chain with no sentence falls back to a
  template built from its prefix — one string per locale, generalised from the
  sentences already there rather than invented. Coverage went from the eleven
  that had a sentence to every chain that has an example.

  *The lesson is about the word "authoring".* Content that already exists and
  needs relocating is migration. What this item called authoring was the
  reflex of seeing a locale file and stopping.

- ~~`supports_diagnostics` is `true` for all 78 catalog rows and
  `supports_endpoint_catalog` for all but Bitcoin SV.~~ **Both are gone**, and
  the guess in this item was half right. `supports_diagnostics` selected
  nothing: true on every row, and its Swift accessor had no reader at all.
  `supports_endpoint_catalog` did select something, and that was the problem —
  the endpoints settings screen filtered on it, so **Bitcoin SV was hidden from
  a screen whose catalog holds three `whatsonchain` records for it**. A flag
  that claims to describe the catalog and disagrees with it is worse than one
  that selects nothing. The screen asks the catalog now
  (`AppEndpointDirectory.hasEndpoints`), which cannot disagree with itself.

  156 lines out of `chains.toml`, two fields off the `ChainEntry` record that
  crosses the FFI on every row, and two Swift accessors.

- **Which endpoint slot a chain's supplemental explorer endpoints go into is
  still decided in Swift.** `WalletServiceBridge.buildEndpoints` puts
  Polkadot's and Internet Computer's in `.secondary` and fourteen EVM chains'
  in `.explorer`. That is a per-chain fact and belongs on `registry::Chain`; it
  was left where it is because getting it wrong misfiles endpoints rather than
  failing loudly.

- ~~`PrivateKeyImportAddressResolution` still has one field per chain.~~
  **Fixed**, and the record turned out to be the smaller half. The seventeen
  fields and the twenty-arm switch were the third and fourth copies of "which
  chains take a private key"; core held the other two, and all four disagreed —
  see the write-up in Stage C and the behaviour change above. A private-key
  import selects one chain, so the record is one `String?`.

- ~~Decided: the mirror stays; derived state stops being recomputed from the
  projection.~~ **Done** — this item described the plan and the slice landed;
  see "The transaction derived state stopped being ferried to core and back" in
  Stage C. `earliest_transaction_dates` is a `WalletService` method and
  `core_normalized_history_signature` is gone. Kept here in struck-through form
  because the *reasoning* — adopt core's answer rather than redesign the mirror
  — is the shape the next such slice should copy.

  <details>The original text:

  **Decided: the mirror stays; derived state stops being recomputed from the
  projection.** The transaction derived-state slice looked blocked on the
  mirror — Swift holds the projection, the command is async, so core's list is
  stale at the moment `rebuildTransactionDerivedState` runs off the
  projection's `didSet`. Changing the mirror is not the fix and would be a
  large redesign for no gain.

  The fix is the shape `dashboardAssetGroups` already uses: core computes the
  derived state from its own store, Swift *adopts* the answer and caches it as
  view state. `cachedTransactionByID` is an index into the projection and stays
  local; `core_earliest_transaction_dates` and `core_normalize_history` become
  `WalletService` methods that read core's own transactions, and
  `core_normalized_history_signature` — which exists only to decide whether
  recomputing is worth it — disappears, because core can decide that where the
  data is.

  The cost is that the values land a redraw later, which rule 4 already calls
  view state and `dashboardAssetGroups` already accepts. Recorded here rather
  than done, because it belongs to the transaction slice.</details>

- **A state load still overwrites a projection newer than itself.** The
  destructive half is fixed — see "A transaction recorded during launch" above,
  where the load's prune was deleting data. What remains is benign and still
  wrong: `reloadPersistedStateFromSQLite` adopts core's wallet and transaction
  lists over whatever the projection holds, so a wallet recorded while the load
  was in flight briefly disappears from the UI until the next command lands.
  Untouched on purpose: it is the Swift-owns-the-list problem the migration is
  for, and narrowing the window by hand would be a guess about ordering rather
  than a fix.

Carried from the audit, not blocking the stages above but not forgotten:

- ~~Sepolia / Hoodi endpoint records cannot be found by name.~~ **Fixed.** The
  cause was not the data: all eight testnet record sets are written the same
  way, filed under their mainnet's `chainName` with the testnet in
  `groupTitle`. The *index* did not know that convention, which broke both
  directions — the testnet could not be looked up, and the mainnet's RPC list
  silently included the testnet's endpoints. There are two indexes now, because
  two consumers want different answers: per-network for anything that talks to
  a chain, per-chain for the settings screen, which shows a chain and its
  testnets together.
- ~~`EVMChainContext` covers 15 of 23 EVM mainnets.~~ **Fixed** by deleting the
  enum — see Stage 3.
- ~~`scripts/bindgen-ios.sh` and the Xcode "Build Rust Derivation Core" phase
  both regenerate `swift/generated/` and apply *different* Swift 6 patches.~~
  **Fixed.** The Xcode phase calls the script now, so generating and patching
  happen in one place; the phase keeps only what is genuinely its own, the
  per-platform static libraries. Measured before the fix: the script wrote
  `nonisolated` onto 678 declarations and the next Xcode build removed every
  one of them, so which version was on disk depended on which had run last.
  Only the `vtablePtr` patch is needed — the rest was for a UniFFI version this
  project no longer uses, and had been dead for as long as Xcode was the last
  writer.
- ~~`registry::Chain` calls Internet Computer `"ICP"`.~~ **Fixed.** The enum
  says "Internet Computer" now; `coin_symbol` still says "ICP", which is what
  that field is for. The special case in `from_display_name` is gone, and
  `every_catalog_name_resolves` walks all 78 chains — that rename was the
  only disagreement, and there cannot be another one silently.
