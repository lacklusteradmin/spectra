import Foundation
import SwiftUI

struct TokenRegistryDetailView: View {
    let tokens: TokenPreferencesState
    let groupKey: String
    @State private var isShowingRemoveConfirmation = false
    @Environment(\.dismiss) private var dismiss
    private var groupEntries: [TokenPreferenceEntry] {
        tokens.entries.filter { $0.token.tokenId == groupKey }
    }
    var body: some View {
        Group {
            if let entry = groupEntries.first {
                ScrollView(showsIndicators: false) {
                    LazyVStack(spacing: SpectraLayout.sectionSpacing) {
                        heroCard(entry)
                        if let error = tokens.error {
                            TokenPreferenceErrorNotice(message: error) { tokens.error = nil }
                        }
                        spectraDetailCard(title: "Price Sources") {
                            providerRow("CoinGecko", id: entry.token.coingeckoId)
                            Divider().opacity(0.4)
                            providerRow("CoinPaprika", id: entry.token.coinpaprikaId)
                        }
                        networksCard
                        if !entry.isBuiltIn {
                            Button(role: .destructive) {
                                isShowingRemoveConfirmation = true
                            } label: {
                                Label(AppLocalization.string("Remove Token"), systemImage: "trash")
                                    .frame(maxWidth: .infinity)
                            }
                            .buttonStyle(.glass).tint(.red).controlSize(.large)
                        }
                    }
                    .spectraScreenPadding()
                }
                .background(SpectraBackdrop().ignoresSafeArea())
                .toolbarBackground(.hidden, for: .navigationBar)
                .navigationTitle(entry.token.symbol)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    if !entry.isBuiltIn {
                        ToolbarItem(placement: .topBarTrailing) {
                            NavigationLink(AppLocalization.string("Edit")) {
                                AddCustomTokenView(tokens: tokens, editing: entry)
                            }
                        }
                    }
                }
                .confirmationDialog(AppLocalization.string("Remove Token"), isPresented: $isShowingRemoveConfirmation,
                    titleVisibility: .visible) {
                    Button(AppLocalization.string("Remove"), role: .destructive) { tokens.removeCustom(entry) }
                    Button(AppLocalization.string("Cancel"), role: .cancel) {}
                } message: {
                    Text(AppLocalization.string("This custom token will be removed and will no longer appear in your portfolio."))
                }
                .onAppear { tokens.error = nil }
            } else {
                ContentUnavailableView(AppLocalization.string("Token Not Found"), systemImage: "questionmark.circle")
            }
        }
        .onChange(of: groupEntries.isEmpty) { _, empty in
            if empty { dismiss() }
        }
    }

    private func heroCard(_ entry: TokenPreferenceEntry) -> some View {
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(artworkName: entry.settingsArtworkName, fallbackText: entry.settingsFallbackMark,
                color: entry.settingsBadgeTint, size: 52)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(entry.token.name).font(.title3.weight(.semibold))
                Text(entry.token.symbol).font(.subheadline.monospaced()).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            TokenSourceTag(isBuiltIn: entry.isBuiltIn)
        }
        .padding(SpectraLayout.cardPadding).frame(maxWidth: .infinity, alignment: .leading)
        .spectraElevatedFill()
    }

    private var networksCard: some View {
        spectraDetailCard(title: "Networks") {
            ForEach(Array(groupEntries.enumerated()), id: \.element.id) { index, entry in
                TokenRegistryNetworkRow(entry: entry)
                if index < groupEntries.count - 1 { Divider().opacity(0.4) }
            }
        }
    }

    private func providerRow(_ name: String, id: String) -> some View {
        LabeledContent(name) {
            Text(id.isEmpty ? AppLocalization.string("Not Configured") : id)
                .foregroundStyle(id.isEmpty ? .secondary : .primary).textSelection(.enabled)
                .lineLimit(1).truncationMode(.middle)
        }
        .font(.subheadline)
    }
}
