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
| Swift, non-generated, excluding tests | 30,879 lines | **24,258** |
| — `views/` + `extensions/` (genuine UI) | 11,113 (36%) | **10,693 (44%)** |
| — root of `swift/` (`AppState`, stores, persistence, bridges) | 19,766 (64%) | **13,558 (56%)** |
| `core_plan_*` FFI exports (core advises, Swift applies) | 42 | **0** |
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

### Stage C — Rewrite core — **started**

Stages 0-3 moved ownership *into* core without reshaping core. It shows: the
crate is 59,276 lines with **290 exported functions**, which is the plainest
statement that it is still a library of helpers rather than a program. A front
end that has 290 ways in does not have to go through the ten that matter.

Measured, not estimated:

| | Start | Now |
|---|---|---|
| Exported functions and methods | 234 | **185** |
| Largest file in `core/` | `service.rs`, 4,781 lines | `store/tests.rs`, 3,418 |
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

  Every branch is `preflight.submitKind` now. What that surfaced: three of the fifteen returned *above*
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

  What belongs here is what it did to the shape. `PersistedAppSettings` was the last typed record
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

- **Rule 0's first two applications.** What belongs here is what they did
  to the structure.

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

**A snapshot surface nobody read, whose one address per wallet came out
twenty-three times.**

*Was:* `swift/Platform.swift` — 165 lines defining `PlatformSnapshotEnvelope`
and six `Codable` structs, plus `makeAddressSnapshots()`,
`makePlatformSnapshotEnvelope` and `exportPlatformSnapshotJSON`.

*Is:* deleted, along with the one test that exercised it
(`testExportsPlatformSnapshotEnvelopeWithStableFoundationModels`) and its three
`project.pbxproj` references.

*Why that side:* it had no production consumer. Every one of its seven types
had zero references outside the file, so the test was the only caller and the
only thing keeping the compiler quiet about it.

The address multiplication is worth recording because it is what made the
surface worth looking at. `makeAddressSnapshots()` said in a comment that every
*slot* the wallet has gets a snapshot and that `WalletChainID` narrows those to
the ones the platform surface knows. Neither half held: it iterated chain
*names*, so a single EVM wallet — which stores one address in the `ethereum`
slot — emitted about twenty-three identical snapshots, one per EVM mainnet; and
the `WalletChainID` filter narrowed nothing, because every name it iterated was
already a `WalletChainID`. The test passed only because Ethereum sorts first in
`Chain.all`, so the first of the twenty-three duplicates happened to be the one
it asserted on.

That reads as an accident, not a design: the comment describes slot
granularity, and slot granularity is what the rest of the app uses. It is moot
either way with the file gone — but if the surface is ever wanted back, it
should be built from `wallet.addresses` (slots) rather than from `Chain.all`.

*How to check:* `grep -rn Platform swift/ --include='*.swift'` finds no
`PlatformSnapshot*` type, and the iOS suite is green at 43 tests.

**A Sui address without its `0x` was refused by the UI and accepted by the
store.**

*Was:* `is_valid_send_address` validated the string it was given.
`normalize_address` trims, lowercases, and for Sui and Aptos prepends `0x`
(`AddressNormalization::LowercaseHexPrefixed`). Whether an address passed
therefore depended on which the caller reached for first — and the two sides
chose differently:

| caller | order |
|---|---|
| `AddAddressBookEntry` in `store::state` | normalize, then validate |
| every Swift caller — the send composer, `canSaveAddressBookEntry` | validate the raw input |

Measured, for a 64-hex Sui address typed without its prefix:

```
validate(raw)         false
normalize             adds 0x
validate(normalized)  true
```

So the composer refused a Sui address the store would have accepted, and the
address-book button stayed disabled for an entry core would have saved. Aptos
happens to agree either way because its validator is the more lenient of the
two; Sui's is not.

*Is:* `is_valid_send_address` normalizes before validating — the order the
authoritative path already used, and the form that gets stored and sent either
way.

*Why that side rather than the stricter-looking one:* refusing the address
would be "stricter" only in the sense of accepting less. It is not safer — the
address is valid, core stores it, and the two answers disagreeing is the actual
hazard. Making the validator answer about the normalized form removes the
question of order rather than picking a winner.

*How to check:* `no_chain_answers_differently_before_and_after_normalising`
runs every mainnet against four inputs — a spaced and mixed-case EVM address, a
bare 64-hex string, a non-address and an empty string — and requires the same
answer before and after normalizing. It is the assertion that would have caught
this when `LowercaseHexPrefixed` was introduced.

*A correction to this document.* The export arithmetic above lists
`is_valid_send_address` + `normalized_send_address` as "one call" and
`core_endpoint_str_id` / `core_resolve_chain_id` as "identity columns". Neither
holds. The first pair is better kept separate now that the ordering hazard is
gone — a caller that wants validity should not have to take a normalization it
will not use. And `core_endpoint_str_id` is not identity: it answers
`"<id>:secondary"` and `"<id>:explorer"` for the other two slots, which is a
string format that would become a second copy the moment Swift built it.
Merging either would cost more than the export it saves.

**The router and the send builder are two tables, and nothing made them
agree.**

*Was:* `route_send_asset` decides in the preflight whether a send is offered;
`build_send_params` turns the request into a signable shape. A chain in the
first and missing from the second passes every check the UI runs — destination
valid, amount affordable, secret present, biometrics cleared — and fails at the
last step, **after the user has authorised it**. Nothing asserted the two
tables covered the same set.

*Is:* `every_routable_mainnet_builds_send_params` walks every mainnet the
router offers and requires the builder to produce params for it. All 46 pass
today, so this pins what is already true rather than fixing a break.

*Why this one and not an end-to-end test:* the send path has no end-to-end
coverage and cannot easily get one here — `cli-acceptance.sh` runs with no
network and the iOS suite does not broadcast. This is the strongest assertion
available offline: not that a send works, but that the two tables a send is
routed through cannot drift apart.

*Checked for vacuity and for redness, because a test that passes on the day it
is written proves neither.* It counts the chains it reached and asserts that
count equals `Chain::mainnets().count()` — otherwise a router that stopped
offering a chain would leave the first assertion holding while checking
nothing. And breaking one builder arm behind a temporary env flag produced:

```
the router offers these sends and the builder cannot build them: [
    "Kaspa (BROKEN)",
]
```

*Still missing, and named here so it stays named:* nothing exercises a
broadcast. `spectra send broadcast` exists and the acceptance run cannot use
it. Until the CLI can drive a send against a testnet, changes to
`AppState+SendExecution.swift` are compiler-checked and audited, not tested —
which is what the near-miss in the entry above cost to catch by hand.

**`submitSend`'s five identical tails became one, and the concurrency guard
stopped racing itself.**

*Was:* nine per-chain branches, each ending the same way — claim the in-flight
flag, `do`, `executeSend`, `recordSuccessfulBroadcast`, `catch`, report. Five
of those tails were identical apart from which preview to clear; Tron's differed
only in how it words an error.

*Is:* `broadcastPreparedSend(holding:wallet:destinationAddress:amount:request:
clearPreview:mapError:onFailure:)`. Each branch now prepares — guards, secrets,
address, fee — and hands over a request. `submitSend` 512 → **457 lines**.

**It also closes a race.** Every branch checked `sendingChains` at its top and
inserted much later, with `await`s in between for a preview refresh and secret
reads. Two sends on one chain could both pass the check before either claimed
the flag; the guard was not guarding. In the shared runner the check and the
insert are adjacent with no suspension between them.

*Two branches kept their own tail, and the reasons are real.* Dogecoin records
a transaction the shared recorder cannot build — a fee rate, a change flag, a
confirmation count and a source address — and Ethereum holds a second flag
(`activeEthereumSendWalletIDs`) plus a nonce that the record carries. Forcing
either through the runner would mean widening it until it was a parameter list
per chain again.

*The near-miss is the point.* Converting Solana in two edits left its own
`sendingChains.insert` in place above a call to a runner that guards on
`!contains` — so **every Solana send would have returned silently, and it
compiled**. Nothing in the three suites covers a broadcast, so nothing would
have caught it. What caught it was checking, for each branch, whether it both
calls the runner and inserts:

```
icp      runner=yes  own-insert=-
bitcoin  runner=yes  own-insert=-
tron     runner=yes  own-insert=-
solana   runner=yes  own-insert=-
near     runner=yes  own-insert=-
dogecoin runner=-    own-insert=yes
ethereum runner=-    own-insert=yes
```

After that, every branch was converted as one whole-block replacement rather
than in pieces.

*What this is not.* The send path has no offline coverage — `cli-acceptance.sh`
runs without network and the iOS suite does not broadcast. This refactor is
compiler-checked and audited, not tested. That is the honest status of the
largest function in `swift/`, and it is the argument for the CLI growing a
`send` path that can be exercised against a testnet.

**A Dogecoin send left the balance stale, and Monero had a branch to carry a
default core already applied.**

Two findings from reading `submitSend`'s six per-chain branches against each
other rather than against the spec.

**Dogecoin skipped the shared post-send routine.** Eight of the nine successful
broadcasts go through `recordSuccessfulBroadcast`, which records the
transaction and then calls `runPostSendRefreshActions` — apply the verification
status, note it, **refresh balances**, run the chain's pending poll, update the
notice. Dogecoin records its own transaction, because its row carries a fee
rate, a change flag, a confirmation count and a source address the shared
recorder does not take, and it hand-rolled the rest: the history refresh, the
pending poll, the notice. It never refreshed balances and never applied a
verification status.

So after sending DOGE the wallet's balance stayed at its pre-send value until
the next scheduled refresh — on the one chain of the nine where that happens.
It now calls `runPostSendRefreshActions` and keeps only the history refresh,
which is the one thing that routine does not do for a UTXO chain.

**Monero's branch was four lines and existed to pass a `2`.** It called
`submitNativeChainSend(..., moneroPriority: 2)`, and `build_send_params` has
always applied `req.monero_priority.unwrap_or(2)` — the same default, written
twice, with a branch and a parameter threaded through the generic function to
carry one of the copies. Monero is in `uses_generic_send_submit` now, the
branch and the parameter are gone, and the call passes `nil`.

