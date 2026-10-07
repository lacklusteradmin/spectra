import SwiftUI

/// The password, signing and broadcast sections of a page that reviews one
/// core-built transaction itself, such as an approval revocation or an
/// account closing. Core keeps the artifact; `session` adopts its answers.
struct SendArtifactStageSections: View {
    let store: AppState
    let wallet: WalletView
    let session: SendSession
    /// Why the device asks to authenticate before signing.
    let authenticationReason: String
    /// What the page says once the transaction is submitted.
    let submittedText: String
    /// Whether the page's own confirmation lets the transaction be signed.
    var canSign = true
    @State private var password = ""

    private var needsPassword: Bool { wallet.signing.requiresPassword }

    var body: some View {
        if let artifact = session.artifact {
            switch artifact.stage {
            case .prepared:
                if needsPassword {
                    Section {
                        SecureField(AppLocalization.string("Wallet Password"), text: $password)
                            .textInputAutocapitalization(.never).autocorrectionDisabled().privacySensitive()
                    }
                }
                Section {
                    Button(AppLocalization.string("Sign Transaction")) {
                        Task {
                            await session.sign(
                                password: needsPassword ? password : nil,
                                authenticate: { await store.authenticate(.send, reason: authenticationReason) },
                                sign: { try await store.bridge.ready().signSend(id: $0, reviewDigest: $1, password: $2) })
                            password = ""
                        }
                    }.disabled(!canSign || session.isBusy || (needsPassword && password.isEmpty))
                }.onDisappear { password = "" }
            case .signed:
                if artifact.attempts.isEmpty {
                    Section(AppLocalization.string("Broadcast To")) {
                        ForEach(session.endpoints, id: \.self) { endpoint in
                            Toggle(isOn: Binding(
                                get: { session.selectedEndpoints.contains(endpoint) },
                                set: { selected in
                                    if selected { session.selectedEndpoints.insert(endpoint) }
                                    else { session.selectedEndpoints.remove(endpoint) }
                                }
                            )) {
                                Text(verbatim: endpoint).font(.caption.monospaced()).lineLimit(1)
                            }
                        }
                        Button(AppLocalization.string("Broadcast Transaction")) {
                            Task {
                                if await session.broadcast(submit: {
                                    try await store.bridge.ready().broadcastSend(id: $0, endpoints: $1)
                                }) != nil {
                                    _ = await store.refreshTransactionProjection()
                                }
                            }
                        }.disabled(session.isBusy || session.selectedEndpoints.isEmpty)
                    }
                } else {
                    Section {
                        Label(submittedText, systemImage: "checkmark.circle.fill").foregroundStyle(.tint)
                        if let hash = artifact.transactionHash {
                            Text(verbatim: hash).font(.caption2.monospaced()).textSelection(.enabled)
                        }
                    }
                }
            }
        }
        if session.isBusy {
            Section { ProgressView().frame(maxWidth: .infinity) }
        }
        if let error = session.error {
            Section { Text(error).foregroundStyle(.red) }
        }
    }
}
