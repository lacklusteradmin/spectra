import Foundation
import SwiftUI

enum AppearanceMode: String, CaseIterable, Identifiable {
    case system, light, dark
    var id: String { rawValue }
    var label: String {
        switch self {
        case .dark: return "Dark"
        case .light: return "Light"
        case .system: return "System"
        }
    }
    /// `nil` follows the system.
    var colorScheme: ColorScheme? {
        switch self {
        case .dark: return .dark
        case .light: return .light
        case .system: return nil
        }
    }
}

/// Device-local preferences stored synchronously in `UserDefaults`.
/// Appearance and balance privacy must be available before the first frame.
private enum PlatformDefaults {
    static let hideBalances = "settings.platform.hideBalances"
    static let hideSmallBalances = "settings.platform.hideSmallBalances"
    static let appearanceMode = "settings.appearanceMode"
    static let useFaceId = "settings.platform.useFaceId"
    static let useAutoLock = "settings.platform.useAutoLock"
    static let requireBiometricForSendActions = "settings.platform.requireBiometricForSendActions"

    static func bool(_ key: String, default value: Bool) -> Bool {
        UserDefaults.standard.object(forKey: key) as? Bool ?? value
    }
    static func set(_ value: Bool, _ key: String) { UserDefaults.standard.set(value, forKey: key) }
}

/// The six preferences this platform keeps for itself, split out of
/// `AppState` so that views which only read them are not invalidated whenever
/// wallets, balances or transactions change. Settings core owns are
/// `AppState.appSettings`.
@MainActor
@Observable
final class AppUserPreferences {
    // ── UI ──────────────────────────────────────────────────────────────
    var hideBalances: Bool = PlatformDefaults.bool(PlatformDefaults.hideBalances, default: false) {
        didSet { if hideBalances != oldValue { PlatformDefaults.set(hideBalances, PlatformDefaults.hideBalances) } }
    }
    /// Fold Home's rows worth less than core's small-balance threshold — the
    /// dust and airdropped tokens anyone can send an address — behind one row.
    var hideSmallBalances: Bool = PlatformDefaults.bool(PlatformDefaults.hideSmallBalances, default: true) {
        didSet { if hideSmallBalances != oldValue { PlatformDefaults.set(hideSmallBalances, PlatformDefaults.hideSmallBalances) } }
    }
    var appearanceMode: AppearanceMode = {
        if let raw = UserDefaults.standard.string(forKey: PlatformDefaults.appearanceMode),
           let saved = AppearanceMode(rawValue: raw) { return saved }
        return .system
    }() {
        didSet {
            guard appearanceMode != oldValue else { return }
            UserDefaults.standard.set(appearanceMode.rawValue, forKey: PlatformDefaults.appearanceMode)
        }
    }

    // ── Security ────────────────────────────────────────────────────────
    var useFaceId: Bool = PlatformDefaults.bool(PlatformDefaults.useFaceId, default: true) {
        didSet {
            guard useFaceId != oldValue else { return }
            PlatformDefaults.set(useFaceId, PlatformDefaults.useFaceId)
            if !useFaceId { useFaceIDDisabledHandler?() }
        }
    }
    var useAutoLock: Bool = PlatformDefaults.bool(PlatformDefaults.useAutoLock, default: false) {
        didSet { if useAutoLock != oldValue { PlatformDefaults.set(useAutoLock, PlatformDefaults.useAutoLock) } }
    }
    var requireBiometricForSendActions: Bool = PlatformDefaults.bool(
        PlatformDefaults.requireBiometricForSendActions, default: true)
    {
        didSet {
            guard requireBiometricForSendActions != oldValue else { return }
            PlatformDefaults.set(requireBiometricForSendActions, PlatformDefaults.requireBiometricForSendActions)
        }
    }

    /// Wired by `AppState` in its init. Kept out of `@Observable` tracking so
    /// assigning the closure does not invalidate views.
    @ObservationIgnored var useFaceIDDisabledHandler: (() -> Void)?

    /// Reset to factory defaults. Each value writes itself back to
    /// `UserDefaults` as it changes.
    func resetToDefaults() {
        hideBalances = false
        hideSmallBalances = true
        appearanceMode = .system
        useFaceId = true
        useAutoLock = false
        requireBiometricForSendActions = true
    }
}