That flag needs a shared-path preview to be safe to set, because
`has_send_preview` answers through `!uses_generic_send_submit` for anything
without one. Monero has `SimpleChain::Monero`, so it stays true;
`monero_takes_the_shared_submit_path` asserts all three facts together so the
next chain to join cannot skip the check.

*How to check:* the shared-submit set is eighteen mainnets, and
`monero_takes_the_shared_submit_path` holds its preconditions. The Dogecoin
change is behavioural with no offline assertion — the balance refresh it
restores needs a broadcast to observe.

`submitSend` did **not** get shorter: 509 → 512. Deleting Monero's four-line
branch bought less than the comment explaining what Dogecoin had been skipping,
and that comment is worth more than the four lines. The function is still the
largest in `swift/` and still wants the deliberate pass described above.

**A five-name set decided the post-send refresh, and the registry answers
for twelve.**

*Was:* `AppState.utxoPostSendChains` — `["Bitcoin", "Bitcoin Cash",
"Bitcoin SV", "Litecoin", "Dogecoin"]` — decided whether
`runPostSendRefreshActions` polls for a pending status or refreshes history.

*Is:* `Chain::pending_status_poll`, which answers `Utxo` for those five **and
their seven testnets**, and `EvmReceipt` for the EVM family the same condition
already special-cased separately.

*Why it matters:* a send on Bitcoin Testnet, Testnet4, Signet, Litecoin
Testnet, Bitcoin Cash Testnet, Bitcoin SV Testnet or Dogecoin Testnet ran the
**history** refresh instead of the pending one — so the transaction it had just
broadcast was not polled for confirmation on the path that exists to do exactly
that.

This is the third instance of one shape in this plan: a hand-written list of
mainnets standing next to a registry column that covers the testnets too. The
first made address discovery dead on the same seven chains
(`utxoDiscoveryDerivationChain`); the second was `EVMChainContext` at 15 of 23.
`every_utxo_testnet_polls_the_way_its_mainnet_does` pins the count at twelve
and names each mainnet/testnet pair, so the next chain cannot be added to one
side only.

*Found by asking a different question.* The sweep that turned this up was not
looking for it — it compared what each of `submitSend`'s six per-chain branches
does after a successful broadcast, to see whether they disagreed anywhere they
should not. They mostly did not: Dogecoin looked like it never cleared its
preview and in fact clears it through `resetSendComposerState`, which is
duplication rather than a defect. The one real difference was that only the
Ethereum branch calls `runPostSendRefreshActions` at all — and reading that
function to find out why is what surfaced the list inside it.

*Still true and not acted on:* `submitSend` is 509 lines, the largest function
in `swift/`, and its six branches repeat one skeleton — the `sendingChains`
guard, the `do`/`catch`, the `executeSend` call, the record-and-clear. The
per-chain parts are small and real (a UTXO fee rate, a resource model, a mint,
a resolved ICP account). Collapsing it is a Swift refactor on the funds path
for a line count, which is worth doing deliberately and not as a side effect of
looking for something else.

**Thirty-one declarations in the root that nothing read.**

*Was:* `AppState.swift` and eleven other root files carried constants,
accessors and helpers with no caller left. Most were orphaned by earlier
passes of this plan rather than born dead.

*Is:* deleted. Swift root 13,673 → **13,565**; `AppState.swift` 1,086 → 1,060.

**Nine of them were one thing: the maintenance schedule.**
`activeMaintenancePollSeconds`, `inactiveMaintenancePollSeconds`,
`activePendingRefreshInterval`, `activePriceRefreshInterval`,
`backgroundMaintenanceInterval` and its constrained / low-power / low-battery
variants, and `automaticChainRefreshStalenessInterval`. Core owns that schedule
now — `core_active_maintenance_plan` hands back a `pollSeconds` the app reads —
and the constants it replaced were left behind. A reader would have taken them
for the live values.

`utxoDiscoveryGapLimit` and `utxoDiscoveryMaxIndex` went the same way: they are
`GAP_LIMIT` and `MAX_INDEX` inside `discover_utxo_addresses` now, and
`deriveSeedPhraseAddress` in `AppState+ReceiveFlow` was the derivation the same
move took into core.

*What the scan got wrong, twice, in the same way.* The sweep flagged
`SecureStores`' `loadSecret` / `saveSecret` / `deleteSecret` / `listKeys` and
`WalletBalanceObserver`'s `onRefreshCycleComplete` as unused. They are
implementations of `SecretStore` and `BalanceObserver`, the two
`#[uniffi::export(with_foreign)]` traits **Rust** calls — no Swift caller
exists by design. Any scan for dead Swift has that blind spot; both were kept.

*Left alone on purpose:* six `StaticContentCatalog` fields — `heroTitle`,
`crossChainSectionTitle`, `rpcErrorFormat`, `publicAddressOnlyMessage` and the
two import-method descriptions — are `Codable` `let`s whose keys carry
translated copy in `resources/strings/`. Nothing reads them, which means either
a screen should be showing that copy and does not, or the copy is surplus.
Deleting translated content is not a call to make from a usage count.

*How to check:* the compiler. Every deletion here is a declaration, so a
protocol conformance or key-path use fails the build — which is how the
multi-line signature this first pass mangled was caught. Three suites green.

**Bittensor could not send, because a reason that had expired was never
revisited.**

*Was:* `Chain::uses_generic_send_submit` answered no for Bittensor, and
`route_send_asset` had no row for it, so `submit_kind` was `None` and the
preflight refused. It was the one chain in the "cannot send" list.

*Is:* it routes to `"bittensor"`, has a `SimpleChain` shape, takes the shared
submit path, and the list of chains that cannot send is empty. The assertion
says so directly rather than naming a survivor.

*Why that side, and a correction.* An earlier entry of mine said "nothing
explains why". That was wrong — the explanation was in
`send::tests::every_sendable_chain_has_a_routing_kind_from_the_known_set`,
which is not where I looked:

> Bittensor is genuinely still out: its `execute_send` arm takes no fee
> parameter, it has no shared-path preview, and the generic submit needs a fee
> to validate the balance against. Giving it a fallback would mean inventing a
> TAO fee, which is not this document's to invent.

Sound when written, and **expired**. `Chain::static_fee_units` carries
`Bittensor => 125_000` — the fee had been decided, and nobody came back to the
exclusion that depended on it not existing. `native_fee_estimate` already
answered `("125000", "0.000125", "static")` before this change.

Everything else was already in place: `fetch_simple_chain_send_preview` has no
per-chain arm at all, `execute_send` has had a `SendParams::Bittensor` arm, and
Swift's dispatch checks `usesGenericSubmit` before the `submitKind` switch, so
a chain joining the shared path needs no Swift arm. What was missing was three
registry rows and a preview tag.

*Its own preview tag, not Polkadot's.* The record shape is Polkadot's — same
Substrate extrinsic with fewer fields — but `SendPreview::Bittensor` is a
separate variant because the front end keys its preview slots on the tag, and a
Bittensor preview filed under Polkadot would be shown for the wrong chain.

*The compiler found the rest.* Adding the variant broke four exhaustive
matches in turn — `send::flow`'s projection, `SendPreviewTypes`' two switches,
and the routing-kind set. Each one is a place that would have silently
mishandled the new chain if the enum had been open.

**A second static-fee table, dead and drifted.** `simple_chain_default_fee_raw`
in `send/preview_decode.rs` held one row per `SimpleChain`. It had no caller
outside its own four assertions, and it disagreed with `Chain::static_fee_units`
on **eight of the twelve** chains they shared — Polkadot 160,000,000 against
10,000,000,000, a factor of sixty-two, and Monero 500,000,000 against zero.
Deleted. It is the same two-copies shape this plan keeps finding, caught here
only because adding a chain meant writing a row into both.

*How to check:* `spectra send affordability --chain Bittensor --symbol TAO`
prices a send; the routing-kind test asserts no mainnet is unsendable.

**The endpoint directory moved to TOML, and the settings screen shows what
each endpoint is.**

*Was:* `core/data/AppEndpointDirectory.json` — 166 records beside
`chains.toml`, in a format that cannot hold a comment. And the settings screen
rendered `Text(endpoint)` and nothing else, so six identical-looking URLs under
one chain gave no way to tell the node that answers balances from the indexer
that answers history.

*Is:* `core/data/endpoints.toml`, read the way `chains.toml` already is — a
file-shaped `TomlEndpoint` that converts into the FFI record, so the file may
omit anything empty and the record still has every field. And each settings row
carries a caption: `Node · Balance · Fees · Broadcast`, or `Indexer · History`.

*Why TOML:* the file wanted comments more than any other data in the repo.
Sixteen endpoints were deleted, three replaced and ten re-tagged over the last
two passes, and every one of those decisions had a reason that could only be
written down in `PLAN.md`, a long way from the row it explains. The header now
carries the two vocabularies and the reason `history` needs care; individual
rows carry what was verified and when — `etc.etcdesktop.com` says which dead
endpoint it replaced and that it returns chain id `0x3d`.

*Why a lookup rather than a field:* the settings screen assembles some of its
groups itself — Bitcoin's Esplora bases, and whatever RPC the user typed. Those
have no catalog row. `app_core_endpoint_tag(url)` returns `None` for them and
the row shows the URL alone, which is honest; putting `kind` on
`AppCoreGroupedSettingsEntry` would have forced those call sites to invent one.

*How to check:* `spectra endpoints --chain Ethereum` prints the same
kind/capabilities the screen now shows. Three core tests cover the lookup — a
catalog endpoint reports its kind, a trailing slash is not a different
endpoint, and an unlisted URL has no tag. The four schema assertions from the
pass above still hold against the TOML, which is the real check that the
conversion changed nothing: 549 tests pass on the new file.

**An endpoint's `roles` said what it *is* and what it is *for* in one array,
and forty-six records lied as a result.**

*Was:* one `roles: [...]` per record, mixing two vocabularies — `rpc`,
`explorer`, `backend` (what the endpoint **is**) alongside `read`, `balance`,
`history`, `utxo`, `fee`, `broadcast`, `verification` (what it is **used
for**). Nothing kept the two consistent, because nothing could tell them apart.

*Is:* `kind` — one of `rpc-node`, `indexer`, `web-link`, `backend` — and
`capabilities`, the seven use-for names. `kind` decides how to talk to an
endpoint; `capabilities` decides what to ask it.

*Why that side:* both halves had already drifted, and each cost something real.

