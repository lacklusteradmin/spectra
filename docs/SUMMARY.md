# Spectra Swift layer — file-by-file map

This document describes what each file in `swift/` does and how it relates to the Rust core in `core/src/`. The Swift layer is intentionally thin: all business logic (derivation, balance decoding, HTTP calls, SQLite persistence, send-flow planning, diagnostics aggregation, secrets policy) lives in Rust and is surfaced via UniFFI-generated bindings in [generated/spectra_core.swift](generated/spectra_core.swift). Swift only keeps SwiftUI views, `@MainActor @Observable` forwarding, and callbacks for iOS-only APIs that Rust cannot reach (Keychain, ActivityKit, UNUserNotificationCenter, UIDevice battery/network signals, biometric prompts).

## Top-level layout

```
swift/
├── *.swift                       AppState + extensions, models, stores, bridges (flat at root)
├── views/                        SwiftUI views
├── tests/                        XCTest targets
├── extensions/                   Live Activity widget targets
├── generated/                    UniFFI-generated bindings (gitignored, do not edit)
├── resources/                    Assets.xcassets
├── Spectra.xcodeproj/            Xcode project
└── Spectra.xctestplan
```

The earlier `shell/`, `send/`, `fetch/`, `derivation/`, `rustbridge/` subfolders were flattened — every Swift file now sits at the root of `swift/`, matching the flat layout used in `core/src/`. Folders are kept only where they correspond to separate Xcode targets (`views/`, `tests/`, `extensions/`) or are required build artifacts (`generated/`, `resources/`, `Spectra.xcodeproj/`).

---

## `AppState` and its extensions

`AppState` is the single `@MainActor @Observable final class` the whole SwiftUI app observes. It is sliced across many extension files so individual domains can be read in isolation. `wallets`, `transactions`, and `addressBook` are canonical Swift arrays tracked by the `@Observable` macro; persistence is durable via [PersistenceStore.swift](PersistenceStore.swift) into Rust's SQLite KV store.

- [AppState.swift](AppState.swift) — Declares `@Observable final class AppState`. Owns the observable collections + scalars, derived-state caches, persistence-debounce `Task` handles, JSON coders, and logger categories.
- [AppStateTypes.swift](AppStateTypes.swift) — Nested types lifted out of `AppState.swift` for focus: `ResetScope`, `TimeoutError`, `BackgroundSyncProfile`, `MainAppTab`, etc.
- [AppState+CoreStateStore.swift](AppState+CoreStateStore.swift) — Mutation helpers (`setWallets`, `appendWallet`, `upsertWallet`, `removeWallet(id:)`, `setTransactions`, `prependTransaction`, `setAddressBook`, …) that centralise add/replace/remove on the three core observable arrays.
- [AppState+BalanceRefresh.swift](AppState+BalanceRefresh.swift) — Kicks `WalletServiceBridge.triggerImmediateBalanceRefresh`, then batches per-chain balance deltas with a 30 ms debounce and flushes as one `wallets` mutation to avoid re-render storms.
- [AppState+ReceiveFlow.swift](AppState+ReceiveFlow.swift) — Receive-sheet orchestration. The `receiveAddress()` per-chain dispatch is routed through Rust's send-flow helpers; Swift only holds the thin `switch` that maps resolver kinds back to bridge calls.
- [AppState+SendFlow.swift](AppState+SendFlow.swift) — Send-flow state machine: destination validation, risk-probe orchestration, EVM error mapping. Pure-logic helpers (`coreSeedDerivationChainRaw`, `coreMapEthereumSendError`, etc.) live in `core/src/send/flow.rs`; this file glues them to SwiftUI-observable state.
- [AppState+SendRouting.swift](AppState+SendRouting.swift) — `refreshSendPreview()` top-level dispatch by `SendPreviewKind` (chain).
- [AppState+SendPreview.swift](AppState+SendPreview.swift) — Per-chain preview decoders. UTXO, EVM, and simple-chain previews are built by Rust (`core/src/send/preview_decode.rs`); Swift orchestrates the async fetch.
- [AppState+SendExecution.swift](AppState+SendExecution.swift) — `submitSend()` final signing/broadcast path. Delegates to Rust `executeSend` for most chains; only the host/iOS state machine (Live Activity start/complete/fail, AppState mutations) remains in Swift.
- [AppState+AddressResolution.swift](AppState+AddressResolution.swift) — Resolves chain addresses for an `ImportedWallet` by calling [WalletDerivation.swift](WalletDerivation.swift). Handles watch-only addresses and Solana legacy-path preference.
- [AppState+ImportLifecycle.swift](AppState+ImportLifecycle.swift) — Drives `WalletImportDraft` state and the import/edit sheet presentation. Heavy parsing happens in Rust via `corePlanWalletImport`.
- [AppState+PricingFiat.swift](AppState+PricingFiat.swift) — Live-price refresh cadence, fiat-rate refresh, fiat-currency display names.
- [AppState+OperationalTelemetry.swift](AppState+OperationalTelemetry.swift) — Network-status text, operational log append/export.
- [AppState+DiagnosticsEndpoints.swift](AppState+DiagnosticsEndpoints.swift) — Per-chain diagnostics wiring. All JSON decoding / record construction lives in `core/src/diagnostics/`; this file only keeps KeyPath-driven AppState wiring around the Rust HTTP / probe FFI.

