import Foundation
import UIKit
#if canImport(Network)
    import Network
#endif

extension AppState {
    func startNetworkPathMonitorIfNeeded() {
        #if canImport(Network)
            networkPathMonitor.pathUpdateHandler = { [weak self] path in
                let reachable = path.status == .satisfied
                let constrained = path.isConstrained
                let expensive = path.isExpensive
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    self.isNetworkReachable = reachable
                    self.isConstrainedNetwork = constrained
                    self.isExpensiveNetwork = expensive
                    self.reportDeviceConditions()
                }
            }
            networkPathMonitor.start(queue: networkPathMonitorQueue)
        #endif
    }
    /// Core's engine starts both refresh loops in the foreground and stops
    /// them in the background. Called on entering the foreground and the
    /// background only; `.inactive` is not leaving the app.
    func setAppIsActive(_ isActive: Bool) {
        appIsActive = isActive
        if !isActive, preferences.useFaceId, preferences.useAutoLock {
            isAppLocked = true
            appLockError = nil
        }
        reportDeviceConditions()
    }
    /// Watch the battery and the network path, which core's refresh policy
    /// reads through `deviceConditions()`. Domain state all arrives from core.
    func startDeviceMonitoring() {
        UIDevice.current.isBatteryMonitoringEnabled = true
        startNetworkPathMonitorIfNeeded()
    }
    func refreshForForegroundIfNeeded() async {
        await performCoreRefresh(.foreground)
        await notifications.reconcileSendLiveActivities(amounts: amounts)
    }
}
