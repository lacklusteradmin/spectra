import SwiftUI

// The wallet tool pages — coins, approvals, NFTs, access keys, trust lines
// and the rest — are system lists over one read from core. They share the
// states around that read, drawn here once so every page says them the same
// way.

/// Core's answer was a failure: its reason, with a symbol, so the state is
/// not carried by colour alone. Pull to refresh asks again.
struct WalletToolErrorSection: View {
    let message: String
    var body: some View {
        Section {
            Label(message, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.red)
        }
    }
}

/// The first read has not answered yet.
struct WalletToolLoadingSection: View {
    var body: some View {
        Section { ProgressView().frame(maxWidth: .infinity) }
    }
}

/// The read answered with nothing to list.
struct WalletToolEmptySection: View {
    let message: String
    var body: some View {
        Section { Text(AppLocalization.string(message)).foregroundStyle(.secondary) }
    }
}
