import SwiftUI

/// The ERC-20 allowances an EVM wallet has granted, as core reads them from
/// its indexer and confirms live, and revoking one: core builds
/// `approve(spender, 0)` as a send, and this page signs and broadcasts it
/// through the same stages.
struct WalletApprovalsView: View {
    let store: AppState
    let wallet: WalletView
    @State private var approvals: TokenApprovals?
    @State private var error: String?
    @State private var isLoading = false
    @State private var revoking: TokenApproval?

    var body: some View {
        List {
            if let approvals {
                if approvals.approvals.isEmpty {
                    Section { Text(AppLocalization.string("This wallet has no standing token approvals.")).foregroundStyle(.secondary) }
                }
                Section {
                    ForEach(approvals.approvals, id: \.id) { approval in
                        row(approval)
                    }
                } footer: {
                    if !approvals.complete {
                        Text(AppLocalization.string("The indexer's list stopped short; more approvals may stand."))
                    }
                }
            } else if let error {
                Section { Text(error).foregroundStyle(.red) }
            } else {
                Section { ProgressView().frame(maxWidth: .infinity) }
            }
        }
        .navigationTitle(WalletAction.tokenApprovals.title).navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .sheet(item: $revoking, onDismiss: { Task { await load() } }) { approval in
            NavigationStack { RevokeApprovalView(store: store, wallet: wallet, approval: approval) }
        }
    }

    private func row(_ approval: TokenApproval) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            HStack {
                Text(approval.symbol.isEmpty ? AppLocalization.string("Token") : approval.symbol)
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Text(approval.unlimited ? AppLocalization.string("Unlimited") : approval.allowance)
                    .font(.subheadline).monospacedDigit()
                    .foregroundStyle(approval.unlimited ? Color.spectraWarning : Color.primary)
            }
            Text(verbatim: approval.token).font(.caption2.monospaced()).foregroundStyle(.secondary)
                .lineLimit(1).truncationMode(.middle)
            HStack {
                Text(AppLocalization.string("Spender")).font(.caption).foregroundStyle(.secondary)
                Text(verbatim: approval.spender).font(.caption2.monospaced()).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
            }
            if !wallet.signing.isWatchOnly {
                Button(role: .destructive) { revoking = approval } label: {
                    Text(AppLocalization.string("Revoke"))
                }.font(.caption.weight(.semibold))
            }
        }.padding(.vertical, SpectraLayout.Space.xxs)
    }

    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            approvals = try await store.bridge.ready().walletTokenApprovals(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}

extension TokenApproval: Identifiable {
    public var id: String { "\(token):\(spender)" }
}

/// Building, signing and broadcasting one revocation. Core keeps the
/// artifact; the session adopts its answers for this sheet.
private struct RevokeApprovalView: View {
    let store: AppState
    let wallet: WalletView
    let approval: TokenApproval
    @Environment(\.dismiss) private var dismiss
    @State private var session = SendSession()

    var body: some View {
        Form {
            Section {
                LabeledContent(AppLocalization.string("Token"), value: approval.symbol.isEmpty ? approval.token : approval.symbol)
                LabeledContent(AppLocalization.string("Spender")) {
                    Text(verbatim: approval.spender).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
                }
                if case let .revokeApproval(_, _, networkFee) = session.artifact?.operation {
                    LabeledContent(AppLocalization.string("Network Fee"),
                        value: "≤ \(networkFee) \(wallet.chain.gasTokenSymbol)")
                }
            } footer: {
                Text(AppLocalization.string("This sets the allowance to zero. The spender can no longer move this token from the wallet."))
            }
            SendArtifactStageSections(
                store: store, wallet: wallet, session: session,
                authenticationReason: AppLocalization.format("Authenticate to revoke an approval from %@", wallet.name),
                submittedText: AppLocalization.string("Submitted. The allowance is zero once it confirms."))
        }
        .navigationTitle(AppLocalization.string("Revoke Approval")).navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(AppLocalization.string("Done")) { dismiss() }
            }
        }
        .task {
            await session.load(
                operation: .build,
                prepare: {
                    try await store.bridge.ready().buildApprovalRevocation(
                        walletId: wallet.id, token: approval.token, spender: approval.spender)
                },
                endpoints: { try await store.bridge.ready().sendEndpoints(chain: $0) })
        }
    }
}
