import SwiftUI
private struct HistoryRowPresentation: Identifiable, Equatable {
    let transaction: TransactionRecord
    let amountText: String?
    let amountColor: Color?
    let subtitleText: String
    let fullTimestampText: String
    let metadataText: String?
    var id: String { transaction.id }
}

private struct HistoryTransactionRowView: View, Equatable {
    let row: HistoryRowPresentation
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool { lhs.row == rhs.row }
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            HStack(spacing: SpectraLayout.Space.s) {
                CoinBadge(
                    artworkName: row.transaction.artworkName, fallbackText: row.transaction.symbol,
                    color: row.transaction.badgeColor, size: 36)
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    if let amountText = row.amountText {
                        Text(amountText).font(.headline.weight(.semibold)).foregroundStyle(row.amountColor ?? Color.primary)
                            .spectraNumericTextLayout()
                    }
                    Text(row.subtitleText).spectraHintText().lineLimit(1)
                }
                Spacer()
                VStack(alignment: .trailing, spacing: SpectraLayout.Space.xs) {
                    TransactionStatusBadge(status: row.transaction.status)
                    Text(row.fullTimestampText).font(.caption2).foregroundStyle(.secondary).multilineTextAlignment(
                        .trailing)
                }
                Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
            }
            if let metadataText = row.metadataText {
                Text(metadataText).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
            }
        }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
    }
}
/// Where a row sits in time, newest first. A pending row with no date sorts
/// as the newest: core puts it there.
private enum HistoryDateGroup: CaseIterable {
    case unconfirmed, today, yesterday, older

    init(_ transaction: TransactionRecord, calendar: Calendar) {
        if !transaction.hasKnownDate, transaction.status == .pending {
            self = .unconfirmed
        } else if calendar.isDateInToday(transaction.createdDate) {
            self = .today
        } else if calendar.isDateInYesterday(transaction.createdDate) {
            self = .yesterday
        } else {
            self = .older
        }
    }

    var title: String {
        switch self {
        case .unconfirmed: return AppLocalization.string("Unconfirmed")
        case .today: return AppLocalization.string("Today")
        case .yesterday: return AppLocalization.string("Yesterday")
        case .older: return AppLocalization.string("Older")
        }
    }
}

