import SwiftUI
private struct HistoryRowPresentation: Identifiable, Equatable {
    let transaction: TransactionRecord
    let titleText: String
    let amountText: String
    let amountColor: Color
    let subtitleText: String
    let timeText: String
    let hidesBalance: Bool
    var id: String { transaction.id }
}

/// What the transaction did, then the amount it moved: a stake reads as a
/// stake, not as a send, and a fee-only operation still names itself.
private struct HistoryTransactionRowView: View, Equatable {
    let row: HistoryRowPresentation
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool { lhs.row == rhs.row }
    var body: some View {
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: row.transaction.artworkName, fallbackText: row.transaction.symbol,
                color: row.transaction.badgeColor, size: 36)
            if dynamicTypeSize.isAccessibilitySize {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    titleLine
                    amount
                    Text(row.subtitleText).font(.caption).foregroundStyle(.secondary)
                    Text(row.timeText).font(.caption2).foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            } else {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    titleLine
                    Text(row.subtitleText).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: SpectraLayout.Space.s)
                VStack(alignment: .trailing, spacing: SpectraLayout.Space.xxs) {
                    amount.spectraNumericTextLayout()
                    Text(row.timeText).font(.caption2).foregroundStyle(.secondary)
                }
            }
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }
    }
    private var titleLine: some View {
        HStack(spacing: SpectraLayout.Space.xs) {
            Text(row.titleText).font(.headline).foregroundStyle(Color.primary).lineLimit(2)
            // A confirmed row says nothing more; only the exceptions do.
            if row.transaction.status != .confirmed {
                TransactionStatusBadge(status: row.transaction.status, compact: true)
            }
        }
    }
    private var amount: some View {
        BalanceText(text: row.amountText, isHidden: row.hidesBalance)
            .font(.subheadline.weight(.semibold)).foregroundStyle(row.amountColor)
    }
}
/// Where a row sits in time, newest first: the unconfirmed with no date,
/// today, yesterday, then one group per month.
private enum HistoryDateGroup: Hashable, Comparable {
    case unconfirmed, today, yesterday
    case month(year: Int, month: Int)

    init(_ transaction: TransactionRecord, calendar: Calendar) {
        if !transaction.hasKnownDate, transaction.status == .pending {
            self = .unconfirmed
        } else if calendar.isDateInToday(transaction.createdDate) {
            self = .today
        } else if calendar.isDateInYesterday(transaction.createdDate) {
            self = .yesterday
        } else {
            let parts = calendar.dateComponents([.year, .month], from: transaction.createdDate)
            self = .month(year: parts.year ?? 0, month: parts.month ?? 0)
        }
    }

    /// Newest first.
    static func < (lhs: Self, rhs: Self) -> Bool {
        func rank(_ group: Self) -> (Int, Int) {
            switch group {
            case .unconfirmed: return (0, 0)
            case .today: return (1, 0)
            case .yesterday: return (2, 0)
            case .month(let year, let month): return (3, -(year * 12 + month))
            }
        }
        return rank(lhs) < rank(rhs)
    }

    var title: String {
        switch self {
        case .unconfirmed: return AppLocalization.string("Unconfirmed")
        case .today: return AppLocalization.string("Today")
        case .yesterday: return AppLocalization.string("Yesterday")
        case .month(let year, let month):
            let date = Calendar.current.date(from: DateComponents(year: year, month: month)) ?? .distantPast
            return date.formatted(.dateTime.month(.wide).year().locale(AppLocalization.locale))
        }
    }

    /// Inside today and yesterday the header gives the day; a month header
    /// gives only the month.
    var showsDate: Bool {
        switch self {
        case .today, .yesterday: return false
        case .unconfirmed, .month: return true
        }
    }
}

