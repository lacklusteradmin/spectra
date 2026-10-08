import SwiftUI

/// A cancellable projection of core's durable scan. No keys or scan state live in Swift.
struct MoneroSyncView: View {
    let store: AppState
    let walletId: String
    @State private var vm = MoneroSyncViewModel()

    var body: some View {
        Group {
            if let status = vm.status {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    Text(AppLocalization.string("Local Monero Wallet")).font(.headline)
                    Text(AppLocalization.string("Scanning and signing happen on this device. Keys stay on this device."))
                        .font(.caption).foregroundStyle(.secondary)
                    Text(verbatim: "\(status.scannedHeight) / \(status.targetHeight)")
                        .monospacedDigit()
                    if !status.spendsKnown {
                        Text(AppLocalization.string("A view key shows what the wallet receives, not what it spends: its balance does not drop when it pays from another device."))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if vm.isRunning {
                        ProgressView()
                        Button(AppLocalization.string("Cancel")) { vm.cancel() }
                            .buttonStyle(.glass)
                    } else {
                        if store.wallet(for: walletId)?.signing.requiresPassword ?? true {
                            SecureField(AppLocalization.string("Wallet Password"), text: $vm.password)
                                .spectraInputFieldStyle()
                        }
                        Button(AppLocalization.string("Sync Local Wallet")) { vm.begin() }
                            .buttonStyle(.glassProminent)
                    }
                    if let error = vm.error { Text(error).font(.caption).foregroundStyle(.red) }
                }
                .padding(SpectraLayout.cardPadding)
                .spectraCardFill()
            }
        }
        .task(id: walletId) {
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