## Models, types, registries

- [CoreModels.swift](CoreModels.swift) — Swift-side model structs and enums (`Coin`, `ImportedWallet`, `ChainFeePriorityOption`, `SendPreviewDetails`, `TransactionRecord`, …). Several types are typealiased onto Rust-generated records.
- [RegistryModels.swift](RegistryModels.swift) — `WalletChainID` value type + `TokenTrackingChain` / `CoreTokenTrackingChain` conformances.
- [ChainTypes.swift](ChainTypes.swift) — `RustBalanceDecoder` thin forwarders into `core/src/fetch/` + `RustStringEnum` protocol for Rust-owned enums that need `RawRepresentable` / `CaseIterable` in Swift.
- [SendPreviewTypes.swift](SendPreviewTypes.swift) — `EVMChainContext` enum + chain-ID mappings, Swift-side preview/result struct shapes.
- [StakingTypes.swift](StakingTypes.swift) — Swift-facing staking types over the Rust `StakingValidator` / `StakingPosition` / `StakingActionPreview` records.
- [ImportDraft.swift](ImportDraft.swift) — `@Observable final class WalletImportDraft` used by the import sheet.
- [SetupFlow.swift](SetupFlow.swift) — Wallet-setup state machine (network mode, derivation overrides, watch-only flag).
- [Identifiers.swift](Identifiers.swift) — Stable identifier value types used across views (wallet IDs, asset keys, etc.).
- [AppUserPreferences.swift](AppUserPreferences.swift) — `@AppStorage`-backed user preferences (theme, fiat, diagnostic verbosity).
- [Platform.swift](Platform.swift) — `PlatformSnapshotEnvelope` + `makePlatformSnapshot()` for diagnostics bundle export.
- [ChainRefreshDescriptors.swift](ChainRefreshDescriptors.swift) — `WalletChainRefreshDescriptor` closures per chain; `plannedChainRefreshDescriptors` decides which refresh/balance/history functions run per chain.

## Stores (persistence + Swift-side caches)

