# Core-first migration plan

Spectra's iOS app was built before its Rust core. This document states the
architecture the project is moving to, why, what still stands in the way, and
the order the work happens in.

It is the plan of record. When it disagrees with `docs/ARCHITECTURE.md`, this
file describes the destination and that file describes the present.

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

| | |
|---|---|
| Swift, non-generated, excluding tests | 30,879 lines |
| — `views/` + `extensions/` (genuine UI) | 11,113 lines (36%) |
| — root of `swift/` (`AppState`, stores, persistence, bridges) | 19,766 lines (64%) |
| `core_plan_*` FFI exports (core advises, Swift applies) | 42 |
| Swift calls to `StateCommand` / `reduce_state_in_place` | 0 |

The third and fourth rows are the same fact stated twice. Rust has a state
reducer; Swift has never called it. `swift/AppState+CoreStateStore.swift` says
so in a comment: *"Swift's `@Observable` arrays on AppState are the canonical
store … There is no Rust round-trip."*

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

**Done when:** every remaining `core_plan_*` is a planner over state that has
not moved yet — not merely a function whose name starts with `core_plan_`.

The original wording ("under 10") counted the prefix, not the shape, and those
are different things. Roughly twenty of the exports were never planners at all:
`core_icon_identifier` builds a string, `core_ethereum_send_error_code`
classifies a message. They hold no state, they were correctly placed on the
FFI, and they only looked like debt because of the name. They have been renamed
to drop the prefix — a naming fix, and worth saying plainly that it is not a
conversion.

**Stage 2 is complete.** 42 → 10. The ten survivors are all genuine planners
whose state Stage 3 moves: `store_derived_state` and `transfer_availability`
(derived caches in `StorePersistenceNormalization`), the two keypool baselines
(they read the Swift-held owned-address table), the dashboard and token
preference trio, `price_alert_evaluation`, `append_chain_operational_event`,
and `reset_dispatch`. Converting them before their state moves would just
relocate the round trip.

### Stage 3 — Thin the shell

> **Method change, decided partway through this stage.** Everything above was
> done as *equivalent migration*: move the code, change no behaviour. That
> works until it meets a place where the Swift is not self-consistent — 21
> chains accept an unvalidated import address and 3 do not; 3 EVM chains gate
> non-native sends on token support and 20 do not. Preserving those exactly
> means either reproducing the inconsistency in core or stopping to ask which
> side is right, and there are more of them ahead.
>
> The remit here is a rewrite, so from this point the rule is: **write the one
> correct rule in core and delete the Swift, rather than port what is there.**
> Where the existing behaviour splits, take the safe side — validate every
> address, gate every chain. Every such change is listed under "Behaviour
> changed on purpose" below, with what it was and why, so any of them can be
> reversed on inspection.
>
> Slices are proven by the CLI before the Swift is deleted. That is the same
> test rule 1 has always stated; it is now also the acceptance gate.

## Behaviour changed on purpose

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
  38 assertions against a scratch data directory with no network.

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
adding a chain requires no Swift change at all. Currently 17,922 root vs
10,969 in `views/` — the number that has to invert. Down 1,023 so far, and
worth splitting honestly: 965 of that is one dead file that was never in the
build, so only ~60 net lines have actually been moved rather than found — the
`resolved<Chain>` deletion took out 24 lines while the two error paths added
back more than that, which is the right trade and still not progress on this
metric. The remaining weight is in `AppState+SendFlow` (1,566), `AppState`
(1,239), `AppState+ReceiveFlow` (708) and `AppState+DiagnosticsEndpoints`
(859), and those come down only when the caches they feed stop existing.

### Stage C — Rewrite core — **started**

Stages 0-3 moved ownership *into* core without reshaping core. It shows: the
crate is 59,564 lines with **234 exported functions**, which is the plainest
statement that it is still a library of helpers rather than a program. A front
end that has 234 ways in does not have to go through the ten that matter.

Measured, not estimated:

| | |
|---|---|
| `#[uniffi::export]` sites / exported functions | 251 / 234 |
| `service.rs` | 4,781 lines, 90 functions, 22 `match chain` sites |
| Chain tables | two — `chains.rs` (TOML) and `registry.rs` (enum) |
| Duplicate module pairs | `service.rs`/`service/`, `validation.rs`/`derivation/validation.rs`, `fetch/refresh.rs`/`refresh_engine.rs` |

**C1 — the skeleton.** Holds the FFI surface still, so no Swift changes:
merge the two chain tables, split `service.rs` into the three owners it
actually has (resident state, per-chain network clients, send execution), and
collapse the duplicate module pairs.

**C2 — the surface.** 234 → 30-40. Most exports are "core computes a value,
Swift assembles it" — those should *disappear*, not be renamed. What survives:
`WalletService` methods, `StateCommand`, and the few genuinely pure
calculations. This one moves every Swift call site, so it runs with the rest of
Stage 3 rather than beside it.

Done so far:

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
  the only drift — and `display_names_match_the_catalog` means the next one
  fails the build.

### Stage 4 — Android

Only meaningful once the above holds. If Kotlin can be brought up against the
same core without discovering new iOS assumptions, the migration worked.

---

## How progress is measured

Not by feel. These four numbers, checked at the end of each stage:

| Metric | Start | Now | Target |
|---|---|---|---|
| `core_plan_*` exports | 42 | 8 | 0 |
| Swift root lines vs `views/` | 19,766 vs 11,113 | 17,922 vs 10,969 | inverted |
| Domain collections stored on `AppState` | 3 | 0 | 0 |
| Domain settings owned by core | 0 | 1 | all |
| Wallet operations reachable from the CLI | partial | partial | all |
| CLI commands drivable without a TTY | 0 of 24 | all | all |

The last row is new, and it is the one that makes the others checkable. Every
earlier "proven by the CLI" claim in this document was proven by a person typing
into a prompt. `scripts/cli-acceptance.sh` replaces that with 38 assertions on
exit codes and JSON, over a scratch data directory and with no network.

Both iOS suites are green as of this pass — 34 tests, 0 failures. The
`testEthereumTestNetworksExposeExpectedContextsAndEndpoints` failure this
document told readers to expect is fixed, so a red test is now a real one.

Stage 0 built the mechanism and moved nothing, so the Swift count went *up*.
The address book is the first collection to actually move; the count starts
coming down as the paths it replaced are deleted.

## Rules for new work while this is in progress

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
- `scripts/bindgen-ios.sh` and the Xcode "Build Rust Derivation Core" phase both
  regenerate `swift/generated/` and apply *different* Swift 6 patches. One
  should go.
- ~~`registry::Chain` calls Internet Computer `"ICP"`.~~ **Fixed.** The enum
  says "Internet Computer" now; `coin_symbol` still says "ICP", which is what
  that field is for. The special case in `from_display_name` is gone, and
  `display_names_match_the_catalog` walks all 78 chains — that rename was the
  only disagreement, and there cannot be another one silently.
