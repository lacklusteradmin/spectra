import SwiftUI

/// A Litecoin wallet's MWEB funds, as core keeps them: the scan of a
/// Litecoin node's MWEB outputs, the stealth address the wallet receives at,
/// moving its transparent LTC into MWEB, and paying from MWEB. Core builds
/// each transaction; this page signs and broadcasts it through the same
/// stages as any send.
struct WalletMwebFundsView: View {
    let store: AppState
    let wallet: WalletView
    @State private var status: LitecoinMwebStatus?
    @State private var error: String?
    @State private var scan = ScanRun()
    @State private var flow: PrivateFundsFlow?

    var body: some View {
        List {
            if let status {
                scanSection(status)
                if status.ready {
                    balanceSection(status)
                }
                if let address = status.address {
                    addressSection(address)
                }
            } else if let error {
                WalletToolErrorSection(message: error)
            } else {
                WalletToolLoadingSection()
            }
        }
        .navigationTitle(WalletAction.mwebFunds.title).navigationBarTitleDisplayMode(.inline)
        .task(id: wallet.id) { await load() }
        .task(id: scan.requestId) {
            guard let request = scan.requestId else { return }
            await run(request)
        }
        .onDisappear { scan.cancel() }
        .sheet(item: $flow, onDismiss: { Task { await load() } }) { flow in
            NavigationStack { MwebTransactionView(store: store, wallet: wallet, flow: flow) }
        }
    }

    private func scanSection(_ status: LitecoinMwebStatus) -> some View {
        Section {
            if status.ready {
                LabeledContent(AppLocalization.string("Scanned")) {
                    Text(verbatim: "\(status.scannedHeight)").monospacedDigit()
                }
            }
            if scan.isRunning {
                ProgressView(value: Double(status.progressPermille), total: 1000)
                Button(AppLocalization.string("Cancel")) { scan.cancel() }
            } else {
                // The first batch derives the MWEB keys from the seed.
                if !status.ready && wallet.signing.requiresPassword {
                    SecureField(AppLocalization.string("Wallet Password"), text: $scan.password)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().privacySensitive()
                }
                Button(status.complete
                       ? AppLocalization.string("Scan Again")
                       : AppLocalization.string("Scan for MWEB Funds")) { scan.begin() }
                    .disabled(!status.ready && wallet.signing.requiresPassword && scan.password.isEmpty)
            }
            if let failure = scan.error {
                Text(failure).font(.caption).foregroundStyle(.red)
            }
        } footer: {
            Text(AppLocalization.string(
                "This device reads the unspent MWEB outputs a Litecoin node holds and finds the ones sent to this wallet; the keys never leave it. The node learns only that a light client asked."
            ))
        }
    }

    private func balanceSection(_ status: LitecoinMwebStatus) -> some View {
        let symbol = wallet.chain.gasTokenSymbol
        return Section {
            LabeledContent(AppLocalization.string("Spendable"), value: "\(status.spendable) \(symbol)")
            if status.pending != "0" {
                LabeledContent(AppLocalization.string("Pending"), value: "\(status.pending) \(symbol)")
            }
            Button(AppLocalization.string("Move LTC into MWEB")) { flow = .moveIn }
                .disabled(scan.isRunning)
            Button(AppLocalization.string("Send from MWEB")) { flow = .send }
                .disabled(scan.isRunning || status.spendable == "0")
        } footer: {
            if !status.complete {
                Text(AppLocalization.string("The scan is not finished; funds it has not reached are not counted."))
            }
        }
    }

    private func addressSection(_ address: String) -> some View {
        Section {
            QRCodeImage(address: address).frame(maxWidth: 220).frame(maxWidth: .infinity)
            Text(verbatim: address).font(.caption.monospaced()).textSelection(.enabled)
            Button(AppLocalization.string("Copy Address")) { UIPasteboard.general.string = address }
        } header: {
            Text(AppLocalization.string("MWEB Address"))
        } footer: {
            Text(AppLocalization.string(
                "A stealth address: each payment to it makes an output only this wallet recognizes, unlinked to its transparent address."
            ))
        }
    }