- **Ten EVM chains' JSON-RPC nodes were missing the `rpc` marker** while their
  capabilities looked complete, so the diagnostics screen GET a JSON-RPC
  endpoint and reported nine chains unreachable. Fixed in the pass above; this
  is the shape that stops it recurring.
- **Forty-six EVM node records claimed a `history` capability.** No EVM node
  can serve address history: `eth_getTransactionsByAddress` is not a method,
  because a node indexes transactions by block and answering per-address means
  scanning every block ever produced. Verified against a live node — the reply
  is `does not exist/is not available`. The claim went unread (EVM history
  comes from `Chain::evm_history_source`) but it was false, and the next reader
  would have inherited it.

Non-EVM nodes are deliberately untouched: Solana's `getSignaturesForAddress`
and XRP's `account_tx` are real node methods, verified live, so those nodes do
carry `history`. Whether a node can index by address is a protocol fact, not a
provider one, which is exactly why it belongs on the record rather than in a
reader's head.

*Four assertions hold the shape:* an EVM `rpc-node` may not claim `history`;
every record's `kind` is one of the four names; the two vocabularies never
overlap; and a `web-link` claims nothing. The first was checked by putting
`history` back on `ethereum-rpc.publicnode.com` and watching it fail.

*One concept did not survive the split, and was replaced rather than
reinterpreted.* `explorer_supplemental` — endpoints registered alongside a
chain's RPC list — filtered on the `explorer` tag, which four records carried:
`api.etherscan.io`, `api.bscscan.com`, `api.hyperevmscan.io` and
`api.ethplorer.io`. The first three are Etherscan V1 endpoints shut down since,
so the tag was down to one member, while XRP's `xrpscan` and NEAR's
`nearblocks` are the same kind of thing and never carried it. Deriving it from
`kind` instead swept in every settings-visible indexer — twenty chains,
including Bitcoin's nine Esplora endpoints, which are already registered by
their own path. So it is an explicit `supplementsRpcList` boolean on the one
record that means it. A tag describing one member was not describing anything.

*How to check:* `spectra endpoints --chain Ethereum` prints each endpoint's
kind and capabilities — three `rpc-node`s with `read,balance,fee,broadcast`,
two `indexer`s with `read,history`, one `web-link` with none. Two acceptance
assertions (182 now) pin that an EVM node is an `rpc-node` and does not claim
history.

**Sixteen dead RPC endpoints, and no way to find out except opening the
diagnostics screen one chain at a time.**

*Was:* `AppEndpointDirectory.json` is static, and `with_fallback` walks a
chain's endpoints in order — so a dead entry costs a full timeout plus 180 ms
on every call that reaches it, forever, silently. Nothing checked.

*Is:* `spectra endpoints [--chain X]` calls every registered endpoint and
reports which answer, and sixteen dead ones are out of the catalog.

**What was dead** — Ethereum `eth.llamarpc.com` (521) and `rpc.ankr.com/eth`
(now demands a key), Polygon `polygon.llamarpc.com`, Ethereum Classic
`etc.rivet.link` / `besu-at.etc-network.info` / `geth-at.etc-network.info`,
Blast and Aptos's `blastapi.io` hosts, Mantle `1rpc.io/mantle`, Solana
`rpc.ankr.com/solana`, NEAR `near.lava.build` (410 Gone), Tron
`api.trongrid.pro` and `.network`, Polkadot `polkadot.dotters.network` and
`rpc.ibp.network/polkadot`, and Sui's `fullnode.mainnet.sui.io` — that last one
answers **"JSON-RPC on public fullnodes has been deprecated. Please migrate to
gRPC or GraphQL"**, which is a change in Sui rather than an outage.

Polkadot had three RPCs and two of them were dead; the official
`rpc.polkadot.io` was not in the catalog and answers fine, so it is now.
Ethereum Classic and Tron were left on one endpoint each by the removals and
got a verified replacement — `etc.etcdesktop.com` (returns chain id `0x3d`) and
`tron-rpc.publicnode.com`.

*Two probe bugs found by writing the probe:*

**`rpc.ankr.com/eth` answers HTTP 200 with a JSON-RPC error body.** A status-code
probe passes it; the app then gets "Unauthorized: you must authenticate" on
every real call. The probe reads the `error` member, which is the only way to
see it.

**Ten EVM chains' JSON-RPC nodes were missing the `rpc` role** — Sei, Celo,
Cronos, opBNB, zkSync Era, Sonic, Berachain, Unichain, Ink and X Layer carried
`read,balance,history,fee,broadcast` and no `rpc`, while the same kind of
endpoint on other chains carries it. `diagnostics_checks` gates on that role,
so **the app's own diagnostics screen was GETting those ten JSON-RPC endpoints
and reporting them unreachable**. The role is on them now, which fixes the
screen as well as this command. `Chain::rpc_health_method` also grew Solana
(`getHealth`) and Sui (`sui_getLatestCheckpointSequenceNumber`); both had a
dead endpoint that nothing could see, because a chain with no method there is
never probed.

*The probe confirms before it accuses.* Sweeping every chain fires 161
requests, and the first version reported four BNB seeds, Polygon, Hyperliquid
and Ethereum Classic as dead — every one of which answered when asked again on
its own. A failed probe now waits 600 ms and asks a second time. That took the
sweep from 39 reported failures to 10, and the ones that remain are real. A
probe that cries wolf is worse than no probe: it is the one people learn to
scroll past.

*A record with only the `explorer` role is not probed at all.* Those are `/tx/`
links for a person to tap. The first version POSTed JSON-RPC at them.

*Known weak spots, left as they are:* three `probeURL`s answer 404/403 on their
own — `api.blockchair.com/bitcoin-sv/stats`, `rosetta-api.internetcomputer.org/network/list`,
`zec1.trezor.io/api/v2`. The trezor one looks like user-agent blocking rather
than an outage. They are reported honestly as failing their configured probe,
but the probe URL is the more likely thing to be wrong.

*Eleven chains have no RPC failover*: Ethereum Classic, Sei, Celo, Cronos,
opBNB, zkSync Era, Sonic, Berachain, Unichain, Ink and X Layer each have a
single endpoint. That is pre-existing — giving those ten the `rpc` role is what
made it countable.

*How to check:* `spectra endpoints` sweeps every chain; `spectra endpoints
--chain Ethereum` does one. `--json` reports `unreachable` and `unchecked`
counts, and exits `ok: false` when anything is unreachable, so this can run on a
schedule rather than waiting for someone to open a screen.

**EVM history needed an Etherscan key for every chain, and answered "no
transactions" when it did not have one.**

*Was:* `evm_explorer_api_base()` returned `Some("https://api.etherscan.io")`
for all twenty-three EVM chains, and the caller built an Etherscan **V2** URL.
V2 has no keyless tier — with a blank key it answers
`{"status":"0","message":"NOTOK","result":"Missing/Invalid API Key"}`. Both
call sites in `fetch/chains/evm.rs` then did this:

```rust
if resp.status != "1" {
    // Empty history returns status "0" — not an error.
    return Ok(vec![]);
}
```

An empty history *is* `status: "0"`. So is a refusal. **An EVM wallet with no
API key showed "no transactions" — indistinguishable from an address that
genuinely had none**, with no error anywhere. `ApiResp` did not even decode
`message`.

*Is:* two changes.

**The refusal is told apart from the empty result by the shape of `result`.**
An empty history carries an empty array; every refusal carries something else —
Etherscan's `"Missing/Invalid API Key"` string, Blockscout's transient
`"Something went wrong."` with a null. Judged on the type rather than on
`message`, so it does not depend on an explorer's wording. Three tests use the
payloads captured live from `api.etherscan.io` and `base.blockscout.com` rather
than invented ones.

**Where history comes from is a per-chain registry fact**,
`Chain::evm_history_source() -> EvmHistorySource`, replacing the one constant:

| | chains | needs a key |
|---|---|---|
| `Open` — Blockscout | Ethereum, Arbitrum, Optimism, Ethereum Classic, Polygon, Scroll, Celo, zkSync Era, Unichain, Ink | no |
| `Open` — Routescan | Avalanche, Berachain, Blast, Mantle | no |
| `EtherscanV2` | BNB Chain, Sonic, opBNB, Sei, Linea, Hyperliquid, Base | yes |
| `Unavailable` | Cronos, X Layer | no key helps |

**Fourteen of the twenty-three need no API key at all now.** `Open` and
`EtherscanV2` are two request shapes rather than two vendors: Blockscout and
Routescan both serve the Etherscan **V1** query (`{base}/api?module=…`), and
the base already identifies the chain, so no `chainid` and no key are sent.

*How the table was built:* every host was called three times and only ones that
answered all three are in it. Base has a Blockscout instance that answered one
call in three — worse than a source that says so, hence `EtherscanV2` for it.
Cronos and X Layer are in neither Etherscan V2's published chain list nor any
keyless indexer; they were pointed at Etherscan like everything else and have
**never** returned a transaction, which the old code showed as "no
transactions".

*Etherscan V1 is not a fallback for anything.* `api.etherscan.io`,
`api.bscscan.com`, `api.lineascan.build`, `api.sonicscan.org`,
`api.basescan.org` and `api.hyperevmscan.io` all answer "You are using a
deprecated V1 endpoint, switch to Etherscan API V2". It was shut down across
the family at once, key or no key.

**And nothing normalized EVM history even when the fetch worked.**
`normalize_chain_history` had fifteen chain arms and no EVM one, so all
twenty-three fell to `_ => vec![]`. It was invisible while the fetch was also
returning nothing, and only surfaced once the key requirement was gone: the raw
call came back with fifty rows and `fetch_normalized_history` still returned
zero. There is a `c if c.is_evm()` arm now — one guard, not twenty-three names,
because `EvmHistoryEntry` is one shape for the family.

*Three dead records left the endpoint directory.* Its only three entries with
an API endpoint and the `history` role — `api.etherscan.io/api`,
`api.bscscan.com/api`, `api.hyperevmscan.io/api` — were all deprecated V1
addresses. 182 records → 179. Hyperliquid's supplemental-endpoint slot was the
`hyperevmscan` one, so the test that asserted it had a supplement now asserts
the same property through Ethereum's `api.ethplorer.io`, which is the surviving
one.

