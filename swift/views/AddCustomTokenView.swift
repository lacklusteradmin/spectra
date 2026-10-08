import Foundation
import SwiftUI

struct AddCustomTokenView: View {
    let tokens: TokenPreferencesState
    var editing: TokenPreferenceEntry? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var selectedChain: Chain? = Chain.tokenHostingChains.first
    @State private var symbolInput = ""
    @State private var nameInput = ""
    @State private var identifierInput = ""
    @State private var coingeckoIdInput = ""
    @State private var coinpaprikaIdInput = ""
    @State private var decimalsInput: UInt32 = 6
    @State private var formMessage: String?
    @State private var isSaving = false
    @State private var hasLoaded = false

    /// What the selected network calls a token's identifier.
    private var identifierPrompt: String {
        let prompt = selectedChain?.entry?.contractAddressPrompt ?? ""
        return prompt.isEmpty ? "Token Identifier" : prompt
    }

    /// The places the selected network's protocol gives every token, if it fixes them.
    private var fixedDecimals: UInt32? { selectedChain?.entry?.fixedTokenDecimals }

    var body: some View {
        Form {
            Section(AppLocalization.string("Network")) {
                if let editing {
                    LabeledContent(AppLocalization.string("Network"), value: editing.token.chainId.displayName)
                    Text(editing.token.contract).font(.caption.monospaced()).textSelection(.enabled)
                } else {
                    Picker(AppLocalization.string("Network"), selection: $selectedChain) {
                        Text(AppLocalization.string("Select a chain")).tag(nil as Chain?)
                        ForEach(Chain.tokenHostingChains) { chain in Text(chain.displayName).tag(Optional(chain)) }
                    }
                    TextField(AppLocalization.string(identifierPrompt), text: $identifierInput)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                }
            }
            Section(AppLocalization.string("Token Details")) {
                TextField(AppLocalization.string("Name"), text: $nameInput)
                TextField(AppLocalization.string("Symbol"), text: $symbolInput)
                    .textInputAutocapitalization(.characters).autocorrectionDisabled()
                if let fixed = fixedDecimals {
                    // The network's protocol fixes the places; there is nothing to choose.
                    Text(AppLocalization.format("Token Supports: %lld decimals", count: Int(fixed), Int(fixed)))
                } else {
                    Stepper(AppLocalization.format("Token Supports: %lld decimals", count: Int(decimalsInput), Int(decimalsInput)), value: $decimalsInput, in: 0...CoreReferenceTables.bounds.maxTokenDecimals)
                }
            }
            Section {
                TextField(AppLocalization.string("CoinGecko ID (Optional)"), text: $coingeckoIdInput)
                TextField(AppLocalization.string("CoinPaprika ID (Optional)"), text: $coinpaprikaIdInput)
            } header: {
                Text(AppLocalization.string("Price Sources"))
            } footer: {
                Text(AppLocalization.string("Both price sources are optional. Use the provider's token ID, not its website URL."))
            }.textInputAutocapitalization(.never).autocorrectionDisabled()
            if let formMessage {
                Section { Text(formMessage).foregroundStyle(.red) }
            }
        }
        .navigationTitle(AppLocalization.string(editing == nil ? "New Token" : "Edit Token"))
        .navigationBarTitleDisplayMode(.inline)
        .disabled(isSaving)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button(AppLocalization.string("Save")) {
                    guard let selectedChain else { return }
                    isSaving = true
                    Task { @MainActor in
                        formMessage = await tokens.addCustom(
                            chain: selectedChain, symbol: symbolInput, name: nameInput,
                            contractAddress: identifierInput, coingeckoId: coingeckoIdInput,
                            coinpaprikaId: coinpaprikaIdInput, decimals: fixedDecimals ?? decimalsInput, editing: editing)
                        isSaving = false
                        if formMessage == nil { dismiss() }
                    }
                }.disabled(isSaving || selectedChain == nil)
            }
        }
        .onAppear {
            guard !hasLoaded else { return }
            hasLoaded = true
            guard let editing else { return }
            selectedChain = editing.hostingChain
            if selectedChain == nil { formMessage = AppLocalization.string("That network cannot hold tokens.") }
            symbolInput = editing.token.symbol
            nameInput = editing.token.name
            identifierInput = editing.token.contract
            coingeckoIdInput = editing.token.coingeckoId
            coinpaprikaIdInput = editing.token.coinpaprikaId
            decimalsInput = editing.token.decimals
        }
    }
}
