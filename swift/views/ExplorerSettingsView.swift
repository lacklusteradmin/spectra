import SwiftUI

/// The explorer each network's transactions and addresses open in.
/// Read-only: an explorer is a page Spectra links to, not a service it
/// requests.
struct ExplorerSettingsView: View {
    private let explorers = Spectra.explorers()

    var body: some View {
        Form {
            Section {
                ForEach(explorers, id: \.chainId) { explorer in
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                        HStack(alignment: .firstTextBaseline) {
                            Text(explorer.chainId.displayName)
                                .font(.subheadline.weight(.semibold))
                            Spacer(minLength: SpectraLayout.Space.s)
                            Text(explorer.name).font(.caption).foregroundStyle(.secondary)
                        }
                        Text(explorer.txUrl).font(.caption.monospaced()).foregroundStyle(.secondary)
                            .textSelection(.enabled).lineLimit(3)
                        if let addressUrl = explorer.addressUrl {
                            Text(addressUrl).font(.caption.monospaced()).foregroundStyle(.secondary)
                                .textSelection(.enabled).lineLimit(3)
                        }
                    }
                }
            } footer: {
                Text(AppLocalization.string("explorers.footer"))
            }
        }
        .navigationTitle(AppLocalization.string("Explorers"))
    }
}
