// Presentation helpers for core records, app navigation and displayed errors.

import Foundation

/// An operational log entry recorded, bounded, and persisted by core.
/// Both the logs screen and chain diagnostics read this record.
extension DiagnosticLog: Identifiable {
    var timestamp: Date { Date(timeIntervalSince1970: timestampUnix) }
}

extension DiagnosticLogLevel {
    static let allCases: [DiagnosticLogLevel] = [.debug, .info, .warning, .error]
    var displayName: String {
        switch self {
        case .debug: return AppLocalization.string("Debug")
        case .info: return AppLocalization.string("Info")
        case .warning: return AppLocalization.string("Warning")
        case .error: return AppLocalization.string("Error")
        }
    }
    /// The export's tag. Not localized: the export is read by whoever debugs it.
    var exportTag: String {
        switch self {
        case .debug: return "DEBUG"
        case .info: return "INFO"
        case .warning: return "WARNING"
        case .error: return "ERROR"
        }
    }
}
/// A failure already worded for the reader.
/// Where launch stands with the stored data.
enum StoreStatus: Equatable {
    /// Core has not answered yet.
    case opening
    /// Core has answered and the app runs on what it read. A failure other
    /// than unreadable data — the file could not be opened — is logged and
    /// lands here too: offering to discard data over an error that may pass
    /// would destroy it.
    case open
    /// Core cannot read what is stored: discarding it is the only way out,
    /// and nothing else can run.
    case unreadable
}

struct DisplayedError: LocalizedError {
    let errorDescription: String?
    init(_ message: String) { errorDescription = message }
}

enum MainAppTab: Hashable {
    case home
    case history
    case settings
}

/// What a reset clears. Core's scope, with this app's words for it.
extension ResetScope {
    static let allCases: [ResetScope] = [
        .walletsAndSecrets, .historyAndCache, .alertsAndContacts, .settingsAndEndpoints, .dashboardCustomization,
    ]
    @MainActor
    var title: String {
        switch self {
        case .walletsAndSecrets: return AppLocalization.string("Wallets & Secrets")
        case .historyAndCache: return AppLocalization.string("History & Cache")
        case .alertsAndContacts: return AppLocalization.string("Alerts & Contacts")
        case .settingsAndEndpoints: return AppLocalization.string("Settings & Endpoints")
        case .dashboardCustomization: return AppLocalization.string("Dashboard Customization")
        }
    }
    @MainActor
    var detail: String {
        switch self {
        case .walletsAndSecrets:
            return AppLocalization.string("Imported wallets, seed phrases, watched addresses, and local wallet access data.")
        case .historyAndCache:
            return AppLocalization.string("Transactions, history database, diagnostics snapshots, and cached chain state.")
        case .alertsAndContacts: return AppLocalization.string("Price alerts, notification rules, and saved address book recipients.")
        case .settingsAndEndpoints:
            return AppLocalization.string("Known tokens, pricing and RPC settings, preferences, and icon customizations.")
        case .dashboardCustomization:
            return AppLocalization.string("Pinned assets and other home page customization choices stored on this device.")
        }
    }
}

extension AppState {
    enum SeedPhraseRevealError: LocalizedError {
        case unavailable
        case authenticationFailed(String)
        case passwordRequired
        case invalidPassword
        case passwordNotRequired
        var errorDescription: String? {
            switch self {
            case .unavailable: return AppLocalization.string("No seed phrase is stored for this wallet.")
            case .authenticationFailed(let reason): return reason
            case .passwordRequired: return AppLocalization.string("Enter the wallet password to view this seed phrase.")
            case .invalidPassword: return AppLocalization.string("The wallet password is incorrect.")
            case .passwordNotRequired: return AppLocalization.string("This wallet has no password.")
            }
        }
    }
}

extension KeypoolDiagnostic: Identifiable {
    public var id: String { walletId }
}
