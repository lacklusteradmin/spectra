import SwiftUI

/// A multisig wallet's PSBTs, each as core reviews it against the wallet's
/// policy: create one, read one a cosigner sent, and open one to sign it,
/// hand it on, or broadcast it once enough cosigners have signed.
struct WalletPsbtView: View {
    let store: AppState
    let wallet: WalletView
    @State private var sessions: [PsbtSession]?
    @State private var error: String?
    @State private var recipient = ""
    @State private var amount = ""
    @State private var pasted = ""
    @State private var isWorking = false
    @State private var opened: PsbtSession?

    var body: some View {
        List {
            if let sessions {
                Section {
                    TextField(AppLocalization.string("Recipient"), text: $recipient)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().font(.callout.monospaced())
                    TextField(AppLocalization.format("Amount (%@)", wallet.chain.gasTokenSymbol), text: $amount)
                        .keyboardType(.decimalPad)
                    Button(AppLocalization.string("Create PSBT")) { Task { await create() } }
                        .disabled(isWorking || recipient.isEmpty || amount.isEmpty)
                } header: {
                    Text(AppLocalization.string("Spend"))
                } footer: {
                    Text(AppLocalization.string("The PSBT spends this wallet's confirmed coins, largest first, its change to the wallet's next change address."))
                }
                Section {
                    TextField(AppLocalization.string("PSBT (base64)"), text: $pasted, axis: .vertical).lineLimit(3...6)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().font(.caption.monospaced())
                    Button(AppLocalization.string("Read PSBT")) { Task { await read() } }
                        .disabled(isWorking || pasted.isEmpty)
                } header: {
                    Text(AppLocalization.string("From a Cosigner"))
                } footer: {
                    Text(AppLocalization.string("A PSBT of a transaction already here joins it, its signatures with the others."))
                }
                if !sessions.isEmpty {
                    Section(AppLocalization.string("Transactions")) {
                        ForEach(sessions, id: \.id) { session in
                            Button { opened = session } label: { row(session) }
                        }
                    }
                }
                if let error { Section { Text(error).foregroundStyle(.red) } }
            } else if let error {
                Section { Text(error).foregroundStyle(.red) }
            } else {
                Section { ProgressView().frame(maxWidth: .infinity) }
            }
        }
        .navigationTitle(WalletAction.psbts.title).navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .navigationDestination(item: $opened) { session in
            PsbtSessionView(store: store, wallet: wallet, session: session) { await load() }
        }
    }

    private func row(_ session: PsbtSession) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Text(verbatim: session.txid).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
            Text(session.broadcastTxid != nil
                 ? AppLocalization.string("Broadcast")
                 : AppLocalization.format("%lld of %lld signed", Int64(session.signedBy.count), Int64(session.threshold)))
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    private func load() async {
        do {
            sessions = try await store.bridge.ready().psbtSessions(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }

    private func create() async {
        await work {
            let session = try await store.bridge.ready().createPsbt(
                walletId: wallet.id, toAddress: recipient, amount: amount, feeRateSvb: nil)
            recipient = ""
            amount = ""
            opened = session
        }
    }

    private func read() async {
        await work {
            let session = try await store.bridge.ready().importPsbt(walletId: wallet.id, psbtBase64: pasted)
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

/// One PSBT: what it spends and pays, who has signed, and what can be done
/// with it now.
private struct PsbtSessionView: View {
    let store: AppState
    let wallet: WalletView
    @State var session: PsbtSession
    let changed: () async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var password = ""
    @State private var error: String?
    @State private var isWorking = false

    var body: some View {
        Form {
            Section(AppLocalization.string("Spends")) {
                ForEach(session.inputs, id: \.outpoint) { input in
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                        LabeledContent(AppLocalization.format("%llu sat", input.valueSat)) {
                            Text(AppLocalization.format("%lld of %lld signed", Int64(input.signatures), Int64(session.threshold)))
                        }
                        Text(verbatim: input.address).font(.caption2.monospaced()).foregroundStyle(.secondary)
                            .lineLimit(1).truncationMode(.middle)
                    }
                }
            }
            Section(AppLocalization.string("Pays")) {
                ForEach(Array(session.outputs.enumerated()), id: \.offset) { _, output in
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                        LabeledContent(AppLocalization.format("%llu sat", output.valueSat)) {
                            if output.isChange { Text(AppLocalization.string("Change")) }
                        }
                        Text(verbatim: output.address).font(.caption2.monospaced()).foregroundStyle(.secondary)
                            .lineLimit(1).truncationMode(.middle)
                    }
                }
                LabeledContent(AppLocalization.string("Network Fee"), value: AppLocalization.format("%llu sat", session.feeSat))
            }
            Section(AppLocalization.string("Signed By")) {
                if session.signedBy.isEmpty {
                    Text(AppLocalization.string("No cosigner yet.")).foregroundStyle(.secondary)
                }
                ForEach(session.signedBy, id: \.self) { Text(verbatim: $0).font(.callout.monospaced()) }
            }
            Section {
                if let txid = session.broadcastTxid {
                    LabeledContent(AppLocalization.string("Broadcast"), value: txid).font(.caption.monospaced())
                } else {
                    if !wallet.signing.isWatchOnly {
                        if wallet.signing.requiresPassword {
                            SecureField(AppLocalization.string("Wallet Password"), text: $password)
                        }
                        Button(AppLocalization.string("Sign")) { Task { await sign() } }.disabled(isWorking)
                    }
                    if session.complete {
                        Button(AppLocalization.string("Broadcast")) { Task { await broadcast() } }.disabled(isWorking)
                    }
                }
                ShareLink(item: session.psbt) { Label(AppLocalization.string("Share PSBT"), systemImage: "square.and.arrow.up") }
                if session.broadcastTxid == nil {
                    Button(AppLocalization.string("Discard"), role: .destructive) { Task { await discard() } }
                        .disabled(isWorking)
                }
            } footer: {
                Text(AppLocalization.string("Hand the PSBT to the next cosigner; once enough have signed, any of them can broadcast it."))
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .navigationTitle(AppLocalization.string("PSBT")).navigationBarTitleDisplayMode(.inline)
    }

    private func sign() async {
        if let failure = await store.authenticate(.send, reason: AppLocalization.format("Authenticate to sign from %@", wallet.name)) {
            error = failure
            return
        }
        await work {
            session = try await store.bridge.ready().signPsbt(
                sessionId: session.id, reviewDigest: session.reviewDigest,
                password: password.isEmpty ? nil : password)
            password = ""
        }
    }

    private func broadcast() async {
        await work { session = try await store.bridge.ready().broadcastPsbt(sessionId: session.id) }
    }

    private func discard() async {
        await work {
            try await store.bridge.ready().discardPsbt(sessionId: session.id)
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
