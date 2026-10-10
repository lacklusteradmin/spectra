import SwiftUI

/// A multisig account as core reads its policy, and its sessions: build a
/// spend, read one another signer sent, and open one to sign it, hand it on,
/// or submit it once enough signers have signed. Core decides who may sign
/// and how a session reaches the network; this view only asks.
struct WalletMultisigView: View {
    let store: AppState
    let wallet: WalletView
    @State private var account: MultisigAccount?
    @State private var sessions: [MultisigSession]?
    @State private var error: String?
    @State private var recipient = ""
    @State private var amount = ""
    @State private var memoKind: PaymentMemoKind?
    @State private var memo = ""
    @State private var pasted = ""
    @State private var isWorking = false
    @State private var opened: MultisigSession?

    var body: some View {
        List {
            if let account, let sessions {
                MultisigPolicySections(store: store, account: account)
                Section {
                    AddressEntryRow(
                        title: AppLocalization.string("Recipient"), text: $recipient, chain: wallet.chain,
                        contacts: store.addressBook.entries.filter { $0.chainId == wallet.chain })
                    TextField(AppLocalization.format("Amount (%@)", wallet.chain.gasTokenSymbol), text: $amount)
                        .keyboardType(.decimalPad)
                    if case let kinds = paymentMemoKinds(chain: wallet.chain), !kinds.isEmpty {
                        SendPaymentMemoField(kinds: kinds, kind: $memoKind, text: $memo)
                    }
                    Button(AppLocalization.string("Build Transaction")) { Task { await create() } }
                        .disabled(isWorking || recipient.isEmpty || amount.isEmpty)
                } header: {
                    Text(AppLocalization.string("Spend"))
                }
                Section {
                    TextField(AppLocalization.string("Transaction from a signer"), text: $pasted, axis: .vertical)
                        .lineLimit(3...6)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().font(.caption.monospaced())
                    Button(AppLocalization.string("Read Transaction")) { Task { await read() } }
                        .disabled(isWorking || pasted.isEmpty)
                } header: {
                    Text(AppLocalization.string("From a Signer"))
                } footer: {
                    Text(AppLocalization.string("A transaction already here joins it, its signatures with the others."))
                }
                if !sessions.isEmpty {
                    Section(AppLocalization.string("Transactions")) {
                        ForEach(sessions, id: \.id) { session in
                            Button { opened = session } label: { row(session) }
                        }
                    }
                }
                if let error { WalletToolErrorSection(message: error) }
            } else if let error {
                WalletToolErrorSection(message: error)
            } else {
                WalletToolLoadingSection()
            }
        }
        .navigationTitle(WalletAction.multisig.title).navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .navigationDestination(item: $opened) { session in
            if let account {
                MultisigSessionView(store: store, wallet: wallet, account: account, session: session) { await load() }
            }
        }
    }

