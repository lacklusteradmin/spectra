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

This is the plan of record. [Architecture](docs/ARCHITECTURE.md) explains the
ownership model; [FFI boundary](docs/FFI-BOUNDARY.md) covers integration traps.

## The target

```text
core/   domain state, domain rules, persistence, network, crypto
cli/    a full front end that proves core needs no platform
swift/  native UI: renders core's results and forwards user intents
kotlin/ the same boundary, later
```

Done means core owns both the data and the decisions, no platform persists a
second authoritative copy, and the CLI can drive every domain operation.
Navigation, editing and rendering caches remain platform view state.

## Rules for new work while this is in progress

0. Apply Rule 0 above; record behaviour changes and take the stricter side for
   funds, keys and addresses.
1. New domain logic goes in `core/`. If `spectra` cannot drive it, it is in the
   wrong place.
2. Per-chain facts go on `registry::Chain`, not into caller-owned lists.
3. Do not add `core_plan_*` functions. Core must own the state it decides about.
4. Swift may hold view state and projections, not authoritative domain state.
5. Prefer deleting unnecessary Swift code over porting it.

## The stages

| Stage | Status | Result or remaining work |
|---|---|---|
| 0 — Prove ownership on display currency | Done | `open_state` and state commands bind, update and persist core-owned state |
| 1 — Move domain collections | Done | Wallets and address book are core-owned; history has its own queryable store; Swift renders projections |
| 2 — Replace planners with intents | Done | No `core_plan_*` exports remain; some pure helpers only needed renaming |
| 3 — Thin the shell | In progress | Remove remaining Swift orchestration and duplicate domain calculations |
| C1 — Reshape core | Done | Shared chain catalog, service modules split by responsibility, duplicate modules and derivation primitives consolidated |
| C2 — Reduce the FFI surface | In progress | Prefer operations over caller-assembled advice; retain distinct typed protocol inputs |
| 4 — Android | Not started beyond skeleton | Implement against the shared core once the boundary is ready |

Other completed ownership slices: settings, token preferences, price alerts,
keypool, owned addresses, operational events, refresh scheduling, send routing,
recipient checks, dashboard grouping and transaction-derived data. UTXO discovery
and receive reservation now run in core through the shared secret layout.
The asset wiki and its follow-up cleanup are also complete. The unused
`LoadingTaskRegistry` and its Xcode references have been removed.

### Remaining direction

- Find Swift code that reads core-owned data only to send it back for a decision;
  make the owning service compute the answer instead.
- Keep protocol-specific preview inputs where they differ. Do not collapse them
  into one wide enum merely to reduce a count.
- Keep one writer per UI projection. Adopt core-derived answers asynchronously;
  local indexes and button-enabling checks may remain view state, with core
  enforcing validation on writes.
- Remove dead wrappers only after checking direct FFI callers and foreign
  callback implementations. Delete tests of removed helpers only when the
  replacement's meaningful coverage is identified.

## How progress is measured

Use reproducible checks rather than retaining per-session counts:

- `scripts/count-exports.sh`: callable FFI surface, with a working target of
  roughly 150. Cross-check macros against generated bindings and exclude
  converter helpers. The earlier target of 60 required merging unrelated
  operations behind a wide union and was rejected.
- `scripts/unreachable-exports.sh`: unused export candidates.
- Domain collections and decisions must have one owner; new operations must be
  reachable through the CLI and state must survive reopening the database.
- Compare non-generated Swift orchestration with UI code to locate remaining
  debt. Line counts are diagnostic, not a reason to relocate code artificially
  or keep dead views. An export that removes a Swift rule can be worthwhile.

Run all three suites at each stage:

```sh
cargo test --workspace
./scripts/cli-acceptance.sh
(cd swift && xcodebuild test -scheme Spectra \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro')
```

CLI acceptance uses a throwaway directory without network. There are no
expected-red iOS tests. Exercise changed FFI/UI paths in the app too: CLI tests
cannot detect a missing Tokio runtime on a Swift async export, and offline
assembly cannot verify a broadcast.

## Known open items

- **Broadcast coverage:** `spectra send broadcast` exists, but the offline CLI
  gate and iOS suites do not broadcast. Add controlled testnet coverage before
  treating send orchestration as end-to-end tested. Registry/router/builder
  agreement is an offline check, not evidence that a send lands.
- **App-only domain exports:** some rules still lack a CLI caller or direct Rust
  coverage. Audit callers before counting coverage. Custom EVM fees now have
  a shared parser, CLI entry point and Swift binding tests.
- **Decred and Kaspa mnemonic vectors:** no independent known-mnemonic →
  known-address test was identified. Existing address-validation and Decred
  private-key-import checks do not substitute. Use published or independently
  derived vectors, not this implementation's output as its own expected value.
- **Endpoint probes and redundancy:** the earlier sweep flagged probe URLs for
  Bitcoin SV, Internet Computer and Zcash, plus chains with only one RPC node.
  Recheck through `spectra endpoints` before changing rows; recorded failures
  may be bad probe configuration rather than an unavailable service.
- **EVM history availability:** the registry distinguishes open indexers,
  Etherscan V2 requiring a key, and unavailable sources. Remaining key-dependent
  chains need supported infrastructure if keyless history is required. Do not
  silently depend on another wallet's private backend.

- **Testnet balances query mainnet endpoints:** a refresh entry files its
  balance under the wallet's own chain id, not the network it is on, because
  the holding it produces is merged by chain name — filing a Testnet4 balance
  under "Bitcoin Testnet4" would add a second holding rather than update the
  one the wallet shows. So a wallet on a testnet fetches its testnet address
  from that chain's *mainnet* endpoints and reads zero. Fixing it means
  deciding what a testnet holding is called and how it merges, which is a UI
  question as much as a core one. `refresh_entries_for` marks the spot.

## Behaviour changed on purpose

Keep entries to the previous behaviour, the new behaviour, the reason and a
CLI check. If a check needs network, a simulator or new coverage, say so.
Completed refactor diaries and old test/line counts do not belong here.
The entries below summarize the retained decisions, not a fresh test run.

### Existing core-state migration: Swift integration

The full iOS gate exposed a stale call to the already-removed
`priceMergeFiatRateUpdates` export. Swift now calls core's existing
`refresh_fiat_rates` operation and mirrors `CoreAppState.fiat_rates_from_usd` on
load. Its duplicate SQLite/UserDefaults rate reads and writes are removed, so
an older platform cache cannot overwrite core's stored rates. CLI check:
`cargo test -p spectra_core stored_fiat_rates_survive_reopening`; the complete
iOS suite verifies the generated `refreshFiatRates` binding compiles. The same
build exposed stale references to the four removed Tor UserDefaults keys; those
reads/deletes are removed too, since `adoptAppSettings` already mirrors all four
values from core. Check `spectra settings list` and `./scripts/cli-acceptance.sh`
for the core-owned settings and reset paths.

