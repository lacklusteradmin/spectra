import SwiftUI

/// Who holds an address, as core names it: one of the user's wallets or a
/// saved contact. Shown beside an address wherever one is reviewed, so a
/// reader sees "Alice" before comparing characters.
struct EndpointHolderLabel: View {
    let holder: EndpointHolder

    var body: some View {
        HStack(spacing: SpectraLayout.Space.xs) {
            Image(systemName: systemImage).foregroundStyle(.tint)
            Text(verbatim: name).foregroundStyle(.primary).lineLimit(1)
        }
        .font(.subheadline.weight(.semibold))
        .accessibilityElement(children: .combine)
    }

    private var name: String {
        switch holder {
        case .wallet(let name), .contact(let name): name
        }
    }

    private var systemImage: String {
        switch holder {
        case .wallet: "wallet.bifold"
        case .contact: "person.crop.circle"
        }
    }
}