    private func load() async {
        do {
            status = try await store.bridge.ready().litecoinMwebStatus(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }

    /// Scan one core batch at a time until the scan is complete; the first
    /// batch derives the MWEB keys from the seed, and only it asks for the
    /// password.
    private func run(_ request: UUID) async {
        defer { scan.finish(request) }
        scan.error = nil
        let derivesKeys = !(status?.ready ?? false)
        if derivesKeys, let failure = await store.authenticate(
            .send, reason: AppLocalization.string("Authorize the MWEB scan")) {
            scan.error = failure
            return
        }
        let password = derivesKeys && !scan.password.isEmpty ? scan.password : nil
        do {
            while scan.isCurrent(request) {
                let next = try await store.bridge.ready().syncLitecoinMweb(walletId: wallet.id, password: password)
                guard scan.isCurrent(request) else { return }
                status = next
                if next.complete { break }
            }
        } catch is CancellationError {
            return
        } catch {
            if scan.isCurrent(request) { scan.error = userErrorMessage(error) }
            return
        }
        await store.refreshBalances()
    }
}

/// A peg-in's amount, or a payment's recipient and amount; then the
/// transaction core builds, signed and broadcast through the stages.
private struct MwebTransactionView: View {
    let store: AppState
    let wallet: WalletView
    let flow: PrivateFundsFlow
    @Environment(\.dismiss) private var dismiss
    @State private var session = SendSession()
    @State private var recipient = ""
    @State private var amount = ""

    private var symbol: String { wallet.chain.gasTokenSymbol }

    var body: some View {
        Form {
            if let artifact = session.artifact {
                reviewSection(artifact)
            } else {
                Section {
                    if flow == .send {
                        AddressEntryRow(
                            title: AppLocalization.string("Destination Address"), text: $recipient, chain: wallet.chain,
                            contacts: store.addressBook.entries.filter { $0.chainId == wallet.chain })
                    }
                    TextField(AppLocalization.string("Amount"), text: $amount)
                        .keyboardType(.decimalPad).monospacedDigit()
                } footer: {
                    Text(flow == .send
                         ? AppLocalization.string(
                            "An MWEB address is paid privately. Any other Litecoin address is paid by a peg-out, which shows the amount and the recipient on the chain.")
                         : AppLocalization.string(
                            "Transparent LTC moves into this wallet's MWEB funds. The transaction shows the transparent address it comes from."))
                }
                Section {
                    Button(AppLocalization.string("Review")) { Task { await review() } }
                        .disabled((flow == .send && recipient.trimmingCharacters(in: .whitespaces).isEmpty)
                                  || amount.trimmingCharacters(in: .whitespaces).isEmpty || session.isBusy)
                }
            }
            SendArtifactStageSections(
                store: store, wallet: wallet, session: session,
                authenticationReason: AppLocalization.format("Authenticate to send MWEB funds from %@", wallet.name),
                submittedText: AppLocalization.string("Submitted. The funds move once it confirms; scan again to see them."))
        }
        .navigationTitle(flow == .moveIn
                         ? AppLocalization.string("Move LTC into MWEB")
                         : AppLocalization.string("Send from MWEB"))
        .navigationBarTitleDisplayMode(.inline)
        .sendSheetDismissal(session: session) { dismiss() }
    }

    @ViewBuilder
    private func reviewSection(_ artifact: SendArtifact) -> some View {
        switch artifact.operation {
        case let .shieldTransparent(amount, networkFee):
            Section {
                LabeledContent(AppLocalization.string("Into MWEB"), value: "\(amount) \(symbol)")
                LabeledContent(AppLocalization.string("Network Fee"), value: "\(networkFee) \(symbol)")
            }
        case let .shieldedPayment(_, networkFee):
            Section {
                LabeledContent(AppLocalization.string("To")) {
                    Text(verbatim: artifact.recipient).font(.caption.monospaced()).lineLimit(2)
                        .truncationMode(.middle)
                }
                LabeledContent(AppLocalization.string("Amount"), value: "\(artifact.amount) \(symbol)")
                LabeledContent(AppLocalization.string("Network Fee"), value: "\(networkFee) \(symbol)")
            }
            let warnings = highRiskSendMessages(artifact.review.warnings)
            if !warnings.isEmpty {
                Section {
                    ForEach(warnings, id: \.self) { Text($0).foregroundStyle(Color.spectraWarning) }
                }
            }
        default:
            EmptyView()
        }
    }

    private func review() async {
        let recipient = recipient.trimmingCharacters(in: .whitespacesAndNewlines)
        let amount = amount.trimmingCharacters(in: .whitespaces)
        await session.load(
            operation: .build,
            prepare: {
                switch flow {
                case .moveIn:
                    try await store.bridge.ready().buildLitecoinMwebPegin(walletId: wallet.id, amount: amount)
                case .send:
                    try await store.bridge.ready().buildLitecoinMwebSend(
                        walletId: wallet.id, recipient: recipient, amount: amount)
                }
            },
            endpoints: { try await store.bridge.ready().sendEndpoints(chain: $0) })
    }
}
