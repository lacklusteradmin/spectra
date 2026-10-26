import Foundation
import SwiftUI

extension Chain {
    var settingsIconTint: Color { entry?.color.color ?? .accentColor }
}
extension TokenPreferenceEntry {
    var settingsArtworkName: String { AssetPresentationCatalog.artwork(deploymentId: token.deploymentId) }
    var settingsFallbackMark: String {
        String(token.symbol.trimmingCharacters(in: .whitespacesAndNewlines).prefix(2)).uppercased()
    }
    var settingsBadgeTint: Color { hostingChain?.settingsIconTint ?? .accentColor }
}
struct TokenRegistryGroup: Identifiable {
    let key: String
    let name: String
    let symbol: String
    let entries: [TokenPreferenceEntry]
    var id: String { key }
    var representativeEntry: TokenPreferenceEntry { entries[0] }
}
/// One token in the known-token list: the wiki's row, so the two libraries read alike.
struct TokenRegistryGroupRowView: View {
    let group: TokenRegistryGroup
    var body: some View {
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: group.representativeEntry.settingsArtworkName,
                fallbackText: group.representativeEntry.settingsFallbackMark,
                color: group.representativeEntry.settingsBadgeTint, size: 36)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(group.name).font(.headline).foregroundStyle(Color.primary).lineLimit(1)
                Text(subtitle).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 0)
            if !group.representativeEntry.isBuiltIn {
                TokenSourceTag(isBuiltIn: false)
            }
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }
        .spectraRowPadding()
    }
    private var subtitle: String {
        let places = group.entries.count == 1
            ? AppLocalization.format("dashboard.asset.onChain", group.representativeEntry.token.chainId.displayName)
            : AppLocalization.format("wiki.asset.onChains", "\(group.entries.count)")
        return "\(group.symbol) · \(places)"
    }
}
/// Whether a token ships with the app or was added by the user.
struct TokenSourceTag: View {
    let isBuiltIn: Bool
    var body: some View {
        Text(AppLocalization.string(isBuiltIn ? "Built-In" : "Custom"))
            .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            .padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.xxs)
            .background(SpectraLayout.insetFill, in: Capsule())
    }
}
/// Core's reason for refusing a token change, as the address book shows its own.
struct TokenPreferenceErrorNotice: View {
    let message: String
    let onDismiss: () -> Void
    var body: some View {
        HStack(alignment: .top, spacing: SpectraLayout.Space.m) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.subheadline.weight(.semibold)).foregroundStyle(.red)
            Text(verbatim: message).font(.subheadline).frame(maxWidth: .infinity, alignment: .leading)
            Button(action: onDismiss) {
                Image(systemName: "xmark").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(AppLocalization.string("Close"))
        }
        .padding(SpectraLayout.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassEffect(.regular.tint(Color.red.opacity(0.12)), in: .rect(cornerRadius: SpectraLayout.Radius.card))
    }
}
/// One network a token lives on: chain and standard, then precision and identifier.
struct TokenRegistryNetworkRow: View {
    let entry: TokenPreferenceEntry
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            HStack(spacing: SpectraLayout.Space.s) {
                Text(entry.token.chainId.displayName).font(.subheadline.weight(.semibold))
                    .foregroundStyle(Color.primary)
                Spacer()
                Text(entry.token.tokenStandard).font(.caption.weight(.semibold)).foregroundStyle(.tint)
                    .padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.xxs)
                    .background(Capsule(style: .continuous).fill(Color.accentColor.opacity(0.12)))
            }
            LabeledContent(AppLocalization.string("Supported Decimals"), value: "\(entry.token.decimals)")
                .font(.footnote).foregroundStyle(.secondary)
            if !entry.token.contract.isEmpty {
                Text(entry.token.contract).font(.footnote.monospaced()).foregroundStyle(.secondary)
                    .textSelection(.enabled).lineLimit(2).truncationMode(.middle)
                    .accessibilityLabel(AppLocalization.string("Token Identifier"))
            }
        }
        .padding(.vertical, SpectraLayout.Space.xs)
    }
}