*How to check:* `spectra --json chains --filter Ethereum` reports
`"historySource":"https://eth.blockscout.com"` and `"needsApiKey":false`; BNB
Chain reports `needsApiKey: true`; Cronos reports `"historySource":"none"`.
Verified end to end against the live network: a watch-only Ethereum wallet's
`spectra history` returns **50 transactions with no API key configured**, where
before it returned 0. Three acceptance assertions (180 now) and seven core
tests.

*Five acceptance assertions were brittle and are fixed on the way.* They
matched adjacent JSON keys (`'"name":"Polygon","privateKeyImport":true'`), so
adding a field between them broke tests that had nothing to do with the change.
They assert one fact each now.

*Considered and deliberately not taken:* every other wallet solves this by
running a backend that holds the key — MetaMask's
`accounts.api.cx.metamask.io/v1/accounts/{addr}/transactions` is exactly that,
is reachable without a key today, covers fourteen chains including BNB, Linea,
Sei, Hyperliquid and Base, and returns native and ERC-20 transfers in one call.
It is infrastructure MetaMask runs for its own client: no agreement, no SLA,
and blockable by user agent or origin at any time. Building a wallet's history
on a competitor's server is a dependency whose failure mode is silent — the
user would see "no transactions" again. If the key is to disappear for the
remaining seven chains, the answer is the same one everyone else reached: a
thin backend of our own.

**Reserving a receive address was five steps in Swift with a window in the
middle.**

*Was:* `refreshUTXOReceiveReservationState` reserved an index, derived its
address, registered it, checked whether it had been used, and if so released
the reservation and took the next one — from the front end, with an `await` at
every step. `reservedReceiveAddress` and `reservedReceiveAddressForDisplay` did
the first three again for the receive screen.

*Is:* `utxo_receive_address(wallet_id, chain_id, reserve)` and
`advance_used_utxo_reservations(chain_id)`. The three Swift methods are shims;
`deriveUTXOAddress`, `utxoDiscoveryDerivationPath`, `hasUTXOOnChainActivity`
and `utxoDiscoveryDerivationChain` are deleted.

*Why that side:* the comment already on the release-and-retake pair said *"both
halves run in core, so nothing else can slip in between and reissue what we
just used"* — and that was true of the two calls it named and of nothing
around them. Between `clearReservedReceiveIndex` and the next
`reserveReceiveIndex` the front end held no lock at all, so a refresh landing
there could hand the same address to two people. All five steps are one call
now, and the floor of 1 — a deep-UTXO chain never issues index 0 as a receive
address — is core's rule rather than an argument each caller passed.

*A shared context, not a repeated one.* The scan resolved the seed and the base
path per index; `UtxoDerivation` resolves them once and `derive(index)` is the
only thing the loops call. Forty indices used to mean forty phrase loads and
forty path resolutions.

*How to check:* `spectra pool next <wallet>` reserves and reports the index —
an assertion now pins it to 1 for a UTXO wallet (177 assertions). A core test
covers every entry point answering empty for a chain without the walk, since
the refresh loop asks for each chain a wallet is on.

*One export came off by itself.* `cli-acceptance.sh`'s unreachable-export check
failed after the Swift deletions: `core_derivation_path_replacing_last_two` had
one Swift caller, `utxoDiscoveryDerivationPath`, and nothing else. It is
`pub(crate)` now — core still builds paths with it. That check is the reason
this was noticed at all rather than left as a dead FFI entry.

*Exports:* 193 → 194 (two methods on, one free function off). Swift root:
13,752 → **13,661**; `AppState+SendFlow.swift` 1,193 → 1,107.

**The address walk ran in Swift because the seed phrase only opened there.**

*Was:* `AppState.discoverUTXOAddresses` plus the four methods under it —
`knownUTXOAddresses`, `deriveUTXOAddress`, `utxoDiscoveryDerivationPath` and
`hasUTXOOnChainActivity` — derived an address per index from the wallet's
phrase and probed each one. Every input it needed was core's except the phrase,
and the phrase was only readable from Swift because the front end owned a
secret layout of its own.

*Is:* `WalletService::known_utxo_addresses` and
`discover_utxo_addresses(wallet_id, chain_id)`. The two Swift methods are
four-line shims. Core reads the seed, the wallet's derivation path, the keypool
bound, the balance and the history — all of which it already held.

*Why split in two:* the walk costs a balance and a history call per index, and
three callers — `StoreHistoryRefresh` twice and `AppState+OperationalTelemetry`
once — want only the addresses already on record. `known_utxo_addresses` is
that answer with no network in it; `discover_utxo_addresses` calls it and then
scans. The Swift original had the same split and it was worth keeping.

**`hasUTXOOnChainActivity` asked one question three ways.** Bitcoin looked at
the UTXO count and a confirmed balance and never at history; Bitcoin Cash,
Bitcoin SV, Litecoin and Dogecoin looked at balance and history and never at
the UTXO count. It is one question — any balance, any UTXO, or any history —
and `utxo_address_has_activity` asks it once for all twelve chains.

**And discovery was dead on every UTXO testnet.** `deriveUTXOAddress` required
both `supportsDeepUTXODiscovery` *and* `utxoDiscoveryDerivationChain`, and the
second was a five-name dictionary in `AppState+ReceiveFlow` while the registry
answers for twelve — the five mainnets **and their seven testnets**. The
shorter list won every time, so the registry said a testnet supported the walk
and the table returned nil. The dictionary is gone; the function asks
`supportsDeepUtxoDiscovery`. A core test pins the count at twelve and names all
five mainnet/testnet pairs, so a sixth chain cannot be added to one and not the
other.

*A sealed wallet does not scan.* `discover_utxo_addresses` reads the phrase
with no password, which is exactly what the previous change made meaningful: a
password-protected wallet has nothing readable here, so the walk returns the
addresses already known and skips the derivation. That is the consequence of
the password becoming a key rather than a gate, and it is stated in the code at
the point where it happens.

*How to check:* `spectra pool discover <wallet>` runs the walk and prints what
it found. Two acceptance assertions (176 now) and two core tests — one that a
chain without the walk answers empty rather than failing, because the refresh
loop asks for every chain a wallet is on.

*Exports:* 191 → 193. Swift root: 13,777 → **13,752**.

*The reservation path followed in the next pass* — see "Reserving a receive
address was five steps in Swift" above.

**One keychain held two secret layouts, and neither side could read the
other's wallets.**

*Was:* the same `SecretStore` delegate, the same `SecretClass::Seed` bucket,
two schemes inside `core/` itself.

| | key | value | written by |
|---|---|---|---|
| `store/mod.rs`'s `SecretMaterialDescriptor` | `wallet.seed.<id>` | the phrase in the clear | the iOS app |
| `store/wallet_secrets.rs`'s `Blob` | `<id>.seed` + `<id>.salt` + `<id>.password` | AES-GCM envelope under a PBKDF2 key | the CLI |

Core computed the *app's* keys in one file, handed them over the FFI as three
`String` fields, and used its own in another. A wallet imported in the app was
invisible to `wallet_secrets`; a wallet imported by the CLI was invisible to the
app.

*Is:* one layout — `wallet_secrets`' — and core does every read and write.
`store_seed_phrase` / `store_private_key` seal when given a password and store
unsealed when not; `load_seed_phrase` / `load_private_key` require the password
exactly when the wallet is sealed. Six `WalletService` methods carry it, and the
three `*_store_key` fields are off `SecretMaterialDescriptor`, so the front end
cannot compute a keychain key at all. `AGENTS.md` already said this was the
intent — *"all secret traffic is driven by Rust"* — and the app had simply never
been moved over.

*Why that side:* core's scheme is the stricter one and it was already written.
The app's password was **not** encrypting anything: it stored a verifier beside
a plaintext phrase, so it gated the reveal screen and nothing else, and anything
that could read the keychain could read the seed whether or not a password was
set. Under core's scheme the password *is* the key, so a wrong one cannot
produce a phrase rather than merely failing a check.

**The state core was missing was the one the app used most.** `seal` required a
non-empty password, so a wallet without one had no home in `wallet_secrets` —
which is why the front end grew a layout of its own rather than a bug being
noticed. Sealed and unsealed are told apart by the **verifier's presence**,
never by looking at the seed value: identifying key material by its shape is
how a corrupt read becomes a wrong answer. Storing unsealed deletes any salt and
verifier, because a stale verifier would make `is_sealed` claim material is
encrypted when it is not, and `is_sealed` moved from the envelope to the
verifier for the same reason — both states now write a seed blob.

Supplying a password for an unsealed wallet is `PasswordNotRequired` rather
than something to ignore: a caller that thinks it is unlocking something has a
wrong idea of what it is holding.

*What changed for a user:* a password-protected wallet's seed now genuinely
needs the password. Background work that reads a phrase — the UTXO address
discovery loop — gets `nil` for such a wallet until it is unlocked, where before
it read the plaintext regardless. That is the point of the change rather than a
side effect of it, and it is the behaviour a user with a password already
believed they had.

*Deleted on the way:* Swift's `SecureSeedPasswordStore` (25 lines) had no
callers left once core owned the verifier — core files salt and verifier under
`SecretClass::Generic`. Its two FFI exports, `create_password_verifier` and
`verify_password_verifier`, existed only to serve it; both doc comments named it
by name. Gone, with two `SecureSeedStoreTests` cases. `SecureSeedStore` and
`SecurePrivateKeyStore` stay, now purely as the adapter's backend for the two
buckets — which is the layering that was wanted.

*How to check:* `spectra wallet import --no-password` stores unsealed, and
`wallet export` on it asks for nothing while the sealed wallet beside it still
refuses a wrong password. Seven acceptance assertions (174 now) and five core
tests, including that adding a password replaces the cleartext blob and that
dropping one clears the verifier rather than leaving a lie behind.

*Cost:* exports 187 → 191. Six new `WalletService` methods, two deleted free
functions, and three fields off a record. That is the wrong direction for the
export target and the right one for this plan: the alternative is a front end
that computes keychain keys.

*Still to do:* the UTXO discovery loop can now move into core, which is what
this unblocks. `wallet_seed_phrase` is exported for the reveal path and for that
loop's remaining Swift caller; when the loop moves, no phrase crosses the FFI
for derivation at all.

**Every EVM pending refresh recursed until the process died.**

