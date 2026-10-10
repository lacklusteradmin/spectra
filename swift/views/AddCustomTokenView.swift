import Foundation
import SwiftUI

struct AddCustomTokenView: View {
    let tokens: TokenPreferencesState
    var editing: TokenPreferenceEntry? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var selectedChain: Chain? = Chain.tokenHostingChains.first
    /// The protocol the token is added under. Chosen, not guessed, where the
    /// network has several: an ERC-20 and a BEP-20 contract look alike, and a
    /// TRC-10 ID is not a TRC-20 address.
    @State private var selectedStandard: String?
    @State private var symbolInput = ""
    @State private var nameInput = ""
    @State private var identifierInput = ""
    @State private var coingeckoIdInput = ""
    @State private var coinpaprikaIdInput = ""
    @State private var decimalsInput: UInt32 = 6
    @State private var formMessage: String?
    @State private var isSaving = false
    @State private var hasLoaded = false

    /// The protocols the selected network supports, in the catalog's order.
    private var standards: [TokenStandardEntry] { selectedChain?.entry?.tokenStandards ?? [] }

    /// The chosen protocol: the network's only one, or the one picked.
    private var standard: TokenStandardEntry? {
        standards.count == 1 ? standards.first : standards.first { $0.standard == selectedStandard }
    }

    /// What the chosen protocol identifies a token by.
    private var identifierPrompt: String { standard?.identifierPrompt ?? "Token Identifier" }

    /// The places the chosen protocol gives every token, if it fixes them.
    private var fixedDecimals: UInt32? { standard?.fixedDecimals }

    var body: some View {
        Form {
            Section {
                if let editing {
                    // A token keeps the network and protocol it was added under.
                    LabeledContent(AppLocalization.string("Network"), value: editing.token.chainId.displayName)
                    LabeledContent(AppLocalization.string("Standard"), value: editing.token.tokenStandard)
                    Text(editing.token.contract).font(.caption.monospaced()).textSelection(.enabled)
                } else {
                    Picker(AppLocalization.string("Network"), selection: $selectedChain) {
                        Text(AppLocalization.string("Select a chain")).tag(nil as Chain?)
                        ForEach(Chain.tokenHostingChains) { chain in Text(chain.displayName).tag(Optional(chain)) }
                    }
                    if standards.count > 1 {
                        Picker(AppLocalization.string("Standard"), selection: $selectedStandard) {
                            Text(AppLocalization.string("Select a standard")).tag(nil as String?)
                            ForEach(standards, id: \.standard) { entry in
                                Text(verbatim: entry.standard).tag(Optional(entry.standard))
                            }
                        }
                    } else if let only = standards.first {
                        LabeledContent(AppLocalization.string("Standard"), value: only.standard)
                    }
                    TextField(AppLocalization.string(identifierPrompt), text: $identifierInput)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                }
            } header: {
                Text(AppLocalization.string("Network"))
            } footer: {
                if editing == nil, standards.count > 1 {
                    Text(AppLocalization.string("This network has more than one token standard. Choose the one the token was issued under."))
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
                            chain: selectedChain, standard: standard?.standard, symbol: symbolInput, name: nameInput,
                            contractAddress: identifierInput, coingeckoId: coingeckoIdInput,
                            coinpaprikaId: coinpaprikaIdInput, decimals: fixedDecimals ?? decimalsInput, editing: editing)
                        isSaving = false
                        if formMessage == nil { dismiss() }
                    }
                }.disabled(isSaving || selectedChain == nil || (editing == nil && standard == nil))
            }
        }
        // Another network's protocols are not this one's.
        .onChange(of: selectedChain) { _, _ in
            if editing == nil { selectedStandard = nil }
        }
        .onAppear {
            guard !hasLoaded else { return }
            hasLoaded = true
            guard let editing else { return }
            selectedChain = editing.hostingChain
            selectedStandard = editing.token.tokenStandard
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
