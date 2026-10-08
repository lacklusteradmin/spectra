import SwiftUI

/// The token contracts holding a storage deposit this NEAR account can have
/// back, as core reads them from a node; and getting one back: core builds
/// the unregistration as a send, and this page signs and broadcasts it
/// through the same stages.
struct WalletTokenStorageView: View {
    let store: AppState
    let wallet: WalletView
    @State private var storage: NearStorageDeposits?
    @State private var error: String?
    @State private var isLoading = false
    @State private var refunding: NearStorageDeposit?

    var body: some View {
        List {
            if let storage {
                Section {
                    if storage.deposits.isEmpty {
                        Text(AppLocalization.string("No storage deposits to get back.")).foregroundStyle(.secondary)
                    }
                    ForEach(storage.deposits, id: \.contract) { row($0) }
                } footer: {
                    Text(AppLocalization.string("A token contract keeps a deposit, usually 0.00125 NEAR, for each account it holds a balance for, even after the tokens are gone. Once the account holds none of a token, unregistering returns the deposit."))
                }
                if !storage.deposits.isEmpty {
                    Section {
                        LabeledContent(AppLocalization.string("Refundable"), value: "\(storage.refundable) NEAR")
                    }
                }
            } else if let error {
                Section { Text(error).foregroundStyle(.red) }
            } else {
                Section { ProgressView().frame(maxWidth: .infinity) }
            }
        }
        .navigationTitle(WalletAction.tokenStorage.title).navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .sheet(item: $refunding, onDismiss: { Task { await load() } }) { deposit in
            NavigationStack { RefundTokenStorageView(store: store, wallet: wallet, deposit: deposit) }
        }
    }

    private func row(_ deposit: NearStorageDeposit) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            HStack {
                Text(verbatim: deposit.symbol).font(.subheadline.weight(.semibold)).lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                Text(verbatim: "\(deposit.refund) NEAR").font(.subheadline).monospacedDigit()
            }
            Text(verbatim: deposit.contract).font(.caption2.monospaced()).foregroundStyle(.secondary)
                .lineLimit(1).truncationMode(.middle)
            if !wallet.signing.isWatchOnly {
                Button(AppLocalization.string("Get Deposit Back")) { refunding = deposit }
                    .font(.caption.weight(.semibold))
            }
        }.padding(.vertical, SpectraLayout.Space.xxs)
    }

    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            storage = try await store.bridge.ready().walletTokenStorage(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}

extension NearStorageDeposit: Identifiable {
    public var id: String { contract }
}

/// Building, signing and broadcasting one unregistration.
private struct RefundTokenStorageView: View {
    let store: AppState
    let wallet: WalletView
    let deposit: NearStorageDeposit
    @Environment(\.dismiss) private var dismiss
    @State private var session = SendSession()

    var body: some View {
        Form {
            Section {
                LabeledContent(AppLocalization.string("Contract")) {
                    Text(verbatim: deposit.contract).font(.caption.monospaced()).lineLimit(1)
                        .truncationMode(.middle)
                }
                if case let .refundTokenStorage(_, refund, networkFee) = session.artifact?.operation {
                    LabeledContent(AppLocalization.string("Deposit Returned"), value: "\(refund) NEAR")
                    LabeledContent(AppLocalization.string("Network Fee"), value: "≤ \(networkFee) NEAR")
                }
            } footer: {
                Text(AppLocalization.string("The contract unregisters this account and returns its deposit. Receiving the token again registers the account anew, for a new deposit."))
            }
            SendArtifactStageSections(
                store: store, wallet: wallet, session: session,
                authenticationReason: AppLocalization.format("Authenticate to get a storage deposit back for %@", wallet.name),
                submittedText: AppLocalization.string("Submitted. The deposit is back once it confirms."))
        }
        .navigationTitle(AppLocalization.string("Get Deposit Back")).navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(AppLocalization.string("Done")) { dismiss() }
            }
        }
        .task {
            await session.load(
                operation: .build,
                prepare: {
                    try await store.bridge.ready().buildTokenStorageRefund(
                        walletId: wallet.id, contract: deposit.contract)
                },
                endpoints: { try await store.bridge.ready().sendEndpoints(chain: $0) })
        }
    }
}