*Was:* `AppState.refreshPendingTransactions(chainName:)` dispatches on
`Chain::pending_status_poll`. Its `.evmReceipt` arm was

```swift
case .evmReceipt:
    await refreshPendingTransactions(chainName: chainName)
```

— the function containing it, called with the argument it was given. There is
only one declaration of that name taking `chainName:`, and `pending_status_poll`
is a pure function of the chain, so nothing between the entry and the recursive
call can change the outcome. No path terminates.

*Is:* a real receipt poll. `WalletService::evm_transaction_status(chain_id,
tx_hash)` fetches the receipt and returns an `EvmReceiptClassification`;
`refreshPendingEVMChainTransactions` walks the chain's pending sends and
forwards each outcome to core's poll schedule.

*Why it matters more than the shape of it:* `pending_status_poll` answers
`EvmReceipt` for every EVM chain — the `other if other.is_evm()` arm, so all
twenty-three mainnets and their testnets — and `ChainRefreshDescriptors` gives
every chain an `executePendingOnly`, with `executeRefresh` also calling it
unconditionally. So this was on the routine refresh path, not a corner: **an
EVM balance refresh could not complete.** Confirmed rather than reasoned: with
the arm put back, the test below does not fail, it takes the test runner down
with it — `Restarting after unexpected exit, crash, or test timeout`.

*Where it came from.* Beta Commit 113, the one that collapsed eighteen
per-chain pending wrappers into this registry switch. It deleted
`refreshPendingEVMTransactions` from `AppState+BalanceRefresh.swift` and wired
the arm to the enclosing function instead of to its replacement. The two
sibling arms got real callees; this one got the name of the switch.

*Why a receipt and not the history summary:* the `.historyTxids` sibling asks
whether the hash appeared in history, and that cannot tell a mined-and-reverted
send from a successful one — a revert would read as a confirmation. A receipt
carries `status: "0x0"`, so a reverted send resolves to `.failed`. That is the
stricter side, and it is the reason this arm existed separately at all.

A `classify_evm_receipt_json` sat next to the new method in `send/flow.rs`,
taking the receipt as a JSON string and re-parsing it for three fields core had
already decoded. It had no callers in `core/`, `cli/` or `swift/`. Deleted; the
projection is direct.

*How to check:* `testAnEVMPendingRefreshTerminates` starts an empty store, runs
the refresh for Ethereum under a five-second deadline and asserts it returned.
With no wallets the fixed code stops at the first guard in 0.1s; the old code
never reached that guard. iOS suite 43 → 44. This one *added* 50 lines to the
root rather than removing them — the poll it replaced had been deleted, so the
ratio went the wrong way on purpose.

**The fee half of "can this send land" was on `AppState`.**

*Was:* `route_send_asset`'s preflight refuses `amount > available_balance` and
stops there. The other half — the fee, which comes out of the chain's gas asset
and so, for a token send, out of a different balance entirely — was
`AppState.validateSendBalance`: nine parameters in, a formatted sentence out,
four callers.

*Is:* `send_affordability(SendAffordabilityInput)` returns a
`SendAffordability` enum. Six inputs, and four of the nine are gone because
they were registry facts the callers were passing by hand.

*Why that side:* the four callers each decided *"is this the chain's own
asset"* themselves, and spelled it four ways — `holding.symbol == "TRX"`,
`holding.symbol == "SOL"`, a literal `true`, and `preflight.isNativeEvmAsset`.
That is the `gas_token_symbol` question, and it is the same pair that the
destination probe's five-symbol list got wrong in both directions: Arbitrum's
catalog `symbol` is ARB and its `gas_token_symbol` is ETH, so anything reading
the wrong column checks an ARB send's fee against the ARB balance. Two of the
four also wrote `feeDecimals: 6` rather than asking `send_execution_shape()`,
which is 8 on the UTXO chains and 7 on Stellar. Naming the chain is the whole
input now.

One message went with it. The token-fee refusal had two spellings — "to cover
%@ network fee" when the caller passed a chain label and "to cover the network
fee" when it passed `nil` — and which one a user saw depended on which of the
four callers asked, not on anything about the send. Core names the chain on
every token verdict, so the unlabelled template is deleted from all three
locale files.

An unresolvable chain takes the native path rather than answering
`Affordable`: that path needs no gas symbol and still refuses amount + fee over
the balance, which is the stricter side of not being able to name the asset.

*How to check:* `spectra send affordability --chain <c> --symbol <s> --amount
<a> --fee <f> --balance <b> [--gas-balance <g>]`. Six assertions in
`cli-acceptance.sh` (167 now), including that ARB on Arbitrum is a token send
whose fee is quoted in ETH; five core tests, one of which is that same
governance-token case. The assertion is not vacuous — the catalog really does
carry ARB in `symbol` and ETH in `gas_token_symbol` for that row.

*Exports:* 185 → 186. `send_affordability` is a new free function with no merge
partner; it buys four registry lookups back from the call sites. Swift root:
13,739 → **13,721**.

*Not taken, and why.* Two other candidates in this file were looked at and left:

- **The UTXO discovery block (~340 lines) is blocked on a real fork.** Moving
  `discoverUTXOAddresses` into core means core reading the wallet's seed, and
  **Swift and core do not store seeds the same way**. Swift reads a raw
  Keychain value at `cachedSecretDescriptorsByWalletID[id].seedPhraseStoreKey`;
  core's `wallet_secrets` writes a salt / verifier / envelope triple under its
  own layout and `unlock` needs a password. Both use the same `SecretStore`
  delegate, and neither can read the other's wallets. That is a slice worth
  taking — it is the last thing keeping address derivation in Swift — but it is
  key material, it wants its own pass, and prelaunch means it can be settled by
  picking one layout rather than bridging two. Filed under Known open items.
- **The address-book pre-check is not the same kind of duplication.**
  `canSaveAddressBookEntry` does repeat core's three rules, but core re-checks
  and refuses authoritatively; the Swift copy only disables a button, which
  rule 4 calls view state. Making it authoritative would mean an `await` inside
  a SwiftUI `.disabled(...)`, so the honest version is view-state plumbing for
  no change in behaviour. Left alone deliberately.

**The recipient warning was four chain arms wording one verdict three ways.**

*Was:* `AppState.refreshSendDestinationRiskWarning` — 163 lines in
`AppState+SendFlow.swift` — switched on chain name into four arms. Bitcoin
fetched a native balance summary; "every EVM chain" fetched the address probe
and then branched again on gas token vs catalog token; Tron fetched a balance,
a history summary and possibly a token balance; everything else went through a
private `fetchChainRiskWarning`. Each arm then built its own sentence.

*Is:* `WalletService::send_destination_risk(chain_id, address, token)` returns
`SendDestinationRisk { balance_is_zero, has_history }` and nothing else. Swift
picks the token descriptor — which contract a symbol means is a catalog
question it already answers with `supportedToken(for:)` — awaits once, and
renders one sentence pair. 142 lines out of the root, 44 back in.

*Why that side:* three of the four arms were the same two fetches with
different spellings, and the differences between them were not decisions
anyone had made:

- **Two of the three sentence templates were interpolated in Swift**, so they
  never reached `RuntimeStrings.*.json`. A Tron send, or an EVM token send, showed
  an English warning inside a Chinese app. There is one template pair now, both
  localized, and both name the asset — which the two interpolated ones did and
  the localized one did not, so no information is lost by merging onto it.
- **Bitcoin asked `utxo_count > 0` for "has history"**, which is a different
  question. An address that received and later spent everything has history and
  no UTXOs, and got the warning that says in so many words that it has "no
  transaction history" — a claim the app could see was false. The history
  signal is `entry_count > 0` for every chain now, with the EVM nonce OR-ed in
  because the balance probe returns it anyway and it needs no explorer key.
- **The three non-EVM arms probed the chain's own asset regardless of what was
  being sent.** Sending USDC on a chain in the `default` arm checked the
  destination's *gas token* balance and reported it as though it were the
  asset. Core now takes the token or `None`, and an asset with no catalog entry
  gets no check rather than a check of the wrong thing — which is what the EVM
  arm already did.

An unresolvable chain is an error rather than a clean verdict. The `default`
arm answered "no warning" for anything it could not resolve, which looks
identical to a destination that passed; the caller has to be able to tell that
the question was not asked.

*How to check:* `spectra send probe --chain <c> --address <a>` runs it, with
`--contract/--symbol/--decimals` for a token. Three offline assertions in
`cli-acceptance.sh` (161 now) cover the refusals; the verdict itself is two
network reads and is not assertable there. One core test
(`an_unknown_chain_is_an_error_and_not_a_clean_verdict`) pins the refusal.

*Exports:* net zero, deliberately. `send_destination_risk` is new on the FFI,
and `fetch_evm_address_probe` came off it — with core asking it, Swift no
longer does, so it moved to the plain-impl block and its bridge wrapper is
deleted. `EvmAddressProbe` is no longer a `uniffi::Record`; it stops at the
crate boundary. Swift root: 13,837 → **13,739**.

**Comments that only restate the line under them.**

*Was:* the house style here is *why*, and what the code used to be. A handful
of comments broke it by describing the present, which the code already shows.

- `service/network.rs` carried three `// ── ` section headers over an empty
  region — "Bitcoin HD — seed → account xpub derivation", "Price / fiat rate
  service" and "EVM receipt polling". The first two name work that lives seven
  hundred lines further down in the plain-impl block, which has no dividers of
  its own; the third names work this file does not do at all — `receipt` did
  not appear anywhere else in it. Deleted. The neighbouring headers that do
  have a section, and the `… lives in the plain-impl block below` pointers,
  are left as they are.
- `fetch/chains/evm.rs` had `// base * 110 / 100, saturating.` directly above
  `base.saturating_mul(110) / 100`, under a doc comment already saying "+10%
  (the minimum EIP-1559 replacement rule)". Deleted.
- `fetch/chains/monero.rs` had `// Sort by timestamp descending.` over
  `entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp))`. Deleted.
- `AppState.networkChainID(forFamily:)` was documented "The chain id this
  family is on", which is the signature read aloud. It now says the part the
  signature does not: a family with no selection reports itself, so the
  mainnet id is the default without being stored as one.

*Why that side:* a comment that restates its line costs a reader the time to
check whether it says anything, every time. A section header over an empty
region costs more than that — it is a claim about how the file is organized,
and it was wrong.

