import SwiftUI

/// A cancellable projection of core's durable scan. No keys or scan state live in Swift.
struct MoneroSyncView: View {
    let store: AppState
    let walletId: String
    @State private var vm = MoneroSyncViewModel()
    @State private var statusAttempt = 0

    var body: some View {
        // Always drawn: a status that cannot be read is said here, with a
        // way to ask again, rather than leaving the page empty.
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(AppLocalization.string("Local Monero Wallet")).font(.headline)
            Text(AppLocalization.string("Scanning and signing happen on this device. Keys stay on this device."))
                .font(.caption).foregroundStyle(.secondary)
            if let status = vm.status {
                progress(status)
                if !status.spendsKnown {
                    Text(AppLocalization.string("A view key shows what the wallet receives, not what it spends: its balance does not drop when it pays from another device."))
                        .font(.caption).foregroundStyle(.secondary)
                }
                if vm.isRunning {
                    Button(AppLocalization.string("Cancel")) { vm.cancel() }
                        .buttonStyle(.glass)
                } else {
                    if store.wallet(for: walletId)?.signing.requiresPassword ?? true {
                        SecureField(AppLocalization.string("Wallet Password"), text: $vm.password)
                            .padding(SpectraLayout.Space.m)
                            .spectraInputFieldStyle()
                    }
                    Button(AppLocalization.string("Sync Local Wallet")) { vm.begin() }
                        .buttonStyle(.glassProminent)
                }
            } else if vm.error == nil {
                ProgressView().frame(maxWidth: .infinity)
            }
            if let error = vm.error {
                Label(error, systemImage: "exclamationmark.triangle.fill").font(.caption).foregroundStyle(.red)
                if vm.status == nil {
                    Button(AppLocalization.string("Retry")) { statusAttempt += 1 }.buttonStyle(.glass)
                }
            }
        }
        .padding(SpectraLayout.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
        .task(id: "\(walletId)#\(statusAttempt)") {
            vm.error = nil
            do {
                let status = try await store.moneroSyncStatus(walletId: walletId)
                guard !Task.isCancelled else { return }
                vm.status = status
            } catch {
                guard !Task.isCancelled else { return }
                vm.error = userErrorMessage(error)
            }
        }
        .task(id: vm.requestId) {
            guard let request = vm.requestId else { return }
            await vm.sync(request: request) { password, progress in
                await store.syncMoneroWallet(walletId: walletId, password: password, progress: progress)
            }
        }
        .onDisappear { vm.cancel() }
    }

    /// How far the scan has come, in blocks from where it started — the
    /// wallet's restore height — to the chain's tip.
    @ViewBuilder
    private func progress(_ status: MoneroSyncStatus) -> some View {
        let start = min(store.wallet(for: walletId)?.restoreHeight ?? 0, status.scannedHeight)
        let total = status.targetHeight > start ? Double(status.targetHeight - start) : 0
        let done = Double(status.scannedHeight - start)
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            if total > 0 {
                ProgressView(value: min(done, total), total: total)
            }
            Text(AppLocalization.format(
                "monero.scan.blocks_format",
                status.scannedHeight.formatted(.number.locale(AppLocalization.locale)),
                status.targetHeight.formatted(.number.locale(AppLocalization.locale))))
                .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }
}

/// A scanning wallet's page for its scan: where the scan started, set when
/// the wallet was added, and the scan itself.
struct WalletBlockScanView: View {
    let store: AppState
    let wallet: WalletView

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                if let height = wallet.restoreHeight {
                    HStack {
                        Text(AppLocalization.string("Restore Height")).font(.subheadline)
                        Spacer()
                        Text(verbatim: "\(height)").font(.subheadline.monospacedDigit()).foregroundStyle(.secondary)
                    }
                    .padding(SpectraLayout.cardPadding).spectraCardFill()
                }
                MoneroSyncView(store: store, walletId: wallet.id)
            }.spectraScreenPadding()
        }
        .background(SpectraBackdrop().ignoresSafeArea())
        .navigationTitle(WalletAction.scanBlocks.title).navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
    }
}
