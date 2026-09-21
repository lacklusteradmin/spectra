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
| 3 — Thin the shell | Done | Audited alert, send/preview, replacement, address and self-send decisions owned by core; transport lifecycle and destination activity closed in the follow-up review; UI draft abstraction removed |
| C1 — Reshape core | Done | Shared chain catalog, service modules split by responsibility, duplicate modules and derivation primitives consolidated |
| C2 — Reduce the FFI surface | Done | Owned operations and coherent snapshots replace caller-assembled decisions |
| 4 — Android | Not started beyond skeleton | Implement against the shared core once the boundary is ready |

### Boundary rules for future work

- Find Swift code that reads core-owned data only to send it back for a decision;
  make the owning service compute the answer instead.
- Keep protocol-specific preview inputs where they differ. Do not collapse them
  into one wide enum merely to reduce a count.
- Keep one writer per UI projection. Adopt core-derived answers asynchronously;
  local indexes and button-enabling checks may remain view state, with core
  enforcing validation on writes.
- Include views and record extensions when auditing hand-written Swift.
- Delete projection/cache fields when their last reader disappears.
- Keep concrete network, token and deployment identities distinct; never infer
  identity from ticker or price-provider identity.
- Remove dead wrappers only after checking direct FFI callers and foreign
  callback implementations. Delete tests of removed helpers only when the
  replacement's meaningful coverage is identified.

## How progress is measured

Use reproducible checks rather than retaining per-session counts:

- `scripts/count-exports.sh`: callable FFI surface, with a working target of
  roughly 150. Cross-check macros against generated bindings and exclude
  converter helpers. Do not merge unrelated operations merely to reduce counts.
- `scripts/unreachable-exports.sh`: unused export candidates.
- Domain collections and decisions must have one owner; new operations must be
  reachable through the CLI and state must survive reopening the database.
- Compare non-generated Swift orchestration with UI code to locate remaining
  debt. Line counts are diagnostic, not a reason to relocate code artificially
  or keep dead views. An export that removes a Swift rule can be worthwhile.

Run the full verification gate before calling a change done:

```sh
make verify
```

This runs formatting, clippy, workspace Rust tests, offline CLI acceptance and
an iPhone simulator test suite. There are no expected-red iOS tests, including
`testEthereumTestNetworksExposeExpectedContextsAndEndpoints`.
CLI acceptance uses a throwaway directory without network. Tests must assert
rules rather than the ordering of concurrent failures. Exercise changed FFI/UI
paths in the app too: CLI tests cannot detect a missing Tokio runtime on a
Swift async export, and offline assembly cannot verify a broadcast.

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
    before/after behaviour and concrete CLI checks in
    [docs/BEHAVIOUR-CHANGES.md](docs/BEHAVIOUR-CHANGES.md).
- [ ] **The CLI cannot show the endpoint table it is about to use.** Every
  `spectra` command builds its service through `WalletService::new_catalog()` →
  `catalog_endpoints()`, but nothing prints the result. `spectra endpoints`
  probes the catalog's rows for a chain, which is a different list — it answers
  "which registered endpoints are up", not "which ones will this command try,
  in what order". Add an offline subcommand that prints the configured table
  per chain, in order, and cover it in `scripts/cli-acceptance.sh`.
- [ ] **Detect FFI record fields with no production writer.** A syntactic scan
  cannot reliably distinguish unwritten fields from serde or multi-line writes.
  A useful gate needs type-aware analysis before it can reject unused fields.
- [ ] **Staking transaction execution.** The app currently provides information
  and validator queries. Transaction execution remains future work.
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
