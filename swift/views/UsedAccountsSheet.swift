import SwiftUI

/// Find the accounts a phrase has used on the chosen network: core scans the
/// network's derivation profiles at the first accounts, reading balance and
/// history, or on TON each wallet version's account. The addresses go to a
/// provider before any wallet exists, so the sheet names the endpoints first
/// and scans only when asked. Choosing a used account sets the import's
/// profile and account, or its wallet version.
struct UsedAccountsSheet: View {
    let store: AppState
    @Bindable var draft: WalletImportDraft
    @Environment(\.dismiss) private var dismiss
    @State private var scan: FundsScan?
    @State private var endpoints: [FundsScanEndpoint] = []
    @State private var candidateCount = 0
    @State private var reads: [FundsScanRead] = []
    @State private var isScanning = false
    @State private var hasScanned = false
    @State private var errorMessage: String?

    var body: some View {
        NavigationStack {
            List {
                if let errorMessage {
                    Section { Text(errorMessage).foregroundStyle(.red) }
                }
                if scan != nil {
                    endpointsSection
                }
                if !reads.isEmpty {
                    Section(AppLocalization.string("Accounts")) {
                        ForEach(reads, id: \.candidate.address) { read in
                            readRow(read)
                        }
                    }
                }
            }
            .navigationTitle(AppLocalization.string("Find Used Accounts"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(AppLocalization.string("Done")) { dismiss() }
                }
            }
            .task { await prepare() }
        }
    }

    @ViewBuilder
    private var endpointsSection: some View {
        Section {
            ForEach(endpoints, id: \.endpoint) { endpoint in
                Text(endpoint.endpoint).font(.caption.monospaced()).textSelection(.enabled)
            }
            if endpoints.isEmpty {
                Text(AppLocalization.string("No endpoint is configured for this network.")).font(.caption)
                    .foregroundStyle(.secondary)
            }
            Button {
                Task { await runScan() }
            } label: {
                if isScanning {
                    HStack(spacing: SpectraLayout.Space.s) {
                        SpectraLoadingGlyph(size: 18, tint: .accentColor)
                        Text(AppLocalization.string("Scanning…"))
                    }
                } else {
                    Text(AppLocalization.string("Scan Accounts"))
                }
            }.disabled(isScanning || endpoints.isEmpty)
        } header: {
            Text(AppLocalization.format("Spectra will send %lld addresses to:", count: candidateCount, candidateCount))
        } footer: {
            Text(AppLocalization.string("Nothing is sent until you scan."))
        }
    }

    private func readRow(_ read: FundsScanRead) -> some View {
        Button {
            guard read.used else { return }
            if let version = read.candidate.tonWalletVersion {
                draft.tonWalletVersion = version
            } else if let profile = read.candidate.profile {
                draft.derivationProfile = profile
                draft.derivationAccount = read.candidate.account
                draft.customDerivationPath = ""
            } else {
                return
            }
            dismiss()
        } label: {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                HStack {
                    Text(read.candidate.profileTitle ?? "").font(.subheadline.weight(.semibold))
                    Spacer()
                    Text(status(read)).font(.caption.weight(.semibold))
                        .foregroundStyle(read.used ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                }
                Text(read.candidate.address).font(.caption.monospaced()).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
                if let error = read.error {
                    Text(userErrorMessage(error)).font(.caption).foregroundStyle(.red.opacity(0.9))
                }
            }
        }
        .buttonStyle(.plain)
        .disabled(!read.used)
    }

    private func status(_ read: FundsScanRead) -> String {
        if read.error != nil { return AppLocalization.string("Not read") }
        if read.funded, let balance = read.balance { return balance.amountDisplay }
        return AppLocalization.string(read.used ? "Used" : "Unused")
    }

    /// Derive the candidates and name the endpoints; nothing is sent.
    private func prepare() async {
        guard scan == nil, let chain = draft.chain else { return }
        let request = FundsFinderRequest(
            seedPhrase: draft.seedPhrase,
            passphrase: draft.overridePassphrase.isEmpty ? nil : draft.overridePassphrase)
        do {
            let service = try store.bridge.service()
            // Deriving every candidate is synchronous work; keep it off the
            // main actor.
            let begun = try await Task.detached { try service.beginFundsScan(request: request, chainId: chain) }.value
            candidateCount = begun.candidates().count
            endpoints = await begun.endpoints()
            scan = begun
        } catch {
            errorMessage = userErrorMessage(error)
        }
    }

    private func runScan() async {
        guard let chain = draft.chain, !isScanning else { return }
        // A scan session is consumed as it runs, so each scan is a new one.
        if hasScanned {
            scan = nil
            await prepare()
        }
        guard let scan, draft.chain == chain else { return }
        isScanning = true
        reads = []
        repeat {
            let batch = await scan.nextBatch()
            reads.append(contentsOf: batch.reads)
            if batch.complete { break }
        } while !Task.isCancelled
        hasScanned = true
        isScanning = false
    }
}
