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
The asset wiki and its follow-up cleanup are also complete.

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
  coverage. Audit callers before counting coverage; the previous audit named
  `core_ethereum_custom_fee_validation` as one such funds-path rule.
- **Decred and Kaspa mnemonic vectors:** no independent known-mnemonic →
  known-address test was identified. Existing address-validation and Decred
  private-key-import checks do not substitute. Use published or independently
  derived vectors, not this implementation's output as its own expected value.
- **Dead Swift file:** `swift/LoadingTaskRegistry.swift` has no callers but is
  still in the Xcode project. Remove the file and project references together.
- **Unused presentation data:** review `TokenVisualRegistryEntry`'s unread
  fields and unused translated `StaticContentCatalog` properties. Decide whether
  the UI is missing content or the data is surplus before deleting it.
- **Endpoint probes and redundancy:** the earlier sweep flagged probe URLs for
  Bitcoin SV, Internet Computer and Zcash, plus chains with only one RPC node.
  Recheck through `spectra endpoints` before changing rows; recorded failures
  may be bad probe configuration rather than an unavailable service.
- **EVM history availability:** the registry distinguishes open indexers,
  Etherscan V2 requiring a key, and unavailable sources. Remaining key-dependent
  chains need supported infrastructure if keyless history is required. Do not
  silently depend on another wallet's private backend.

## Behaviour changed on purpose

Keep entries to the previous behaviour, the new behaviour, the reason and a
CLI check. If a check needs network, a simulator or new coverage, say so.
Completed refactor diaries and old test/line counts do not belong here.
The entries below summarize the retained decisions, not a fresh test run.

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

### Networks, assets and UI

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
