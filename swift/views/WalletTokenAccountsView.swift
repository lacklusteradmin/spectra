import SwiftUI

/// A Solana wallet's empty token accounts and the rent each holds, as core
/// reads them from a node; and closing the closable ones for their rent:
/// core builds the closure as a send, and this page signs and broadcasts it
/// through the same stages.
struct WalletTokenAccountsView: View {
    let store: AppState
    let wallet: WalletView
    @State private var empty: EmptyTokenAccounts?
    @State private var error: String?
    @State private var isLoading = false
    @State private var isClosing = false

    var body: some View {
        List {
            if let empty {
                if empty.accounts.isEmpty {
                    Section { Text(AppLocalization.string("No empty token accounts.")).foregroundStyle(.secondary) }
                } else {
                    Section {
                        ForEach(empty.accounts, id: \.address) { row($0) }
                    } footer: {
                        Text(AppLocalization.string("Each token this address has held keeps an account with rent in it, even after the tokens are gone. Closing an empty one returns its rent."))
                    }
                    if empty.accounts.contains(where: { $0.blocked == nil }) && !wallet.signing.isWatchOnly {
                        Section {
                            Button(AppLocalization.format("Close and Reclaim %@ SOL", empty.reclaimable)) {
                                isClosing = true
                            }
                        } footer: {
                            Text(AppLocalization.string("One transaction closes up to 20 accounts; close again for the rest."))
                        }
                    }
                }
            } else if let error {
                Section { Text(error).foregroundStyle(.red) }
            } else {
                Section { ProgressView().frame(maxWidth: .infinity) }
            }
        }
        .navigationTitle(WalletAction.tokenAccounts.title).navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .sheet(isPresented: $isClosing, onDismiss: { Task { await load() } }) {
            NavigationStack { CloseTokenAccountsView(store: store, wallet: wallet) }
        }
    }

    private func row(_ account: EmptyTokenAccount) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            HStack {
                Text(verbatim: account.symbol).font(.subheadline.weight(.semibold)).lineLimit(1)
                    .truncationMode(.middle)
                if account.token2022 {
                    Text(verbatim: "Token-2022").font(.caption2).foregroundStyle(.secondary)
                }
                Spacer()
                Text(verbatim: "\(account.rent) SOL").font(.subheadline).monospacedDigit()
            }
            Text(verbatim: account.address).font(.caption2.monospaced()).foregroundStyle(.secondary)
                .lineLimit(1).truncationMode(.middle)
            if let blocked = account.blocked {
                Label(AppLocalization.string(blocked), systemImage: "lock")
                    .font(.caption).foregroundStyle(Color.spectraWarning)
            }
        }.padding(.vertical, SpectraLayout.Space.xxs)
    }

    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            empty = try await store.bridge.ready().walletEmptyTokenAccounts(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}

/// Building, signing and broadcasting one closure of every closable account.
private struct CloseTokenAccountsView: View {
    let store: AppState
    let wallet: WalletView
    @Environment(\.dismiss) private var dismiss
    @State private var session = SendSession()

    var body: some View {
        Form {
            if case let .closeTokenAccounts(accounts, rent, networkFee) = session.artifact?.operation {
                Section {
                    LabeledContent(
                        AppLocalization.string("Accounts"),
                        value: AppLocalization.format("%lld accounts", count: accounts.count, accounts.count))
                    LabeledContent(AppLocalization.string("Rent Returned"), value: "\(rent) SOL")
                    LabeledContent(AppLocalization.string("Network Fee"), value: "\(networkFee) SOL")
                } footer: {
                    Text(AppLocalization.string("The accounts are deleted and their rent comes back to this wallet. A token sent here later opens a new account."))
                }
            }
            SendArtifactStageSections(
                store: store, wallet: wallet, session: session,
                authenticationReason: AppLocalization.format("Authenticate to close token accounts of %@", wallet.name),
                submittedText: AppLocalization.string("Submitted. The rent is back once it confirms."))
        }
        .navigationTitle(AppLocalization.string("Close Token Accounts")).navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(AppLocalization.string("Done")) { dismiss() }
            }
        }
        .task {
            await session.load(
                operation: .build,
                prepare: { try await store.bridge.ready().buildTokenAccountClosure(walletId: wallet.id, accounts: []) },
                endpoints: { try await store.bridge.ready().sendEndpoints(chain: $0) })
        }
    }
}