### Checked signing, fees and history pages

- **EVM signing gas:** native signing used 21000 without estimating, and ERC-20
  signing swallowed estimate failures into 65000. ERC-20 estimates also preceded
  calldata overrides. Both paths now estimate the final calldata, value, nonce,
  fees and access list; failures and zero/overflowing estimates refuse signing.
  An explicit positive gas limit still skips estimation. The 20% default buffer
  uses checked-width integer arithmetic and rounds upward. CLI check:
  `cargo test -p spectra_core gas_tests` uses local RPC mocks and sign-only mode;
  no transaction is broadcast to a live chain.
- **Fixed-fee UTXO accounting:** BCH, BSV, BTG, DASH, DOGE, LTC, ZEC, DCR and ADA
  summed inputs unchecked and saturated insufficient funds into zero change.
  They now share checked totals, amount-plus-fee addition and subtraction, and
  refuse zero sends. Cardano previously omitted small change without adjusting
  its explicit fee, creating an unbalanced body. It now refuses sub-minimum
  change; exact minimum change is included. Bitcoin-family dust continues to
  become implicit fee. CLI check: `cargo test -p spectra_core accounting`.
- **Fee conversion:** DOGE fee rates, ADA fees and Sui budgets used a rounding,
  saturating float-to-u64 cast. Native fees now convert the shortest decimal
  representation exactly, rejecting nonpositive/nonfinite, excess precision and
  out-of-range input before signing identity reads. DOGE's existing 350-byte
  estimate uses integer arithmetic and rounds up to a whole unit. CLI checks:
  `spectra send fee-units --chain Cardano --amount 0.17` returns 170000;
  negative, NaN, excess-precision and overflowing inputs are covered by
  `./scripts/cli-acceptance.sh`; `cargo test -p spectra_core invalid_fees` checks
  the actual execute-send entry point before keys/network.
- **EVM history:** native history always fetched page 1 / 50 while token history
  used the requested page. Both now use the requested page and size (minimum
  page 1, size 1–500), and either provider failure fails the operation rather than
  returning an empty/partial success. An empty tracked-token list skips the token
  request as documented. CLI check: `cargo test -p spectra_core history_page`
  exercises distinct pages and provider errors with an offline HTTP server.
- **Zcash expiry:** a failed tip read became height 0 and a transaction expiring
  at 40. The read now propagates errors and tip + 40 must fit u32 before any
  signing/broadcast. CLI check: `cargo test -p spectra_core expiry_tests` covers
  missing height and both addition/conversion overflow boundaries.

### Failed reads refuse rather than fabricate state

- **Token sends:** EVM/Tron metadata failures previously fell back to caller
  decimals. These failures now abort parameter construction; caller precision is
  used only for families without a metadata reader. A successful on-chain read
  still overrides the caller. Check `cargo test -p spectra_core failed_metadata`
  and `exact_amounts_reach_native_and_token_signing_params` (local mock RPC).
  Live `spectra send broadcast` needs network and a funded test wallet; no live
  transaction is part of the offline gate.
- **Token integers:** TRC-20 returned only the low u128 bits of uint256 and cast
  decimals to u8. ABI integers now require one complete hex word, reject nonzero
  high bits and malformed data, and reject unsupported precision instead of
  wrapping. EVM/Tron and typed token-balance paths enforce the core's 38-decimal
  scaling limit before formatting; sends already enforce this limit. Check
  `cargo test -p spectra_core integer_tests` (mock RPC and boundary vectors).
- **Keypool failures:** failed history queries were treated as absent history.
  Reads, reservations and conditional advancement now return the error, without
  lowering the floor or publishing a mutation. Out-of-range indices also refuse.
  The UniFFI read now throws; CLI `spectra pool show/next <wallet>` reports failure,
  diagnostics show the error, and receive flows do not substitute old addresses
  after a failed read. Check `cargo test -p spectra_core unreadable_history` and
  the Swift `testInvalidKeypoolBaselineThrowsAcrossAsyncBinding` (offline).
- **Balance/preview failures:** token reads across supported families propagate
  provider errors instead of returning zero/fallback precision. A successfully
  queried empty account set remains zero. SPL reads validate and sum all
  returned accounts for a mint instead of taking only the first. EVM preview
  requires valid amount input plus successful nonce, gas, balance and fee reads;
  no invented 21000 gas or default fee quote.

  What a propagated error is allowed to take with it depends on who asked.
  `fetch_token_balances` first collected its per-token futures into one
  `Result`, so a single self-destructed contract failed every other token in
  the request, and the only caller does `try?` — a wallet's whole token list
  silently stopped refreshing until that contract was removed. A token that
  cannot be read is now left out of the result instead: the caller does not
  update it, so its last known balance stays, which claims nothing. A call
  covering the whole chain rather than one token — TON reads every jetton at
  once — is still an error, and so is a row whose contract is empty, except
  that the row is skipped rather than failing its neighbours. The send path is
  the one that must not read an absence as zero, so `send_destination_risk`
  refuses a token it got no row for rather than treating it as an empty wallet.
  Check `cargo test -p spectra_core failed_reads`,
  `an_unreadable_token_does_not_take_the_readable_ones_with_it`,
  `an_unreadable_token_balance_is_not_an_empty_wallet` and `balance_read_tests`
  (offline/mock RPC). `spectra send probe` needs network; Swift tests check that
  an unavailable EVM preview throws across the binding.
- **Fee history with no sample:** an `eth_feeHistory` whose most recent block
  carries an empty reward list is not a broken response — it is a block nobody
  transacted in, which is the normal state of a quiet L2 or testnet — and
  refusing it left the send screen unable to quote a fee at all there. That
  case reads as a zero priority fee: `max_fee` still covers the base fee, so
  the transaction lands, where a gwei picked to stand in would only overpay. A
  missing `reward` array remains an error, since the request always names
  percentiles and a node that omits it did not answer what was asked. Check
  `cargo test -p spectra_core fee_history_tests`.
- **User metadata:** an unreadable `token_preferences`, `price_alerts` or
  `fiat_rates_from_usd` row is named in the log, dropped, and rebuilt — the
  first two from the catalog on the next evaluation, the third on the next
  refresh. The stored bytes are left exactly as they were, so a build that can
  read the row still will, and no reset or recovery shim is added. Failing the
  load over one of them was tried and reverted: it takes the wallet list down
  with it, and the wallet list cannot be rebuilt from anything, which is the
  whole reason `settings` and the wallet rows *do* stay fatal. Check
  `cargo test -p spectra_core unreadable_rebuildable_metadata_costs_only_that_row`,
  `unreadable_settings_still_fail_the_load` and
  `unreadable_preferences_refuse_loading_without_deleting_wallets`. CLI
  acceptance covers normal persistence and the no-op paths.