*Not changed, on purpose:* two things that look like this class and are not.
The byte-layout markers in `send/chains/` (`// nonce: u64`, `// asset`,
`// Header.`) name wire-format fields, so they describe the spec rather than
the code. And `store/wallet_db.rs` puts a one-line doc on every public
function, most of them a restatement of the name — that is uniform across the
file, and for a thin SQL layer "what it does" is the whole contract, with no
why to give. Deleting one of twenty would only make the file less predictable.

**Comments that named code which had moved or gone.**

*Was:* five comments describing a version of the code that no longer exists —
found by checking every comment that makes a mechanically verifiable claim
(a count, or a `lives in` / `projects into` pointer at a named symbol).

- `formatting.rs` said `chains.toml` "carries `native_decimals` on all
  seventy-eight rows". Since the catalog split, forty-six chain rows carry the
  column and thirty-two network rows inherit it from their mainnet. The
  *catalog* still answers for all seventy-eight; the *file* does not.
- `service/mod.rs` pointed at `fetch_balance`, deleted with the JSON balance
  shuttle.
- `service/network.rs` pointed at `sign_and_send` / `sign_and_send_token`,
  which are now one `sign_and_broadcast_send`.
- `service/network.rs` called `fetch_bitcoin_xpub_balance` a "JSON shuttle". It
  is `bitcoin_xpub_balance` now and returns a typed `HdXpubBalance`.
- `send/ethereum.rs` said `EvmSendOverridesInput` is projected by
  `build_execute_send_payload` into a JSON fragment for
  `build_evm_*_send_payload`. All three names are gone; `evm_send_overrides`
  builds an `EvmSendOverrides` value.

*Is:* each rewritten to describe the code that is there.

*Why that side:* a pointer comment that names a deleted symbol is worse than no
comment — it sends a reader looking for something that cannot be found, and it
is the kind of error that compounds, because the next person to move that code
has no way to tell the stale pointer from the live ones.

*How to check:* nothing in `core/src` or `swift/` outside a "was" clause names
a symbol that `cargo build --release` cannot resolve. Two classes deliberately
survive: comments in the past tense recording what a function *used to* do
(`send_execution.rs` keeps several, and they are the record of the JSON
boundary coming out), and byte-layout comments in `send/chains/` that restate
the line below them — `// nonce: u64` above a `to_le_bytes()` call is the wire
format, not a description of the code.

There are no `TODO`, `FIXME`, `HACK` or `XXX` comments anywhere in `core/`,
`cli/` or `swift/`.


### Stage 4 — Android

Only meaningful once the above holds. If Kotlin can be brought up against the
same core without discovering new iOS assumptions, the migration worked.

---

## Behaviour changed on purpose

Cleared. Rule 0 still requires every behaviour change to be recorded here —
what it was, what it is, why that side, and how to check it from the CLI.
New entries go below.

---

## How progress is measured

Not by feel. These four numbers, checked at the end of each stage:

| Metric | Start | Now | Target |
|---|---|---|---|
| `core_plan_*` exports | 42 | **0** | 0 |
| Swift root lines vs `views/` | 19,766 vs 11,113 | **13,515 vs 10,542** | inverted |
| Domain collections stored on `AppState` | 3 | 0 | 0 |
| Domain settings owned by core | 0 | **21 fields; 4 left on iOS on purpose** | all |
| Wallet operations reachable from the CLI | partial | **all** | all |
| CLI commands drivable without a TTY | 0 of 24 | all (25 now) | all |
| Exported functions and methods | 234 | **196** (95 free + 101 methods) | ~150 (see C2) |
| Largest file in `core/` | 4,781 | 3,434 (`store/tests.rs`); largest non-test 2,315 | — |

*Re-measured, and two rows moved the wrong way.* The previous pass recorded
13,558 vs 10,693 and 194 exports; both were right when written.

**The Swift gap grew, from 2,865 to 2,973.** The root fell 43 lines and
`views/` fell 151 — deleting from `views/` counts against this metric exactly
as adding to the root does, and the recent passes have been deleting from both.
Nothing is wrong with the deletions; the number simply does not measure them.
It measures a ratio, and the only two things that move it are deleting from the
root and moving root code into `views/`.

**Exports went 194 → 196.** `send_affordability` and `app_core_endpoint_tag`
were added this stretch, each replacing a rule that had been re-derived on the
Swift side. That is the trade this metric cannot see: an export that removes a
Swift copy is a win the count records as a loss. It is still the right target —
the point of ~150 is that the boundary should shrink — but a pass that adds a
correct export and deletes the duplicate it replaces should say so rather than
look like a regression.

*The export row is counted by `scripts/count-exports.sh`,* which states the
definition so the number is reproducible: a free function carrying
`#[uniffi::export]`, plus every `pub fn` inside an `#[uniffi::export] impl`
block — UniFFI exports all of them whether or not anyone calls them. The 180
recorded before was counted by hand and by a narrower rule, which is why it did
not match; the script's number is the one to compare against from here.

*The other unmet row was checked by the same standard and stands.* Inverting
the Swift ratio needs **2,865** lines *deleted* from the root, or **1,433**
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
into a prompt. `scripts/cli-acceptance.sh` replaces that with 182 assertions on
exit codes and JSON, over a scratch data directory and with no network.

*What it still cannot see.* The CLI drives core from inside its own Tokio
runtime, so a UniFFI export that needs `async_runtime = "tokio"` and does not
declare it passes every assertion here and fails on the first call from Swift —
which is how the staking tab came to be inert with all three suites green. See
the behaviour change above. Running the app is a fourth gate, not a formality.

Both iOS suites are green as of this pass — 42 test methods in `swift/tests`,
43 executed across the two bundles, 0 failures, over consecutive full runs.
(Forty-two rather than thirty-six: three keep the staking
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

## Queued: features with a more elegant shape

### The wiki becomes an asset wiki

The chain wiki is the only screen in the app organised by chain; everything else
— the dashboard row, the pin list, holdings — is organised by asset. A user who
taps their USDC and wants to know what USDC is has nowhere to go: there is a
page for Base and none for USDC.

Half of it is already built and gated on the wrong thing. `AssetContractsCard`
in `DashboardViews.swift` already renders chain / standard / contract per asset
— but only inside `AssetGroupDetailView`, so only for assets the user *holds*.
The chain wiki has the editorial content and no contract table; the asset detail
has the contract table and no editorial content. This joins them.

It is also the reference side of a safety property this plan already took: an
unvouched token is shown by contract because a deployer cannot forge one, and a
per-asset page listing the vouched contract on every chain is where a holder
checks the one they were given.

Scale: 46 chain pages become ~66 asset pages — 36 distinct native assets (ETH is
native on **ten** chains, BNB on two) plus 31 token assets. The 31 already have
`comment`.

1. ~~**`chain-wiki.toml` → `crypto-wiki.toml`, one row per asset.**~~ **Done.**
2. ~~**One asset table in core.**~~ **Done** — `core/src/wiki.rs`.
3. ~~**Chain pages stay, one level down.**~~ **Done.**
4. ~~**Swift: lift `AssetContractsCard` out of the detail view.**~~ **Done.**

### The sweep after it

A survey for inefficiency and for names that lie, run once the wiki work
landed. Five findings, ordered by cost.

All five are done.

1. ~~Four per-render FFI scans.~~ Routed through `CachedCoreHelpers`.
2. ~~`cachesRevision`: twelve writers, zero readers.~~ Deleted.
3. ~~`displayColor(for:)`'s four hardcoded symbols.~~ Deleted.
4. ~~Four functions with no callers.~~ Three, and they are deleted; the fourth
   was a miscount.
5. ~~Two names that lie.~~ Split and moved.

*Prerequisite, done:* CRO carried two market-data ids, which would have made it
two entries in an asset-keyed wiki — see "CRO was priced as a different coin"
above.

The five entries this section previously held are all done.

The survey that produced them also read and left alone:
`AppEndpointDirectory.json` (2,813 lines over 182 records — real data, several
providers per chain with roles and probe URLs, not duplication); Tor (sixteen
references and a proxy switch); `receive_address_resolver_kind`'s dispatch table
(each arm maps to a *different* resolver — an enumeration, not a subset); the
eight-chain "popular" list on the setup screen (curation, not a registry fact);
and the two alert systems (per-holding absolute price alerts, global
percent/USD movement alerts) — one rule engine with an absolute-or-relative
condition would cover both, but they differ in scope as well as in condition,
so the case was weaker than the five and it was not queued.

## Known open items

- ~~**Swift and core store seeds in two different layouts, and neither can read
  the other's wallets.**~~ **Fixed.** The original text: Swift's `storedSeedPhrase(for:)` reads a raw phrase
  from the Keychain under
  `cachedSecretDescriptorsByWalletID[id]?.seedPhraseStoreKey`. Core's
  `store::wallet_secrets` writes three blobs — salt, password verifier,
  encrypted envelope — under a key it derives from `wallet_id` itself, and
  `unlock` needs the password. Both go through the same `SecretStore`
  delegate, so this is one keychain holding two schemes.

  It is what keeps UTXO address derivation and discovery in Swift: about 340
  lines in `AppState+SendFlow.swift` that derive an address per index and probe
  each one, all of which core has the pieces for except the seed. Prelaunch
  means the fix is to pick one layout outright rather than bridge two. Take the
  stricter side when picking: core's is sealed and Swift's is not.

- ~~**Bittensor is excluded from the shared submit path, and nothing explains
  why.**~~ **Fixed**, and the second half of that title was wrong — the
  explanation was in a test comment, and it had expired. The original text: `Chain::uses_generic_send_submit` answers yes for sixteen mainnets.
  Bittensor is one of seven non-EVM chains that answer no, and it is the only
  one of the seven without a reason: the other six need a UTXO selection
  (Bitcoin, Dogecoin), a resolved source account (Internet Computer), a
  resource model (Tron), a mint account (Solana), or a view key and a backend
  (Monero). Bittensor's `SendParams` arm is `{from, to, rao, private_key_hex,
  public_key_hex}` — strictly *fewer* fields than Polkadot's, which adds `era`
  and `tip` and does take the shared path.

  Not changed here, because the flag is on a funds path and flipping it is not
  a no-op: `has_send_preview` currently returns true for Bittensor through its
  `!uses_generic_send_submit()` branch, and Bittensor is not in
  `simple_preview_chain`, so a flip would make its preview depend on
  `send_execution_shape().fee_fallback > 0.0` — the exact condition
  `send::mod`'s shared-path test asserts for the other sixteen. Whoever picks
  this up should check that fallback first, then flip the flag and let that
  test cover it.

- ~~`fetch_token_balances` takes decimals from the caller on every family but
  Tron.~~ **Fixed** — see "A balance's decimals are the contract's" below. The
  original text: Tron reads the contract's own and reports it; EVM passes
  `token.decimals` straight through without ever calling `decimals()`, and
  Solana takes it from the descriptor although `getTokenAccountsByOwner`
  returns the mint's in the parsed account data it is already fetching. Where
  the two disagree the contract is right, so the catalog's copy is a column
  that can only be wrong. Reading it per family would also let the sending path
  stop needing a catalog entry to know how to denominate a transfer — see the
  Tron decimals entry above for what that assumption cost.

- ~~Is a dashboard row per asset, or per (chain, asset)?~~ **Answered: per
  asset.** ETH on Ethereum and ETH on Arbitrum are one row with a per-chain
  breakdown; see "A dashboard row is an asset, not an asset on a chain" below.
  The layout question was the only thing left open here — the shape problem
  underneath it (a `chain_entries` vector that could only ever hold one thing,
  and three fields derived from it) was fixed first, in the entry above that.


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

- ~~Which endpoint slot a chain's supplemental explorer endpoints go into is
  still decided in Swift.~~ **Fixed** — `Chain::supplemental_endpoint_slot()`.
  The table also turned out to be wrong in both directions: twelve of its
  sixteen names had no supplement to register, and Hyperliquid, which has one,
  was not in it. See "A balance's decimals are the contract's" above.

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

- ~~A state load still overwrites a projection newer than itself.~~ **Fixed**
  — see "The launch load dropped a wallet imported while it was reading" below.
  It was not "briefly, until the next command lands" as this entry claimed: the
  wallet stayed missing until the next launch. The underlying
  Swift-owns-the-list problem is still what the migration is for; what is fixed
  is that losing the race no longer loses data from the screen.

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

### Comment audit — what a comment has to earn

`core/Cargo.toml` was the trigger: 124 lines of which 39 were comments
restating the crate name on the line below, and three that pointed at files
that do not exist. A repo-wide pass followed, on one rule:

> A comment earns its place when it says something the code cannot, **and a
> reader could act on it.** A comment that narrates a completed move — "this
> used to live in X", "no longer exported" — names something unreachable and
> unfindable, so there is nothing to act on. A comment that names the concrete
> failure the old shape caused is the opposite: it is the only thing stopping
> the next person from writing it again.

Most of the ~155 "used to" comments in the repo pass that test and were kept.
What did not:

- **Tombstones.** `core_derivation_path_replacing_last_two`,
  `SimpleChain`, `store/mod.rs`'s transaction builders,
  `fetch_evm_address_probe`, `wiki.rs`'s prose loader,
  `advanceHistoryPage`, `AppStateTypes.swift` — each carried a paragraph
  about where it had moved from. Trimmed to what is true now.
- **A comment about a comment.** `AppState+CoreStateStore.swift` explained
  that its own previous version had said the opposite. The current claim
  stands on its own.
- **Merge scars.** Six `── Merged from <file> ──` dividers naming files that
  no longer exist (`app_core_derivation_paths.rs`, `flow_helpers.rs`,
  `ChainBackendModels.swift`, …). Replaced by headers that describe the
  section, or deleted.
- **Counts.** `preview_decode.rs`'s header said "the 11 simple-fee chains" and
  listed them; Bittensor made it 12 the same week. It names
  `Chain::simple_preview_chain` now — the registry can be asked, a header
  cannot.
- **Restatements.** `// Load one wallet by id.` over `wallet_load(db_path,
  wallet_id)`; `// RFC 4648 base32 encode without padding` over a one-line
  wrapper, under a section header that already said "base32"; a test comment
  identical to the test's own name.