- [PersistenceStore.swift](PersistenceStore.swift) — `loadCodableFromSQLite` / `persistCodableToSQLite` round-trips through `WalletServiceBridge.saveState/loadState`, mapped to Rust `core/src/store/`.
- [DashboardStore.swift](DashboardStore.swift) — Pinned-asset prototype catalog + pinning state. Color fields are SwiftUI `Color`, so this stays Swift-side.
- [MaintenanceStore.swift](MaintenanceStore.swift) — Battery/network-gated background maintenance. Decision logic is in Rust (`core/src/store/`); this file only injects iOS inputs (`UIDevice.batteryLevel`, `ProcessInfo.isLowPowerModeEnabled`).
- [DiagnosticsState.swift](DiagnosticsState.swift) — `@Observable final class WalletDiagnosticsState`. Per-chain degraded-state maps + operational-log ring buffer; debounces persistence to the Rust SQLite store.
- [DiagnosticsStore.swift](DiagnosticsStore.swift) — Per-chain property shims bridging `AppState.<chain>SelfTestResults` → `chainDiagnosticsState.<chain>SelfTestResults`. Pure forwarding so views observe a single surface on AppState.
- [StoreDiagnosticsExport.swift](StoreDiagnosticsExport.swift) — `DiagnosticsEnvironmentMetadata` envelope + per-chain JSON builders that fan into `diagnosticsBuild*Json` Rust helpers.
- [StoreHistoryRefresh.swift](StoreHistoryRefresh.swift) — Transaction-history pagination. Cursors/pages live in Rust; Swift just notifies `objectWillChange`.
- [StoreLifecycleReset.swift](StoreLifecycleReset.swift) — App-launch state restoration and `ResetScope`-driven wipes.
- [StorePersistenceNormalization.swift](StorePersistenceNormalization.swift) — Rebuilds cached derived state (`cachedWalletByID`, `cachedIncludedPortfolioHoldings`, token-preference caches) using Rust's `corePlanStoreDerivedState`.
- [Store+Formatting.swift](Store+Formatting.swift) — Localized string + USD↔fiat helpers; number formatting delegates to `core/src/formatting.rs`.
- [Store+Notifications.swift](Store+Notifications.swift) — Token-preference merge + price-alert evaluation + `UNUserNotificationCenter` scheduling (iOS-only).

## Bridges (Swift facade onto UniFFI)

- [WalletServiceBridge.swift](WalletServiceBridge.swift) — `actor WalletServiceBridge` + `enum SpectraChainID`. Owns the singleton `WalletService` instance, surfaces typed helpers like `fetchNativeBalanceSummary`, `fetchHistoryHasActivity`, `fetchEvmSendPreviewTyped`, `fetchEVMHistoryPage`, `executeSend`, `signAndSend`, `resolveENSName`, `deriveBitcoinAccountXpub`, `saveState/loadState`. JSON surfaces are now only the state-persistence blobs.
- [WalletRustDerivationBridge.swift](WalletRustDerivationBridge.swift) — Maps Swift's `SeedDerivationChain` / `WalletDerivationNetwork` / requested-output sets onto the Rust FFI enum forms and decodes responses.
- [WalletRustEndpointCatalogBridge.swift](WalletRustEndpointCatalogBridge.swift) — Thin wrapper over typed UniFFI exports (`appCoreEndpointForId`, `appCoreEndpointRecordsForChain`, …). The endpoint catalog itself lives in `core/data/AppEndpointDirectory.json`.
- [CachedCoreHelpers.swift](CachedCoreHelpers.swift) — Swift-side memoization wrappers for Rust-core pure functions called from view bodies (decimal resolution, icon-id normalization, etc.). Every helper is a thin wrapper over a UniFFI export.
- [WalletBalanceObserver.swift](WalletBalanceObserver.swift) — `WalletBalanceObserver` callback that bridges Rust's per-chain refresh engine into AppState's `@Observable` mirrors on the main actor.
- [WalletDerivation.swift](WalletDerivation.swift) — `WalletDerivationLayer.derive(...)`. Builds a derivation request and calls the UniFFI entry point backed by `core/src/derivation/`.
- [WalletDerivedCache.swift](WalletDerivedCache.swift) — Memoizes already-derived addresses for the lifetime of a session so Receive / Wallet rows don't re-hit Rust on every render.

## Staking clients (Swift wrappers)

One file per chain that supports staking. Each wraps the Rust `core/src/staking/chains/<chain>.rs` client into Swift-observable state plus a thin form translator.

- [AptosStakingClient.swift](AptosStakingClient.swift)
- [CardanoStakingClient.swift](CardanoStakingClient.swift)
- [IcpStakingClient.swift](IcpStakingClient.swift)
- [NearStakingClient.swift](NearStakingClient.swift)
- [PolkadotStakingClient.swift](PolkadotStakingClient.swift)
- [SolanaStakingClient.swift](SolanaStakingClient.swift)
- [SuiStakingClient.swift](SuiStakingClient.swift)

## Secrets, content, view utilities

