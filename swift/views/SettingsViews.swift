import Foundation
import SwiftUI
struct SettingsView: View {
    @Bindable var store: AppState
    @State private var isShowingResetWalletWarning: Bool = false
    private enum Route: Hashable {
        case addressBook
        case knownTokens
        case appearance
        case priceAlerts
        case largeMovementAlerts
        case pricing
        case endpoints
        case explorers
        case diagnostics
        case operationalLogs
        case reportProblem
        case buyCryptoHelp
        case about
        case cryptoWiki
        case advanced
        case donate
        case tor
    }
    var body: some View {
        NavigationStack {
            Form {
                Section(AppLocalization.string("Wallet & Transfers")) {
                    settingsLink("Address Book", systemImage: "book.closed", route: .addressBook)
                    settingsLink("Known Tokens", systemImage: "bitcoinsign.bank.building", route: .knownTokens)
                }
                Section(AppLocalization.string("Display")) {
                    settingsToggle("Hide balances", systemImage: "eye.slash", isOn: preferenceBinding(\.hideBalances))
                    settingsLink("Appearance", systemImage: "circle.lefthalf.filled", route: .appearance)
                }
                Section(AppLocalization.string("Notifications")) {
                    settingsLink("Price Alerts", systemImage: "bell.badge", route: .priceAlerts)
                    settingsToggle(
                        "Transaction Status Updates", systemImage: "clock.badge.checkmark",
                        isOn: store.settingBinding(\.useTransactionStatusNotifications) {
                            .useTransactionStatusNotifications(value: $0)
                        })
                    settingsLink("Large Movement Alerts", systemImage: "chart.line.uptrend.xyaxis", route: .largeMovementAlerts)
                }
                Section(AppLocalization.string("Security & Privacy")) {
                    settingsToggle("Use Face ID", systemImage: "faceid", isOn: preferenceBinding(\.useFaceId))
                    settingsToggle("Auto Lock", systemImage: "lock", isOn: preferenceBinding(\.useAutoLock))
                        .disabled(!store.preferences.useFaceId)
                }
                Section(AppLocalization.string("Tor")) {
                    NavigationLink(value: Route.tor) {
                        HStack(spacing: SpectraLayout.Space.m) {
                            Label(AppLocalization.string("Tor Network"), systemImage: "network.badge.shield.half.filled")
                            Spacer(minLength: SpectraLayout.Space.s)
                            TorStatusBadge(status: store.tor.status)
                        }
                    }
                }
                Section(AppLocalization.string("Data & Connectivity")) {
                    settingsLink("Pricing", systemImage: "dollarsign.circle", route: .pricing)
                    settingsLink("Endpoints", systemImage: "network", route: .endpoints)
                    settingsLink("Explorers", systemImage: "safari", route: .explorers)
                }
                Section(AppLocalization.string("Diagnostics & Support")) {
                    settingsLink("Diagnostics", systemImage: "waveform.path.ecg.rectangle", route: .diagnostics)
                    settingsLink("Operational Logs", systemImage: "doc.text.magnifyingglass", route: .operationalLogs)
                    settingsLink("Report a Problem", systemImage: "exclamationmark.bubble", route: .reportProblem)
                }
                Section(AppLocalization.string("Help")) {
                    settingsLink("Where can I buy crypto?", systemImage: "creditcard", route: .buyCryptoHelp)
                }
                Section(AppLocalization.string("About")) {
                    settingsLink("About Spectra", systemImage: "info.circle", route: .about)
                    settingsLink("Crypto Wiki", systemImage: "books.vertical", route: .cryptoWiki)
                    settingsLink("Donate", systemImage: "heart", route: .donate)
                }
                Section(AppLocalization.string("Advanced")) {
                    settingsLink("Advanced", systemImage: "slider.horizontal.3", route: .advanced)
                }
                Section(AppLocalization.string("Reset")) {
                    Button(role: .destructive) {
                        isShowingResetWalletWarning = true
                    } label: {
                        Label(AppLocalization.string("Reset Wallet"), systemImage: "trash")
                    }
                }
            }
            .navigationTitle(AppLocalization.string("Settings"))
            .navigationBarTitleDisplayMode(.inline)
            .navigationDestination(for: Route.self) { route in
                switch route {
                case .addressBook: AddressBookView(addressBook: store.addressBook)
                case .knownTokens: TokenRegistrySettingsView(tokens: store.tokenPreferences)
                case .appearance: AppearanceSettingsView(preferences: store.preferences)
                case .priceAlerts: PriceAlertsView(store: store)
                case .largeMovementAlerts: LargeMovementAlertsSettingsView(store: store)
                case .pricing: PricingSettingsView(store: store)
                case .endpoints: EndpointCatalogSettingsView(store: store)
                case .explorers: ExplorerSettingsView()
                case .diagnostics: DiagnosticsHubView(store: store)
                case .operationalLogs: LogsView(store: store)
                case .reportProblem: ReportProblemView()
                case .buyCryptoHelp: BuyCryptoHelpView()
                case .about: AboutView()
                case .cryptoWiki: CryptoWikiLibraryView()
                case .donate: DonationsView()
                case .advanced: AdvancedSettingsView(store: store)
                case .tor: TorSettingsView(store: store)
                }
            }.sheet(isPresented: $isShowingResetWalletWarning) {
                ResetWalletWarningView(store: store)
            }
        }
    }

    private func preferenceBinding(_ keyPath: ReferenceWritableKeyPath<AppUserPreferences, Bool>) -> Binding<Bool> {
        Binding(
            get: { store.preferences[keyPath: keyPath] },
            set: { store.preferences[keyPath: keyPath] = $0 }
        )
    }

    @ViewBuilder
    private func settingsLink(_ title: String, systemImage: String, route: Route) -> some View {
        NavigationLink(value: route) {
            Label(AppLocalization.string(title), systemImage: systemImage)
        }
    }
    @ViewBuilder
    private func settingsToggle(_ title: String, systemImage: String, isOn: Binding<Bool>) -> some View {
        Toggle(isOn: isOn) {
            Label(AppLocalization.string(title), systemImage: systemImage)
        }
    }

}
