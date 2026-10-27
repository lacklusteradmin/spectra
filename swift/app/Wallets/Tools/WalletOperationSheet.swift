import SwiftUI

/// One transaction a wallet tool builds — a revocation, a closing, a merge, an
/// NFT sent, private funds moved — from what it asks for to its broadcast:
/// the form, when the operation needs input; core's build, summarised as the
/// tool describes it, with the fee and the warnings core's review carries;
/// then the stages every transaction goes through. Each tool says only what
/// is its own.
struct WalletOperationSheet<Fields: View, Summary: View>: View {
    let store: AppState
    let wallet: WalletView
    let title: String
    /// Why the device asks to authenticate before signing.
    let authenticationReason: String
    /// What the sheet says once the transaction is submitted.
    let submittedText: String
    /// Whether the form holds what the build needs; `nil` for an operation
    /// that asks for nothing, which is built as the sheet opens.
    var formIsComplete: Bool? = nil
    /// Whether the sheet's own confirmation lets the transaction be signed.
    var canSign = true
    /// The nodes the signed transaction can go to, when not the network's
    /// send endpoints.
    var endpoints: ((Chain) async throws -> [String])? = nil
    /// Core's build of the operation.
    let build: () async throws -> SendArtifact
    @ViewBuilder let fields: () -> Fields
    /// What the operation does: shown from the start when it asks for
    /// nothing, without an artifact until core has built it, and in place of
    /// the form once it has.
    @ViewBuilder let summary: (SendArtifact?) -> Summary
    @Environment(\.dismiss) private var dismiss
    @State private var session: SendSession

    init(
        store: AppState, wallet: WalletView, title: String, authenticationReason: String, submittedText: String,
        formIsComplete: Bool? = nil, canSign: Bool = true,
        endpoints: ((Chain) async throws -> [String])? = nil,
        build: @escaping () async throws -> SendArtifact,
        @ViewBuilder fields: @escaping () -> Fields = { EmptyView() },
        @ViewBuilder summary: @escaping (SendArtifact?) -> Summary
    ) {
        self.store = store
        self.wallet = wallet
        self.title = title
        self.authenticationReason = authenticationReason
        self.submittedText = submittedText
        self.formIsComplete = formIsComplete
        self.canSign = canSign
        self.endpoints = endpoints
        self.build = build
        self.fields = fields
        self.summary = summary
        _session = State(initialValue: SendSession(bridge: store.bridge))
    }

    var body: some View {
        Form {
            if let artifact = session.artifact {
                summary(artifact)
                if let fee = artifact.review.networkFee {
                    Section {
                        LabeledContent(
                            AppLocalization.string("Network Fee"),
                            value: store.amounts.reviewedNetworkFee(fee, chain: artifact.chainId))
                    }
                }
                let warnings = artifact.reviewWarningMessages
                if !warnings.isEmpty {
                    Section {
                        ForEach(warnings, id: \.self) {
                            Label($0, systemImage: "exclamationmark.triangle").foregroundStyle(Color.spectraWarning)
                        }
                    }
                }
            } else if let formIsComplete {
                fields()
                Section {
                    Button(AppLocalization.string("Review")) { Task { await load() } }
                        .disabled(!formIsComplete || session.isBusy)
                }
            } else {
                summary(nil)
            }
            SendArtifactStageSections(
                store: store, wallet: wallet, session: session,
                authenticationReason: authenticationReason, submittedText: submittedText, canSign: canSign)
        }
        .navigationTitle(title).navigationBarTitleDisplayMode(.inline)
        .sendSheetDismissal(session: session) { dismiss() }
        .task { if formIsComplete == nil { await load() } }
    }

    private func load() async {
        await session.load(
            operation: .build, prepare: build,
            endpoints: { chain in
                if let endpoints { return try await endpoints(chain) }
                return try await store.bridge.ready().sendEndpoints(chain: chain)
            })
    }
}