    private func row(_ session: MultisigSession) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Text(verbatim: session.transactionId).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
            Text(session.submittedTxid != nil
                 ? AppLocalization.string("Submitted")
                 : AppLocalization.format("%lld of %lld signed", Int64(session.signedWeight), Int64(session.threshold)))
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    private func load() async {
        do {
            let service = try await store.bridge.ready()
            account = try await service.multisigAccount(walletId: wallet.id)
            sessions = try await service.multisigSessions(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }

    private func create() async {
        await work {
            let memo = memo.trimmingCharacters(in: .whitespacesAndNewlines)
            let session = try await store.bridge.ready().createMultisig(
                walletId: wallet.id,
                spend: MultisigSpend(
                    toAddress: recipient, amount: AmountPresentation.canonicalDecimalInput(amount),
                    feeRate: nil, expiresInSecs: nil,
                    memo: memo.isEmpty ? nil : memoKind.map { PaymentMemo(kind: $0, value: memo) }))
            recipient = ""
            amount = ""
            self.memo = ""
            opened = session
        }
    }

    private func read() async {
        await work {
            let session = try await store.bridge.ready().importMultisig(walletId: wallet.id, data: pasted)
            pasted = ""
            opened = session
        }
    }

    private func work(_ body: () async throws -> Void) async {
        isWorking = true
        defer { isWorking = false }
        do {
            try await body()
            error = nil
            await load()
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}

/// The account's address, its signers and thresholds, and what else can
/// move or block its funds.
private struct MultisigPolicySections: View {
    let store: AppState
    let account: MultisigAccount

    var body: some View {
        Section {
            Text(verbatim: account.address).font(.callout.monospaced()).textSelection(.enabled)
        } header: {
            Text(AppLocalization.string("Account"))
        }
        ForEach(Array(account.permissions.enumerated()), id: \.offset) { _, permission in
            Section {
                ForEach(Array(permission.signers.enumerated()), id: \.offset) { _, signer in
                    MultisigSignerRow(store: store, signer: signer)
                }
                if !permission.covers.isEmpty {
                    Text(verbatim: permission.covers.joined(separator: ", ")).font(.caption).foregroundStyle(.secondary)
                }
            } header: {
                Text(AppLocalization.format("%@: %lld to sign", permission.name, Int64(permission.threshold)))
            }
        }
        if !account.warnings.isEmpty {
            Section {
                ForEach(Array(account.warnings.enumerated()), id: \.offset) { _, warning in
                    Label(warning.localizedText, systemImage: "exclamationmark.triangle").font(.callout)
                }
            }
        }
    }
}

/// One signer: its key or address, its weight where weights differ, and the
/// wallet on this device that holds it.
private struct MultisigSignerRow: View {
    let store: AppState
    let signer: MultisigSigner

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
            HStack {
                if signer.signed { Image(systemName: "checkmark.circle.fill").foregroundStyle(.green) }
                Text(verbatim: signer.signer).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
                if signer.weight != 1 {
                    Spacer()
                    Text(AppLocalization.format("Weight %lld", Int64(signer.weight))).font(.caption).foregroundStyle(.secondary)
                }
            }
            if let name = signer.walletId.flatMap({ id in store.wallets.first { $0.id == id }?.name }) {
                Text(AppLocalization.format("On this device: %@", name)).font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

/// One session: what it spends and pays, who has signed, and what can be
/// done with it now.
private struct MultisigSessionView: View {
    let store: AppState
    let wallet: WalletView
    let account: MultisigAccount
    @State var session: MultisigSession
    let changed: () async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var signerId: String?
    @State private var password = ""
    @State private var error: String?
    @State private var isWorking = false

    private var signers: [WalletView] {
        account.signerWalletIds.compactMap { id in store.wallets.first { $0.id == id } }
    }

    private var signer: WalletView? {
        signers.first { $0.id == signerId } ?? signers.first
    }

    private func amount(_ smallestUnit: String) -> String {
        "\(formatNativeAmount(chain: session.chain, smallestUnit: smallestUnit).map(AmountPresentation.localizedDecimal) ?? smallestUnit) \(session.chain.gasTokenSymbol)"
    }

    var body: some View {
        Form {
            Section(AppLocalization.string("Transaction")) {
                Text(verbatim: session.transactionId).font(.caption.monospaced()).textSelection(.enabled)
                if let sequence = session.sequence {
                    LabeledContent(AppLocalization.string("Sequence"), value: sequence)
                }
                if let expiresAt = session.expiresAt {
                    LabeledContent(AppLocalization.string("Expires")) {
                        Text(Date(timeIntervalSince1970: TimeInterval(expiresAt)), style: .date)
                    }
                }
                if let height = session.expiresAtHeight {
                    LabeledContent(AppLocalization.string("Expires After"), value: String(height))
                }
            }
            if !session.inputs.isEmpty {
                Section(AppLocalization.string("Spends")) {
                    ForEach(session.inputs, id: \.outpoint) { input in
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                            LabeledContent(amount(input.value)) {
                                Text(AppLocalization.format("%lld of %lld signed", Int64(input.signatures), Int64(session.threshold)))
                            }
                            Text(verbatim: input.address).font(.caption2.monospaced()).foregroundStyle(.secondary)
                                .lineLimit(1).truncationMode(.middle)
                        }
                    }
                }
            }
            Section(AppLocalization.string("Pays")) {
                ForEach(Array(session.outputs.enumerated()), id: \.offset) { _, output in
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                        LabeledContent(output.asset == nil ? amount(output.value) : output.value) {
                            if output.isChange { Text(AppLocalization.string("Change")) }
                        }
                        Text(verbatim: output.address).font(.caption2.monospaced()).foregroundStyle(.secondary)
                            .lineLimit(1).truncationMode(.middle)
                        if let asset = output.asset {
                            Text(verbatim: asset).font(.caption2.monospaced()).foregroundStyle(.secondary)
                        }
                        if let memo = output.memo {
                            Text(verbatim: memo).font(.caption.monospaced())
                        }
                        if let data = output.data {
                            Text(verbatim: data).font(.caption2.monospaced()).foregroundStyle(.secondary).lineLimit(2)
                        }
                    }
                }
                if session.fee != "0" {
                    LabeledContent(AppLocalization.string("Network Fee"), value: amount(session.fee))
                }
            }
            Section {
                ForEach(Array(session.signers.enumerated()), id: \.offset) { _, signer in
                    MultisigSignerRow(store: store, signer: signer)
                }
            } header: {
                Text(AppLocalization.format("%lld of %lld signed", Int64(session.signedWeight), Int64(session.threshold)))
            }
            actions
            if let error { WalletToolErrorSection(message: error) }
        }
        .navigationTitle(AppLocalization.string("Multisig Transaction")).navigationBarTitleDisplayMode(.inline)
    }

    @ViewBuilder private var actions: some View {
        Section {
            if let txid = session.submittedTxid {
                LabeledContent(AppLocalization.string("Submitted"), value: txid).font(.caption.monospaced())
            } else {
                if signers.count > 1 {
                    Picker(AppLocalization.string("Signer"), selection: Binding(get: { signer?.id }, set: { signerId = $0 })) {
                        ForEach(signers, id: \.id) { Text($0.name).tag(Optional($0.id)) }
                    }
                }
                if signer?.signing.requiresPassword == true {
                    SecureField(AppLocalization.string("Wallet Password"), text: $password)
                }
                if signer != nil {
                    Button(AppLocalization.string(account.submission == .byApproval ? "Approve" : "Sign")) {
                        Task { await sign() }
                    }
                    .disabled(isWorking)
                }
                if session.complete {
                    switch account.submission {
                    case .asIs:
                        Button(AppLocalization.string("Submit")) { Task { await submit(executor: nil) } }.disabled(isWorking)
                    case .byExecutor:
                        if let signer {
                            Button(AppLocalization.format("Execute as %@", signer.name)) {
                                Task { await submit(executor: signer) }
                            }
                            .disabled(isWorking)
                        }
                    case .byApproval:
                        EmptyView()
                    }
                }
            }
            ShareLink(item: session.data) {
                Label(AppLocalization.string("Share Transaction"), systemImage: "square.and.arrow.up")
            }
            if session.submittedTxid == nil {
                Button(AppLocalization.string("Discard"), role: .destructive) { Task { await discard() } }
                    .disabled(isWorking)
            }
        } footer: {
            Text(footer)
        }
    }

    private var footer: String {
        if signers.isEmpty && session.submittedTxid == nil {
            return AppLocalization.string("No wallet on this device can sign for this account: hand the transaction to a signer.")
        }
        switch account.submission {
        case .asIs:
            return AppLocalization.string("Hand the transaction to the next signer; once enough have signed, any of them can submit it.")
        case .byExecutor:
            return AppLocalization.string("Hand the transaction to the next owner; once enough have signed, one of them executes it and pays its fee.")
        case .byApproval:
            return AppLocalization.string("Each signer's approval is a transaction of its own; the one that meets the threshold executes the transfer.")
        }
    }

    private func sign() async {
        guard let signer else { return }
        if let failure = await store.authenticate(.send, reason: AppLocalization.format("Authenticate to sign from %@", signer.name)) {
            error = failure
            return
        }
        await work {
            session = try await store.bridge.ready().signMultisig(
                sessionId: session.id, reviewDigest: session.reviewDigest,
                signerWalletId: signer.id, password: password.isEmpty ? nil : password)
            password = ""
        }
    }

    private func submit(executor: WalletView?) async {
        if let executor,
           let failure = await store.authenticate(.send, reason: AppLocalization.format("Authenticate to sign from %@", executor.name)) {
            error = failure
            return
        }
        await work {
            session = try await store.bridge.ready().submitMultisig(
                sessionId: session.id, executorWalletId: executor?.id,
                password: executor == nil || password.isEmpty ? nil : password)
            password = ""
        }
    }

    private func discard() async {
        await work {
            try await store.bridge.ready().discardMultisig(sessionId: session.id)
            dismiss()
        }
    }

    private func work(_ body: () async throws -> Void) async {
        isWorking = true
        defer { isWorking = false }
        do {
            try await body()
            error = nil
            await changed()
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}
