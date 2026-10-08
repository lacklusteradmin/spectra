import SwiftUI

/// The ERC-721 and ERC-1155 tokens an EVM wallet holds, as core reads them
/// from its network's Blockscout inventory, and sending one: core reads the
/// collection's standard and the wallet's ownership, builds the collection's
/// `safeTransferFrom` as a send, and this page signs and broadcasts it
/// through the same stages. Token images are not loaded: a token's metadata
/// can point at any host, and fetching from it tells that host who looked.
struct WalletNftsView: View {
    let store: AppState
    let wallet: WalletView
    @State private var nfts: WalletNfts?
    @State private var error: String?
    @State private var isLoading = false
    @State private var sending: WalletNft?

    var body: some View {
        List {
            if let nfts {
                if nfts.nfts.isEmpty {
                    Section { Text(AppLocalization.string("This wallet holds no NFTs.")).foregroundStyle(.secondary) }
                }
                Section {
                    ForEach(nfts.nfts) { row($0) }
                } footer: {
                    if !nfts.complete {
                        Text(AppLocalization.string("The explorer's list stopped short; the wallet may hold more NFTs."))
                    }
                }
            } else if let error {
                Section { Text(error).foregroundStyle(.red) }
            } else {
                Section { ProgressView().frame(maxWidth: .infinity) }
            }
        }
        .navigationTitle(WalletAction.nfts.title).navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .sheet(item: $sending, onDismiss: { Task { await load() } }) { nft in
            NavigationStack { SendNftView(store: store, wallet: wallet, nft: nft) }
        }
    }

    private func row(_ nft: WalletNft) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            HStack {
                Text(verbatim: nft.title).font(.subheadline.weight(.semibold)).lineLimit(1)
                Spacer()
                if nft.standard == .erc1155 {
                    Text(verbatim: "×\(nft.quantity)").font(.subheadline).monospacedDigit().lineLimit(1)
                }
            }
            Text(verbatim: "#\(nft.tokenId)").font(.caption.monospaced()).foregroundStyle(.secondary)
                .lineLimit(1).truncationMode(.middle)
            Text(verbatim: nft.contract).font(.caption2.monospaced()).foregroundStyle(.secondary)
                .lineLimit(1).truncationMode(.middle)
            if !wallet.signing.isWatchOnly {
                Button(AppLocalization.string("Send")) { sending = nft }.font(.caption.weight(.semibold))
            }
        }.padding(.vertical, SpectraLayout.Space.xxs)
    }

    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            nfts = try await store.bridge.ready().walletNfts(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}

extension WalletNft: Identifiable {
    public var id: String { "\(contract):\(tokenId)" }
    /// The token's own name, else its collection's, else its standard.
    var title: String {
        if let name, !name.isEmpty { return name }
        return collection.isEmpty ? standard.label : collection
    }
}

extension NftStandard {
    var label: String {
        switch self {
        case .erc721: "ERC-721"
        case .erc1155: "ERC-1155"
        }
    }
}

/// Sending one NFT: the recipient, and how many of an ERC-1155 token; then
/// the transaction core builds, signed and broadcast through the stages.
private struct SendNftView: View {
    let store: AppState
    let wallet: WalletView
    let nft: WalletNft
    @Environment(\.dismiss) private var dismiss
    @State private var session = SendSession()
    @State private var recipient = ""
    @State private var quantity = "1"

    var body: some View {
        Form {
            Section {
                LabeledContent(AppLocalization.string("Token"), value: nft.title)
                LabeledContent(AppLocalization.string("Token ID")) {
                    Text(verbatim: nft.tokenId).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
                }
                LabeledContent(AppLocalization.string("Standard"), value: nft.standard.label)
                LabeledContent(AppLocalization.string("Contract")) {
                    Text(verbatim: nft.contract).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
                }
            }
            if let artifact = session.artifact,
               case let .transferNft(_, _, _, quantity, _, networkFee) = artifact.operation {
                Section {
                    LabeledContent(AppLocalization.string("To")) {
                        Text(verbatim: artifact.recipient).font(.caption.monospaced()).lineLimit(1)
                            .truncationMode(.middle)
                    }
                    LabeledContent(AppLocalization.string("Quantity"), value: quantity)
                    LabeledContent(AppLocalization.string("Network Fee"),
                        value: "≤ \(networkFee) \(wallet.chain.gasTokenSymbol)")
                } footer: {
                    Text(AppLocalization.string("A contract recipient must accept the token, or the transfer is refused and only the fee is spent."))
                }
            } else {
                Section {
                    TextField(AppLocalization.string("Destination Address"), text: $recipient)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().font(.body.monospaced())
                    if nft.standard == .erc1155 {
                        TextField(AppLocalization.string("Quantity"), text: $quantity)
                            .keyboardType(.numberPad).monospacedDigit()
                    }
                } footer: {
                    if nft.standard == .erc1155 {
                        Text(AppLocalization.format("This wallet holds %@ of this token.", nft.quantity))
                    }
                }
                Section {
                    Button(AppLocalization.string("Review")) { Task { await review() } }
                        .disabled(recipient.trimmingCharacters(in: .whitespaces).isEmpty || session.isBusy)
                }
            }
            SendArtifactStageSections(
                store: store, wallet: wallet, session: session,
                authenticationReason: AppLocalization.format("Authenticate to send an NFT from %@", wallet.name),
                submittedText: AppLocalization.string("Submitted. The token moves once it confirms."))
        }
        .navigationTitle(AppLocalization.string("Send NFT")).navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(AppLocalization.string("Done")) { dismiss() }
            }
        }
    }

    private func review() async {
        let recipient = recipient.trimmingCharacters(in: .whitespacesAndNewlines)
        let quantity = nft.standard == .erc1155 ? quantity.trimmingCharacters(in: .whitespaces) : "1"
        await session.load(
            operation: .build,
            prepare: {
                try await store.bridge.ready().buildNftTransfer(
                    walletId: wallet.id, contract: nft.contract, tokenId: nft.tokenId, quantity: quantity,
                    recipient: recipient)
            },
            endpoints: { try await store.bridge.ready().sendEndpoints(chain: $0) })
    }
}