**Six were not noise but wrong**, which is why this was worth doing:

| Comment said | Actually |
|---|---|
| `icp.rs`: "the `pubkey_der_to_icp_address` encoder below is preserved for callers that…" | No such function, in that file or any other |
| `wallet_domain.rs`: "Swift `WalletRustSecretMaterialDescriptor`. JSON keys preserved for decode compat" | No such Swift type; Swift uses the UniFFI record. The `serde` renames satisfy only their own roundtrip test |
| `SendPreviewTypes.swift`: "The named statics are kept so the existing `EVMChainContext.arbitrum` call sites read the same" | `EVMChainContext` has no statics and no such call site |
| `SendPreviewTypes.swift`: "UniFFI-generated from Rust (`core/src/wallet_core.rs`)" | No such file |
| `StakingView.swift` ×2: "`everyStakingChainHasADescriptor` is what keeps the two in step" | The test is `testEveryStakingChainHasADescriptor`; the name as written finds nothing |
| `WalletServiceBridge.swift`, `AppState+ReceiveFlow.swift`: "…is `coreIsEvmChain`" | Not exported to Swift; both call sites read `Chain.isEVM` |

**And three contradicted each other about who owns the wallet list.**
`AppState+CoreStateStore.swift` says core owns it and direct assignment to
`self.wallets` is a bug — which the code backs up: `wallets` is
`private(set)` with one writer. `refresh/engine.rs` and
`WalletBalanceObserver.swift` both still said Swift owns it and Rust does not
mirror wallet state. A reader trusting either of those would write the bug
the third one warns about. Both corrected.

### Comment audit, second pass — the counts

The `preview_decode.rs` header ("the 11 simple-fee chains") was not the only
count that had drifted. Every numeric claim about the registry was checked
against the registry:

| Claim | Real | |
|---|---|---|
| `formatting.rs`: seventy-eight entries, forty-six chain rows, thirty-two networks | 78 / 46 / 32 | ✅ |
| `registry.rs`, `send/mod.rs`, `send/ethereum.rs`, `store/tests.rs`, `EndpointsViews.swift`: twenty-three EVM mainnets | 23 | ✅ |
| `AppState.swift`: "All 12 EVM chains" host known tokens | 12 of `CoreTokenHostingChain` | ✅ |
| `registry.rs`: "Sixteen of the forty-six mainnets" take the shared submit path | **18** | ❌ |

The last one had gone stale in this session's own work: Bittensor and Monero
joined `uses_generic_send_submit`, and the doc above it still listed both as
chains that *cannot* take the shared path — Monero "needs a view key and a
backend", Bittensor "is here without a reason of that kind, see Known open
items in `PLAN.md`". That open item is struck through; the pointer outlived it.

Rewritten without a count: it now describes the chains that answer **no** (the
EVM family, plus Bitcoin, Dogecoin, Internet Computer, Tron and Solana, each
with the reason it is out), which is the short side and the side that changes
for a reason rather than by accretion.

Also fixed in `AppState.swift`: `enabledKnownTokens` carried two stacked
summary lines from two different doc comments, the second starting mid-sentence
("Built a `ChainTokenRegistryEntry` —") with no subject.

**Nothing found in two other categories**, which is worth recording so the next
audit skips them: no commented-out code anywhere in `core/`, `cli/`, `ffi/` or
`swift/`, and no `TODO`/`FIXME`/`HACK` debris.

### Known open item — a dead Swift file

`swift/LoadingTaskRegistry.swift` is 63 lines, compiled into the app by
`project.pbxproj`, and called by nothing. Its header was a migration memo with
an "Audit pass" checklist of `is…ing` flags to convert, one of which
(`isPreparingEthereumSend`) no longer exists. The stale checklist is gone and
the header now says plainly that nothing calls the class.

Deleting the file means editing `project.pbxproj`, which is a different change
from a comment pass — left for a deliberate one. It is the kind of thing rule 5
covers: prefer deleting a Swift file over keeping it.

### Comment audit, third pass — what a repeated comment was pointing at

Scanning for **comment blocks that appear verbatim in more than one file** was
meant to find copy-paste in the prose. It found copy-paste in the code.

Eleven one-line function docs in `core/src/derivation/` are repeated across
chain files. In every case the function under them is repeated too:

| Doc line | Copies | Files |
|---|---|---|
| "HMAC-SHA512 over concatenated chunks; returns a 64-byte Zeroizing buffer." | 7, identical | aptos, cardano, icp, solana, stellar, sui, ton |
| "Derive a BIP-32 child key; hardened indices use private key…" | 6, 5 identical | bitcoin, decred, evm, kaspa, tron, xrp |
| "Walk the full BIP-32 derivation path by applying derive_child…" | 6, 5 identical | same six |
| "Derive BIP-32 master key: HMAC-SHA512(hmac_key, seed) → IL + IR" | 5, identical | decred, evm, kaspa, tron, xrp |
| "Map locale string ("en", "zh-cn", …) to BIP-39 wordlist" | 5, identical bodies | cardano, decred, kaspa, monero, **primitives** |
| "BIP-39 mnemonic → 64-byte seed via NFKD normalization and PBKDF2" | 3, identical | decred, kaspa, monero |
| "Walk SLIP-10 hardened child derivation from seed…" | 3, identical | aptos, icp, solana |
| "Parse SLIP-10 ed25519 derivation path…" / "Walk the SLIP-10…" | 2 each, identical | stellar, sui |
| "Validate and decode a 64-character hex private key…" | 2, identical | bitcoin, evm |

**~303 lines are byte-identical copies**, in the derivation layer.

This is the case `docs/ARCHITECTURE.md` already rules on: "Some chain-local
duplication is intentional where it makes protocol behaviour clearer. Pure
wrapper repetition is not… the rule that separates the two is whether the
repetition carries protocol meaning or only a name." None of these carry
protocol meaning — BIP-32 child derivation is BIP-32 child derivation, and the
bodies agreeing byte for byte is the proof. It is also what rule 2 in
`AGENTS.md` describes: a per-chain fact duplicated three to seven times, where
the stale copy becomes the bug.

The clearest one: `resolve_bip39_language` already has a canonical `pub(crate)`
home in `derivation/primitives.rs`, and four chain files keep a private copy of
it anyway. The only difference between the five is the visibility keyword.

