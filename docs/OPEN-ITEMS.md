# Open items

Engineering work that remains. Rule 0 in [AGENTS.md](../AGENTS.md) applies.
Delete an item once it is done; what changed belongs in
[BEHAVIOUR-CHANGES.md](BEHAVIOUR-CHANGES.md).

## Tasks

- [ ] **Detect FFI record fields with no production writer.** A syntactic scan
  cannot reliably distinguish unwritten fields from serde or multi-line writes.
  A useful gate needs type-aware analysis before it can reject unused fields.

- [ ] **Name FFI types once, in Rust, so Swift needs no typealiases.** Swift
  renames nine UniFFI types with `typealias`, so each has two names and both
  appear in code: `Coin = AssetHolding`, `TransactionRecord =
  CorePersistedTransactionRecord`, `TransactionStatus = CoreTransactionStatus`,
  `PriceAlertRule = PriceAlertEvaluationAlert`, `PriceAlertCondition =
  CorePriceAlertCondition`, `TokenPreferenceEntry = CoreTokenPreferenceEntry`,
  `SeedDerivationPaths = CoreSeedDerivationPaths`, `DashboardAssetGroup =
  CoreDashboardAssetGroup` and `DashboardPinOption = CoreDashboardPinOption`
  (the last two in `swift/views/DashboardViews.swift`, the rest in
  `CoreModels.swift`, `ChainTypes.swift` and `RegistryModels.swift`). Pick the
  one name each type should have — drop the `Core` prefix, which says where a
  type lives rather than what it is, and settle `Coin` versus `AssetHolding` —
  and rename the Rust type (or set its UniFFI name) so the binding carries it.
  Apply the same rule to the remaining `Core*` exports Swift uses unaliased
  (`CoreSeedDerivationPreset`, `CoreWalletDerivationOverrides`,
  `CoreTokenPreferenceKey`, `CoreAppState`). Rename in CLI and Kotlin call
  sites in the same change, regenerate the bindings, delete every alias, and
  pass `make verify`. No behaviour changes; nothing to record beyond the
  commit.
- [ ] **Give `AppState`'s domains their own observable state.** `AppState` is
  one `@Observable` class whose methods are spread over 30
  `AppState+<Domain>.swift` extensions, a third of them under 40 lines. The
  extensions share every stored property, so the split hides line count but
  not coupling, and any view that reads one property is in the same
  invalidation scope as the rest. `sendFlow`, `receiveFlow`, `walletImport`,
  `preferences` and `diagnostics` already show the target shape: a small
  `@MainActor @Observable` type that `AppState` owns, holding that domain's
  view state and exposing its actions. Move the remaining domains the same way
  — address book, token preferences, price alerts, Tor, history paging,
  send execution and preview, notifications and Live
  Activities — one domain per change, each taking its properties out of
  `AppState` and its views reading the new object rather than the store.
  Merge the tiny extensions that are only adapters (`Diagnostics`,
  `Persistence`, `TorLifecycle`) into the domain that owns them
  rather than giving each a type. Keep core as the owner of domain state: the
  new types hold projections and view state only, per AGENTS.md. Each step
  needs the iOS suite green and no user-visible change; record nothing unless
  behaviour moves.

## Supported chain scope

[CHAIN-SUPPORT.md](CHAIN-SUPPORT.md) defines supported protocols, import formats,
provider coverage, staking actions and explicit exclusions. The 59 unverified
deployments and associated identity/wiki records are preserved as commented
TODO records in the source catalog; verification may activate them individually.

- [ ] Verify the issuer or official bridge evidence for each pending deployment
  before making it a trusted default. Preserve the researched records while
  checking them.
- [ ] Extend protocol support for XRP/Stellar issued assets, Cardano native
  assets and Token-2022 transfer hooks/fees, with exact ownership, metadata and
  signing semantics. These are explicit exclusions today.
- [ ] Add independently verified keyless history/discovery providers for networks
  listed as requiring custom sources. A general RPC does not supply complete
  address history.
