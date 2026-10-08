import SwiftUI

/// The setup's last page: what a wallet on the network will do once added,
/// its limits, and the endpoints its first refresh reads from — all core's —
/// with the way to change those endpoints before the address goes anywhere.
struct WalletSetupSummaryCard: View {
    let store: AppState
    let chain: Chain
    @State private var summary: WalletSetupSummary?
    @State private var isAddingEndpoint = false

    var body: some View {
        // A container that is always there: an empty conditional is no view,
        // and its task would never start.
        VStack(spacing: 0) {
            if let summary {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    Text(AppLocalization.string("What This Wallet Does")).font(.headline)
                    capabilityRow(AppLocalization.string("Balance"), summary.balance)
                    capabilityRow(AppLocalization.string("History"), summary.history)
                    if let tokens = summary.tokenDiscovery {
                        capabilityRow(AppLocalization.string("Token Discovery"), tokens)
                    }
                    HStack {
                        Text(AppLocalization.string("Staking")).font(.subheadline)
                        Spacer()
                        Text(AppLocalization.string(summary.staking ? "Available" : "Not available"))
                            .font(.subheadline).foregroundStyle(.secondary)
                    }
                    ForEach(summary.limits, id: \.self) { limit in
                        Label(limit.explanation, systemImage: "info.circle").font(.caption).foregroundStyle(.secondary)
                    }
                    Divider().opacity(0.4)
                    endpointsSection(summary)
                }
                .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
            }
        }
        // Read again whenever settings change: an endpoint added, or the
        // switch below, changes whom the first refresh asks.
        .task(id: store.appSettings) { await reload() }
        .sheet(isPresented: $isAddingEndpoint, onDismiss: { Task { await reload() } }) {
            NavigationStack {
                AddCustomEndpointView(store: store, initialChain: chain)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button(AppLocalization.string("Cancel")) { isAddingEndpoint = false }
                        }
                    }
            }
        }
    }

    private func reload() async {
        summary = try? await store.bridge.ready().walletSetupSummary(chain: chain)
    }

    /// Committed before the summary is read again, so the list shown is the
    /// one core now holds.
    private func setCustomEndpointsOnly(_ value: Bool) async {
        do {
            _ = try await store.stateCommands.apply(
                .setAppSetting(update: .customEndpointsOnly(chainId: chain, value: value)))
        } catch {
            store.reportCommandError(error)
        }
        await reload()
    }

    private func capabilityRow(_ title: String, _ coverage: CapabilityCoverage) -> some View {
        HStack {
            Text(title).font(.subheadline)
            Spacer()
            Text(coverage.title).font(.subheadline)
                .foregroundStyle(coverage == .configured ? AnyShapeStyle(.secondary) : AnyShapeStyle(Color.spectraWarning))
        }
    }

    @ViewBuilder
    private func endpointsSection(_ summary: WalletSetupSummary) -> some View {
        Text(AppLocalization.string("Its first refresh reads from")).font(.subheadline.weight(.semibold))
        if summary.endpoints.isEmpty {
            Text(AppLocalization.string("No endpoint yet: nothing is read until you add one."))
                .font(.caption).foregroundStyle(.secondary)
        }
        ForEach(summary.endpoints, id: \.endpoint) { endpoint in
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(endpoint.endpoint).font(.caption.monospaced()).textSelection(.enabled).lineLimit(2)
                Text(AppLocalization.string(endpoint.isBuiltIn ? "Built-In" : "Custom"))
                    .font(.caption2).foregroundStyle(.secondary)
            }
        }
        Toggle(isOn: Binding(
            get: { summary.customEndpointsOnly },
            set: { value in Task { await setCustomEndpointsOnly(value) } }
        )) {
            Text(AppLocalization.string("Use only my endpoints")).font(.subheadline)
        }
        Button(AppLocalization.string("Add Endpoint")) { isAddingEndpoint = true }
            .font(.subheadline.weight(.semibold))
    }
}

extension CapabilityCoverage {
    var title: String {
        switch self {
        case .configured: AppLocalization.string("Available")
        case .needsCustomEndpoint: AppLocalization.string("Needs your endpoint")
        case .unavailable: AppLocalization.string("Not available")
        }
    }
}

extension WalletSetupLimit {
    var explanation: String {
        switch self {
        case .singleAddress: AppLocalization.string("One address receives and sends; no fresh receive addresses.")
        case .accountReserve: AppLocalization.string("The account exists only once it holds the network's reserve; a smaller first payment fails.")
        case .tokenBearingInputsUntouched: AppLocalization.string("ADA held beside native tokens cannot pay; only pure-ADA inputs are spent.")
        case .shieldedScan: AppLocalization.string("Shielded funds are found by scanning blocks on this device from the restore height on; only a wallet restored from its seed phrase holds them.")
        case .scansOnDevice: AppLocalization.string("Balance and history come from scanning blocks on this device, from the restore height on.")
        }
    }
}
