# Future product plans

These are planned directions, not completed features. Some build on existing
capabilities; audit those before implementing replacements. Active architecture
and implementation work remains in [PLAN.md](PLAN.md), whose Rule 0 applies.

## Product principles

Spectra keeps users in control of their keys, wallet data and network activity.
Users should be able to inspect what the app did, what evidence it relied on,
and what happens next. Custody, payments and recovery take priority.

Use concise summaries with details available on demand. Meaningful action
boundaries such as Build, Sign and Broadcast deserve explicit steps; other
technical details need not become mandatory pages.

Core owns domain state, validation, persistence and decisions. Swift renders
core-derived information and forwards user intent. New rules must be drivable
through the CLI. Follow PLAN.md's verification requirements when implementing
these items and record intentional behaviour changes there.

## High priority

- [ ] **Data provenance and freshness.** Let users inspect the provider/endpoint,
  last successful update time and relevant block reference for balances, fee
  estimates and transaction status. Distinguish cached or stale data, failed
  reads and unknown values; a failed query must not look like a fresh zero.
  This is the first recommended product improvement because it makes everyday
  wallet information understandable and inspectable throughout the app.

- [ ] **Network activity controls.** Show which services handle balance,
  history, price and other requests, and whether traffic actually uses Tor.
  Allow users to disable optional network features such as price queries.
  Build on existing endpoint settings and diagnostics so users can understand
  and control routine queries as well as transaction submission. Avoid leaking
  credentials or sensitive request data through activity displays and exports.
  This is the second recommended product improvement.

- [ ] **Backup and recovery rehearsal.** Verify a backup in an isolated context
  without changing the active wallet. Check that recovery reproduces the
  expected addresses, explain which data the backup contains, and identify
  what needs to be rescanned or obtained separately. Report what was actually
  verified rather than treating a successful file export as proven recovery.

- [ ] **Transaction explanations and signing changes.** Explain outgoing
  assets, fees, change outputs and supported contract operations before signing.
  Surface unknown or undecodable operations explicitly. If a transaction must
  be rebuilt, highlight changes relative to the previously reviewed version
  and require fresh confirmation and signing. Implement this alongside or
  immediately after the transparent send stages already tracked under
  “Known open items” in [PLAN.md](PLAN.md): core and Swift Build/Sign/Broadcast
  separation, user-selected broadcast endpoints and per-endpoint outcomes.

## Medium priority

- [ ] **Explainable coin selection and address management.** For UTXO chains,
  show selected inputs, the selection rationale and the derived change
  destination. Let users freeze specific UTXOs and explain the privacy impact
  of combining inputs. Core enforces selection constraints and owns durable
  preferences; Swift presents the choices and their consequences.

- [ ] **Portable wallet data.** Offer documented exports of contacts, labels,
  history and settings so users can inspect their data independently, use it
  through the CLI, or leave Spectra. Keep secret material in a separate
  encrypted backup flow. Document export contents and limitations explicitly.

- [ ] **Multiple-node comparisons.** Let users explicitly request independent
  node checks for important chain data and inspect disagreements and sources.
  Distinguish agreement among providers from cryptographic verification; do
  not describe one as proof of the other. Additional queries can expose wallet
  addresses to additional services, so make the selected destinations and that
  consequence clear rather than silently contacting extra providers.