- **Tor's proxy goes on before the status says Ready:** `bootstrap_tor`
  published `Running` and applied the SOCKS proxy afterwards. The kill switch
  reads that status, so between the two a request found the switch open and the
  shared client still unproxied, and went to the provider over a direct
  connection — the one thing the switch exists to prevent. Reversed, the window
  fails the other way: a request landing in it is blocked a moment longer than
  it needs to be. No CLI check; the window is internal to bootstrap.

### Blockbook clients, history shapes and Bitcoin signers

- **Blockbook family:** Litecoin, Bitcoin Cash, Bitcoin Gold, Zcash and Dash
  each carried a private copy of the same Trezor Blockbook client — five files
  matching 76–91% pairwise, redeclaring the same six JSON shapes. They are now
  one `BlockbookClient<N>` in `core/src/fetch/chains/blockbook.rs`, with a
  per-chain marker carrying the only real difference (Bitcoin Cash normalizes
  CashAddr before asking). The copies had drifted rather than diverged:
  `has_activity` existed on two of the five and `fetch_chain_tip_height` on
  one, so Dash, Bitcoin Gold and Zcash now have both. Nothing calls them there
  yet — `Chain::supports_deep_utxo_discovery` still gates discovery to the
  five chains it always did. Check `spectra balance <wallet> --chain zcash` and
  `cargo test -p spectra_core blockbook` (offline for the unit tests; the
  balance check needs network).
- **UTXO testnet history:** `normalize_chain_history` matched on the exact
  chain, so every UTXO testnet fell through to an empty result however well its
  fetch had gone — the same shape as the five-name lists this repo has removed
  elsewhere. It now reads its mainnet's shape through `mainnet_counterpart()`.
  Check `cargo test -p spectra_core testnets_normalize_like_their_mainnets`.
- **History amounts are magnitudes:** the fifteen per-chain arms disagreed on
  whether `amount` could be negative — five took `unsigned_abs()`, the rest
  passed the raw value through. It is now always a magnitude, with direction in
  `kind`, which is what every reader already assumed. Only rows whose chain
  reports a signed amount *and* an explicit `is_incoming` are affected, and no
  client emits that combination today. Check
  `cargo test -p spectra_core amounts_are_magnitudes`.
- **Legacy P2PKH fee sizing:** `select_coins` hardcoded 31 vbytes per output
  and ignored the size it was passed, so a Bitcoin legacy send that selected
  its own coins paid for 31-byte outputs while the same send with pinned UTXOs
  paid for 34. Both now size from `SpendSizing`, so a P2PKH send costs 3
  vbytes per output more than before — the correct amount. Check
  `cargo test -p spectra_core pinned_and_selected_coins_charge_the_same_fee`.
- **The signed fee is the quoted fee:** the fallback used when a caller
  supplies no fee lived in three tables — a literal per signing arm, a second
  per arm of `build_send_params`, and `Chain::static_fee_units`, which is where
  the fee shown on the send screen comes from. Litecoin signed 10 000 against a
  1 000 quote and Bitcoin Cash 1 000 against 2 000. All three now read the
  registry. Check
  `cargo test -p spectra_core an_unquoted_fee_falls_back_to_what_the_estimate_reports`.
- **A bad txid is refused:** two copies of the legacy input builder decoded the
  txid with `unwrap_or_default()`, so a malformed one produced an empty
  outpoint and a structurally invalid transaction reported as success. The
  shared `send::chains::wire` builder returns an error, and Decred decodes once
  when it builds its inputs rather than at each serialization. Check
  `cargo test -p spectra_core a_bad_txid_is_refused`.
- **Bitcoin Cash `has_activity` normalizes first:** it asked its Blockbook
  instance about the address exactly as given, while `fetch_balance` — the same
  endpoint, one line away — normalized CashAddr to legacy first. The shared
  client normalizes once for every read. Check
  `cargo test -p spectra_core blockbook`.
- **The Dogecoin fee fallback is the registry's:** the signing arm carried
  `unwrap_or(200_000)` where `Chain::static_fee_units` says 1 000 000. Nothing
  reached it — `build_send_params` computes Dogecoin's fee from its kB rate and
  always supplies one — so this removes a dead third opinion rather than
  changing a fee. Check `cargo test -p spectra_core dogecoin_uses_its_own_scale`.
- **A history row with no amount is kept, not dropped:** Bitcoin and the
  Blockbook family required their amount field and dropped the entry without
  it; the other thirteen chains defaulted it to zero. That split followed which
  arm was written first, not a decision. All of them now default, which is the
  majority behaviour — and the rows reach this function serialized from typed
  structs where the field is not optional, so neither branch fires in practice.
  Check `cargo test -p spectra_core every_chain_shape_normalizes`.
- **Error precedence in `sign_p2wpkh`:** it parsed the from-address after coin
  selection while the other three signers parsed it before, so a send that was
  both underfunded and malformed reported different errors depending on script
  type. All four now parse first. No CLI check: only the message differs.

### Keypool history projections and TRC-20 read metadata

- **Keypool reads:** every read/reservation loaded and decoded a wallet's full
  transaction history, then filtered by chain and parsed repeated paths. Core
  now reads distinct source/change paths scoped to the wallet and chain through
  SQLite expression indexes. Only those paths are decoded; transaction bodies
  are not loaded. This is an indexed projection, not a constant-time cached
  maximum. Indexes follow transaction edits/deletions automatically. Owned-address
  maxima are folded without temporary vectors. Check `spectra pool show <wallet>`
  and `cargo test -p spectra_core keypool_history_projection` (offline; checks
  chain/wallet isolation, query plans and mutations).
- **Repeated reservations:** an existing reservation used to write the same row
  again. Core still merges the latest discovered floor, then skips persistence
  when the complete merged record is unchanged. New discoveries still raise the
  persisted floor without replacing the held reservation. Check consecutive
  `spectra pool next <wallet>` calls and
  `cargo test -p spectra_core unchanged_receive_reservation` for zero redundant
  SQL updates and persistence of newly owned indices.
