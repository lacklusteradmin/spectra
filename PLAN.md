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
> - **Silence.** Every behaviour change goes in
>   [docs/BEHAVIOUR-CHANGES.md](docs/BEHAVIOUR-CHANGES.md): what it was, what it
>   is, why that side, and how to check it without the app. A change nobody can
>   find is not reversible.
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
| 3 — Thin the shell | Done | Audited alert, send/preview, replacement, address and self-send decisions now owned by core; obsolete projection mutations removed; see closure audit |
| C1 — Reshape core | Done | Shared chain catalog, service modules split by responsibility, duplicate modules and derivation primitives consolidated |
| C2 — Reduce the FFI surface | Done | 157 callable exports, zero unreachable candidates; owned operations replace caller-assembled send decisions |
| 4 — Android | Not started beyond skeleton | Implement against the shared core once the boundary is ready |

Other completed ownership slices: settings, token preferences, price alerts,
keypool, owned addresses, operational events, refresh scheduling, send routing,
recipient checks, dashboard grouping and transaction-derived data. UTXO discovery
and receive reservation now run in core through the shared secret layout.
The asset wiki and its follow-up cleanup are also complete. The unused
`LoadingTaskRegistry` and its Xcode references have been removed.

### Boundary rules for future work

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
expected-red iOS tests, and no flaky ones: a test that asserted which of two
concurrent reads reported its error first passed alone and failed under a
loaded parallel run, which is the run the gate does. Assert the rule, not the
race — mock the reads the test is not about so they succeed. Exercise changed FFI/UI paths in the app too: CLI tests
cannot detect a missing Tokio runtime on a Swift async export, and offline
assembly cannot verify a broadcast.

## Network and token identity rewrite (2026-09-12)

Approved scope: retain `chains.toml` and `tokens.toml`; separate concrete
networks from token identities and their network-specific deployments. The catalog contains
46 mainnets, 32 testnets, 131 token identities and 268 deployments (78 native).

- [x] Put mainnets and testnets in equal explicit network records; remove
  testnet inheritance and ambiguous network ticker. Keep stable network ids.
- [x] Give tokens independent ids and typed native/protocol deployments;
  register native tokens in `tokens.toml`, move precision and market identity
  out of network data, and explicitly separate testnet token identities.
- [x] Validate catalog references, unique deployment identities and protocol
  identifiers. Never infer identity from ticker or price-provider identity.
- [x] Bind stored holdings, balance refresh, sends, fees and transaction
  context to concrete network/deployment identity; core derives metadata.
- [x] Use explicit token identity for portfolio grouping and quotes; keep
  wrapped/bridged tokens distinct and testnet tokens unpriced.
- [x] Update CLI and Swift projections, network/token selection and icons;
  regenerate UniFFI 0.31 / Swift 6 bindings, remove superseded inference.
- [x] Add offline CLI coverage for equal network listing, native ETH/BTC,
  native versus ERC-20 MNT, ARB versus ETH fees, duplicate symbols, cross-network
  balances and unpriced testnets. Record purposeful behaviour changes below.
- [x] Pass `cargo test --workspace`, `./scripts/cli-acceptance.sh` and the
  required iPhone 17 Pro `xcodebuild test` suite.

Verification completed 2026-09-12: workspace Rust tests **843 passed**; full
CLI acceptance **347 passed**, including offline identity and fixture suites;
iPhone 17 Pro tests **84 passed**, including
`testEthereumTestNetworksExposeExpectedContextsAndEndpoints`.
`git diff --check` and `scripts/unreachable-exports.sh` also pass.

No compatibility shims or Git state changes. Network grouping is display
metadata, not transaction identity; protocol/endpoint facts stay core-owned.

## Known open items

