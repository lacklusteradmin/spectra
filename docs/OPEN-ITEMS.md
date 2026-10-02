# Open items

Work that remains after the stages in [PLAN.md](PLAN.md), whose Rule 0 applies.
Delete an item once it is done; what changed belongs in
[BEHAVIOUR-CHANGES.md](BEHAVIOUR-CHANGES.md).

## Tasks

- [ ] **Detect FFI record fields with no production writer.** A syntactic scan
  cannot reliably distinguish unwritten fields from serde or multi-line writes.
  A useful gate needs type-aware analysis before it can reject unused fields.
- [ ] **Staking transaction execution.** The app currently provides information
  and validator queries. Transaction execution remains future work.
- [ ] **Staking reads go through the API adapters.** AGENTS.md puts an API's
  network I/O in `core/src/api/<api>.rs`; `staking/` predates that rule.
  `SolanaStakingClient`, `SuiStakingClient`, `NearStakingClient` and
  `AptosStakingClient` each build their own JSON-RPC or REST request and race
  their own endpoint list in `fetch_validators` (`getVoteAccounts`,
  `suix_getLatestSuiSystemState`, `validators`, the Aptos validator-set
  resource). Move each request and its response type into the matching adapter
  (`solana_json_rpc`, `sui_json_rpc`, `near_json_rpc`, `aptos_rest`) as a client
  method, and have `staking/` take that client and keep only the staking
  decisions: ranking, commission and APY projection, position shaping.
  `IcpStakingClient` and `PolkadotStakingClient` hold endpoint lists they never
  read (a static NNS neuron directory; no keyless Sidecar), so drop those fields
  rather than route them. Record any change in what a validator query returns
  in [BEHAVIOUR-CHANGES.md](BEHAVIOUR-CHANGES.md) and pass `make verify`.
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
  one `@Observable` class whose methods are spread over 31
  `AppState+<Domain>.swift` extensions, a third of them under 40 lines. The
  extensions share every stored property, so the split hides line count but
  not coupling, and any view that reads one property is in the same
  invalidation scope as the rest. `sendFlow`, `receiveFlow`, `walletImport`,
  `preferences` and `diagnostics` already show the target shape: a small
  `@MainActor @Observable` type that `AppState` owns, holding that domain's
  view state and exposing its actions. Move the remaining domains the same way
  — address book, token preferences, price alerts, Tor, networks/endpoints,
  history paging, send execution and preview, notifications and Live
  Activities — one domain per change, each taking its properties out of
  `AppState` and its views reading the new object rather than the store.
  Merge the tiny extensions that are only adapters (`Diagnostics`,
  `Persistence`, `Networks`, `TorLifecycle`) into the domain that owns them
  rather than giving each a type. Keep core as the owner of domain state: the
  new types hold projections and view state only, per AGENTS.md. Each step
  needs the iOS suite green and no user-visible change; record nothing unless
  behaviour moves.

## Known limitations

- **Endpoint availability and redundancy:** the 2026-09-23 live audit removed
  failed built-in providers instead of retaining broken fallbacks. Zcash,
  Bitcoin Gold, Dash, Dogecoin testnet and Monero stagenet now have no built-in
  API; supported custom nodes remain configurable. Tron PublicNode uses its
  verified `/jsonrpc` path. Diagnostics probe actual read methods on the selected
  endpoint, including testnets and catalogued EVM history sources. Passing these
  checks does not prove every advertised capability or transaction broadcast.
- **EVM history availability:** only verified keyless indexers are configured.
  BNB Chain, Sonic, opBNB, Sei, Linea, Hyperliquid, Cronos, X Layer and
  Berachain have no built-in history source;
  Etherscan V2 and its API-key setting were removed by user request.

- **Keyless provider policy:** API-key configuration and authenticated provider
  adapters are removed. Polkadot/Westend and Bittensor read balances from
  `System.Account` storage over their own RPC; their history remains
  unavailable until a keyless indexer exists. Blockchair, SoChain and Trezor's
  Blockbooks refuse keyless clients and are not candidates. Cardano staking
  queries are unavailable; its keyless Koios broadcast is implemented.