private struct HistoryPresentationSection: Identifiable {
    let group: HistoryDateGroup
    let rows: [HistoryRowPresentation]
    var id: HistoryDateGroup { group }
}
struct HistoryView: View {
    let store: AppState
    var body: some View {
        NavigationStack {
            HistoryListView(store: store)
                // Speed Up and Cancel open the composer over the transaction
                // they replace, and Back returns to it.
                .navigationDestination(isPresented: store.sendFlowBinding(on: .history)) {
                    SendView(store: store)
                }
        }
    }
}
/// Every wallet's history, or one wallet's: the History tab, and the list a
/// wallet's page opens.
struct HistoryListView: View {
    let store: AppState
    /// The one wallet the list shows; `nil` lets the filter choose.
    private let fixedWalletId: String?
    @State private var selectedFilter: HistoryQueryFilter = .all
    @State private var selectedSortOrder: HistorySortOrder = .newest
    @State private var selectedWalletId: String?
    @State private var hidesSmallAmounts = false
    @State private var searchText: String = ""
    @State private var pageRecords: [TransactionRecord] = []
    /// A page has landed for the query on screen. Until then an empty list
    /// is not an answer, and says nothing about matches.
    @State private var hasLoadedPage = false
    @State private var nextCursor: String?
    @State private var hasMoreStoredHistory = false
    @State private var pageError: String?
    @State private var isLoadingPage = false
    @State private var pageRequestId = UUID()
    @State private var loadedFilterKey: String?
    /// Rows the next reload adds beyond those on screen; cleared once a reload lands.
    @State private var pendingGrowth = 0
    @State private var isRetrying = false
    /// What a recheck or rebroadcast from a row's menu came to.
    @State private var actionNotice: SpectraTransientNotice?
    init(store: AppState, walletId: String? = nil) {
        self.store = store
        fixedWalletId = walletId
        _selectedWalletId = State(initialValue: walletId)
    }
    var body: some View {
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
                    if !hasLoadedPage && historyError == nil {
                        loadingPlaceholder
                    } else if pageRecords.isEmpty && historyError == nil {
                        historyEmptyStateCard
                    }
                    ForEach(groupedSections) { section in
                        SpectraRowGroup(title: section.group.title, data: section.rows) { row in
                            NavigationLink {
                                TransactionDetailView(store: store, transaction: row.transaction)
                            } label: {
                                HistoryTransactionRowView(row: row).equatable().spectraRowPadding()
                            }
                            .buttonStyle(.plain)
                            .contextMenu { rowActions(row.transaction) }
                        }
                    }
                    // A new query keeps the old rows until its answer lands,
                    // dimmed, rather than flashing an empty list.
                    .opacity(isLoadingPage && loadedFilterKey != filterKey ? 0.5 : 1)
                    if shouldShowPagingControls { historyPagingControls }
                }.spectraScreenPadding()
            }.refreshable {
                await store.performUserInitiatedRefresh()
            }.scrollBounceBehavior(.always)
        }
        .overlay(alignment: .bottom) {
            if let actionNotice {
                Text(actionNotice.text).font(.subheadline.weight(.semibold))
                    .padding(.horizontal, SpectraLayout.Space.l).padding(.vertical, SpectraLayout.Space.m)
                    .spectraElevatedFill()
                    .padding(.horizontal, SpectraLayout.screenHorizontal).padding(.bottom, SpectraLayout.Space.l)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .spectraTransientNotice($actionNotice, seconds: 4)
        .animation(.snappy, value: actionNotice)
        .searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always),
                     prompt: AppLocalization.string("Search history"))
        .textInputAutocapitalization(.never).autocorrectionDisabled()
        .navigationTitle(AppLocalization.string("History")).navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) { historyFilterMenu }
        }.task(id: queryKey) {
            // Typing searches once it pauses, not on every keystroke.
            if loadedFilterKey != nil, loadedFilterKey != filterKey, !searchText.isEmpty {
                try? await Task.sleep(for: .milliseconds(250))
                guard !Task.isCancelled else { return }
            }
            await loadPage(reset: true)
        }
    }
    @ViewBuilder
    private func rowActions(_ transaction: TransactionRecord) -> some View {
        if transaction.actions.recheckUnavailableReason == nil {
            Button {
                spectraHaptic(.light)
                Task { actionNotice = SpectraTransientNotice(await store.retryUTXOTransactionStatus(for: transaction.id)) }
            } label: {
                Label(AppLocalization.string("Recheck"), systemImage: "arrow.clockwise")
            }
        }
        if transaction.actions.rebroadcastUnavailableReason == nil {
            Button {
                spectraHaptic(.light)
                Task { actionNotice = SpectraTransientNotice(await store.rebroadcastSignedTransaction(for: transaction.id)) }
            } label: {
                Label(AppLocalization.string("Rebroadcast"), systemImage: "dot.radiowaves.up.forward")
            }
        }
    }
    private var loadingPlaceholder: some View {
        VStack(spacing: SpectraLayout.Space.m) {
            ForEach(0..<4, id: \.self) { _ in
                HStack(spacing: SpectraLayout.Space.m) {
                    Circle().fill(SpectraLayout.insetFill).frame(width: 36, height: 36)
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                        SpectraShimmer(height: 14).frame(maxWidth: 150)
                        SpectraShimmer(height: 10).frame(maxWidth: 90)
                    }
                    Spacer(minLength: 0)
                    SpectraShimmer(height: 14).frame(width: 70)
                }
            }
        }
        .padding(SpectraLayout.cardPadding)
        .frame(maxWidth: .infinity)
        .spectraCardFill()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(AppLocalization.string("Loading"))
    }
    /// Anything the filter menu narrows the list by.
    private var filtersAreActive: Bool {
        (fixedWalletId == nil && selectedWalletId != nil) || selectedFilter != .all || hidesSmallAmounts
    }
    private var historyFilterMenu: some View {
        Menu {
            if fixedWalletId == nil {
                Picker(AppLocalization.string("Wallet"), selection: $selectedWalletId) {
                    Text(AppLocalization.string("All Wallets")).tag(Optional<String>.none)
                    ForEach(store.wallets) { wallet in Text(wallet.name).tag(Optional(wallet.id)) }
                }
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
            if filtersAreActive {
                Button(AppLocalization.string("Clear Filters"), systemImage: "xmark.circle", action: clearFilters)
            }
        } label: {
            Image(systemName: filtersAreActive
                ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
        }
        .accessibilityLabel(AppLocalization.string("Filter history"))
        .accessibilityValue(filtersAreActive ? AppLocalization.string("Filtered") : "")
    }
    private func clearFilters() {
        if fixedWalletId == nil { selectedWalletId = nil }
        selectedFilter = .all
        hidesSmallAmounts = false
        searchText = ""
    }

    private var historyWalletIds: Set<String> {
        if let selectedWalletId { return [selectedWalletId] }
        return Set(store.wallets.map(\.id))
    }
    private var canLoadMoreVisibleHistory: Bool { store.historyPaging.canLoadMore(for: historyWalletIds) }
    private var shouldShowPagingControls: Bool {
        hasMoreStoredHistory || canLoadMoreVisibleHistory || store.historyPaging.isLoadingMore
    }
    private var groupedSections: [HistoryPresentationSection] {
        let calendar = Calendar.current
        let grouped = Dictionary(grouping: pageRecords) { HistoryDateGroup($0, calendar: calendar) }
        let order = selectedSortOrder == .newest ? grouped.keys.sorted() : grouped.keys.sorted().reversed()
        return order.compactMap { group in
            guard let records = grouped[group], !records.isEmpty else { return nil }
            return HistoryPresentationSection(group: group, rows: records.map { historyRowPresentation(for: $0, in: group) })
        }
    }
    private var historyError: String? { pageError ?? store.historyReadError }
    private var filterKey: String {
        "\(selectedWalletId ?? "")|\(selectedFilter)|\(selectedSortOrder)|\(hidesSmallAmounts)|\(searchText)"
    }
    private var queryKey: String { "\(filterKey)|\(store.transactionRevision)|\(store.walletIdentityRevision)" }
    private static let pageSize = 20
    private func loadPage(reset: Bool) async {
        let key = queryKey
        let filter = filterKey
        let requestId = UUID()
        pageRequestId = requestId
        isLoadingPage = true
        defer { if pageRequestId == requestId { isLoadingPage = false } }
        do {
            let bridge = try await store.bridge.ready()
            // A reload keeps every row already on screen, so a refresh or an
            // on-chain fetch does not snap the list back to its first page.
            // A new filter starts from one page.
            let sameQuery = loadedFilterKey == filter
            let target = reset ? max(Self.pageSize, sameQuery ? pageRecords.count + pendingGrowth : 0) : Self.pageSize
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
            loadedFilterKey = filter
            hasLoadedPage = true
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
                if store.historyPaging.isLoadingMore { ProgressView() }
                Text(AppLocalization.string(store.historyPaging.isLoadingMore ? "Loading" : "Load more"))
            }.frame(maxWidth: .infinity, minHeight: 44)
        }
        .buttonStyle(.glass)
        .disabled(store.historyPaging.isLoadingMore || isLoadingPage)
    }
    @ViewBuilder
    private var historyEmptyStateCard: some View {
        if store.wallets.isEmpty {
            SpectraEmptyStateCard(
                title: "No activity yet", message: "Add a wallet to see its activity here.",
                systemImage: "clock.arrow.circlepath", actionTitle: "Add Wallet", actionSystemImage: "plus"
            ) {
                store.selectedMainTab = .home
                store.isShowingAddWalletEntry = true
            }
        } else if filtersAreActive || !searchText.isEmpty {
            SpectraEmptyStateCard(
                title: "No matches found", message: "Try a different filter or search term.",
                systemImage: "magnifyingglass", actionTitle: "Clear Filters", actionSystemImage: "xmark.circle",
                action: clearFilters)
        } else {
            SpectraEmptyStateCard(
                title: "No activity yet", message: "Send funds or receive funds to build a persistent transaction log.",
                systemImage: "clock.arrow.circlepath")
        }
    }
    private func historyRowPresentation(for transaction: TransactionRecord, in group: HistoryDateGroup) -> HistoryRowPresentation {
        HistoryRowPresentation(
            transaction: transaction, titleText: transaction.titleText,
            amountText: transaction.amountSign + store.amounts.formattedTransactionAmount(transaction),
            amountColor: transaction.amountColor, subtitleText: transaction.walletName,
            timeText: transaction.timestampText(showsDate: group.showsDate),
            hidesBalance: store.preferences.hideBalances
        )
    }
}
