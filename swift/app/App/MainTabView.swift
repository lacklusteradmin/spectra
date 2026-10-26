import SwiftUI

struct MainTabView: View {
    @Bindable var store: AppState
    var body: some View {
        TabView(selection: $store.selectedMainTab) {
            Tab(AppLocalization.string("Home"), systemImage: "chart.pie.fill", value: MainAppTab.home) {
                DashboardView(store: store)
            }
            // A send still waiting on its network is worth a glance from any
            // tab; zero shows no badge.
            Tab(AppLocalization.string("History"), systemImage: "clock.arrow.circlepath", value: MainAppTab.history) {
                HistoryView(store: store)
            }
            .badge(Int(store.pendingTransactionCount))
            Tab(AppLocalization.string("Settings"), systemImage: "gearshape.fill", value: MainAppTab.settings) {
                SettingsView(store: store)
            }
        }
        .tabBarMinimizeBehavior(.onScrollDown)
    }
}
