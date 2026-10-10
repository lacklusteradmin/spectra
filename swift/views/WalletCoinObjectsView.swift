import SwiftUI

/// A Sui wallet's coin types and how many objects hold each, as core reads
/// them from a node; and merging a type's objects into one: core builds and
/// prices the merge as a send, and this page signs and broadcasts it
/// through the same stages.
struct WalletCoinObjectsView: View {
    let store: AppState
    let wallet: WalletView
    @State private var types: [SuiCoinType]?
    @State private var error: String?
    @State private var isLoading = false
    @State private var merging: SuiCoinType?

    var body: some View {
        List {
            if let types {
                Section {
                    ForEach(types, id: \.coinType) { row($0) }
                } footer: {
                    Text(AppLocalization.string("Every coin received on Sui is a separate object, and a send names each one it spends. Merging a coin's objects into one makes later sends cheaper."))
                }
            } else if let error {
                WalletToolErrorSection(message: error)
            } else {
                WalletToolLoadingSection()
            }
        }
        .navigationTitle(WalletAction.coinObjects.title).navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .sheet(item: $merging, onDismiss: { Task { await load() } }) { type in
            NavigationStack { MergeCoinsView(store: store, wallet: wallet, type: type) }
        }
    }

    private func row(_ type: SuiCoinType) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            HStack {
                Text(verbatim: type.symbol).font(.subheadline.weight(.semibold))
                Spacer()
                Text(AppLocalization.format("%lld objects", count: Int(type.objects), Int(type.objects)))
                    .font(.subheadline).monospacedDigit()
            }
            Text(verbatim: type.balance).font(.caption).monospacedDigit().foregroundStyle(.secondary)
            Text(verbatim: type.coinType).font(.caption2.monospaced()).foregroundStyle(.secondary)
                .lineLimit(1).truncationMode(.middle)
            if type.mergeable && !wallet.signing.isWatchOnly {
                Button(AppLocalization.string("Merge")) { merging = type }.font(.caption.weight(.semibold))
            }
        }.padding(.vertical, SpectraLayout.Space.xxs)
    }

    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            types = try await store.bridge.ready().walletCoinObjects(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}

extension SuiCoinType: Identifiable {
    public var id: String { coinType }
}

/// Building, signing and broadcasting one merge.
private struct MergeCoinsView: View {
    let store: AppState
    let wallet: WalletView
    let type: SuiCoinType
    @Environment(\.dismiss) private var dismiss
    @State private var session = SendSession()

    var body: some View {
        Form {
            Section {
                LabeledContent(AppLocalization.string("Coin"), value: type.symbol)
                if case let .mergeCoins(_, objects, networkFee) = session.artifact?.operation {
                    LabeledContent(
                        AppLocalization.string("Objects Merged"),
                        value: AppLocalization.format("%lld objects", count: Int(objects), Int(objects)))
                    LabeledContent(AppLocalization.string("Network Fee"), value: "≤ \(networkFee) SUI")
                }
            } footer: {
                Text(AppLocalization.string("The objects become one, and nothing leaves the wallet. Storage freed by the merged objects is refunded, which often covers the fee."))
            }
            SendArtifactStageSections(
                store: store, wallet: wallet, session: session,
                authenticationReason: AppLocalization.format("Authenticate to merge coins in %@", wallet.name),
                submittedText: AppLocalization.string("Submitted. The objects are one once it confirms."))
        }
        .navigationTitle(AppLocalization.string("Merge Coins")).navigationBarTitleDisplayMode(.inline)
        .sendSheetDismissal(session: session) { dismiss() }
        .task {
            await session.load(
                operation: .build,
                prepare: { try await store.bridge.ready().buildCoinMerge(walletId: wallet.id, coinType: type.coinType) },
                endpoints: { try await store.bridge.ready().sendEndpoints(chain: $0) })
        }
    }
}
