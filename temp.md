# Phase 5 Scratchpad — WalletStore elimination

## Current state
- Store/ → Shell/ (folder rename, pbxproj patched)
- WalletStore → AppState class (185 refs migrated)
- Shell/ is 9,792 LOC / 36 files, all @Published + glue
- iOS green

## Shell/ file inventory (by LOC)
| LOC  | File                                      | Shape                            | Lift strategy |
|------|-------------------------------------------|----------------------------------|---------------|
| 1614 | AppState+SendFlow.swift                   | form state, pending-tx, EVM fees | per-method Rust lifts |
| 1183 | AppState.swift                            | @Published declarations + init   | keep; shrink methods |
| 744  | AppState+ReceiveFlow.swift                | receive flow                     | per-chain lifts |
| 696  | AppState+DiagnosticsEndpoints.swift       | endpoint probing                 | Rust lifts (reqwest already) |
| 682  | AppState+CoreBridge.swift                 | FFI marshalling                  | shrinks passively |
| 674  | StoreHistoryRefresh.swift                 | pagination, dedup                | Rust lifts (pure logic) |
| 444  | StorePersistenceNormalization.swift       | Codable normalization            | Rust lifts |
| 430  | StoreLifecycleReset.swift                 | UserDefaults load + @Published   | keep glue, lift defaults |
| 357  | PersistenceStore.swift                    | SQLite bridge                    | shrink to 1-func calls |
| 315  | AppState+OperationalTelemetry.swift       | event log                        | Rust lift |
| 309  | Store+Formatting.swift                    | display formatting               | Rust lifts (formatting.rs exists) |
| 259  | StoreDiagnosticsExport.swift              | diagnostic bundle                | Rust lift |
| 232  | AppState+PricingFiat.swift                | fiat rates                       | Rust lift |
| 223  | AppState+ImportLifecycle.swift            | import flow                      | per-step Rust lifts |
| 200  | WalletAddressResolver.swift               | address resolution               | Rust lift |
| 175  | ChainRefreshDescriptors.swift             | refresh descriptors              | types |
| 147  | SecureStores.swift                        | Keychain                         | **KEEP Swift** |
| 145  | AppState+BalanceRefresh.swift             | balance orchestration            | Rust lift |
| 97   | Store+Notifications.swift                 | notifications                    | Rust + UserNotifications bridge |
| 91   | RefreshPlan.swift                         | refresh types                    | may lift |
| 67   | MaintenanceIntervals.swift                | constants                        | lift to Rust constants |
| 69   | MaintenanceStore.swift                    | timer logic                      | keep (Timer API) |
| 62   | ChainRefreshRouter.swift                  | routing                          | Rust lift |
| 61   | AppState+PendingTxPolling.swift           | polling loop                     | Rust lift |
| 54   | StoreDiagnosticsTypes.swift               | types                            | mirror in Rust |
| 43   | ChainRefresh.swift                        | types                            | mirror |
| 40   | SpectraSecretStoreAdapter.swift           | Rust callback                    | **KEEP Swift** |
| 37   | PortfolioStore.swift                      | holdings diffing                 | Rust lift |
| 31   | SeedPhraseSafety.swift                    | BIP-39 validation                | already mostly Rust |
| 25   | TransferAvailability.swift                | filter logic                     | Rust lift |
| 24   | FlowStore.swift                           | SwiftUI Bindings                 | **KEEP Swift** |
| 18   | RefreshSignal.swift                       | Combine publisher                | **KEEP Swift** |
| 10   | ReceiveFlowStore.swift                    | SwiftUI Bindings                 | **KEEP Swift** |
| 8    | StoreLocalization.swift                   | i18n wrapper                     | **KEEP Swift** |
| 7    | MainAppTab.swift                          | SwiftUI enum                     | move to Views/ |

**Floor (must stay Swift):** SecureStores.swift, SpectraSecretStoreAdapter.swift, SeedPhraseSafety.swift (keychain adjacent), FlowStore.swift, ReceiveFlowStore.swift, RefreshSignal.swift, StoreLocalization.swift, MaintenanceStore.swift (Timer), AppState.swift (@Published declarations). ~400 LOC.

## Next targets (smallest pure-logic first)
1. TransferAvailability.swift (25) — availableSendCoins filter logic → Rust
2. PortfolioStore.swift (37) — walletHoldingSnapshotsMatch → Rust (tuple equality)
3. AppState+PendingTxPolling.swift (61) — receipt decode + classify → Rust
4. MaintenanceIntervals.swift (67) — move constants to Rust
5. WalletAddressResolver.swift (200) — resolvedBitcoinAddress etc → Rust