- **TRC-20 metadata:** each balance/enumeration read queried symbol and decimals
  again, including concurrent reads of the same contract from different wallets.
  A service now shares up to 256 cached metadata entries for five minutes, keyed
  by chain, endpoint list and contract. In-flight cached requests share one result;
  failures are discarded and cancelled initialization can be retried. Changing
  endpoints or chain uses a separate entry. Read metadata can now remain unchanged
  for up to five minutes after a contract update; balances remain live. Send
  preparation uses the uncached metadata API and still fetches current decimals.
  Check `cargo test -p spectra_core metadata_cache` for mock-RPC call counts,
  source isolation, send bypass, expiry, capacity and cancellation (no live chain).
  Live `spectra token discover --wallet <wallet>` needs network;
  cross-wallet reuse occurs within the long-lived core service, not across CLI
  processes. CLI acceptance continues to check the offline token/state paths.

### Bounded discovery and incremental state writes

- **Address derivation:** every external index repeated BIP-39 and the complete
  BIP-32 path. A scan now derives its external xpub once on a blocking worker,
  then derives public children; no mnemonic is retained during network probes.
  Check `cargo test -p spectra_core public_children_match_full_derivation`
  for every discovery network, Bitcoin script types and the BIP-84 vector.
  `spectra pool next <unsealed-Bitcoin-wallet>` now includes the derived receive
  address; CLI acceptance checks this offline and checks a stable reservation
  after reopening.
