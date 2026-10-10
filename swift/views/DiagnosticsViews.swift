import Foundation
import SwiftUI
import UniformTypeIdentifiers
struct DiagnosticsHubView: View {
    let store: AppState
    @State private var isCheckingAllEndpoints = false
    @State private var diagnosticsNotice: String?
    @State private var isShowingDiagnosticsImporter = false
    @State private var isShowingDiagnosticsExportsBrowser = false
    @State private var lastExportedDiagnosticsURL: URL?
    @State private var searchText: String = ""
    private let copy = DiagnosticsContentCopy.current
    private struct DiagnosticsDestination: Identifiable {
        let id: String
        let title: String
        let keywords: [String]
        let chain: Chain
    }
    /// Every mainnet has a diagnostics screen.
    private var chainDestinations: [DiagnosticsDestination] {
        Chain.mainnets.map { chain in
            DiagnosticsDestination(
                id: chain.id,
                title: AppLocalization.format("%@ Diagnostics", chain.displayName),
                keywords: chain.searchKeywords, chain: chain)
        }
    }
    private func filteredDestinations(_ destinations: [DiagnosticsDestination]) -> [DiagnosticsDestination] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return destinations }
        return destinations.filter { destination in
            destination.title.localizedCaseInsensitiveContains(query)
                || destination.keywords.contains(where: { $0.localizedCaseInsensitiveContains(query) })
        }
    }
    @ViewBuilder
    private func destinationSection(_ title: String, destinations: [DiagnosticsDestination]) -> some View {
        Section(title) {
            ForEach(filteredDestinations(destinations)) { destination in
                NavigationLink {
                    StandardChainDiagnosticsView(store: store, chain: destination.chain)
                } label: {
                    Text(destination.title)
                }
            }
        }
    }
    var body: some View {
        Form {
            Section(copy.actionsSectionTitle) {
                Button(AppLocalization.string(isCheckingAllEndpoints ? "Running Diagnostics..." : "Run All Endpoint Checks")) {
                    isCheckingAllEndpoints = true
                    Task {
                        for chain in Chain.mainnets { await store.runEndpointDiagnostics(for: chain) }
                        isCheckingAllEndpoints = false
                        diagnosticsNotice = AppLocalization.string("Endpoint checks completed.")
                    }
                }.disabled(isCheckingAllEndpoints)
            }
            destinationSection(copy.chainsSectionTitle, destinations: chainDestinations)
            Section(AppLocalization.string("Diagnostics Bundle")) {
                Button(AppLocalization.string("Export Diagnostics Bundle")) {
                    Task {
                        do {
                            let url = try await store.exportDiagnosticsBundle()
                            lastExportedDiagnosticsURL = url
                            diagnosticsNotice = AppLocalization.format("Diagnostics exported to %@", url.lastPathComponent)
                        } catch {
                            diagnosticsNotice = AppLocalization.format("Export failed: %@", userErrorMessage(error))
                        }
                    }
                }
                Button(AppLocalization.string("Past Exports")) {
                    isShowingDiagnosticsExportsBrowser = true
                }
                if let lastExportedDiagnosticsURL {
                    ShareLink(item: lastExportedDiagnosticsURL) {
                        Label(AppLocalization.string("Share Last Export"), systemImage: "square.and.arrow.up")
                    }
                }
                Button(AppLocalization.string("Import Diagnostics Bundle")) {
                    isShowingDiagnosticsImporter = true
                }
            }
            if let diagnosticsNotice {
                Section {
                    Text(diagnosticsNotice).font(.caption).foregroundStyle(.secondary)
                }
            }
        }.navigationTitle(copy.navigationTitle).navigationBarTitleDisplayMode(.inline).searchable(
            text: $searchText, prompt: copy.searchPrompt).sheet(isPresented: $isShowingDiagnosticsExportsBrowser) {
            DiagnosticsExportsBrowserView(store: store)
        }.fileImporter(
            isPresented: $isShowingDiagnosticsImporter, allowedContentTypes: [UTType.json], allowsMultipleSelection: false
        ) { result in
            do {
                guard let fileURL = try result.get().first else { return }
                let didAccess = fileURL.startAccessingSecurityScopedResource()
                defer {
                    if didAccess { fileURL.stopAccessingSecurityScopedResource() }
                }
                let payload = try store.importDiagnosticsBundle(from: fileURL)
                diagnosticsNotice = AppLocalization.format(
                    "Imported diagnostics bundle (%@).", payload.generatedAtDate.appFormatted(date: .abbreviated, time: .shortened))
            } catch {
                diagnosticsNotice = AppLocalization.format("Import failed: %@", userErrorMessage(error))
            }
        }
    }
}
struct StandardChainDiagnosticsView: View {
    @Bindable var store: AppState
    let chain: Chain
    private let copy = DiagnosticsContentCopy.current
    @State private var isRefreshing = false
    @State private var refreshNotice: String?
    @State private var copiedDiagnosticsNotice: SpectraTransientNotice?
    /// What core recorded for this family: history runs, endpoint checks and
    /// the document built from them. Re-read whenever a run finishes.
    @State private var recorded: ChainDiagnostics?
    @State private var recordedError: String?
    @State private var keypoolError: String?
    @State private var cachedKeypoolDiagnostics: [KeypoolDiagnostic] = []
    @State private var cachedOperationalEvents: [DiagnosticLog] = []
    private var runs: WalletChainDiagnosticsState { store.chainDiagnosticsState }
    private var displayChainTitle: String { chain.displayName }
    private var diagnosticsLabel: String { displayChainTitle }