- [SecureStores.swift](SecureStores.swift) — Keychain-backed `SecureStore`, `SecureSeedStore`, `AppLockPinStore`. Wraps `KeychainAccess`; must stay in Swift because Rust cannot reach the iOS Keychain.
- [StaticContentCatalog.swift](StaticContentCatalog.swift) — Prefers `coreStaticResourceJson` first, falls back to bundled JSON across locale paths. Used for `ChainWikiEntries`, `TokenVisualRegistry`, `DerivationPresets`, `SettingsContentCopy`, `DiagnosticsContentCopy`, etc.
- [DebouncedAction.swift](DebouncedAction.swift) — Generic debounce helper used by AppState + various stores.
- [LoadingTaskRegistry.swift](LoadingTaskRegistry.swift) — Tracks in-flight `Task` handles by string key so views can display a unified loading state.
- [ViewExtensions.swift](ViewExtensions.swift) — Hosts `@main struct SpectraApp: App` (app entry point, scene-phase wiring, app-lock overlay) plus shared SwiftUI view modifiers.

## views/ — SwiftUI

Pure UI. Each view observes `AppState` and renders state. No business logic.

- [views/MainTabView.swift](views/MainTabView.swift) — Tab bar host.
- [views/DashboardViews.swift](views/DashboardViews.swift) — Portfolio & pinned-asset dashboard.
- [views/HistoryView.swift](views/HistoryView.swift) — Transactions list with pagination.
- [views/ReceiveFlowViews.swift](views/ReceiveFlowViews.swift) — Receive sheet (address QR, chain picker).
- [views/SendFlowViews.swift](views/SendFlowViews.swift), [views/SendPrimarySectionsView.swift](views/SendPrimarySectionsView.swift) — Send sheet (amount entry, fee priority, preview, confirm).
- [views/WalletFlowViews.swift](views/WalletFlowViews.swift) — Wallet list, detail, edit.
- [views/WalletSetupViews.swift](views/WalletSetupViews.swift) — Wallet import/create flow, network-mode toggles.
- [views/AddWalletEntryView.swift](views/AddWalletEntryView.swift) — Row component for "add wallet" rows.
- [views/SettingsViews.swift](views/SettingsViews.swift) — Settings root.
- [views/SettingsTokenComponents.swift](views/SettingsTokenComponents.swift), [views/AddCustomTokenView.swift](views/AddCustomTokenView.swift), [views/TokenRegistrySettingsView.swift](views/TokenRegistrySettingsView.swift), [views/TokenRegistryDetailView.swift](views/TokenRegistryDetailView.swift) — Token-picker subscreens.
- [views/PricingSettingsView.swift](views/PricingSettingsView.swift), [views/DecimalDisplaySettingsView.swift](views/DecimalDisplaySettingsView.swift), [views/BackgroundSyncSettingsView.swift](views/BackgroundSyncSettingsView.swift), [views/AdvancedSettingsView.swift](views/AdvancedSettingsView.swift), [views/LargeMovementAlertsSettingsView.swift](views/LargeMovementAlertsSettingsView.swift), [views/PriceAlertsView.swift](views/PriceAlertsView.swift) — Settings subscreens.
- [views/AddressBookView.swift](views/AddressBookView.swift) — Address-book editor.
- [views/AllChainsSelectionView.swift](views/AllChainsSelectionView.swift) — Multi-chain selection picker.
- [views/DiagnosticsViews.swift](views/DiagnosticsViews.swift), [views/DiagnosticsExportsBrowserView.swift](views/DiagnosticsExportsBrowserView.swift), [views/LogsView.swift](views/LogsView.swift), [views/ReportProblemView.swift](views/ReportProblemView.swift) — Diagnostics + log + bug-report screens.
- [views/EndpointsViews.swift](views/EndpointsViews.swift) — Per-chain endpoint editors.
- [views/ChainWikiViews.swift](views/ChainWikiViews.swift) — Chain info pages from `ChainWikiEntries.json`.
- [views/TransactionDetailView.swift](views/TransactionDetailView.swift) — Single-transaction detail.
- [views/StakingView.swift](views/StakingView.swift) — Cross-chain staking entry.
- [views/DonationsView.swift](views/DonationsView.swift), [views/AboutView.swift](views/AboutView.swift), [views/BuyCryptoHelpView.swift](views/BuyCryptoHelpView.swift), [views/ResetWalletWarningView.swift](views/ResetWalletWarningView.swift) — Static / informational screens.
- [views/ImageRendering.swift](views/ImageRendering.swift) — Token / chain icon helpers.
- [views/LiquidGlassUI.md](views/LiquidGlassUI.md) — Notes on iOS 26 Liquid Glass usage in Spectra views.

