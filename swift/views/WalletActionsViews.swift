import SwiftUI

/// The rows and buttons a wallet's page draws from core's `WalletActions`.
/// Core decides what a wallet offers and says what each action does; these
/// views give each action its words and symbol and nothing more.
extension WalletAction {
    var title: String {
        switch self {
        case .send: AppLocalization.string("Send")
        case .multisig: AppLocalization.string("Multisig")
        case .receive: AppLocalization.string("Receive")
        case .history: AppLocalization.string("History")
        case .openInExplorer: AppLocalization.string("Explorer")
        case .addKeys: AppLocalization.string("Add Keys")
        case .stake: AppLocalization.string("Staking")
        case .scanBlocks: AppLocalization.string("Sync Local Wallet")
        case .coins: AppLocalization.string("Addresses and Coins")
        case .tokenApprovals: AppLocalization.string("Token Approvals")
        case .nfts: AppLocalization.string("NFTs")
        case .shieldedFunds: AppLocalization.string("Shielded Funds")
        case .mwebFunds: AppLocalization.string("MWEB Funds")
        case .networkAccount: AppLocalization.string("Network Account")
        case .accessKeys: AppLocalization.string("Access Keys")
        case .tokenStorage: AppLocalization.string("Token Storage")
        case .coinObjects: AppLocalization.string("Coin Objects")
        case .getTestCoins: AppLocalization.string("Get Test Coins")
        case .tokenAccounts: AppLocalization.string("Token Accounts")
        case .trustLines: AppLocalization.string("Trust Lines")
        case .signMessage: AppLocalization.string("Sign Message")
        case .verifyMessage: AppLocalization.string("Verify Message")
        case .addToNetwork: AppLocalization.string("Add to Another Network")
        case .rename: AppLocalization.string("Name")
        case .revealPhrase: AppLocalization.string("Show Seed Phrase")
        case .exportKeys: AppLocalization.string("Export Keys")
        case .delete: AppLocalization.string("Delete Wallet")
        }
    }

    var systemImage: String {
        switch self {
        case .send: "arrow.up"
        case .multisig: "signature"
        case .receive: "arrow.down"
        case .history: "clock.arrow.circlepath"
        case .openInExplorer: "safari"
        case .addKeys: "key"
        case .stake: "link.circle"
        case .scanBlocks: "square.stack.3d.down.right"
        case .coins: "list.bullet.rectangle"
        case .tokenApprovals: "checkmark.shield"
        case .nfts: "square.on.square"
        case .shieldedFunds: "eye.slash"
        case .mwebFunds: "eye.slash.circle"
        case .networkAccount: "gauge.with.dots.needle.33percent"
        case .accessKeys: "key.2.on.ring"
        case .tokenStorage: "archivebox"
        case .coinObjects: "circle.grid.3x3"
        case .getTestCoins: "drop"
        case .tokenAccounts: "tray"
        case .trustLines: "link"
        case .signMessage: "signature"
        case .verifyMessage: "checkmark.seal"
        case .addToNetwork: "plus.square.on.square"
        case .rename: "pencil"
        case .revealPhrase: "faceid"
        case .exportKeys: "key.horizontal"
        case .delete: "trash"
        }
    }
}

extension WalletActions {
    func offers(_ action: WalletAction) -> Bool { actions.contains { $0.action == action } }
    func actions(in section: WalletActionSection) -> [WalletActionOffer] {
        actions.filter { $0.section == section }
    }
}

/// Send, receive and history: one button each, in core's order.
struct WalletEverydayActionBar: View {
    let offers: [WalletActionOffer]
    let perform: (WalletAction) -> Void

    var body: some View {
        HStack(spacing: SpectraLayout.Space.s) {
            ForEach(offers, id: \.action) { offer in
                Button {
                    spectraHaptic(.medium)
                    perform(offer.action)
                } label: {
                    VStack(spacing: SpectraLayout.Space.xs) {
                        Image(systemName: offer.action.systemImage).font(.headline)
                        Text(offer.action.title).font(.caption.weight(.semibold))
                    }.frame(maxWidth: .infinity, minHeight: 52)
                }
                .buttonStyle(.glass)
                .accessibilityHint(AppLocalization.string(offer.note))
            }
        }
    }
}

/// What the wallet's network adds, one row each with what it does.
struct WalletNetworkActionsCard: View {
    let offers: [WalletActionOffer]
    let perform: (WalletAction) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(offers.enumerated()), id: \.element.action) { index, offer in
                if index > 0 { Divider().opacity(0.25) }
                Button {
                    spectraHaptic(.light)
                    perform(offer.action)
                } label: {
                    HStack(spacing: SpectraLayout.Space.m) {
                        Image(systemName: offer.action.systemImage).font(.headline).foregroundStyle(.tint)
                            .frame(width: 28)
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                            Text(offer.action.title).font(.subheadline.weight(.semibold))
                                .foregroundStyle(Color.primary)
                            Text(AppLocalization.string(offer.note)).font(.caption).foregroundStyle(.secondary)
                                .multilineTextAlignment(.leading)
                        }
                        Spacer(minLength: SpectraLayout.Space.s)
                        Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
                            .foregroundStyle(Color(.tertiaryLabel))
                    }.padding(.vertical, SpectraLayout.Space.s).contentShape(Rectangle())
                }.buttonStyle(.plain)
            }
        }
        .padding(.horizontal, SpectraLayout.Space.l).padding(.vertical, SpectraLayout.Space.s)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }
}

/// What the network does not do for this wallet, or does only with the
/// user's endpoint: the limits and coverage the setup page showed. Draws
/// nothing when there is nothing to say.
struct WalletNetworkNotesCard: View {
    let summary: WalletSetupSummary

    private var gaps: [(String, CapabilityCoverage)] {
        var reads = [
            (AppLocalization.string("Balance"), summary.balance),
            (AppLocalization.string("History"), summary.history),
        ]
        if let tokens = summary.tokenDiscovery { reads.append((AppLocalization.string("Token Discovery"), tokens)) }
        return reads.filter { $0.1 != .configured }
    }

    var body: some View {
        let gaps = gaps
        if !summary.limits.isEmpty || !gaps.isEmpty {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                Text(AppLocalization.string("About This Network")).font(.headline)
                ForEach(gaps, id: \.0) { title, coverage in
                    HStack {
                        Text(title).font(.subheadline)
                        Spacer()
                        Text(coverage.title).font(.subheadline).foregroundStyle(Color.spectraWarning)
                    }
                }
                ForEach(summary.limits, id: \.self) { limit in
                    Label(limit.explanation, systemImage: "info.circle").font(.caption).foregroundStyle(.secondary)
                }
            }
            .padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
            .spectraCardFill()
        }
    }
}