    var body: some View {
        Form {
            Section(copy.actionsSectionTitle) {
                Button(AppLocalization.string(isRefreshing ? "Refreshing..." : "Refresh Balances and History")) {
                    isRefreshing = true
                    refreshNotice = nil
                    Task {
                        let succeeded = await store.performUserInitiatedRefresh(forChain: chain)
                        isRefreshing = false
                        refreshNotice = refreshOutcomeMessage(succeeded: succeeded)
                    }
                }.disabled(isRefreshing)
                if let refreshNotice {
                    Text(refreshNotice).font(.caption).foregroundStyle(.secondary)
                }
                Button(
                    isRunningHistory
                        ? AppLocalization.string("Running History Diagnostics...")
                        : AppLocalization.string("Run History Diagnostics")
                ) {
                    Task { await store.runHistoryDiagnostics(for: chain) }
                }.disabled(isRunningHistory)
                Button(AppLocalization.string("Copy Diagnostics JSON")) {
                    if let document = recorded?.document {
                        UIPasteboard.general.string = document
                        copiedDiagnosticsNotice = SpectraTransientNotice(
                            AppLocalization.format("%@ diagnostics JSON copied.", diagnosticsLabel))
                    } else {
                        copiedDiagnosticsNotice = SpectraTransientNotice(
                            AppLocalization.format("No %@ diagnostics available to copy.", diagnosticsLabel))
                    }
                }
                Button(
                    isCheckingEndpoints
                        ? AppLocalization.string("Checking Endpoints...")
                        : AppLocalization.string("Check Endpoints")
                ) {
                    Task { await store.runEndpointDiagnostics(for: chain) }
                }.disabled(isCheckingEndpoints)
                Button(isRunningChainSelfTests ? AppLocalization.string("Running Self-Tests...") : AppLocalization.string("Run Self-Tests")) {
                    Task { await store.runSelfTests(for: chain) }
                }.disabled(isRunningChainSelfTests)
                if chain.usesAccountUTXO {
                    Button(AppLocalization.string(isRunningChainRescan ? "Rescanning..." : "Run Rescan")) {
                        Task { await store.runUTXORescan(chain: chain) }
                    }.disabled(isRunningChainRescan)
                }
                if let copiedDiagnosticsNotice {
                    Text(copiedDiagnosticsNotice.text).font(.caption).foregroundStyle(.secondary)
                }
            }
            Section(copy.statusSectionTitle) {
                if let recordedError {
                    Text(recordedError).font(.caption).foregroundStyle(.red)
                }
                if let ranAt = recorded?.historyRunAtUnix {
                    Text(formatCopy(copy.lastHistoryRunFormat, formattedTime(ranAt))).font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    Text(copy.historyNotRunYet).font(.caption).foregroundStyle(.secondary)
                }
                Text(formatCopy(copy.walletDiagnosticsCoveredFormat, String(recorded?.walletCount ?? 0))).font(.caption)
                    .foregroundStyle(.secondary)
                if let primarySource = historySources.first {
                    Text(formatCopy(copy.mostUsedHistorySourceFormat, primarySource.source, String(primarySource.walletCount)))
                        .font(.caption).foregroundStyle(.secondary)
                }
                if let checkedAt = recorded?.endpointsCheckedAtUnix {
                    Text(formatCopy(copy.lastEndpointCheckFormat, formattedTime(checkedAt))).font(.caption).foregroundStyle(.secondary)
                }
                if !endpoints.isEmpty {
                    let reachableCount = endpoints.filter { $0.checked && $0.reachable }.count
                    Text(formatCopy(copy.endpointHealthFormat, String(reachableCount), String(endpoints.count))).font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Section(formatCopy(copy.historySourcesSectionTitleFormat, diagnosticsLabel)) {
                if historySources.isEmpty {
                    Text(copy.noHistoryTelemetryYet).font(.caption).foregroundStyle(.secondary)
                } else {
                    ForEach(historySources, id: \.source) { item in
                        HStack {
                            Text(item.source).font(.subheadline.weight(.semibold))
                            Spacer()
                            Text(AppLocalization.format("diagnostics.countOnly", Int(item.walletCount))).font(.caption.monospacedDigit())
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
            Section(formatCopy(copy.endpointReachabilitySectionTitleFormat, diagnosticsLabel)) {
                if endpoints.isEmpty {
                    Text(copy.noEndpointChecksYet).font(.caption).foregroundStyle(.secondary)
                } else {
                    ForEach(Array(endpoints.enumerated()), id: \.offset) { _, result in
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                            HStack {
                                Image(systemName: endpointStatusIconName(for: result)).foregroundStyle(endpointStatusColor(for: result))
                                Text(result.endpoint).font(.subheadline.weight(.semibold))
                            }
                            Text(result.detail).font(.caption).foregroundStyle(.secondary)
                        }.padding(.vertical, SpectraLayout.Space.xxs)
                    }
                }
            }
            chainSpecificSections
        }.navigationTitle(AppLocalization.format("%@ Diagnostics", displayChainTitle))
        .task(id: chain) {
            do {
                let diagnostics = try await store.chainKeypoolDiagnostics(for: chain)
                guard !Task.isCancelled else { return }
                cachedKeypoolDiagnostics = diagnostics
                keypoolError = nil
            } catch {
                guard !Task.isCancelled else { return }
                cachedKeypoolDiagnostics = []
                keypoolError = userErrorMessage(error)
            }
        }.task(id: runs.diagnosticsRevision) {
            do {
                let diagnostics = try await store.chainDiagnostics(for: chain)
                guard !Task.isCancelled else { return }
                recorded = diagnostics
                recordedError = nil
            } catch {
                guard !Task.isCancelled else { return }
                recordedError = userErrorMessage(error)
            }
            let events = await store.operationalEvents(for: chain)
            guard !Task.isCancelled else { return }
            cachedOperationalEvents = events
        }.spectraTransientNotice($copiedDiagnosticsNotice)
    }
    private var isRunningHistory: Bool { runs.runningHistory.contains(chain) }
    private var isCheckingEndpoints: Bool { runs.checkingEndpoints.contains(chain) }
    private var isRunningChainSelfTests: Bool { runs.runningSelfTests.contains(chain) }
    private var isRunningChainRescan: Bool { runs.runningRescans.contains(chain) }
    private var historySources: [DiagnosticsSourceCount] { recorded?.historySources ?? [] }
    private var endpoints: [EndpointProbe] { recorded?.endpoints ?? [] }
    private func formattedTime(_ unix: Double) -> String {
        Date(timeIntervalSince1970: unix).appFormatted(date: .abbreviated, time: .shortened)
    }
    /// An endpoint nothing knows how to probe is unchecked, not a pass.
    private func endpointStatusIconName(for row: EndpointProbe) -> String {
        guard row.checked else { return "clock.badge.questionmark" }
        return row.reachable ? "checkmark.circle.fill" : "xmark.circle.fill"
    }
    private func endpointStatusColor(for row: EndpointProbe) -> Color {
        guard row.checked else { return .secondary }
        return row.reachable ? .green : .red
    }
    @ViewBuilder
    private var chainSpecificSections: some View {
        Section {
            NavigationLink { EndpointCatalogSettingsView(store: store) } label: {
                Text(EndpointsContentCopy.current.navigationTitle)
            }
        }
        Section(AppLocalization.string("Operational Events")) {
            let events = cachedOperationalEvents
            if events.isEmpty {
                Text(AppLocalization.string("No operational events recorded yet.")).font(.caption).foregroundStyle(.secondary)
            } else {
                ForEach(events.prefix(20)) { log in
                    let event = log.input
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                        Text(event.message).font(.subheadline)
                        Text(event.level.displayName).font(.caption.weight(.semibold)).foregroundStyle(
                            event.level == .error ? .red : (event.level == .warning ? .spectraWarning : .secondary))
                        if let transactionHash = event.transactionHash, !transactionHash.isEmpty {
                            Text(transactionHash).font(.caption.monospaced()).foregroundStyle(.secondary)
                        }
                    }.padding(.vertical, SpectraLayout.Space.xxs)
                }
            }
        }
        Section(AppLocalization.string("Owned Address Management")) {
            let diagnostics = cachedKeypoolDiagnostics
            if let keypoolError {
                Text(keypoolError).font(.caption).foregroundStyle(.red)
            } else if diagnostics.isEmpty {
                Text(AppLocalization.string("No owned-address management state recorded yet.")).font(.caption).foregroundStyle(.secondary)
            } else {
                ForEach(diagnostics) { item in
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                        Text(item.walletName).font(.subheadline.weight(.semibold))
                        Text(AppLocalization.format("Next receive index: %lld", Int(item.keypool.nextExternalIndex))).font(.caption).foregroundStyle(.secondary)
                        Text(AppLocalization.format("Next change index: %lld", Int(item.keypool.nextChangeIndex))).font(.caption).foregroundStyle(.secondary)
                        if let reservedReceiveIndex = item.keypool.reservedReceiveIndex {
                            Text(AppLocalization.format("Reserved receive index: %lld", Int(reservedReceiveIndex))).font(.caption).foregroundStyle(.secondary)
                        }
                        if let reservedReceivePath = item.reservedReceive?.derivationPath, !reservedReceivePath.isEmpty {
                            Text(reservedReceivePath).font(.caption.monospaced()).foregroundStyle(.secondary)
                        }
                        if let reservedReceiveAddress = item.reservedReceive?.address, !reservedReceiveAddress.isEmpty {
                            Text(reservedReceiveAddress).font(.caption.monospaced()).foregroundStyle(.secondary)
                        }
                    }.padding(.vertical, SpectraLayout.Space.xxs)
                }
            }
        }
    }
}
private func formatCopy(_ format: String, _ arguments: CVarArg...) -> String {
    String(format: format, locale: AppLocalization.locale, arguments: arguments)
}

func refreshOutcomeMessage(succeeded: Bool) -> String {
    AppLocalization.string(succeeded ? "Manual refresh completed." : "Refresh failed or completed partially. See refresh errors.")
}
