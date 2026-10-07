import SwiftUI

/// A NEAR account's access keys, as core reads them from a node, with the
/// one Spectra signs with marked; and deleting a function-call key a dapp
/// was given: core builds the deletion as a send, and this page signs and
/// broadcasts it through the same stages.
struct WalletAccessKeysView: View {
    let store: AppState
    let wallet: WalletView
    @State private var keys: NearAccessKeys?
    @State private var error: String?
    @State private var isLoading = false
    @State private var deleting: NearAccessKey?

    var body: some View {
        List {
            if let keys {
                let full = keys.keys.filter(\.fullAccess)
                let calls = keys.keys.filter { !$0.fullAccess }
                Section {
                    ForEach(full, id: \.publicKey) { row($0) }
                } header: {
                    Text(AppLocalization.string("Full Access"))
                } footer: {
                    Text(AppLocalization.string("A full-access key can do anything with the account. Spectra lists these keys and does not delete them."))
                }
                Section {
                    if calls.isEmpty {
                        Text(AppLocalization.string("No function-call keys.")).foregroundStyle(.secondary)
                    }
                    ForEach(calls, id: \.publicKey) { row($0) }
                } header: {
                    Text(AppLocalization.string("Function Call"))
                } footer: {
                    Text(AppLocalization.string("A dapp adds a function-call key when you sign in to it. The key can call only its contract and spend gas only from its allowance. Delete the ones you no longer use."))
                }
            } else if let error {
                Section { Text(error).foregroundStyle(.red) }
            } else {
                Section { ProgressView().frame(maxWidth: .infinity) }
            }
        }
        .navigationTitle(WalletAction.accessKeys.title).navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .sheet(item: $deleting, onDismiss: { Task { await load() } }) { key in
            NavigationStack { DeleteAccessKeyView(store: store, wallet: wallet, key: key) }
        }
    }

    private func row(_ key: NearAccessKey) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Text(verbatim: key.publicKey).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
            if key.signs {
                Label(AppLocalization.string("Spectra signs with this key"), systemImage: "checkmark.seal.fill")
                    .font(.caption).foregroundStyle(.tint)
            }
            if let receiver = key.receiver {
                LabeledContent(AppLocalization.string("Contract")) {
                    Text(verbatim: receiver).font(.caption.monospaced())
                }.font(.caption)
                LabeledContent(
                    AppLocalization.string("Methods"),
                    value: key.methodNames.isEmpty
                        ? AppLocalization.string("Any") : key.methodNames.joined(separator: ", ")
                ).font(.caption)
                LabeledContent(
                    AppLocalization.string("Allowance"),
                    value: key.allowance.map { "\($0) NEAR" } ?? AppLocalization.string("Unlimited")
                ).font(.caption)
                if !wallet.signing.isWatchOnly {
                    Button(role: .destructive) { deleting = key } label: {
                        Text(AppLocalization.string("Delete Key"))
                    }.font(.caption.weight(.semibold))
                }
            }
        }.padding(.vertical, SpectraLayout.Space.xxs)
    }

    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            keys = try await store.bridge.ready().walletAccessKeys(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}

extension NearAccessKey: Identifiable {
    public var id: String { publicKey }
}

/// Building, signing and broadcasting one key deletion.
private struct DeleteAccessKeyView: View {
    let store: AppState
    let wallet: WalletView
    let key: NearAccessKey
    @Environment(\.dismiss) private var dismiss
    @State private var session = SendSession()

    var body: some View {
        Form {
            Section {
                LabeledContent(AppLocalization.string("Key")) {
                    Text(verbatim: key.publicKey).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
                }
                if let receiver = key.receiver {
                    LabeledContent(AppLocalization.string("Contract")) {
                        Text(verbatim: receiver).font(.caption.monospaced())
                    }
                }
                if case let .deleteAccessKey(_, _, networkFee) = session.artifact?.operation {
                    LabeledContent(AppLocalization.string("Network Fee"), value: "≤ \(networkFee) NEAR")
                }
            } footer: {
                Text(AppLocalization.string("The contract can no longer act for this account with this key. The dapp asks you to sign in again to get a new one."))
            }
            SendArtifactStageSections(
                store: store, wallet: wallet, session: session,
                authenticationReason: AppLocalization.format("Authenticate to delete a key from %@", wallet.name),
                submittedText: AppLocalization.string("Submitted. The key is gone once it confirms."))
        }
        .navigationTitle(AppLocalization.string("Delete Key")).navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(AppLocalization.string("Done")) { dismiss() }
            }
        }
        .task {
            await session.load(
                operation: .build,
                prepare: {
                    try await store.bridge.ready().buildAccessKeyDeletion(walletId: wallet.id, publicKey: key.publicKey)
                },
                endpoints: { try await store.bridge.ready().sendEndpoints(chain: $0) })
        }
    }
}