**Done in the following pass** — see "The derivation primitives were thirteen
copies of five functions" below.

### Comment audit, third pass — the FFI boundary doc had drifted

`docs/FFI-BOUNDARY.md` used `prepare_evm_send_assembly` as its worked example
of an export no gate can see: "its only caller is `AppState+SendPreview.swift`
… nothing in `cargo test` or `cli-acceptance.sh` exercises it, so whatever is
inside it can be wrong indefinitely with three green suites."

All of that was fixed and the doc was not updated. It now has five Rust tests,
a CLI command (`spectra tx assemble`) and a section in `cli-acceptance.sh`.
Telling a reader that a covered path is uncovered is worse than saying nothing.

The lesson is still true, so it kept the lesson and changed the example. Two
things were measured to write the replacement:

- **72 exports** have no caller in `core/` or `cli/` outside tests — Swift is
  their only caller.
- **34 of those are not reached by a Rust test either.** Some are thin wrappers
  whose inner function is tested; some are not.
  `core_ethereum_custom_fee_validation` parses and compares two EIP-1559 fee
  fields on the funds path, and nothing but the iOS app calls it.

The doc now names the remedy — a CLI command, not a mock — and uses
`prepare_evm_send_assembly` as the case where that remedy was applied.

Also corrected: the doc comment on `cli/src/cmd/tx.rs::assemble` still said
"the only caller of `prepare_evm_send_assembly` is the iOS send sheet", while
sitting on the second caller.

**Checked and clean:** `scripts/unreachable-exports.sh` reports 0.
`scripts/count-exports.sh` reports 196 (95 free + 101 methods) — the target in
the metrics section is still ~150. Every checkable claim in
`docs/ARCHITECTURE.md` was verified against the tree and all of them hold.

### The derivation primitives were thirteen copies of five functions

**Was:** each file under `core/src/derivation/chains/` carried its own copy of
the BIP-32, SLIP-10 and BIP-39 machinery it needed. Verified by hashing each
block before touching it — every copy listed here was **byte-identical** to the
others, comments included:

| Moved to `primitives.rs` | Copies deleted | From |
|---|---|---|
| `struct ExtendedPrivateKey` + `impl` (master/child/path, secp256k1) | 5 → 1 | decred, evm, kaspa, tron, xrp |
| `hmac_sha512` | 7 → 1 | aptos, cardano, icp, solana, stellar, sui, ton |
| `parse_slip10_ed25519_path` | 5 → 1 | aptos, icp, solana, stellar, sui |
| `derive_slip10_ed25519_key` | 5 → 1 | aptos, icp, solana, stellar, sui |
| `resolve_bip39_language` | 5 → 1 | cardano, decred, kaspa, monero (a canonical one already sat in `primitives.rs`) |
| `derive_bip39_seed` | 4 → 1 | decred, kaspa, monero (same) |

**Is:** one copy of each in `core/src/derivation/primitives.rs`, imported by the
chain files. **783 lines net out of `core/src/derivation/`**, zero warnings,
`cargo test --workspace` unchanged at 550.

**Why:** this is `AGENTS.md` rule 2 and `docs/ARCHITECTURE.md`'s own rule —
"pure wrapper repetition is not [intentional]… whether the repetition carries
protocol meaning or only a name". BIP-32 child derivation is BIP-32's, not
Tron's; the bodies agreeing byte for byte is the proof that nothing chain-shaped
lived in them. Five copies is five places for a fix to reach one of.

`resolve_bip39_language` is the clearest case: `primitives.rs` already held the
canonical `pub(crate)` version and four chain files kept a private duplicate
anyway. The only difference between the five was the visibility keyword.

`chains/bitcoin.rs` keeps its own `ExtendedPrivateKey`. It is not the same code
— it also serialises xpubs and carries `ExtendedPublicKey` — so folding it in
would mean giving every chain the xpub machinery only Bitcoin uses.

**Not a behaviour change.** Every deletion was gated on an md5 of the
comment-stripped body matching the copy being kept, so there was nothing to
diff after. Recorded here because the *shape* changed, not the behaviour.

**Check from the CLI:** `spectra wallet import --chain <any>` derives through
these on every chain; `cli-acceptance.sh` already imports across the families.

### Known open item — Decred and Kaspa derivation has no test

Found while checking that the pass above was not deleting code nothing covered.
`core/src/derivation/tests.rs` names Cardano, Monero, EVM, Tron, XRP and the
rest; it does **not** name Decred or Kaspa. Neither does any other Rust test.

What does exist: address *validation* for both (`validation/address.rs`), and a
Decred *private-key* import in `cli-acceptance.sh`. What does not exist
anywhere: an assertion that a known mnemonic produces a known Decred or Kaspa
address. Both are mainnet chains a user can hold funds on.

Not closed here, deliberately. The useful test is a vector test — a published
mnemonic and the address another implementation derives from it — and inventing
the expected value from this code's own output would pin whatever it does
today, including a bug. That is the case Rule 0 warns about. Whoever picks this
up should bring vectors from `dcrctl` / a Kaspa wallet rather than from here.

### Behaviour changed on purpose — artwork follows the coin, not the chain

Thirty-one of the crypto wiki's sixty-six coins drew a coloured letter instead
of their logo. So did every per-chain breakdown row on the dashboard whose coin
is not its chain's own ticker — USDC on Base, ETH on Base, USDT on Aptos.

*Was:* `CoinBadge` resolved artwork from an **icon identifier**,
`<namespace>:<chain>:<symbol>`, through two Swift tables:

```swift
if let direct = Coin.nativeIconAssetName(forAssetIdentifier: raw) { return direct }
if raw.hasPrefix("native:") { return nil }          // ← the wall
return TokenVisualRegistryEntry.entry(matchingAssetIdentifier: identifier)?.assetName
```

The first table was keyed on `iconIdentifier(chain.symbol, chain.name)` — a
chain's **own** ticker — so it answered only for `native:base:base`, never for
`native:base:eth`. Anything else with a `native:` namespace was then refused the
token table outright. The wiki is indexed by coin and a coin's row leads with
wherever it lives first, so it asked for `native:aptos:usdc`, `native:base:dai`,
`native:arbitrum:link` — the wall, every time. The one native coin that missed
too was OKB: X Layer's `symbol` is the string `"X Layer"`, so the first table
held `native:x-layer:x layer` and the coin asked as `okb`.

The second table matched by **substring** (`":\(symbol)"` contained in the
identifier), which is how a `native:` identifier could ever have found a token
at all, and would have handed a hypothetical `USDCE` the USDC mark.

*Now:* one core export, keyed on the only component that says what to *draw*:

```rust
core_icon_asset_name(identifier) -> String   // "" when nothing ships
```

It reads the symbol out of the identifier and looks it up in one table built in
three passes, most specific first — a chain's own ticker (BASE draws Base), the
token catalog (UNI draws Uniswap, not Unichain), then the gas token of the first
chain in catalog order that pays fees in it (OKB draws X Layer's mark, ETH draws
Ethereum's rather than one of its nine rollups'). Matching is exact, not
substring.

*Why that side:* the chain component of an identifier says where a coin is
*held*, and coins are routinely held away from home. Artwork is a fact about the
coin, so it is keyed on the coin. Both catalogs already carry `asset_name` per
row; nothing new was invented, it was only asked the right question.

Deleted with it: `Coin.nativeIconAssetName`, its dictionary,
`Coin.nativeChainIconDescriptor(forAssetIdentifier:)` (already dead) and its
dictionary, `TokenVisualRegistryEntry.entry(matchingAssetIdentifier:)` and its
fragment list, both `assetName` fields the badge orphaned
(`NativeChainIconDescriptor`, `TokenVisualRegistryEntry`), and the export
`core_normalized_icon_identifier`, whose only caller was the badge line that
normalised an identifier so a substring match could be run over it.

*Leftover, not swept here:* `TokenVisualRegistryEntry` is now a symbol → colour
table with three fields nothing reads (`title`, `assetIdentifier`, and `id`);
they were already unread before this change, so collapsing it into
`Coin.displayColor` belongs to its own pass.

Both wikis now hand `WikiCoinFace` the `assetName` core already puts on the row
rather than building an identifier for Swift to take apart again — which also
ends the wiki claiming `native:aptos:usdc`.

**One coin was genuinely missing artwork, not just looking it up wrong.** USDB
(Blast's stablecoin, `enabled = true`) named the mark `usdb` and no such file
existed. Added `resources/coinicon/usdb.svg` — Blast's yellow with a black
italic dollar, the same palette and shear as `blast.svg` — and synced it with
`scripts/export-swift-icons.sh`.

*How to check it without the app:*

```
cargo test -p spectra_core artwork_follows_the_coin_not_the_chain
```

Six tests, and it is worth being exact about what they are worth, because the
first draft of this entry overclaimed it. **None of them could have failed
before**: `core_icon_asset_name` is new, and the code that actually broke was
Swift. Four of them (`a_token_draws_its_own_mark_on_every_chain_it_lives_on`,
`a_chain_ticker_draws_the_chain`,
`a_gas_token_falls_back_to_the_chain_that_pays_in_it`,
`an_unknown_symbol_resolves_to_nothing`) pin the three-pass precedence and the
exact-vs-substring decision — a specification of a new ten-line function, cheap
and worth having, but not a regression test for anything.
`every_wiki_coin_has_artwork` is the weakest: it checks that two independent
keying schemes over the same two catalog columns agree on all sixty-six rows,
which catches a precedence mistake and nothing else. `every_named_mark_ships_a_file`
is the one that earns its place on its own — it checks each named mark against
`resources/coinicon/`, a different source of truth, and it is what would have
caught USDB.

*The actual regression test is in Swift*, because that is where the bug was:

```
cd swift && xcodebuild test -scheme Spectra \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' \
  -only-testing:SpectraTests/CoinBadgeArtworkTests
```

`CoinBadgeArtworkTests` walks every coin × every chain it lives on and asserts
`UIImage(named:)` loads — Swift identifier → core → asset catalog → a file on
disk, which nothing can satisfy by agreeing with itself. It was red on
thirty-one coins and on ETH across nine rollups before this change.
`testAChainBadgeStillDrawsTheChain` covers the direction the fix could have
overshot in: Base's gas is ETH, and the chain badge must still draw Base.
