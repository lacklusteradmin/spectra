import SwiftUI

/// The address core will store for the form as it stands, from the planning
/// the import itself runs, so the two cannot disagree. It follows the inputs
/// a moment after they settle and derives on core's worker; nothing is sent
/// anywhere and nothing is stored.
struct WalletAddressPreviewCard: View {
    let store: AppState
    let draft: WalletImportDraft

    private enum Preview: Equatable {
        case ready(addresses: [String], rejected: [String], upgrades: String?)
        case failed(String)
    }
    @State private var preview: Preview?

    var body: some View {
        // A container that is always there: an empty conditional is no view,
        // and its task would never start.
        VStack(spacing: 0) { content }
            .task(id: draft.previewCommit) { await refresh(draft.previewCommit) }
    }

    @ViewBuilder
    private var content: some View {
        if let preview {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                switch preview {
                case .ready(let addresses, let rejected, let upgrades):
                    Text(AppLocalization.string(addresses.count == 1 ? "Address" : "Addresses"))
                        .font(.subheadline.weight(.semibold))
                    ForEach(addresses, id: \.self) { address in
                        Text(address).font(.footnote.monospaced()).foregroundStyle(Color.primary)
                            .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    }
                    Text(caption).font(.caption).foregroundStyle(.secondary)
                    if let upgrades {
                        Label(
                            AppLocalization.format("This adds keys to the watched wallet “%@”, keeping its history.", upgrades),
                            systemImage: "key.fill"
                        ).font(.caption.weight(.medium)).foregroundStyle(.tint)
                    }
                    if !rejected.isEmpty {
                        Text(AppLocalization.format(
                            "These lines are not valid addresses and will not be imported: %@", rejected.joined(separator: ", ")))
                            .font(.caption).foregroundStyle(.red.opacity(0.9))
                    }
                case .failed(let message):
                    Label(message, systemImage: "exclamationmark.triangle.fill").font(.footnote.weight(.medium))
                        .foregroundStyle(.red.opacity(0.92))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
        }
    }

    private var caption: String {
        switch draft.method {
        case .createPhrase: AppLocalization.string("Your new wallet receives at this address.")
        case .watchAddresses, .watchAccountXpub: AppLocalization.string("Spectra will watch what is listed here.")
        default: AppLocalization.string("Check that it matches the wallet you are restoring before you continue.")
        }
    }

    /// Cancelled and restarted whenever the commit changes, so a burst of
    /// typing derives once, after it stops.
    private func refresh(_ commit: WalletImportCommit?) async {
        guard let commit else {
            preview = nil
            return
        }
        try? await Task.sleep(for: .milliseconds(300))
        guard !Task.isCancelled else { return }
        do {
            let result = try await store.bridge.ready().previewWalletImport(commit: commit)
            guard !Task.isCancelled else { return }
            preview = .ready(addresses: result.addresses, rejected: result.rejectedAddresses, upgrades: result.upgradesWallet)
        } catch {
            guard !Task.isCancelled else { return }
            preview = .failed(userErrorMessage(error))
        }
    }
}