private struct HistoryPresentationSection: Identifiable {
    let group: HistoryDateGroup
    let rows: [HistoryRowPresentation]
    var id: HistoryDateGroup { group }
    var title: String { group.title }
}
struct HistoryView: View {
    let store: AppState
    @State private var selectedFilter: HistoryQueryFilter = .all
    @State private var selectedSortOrder: HistorySortOrder = .newest
    @State private var selectedWalletId: String?
    @State private var hidesSmallAmounts = false
    @State private var searchText: String = ""
    @State private var pageRecords: [TransactionRecord] = []
    @State private var nextCursor: String?
    @State private var hasMoreStoredHistory = false
    @State private var pageError: String?
    @State private var isLoadingPage = false
    @State private var pageRequestId = UUID()
    @State private var loadedFilterKey: String?
    /// Rows the next reload adds beyond those on screen; cleared once a reload lands.
    @State private var pendingGrowth = 0
    @State private var isRetrying = false
    var body: some View {
        NavigationStack {
            ZStack {
                SpectraBackdrop().ignoresSafeArea()
                ScrollView(showsIndicators: false) {
                    LazyVStack(alignment: .leading, spacing: SpectraLayout.sectionSpacing) {
                        if let error = historyError {
                            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                                Label(AppLocalization.string("Unable to load history"), systemImage: "exclamationmark.triangle")
                                    .font(.headline)
                                Text(error).font(.subheadline).foregroundStyle(.secondary)
                                Button(AppLocalization.string("Retry")) {
                                    isRetrying = true
                                    Task {
                                        await store.refreshTransactionProjection()
                                        await loadPage(reset: true)
                                        await store.performUserInitiatedRefresh()
                                        isRetrying = false
                                    }
                                }.buttonStyle(.glass).disabled(isRetrying)
                            }.padding(SpectraLayout.cardPadding).frame(maxWidth: .infinity, alignment: .leading).spectraCardFill()
                        }
                        if visibleTransactions.isEmpty && historyError == nil {
                            historyEmptyStateCard
                        }
                        ForEach(groupedSections) { section in
                                VStack(spacing: 0) {
                                    HStack {
                                        Text(AppLocalization.format("history.section.titleCount", section.title, section.rows.count))
                                            .font(.subheadline.weight(.semibold)).foregroundStyle(.secondary).textCase(.uppercase)
                                        Spacer()
                                    }.padding(.horizontal, SpectraLayout.rowHorizontal).padding(.vertical, SpectraLayout.cardHeaderVertical)
                                    Divider().opacity(0.25)
                                    VStack(spacing: 0) {
                                        ForEach(Array(section.rows.enumerated()), id: \.element.id) { index, row in
                                            NavigationLink {
                                                TransactionDetailView(store: store, transaction: row.transaction)
                                            } label: {
                                                HistoryTransactionRowView(row: row).equatable()
                                                    .padding(.horizontal, SpectraLayout.rowHorizontal).padding(.vertical, SpectraLayout.rowVertical)
                                            }.buttonStyle(.plain).contextMenu {
                                                if row.transaction.actions.recheckUnavailableReason == nil {
                                                    Button {
                                                        spectraHaptic(.light)
                                                        Task { _ = await store.retryUTXOTransactionStatus(for: row.transaction.id) }
                                                    } label: {
                                                        Label(AppLocalization.string("Recheck"), systemImage: "arrow.clockwise")
                                                    }
                                                }
                                                if row.transaction.actions.rebroadcastUnavailableReason == nil {
                                                    Button {
                                                        spectraHaptic(.light)
                                                        Task { _ = await store.rebroadcastSignedTransaction(for: row.transaction.id) }
                                                    } label: {
                                                        Label(AppLocalization.string("Rebroadcast"), systemImage: "dot.radiowaves.up.forward")
                                                    }
                                                }
                                            }
                                            if index < section.rows.count - 1 { Divider().padding(.leading, SpectraLayout.rowDividerInset).opacity(0.25) }
                                        }
                                    }.padding(.vertical, SpectraLayout.Space.xs)
                                }.frame(maxWidth: .infinity).glassEffect(
                                    .regular.tint(SpectraLayout.GlassTint.content).interactive(),
                                    in: .rect(cornerRadius: SpectraLayout.Radius.card))
                            }
                        if shouldShowPagingControls { historyPagingControls }
                    }.spectraScreenPadding()
                }.refreshable {
                    await store.performUserInitiatedRefresh()
                }.scrollBounceBehavior(.always)
            }.searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always),
                         prompt: AppLocalization.string("Search wallet, asset, symbol, or address"))
            .textInputAutocapitalization(.never).autocorrectionDisabled()
            .navigationTitle(AppLocalization.string("History")).navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) { historyFilterMenu }
            }.task(id: queryKey) { await loadPage(reset: true) }
        }
    }
    private var historyFilterMenu: some View {
        Menu {
            Picker(AppLocalization.string("Wallet"), selection: $selectedWalletId) {
                Text(AppLocalization.string("All Wallets")).tag(Optional<String>.none)
                ForEach(store.wallets) { wallet in Text(wallet.name).tag(Optional(wallet.id)) }
            }
            Picker(AppLocalization.string("Type"), selection: $selectedFilter) {
                ForEach(HistoryQueryFilter.allCases) { filter in Text(filter.localizedTitle).tag(filter) }
            }
            Picker(AppLocalization.string("Sort"), selection: $selectedSortOrder) {
                ForEach(HistorySortOrder.allCases) { sortOrder in Text(sortOrder.localizedTitle).tag(sortOrder) }
            }
            Toggle(isOn: $hidesSmallAmounts) {
                Text(AppLocalization.string("Hide small amounts"))
                Text(AppLocalization.format("Below %@", AmountPresentation.localizedDecimal(historySmallAmountThreshold())))
            }
            // The app-wide switch style has no menu form; a menu shows a switch
            // as a disabled item. `.automatic` is the checkmark item.
            .toggleStyle(.automatic)
        } label: {
            Image(systemName: "line.3.horizontal.decrease.circle")
        }.accessibilityLabel(AppLocalization.string("Filter history"))
    }

    private var historyWalletIds: Set<String> {
        if let selectedWalletId { return [selectedWalletId] }
        return Set(store.wallets.map(\.id))
    }
    private var canLoadMoreVisibleHistory: Bool { store.canLoadMoreOnChainHistory(for: historyWalletIds) }
    private var shouldShowPagingControls: Bool {
        hasMoreStoredHistory || canLoadMoreVisibleHistory || store.isLoadingMoreOnChainHistory
    }
    private var pagedRows: [HistoryRowPresentation] {
        visibleTransactions.map(historyRowPresentation)
    }
    private var groupedSections: [HistoryPresentationSection] {
        let calendar = Calendar.current
        let grouped = Dictionary(grouping: pagedRows) { HistoryDateGroup($0.transaction, calendar: calendar) }
        let order = selectedSortOrder == .newest ? HistoryDateGroup.allCases : HistoryDateGroup.allCases.reversed()
        return order.compactMap { group in
            guard let rows = grouped[group], !rows.isEmpty else { return nil }
            return HistoryPresentationSection(group: group, rows: rows)
        }
    }
    private var historyError: String? { pageError ?? store.historyReadError }
    private var visibleTransactions: [TransactionRecord] { pageRecords }
    private var filterKey: String {
        "\(selectedWalletId ?? "")|\(selectedFilter)|\(selectedSortOrder)|\(hidesSmallAmounts)|\(searchText)"
    }
    private var queryKey: String { "\(filterKey)|\(store.transactionRevision)|\(store.walletIdentityRevision)" }
    private static let pageSize = 20
    private func loadPage(reset: Bool) async {
        let key = queryKey
        if loadedFilterKey != filterKey {
            pageRecords = []
            hasMoreStoredHistory = false
            loadedFilterKey = filterKey
        }
        let requestId = UUID()
        pageRequestId = requestId
        isLoadingPage = true
        defer { if pageRequestId == requestId { isLoadingPage = false } }
        do {
            let bridge = try await store.bridge.ready()
            // A reload keeps every row already on screen, so a refresh or an
            // on-chain fetch does not snap the list back to its first page.
            let target = reset ? max(Self.pageSize, pageRecords.count + pendingGrowth) : Self.pageSize
            var records: [TransactionRecord] = []
            var cursor = reset ? nil : nextCursor
            var hasMore = false
            repeat {
                let page = try await bridge.historyPage(query: HistoryQuery(
                    walletId: selectedWalletId, filter: selectedFilter, search: searchText,
                    oldestFirst: selectedSortOrder == .oldest, cursor: cursor,
                    // Core caps a page; the loop follows its cursor for the rest.
                    limit: UInt32(target - records.count),
                    hideSmallAmounts: hidesSmallAmounts))
                guard !Task.isCancelled, pageRequestId == requestId, queryKey == key else { return }
                records += page.records
                cursor = page.nextCursor
                hasMore = page.hasMore
            } while hasMore && records.count < target
            if reset {
                pageRecords = records
                pendingGrowth = 0
            } else {
                let present = Set(pageRecords.map(\.id))
                pageRecords += records.filter { !present.contains($0.id) }
            }
            nextCursor = cursor
            hasMoreStoredHistory = hasMore
            pageError = nil
        } catch {
            guard !Task.isCancelled, pageRequestId == requestId, queryKey == key else { return }
            pageError = userErrorMessage(error)
            store.appendOperationalLog(.error, category: "History", message: String(describing: error))
        }
    }
    private var historyPagingControls: some View {
        Button {
            Task {
                if !hasMoreStoredHistory {
                    await store.loadMoreOnChainHistory(for: historyWalletIds)
                    // The fetch bumps the transaction revision, so `.task` reloads
                    // too, in either order. Both read `pendingGrowth`, so whichever
                    // lands last shows the extra page.
                    pendingGrowth = Self.pageSize
                    await loadPage(reset: true)
                } else {
                    await loadPage(reset: false)
                }
            }
        } label: {
            HStack {
                if store.isLoadingMoreOnChainHistory { ProgressView() }
                Text(AppLocalization.string(store.isLoadingMoreOnChainHistory ? "Loading" : "Load more"))
            }.frame(maxWidth: .infinity, minHeight: 44)
        }
        .buttonStyle(.glass)
        .disabled(store.isLoadingMoreOnChainHistory || isLoadingPage)
    }
    private var historyEmptyStateCard: some View {
        SpectraEmptyStateCard(
            title: emptyStateTitle,
            message: emptyStateMessage,
            systemImage: store.transactionCount == 0 ? "clock.arrow.circlepath" : "magnifyingglass"
        )
    }
    private var emptyStateTitle: String {
        store.transactionCount == 0
            ? AppLocalization.string("No activity yet")
            : AppLocalization.string("No matches found")
    }
    private var emptyStateMessage: String {
        if store.wallets.isEmpty { return AppLocalization.string("No wallets are currently loaded. Import a wallet to view activity.") }
        if store.transactionCount == 0 {
            return AppLocalization.string("Send funds or receive funds to build a persistent transaction log.")
        }
        return AppLocalization.string("Try a different filter or search term.")
    }
    private func historyRowPresentation(for transaction: TransactionRecord) -> HistoryRowPresentation {
        HistoryRowPresentation(
            transaction: transaction, amountText: signedAmountText(for: transaction), amountColor: amountColor(for: transaction),
            subtitleText: transaction.walletName, fullTimestampText: transaction.fullTimestampText,
            metadataText: store.amounts.historyMetadataText(for: transaction)
        )
    }
    private func signedAmountText(for transaction: TransactionRecord) -> String? {
        let amountText = store.amounts.formattedTransactionAmount(transaction)
        return transaction.amountSign + amountText
    }
    private func amountColor(for transaction: TransactionRecord) -> Color {
        transaction.amountColor
    }
}
