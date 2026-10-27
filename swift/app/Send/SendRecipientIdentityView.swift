import SwiftUI

/// A local core lookup supplies identity; a UI menu selection never does.
@MainActor
struct SendRecipientIdentityView: View {
    let store: AppState
    let chain: Chain?
    let address: String
    var compact = false
    @State private var holder: EndpointHolder?

    private struct LookupIdentity: Hashable {
        let walletId: String
        let chain: Chain?
        let address: String
        let addressBook: [AddressBookEntry]
    }

    var body: some View {
        // Keep the task on a stable container even before core has an identity.
        VStack(alignment: .leading, spacing: 0) {
            if compact {
                HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.xs) {
                    Label(AppLocalization.string("Send to"), systemImage: "arrow.up.right")
                        .foregroundStyle(.secondary)
                    if let holder {
                        EndpointHolderLabel(holder: holder)
                    } else {
                        Text(verbatim: address).font(.subheadline.monospaced())
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .font(.subheadline)
            } else if holder != nil {
                HStack(spacing: SpectraLayout.Space.m) {
                    Image(systemName: holderSystemImage)
                        .font(.headline).foregroundStyle(.tint)
                        .frame(width: 36, height: 36)
                        .spectraInsetFill()
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                        Text(verbatim: holderName).font(.headline)
                        Text(AppLocalization.string(holderSubtitle)).font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                }
                .accessibilityElement(children: .combine)
            }
        }
        .task(id: LookupIdentity(walletId: store.sendFlow.walletId, chain: chain, address: address, addressBook: store.addressBook.entries)) {
            holder = nil
            let trimmed = address.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty, let chain else { return }
            let answer = try? await store.bridge.ready().addressHolder(
                walletId: store.sendFlow.walletId, chainId: chain, address: trimmed)
            guard !Task.isCancelled else { return }
            holder = answer
        }
    }

    private var holderName: String {
        switch holder {
        case .wallet(let name), .contact(let name): name
        case nil: ""
        }
    }

    private var holderSystemImage: String {
        switch holder {
        case .wallet: "wallet.bifold"
        case .contact, nil: "person.crop.circle"
        }
    }

    private var holderSubtitle: String {
        switch holder {
        case .wallet: "Your wallet"
        case .contact: "Saved contact"
        case nil: ""
        }
    }
}
