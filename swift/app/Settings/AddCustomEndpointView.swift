import SwiftUI

struct AddCustomEndpointView: View {
    let store: AppState
    /// The network the form opens on; the first that takes an endpoint when
    /// `nil`.
    var initialChain: Chain?
    /// The custom endpoint being edited, when the form opens on one: its
    /// network stays, and saving changes it where it stands.
    var editing: EndpointDirectoryEntry? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var chain: Chain?
    @State private var api = ""
    @State private var url = ""
    @State private var capabilities: Set<EndpointCapability> = []
    @State private var errorMessage: String?
    @State private var isSaving = false
    private let copy = EndpointsContentCopy.current
    /// Every network that takes an endpoint, in catalog order — including one
    /// with no built-in provider, which is the network that most needs one.
    private var networks: [Chain] { Chain.all.filter { !endpointApiOptions(chain: $0).isEmpty } }
    private var options: [EndpointApiOption] { chain.map { endpointApiOptions(chain: $0) } ?? [] }
    private var types: [String] { options.map(\.id) }
    private var capabilityOptions: [EndpointCapability] { options.first { $0.id == api }?.capabilities ?? [] }
    var body: some View {
        Form {
            Section {
                Picker(AppLocalization.string("Network"), selection: $chain) {
                    ForEach(networks, id: \.self) { network in
                        Text(network.displayName).tag(Optional(network))
                    }
                }
                .disabled(editing != nil)
                Picker(copy.typeTitle, selection: $api) {
                    ForEach(types, id: \.self) { Text($0).tag($0) }
                }
                TextField(copy.urlPlaceholder, text: $url)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL)
            }
            Section {
                ForEach(capabilityOptions, id: \.self) { capability in
                    let id = endpointCapabilityId(capability: capability)
                    Toggle(isOn: Binding(
                        get: { capabilities.contains(capability) },
                        set: { if $0 { capabilities.insert(capability) } else { capabilities.remove(capability) } }
                    )) {
                        VStack(alignment: .leading) {
                            Text(AppLocalization.string("endpointCapability.\(id)"))
                            Text(AppLocalization.string("endpointCapabilityDescription.\(id)"))
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            } header: {
                Text(AppLocalization.string("Endpoint capabilities"))
            } footer: {
                Text(AppLocalization.string(capabilityOptions.isEmpty
                    ? "This API has no supported custom endpoint operations."
                    : "Select the operations enabled on this endpoint. These are your declarations, not verified probe results."))
            }
            if let errorMessage { Section { Text(errorMessage).foregroundStyle(.red) } }
        }
        .navigationTitle(editing == nil ? copy.addEndpointTitle : AppLocalization.string("Edit Endpoint"))
        .navigationBarTitleDisplayMode(.inline)
        .disabled(isSaving)
        .onAppear {
            guard chain == nil else { return }
            if let editing {
                chain = editing.record.chainId
                api = endpointApiOptions(chain: editing.record.chainId).first { $0.api == editing.record.api }?.id ?? ""
                url = editing.record.endpoint
                capabilities = Set(editing.record.capabilities)
            } else {
                chain = initialChain ?? networks.first
                api = types.first ?? ""
            }
        }
        // A change the user makes clears what was declared for the old API;
        // the form filling itself in on appear does not.
        .onChange(of: chain) { old, _ in
            guard old != nil else { return }
            capabilities.removeAll()
            if !types.contains(api) { api = types.first ?? "" }
        }
        .onChange(of: api) { old, _ in
            if !old.isEmpty { capabilities.removeAll() }
        }
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button(AppLocalization.string("Save")) {
                    guard let chain else { return }
                    isSaving = true
                    Task { @MainActor in
                        do {
                            let update: AppSettingUpdate = if let editing {
                                .replaceCustomEndpoint(
                                    chainId: chain, endpoint: editing.record.endpoint, api: api, newEndpoint: url,
                                    capabilities: Array(capabilities))
                            } else {
                                .addCustomEndpoint(capabilities: Array(capabilities), chainId: chain, api: api, endpoint: url)
                            }
                            let transition = try await store.stateCommands.apply(.setAppSetting(update: update))
                            if transition.events.contains(where: { if case .appSettingRejected = $0 { return true }; return false }) {
                                errorMessage = copy.invalidEndpointMessage
                            } else {
                                dismiss()
                            }
                        } catch { errorMessage = userErrorMessage(error) }
                        isSaving = false
                    }
                }.disabled(url.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || api.isEmpty || capabilities.isEmpty || isSaving)
            }
        }
    }
}
