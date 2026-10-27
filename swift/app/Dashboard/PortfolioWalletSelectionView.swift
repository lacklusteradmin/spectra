import SwiftUI

struct PortfolioWalletSelectionView: View {
    let store: AppState
    var body: some View {
        List {
            Section {
                ForEach(store.wallets) { wallet in
                    Toggle(isOn: binding(for: wallet.id)) {
                        PortfolioWalletToggleRowView(walletName: wallet.name, chainTitleText: wallet.networkTitle)
                            .equatable()
                    }
                }
            } header: {
                Text(AppLocalization.string("Included In Portfolio Total"))
            } footer: {
                Text(
                    AppLocalization.string(
                        "Only selected wallets contribute to the portfolio total and the aggregated asset list on the home page."))
            }
        }.navigationTitle(AppLocalization.string("Portfolio Wallets"))
    }
    private func binding(for walletId: String) -> Binding<Bool> {
        Binding(
            get: {
                store.wallet(for: walletId)?.includeInPortfolioTotal ?? true
            }, set: { isIncluded in store.setPortfolioInclusion(isIncluded, for: walletId) }
        )
    }
}

struct PortfolioWalletToggleRowView: View, Equatable {
    let walletName: String
    let chainTitleText: String
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool { lhs.walletName == rhs.walletName && lhs.chainTitleText == rhs.chainTitleText }
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
            Text(walletName)
            Text(chainTitleText).font(.caption).foregroundStyle(.secondary)
        }
    }
}
