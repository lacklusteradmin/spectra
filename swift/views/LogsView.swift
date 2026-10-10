import Foundation
import SwiftUI
import UIKit
struct LogsView: View {
    let store: AppState
    @State private var searchText: String = ""
    /// `nil` shows every level.
    @State private var selectedLevelFilter: DiagnosticLogLevel?
    private let allCategoryFilter = "__all__"
    @State private var selectedCategoryFilter: String = "__all__"
    @State private var copiedNotice: SpectraTransientNotice?
    @State private var isConfirmingClear = false
    @State private var cachedAvailableCategories: [String] = ["__all__"]
    @State private var cachedFilteredLogs: [DiagnosticLog] = []
    private var diagnosticsState: WalletDiagnosticsState { store.diagnostics }
    private var availableCategories: [String] { cachedAvailableCategories }
    private var filteredLogs: [DiagnosticLog] { cachedFilteredLogs }
    private func rebuildLogPresentation() {
        let categories = Set(diagnosticsState.operationalLogs.map { $0.input.category })
        cachedAvailableCategories = [allCategoryFilter] + categories.sorted()
        if selectedCategoryFilter != allCategoryFilter, !cachedAvailableCategories.contains(selectedCategoryFilter) {
            selectedCategoryFilter = allCategoryFilter
        }
        cachedFilteredLogs = diagnosticsState.operationalLogs.filter { log in
            let event = log.input
            let levelMatches = selectedLevelFilter.map { event.level == $0 } ?? true
            let categoryMatches = selectedCategoryFilter == allCategoryFilter || event.category == selectedCategoryFilter
            let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            let searchMatches: Bool
            if query.isEmpty {
                searchMatches = true
            } else {
                let haystack = [
                    event.message, event.category, event.chainName ?? "", event.source ?? "", event.metadata ?? "", event.walletId ?? "",
                    event.transactionHash ?? "",
                ].joined(separator: " ").lowercased()
                searchMatches = haystack.contains(query)
            }
            return levelMatches && categoryMatches && searchMatches
        }
    }
    private var summaryText: String {
        let debugCount = filteredLogs.filter { $0.input.level == .debug }.count
        let infoCount = filteredLogs.filter { $0.input.level == .info }.count
        let warningCount = filteredLogs.filter { $0.input.level == .warning }.count
        let errorCount = filteredLogs.filter { $0.input.level == .error }.count
        return AppLocalization.format(
            "Showing %lld logs • D:%lld I:%lld W:%lld E:%lld", filteredLogs.count, debugCount, infoCount, warningCount, errorCount)
    }
    var body: some View {
        List {
            Section(AppLocalization.string("Status")) {
                Text(store.pendingTransactionRefreshStatusText ?? AppLocalization.string("No refresh status yet")).font(.caption)
                    .foregroundStyle(.secondary)
                Text(store.networkSyncStatusText).font(.caption).foregroundStyle(.secondary)
                Text(summaryText).font(.caption).foregroundStyle(.secondary)
                if let copiedNotice { Text(copiedNotice.text).font(.caption).foregroundStyle(.secondary) }
            }
            Section(AppLocalization.string("Filters")) {
                Picker(AppLocalization.string("Level"), selection: $selectedLevelFilter) {
                    Text(AppLocalization.string("All")).tag(DiagnosticLogLevel?.none)
                    ForEach(DiagnosticLogLevel.allCases, id: \.self) { level in Text(level.displayName).tag(Optional(level)) }
                }
                Picker(AppLocalization.string("Category"), selection: $selectedCategoryFilter) {
                    ForEach(availableCategories, id: \.self) { category in
                        let label: String = category == allCategoryFilter ? AppLocalization.string("All") : category
                        Text(label).tag(category)
                    }
                }
            }
            if filteredLogs.isEmpty {
                Section(AppLocalization.string("Events")) {
                    Text(AppLocalization.string("No operational events yet.")).font(.caption).foregroundStyle(.secondary)
                }
            } else {
                Section(AppLocalization.string("Events")) {
                    ForEach(filteredLogs) { log in
                        let event = log.input
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                            HStack(spacing: SpectraLayout.Space.s) {
                                Image(systemName: iconName(for: event.level)).foregroundStyle(color(for: event.level))
                                Text(log.timestamp.appFormatted(date: .abbreviated, time: .standard)).font(.caption.bold()).foregroundStyle(
                                    .secondary)
                                Text(event.category).font(.caption2.weight(.semibold)).foregroundStyle(.secondary).padding(.horizontal, SpectraLayout.Space.xs)
                                    .padding(.vertical, SpectraLayout.Space.xxs).background(Color.secondary.opacity(0.12), in: Capsule())
                            }
                            Text(event.message).font(.subheadline)
                            if let source = event.source, !source.isEmpty {
                                Text(AppLocalization.format("source: %@", source)).font(.caption.monospaced()).foregroundStyle(.secondary)
                            }
                            if let chainName = event.chainName, !chainName.isEmpty {
                                Text(AppLocalization.format("chain: %@", chainName)).font(.caption.monospaced()).foregroundStyle(
                                    .secondary)
                            }
                            if let walletId = event.walletId {
                                Text(AppLocalization.format("wallet: %@", walletId)).font(.caption.monospaced()).foregroundStyle(
                                    .secondary
                                ).textSelection(.enabled)
                            }
                            if let transactionHash = event.transactionHash, !transactionHash.isEmpty {
                                Text(transactionHash).font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                            }
                            if let metadata = event.metadata, !metadata.isEmpty {
                                Text(metadata).font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                            }
                        }.padding(.vertical, SpectraLayout.Space.xxs)
                    }
                }
            }
        }.navigationTitle(AppLocalization.string("Logs")).searchable(
            text: $searchText, prompt: AppLocalization.string("Search message, chain, tx hash, wallet")
        ).onAppear {
            rebuildLogPresentation()
        }.onChange(of: diagnosticsState.operationalLogsRevision) { _, _ in
            rebuildLogPresentation()
        }.onChange(of: selectedLevelFilter) { _, _ in
            rebuildLogPresentation()
        }.onChange(of: selectedCategoryFilter) { _, _ in
            rebuildLogPresentation()
        }.onChange(of: searchText) { _, _ in
            rebuildLogPresentation()
        }.spectraTransientNotice($copiedNotice).toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(AppLocalization.string("Copy")) {
                    UIPasteboard.general.string = store.exportOperationalLogsText(events: filteredLogs)
                    copiedNotice = SpectraTransientNotice(AppLocalization.format("Copied %lld log entries", count: filteredLogs.count, filteredLogs.count))
                }.disabled(filteredLogs.isEmpty)
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button(AppLocalization.string("Clear"), role: .destructive) {
                    isConfirmingClear = true
                }.disabled(diagnosticsState.operationalLogs.isEmpty)
            }
        }
        // The log is what a problem report is made from; clearing it is not
        // undone.
        .confirmationDialog(
            AppLocalization.string("Clear every log entry?"), isPresented: $isConfirmingClear, titleVisibility: .visible
        ) {
            Button(AppLocalization.string("Clear"), role: .destructive) { diagnosticsState.clearOperationalLogs() }
        }
    }
    private func iconName(for level: DiagnosticLogLevel) -> String {
        switch level {
        case .debug: return "ladybug.fill"
        case .info: return "info.circle.fill"
        case .warning: return "exclamationmark.triangle.fill"
        case .error: return "xmark.octagon.fill"
        }
    }
    private func color(for level: DiagnosticLogLevel) -> Color {
        switch level {
        case .debug: return .gray
        case .info: return .blue
        case .warning: return .spectraWarning
        case .error: return .red
        }
    }
}
