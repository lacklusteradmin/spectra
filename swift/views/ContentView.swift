import SwiftUI

struct ContentView: View {
    let store: AppState
    @Environment(\.scenePhase) private var scenePhase
    @State private var hasBeenActive = false

    /// What covers the app, if anything. The snapshot cover wins while the
    /// app is not active, whatever the lock settings, so the app switcher
    /// never shows balances or addresses; it lifts on return without asking
    /// anything of the user. The lock guards use and stays until unlocked.
    private var cover: AppCover? {
        if scenePhase != .active { return .snapshot }
        if store.isAppLocked { return .locked }
        return store.isStoreUnreadable ? .unreadableStore : nil
    }

    private func handleScenePhase(_ phase: ScenePhase) {
        switch phase {
        case .active:
            store.setAppIsActive(true)
            // The first activation is the launch, whose refresh is the
            // engine's first tick; only the activities left from a previous
            // run need settling.
            let isLaunch = !hasBeenActive
            hasBeenActive = true
            Task {
                if isLaunch { await store.notifications.reconcileSendLiveActivities(amounts: store.amounts) }
                else { await store.refreshForForegroundIfNeeded() }
            }
        case .background:
            store.setAppIsActive(false)
        // Not leaving the app: the Face ID sheet, Control Center and the app
        // switcher all pass through here. Locking on it locked the app behind
        // every authentication it asked for.
        case .inactive: break
        @unknown default: break
        }
    }

    var body: some View {
        MainTabView(store: store)
            .accessibilityHidden(cover != nil)
            .sceneCover(item: cover) { cover in
                AppCoverView(store: store, cover: cover)
                    .preferredColorScheme(store.preferences.appearanceMode.colorScheme)
                    .environment(\.locale, AppLocalization.locale)
            }
            .preferredColorScheme(store.preferences.appearanceMode.colorScheme)
            .environment(\.locale, AppLocalization.locale)
            .onChange(of: scenePhase, initial: true) { _, phase in handleScenePhase(phase) }
    }
}

private enum AppCover: Equatable {
    case snapshot, locked, unreadableStore
}

private struct AppCoverView: View {
    let store: AppState
    let cover: AppCover

    var body: some View {
        ZStack {
            SpectraBackdrop()
            switch cover {
            case .snapshot:
                SpectraLogo()
            case .locked:
                lockCard
            case .unreadableStore:
                UnreadableStoreCard(store: store)
            }
        }
    }

    private var lockCard: some View {
        VStack(spacing: SpectraLayout.Space.m) {
            Image(systemName: "lock.fill").font(.system(size: 40, weight: .semibold)).foregroundStyle(.secondary)
            Text(AppLocalization.string("content.locked.title")).font(.title3.weight(.semibold))
            Text(AppLocalization.string("content.locked.subtitle")).font(.subheadline).foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            if let appLockError = store.appLockError { Text(appLockError).font(.caption).foregroundStyle(.red) }
            Button {
                Task { await store.unlockApp() }
            } label: {
                Label(AppLocalization.format("Unlock with %@", DeviceBiometry.current.name), systemImage: DeviceBiometry.current.symbol)
                    .font(.body.weight(.semibold)).frame(maxWidth: 220).padding(.vertical, SpectraLayout.Space.xs)
            }.buttonStyle(.glassProminent).controlSize(.large)
        }.padding(SpectraLayout.Space.xl).spectraElevatedFill().padding(SpectraLayout.Space.xl)
    }
}

/// The store holds data this build cannot read. Nothing else can run, so
/// this is the whole app until the data is discarded.
private struct UnreadableStoreCard: View {
    let store: AppState
    @State private var isConfirming = false
    @State private var isDiscarding = false
    @State private var errorMessage: String?

    var body: some View {
        VStack(spacing: SpectraLayout.Space.m) {
            Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 40, weight: .semibold))
                .foregroundStyle(.spectraWarning)
            Text(AppLocalization.string("content.unreadable.title")).font(.title3.weight(.semibold))
                .multilineTextAlignment(.center)
            Text(AppLocalization.string("content.unreadable.detail")).font(.subheadline).foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            if let errorMessage { Text(errorMessage).font(.caption).foregroundStyle(.red) }
            Button(role: .destructive) {
                isConfirming = true
            } label: {
                Label(AppLocalization.string("content.unreadable.reset"), systemImage: "trash")
                    .font(.body.weight(.semibold)).frame(maxWidth: 220).padding(.vertical, SpectraLayout.Space.xs)
            }
            .buttonStyle(.glassProminent).tint(.red).controlSize(.large).disabled(isDiscarding)
            .confirmationDialog(
                AppLocalization.string("content.unreadable.confirm"), isPresented: $isConfirming, titleVisibility: .visible
            ) {
                Button(AppLocalization.string("content.unreadable.reset"), role: .destructive) {
                    Task {
                        isDiscarding = true
                        errorMessage = await store.discardUnreadableStore()
                        isDiscarding = false
                    }
                }
            }
        }.padding(SpectraLayout.Space.xl).spectraElevatedFill().padding(SpectraLayout.Space.xl)
    }
}

#Preview {
    ContentView(store: AppState())
}
