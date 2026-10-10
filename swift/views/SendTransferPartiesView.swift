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
    /// The destination tag or memo the transaction carries to the recipient.
    var memo: PaymentMemo? = nil
    /// The name a save to the address book starts from; `nil` offers none.
    var suggestedContactName: String? = nil

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
                address: recipient, isSender: false, suggestedContactName: suggestedContactName)
            if let memo {
                PaymentMemoRow(memo: memo)
                    .accessibilityIdentifier("send.review.memo")
            }
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
    var suggestedContactName: String? = nil
    @State private var holder: EndpointHolder?
    @State private var isSavingContact = false
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
                    // Two lines for every party once core has answered, so
                    // the sender and the recipient sit the same way above
                    // their addresses.
                    if let role {
                        Text(AppLocalization.string(role))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }

            HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
                Text(readableAddress(address, chain: chain))
                    .font(.subheadline.monospaced())
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityLabel(Text(verbatim: address))
                    .accessibilityIdentifier(isSender ? "send.review.sender" : "send.review.recipient")
                CopyButton(value: displayAddress(chain: chain, address: address))
            }

            if !isSender, lookupCompleted, holder == nil, let suggestedContactName {
                Button {
                    spectraHaptic(.light)
                    isSavingContact = true
                } label: {
                    Label(AppLocalization.string("Save Recipient To Address Book"), systemImage: "book.closed")
                        .font(.subheadline.weight(.semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, SpectraLayout.Space.s)
                }
                .buttonStyle(.glass)
                .sheet(isPresented: $isSavingContact) {
                    SaveContactSheet(
                        addressBook: store.addressBook, chain: chain,
                        address: displayAddress(chain: chain, address: address), name: suggestedContactName)
                }
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

    private var role: String? {
        if isSender { return holder == nil ? nil : "Sending wallet" }
        switch holder {
        case .contact: return "Saved contact"
        case .wallet: return "Your wallet"
        case nil: return lookupCompleted ? "Not in your address book" : nil
        }
    }

    private var systemImage: String {
        if isSender { return "wallet.bifold" }
        switch holder {
        case .wallet: return "wallet.bifold"
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