- [ ] **Transparent send stages: build, sign, broadcast (core, CLI and Swift).**
  Split the current combined signing/submission path into explicit core-owned
  operations with typed prepared transactions, signed transactions and
  broadcast results. Keep `execute_send` as a convenience that composes these
  operations where appropriate. Chain-specific payloads and capabilities remain
  explicit; do not force every protocol into an identical internal workflow.
  This supports Spectra's transparency ethos and future external/offline
  signers, while making each stage independently inspectable and testable.
  - Core owns the prepared/signed transaction lifecycle, validation and any
    durable state needed to resume after restart. Bind the reviewed content to
    the transaction actually signed; validate network, signer, amounts, fees
    and chain-specific freshness constraints (nonce, UTXOs, blockhash/expiry).
    If rebuilding changes the reviewed transaction, require fresh review and
    signing rather than silently replacing it. Never persist private keys in
    transaction artifacts.
  - Split Swift's send flow into visible Build, Sign and Broadcast steps with
    explicit user actions and inspectable core-derived details/results. Show
    what will be signed, when signing has completed without submission, and
    which nodes received the transaction. Swift owns navigation/view state;
    core owns transaction state and the rules for advancing each stage.
  - Let users select one or more compatible broadcast endpoints and inspect
    the selected network, endpoint URLs and submission outcomes. Core validates
    endpoint network and broadcast capability before submission and uses only
    the selected destinations; no silent fallback to unselected providers.
    Keep chain capability facts in the registry and endpoint configuration in
    core. Explain that selected nodes can propagate the transaction onward.
  - Broadcast/retry the same signed payload, track outcomes per endpoint, and
    distinguish node acceptance, uncertain submission and on-chain confirmation.
    Partial success or a timeout must not trigger an automatic new transaction.
  - Add CLI operations for each stage and endpoint selection, with offline
    acceptance coverage using mock nodes for separate build/sign/broadcast,
    stale or altered artifacts, incompatible endpoints, partial submissions,
    identical-payload retries and reopening persisted state. Prove core rules
    before replacing Swift orchestration, regenerate UniFFI bindings, exercise
    the new UI, and pass all three required suites. Record implemented
    before/after behaviour and concrete CLI checks under “Behaviour changed on
    purpose”; this item is planned work, not a completed behaviour change.
- [ ] **The CLI cannot show the endpoint table it is about to use.** Every
  `spectra` command builds its service through `WalletService::new_catalog()` →
  `catalog_endpoints()`, but nothing prints the result. `spectra endpoints`
  probes the catalog's rows for a chain, which is a different list — it answers
  "which registered endpoints are up", not "which ones will this command try,
  in what order". That gap is why the role-bit collision above went unseen: the
  wrong endpoints were in every CLI invocation's fallback chain and no
  subcommand could show them, so the behaviour change had to be measured by
  calling `catalog_endpoints()` from a test rather than by a CLI check. An
  offline subcommand that prints the configured table per chain, in order,
  would make this class of defect visible to `scripts/cli-acceptance.sh`.
- **Endpoint availability and redundancy:** ICP's Rosetta POST probe and the
  skipped Bitcoin SV explicit probe are fixed. Zcash's registered Trezor API
  still returns HTTP 403; Bitcoin SV's Blockchair provider remains unavailable
  in the live check. Single-provider chains still need independent supported
  providers. A successful health probe does not prove every capability works.
  The user explicitly retained these as external dependencies on 2026-09-12.
- **EVM history availability:** the registry distinguishes open indexers,
  Etherscan V2 requiring a key, and unavailable sources. Remaining key-dependent
  chains need supported infrastructure if keyless history is required. Do not
  silently depend on another wallet's private backend. Base now uses its verified
  public Blockscout source; the user retained the remaining providers as external
  dependencies on 2026-09-12.