## extensions/ — Live Activity widget targets

- [extensions/SharedLiveActivities/SendLiveActivityAttributes.swift](extensions/SharedLiveActivities/SendLiveActivityAttributes.swift) — `ActivityAttributes` shared between the main app and the widget extension.
- [extensions/SpectraLiveActivityExtension/SendLiveActivityWidget.swift](extensions/SpectraLiveActivityExtension/SendLiveActivityWidget.swift) — `WidgetBundle` / `ActivityConfiguration` rendered on the Lock Screen and Dynamic Island.
- `extensions/SpectraLiveActivityExtension/Info.plist` — extension Info.plist.

## tests/ — XCTest

- [tests/AppStateTests.swift](tests/AppStateTests.swift) — High-level `AppState` behavior.
- [tests/RefreshPlannerTests.swift](tests/RefreshPlannerTests.swift) — `plannedChainRefreshDescriptors` ordering and chain-name → ID mapping.
- [tests/DiagnosticsStateTests.swift](tests/DiagnosticsStateTests.swift), [tests/DiagnosticsBundleTests.swift](tests/DiagnosticsBundleTests.swift) — Diagnostics state + bundle export.
- [tests/SecureSeedStoreTests.swift](tests/SecureSeedStoreTests.swift) — Keychain-backed seed store round-trip.

## generated/ — UniFFI output (gitignored)

- [generated/spectra_core.swift](generated/spectra_core.swift) — Swift bindings for every `#[uniffi::export]` in `core/src/` and `ffi/src/`. Regenerated by `scripts/bindgen-ios.sh`; folder is gitignored.
- `generated/spectra_coreFFI.h`, `.modulemap`, `module.modulemap` — C-ABI headers the Swift module imports.

## resources/

- `resources/Assets.xcassets` — Image/color asset catalog. Token icons live in the top-level `resources/icons/` folder (referenced via `PBXFileSystemSynchronizedRootGroup`); JSON content strings live in `core/data/` so Rust can load them too.

---

## How Swift talks to Rust

1. **Static content.** JSON/text data files live in `core/data/` (chain wiki, endpoint directory, derivation presets, token visual registry, BIP-39 word list). Swift loads them via `coreStaticResourceJson` in [StaticContentCatalog.swift](StaticContentCatalog.swift); Bundle JSON is only a fallback for localized strings.
2. **Canonical state.** Wallets / transactions / address book are Swift arrays on the `@Observable` `AppState`. Durable persistence lives in `core/src/store/` (SQLite via `rusqlite`); writes flow through [PersistenceStore.swift](PersistenceStore.swift) and helpers in [AppState+CoreStateStore.swift](AppState+CoreStateStore.swift).
3. **HTTP / fetch.** `WalletService` in Rust owns `reqwest` clients. [WalletServiceBridge.swift](WalletServiceBridge.swift) is the async-facing actor; every network call eventually goes through it.
4. **Derivation.** Swift never does key derivation. [WalletDerivation.swift](WalletDerivation.swift) builds a request and calls the UniFFI entry point backed by `core/src/derivation/`.
5. **Secrets.** Seed phrases and PINs sit in the iOS Keychain via [SecureStores.swift](SecureStores.swift). Rust's secrets *policy* (which keys, which access classes) is mirrored in Rust types, but the actual Keychain IO is Swift-only by necessity.
6. **Platform callbacks Rust can't reach.** `UIDevice.batteryLevel`, `ProcessInfo.isLowPowerModeEnabled`, `NWPathMonitor`, `ActivityKit` live activities, `UNUserNotificationCenter`, biometric prompts — all stay in Swift and feed signals into Rust-evaluated policy functions (see [MaintenanceStore.swift](MaintenanceStore.swift)).
