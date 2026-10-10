import Foundation
import SwiftUI
#if canImport(Network)
    import Network
#endif

// MARK: - @Observable opt-in convention
//
// Swift's `@Observable` macro turns every stored property into
// observation-tracked unless it's tagged `@ObservationIgnored`. The
// default-on / opt-out shape means new properties accidentally become
// observable unless the author remembers — and a SwiftUI view that
// reads any tracked property re-renders on its mutation.
//
// AppState's rule: every new stored property MUST be one of
//   1. observed by views (no annotation; the property genuinely drives UI)
//   2. `@ObservationIgnored` with a one-line comment naming why it's
//      excluded (caches, debounce handles, weak observers, persistence
//      task storage — anything views shouldn't see)
//
// Reviewing a new stored property: if the author can't justify "yes
// SwiftUI views observe this," it should be `@ObservationIgnored`. The
// existing properties already follow this — see the dense `@ObservationIgnored`
// block at the top for the catalog. New work that doesn't make a choice
// is a bug surface (silent over-invalidation).

// MARK: - AppState architecture
//
// `AppState` is the app's central `@Observable` store: the wallet, portfolio
// and transaction projections, and the composition root for the domains.
// A domain with state of its own — the send and receive flows, wallet import,
// the address book, token preferences, price alerts, Tor, history paging,
// diagnostics, the platform's own preferences — is a small `@MainActor`
// `@Observable` type `AppState` owns, holding that domain's projection or view
// state and its actions. Its views read that object, not properties here.
// `notifications` holds no state at all.
//
// The `AppState+<Domain>.swift` extensions are the adapters left over: what
// needs the wallet projection, or more than one domain, to answer.
//
// Adding state? Put it in its domain's type, or start one for a new domain,
// rather than growing this class.
@MainActor
@Observable
final class AppState {
    @ObservationIgnored let bridge: WalletServiceBridge // Service identity is not view state.
    @ObservationIgnored let servicesEnabled: Bool // Controls automatic platform work.
    static let exportFilenameTimestampFormatter: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withDashSeparatorInDate, .withColonSeparatorInTime]
        return formatter
    }()

    // ── Domains ─────────────────────────────────────────────────────────
    // `let`, so reading one is not itself observed: a view tracks only the
    // properties it reads inside the domain object.

    /// Every state command goes through this one queue; see `StateCommandQueue`.
    let stateCommands: StateCommandQueue
    let sendFlow: SendFlowState
    let receiveFlow = ReceiveFlowState()
    let walletImport = WalletImportSession()
    let addressBook: AddressBookState
    let tokenPreferences: TokenPreferencesState
    let priceAlerts: PriceAlertsState
    let historyPaging = HistoryPagingState()
    let tor: TorState
    /// The six preferences this platform keeps for itself. Settings core owns
    /// are `appSettings`.
    let preferences = AppUserPreferences()
    let diagnostics: WalletDiagnosticsState
    let chainDiagnosticsState = WalletChainDiagnosticsState()
    let notifications: PlatformNotifications

    /// Recorded transactions.
    ///
    /// Domain state: core owns the store and its persistence. This is a
    /// projection — assigning to it would only desynchronise the two, so it is
    /// `private(set)` and replaced only with what core returns.
    private(set) var transactions: [TransactionRecord] = [] {
        didSet { transactionRevision &+= 1 }
    }

    /// The only place the transaction projection is written.
    func setTransactionProjection(_ records: [TransactionRecord]) {
        transactions = records
    }
    var historyReadError: String? = nil
    @ObservationIgnored var portfolioSnapshotRevision: UInt64 = 0 // Core snapshot order, not request order.
    @ObservationIgnored var transactionSnapshotRevision: UInt64 = 0 // Reject delayed history summaries.
    var portfolioValuation: PortfolioValuation?
    private(set) var assetPrecision: AssetPrecisionCatalog?

    func adoptAssetPrecision(_ precision: AssetPrecisionCatalog) { assetPrecision = precision }
    var transactionCount: UInt64 = 0
    /// Core's count of transactions still pending, for the History tab's badge.
    var pendingTransactionCount: UInt64 = 0
    /// The pending sends core says can still be replaced on their chain.
    /// Adopted with the rest of the transaction-derived views; observed,
    /// because the transaction page's Speed Up / Cancel read it.
    var replaceableSends: [ReplaceableSend] = []
    private(set) var transactionRevision: UInt64 = 0
    /// When each wallet's earliest stored transaction happened, from core's
    /// transaction snapshot. Observed: the wallet detail shows it.
    var cachedFirstActivityDateByWalletId: [String: Date] = [:]
    /// Imported wallets.
    ///
    /// Domain state: core owns the list and persists it. This is a projection
    /// of `ResidentState.wallets`, rendered into the shape the views use — see
    /// `WalletState::to_wallet_view`. `private(set)`, because assigning to it
    /// would only desynchronise it from core; change it with import and field
    /// intents, wallet deletion, or a reset. Core's refresh engine follows the
    /// same change on its own; nothing here has to tell it.
    ///
    /// **Observation note for view code**: `@Observable` tracks whole
    /// properties, never a key or an index inside one. A view that reads
    /// `wallets`, or any field of `walletDerivedCache` (`wallet(for:)`,
    /// `portfolio` and the rest), is invalidated whenever that property is
    /// assigned, even for another wallet's balance.
    private(set) var wallets: [WalletView] = []

    /// The only place the wallet projection is written. Everything else goes
    /// through a `StateCommand` and lands back here. Each value is assigned
    /// only when it changed, so an unchanged one invalidates no view.
    func setWalletProjection(_ records: [WalletView], identityRevision: UInt64) {
        if wallets != records { wallets = records }
        if walletIdentityRevision != identityRevision { walletIdentityRevision = identityRevision }
    }
    /// Core's count of changes to a wallet in anything but its balances — the
    /// name, addresses and settings that history rows and transaction details
    /// are read against. A balance landing every few hundred milliseconds of a
    /// sweep does not re-run those reads.
    private(set) var walletIdentityRevision: UInt64 = 0
    // Derived caches. Recomputed by `rebuildWalletDerivedStateFromCore`.
    //
    // No revision counter here. Under `@Observable` a view already tracks the
    // properties it reads, so a counter bumped on every cache write could only
    // make things worse: a view that observed it would invalidate on every
    // unrelated write. `walletIdentityRevision` above is different — views
    // key `.task(id:)` on it, which needs a value that changes.
    /// Bundled derived state of the wallet collection, rebuilt as a single
    /// value so a rebuild is one assignment. Read it through `wallet(for:)`,
    /// `portfolio`, `availableSendCoins(for:)` and their siblings.
    var walletDerivedCache: WalletDerivedCache = .empty
    var isShowingAddWalletEntry: Bool = false
    /// Why the last state command, or a wallet deletion, did not go through.
    /// Flows with their own refusal field — contacts, tokens, alerts — use that.
    var commandError: String?
    var selectedMainTab: MainAppTab = .home {
        didSet { if selectedMainTab != oldValue { reportDeviceConditions() } }
    }
    var isAppLocked: Bool = false
    var appLockError: String? = nil
    /// Set when core reports the stored data unreadable at launch. Observed:
    /// nothing works without the store, so the app covers itself with the one
    /// way out — discarding it — instead of showing an empty wallet list.
    var isStoreUnreadable = false
    /// Set when core could not be given the keychain-backed secret store.
    /// Observed: nothing that touches a seed or a private key works without
    /// it, so the failure has to reach the user rather than only the log.
    var secretStoreRegistrationError: String? = nil
    @ObservationIgnored var isNetworkReachable: Bool = true
    @ObservationIgnored var isConstrainedNetwork: Bool = false
    @ObservationIgnored var isExpensiveNetwork: Bool = false
    /// When core last checked pending sends without a failure, from its clock.
    var lastPendingTransactionRefreshAt: Date? = nil
    /// Display currency for prices and totals: core's setting, changed like
    /// any other through `updateSetting(.fiatCurrency(value:))`.
    var selectedFiatCurrency: FiatCurrency { appSettings.fiatCurrency }

    /// The last committed core settings, with pending edits applied by core's rule.
    /// Views change fields through `updateSetting`.
    private(set) var appSettings: AppSettings = appSettingsDefaults()
    @ObservationIgnored private(set) var committedAppSettings: AppSettings = appSettingsDefaults() // Runtime effects use only core-committed settings.
    /// Pending edits protect the optimistic form from readback. Committed
    /// settings still advance independently to drive runtime effects.
    @ObservationIgnored private var settingCommandsInFlight = 0

    /// Change one setting.
    ///
    /// Shown at once — `appSettingsApplying` is the reducer's rule, so the value
    /// shown is the value core will store — and sent to core in order. Core's
    /// committed settings replace the shown ones when the last edit in flight
    /// lands, or when one fails.
    func updateSetting(_ update: AppSettingUpdate) {
        let before = appSettings
        let after = appSettingsApplying(settings: before, update: update)
        guard after != before else { return }
        appSettings = after
        settingCommandsInFlight += 1
        stateCommands.enqueue(.setAppSetting(update: update)) { [weak self] result in
            guard let self else { return }
            self.settingCommandsInFlight -= 1
            // The last edit in flight hands the form back to core's settings.
            // A failed write never changes runtime services, and the committed
            // projection is restored even if storage cannot be read again.
            if self.settingCommandsInFlight == 0 { self.appSettings = self.committedAppSettings }
            guard case .failure(let error) = result else { return }
            self.reportCommandError(error)
            if let state = try? await self.bridge.ready().appState() { self.applyCoreState(state) }
        }
    }

    /// A two-way binding onto one setting, for a toggle, picker or slider.
    func settingBinding<Value>(
        _ keyPath: KeyPath<AppSettings, Value>, _ update: @escaping (Value) -> AppSettingUpdate
    ) -> Binding<Value> {
        Binding(
            get: { self.appSettings[keyPath: keyPath] },
            set: { self.updateSetting(update($0)) })
    }

    /// What a settings change sets in motion on this platform: the
    /// notification permission a newly enabled alert needs.
    private func reactToSettingsChange(from before: AppSettings) {
        let appSettings = committedAppSettings
        if (appSettings.useTransactionStatusNotifications && !before.useTransactionStatusNotifications)
            || (appSettings.useLargeMovementNotifications && !before.useLargeMovementNotifications)
        {
            notifications.requestPermission()
        }
    }

    @ObservationIgnored private(set) var appliedCoreStateRevision: UInt64 = 0

    /// The only place the core-owned mirrors are written. Everything else goes
    /// through a `StateCommand` and lands back here. Wallet-derived values come
    /// with the portfolio snapshot, which the caller reads when it needs one.
    @discardableResult
    func applyCoreState(_ state: ResidentState) -> Bool {
        guard state.revision >= appliedCoreStateRevision else { return false }
        appliedCoreStateRevision = state.revision
        if state.settings != committedAppSettings {
            let before = committedAppSettings
            committedAppSettings = state.settings
            reactToSettingsChange(from: before)
        }
        if settingCommandsInFlight == 0 { appSettings = state.settings }
        addressBook.adopt(state.addressBook)
        tokenPreferences.adopt(state.tokenPreferences)
        priceAlerts.adopt(state.priceAlerts)
        return true
    }
    // Quote notices and groups are adopted together from the same core
    // snapshot; every money figure arrives valued, in `portfolioValuation`.
    // `applyQuoteProjection` is the notices' only writer.
    var fiatRatesRefreshError: String? = nil
    var quoteRefreshError: String? = nil
    var cachedAvailableDashboardPinOptions: [DashboardPinOption] = []
    var cachedDashboardAssetGroups: [DashboardAssetGroup] = []

    var amounts: AmountPresentation {
        AmountPresentation(assetPrecision: assetPrecision, valuation: portfolioValuation,
            selectedFiatCurrency: selectedFiatCurrency)
    }
    @ObservationIgnored var userInitiatedRefreshTask: Task<Bool, Never>?
    @ObservationIgnored var balanceProgressTask: Task<Void, Never>? // Coalesces mid-sweep portfolio reads.
    @ObservationIgnored var appIsActive = true
    @ObservationIgnored var deviceConditionsTask: Task<Void, Never>? // Orders reports to core's engine.
    @ObservationIgnored var refreshEventsTask: Task<Void, Never>? // Drains core's refresh events in order.
    #if canImport(Network)
        let networkPathMonitor = NWPathMonitor()
        let networkPathMonitorQueue = DispatchQueue(label: "spectra.network.monitor")
    #endif
    init(bridge: WalletServiceBridge = .shared, startServices: Bool = true) {
        self.bridge = bridge
        self.servicesEnabled = startServices
        let diagnostics = WalletDiagnosticsState(bridge: bridge)
        let commands = StateCommandQueue(bridge: bridge)
        self.diagnostics = diagnostics
        self.stateCommands = commands
        self.notifications = PlatformNotifications(bridge: bridge, diagnostics: diagnostics)
        self.sendFlow = SendFlowState(bridge: bridge)
        self.addressBook = AddressBookState(commands: commands)
        self.tokenPreferences = TokenPreferencesState(commands: commands, diagnostics: diagnostics)
        self.priceAlerts = PriceAlertsState(commands: commands)
        self.tor = TorState(bridge: bridge)
        commands.adopt = { [weak self] in await self?.adoptCommittedTransition($0) }
        guard startServices else { return }
        // A launch is a return to the app too: without this, closing it from
        // the app switcher and opening it again would get past auto-lock.
        isAppLocked = preferences.useFaceId && preferences.useAutoLock
        // Wire the preferences' side effect back to AppState. A closure rather
        // than an observation loop keeps the coupling explicit.
        preferences.useFaceIDDisabledHandler = { [weak self] in
            self?.isAppLocked = false
            self?.appLockError = nil
        }
        startDeviceMonitoring()
        // Use [weak self] so that if SwiftUI/Xcode discards this AppState
        // while the init task is still awaiting SQLite / HTTP, the old
        // instance can release promptly instead of being pinned alive by a
        // strong capture on `self` through the awaited method calls.
        Task { @MainActor [weak self] in await self?.warmUpAfterLaunch() }
    }

    /// Registers the secret store before any launch work that might read a
    /// seed or a private key, and records the failure where both the user and
    /// a diagnostics export can see it. The service registers the
    /// Keychain-backed secret store as it is created.
    private func registerSecretStoreWithBridge() async {
        do {
            _ = try bridge.service()
            secretStoreRegistrationError = nil
        } catch {
            secretStoreRegistrationError = userErrorMessage(error)
            appendOperationalLog(
                .error, category: "Secret Store", message: "Secret store registration failed: \(String(describing: error))",
                source: "WalletServiceBridge.service")
        }
    }
    private func warmUpAfterLaunch() async {
        await registerSecretStoreWithBridge()
        setupRustRefreshEngine()
        await reloadCoreProjections()
        // Configuring the engine starts it; its first tick performs the launch
        // sweep, prices and exchange rates included.
    }
    deinit {
        userInitiatedRefreshTask?.cancel()
        balanceProgressTask?.cancel()
        deviceConditionsTask?.cancel()
        refreshEventsTask?.cancel()
        #if canImport(Network)
            networkPathMonitor.cancel()
        #endif
    }
    var canImportWallet: Bool {
        walletImport.draft.canImportWallet
    }
}
