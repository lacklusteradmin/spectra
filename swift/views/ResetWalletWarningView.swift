import Foundation
import SwiftUI

/// What to remove from this device. Nothing is chosen until the user chooses
/// it, and every reset is confirmed once more before anything is deleted.
struct ResetWalletWarningView: View {
    let store: AppState
    @Environment(\.dismiss) private var dismiss
    @State private var selectedScopes: Set<ResetScope> = []
    @State private var isConfirming = false
    @State private var isResetting = false
    @State private var errorMessage: String?
    var body: some View {
        Form {
            Section {
                Text(
                    AppLocalization.string(
                        "Choose which categories to remove from this device. Selected items are deleted locally and some options also clear secure keychain data."
                    ))
                Label(
                    AppLocalization.string(
                        "You must have your seed phrase backed up. Without it, you cannot recover your funds after reset."),
                    systemImage: "exclamationmark.triangle.fill"
                ).font(.body.weight(.semibold)).foregroundStyle(.red)
            }
            Section {
                ForEach(ResetScope.allCases, id: \.self) { scope in
                    Toggle(isOn: binding(for: scope)) {
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                            Text(scope.title)
                            Text(scope.detail).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            } header: {
                Text(AppLocalization.string("Choose What To Reset"))
            } footer: {
                if selectedScopes.isEmpty {
                    Text(AppLocalization.string("Select at least one category to enable reset."))
                }
            }
            Section {
                Button(AppLocalization.string("Reset Selected Data"), role: .destructive) { isConfirming = true }
                    .disabled(selectedScopes.isEmpty || isResetting)
                if let errorMessage {
                    Text(errorMessage).font(.caption).foregroundStyle(.red)
                }
            }
        }
        .navigationTitle(AppLocalization.string("Reset Wallet"))
        .navigationBarTitleDisplayMode(.inline)
        .confirmationDialog(
            AppLocalization.string("Delete the selected data?"), isPresented: $isConfirming, titleVisibility: .visible
        ) {
            Button(AppLocalization.string("Reset Selected Data"), role: .destructive) { reset() }
        } message: {
            Text(confirmationMessage)
        }
    }

    /// The chosen categories by name, in the page's order, and that this
    /// cannot be undone.
    private var confirmationMessage: String {
        let titles = ResetScope.allCases.filter(selectedScopes.contains).map(\.title)
        return AppLocalization.format(
            "reset.confirm.message_format", titles.formatted(.list(type: .and).locale(AppLocalization.locale)))
    }

    private func reset() {
        isResetting = true
        Task {
            errorMessage = await store.resetSelectedData(scopes: selectedScopes)
            isResetting = false
            if errorMessage == nil { dismiss() }
        }
    }

    private func binding(for scope: ResetScope) -> Binding<Bool> {
        Binding(
            get: { selectedScopes.contains(scope) },
            set: { isSelected in
                if isSelected { selectedScopes.insert(scope) } else { selectedScopes.remove(scope) }
            }
        )
    }
}
