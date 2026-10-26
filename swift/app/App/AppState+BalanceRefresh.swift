import Foundation
import SwiftUI
extension AppState {
    /// Refresh every wallet's balances now: the engine sweeps its entries together.
    func refreshBalances() async { try? await self.bridge.refreshEngine().triggerImmediate() }

    /// Install the observer and report the device. Core owns both refresh
    /// loops and their cadence from here on.
    func setupRustRefreshEngine() {
        let observer = WalletRefreshObserver()
        observeRefreshEvents(from: observer)
        let bridge = self.bridge
        // The observer is in place before the first report starts the loops.
        let previous = deviceConditionsTask
        deviceConditionsTask = Task {
            await previous?.value
            try? await bridge.setRefreshObserver(observer)
        }
        reportDeviceConditions()
    }

    /// Adopt the observer's events one at a time, in the order core sent them.
    /// The task holds only the stream, so neither it nor core's engine keeps
    /// this state alive.
    func observeRefreshEvents(from observer: WalletRefreshObserver) {
        let events = observer.events
        refreshEventsTask?.cancel()
        refreshEventsTask = Task { [weak self] in
            for await event in events {
                guard let self else { return }
                switch event {
                case .balanceUpdated: self.adoptBalanceProgress()
                case .refreshComplete(let result): await self.adoptRefreshResult(result)
                case .torStatusChanged(let status): self.tor.adopt(status)
                }
            }
        }
    }

    /// Device-local inputs to core's refresh policy. Core owns the profile,
    /// cadence, last-run times and pending-send polling decisions.
    func deviceConditions() -> DeviceConditions {
        let battery = UIDevice.current.batteryLevel
        return DeviceConditions(
            appIsActive: appIsActive,
            isNetworkReachable: isNetworkReachable,
            isConstrainedNetwork: isConstrainedNetwork,
            isExpensiveNetwork: isExpensiveNetwork,
            isLowPowerMode: ProcessInfo.processInfo.isLowPowerModeEnabled,
            batteryLevel: battery < 0 ? 1.0 : battery,
            // Prices are on screen only on the home tab.
            wantsPriceRefresh: selectedMainTab == .home)
    }

    /// Tell core's engine what changed on the device, in the order it changed.
    func reportDeviceConditions() {
        guard servicesEnabled else { return }
        let conditions = deviceConditions()
        let previous = deviceConditionsTask
        let bridge = self.bridge
        deviceConditionsTask = Task {
            await previous?.value
            try? await bridge.refreshEngine().setDeviceConditions(conditions: conditions)
        }
    }
}
