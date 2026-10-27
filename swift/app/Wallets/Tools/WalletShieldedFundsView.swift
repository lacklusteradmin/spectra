import SwiftUI

/// A Zcash wallet's shielded funds, as core keeps them: the scan from the
/// wallet's restore height through a lightwalletd server, the unified address
/// it receives at, moving its transparent funds into the shielded pool, and
/// paying from it. Core builds each transaction; this page signs and
/// broadcasts it through the same stages as any send.
struct WalletShieldedFundsView: View {
    let store: AppState
    let wallet: WalletView
    @State private var status: ZcashShieldedStatus?
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
        .navigationTitle(WalletAction.shieldedFunds.title).navigationBarTitleDisplayMode(.inline)
        .task(id: wallet.id) { await load() }
        .task(id: scan.requestId) {
            guard let request = scan.requestId else { return }
            await run(request)
        }
        .onDisappear { scan.cancel() }
        .sheet(item: $flow, onDismiss: { Task { await load() } }) { flow in
            NavigationStack { ShieldedTransactionView(store: store, wallet: wallet, flow: flow) }
        }
    }

    private func scanSection(_ status: ZcashShieldedStatus) -> some View {
        Section {
            LabeledContent(AppLocalization.string("Restore Height")) {
                Text(verbatim: "\(status.restoreHeight)").monospacedDigit()
            }
            if status.ready {
                LabeledContent(AppLocalization.string("Scanned")) {
                    Text(verbatim: "\(status.scannedHeight) / \(status.chainTipHeight)").monospacedDigit()
                }
            }
            if scan.isRunning {
                ProgressView(value: Double(status.progressPermille), total: 1000)
                if status.unreadTransactions > 0 {
                    Text(AppLocalization.format("Reading %@ transactions found by the scan.",
                                                String(status.unreadTransactions)))
                        .font(.caption).foregroundStyle(.secondary)
                }
                Button(AppLocalization.string("Cancel")) { scan.cancel() }
            } else {
                // The first batch makes the shielded account from the seed.
                if !status.ready && wallet.signing.requiresPassword {
                    SecureField(AppLocalization.string("Wallet Password"), text: $scan.password)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().privacySensitive()
                }
                Button(status.complete
                       ? AppLocalization.string("Scan Again")
                       : AppLocalization.string("Scan for Shielded Funds")) { scan.begin() }
                    .disabled(!status.ready && wallet.signing.requiresPassword && scan.password.isEmpty)
            }
            if let failure = scan.error {
                Text(failure).font(.caption).foregroundStyle(.red)
            }
        } footer: {
            Text(AppLocalization.string(
                "This device reads every block from the restore height on and finds what was sent to this wallet; the keys never leave it. The server sees which transactions are fetched in full, and the transparent address."
            ))
        }
    }

    private func balanceSection(_ status: ZcashShieldedStatus) -> some View {
        let symbol = wallet.chain.gasTokenSymbol
        return Section {
            LabeledContent(AppLocalization.string("Spendable"), value: "\(status.spendable) \(symbol)")
            if status.pending != "0" {
                LabeledContent(AppLocalization.string("Pending"), value: "\(status.pending) \(symbol)")
            }
            if status.shieldable != "0" {
                LabeledContent(AppLocalization.string("Transparent, to Shield"),
                               value: "\(status.shieldable) \(symbol)")
                Button(AppLocalization.string("Shield Transparent Funds")) { flow = .moveIn }
                    .disabled(scan.isRunning)
            }
            Button(AppLocalization.string("Send Shielded Funds")) { flow = .send }
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
            Text(AppLocalization.string("Shielded Address"))
        } footer: {
            Text(AppLocalization.string(
                "A unified address with Orchard and Sapling receivers. Funds sent here are shielded; the wallet's transparent address stays unlinked to it."
            ))
        }
    }

    private func load() async {
        do {
            status = try await store.bridge.ready().zcashShieldedStatus(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }

    /// Scan one core batch at a time until the scan is complete. Core keeps
    /// every batch, so a cancelled scan resumes where it stopped; the first
    /// batch makes the shielded account from the seed, and only it asks for
    /// the password.
    private func run(_ request: UUID) async {
        defer { scan.finish(request) }
        scan.error = nil
        let makesAccount = !(status?.ready ?? false)
        if makesAccount, let failure = await store.authenticate(
            .send, reason: AppLocalization.string("Authorize the shielded scan")) {
            scan.error = failure
            return
        }
        let password = makesAccount && !scan.password.isEmpty ? scan.password : nil
        do {
            while scan.isCurrent(request) {
                let next = try await store.bridge.ready().syncZcashShielded(walletId: wallet.id, password: password)
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


/// Shielding, or a payment's recipient, amount and memo; then the
/// transaction core builds, signed and broadcast through the stages to the
/// network's lightwalletd servers.
private struct ShieldedTransactionView: View {
    let store: AppState
    let wallet: WalletView
    let flow: PrivateFundsFlow
    @State private var recipient = ""
    @State private var amount = ""
    @State private var memo = ""

    /// Shielding asks for nothing and is built as the sheet opens.
    private var formIsComplete: Bool? {
        guard flow == .send else { return nil }
        return !recipient.trimmingCharacters(in: .whitespaces).isEmpty
            && !amount.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        WalletOperationSheet(
            store: store, wallet: wallet,
            title: flow == .moveIn
                ? AppLocalization.string("Shield Transparent Funds") : AppLocalization.string("Send Shielded Funds"),
            authenticationReason: AppLocalization.format("Authenticate to send shielded funds from %@", wallet.name),
            submittedText: AppLocalization.string("Submitted. The funds move once it confirms; scan again to see them."),
            formIsComplete: formIsComplete,
            endpoints: { try await store.bridge.ready().zcashShieldedBroadcastEndpoints(chain: $0) },
            build: build
        ) {
            Section {
                AddressEntryRow(
                    title: AppLocalization.string("Destination Address"), text: $recipient, chain: wallet.chain,
                    contacts: store.addressBook.entries.filter { $0.chainId == wallet.chain })
                TextField(AppLocalization.string("Amount"), text: $amount)
                    .keyboardType(.decimalPad).monospacedDigit()
                TextField(AppLocalization.string("Memo (Optional)"), text: $memo, axis: .vertical)
                    .lineLimit(1...4)
            } footer: {
                Text(AppLocalization.string(
                    "A unified or Sapling address receives privately; a transparent one is public. A memo reaches only a shielded recipient, and only the recipient reads it."
                ))
            }
        } summary: { artifact in
            switch artifact?.operation {
            case let .shieldTransparent(amount):
                Section {
                    LabeledContent(
                        AppLocalization.string("Shielded"),
                        value: store.amounts.formattedNativeAmount(amount, chain: wallet.chain))
                } footer: {
                    Text(AppLocalization.string(
                        "Every transparent output of this wallet moves into its own shielded pool. The transaction shows the transparent address it came from."
                    ))
                }
            case let .shieldedPayment(memo):
                if let artifact {
                    PrivatePaymentSummary(store: store, wallet: wallet, artifact: artifact, memo: memo)
                }
            default:
                EmptyView()
            }
        }
    }

    private func build() async throws -> SendArtifact {
        let recipient = recipient.trimmingCharacters(in: .whitespacesAndNewlines)
        let amount = AmountPresentation.canonicalDecimalInput(amount)
        let memo = memo.trimmingCharacters(in: .whitespacesAndNewlines)
        switch flow {
        case .moveIn:
            return try await store.bridge.ready().buildZcashShielding(walletId: wallet.id)
        case .send:
            return try await store.bridge.ready().buildZcashShieldedSend(
                walletId: wallet.id, recipient: recipient, amount: amount, memo: memo.isEmpty ? nil : memo)
        }
    }
}

/// A payment out of a wallet's private funds, as both private-funds sheets
/// review it: the recipient, the amount, and the memo it carries.
struct PrivatePaymentSummary: View {
    let store: AppState
    let wallet: WalletView
    let artifact: SendArtifact
    let memo: String?

    var body: some View {
        Section {
            LabeledContent(AppLocalization.string("To")) {
                Text(verbatim: artifact.recipient).font(.caption.monospaced()).lineLimit(2)
                    .truncationMode(.middle)
            }
            LabeledContent(
                AppLocalization.string("Amount"),
                value: store.amounts.formattedNativeAmount(artifact.amount, chain: wallet.chain))
            if let memo {
                LabeledContent(AppLocalization.string("Memo")) { Text(verbatim: memo).lineLimit(4) }
            }
        }
    }
}
