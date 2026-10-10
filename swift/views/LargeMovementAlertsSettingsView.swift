import Foundation
import SwiftUI
struct LargeMovementAlertsSettingsView: View {
    @Bindable var store: AppState
    @FocusState private var amountFieldFocused: Bool
    private var settings: AppSettings { store.appSettings }
    var body: some View {
        Form {
            Section(AppLocalization.string("Notifications")) {
                Toggle(isOn: store.settingBinding(\.useLargeMovementNotifications) { .useLargeMovementNotifications(value: $0) }) {
                    Label(AppLocalization.string("Large Portfolio Movement Alerts"), systemImage: "chart.line.uptrend.xyaxis")
                }
            }
            Section(AppLocalization.string("Alert Controls")) {
                Stepper(
                    AppLocalization.format(
                        "Large movement threshold: %@",
                        (settings.largeMovementAlertPercentThreshold / 100).formatted(
                            .percent.precision(.fractionLength(0)).locale(AppLocalization.locale))
                    ),
                    value: store.settingBinding(\.largeMovementAlertPercentThreshold) {
                        .largeMovementAlertPercentThreshold(value: $0)
                    }, in: CoreReferenceTables.bounds.largeMovementPercentMin...CoreReferenceTables.bounds.largeMovementPercentMax,
                    step: 1
                ).disabled(!settings.useLargeMovementNotifications)
                LabeledContent(AppLocalization.string("Minimum movement (USD)")) {
                    TextField(
                        AppLocalization.string("Minimum movement (USD)"),
                        value: store.settingBinding(\.largeMovementAlertUsdThreshold) {
                            .largeMovementAlertUsdThreshold(value: $0)
                        },
                        format: .number.grouping(.never).locale(AppLocalization.locale)
                    )
                    .keyboardType(.decimalPad)
                    .multilineTextAlignment(.trailing)
                    .focused($amountFieldFocused)
                }.disabled(!settings.useLargeMovementNotifications)
            }
        }.navigationTitle(AppLocalization.string("Large Movement Alerts"))
        .toolbar {
            ToolbarItemGroup(placement: .keyboard) {
                Spacer()
                Button(AppLocalization.string("Done")) { amountFieldFocused = false }
            }
        }
    }
}