- **Activity:** scans waited for each address and sometimes fetched full histories
  (BSV also enriched every transaction). Four probes can now be in flight, with
  index-ordered results. Esplora, Blockbook and BlockCypher use confirmed/pending
  counters; BSV uses balance plus its history index without transaction details.
  Missing counters, malformed responses and provider failures now return errors
  instead of claiming an address is unused. A failed scan may have recorded earlier
  successful probes; it never advances a reservation from a failed probe.
  Check `cargo test -p spectra_core service::state::performance_tests` using local
  mock HTTP only. Counter schemas: [Blockbook](https://github.com/trezor/blockbook/blob/master/openapi.yaml),
  [BlockCypher](https://www.blockcypher.com/dev/bitcoin/#address-balance-endpoint).
- **CLI discovery:** `pool discover` used a fresh endpoint-only service, so it had
  no wallets or secrets to scan. It now opens the selected database and secret
  store. Check `spectra pool discover <wallet>`: sealed wallets list their known
  addresses offline (CLI acceptance); unsealed discovery needs network or a mock
  provider. Live provider interoperability is outside the offline gates.
- **State commits:** small commands cloned intermediate snapshots and replaced
  every wallet/address-book row. Commands now build an incremental write set,
  serialize changed records only, and save it in one transaction before publishing.
  Settings changes write only their metadata; no-op commands do not write.
  Snapshot replacement remains available for standalone store callers. Check
  `spectra currency EUR` then `spectra currency` in separate processes, CLI
  acceptance, and `cargo test -p spectra_core a_setting_update_only_writes` plus
  `cargo test -p spectra_core incremental_state_reorders` for SQL write counts,
  reordering, removals and transaction rollback. Existing cancellation/concurrent
  writer tests still apply.

### Stored send identity and database connection ownership

- **Send identity:** the execution request separately accepted chain id/name,
  sender, derivation data and two optional secrets, preferring the seed when both
  were supplied. It now accepts a wallet id and optional unlock password; core
  resolves its stored path/overrides and exactly one signing source. Missing or
  watch-only wallets, unrelated chains, ambiguous secret blobs and addresses
  disagreeing with the derived key are refused before provider reads. Private-key
  wallets no longer require a caller-supplied seed/path. Check `spectra send
  identity --from <wallet>` (offline, using the usual password file/environment
  options), including `--chain Arbitrum` for an Ethereum wallet and refusal of
  `--chain Solana`. CLI acceptance and `service::send_identity::tests` cover seed,
  private-key, password, mismatch and ambiguity cases; Swift tests cover the
  new request and missing-wallet refusal across the async binding. Swift passes
  no password today, so sealed-wallet sends still refuse until a password is
  provided; the CLI supports the password path.
- **Account-based signers:** NEAR named-account key authorization remains the
  protocol client's access-key lookup before signing; offline identity resolution
  only resolves that name, while implicit account ids must match the derived key.
  Monero previously sent through any configured wallet-rpc without checking which
  wallet it held. It now checks account 0 against the resolved sender and submits
  through that same endpoint; a mismatch refuses and a failed transfer is not
  retried through a different endpoint. The local mock-RPC test
  `monero_rpc_is_bound_to_the_checked_sender_and_endpoint` checks refusal and
  matched submission. Actual broadcasts still require controlled network testing.
- **Database connections:** a process-global connection map held one mutex through
  every SQL operation and retained connections forever. It now indexes weak
  handles, with each database independently locking initialization and SQL; the
  service retains its active connection and releases it on rebind/drop. Standalone
  path-based operations close their connection when their last owner finishes.
  SQLite waits up to five seconds for file-lock contention. Check ordinary
  `spectra currency` / `spectra pool` persistence in CLI acceptance, and
  `cargo test -p spectra_core connection_lifecycle_tests` for independent-database
  progress, shared handles and connection release (all offline).

### Atomic writes, exact send amounts and receive advancement

- **Persistent mutations:** app-state and event snapshots could be saved out of
  order; keypool/address writes and deletes changed memory before SQLite could
  fail. Persistent mutations now share a service writer, save candidates before
  publishing, and finish admitted writes even when the caller cancels its wait.
  Opening a database uses the same writer and loads all tables before publishing;
  malformed event data is refused instead of silently cleared. Combined wallet
  deletion uses one SQLite transaction. Check `spectra currency EUR` followed by
  `spectra currency` in a separate process and the CLI keypool checks. Failure,
  cancellation and concurrent-write coverage: `cargo test -p spectra_core
  service::state::tests` (offline).
- **Send amount:** `SendExecutionRequest` had both a float and optional decimal
  string; different chains rounded, truncated or ignored the exact input.
  It now requires one decimal string, uses checked integer arithmetic for every
  native/token builder, and refuses invalid syntax, excess precision and integer
  overflow. Supported precision is bounded to 38 digits for the u128 conversion;
  protocol u64/i64 limits are checked before signing. Zero remains allowed for
  native EVM transactions; other transfers require positive amounts. Check
  `spectra send amount --chain Solana --amount 9007199.254740993` (raw units
  `9007199254740993`); Bitcoin `0.000000001`, negative/nonfinite values and u128
  overflow exit 3. CLI acceptance, Rust builder tests and `SendAmountBridgeTests`
  exercise the change offline; live broadcast remains outside these checks.
- **Receive advancement:** probing an address then clearing the current reservation
  could discard a newer reservation created while the probe was in flight.
  Advancement now atomically compares the checked index and reserves its successor;
  stale results do nothing and the successor respects addresses discovered during
  the probe. Check `spectra pool next <wallet>` for stable receive
  reservations and `cargo test -p spectra_core
  concurrent_probes_advance_only_the_reservation_they_checked` for concurrent/stale
  probe results and persisted successors. Live activity probes still need network.

### State and persistence

- **Collections and settings:** Swift arrays, snapshots and separate settings
  blobs became core-owned state and commands. Core validates address-book
  entries, rejects duplicates and clamps setting bounds so front ends cannot
  persist different answers. Check `spectra address book`, `currency` and
  `settings` in separate processes; CLI acceptance covers persistence/refusals.
- **Wallet lifecycle:** repeated `open_state` no longer replaces newer state;
  refresh updates only wallets still present, and launch loads cannot erase a
  newer UI projection. This prevents deleted wallets returning or new imports
  disappearing. Check wallet lifecycle in CLI acceptance; launch races require
  the Swift tests.
- **History:** Swift decided what to merge and save; core now merges incoming
  records against its own store and writes only changes. History methods use
  the database bound by `open_state`, preventing mismatched read/write paths.
  Check `spectra txs` and the Rust history-store tests, including
  `records_land_in_the_database_the_service_was_opened_on`.
- **Tokens and alerts:** tracked tokens previously vanished on restart; alert
  lists had separate platform storage. Both now persist in resident state.
  Display places are capped at token decimals, alert targets must be positive,
  and alert evaluation saves trigger state in core. Built-in token ids are
  `builtin:<chain>:<contract>` instead of regenerated UUIDs. Check `spectra token`
  and `spectra alert` across separate invocations; CLI acceptance covers them.
- **Operational events:** caller-minted ids/timestamps and caller-applied limits
  became an atomic core append with newest-first ordering and a per-chain cap
  of 200. Check `cargo test -p spectra_core the_log_is_newest_first_and_bounded`;
  that bound has a Rust test rather than a dedicated CLI assertion.

### Keys, import and receive addresses

- **A wallet's address is stored, not derived on read:** resolving "this
  wallet's address on this chain" was a Swift function that read the seed out
  of the Keychain, resolved a derivation path, called the deriver and validated
  the result — per call, on the render path, for a value core had already
  computed at import. A password-sealed wallet has no seed to read, so it fell
  through to the stored address whatever network it was on, and said nothing.
  Core now derives an address for **every network of a selected chain's family**
  at import, when the seed is in hand, and stores each under that network's own
  slot; `WalletSummary` carries them all instead of only the selected chain's,
  so they survive into the database and out to a front end. Switching to
  Testnet4 is a lookup. Three consequences, all deliberate: a chain the wallet
  was never imported for now has no address rather than one invented from its
  seed; the Ethereum/Ethereum Classic slot pair is filled in both directions
  from the wallet's *own* address, where before only an ETC wallet filled both,
  so an Ethereum wallet answers on all 23 EVM mainnets; and a wallet whose
  chain the registry does not know converts with the slots it holds rather than
  with none. Swift derives no addresses at all now, and
  `core_resolve_derived_or_stored_address` — the export it handed its two
  candidates to — is gone with `DerivedAddressPostProcess`. Wallets imported
  before this change hold only their mainnet slot; Spectra is prelaunch, so
  they are re-imported rather than migrated. Check `spectra wallet show --json`
  for the `addresses` map and CLI acceptance's "addresses per network" section,
  `cargo test -p spectra_core a_seed_import_stores_one_address_per_network_of_its_family`,
  and iOS `testImportingBitcoinWalletOnTestnet4StoresTheMainnetDerivedAddress`
  and `testAWalletAnswersForItsOwnChainsAndNoOthers`.


- **Address resolution columns:** resolving a wallet's address for a chain went
  through a nineteen-row table in Swift carrying three per-chain columns —
  whether to read the configured derivation path raw or resolved, how to
  post-process the derived address (`trim`, `lowercase`, nothing), and whether
  to normalize the stored one. None was a fact about a chain: ten rows read the
  path raw and nine resolved it, two asked for lowercase and three for a trim,
  six normalized the stored address and thirteen did not, and no row had a
  reason. The table is gone. The path is the resolved one everywhere, so a
  chain the wallet predates derives from the catalog default instead of from
  the empty string; the derived address is used as derived, which
  `a_derived_address_needs_no_post_processing` asserts is already trimmed and
  already lowercase on the two chains that asked; and a stored address comes
  back in its canonical spelling on every chain rather than on six — a stored
  Bitcoin Cash or Sui address may now display in its prefixed form where it
  previously displayed as typed. `core_resolve_derived_or_stored_address` lost
  both parameters and `DerivedAddressPostProcess` with them. A chain outside
  the old table no longer resolves to `nil` on that ground alone, which is what
  a testnet chain name asked by name used to get. Check `spectra wallet show`
  and CLI acceptance's derived-address assertions; iOS
  `testEveryMainnetThatDerivesResolvesAWalletAddress` walks the whole catalog
  and names Monero as the one chain with no derived address, and
  `a_derived_address_passes_its_own_validator` covers the derivation side.


- **Secret layout:** iOS plaintext-plus-password-verifier and CLI encrypted
  blobs became one core-managed layout. Passwords now encrypt seed/private-key
  material rather than only gating reveal. No-password wallets are explicitly
  unsealed; supplying a password for one returns `PasswordNotRequired`.
  Check `spectra wallet import --no-password` and `wallet export`; CLI acceptance
  verifies sealed/unsealed paths and wrong-password refusal.
- **Private-key imports:** disagreeing chain lists became the registry's
  capability flag and one derivation call. Unsupported chains are refused
  before saving a key; export handles private keys as well as phrases.
  Check `spectra chains --json` and `wallet import --private-key-file` in CLI
  acceptance, including supported Decred and refused Cardano imports.
- **Watch imports:** incomplete input lists and address-slot mappings became a
  registry-driven picker with core address validation. Ethereum Classic uses
  its own slot, and Monero watch-only import is refused. Draft reset clears all
  address inputs. Check `spectra wallet watch` and `chains --json`; the input
  layout/reset is checked on iOS.
- **Address validation:** the send UI refused prefixless Sui addresses that the
  store normalized and accepted. Validation now accepts the normalized Sui
  form consistently. This does not waive EVM mixed-case checksum validation.
  Check `spectra address validate`; Rust's
  `a_sui_address_without_its_prefix_is_accepted_either_way` covers that case,
  and CLI acceptance covers EIP-55 refusal.
- **Keypool ownership:** caller baselines and detached writes became core-owned
  reservations and owned-address records; startup loads before reserving and
  deletion clears memory and disk. Display reads do not reserve. Receive
  reservations remain stable until released; change reservations consume an
  index. Check `spectra pool next`, `next-change`, and keypool concurrency tests.
- **UTXO discovery:** inconsistent balance/UTXO/history probes and a mainnet-only
  Swift list became one activity rule for every registry-supported network.
  Password-sealed wallets return known addresses without deriving new ones
  when no password is available. Check `spectra pool discover <wallet>`
  (network needed for the walk); CLI acceptance covers the offline cases.
- **Receive reservation:** five Swift steps became `utxo_receive_address` and
  `advance_used_utxo_reservations` in core to remove caller interleaving between
  release and reservation. Deep-UTXO receive indices start at 1. Check
  `spectra pool next <wallet>`; CLI acceptance checks the floor.

### Sending and refresh

- **EVM history refresh:** eight steps stood on the front end's side of the
  boundary — plan the wallets, group them by normalized address, reset or
  advance each group's page, build the token descriptor list from its mirror of
  the token preferences, fetch the page, plan the records, convert them, merge.
  `refresh_evm_chain_history` does all eight over the wallets, preferences and
  history cursors core already owns, and answers with what changed, whether
  every group reported a short page, and one diagnostics row per wallet — which
  source answered, how many transfers, what failed. The front end keeps the
  rows and the banner, which are its screen. Three exports lost their only
  caller and became ordinary functions core calls itself:
  `core_evm_refresh_targets`, `history_evm_native_asset` and
  `plan_evm_transaction_records`. Check `cargo test -p spectra_core
  history_refresh` — token descriptors, the per-wallet failure accounting for a
  chain no explorer serves (offline: the URL builder refuses before any
  request), the non-EVM refusal and the empty case. The fetch and pagination
  themselves need network: an `Open` history source's base URL comes from the
  registry rather than from the endpoint table, so there is no way to point one
  at a local mock. The Swift implementation this replaces had no test at all.

- **Normalized history refresh:** fetching one chain's history was four steps
  on the front end's side of the boundary. It mapped its wallet projection into
  a planning request and handed that back for core to filter; fetched each
  target; built a transaction record per entry — minting the id, naming the
  wallet, stamping the source; and sent the result back to be merged.
  `refresh_chain_history` does all four where the wallets are, and answers with
  what changed: wallets refreshed, wallets whose provider failed, records added
  and updated. A provider failure for one wallet is counted rather than raised,
  so a partial refresh still merges what answered — which is what the front
  end's "loaded with partial provider failures" banner already said. Core mints
  the record id as a dashed v4 UUID, because a front end reads ids back with
  `UUID(uuidString:)` and silently drops a row whose id does not parse. Which
  network a wallet is on and which address it fetches with is now one rule,
  `WalletSummary::active_address`, shared with the balance refresh; the two used
  to work it out separately. Check `spectra history <wallet> --save` (network:
  it fetches, merges and prints the counts, and `spectra txs` then shows the
  rows) and `cargo test -p spectra_core history_refresh` for targets, network
  selection, record shape and the empty and unknown-chain cases offline. The
  multi-address UTXO and EVM paths still plan in Swift.

- **What the balance refresh refreshes:** the engine's entry list — one
  `(chain, wallet, address)` triple per wallet — was built by whichever front
  end was driving. iOS mapped its wallet projection and resolved each address
  by deriving it from the seed, dropping a wallet it could not resolve with a
  `print` and no other trace; the CLI built its own list beside it and
  disagreed twice, taking any wallet's `xpub` as the fetch key rather than only
  Bitcoin's, and the wallet's first stored address rather than the one for the
  network it is on. `BalanceRefreshEngine::sync_entries` builds the list from
  the wallets core holds, and both front ends ask for it. Check `spectra
  refresh` and `spectra refresh --wallet <wallet>` (network), and `cargo test -p
  spectra_core refresh_entry_tests` for the network, xpub, missing-address and
  EVM-family cases offline.


- **Replaceable pending sends:** two Swift filters over the app's own
  transaction projection decided which pending send could be sped up or
  cancelled, and both spelled the chain as the string `"Ethereum"`. Core now
  answers with `replaceable_sends` — an EVM chain by the registry's family, a
  send, still pending, with a hash to read its nonce by — so the actions appear
  on every EVM chain, and the replacement is signed for the pending send's own
  chain rather than for Ethereum mainnet. Three further changes come with it.
  Speed Up is offered only when the pending send moved the chain's gas asset:
  a token transfer cannot be rebuilt from a record, and composing one used to
  offer a *native* transfer of the token's amount to the token's recipient —
  speeding up a 100 USDC send offered to send 100 ETH. Cancel is unchanged and
  still offered for both, since it is a zero-value self-transfer at the same
  nonce. The composer's Speed Up/Cancel now act on a pending send on the chain
  the composer is showing, not on whichever pending send the wallet had. And the
  replacement fee starts from the chain's live estimate bumped 20% instead of a
  fixed 4/2 gwei pair, which was under the base fee on some chains and far over
  it on others; the constants remain only for when no estimate loads. Check
  `spectra txs --replaceable` (offline, and with `--wallet`), `cargo test -p
  spectra_core replaceable` for the rule, chain family, token/native split and
  store reads, and `ReplaceableSendBridgeTests` across the async binding.
  Recording a pending send needs a broadcast, so CLI acceptance covers the empty
  answer and the wallet filter, not a live replacement. One export was added;
  it removed the Swift rule and the two filters.

- **EVM composer controls:** custom EIP-1559 fees and the manual nonce were
  offered on every EVM chain but resolved only when the selected chain was named
  "Ethereum" — `customEvmFeeConfiguration` returned nothing elsewhere, so fees
  typed on Arbitrum or Base were parsed, reported as applied, and dropped, and
  selecting another asset cleared both toggles on the 22 EVM chains that accept
  them. What the composer shows is now what is applied: the fields are cleared
  when the selection leaves the EVM family, which is `Chain.isEVM` and the same
  family `EvmSendOverridesInput::resolve` validates against, and no second chain
  test stands between the entered value and the send. Check `spectra send fees
  --max-fee 30 --priority-fee 1` and `send overrides --nonce 0` for the shared
  parsers; the composer gate is iOS-side, covered by `EvmCustomFeesBridgeTests`
  and `EvmNonceBridgeTests` for the parsers behind it.

- **Manual EVM nonce:** a Swift reparse and Int32-only validation became one
  core decimal parser returning the nonce. The range now matches the signed
  Int64 preview/FFI fields; signed, fractional, hexadecimal and overflowing
  text is refused. Invalid manual input throws instead of becoming an automatic
  nonce, and previews reject negative caller/RPC nonces. Check `spectra send
  overrides --nonce 2147483648` and refusal of `--nonce +1`; CLI acceptance and
  Swift binding tests cover the parser and its errors.

- **EVM override validation:** access lists were discarded, malformed calldata
  became absent, and negative nonce/gas values wrapped to unsigned integers.
  `EvmSendOverridesInput::resolve` now validates the chain, numeric signs, hex
  bytes and access-list schema/address/storage-key sizes, and carries accepted
  bytes into both native and token sends. Explicit empty calldata stays distinct
  from absent calldata. Custom calldata or a non-empty access list requires an
  explicit gas limit, avoiding the default native limit or an estimate for
  different token calldata. Unknown chains and malformed overrides fail before
  key derivation or network reads. Check `spectra send overrides --nonce 0
  --gas-limit 50000 --calldata 0x0102`; invalid inputs exit 3. Core and CLI tests
  cover validation and builder propagation; gas sufficiency and live broadcast
  remain outside these offline checks. No new FFI export was added.
- **Custom EVM fees:** core previously returned validation advice, accepted
  infinity, and left Swift to parse again. `parse_evm_custom_fees` now returns
  the configuration or a typed error. Both fees must be finite, at least one
  wei, within the current u64-wei conversion range, and max must cover priority.
  Preview and send construction enforce the same checks, preventing zero or
  saturated fees when a caller bypasses the parser. Whole-wei rounding remains.
  Check `spectra send fees --max-fee 30 --priority-fee 1`; `inf`, sub-wei and
  overflow values exit 3. CLI acceptance, direct send-builder regression tests
  and `EvmCustomFeesBridgeTests` cover the paths without a broadcast.

- **Routing and gates:** preview/submit paths used different lists; some submit
  branches bypassed shared biometric/risk checks or sent unroutable tokens.
  Core now derives routing from its wallets, registry and tracked contracts;
  Swift follows the preflight route and shared gates. Check
  `spectra send assemble` and routing tests; biometric/broadcast paths still
  need app/testnet verification.
- **Affordability:** callers checked amounts and fees using separate rules,
  sometimes confusing the catalog ticker with the gas asset. Core now checks
  amount plus fee for native sends and the separate gas balance for tokens.
  Check `spectra send affordability --chain Arbitrum --symbol ARB --amount 1
  --fee 0.01 --balance 2 --gas-balance 0`; the fee is in ETH, not ARB.
- **Recipient risk:** per-chain probes sometimes checked the wrong asset or
  treated spent Bitcoin addresses as having no history. Core checks the chosen
  token/native asset and actual history; unknown chains error and unknown
  assets are not probed as a different asset. Swift uses one localized message
  pair. Check `spectra send probe --chain <chain> --address <address>` with token
  options when needed; verdicts require network, refusals have offline checks.
- **Bittensor and Monero:** Bittensor was unroutable despite having fee and
  signing support; it now uses the shared path with its own preview tag.
  Monero uses that path with core's default priority and its stored signing
  material, without requiring a seed derivation path. Check routing and
  `monero_takes_the_shared_submit_path` tests; live broadcast remains untested.
- **Concurrent sends:** a guard separated from insertion by awaits allowed two
  sends through. The shared runner now checks and claims the in-flight flag
  without suspension. This is an audited Swift fix, not covered by the offline
  CLI gate; verify with controlled app/testnet sends.
- **Post-send refresh:** Dogecoin omitted balance refresh and verification
  updates; it now runs the shared actions. UTXO testnets use their registry
  pending-poll policy instead of falling through to history refresh. Check
  `every_utxo_testnet_polls_the_way_its_mainnet_does`; observing the Dogecoin
  result requires a broadcast.
- **EVM pending status:** a recursive refresh arm became a real receipt poll.
  Reverted receipts resolve to failed instead of being mistaken for successful
  history entries. Check Swift's `testAnEVMPendingRefreshTerminates`; live
  receipt handling is outside offline CLI acceptance.
- **Refresh and CLI output:** fire-and-forget refresh gained awaited
  `refresh_now`, so `spectra refresh` completes before the process exits.
  Logging moved off stdout so `--json` stays parseable; self-test failures emit
  one document. Check CLI acceptance and `spectra diagnostics self-test`.

### Tor routing and the display currency

- **Tor settings:** the four values that decide whether traffic goes through
  Tor — on/off, embedded client vs the user's own SOCKS5 proxy, that proxy's
  address, and the kill switch — lived in one front end's `UserDefaults`. No
  other front end, no test and no script could read or set them, which is the
  one thing "no platform persists a second authoritative copy" rules out. They
  are `AppSettings` fields now, set through `SetAppSetting` like every other
  setting. The address is validated where it is stored: a value that is not a
  `socks5://` or `socks5h://` URL with a host and a port is refused and nothing
  changes, because the HTTP layer fails closed on a proxy it cannot build — so
  storing a typo used to mean every request quietly failing with no reason
  given. An empty value restores the default port. Check `spectra settings set
  tor-enabled true`, the refusals in CLI acceptance's "tor routing" section, and
  `cargo test -p spectra_core tor_settings_persist_and_refuse_an_address_that_is_not_socks5`.
- **The kill switch had no reader.** Its own settings screen promised that
  "network requests are paused if the Tor circuit drops instead of falling back
  to a direct connection"; nothing anywhere read the value. Core enforces it
  now: while Tor is wanted, the switch is on and Tor is not carrying traffic,
  `fetch::http` refuses the request with that reason, and the one function every
  caller takes its client from hands back the blocked client so a path that
  skips the check still cannot send in the clear. The rule is asserted over
  values rather than the process-wide flag — `the_kill_switch_engages_only_while_tor_is_wanted_and_not_ready`
  — because a test that engaged it would refuse every other test's HTTP call.
  Live behaviour with a real circuit still needs the app.
- **Display currency:** `SetFiatCurrency` stored whatever string it was handed,
  so `spectra currency ZZZ` set the display currency to `ZZZ` and every amount
  then rendered unconverted beside that code. The twelve codes the app quotes in
  are `FIAT_CURRENCY_CODES` in core; anything else is refused with an event, and
  the CLI turns that into exit 3. A settings update core cannot store now
  reports the same way — an unknown chain or a bad proxy address used to change
  nothing and say nothing, leaving the caller to notice that the read-back was
  the old value. Check `spectra currency ZZZ` (exit 3) and CLI acceptance.
- **Fiat cross-rates:** iOS fetched them, merged them through an export that
  took its own copy of the stored rates back, and wrote the result to a SQLite
  blob of its own, seeded at launch from an older `UserDefaults` key that still
  won the race. They are `CoreAppState.fiat_rates_from_usd` now, with their own
  persistence row, and `refresh_fiat_rates` is one operation: fetch, merge
  against what core holds, store, publish. A provider failure leaves the stored
  rates alone. `price_merge_fiat_rate_updates` had no caller left and is gone.
  Check `spectra currency --rates` (offline, reads the store) and
  `--refresh-rates` (network), plus `cargo test -p spectra_core
  stored_fiat_rates_survive_reopening`.

### Diagnostics

- **Per-chain diagnostics wiring:** an eight-row table in Swift named each
  chain's history driver and its own address resolver, and the diagnostics hub
  dispatched through it. Six rows had become the generic run written out —
  `resolvedAddress(for:chainName:)` resolves every chain, the EVM family
  included, and which fetch to make is `chain.isEVM` — so the table is gone and
  Bitcoin (an xpub history page) and Monero (its configured backend) are the two
  cases left, each a line in a switch. Dogecoin's row carried no per-wallet
  entry, so a per-wallet Dogecoin run did nothing; that path had no screen at
  all and is deleted rather than fixed, along with the four wrappers that passed
  a chain's own name back to the catalog probe. Check `spectra endpoints --chain
  Dogecoin` for the catalog side; the runs themselves are iOS screens and need
  network, and `DiagnosticsChainTableTests` still pins the hub, bundle and
  screens to one chain list.
- **Run All Endpoint Checks:** the button called twenty-two hand-written chains,
  six of them through a per-chain wrapper. The list predated Base, Polygon,
  Zcash, Kaspa, Dash, Decred, Bitcoin SV, Bitcoin Gold, Dogecoin, Sei and the
  newer rollups, so "all" skipped them. It is now `Chain.mainnets`, the same
  list the hub and the export bundle read. A chain with no catalog endpoints
  publishes no rows, as before.
- **Chain self-tests:** the self-test button lived inside the UTXO block, so the
  suite core keeps for *every* chain in the catalog was reachable on five chains
  — plus Ethereum, which had a second button running a near-copy of the same
  bookkeeping with three extra probes. Every chain's screen now offers the
  suite. Of the three extras, the EVM RPC probe stays and applies to the whole
  family: `self_tests_run_ethereum_rpc` compared the node's reported chain id
  with a literal `1` and labelled every row "Ethereum"; `self_tests_run_evm_rpc`
  takes the chain and compares against `Chain::evm_chain_id`, refusing a
  non-EVM chain instead of probing it. The endpoint comes from the chain's
  configured RPC or its catalog list, not from a hard-coded mainnet URL. The
  other two extras are deleted on purpose: the JSON-shape check tested core's
  own document builder, which `a_document_carries_history_and_endpoints` now
  asserts where the document is built (`core_diagnostics_json_shape_ok` is gone
  with it), and the portfolio probe was the balance refresh with a different
  error message. Check `spectra diagnostics self-test` for the offline suite and
  `cargo test -p spectra_core only_an_evm_chain_has_a_json_rpc_id_to_check` for
  the refusal; the RPC probe itself needs network.

### Networks, assets and UI

- **Presentation catalog:** token colors previously required a Swift hosting-chain
  mapping and a full unused display record; they now come directly from core's
  token catalog into a color cache. Native-chain colors still take precedence.
  This removes a platform-specific filter from asset styling. Six unused
  copy fields and their four locale variants were surplus and have been removed,
  including the outdated private-key support claim. Check catalog metadata with
  `spectra token catalog --chain Ethereum`; color lookup and resource decoding
  require the Swift tests, not the CLI.

- **Network selection:** duplicated network-mode models became registry chain
  ids in `network_chain_by_family`. Endpoint indexing separates per-network
  RPC selection from per-chain settings groups, preventing testnet/mainnet
  mixing. Check `spectra network` and iOS test
  `testEthereumTestNetworksExposeExpectedContextsAndEndpoints`.
- **Staking:** separate support lists became `Chain::supports_staking` and a
  shared service gate. Unsupported chains get a staking refusal; testnets do
  not route to mainnet validators. Async exports declare the Tokio runtime.
  Check `spectra staking validators --chain Bitcoin`, registry routing tests,
  and the staking tab through Swift bindings.
- **Endpoint catalog:** mixed `roles` became `kind` plus `capabilities` in
  `core/data/endpoints.toml`. EVM RPC nodes do not claim address-history access;
  web links are not probed. Supplemental RPC-list registration is explicit.
  Settings show available tags and include chains with actual catalog entries,
  including previously hidden Bitcoin SV. Check `spectra endpoints --chain
  Ethereum` (network); CLI acceptance checks catalog shape.
- **Endpoint health:** unchecked stale URLs became a CLI sweep, with JSON-RPC
  error-body inspection and a retry before reporting failure. Dead entries
  were removed or replaced; row-specific reasons stay in `endpoints.toml`.
  Check `spectra endpoints --json`; availability must be remeasured live.
- **EVM history:** a blanket Etherscan-key requirement and swallowed refusals
  became registry-selected open/keyed/unavailable sources. Errors are distinct
  from empty arrays, and EVM records now reach normalized history. Check
  `spectra chains --json` for source metadata and `spectra history` with a
  watch wallet on a configured open source (network).
- **Asset identity:** dashboard rows previously grouped by chain; they now
  group by asset with a per-chain breakdown. The wiki has asset pages linked to
  chain/contract information even for assets not held. Token decimals come from
  contract/account data rather than caller assumptions. Inspect
  `spectra portfolio` and token balances; wiki layout needs iOS verification.
- **Artwork:** chain-qualified identifiers hid token logos and matched token
  symbols by substring. `core_icon_asset_name` now resolves exact symbols from
  chain, token and gas-asset catalog metadata; chain badges still draw the chain.
  Both wikis use their row's asset name, and USDB artwork now ships. Check
  `cargo test -p spectra_core artwork_follows_the_coin_not_the_chain` and Swift's
  `CoinBadgeArtworkTests`, which loads the actual bundled assets. The Rust tests
  specify the resolver; the Swift test covers the original missing-image bug.
- **Purchase directory:** provider names, descriptions and links scattered
  through locale files became `resources/BuyProviders.json`, grouped into
  on-ramps and exchanges. Rows show names/domains; localized category notes
  explain custody. Banxa's FAQ link and Ramp's redirect were corrected.
  JSON ids replace per-render UUIDs to keep rows stable. Check the directory
  in iOS: MoonPay and Kraken were previously opened successfully. The reported
  “none of the links work” issue was not reproduced on the old clean simulator;
  unstable row identity remains the proposed explanation, not proven causation.
