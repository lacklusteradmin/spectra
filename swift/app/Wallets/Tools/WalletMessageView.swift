import SwiftUI

extension MessageScheme {
    var title: String {
        switch self {
        case .signedMessage: AppLocalization.string("Signed message")
        case .bip322: AppLocalization.string("BIP-322")
        case .personalSign: AppLocalization.string("EIP-191 personal_sign")
        case .tronSignedMessage: AppLocalization.string("TIP-191")
        case .solanaMessage: AppLocalization.string("Solana message")
        case .suiPersonalMessage: AppLocalization.string("Sui personal message")
        case .substrateBytes: AppLocalization.string("Substrate signRaw")
        case .stellarSignedMessage: AppLocalization.string("SEP-53")
        case .cardanoDataSignature: AppLocalization.string("CIP-8 data signature")
        case .kaspaPersonalMessage: AppLocalization.string("Kaspa personal message")
        case .moneroSignature: AppLocalization.string("Monero message signature")
        }
    }
}

/// Signing a message with a wallet's key to prove it holds its address, and
/// checking a signature against the address. Core signs and checks in the
/// network's scheme; a watched wallet can only check.
struct WalletMessageView: View {
    private enum Mode: Hashable { case sign, verify }

    let store: AppState
    let wallet: WalletView
    let canSign: Bool
    @State private var mode: Mode
    @State private var scheme: MessageScheme?
    @State private var message = ""
    @State private var signatureInput = ""
    @State private var password = ""
    @State private var signed: SignedMessage?
    @State private var verdict: Bool?
    @State private var error: String?
    @State private var isWorking = false
    @State private var didCopy = false

    init(store: AppState, wallet: WalletView, canSign: Bool) {
        self.store = store
        self.wallet = wallet
        self.canSign = canSign
        _mode = State(initialValue: canSign ? .sign : .verify)
    }

    private var address: String { wallet.address(on: wallet.chain) ?? "" }
    private var needsPassword: Bool { wallet.signing.requiresPassword }

    var body: some View {
        Form {
            if canSign {
                Picker(AppLocalization.string("Mode"), selection: $mode) {
                    Text(AppLocalization.string("Sign")).tag(Mode.sign)
                    Text(AppLocalization.string("Verify")).tag(Mode.verify)
                }.pickerStyle(.segmented).listRowBackground(Color.clear)
            }
            Section {
                Text(verbatim: address).font(.footnote.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                if let scheme {
                    Text(scheme.title).font(.caption).foregroundStyle(.secondary)
                }
            } header: {
                Text(AppLocalization.string("Address"))
            }
            Section(AppLocalization.string("Message")) {
                TextEditor(text: $message).frame(minHeight: 96)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
            }
            switch mode {
            case .sign: signSection
            case .verify: verifySection
            }
            if let error {
                WalletToolErrorSection(message: error)
            }
        }
        .navigationTitle((canSign ? WalletAction.signMessage : WalletAction.verifyMessage).title).navigationBarTitleDisplayMode(.inline)
        .task { scheme = try? await store.bridge.ready().walletMessageScheme(walletId: wallet.id) }
        .onChange(of: message) { _, _ in
            signed = nil
            verdict = nil
        }
        .onChange(of: signatureInput) { _, _ in verdict = nil }
        .onChange(of: mode) { _, _ in error = nil }
        .onDisappear { password = "" }
    }

    @ViewBuilder
    private var signSection: some View {
        if needsPassword {
            Section {
                SecureField(AppLocalization.string("Wallet Password"), text: $password)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().privacySensitive()
            }
        }
        Section {
            Button {
                Task { await sign() }
            } label: {
                HStack {
                    Text(AppLocalization.string("Sign Message"))
                    if isWorking { Spacer(); ProgressView() }
                }
            }.disabled(isWorking || message.isEmpty || (needsPassword && password.isEmpty))
        } footer: {
            Text(AppLocalization.string("Only plain text is signed. The signature proves this wallet holds the address; it authorises nothing."))
        }
        if let signed {
            Section(AppLocalization.string("Signature")) {
                Text(verbatim: signed.signature).font(.footnote.monospaced()).textSelection(.enabled)
                Button {
                    UIPasteboard.general.string = signed.signature
                    didCopy = true
                    spectraHaptic(.light)
                } label: {
                    Label(AppLocalization.string(didCopy ? "Copied" : "Copy"), systemImage: didCopy ? "checkmark" : "doc.on.doc")
                }
            }
        }
    }

    @ViewBuilder
    private var verifySection: some View {
        Section(AppLocalization.string("Signature")) {
            TextField(AppLocalization.string("Signature"), text: $signatureInput, axis: .vertical)
                .font(.footnote.monospaced()).textInputAutocapitalization(.never).autocorrectionDisabled()
            Button(AppLocalization.string("Verify")) {
                verdict = verifyMessage(
                    chain: wallet.chain, address: address, message: message, signature: signatureInput)
                spectraNotificationHaptic(verdict == true ? .success : .error)
            }.disabled(signatureInput.isEmpty)
        }
        if let verdict {
            Section {
                Label(
                    AppLocalization.string(
                        verdict ? "This address signed this message." : "This address did not sign this message."),
                    systemImage: verdict ? "checkmark.seal.fill" : "xmark.seal.fill"
                ).foregroundStyle(verdict ? Color.green : Color.red)
            }
        }
    }

    private func sign() async {
        isWorking = true
        defer { isWorking = false }
        error = nil
        didCopy = false
        if let failure = await store.authenticate(
            .send, reason: AppLocalization.format("Authenticate to sign a message with %@", wallet.name)) {
            error = failure
            return
        }
        do {
            signed = try await store.bridge.ready().signWalletMessage(
                walletId: wallet.id, message: message, password: needsPassword ? password : nil)
            password = ""
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}
