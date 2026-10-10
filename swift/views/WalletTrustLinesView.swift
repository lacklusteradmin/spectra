import SwiftUI

/// An XRP Ledger or Stellar wallet's trust lines, as core reads them from a
/// node: the issued assets the wallet can hold. Trusting another asset and
/// removing an empty line are transactions core builds; this page signs and
/// broadcasts them through the same stages as a send.
struct WalletTrustLinesView: View {
    let store: AppState
    let wallet: WalletView
    @State private var lines: WalletTrustLines?
    @State private var error: String?
    @State private var isLoading = false
    @State private var assetInput = ""
    @State private var change: TrustLineChange?

    var body: some View {
        List {
            if let lines {
                if lines.lines.isEmpty {
                    WalletToolEmptySection(message: "No trust lines.")
                } else {
                    Section {
                        ForEach(lines.lines, id: \.asset) { row($0) }
                    } footer: {
                        Text(AppLocalization.format(
                            "Each trust line locks %@ %@ of reserve while it exists.",
                            lines.reservePerLine, wallet.chain.gasTokenSymbol))
                    }
                }
                if !wallet.signing.isWatchOnly {
                    Section {
                        TextField(AppLocalization.string(identifierPrompt), text: $assetInput)
                            .textInputAutocapitalization(.never).autocorrectionDisabled()
                            .font(.callout.monospaced())
                        Button(AppLocalization.string("Trust Asset")) {
                            change = TrustLineChange(asset: assetInput.trimmingCharacters(in: .whitespaces), remove: false)
                        }.disabled(assetInput.trimmingCharacters(in: .whitespaces).isEmpty)
                    } header: {
                        Text(AppLocalization.string("Trust an Asset"))
                    } footer: {
                        Text(AppLocalization.string("A wallet can receive an issued asset only once it trusts the asset's issuer for it."))
                    }
                }
            } else if let error {
                WalletToolErrorSection(message: error)
            } else {
                WalletToolLoadingSection()
            }
        }
        .navigationTitle(WalletAction.trustLines.title).navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .sheet(item: $change, onDismiss: { Task { await load() } }) { change in
            NavigationStack { TrustLineChangeView(store: store, wallet: wallet, change: change) }
        }
    }

    /// What the network calls an issued asset's identifier.
    private var identifierPrompt: String {
        let prompt = wallet.chain.entry?.contractAddressPrompt ?? ""
        return prompt.isEmpty ? "Token Identifier" : prompt
    }

    private func row(_ line: WalletTrustLine) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            HStack {
                Text(verbatim: line.code).font(.subheadline.weight(.semibold)).lineLimit(1)
                Spacer()
                Text(verbatim: AmountPresentation.localizedDecimal(line.balance))
                    .font(.subheadline).monospacedDigit().lineLimit(1)
            }
            Text(verbatim: line.issuer).font(.caption2.monospaced()).foregroundStyle(.secondary)
                .lineLimit(1).truncationMode(.middle)
            if !line.authorized {
                Label(AppLocalization.string("The issuer has not authorized this wallet to hold it."), systemImage: "hand.raised")
                    .font(.caption).foregroundStyle(Color.spectraWarning)
            }
            if line.frozen {
                Label(AppLocalization.string("The issuer has frozen this line."), systemImage: "snowflake")
                    .font(.caption).foregroundStyle(Color.spectraWarning)
            }
            if let blocked = line.removalBlocked {
                Label(AppLocalization.string(blocked), systemImage: "lock")
                    .font(.caption).foregroundStyle(.secondary)
            } else if !wallet.signing.isWatchOnly {
                Button(AppLocalization.string("Remove Trust Line"), role: .destructive) {
                    change = TrustLineChange(asset: line.asset, remove: true)
                }.font(.caption)
            }
        }.padding(.vertical, SpectraLayout.Space.xxs)
    }

    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            lines = try await store.bridge.ready().walletTrustLines(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}

/// One trust line to open or remove.
private struct TrustLineChange: Identifiable {
    let asset: String
    let remove: Bool
    var id: String { "\(remove)-\(asset)" }
}

/// Building, signing and broadcasting one trust line change.
private struct TrustLineChangeView: View {
    let store: AppState
    let wallet: WalletView
    let change: TrustLineChange
    @Environment(\.dismiss) private var dismiss
    @State private var session = SendSession()

    var body: some View {
        Form {
            switch session.artifact?.operation {
            case let .trustAsset(asset, reserve, networkFee):
                summary(asset: asset, reserveTitle: "Reserve Locked", reserve: reserve, networkFee: networkFee,
                        note: "The line accepts any amount of this asset from now on; removing it later frees the reserve.")
            case let .removeTrustLine(asset, reserve, networkFee):
                summary(asset: asset, reserveTitle: "Reserve Freed", reserve: reserve, networkFee: networkFee,
                        note: "The wallet stops accepting this asset; trust it again to receive it.")
            default:
                EmptyView()
            }
            SendArtifactStageSections(
                store: store, wallet: wallet, session: session,
                authenticationReason: AppLocalization.format("Authenticate to change trust lines of %@", wallet.name),
                submittedText: AppLocalization.string("Submitted. The trust line changes once it confirms."))
        }
        .navigationTitle(AppLocalization.string(change.remove ? "Remove Trust Line" : "Trust Asset"))
        .navigationBarTitleDisplayMode(.inline)
        .sendSheetDismissal(session: session) { dismiss() }
        .task {
            await session.load(
                operation: .build,
                prepare: {
                    let bridge = try await store.bridge.ready()
                    return change.remove
                        ? try await bridge.buildRemoveTrustLine(walletId: wallet.id, asset: change.asset)
                        : try await bridge.buildTrustAsset(walletId: wallet.id, asset: change.asset)
                },
                endpoints: { try await store.bridge.ready().sendEndpoints(chain: $0) })
        }
    }

    private func summary(asset: String, reserveTitle: String, reserve: String, networkFee: String, note: String) -> some View {
        let coin = wallet.chain.gasTokenSymbol
        return Section {
            Text(verbatim: asset).font(.caption.monospaced()).textSelection(.enabled)
            LabeledContent(AppLocalization.string(reserveTitle), value: "\(reserve) \(coin)")
            LabeledContent(AppLocalization.string("Network Fee"), value: "\(networkFee) \(coin)")
        } footer: {
            Text(AppLocalization.string(note))
        }
    }
}
