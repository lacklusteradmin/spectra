import SwiftUI

// MARK: - TorStatusBadge

/// Compact pill shown in the Settings row and the dashboard toolbar.
struct TorStatusBadge: View {
    let status: TorStatus
    var body: some View {
        HStack(spacing: SpectraLayout.Space.xs) {
            statusDot
            Text(statusLabel).font(.caption.weight(.semibold))
        }
        .foregroundStyle(statusColor)
        .padding(.horizontal, SpectraLayout.Space.s)
        .padding(.vertical, SpectraLayout.Space.xxs)
        .background(statusColor.opacity(0.12), in: Capsule())
    }
    @ViewBuilder private var statusDot: some View {
        switch status {
        case .bootstrapping:
            SpectraLoadingGlyph(size: 10, tint: statusColor)
        default:
            Circle().fill(statusColor).frame(width: 6, height: 6)
        }
    }
    private var statusLabel: String {
        switch status {
        case .stopped:          return AppLocalization.string("Off")
        case .bootstrapping(let p): return p > 0 ? "\(p)%" : AppLocalization.string("Starting")
        case .ready:            return AppLocalization.string("On")
        case .error:            return AppLocalization.string("Error")
        }
    }
    private var statusColor: Color {
        switch status {
        case .stopped:          return .secondary
        case .bootstrapping:    return .spectraWarning
        case .ready:            return .green
        case .error:            return .red
        }
    }
}

struct TorSettingsView: View {
    @Bindable var store: AppState
    @State private var editingProxyAddress: String = ""
    @FocusState private var proxyFieldFocused: Bool
    var body: some View {
        Form {
            torMainSection
            if store.appSettings.torEnabled { connectionModeSection }
            // What the built-in client does; a proxy of the user's is theirs
            // to vouch for.
            if !store.appSettings.torUseCustomProxy { aboutSection }
        }
        .navigationTitle(AppLocalization.string("Tor Network"))
        .toolbarBackground(.hidden, for: .navigationBar)
        .onAppear { editingProxyAddress = store.appSettings.torCustomProxyAddress }
    }

    // MARK: Sections

    private var torMainSection: some View {
        Section {
            Toggle(isOn: store.settingBinding(\.torEnabled) { .torEnabled(value: $0) }) {
                Label(AppLocalization.string("Enable Tor"), systemImage: "network.badge.shield.half.filled")
            }
            statusRow
            if case .error = store.tor.status {
                reconnectButton
            }
        } header: {
            Text(AppLocalization.string("Tor Network"))
        } footer: {
            Text(AppLocalization.string("tor.footer"))
        }
    }

    @ViewBuilder private var statusRow: some View {
        HStack {
            Text(AppLocalization.string("Status"))
            Spacer()
            TorStatusBadge(status: store.tor.status)
        }
        if case .bootstrapping(let pct) = store.tor.status {
            ProgressView(value: Double(pct), total: 100)
                .animation(.easeInOut, value: pct)
        }
        if case .error(let msg) = store.tor.status {
            Text(msg).font(.caption).foregroundStyle(.red).lineLimit(3)
        }
    }

    private var reconnectButton: some View {
        Button {
            store.tor.reconnect()
        } label: {
            Label(AppLocalization.string("Reconnect"), systemImage: "arrow.trianglehead.2.clockwise")
        }
    }

    private var connectionModeSection: some View {
        Section {
            Toggle(isOn: store.settingBinding(\.torUseCustomProxy) { .torUseCustomProxy(value: $0) }) {
                Label(AppLocalization.string("Use Custom SOCKS5 Proxy"), systemImage: "person.2.wave.2")
            }
            if store.appSettings.torUseCustomProxy {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                    Text(AppLocalization.string("SOCKS5 Address")).font(.footnote).foregroundStyle(.secondary)
                    TextField("socks5://127.0.0.1:9150", text: $editingProxyAddress)
                        .keyboardType(.URL)
                        .autocorrectionDisabled()
                        .textInputAutocapitalization(.never)
                        .focused($proxyFieldFocused)
                        .onSubmit { applyProxyAddress() }
                    if editingProxyAddress != store.appSettings.torCustomProxyAddress {
                        Button(AppLocalization.string("Apply")) { applyProxyAddress() }
                            .font(.footnote.weight(.semibold))
                    }
                }
            }
        } header: {
            Text(AppLocalization.string("Connection Mode"))
        } footer: {
            Text(
                store.appSettings.torUseCustomProxy
                    ? AppLocalization.string("Points all traffic at your own SOCKS5 proxy (e.g. Orbot on port 9150). Arti is not started.")
                    : AppLocalization.string("Uses the built-in Arti Tor client. No external app required.")
            )
        }
    }

    private var aboutSection: some View {
        Section(AppLocalization.string("About")) {
            LabeledContent(AppLocalization.string("Tor client"), value: AppLocalization.string("tor.client.arti"))
            LabeledContent(AppLocalization.string("Stream isolation"), value: AppLocalization.string("Per connection"))
            LabeledContent(AppLocalization.string("Onion routing hops"), value: "3")
        }
    }

    // MARK: Helpers

    /// Core parses the address and refuses one that is not a SOCKS5 URL; a
    /// stored change reconnects the proxy (see `reactToSettingsChange`).
    private func applyProxyAddress() {
        proxyFieldFocused = false
        let trimmed = editingProxyAddress.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return }
        store.updateSetting(.torCustomProxyAddress(value: trimmed))
    }
}
