import SwiftUI
import UIKit

/// Core resolves the names; the reviewed addresses remain complete and copyable.
/// The caller owns this content's card so fee rows can share the same surface.
struct SendTransferPartiesView: View {
    let store: AppState
    let walletId: String
    let chain: Chain
    let sender: String?
    let recipient: String
    var saveRecipient: (() -> Void)? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            if let sender {
                SendTransferPartyView(
                    store: store, walletId: walletId, chain: chain,
                    address: sender, isSender: true)
                Label(AppLocalization.string("Transfer to"), systemImage: "arrow.down")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.leading, SpectraLayout.Space.l)
            }
            SendTransferPartyView(
                store: store, walletId: walletId, chain: chain,
                address: recipient, isSender: false, saveRecipient: saveRecipient)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct SendTransferPartyView: View {
    let store: AppState
    let walletId: String
    let chain: Chain
    let address: String
    let isSender: Bool
    var saveRecipient: (() -> Void)? = nil
    @State private var holder: EndpointHolder?
    @State private var lookupCompleted = false

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            HStack(spacing: SpectraLayout.Space.m) {
                Image(systemName: systemImage)
                    .font(.headline)
                    .foregroundStyle(.tint)
                    .frame(width: 36, height: 36)
                    .spectraInsetFill()
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(verbatim: name).font(.headline)
                    if holder != nil {
                        Text(AppLocalization.string(role))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }

            Text(groupedAddress(address))
                .font(.subheadline.monospaced())
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityLabel(Text(verbatim: address))
                .accessibilityIdentifier(isSender ? "send.review.sender" : "send.review.recipient")
                .contextMenu {
                    Button {
                        UIPasteboard.general.string = address
                    } label: {
                        Label(AppLocalization.string("Copy"), systemImage: "doc.on.doc")
                    }
                }

            if !isSender, lookupCompleted, holder == nil, let saveRecipient {
                Button {
                    spectraHaptic(.light)
                    saveRecipient()
                } label: {
                    Label(AppLocalization.string("Save Recipient To Address Book"), systemImage: "book.closed")
                        .font(.subheadline.weight(.semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, SpectraLayout.Space.s)
                }
                .buttonStyle(.glass)
            }
        }
        .task(id: LookupIdentity(
            walletId: walletId, chain: chain, address: address, contacts: store.addressBook.entries)) {
            holder = nil
            lookupCompleted = false
            guard !walletId.isEmpty, !address.isEmpty else { return }
            do {
                let answer = try await store.bridge.ready().addressHolder(
                    walletId: walletId, chainId: chain, address: address)
                guard !Task.isCancelled else { return }
                holder = answer
                lookupCompleted = true
            } catch {
                guard !Task.isCancelled else { return }
            }
        }
    }

    private var name: String {
        switch holder {
        case .wallet(let name), .contact(let name): name
        case nil: AppLocalization.string(isSender ? "Sending wallet" : "Recipient")
        }
    }

    private var role: String {
        if isSender { return "Sending wallet" }
        switch holder {
        case .contact: return "Saved contact"
        case .wallet: return "Your wallet"
        case nil: return "Recipient"
        }
    }

    private var systemImage: String {
        if isSender { return "wallet.pass" }
        switch holder {
        case .wallet: return "wallet.pass"
        case .contact, nil: return "person.crop.circle"
        }
    }

    private struct LookupIdentity: Hashable {
        let walletId: String
        let chain: Chain
        let address: String
        let contacts: [AddressBookEntry]
    }
}