Resolved on 2026-09-12: independent Decred and Kaspa mnemonic/address vectors,
ICP HTTP protocol and Bitcoin SV explicit probe selection, app-only rule
coverage, and Base keyless history. The CLI-signed Sepolia self-transfer was
accepted and mined successfully — 21,000 gas used, 0.000022431561432 Sepolia ETH
fee, [transaction](https://sepolia.etherscan.io/tx/0x7c96dddb0ae86e2ee91dab7b0aeb0de7a4b6acaaf9848c07c7666a9bf7475555).
This closes the original EVM broadcast proof; it does not complete the newly
planned visible build/sign/broadcast workflow or establish broadcasting on every
chain.


## Stage 3 / C2 completion work (requested ten items)

- [x] Core applies and persists native/token balance refreshes; Swift adopts projections.
- [x] Core owns wallet deletion, including failure/retry semantics.
- [x] Core owns outgoing transaction records through broadcast.
- [x] Core owns Funds Finder scanning and reports failed reads.
- [x] Core owns durable diagnostics state.
- [x] Derived wallet state reads signing availability through SecretStore.
- [x] Balance refresh boundary accepts wallet scope rather than caller token lists.
- [x] Stored transaction ID drives rebroadcast and record updates.
- [x] Network selection and derivation reset commit together.
- [x] Chain diagnostic operations replace caller orchestration and JSON state blobs.

Verified: `cargo test --workspace` (788 core tests), `./scripts/cli-acceptance.sh`
(332 checks plus the Stage 3 local fixtures), and the required iPhone 17 Pro
simulator suite (80 tests). Export scripts report 182 callables and zero
unreachable candidates. These ten slices are complete; the broader stage
status and known open items above are unchanged.

## Stage 3 / C2 follow-up (requested six items)

- [x] Import and SecretStore coordination, cleanup and retry.
- [x] Wallet rename and portfolio inclusion as field intents.
- [x] Core quote cache and price/fiat refresh policy.
- [x] Core-derived transaction maintenance scope.
- [x] Stored-asset EVM preview; no seed reads for fee preview.
- [x] FFI caller/coverage audit, CLI entry points and bridge cleanup.

Verified: `cargo test --workspace` (798 core tests), `./scripts/cli-acceptance.sh`
(332 checks plus both Stage 3 fixture batches), and the required iPhone 17 Pro
simulator suite (81 tests, zero failures). The FFI surface is 179 callables,
with zero unreachable export candidates. Broader Stage 3/C2 items remain open.

## Stage 3 / C2 follow-up: pending maintenance ownership

- [x] Core runs the full stored-network pending-status sweep and tracker cleanup.
- [x] Swift adopts committed changes; pending polling no longer uses UI descriptors.
- [x] CLI `txs --refresh-pending` drives the same operation; scope inspection remains.
- [x] Remove scope-query and tracker-pruning FFI exports, add the sweep operation,
  regenerate bindings and exercise the Swift async bridge.

Verified: `cargo test --workspace` (822 core tests), `./scripts/cli-acceptance.sh`
(334 checks plus Stage 3/follow-up fixtures), and the required iPhone 17 Pro
simulator suite (82 tests, zero failures). FFI: 178 callables, zero unreachable
candidates; generated Swift has neither removed method. Stage 3/C2 remain in
progress. Remaining observed orchestration includes manual transaction-status
recheck and history-refresh dispatch; this is not an exhaustive remaining list.

## Stage 3 / C2 follow-up: manual transaction recheck

- [x] Core validates the stored transaction, reads its recorded network and
  commits only its status; explicit rechecks include failed/confirmed records.
- [x] Refuse provider errors, mismatched hashes and deleted/changed identities;
  preserve concurrent metadata and resume tracking after an unconfirmed read.
- [x] CLI `txs --recheck <id>` and Swift use one operation. Remove the low-level
  tracker-reset export, regenerate bindings and test the async bridge.

Verified: `cargo test --workspace` (826 core tests), `./scripts/cli-acceptance.sh`
(337 checks plus Stage 3/follow-up fixtures), and the required iPhone 17 Pro
simulator suite (83 tests, zero failures). FFI remains 178 callables with zero
unreachable candidates. Manual recheck from the previous remaining list is
complete; history-refresh dispatch and broader Stage 3/C2 audit remain open.

## Stage 3 / C2 closure after the Swift ownership follow-up

The previous closure was reopened after finding Swift-owned alert mutations,
send decisions, address fallbacks and self-send inputs. The follow-up also
removed Swift replacement assembly, local-first projection merging, background
orphan deletion and test-only mutation helpers from the app target.

Final verification on 2026-09-12:

- `cargo test --workspace`: 851 core tests, zero failures.
- `./scripts/cli-acceptance.sh`: 347 checks plus Stage 3/follow-up fixtures,
  including `cli-owned-send.py`, all passed.
- Required iPhone 17 Pro `xcodebuild test`: 84 tests, zero failures, including
  Ethereum testnet contexts/endpoints and the new alert-intent async binding test.
- 157 callable FFI exports, zero unreachable candidates; generated bindings
  contain none of the removed low-level preview/self-send/replacement helpers.
- `git diff --check` passes. Changes remain uncommitted.

Earlier progress entries describe their time of writing; this audited follow-up
supersedes their remaining Stage 3/C2 work notes. Android remains a separate stage.

## Swift shell ownership follow-up (2026-09-13)

- [x] Receive uses only the owned optional address, with separate loading/errors
  and stale-response protection. No message can become QR/copy/share payload.
- [x] Core allocates unnamed imports under the serialized writer; remove the
  caller's default-name start index and CLI's different naming defaults.
- [x] Core owns and persists movement baselines, observations and notification
  decisions. Swift supplies device activity and performs notification delivery.
- [x] Staking queries read WalletService's current transport settings; remove the
  separate Swift endpoint builder and cached service.
- [x] Historical transaction labels use their recorded network.
- [x] Delete unused Swift helpers/caches, obsolete receive FFI planners and
  unconnected staking actions/positions/preview UI. Keep real validator queries.
- [x] Export checks exclude declarations/comments and count `pub(crate)` exports;
  move internal-only operations out of exported impl blocks.
- [x] Final verification: workspace Rust **833 passed**, offline CLI **354 passed**
  plus Stage 3/follow-up fixtures, iPhone 17 Pro **87 passed**. The required
  Ethereum test-network test and new history/movement/staking bridge tests pass.
  FFI: **145 callables** (64 free functions + 81 methods/constructors), zero
  unreachable candidates, cross-checked against regenerated Swift bindings.
  `git diff --check` and `scripts/check-design-tokens.sh` pass.

Runtime smoke on iPhone 17: the staking tab explicitly presents query-only
functionality; Solana returned 100 validators and its detail has Overview,
Validators and Learn, with no Actions tab. A public watch-only test address
(`0x1111111111111111111111111111111111111111`) imported without an entered name
as **Wallet 1**, then displayed its receive QR and enabled Copy Address. No
private keys or transactions were used. The simulator retains that read-only
sample. Error/no-wallet receive behavior is also covered through CLI/core and
Swift async-binding regression tests.

Logs: `/tmp/spectra-audit-rust.log`, `/tmp/spectra-audit-cli.log`,
`/tmp/spectra-audit-ios.log`. Changes are unstaged and uncommitted.

This supersedes the earlier closure's residual-boundary claims. Build / Sign /
Broadcast remains a separate planned project. The staking app is explicitly an
information and validator-query interface; staking transaction execution is
future work, not an implemented feature hidden behind nonfunctional buttons.

## Swift shell residue sweep (2026-09-14)

The follow-up above ticked "delete unused Swift helpers/caches". A direct
re-read of the shell found that box was ticked early: the decisions had moved,
but the state they used to feed had not gone with them. What ownership audits
look for is a *reader* — Swift reading core-owned data to decide something —
and that test is blind to the opposite failure, state the app collects and
nobody reads at all. Twelve of those were left. They are listed with their
before/after in [docs/BEHAVIOUR-CHANGES.md](docs/BEHAVIOUR-CHANGES.md) (the
unread-state entry, the Tron notice and the catalog rank); in summary:

- [x] Five `WalletDerivedCache` fields with no reader, including a four-field
  mirror of *which wallets can sign*, plus the duplicate `walletByIDString`.
- [x] Two `WalletDerivedState` columns that crossed the FFI as echoes of an
  input, and the diagnostics export's two always-nil send-error parameters.
- [x] `tronLastSendErrorDetails`/`At`, which nothing ever assigned, and the
  dashboard notice, bundle keys, `== "Tron"` tests and locale string they fed.
- [x] `activeEthereumSendWalletIDs`, `normalizedHistoryRevision`,
  `lastObservedTransactions`, `cachedResolvedTokenPreferences`,
  `cachedTokenPreferencesByChain` and an orphaned `clamped(to:)`.
- [x] The eight-id popular-chain array in `SetupView`, now `popular_rank` on the
  catalog, and the descriptor's duplicate symbol field.

What the sweep did **not** find is as much of the result: no unit conversion,
fee arithmetic or protocol constant anywhere in hand-written Swift; one owned
send operation with no execution branch beside it; no domain collection
persisted outside core beyond the four-value platform-preferences blob; and
`spectra` still drives every decision the app makes.

The standing check this adds: a projection with no reader is not a cache, it is
a second copy of core's answer going stale in the dark. Grep for a `cached…`
field's readers before adding one, and delete it when the last reader goes.

Gates for this sweep: `cargo test --workspace` **799 passed** (one new catalog
test: the popular ranking is `1..=n` with no gaps, duplicates or testnets);
`./scripts/cli-acceptance.sh` **353 passed**, including the two new
`popularRank` checks and `cli-stage3.sh`'s derived-state assertion, rewritten to
require that every field an empty store publishes comes back empty rather than
naming a column that has since gone; iPhone 17 Pro `xcodebuild test` **92
passed, zero failures**, including
`testEthereumTestNetworksExposeExpectedContextsAndEndpoints`. FFI: **144
callables** (65 free functions + 79 methods), zero unreachable candidates,
against regenerated bindings. `scripts/check-design-tokens.sh` and `git diff
--check` pass. Changes remain uncommitted.
## Swift shell: the view layer (2026-09-16)

The 2026-09-14 sweep read `swift/*.swift` and reported no fee arithmetic or
protocol constant in hand-written Swift. `swift/views/` had both, and so did the
record extensions it renders from: fixed `%.6f`/`%.8f`/`%.2f gwei` fee counts,
`"%.8f ETH"` for every EVM receipt, a twelve-chain dispatch naming chains and
their sentences, a QR payload parser, and a second derivation-path resolver.
Seven moves, each in [docs/BEHAVIOUR-CHANGES.md](docs/BEHAVIOUR-CHANGES.md):

- [x] `scanned_send_address` — QR payloads parsed and validated against a chain
  in core, with no unvalidated fallback; `spectra send scan`.
- [x] `SendBroadcastMode` on the chain identity; the network card is one branch
  per preview shape with an exhaustive switch over `SendPreview`.
- [x] Every fee and gas price through `formatting_asset_amount_display`.
- [x] Send previews keyed by mainnet id; nine chain-named accessors deleted.
- [x] `Chain.defaultDerivationPath` from `app_core_resolve_derivation_path`.
- [x] `core_history_source` names the ids core writes; `spectra --json txs`.
- [x] `new_seed_envelope_master_key` mints the Keychain master key.

The check this adds to the last one: a sweep's scope is a directory, and a
claim about "hand-written Swift" covers the views or it does not.

The rest of the same audit, also in BEHAVIOUR-CHANGES:

- [x] Settings and diagnostics sections chosen by registry facts; custom RPC on
  every EVM chain; the Etherscan key on the six chains that read it
  (`needs_etherscan_api_key`); Monero's backends from the catalog.
- [x] EVM receipt cost written to the record (`receipt_network_fee`).
- [x] Platform preferences in `UserDefaults`; `save_state`/`load_state` and the
  `state` table deleted.
- [x] No crash on a failed catalog or path lookup; no cached key candidates;
  typed `CatalogColor`; install marker, `EVMChainContext`, three unread
  endpoint columns and their pinning tests deleted; MWEB badge from core.
- [x] Gates: `swift-shell-literals.sh` reads the app for chain names and fixed
  precision; `unreachable-exports.sh` no longer counts iOS tests as callers.

Not done: a gate for FFI record fields no production code writes. A syntactic
pass over `core/src` flags 56 of 270 `Option` fields, nearly all written through
serde or multi-line merges it cannot follow, so it cannot gate without a
type-aware tool. FFI: **146 callables** (69 free functions + 77 methods), zero
unreachable.

## Behaviour changed on purpose

Moved to [docs/BEHAVIOUR-CHANGES.md](docs/BEHAVIOUR-CHANGES.md), which is where
new entries go. Rule 0's requirement is unchanged: a behaviour change that is
not written down there did not happen on purpose.
