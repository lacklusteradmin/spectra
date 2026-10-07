import Foundation
import SwiftUI
import UIKit
/// One stored transfer, most-read first: what moved and where it stands, the
/// one action a pending send may still take, how it got there, who was on
/// each end, and the few facts a reader checks. Everything else — gas, paths,
/// payload — is folded under Technical Details. Each fact appears once.
struct TransactionDetailView: View {
    let store: AppState
    let transaction: TransactionRecord
    @State private var replacementMessage: String?
    @State private var liveTransaction: TransactionRecord?
    /// Which ends to show and whether each is the wallet's own: core's
    /// answer, cached for the body. View state: losing it costs a redraw.
    @State private var endpoints: TransactionEndpoints?
    /// Folded by default: gas, paths and payload are for the reader who asks.
    @State private var isShowingTechnicalDetails = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    init(store: AppState, transaction: TransactionRecord) {
        self.store = store
        self.transaction = transaction
    }
    private var displayedTransaction: TransactionRecord { liveTransaction ?? transaction }
    var body: some View {
        ZStack {
            SpectraBackdrop().ignoresSafeArea()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.sectionSpacing) {
                    heroCard
                    mempoolActionsCard
                    transactionTimelineCard
                    addressesCard
                    detailsCard
                    technicalDetailsCard
                }.spectraScreenPadding()
            }
        }.navigationTitle(AppLocalization.string("Transaction")).navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar)
            .task(id: refreshKey) { await rebuildDisplayedTransactionState() }
    }
    /// What moved, from which wallet on which network, and where it stands.
    /// The amount is signed and coloured as the history row draws it, so the
    /// direction needs no row of its own.
    private var heroCard: some View {
        let tx = displayedTransaction
        return VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            HStack(alignment: .top, spacing: SpectraLayout.Space.m) {
                CoinBadge(artworkName: tx.artworkName, fallbackText: tx.symbol, color: tx.badgeColor, size: 42)
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(tx.titleText).font(.headline).foregroundStyle(.primary)
                    Text(tx.subtitleText).font(.subheadline).foregroundStyle(.secondary)
                }
                Spacer(minLength: SpectraLayout.Space.s)
                TransactionStatusBadge(status: tx.status)
            }
            Text(signedAmountText).font(.title.weight(.bold))
                .foregroundStyle(tx.amountColor)
                .spectraNumericTextLayout(minimumScaleFactor: 0.5)
        }.padding(SpectraLayout.cardPadding).frame(maxWidth: .infinity, alignment: .leading)
            .spectraElevatedFill()
    }
    private var signedAmountText: String {
        let amount = store.amounts.formattedTransactionDetailAmount(displayedTransaction)
        return displayedTransaction.amountSign + amount
    }
    // Core says which rows can still be replaced; this row is one of them or
    // it is not. Second on the page because it is the only thing here a
    // reader can act on, and it cannot wait.
    @ViewBuilder
    private var mempoolActionsCard: some View {
        if let pending = store.replaceableSend(forTransaction: displayedTransaction.id) {
            spectraDetailCard(title: AppLocalization.format("%@ Mempool Actions", pending.chainId.displayName)) {
                if store.sendFlow.isPreparingReplacement {
                    SpectraLoadingRow(title: "Preparing replacement/cancel context...")
                } else {
                    if pending.canSpeedUp {
                        Button {
                            Task {
                                replacementMessage = await store.openReplacementComposer(
                                    for: displayedTransaction.id, cancel: false
                                )
                            }
                        } label: {
                            Text(AppLocalization.string("Speed Up This Transaction")).font(.headline).frame(maxWidth: .infinity)
                                .padding(.vertical, SpectraLayout.Space.m)
                        }.buttonStyle(.glassProminent)
                    }
                    Button {
                        Task {
                            replacementMessage = await store.openReplacementComposer(
                                for: displayedTransaction.id, cancel: true
                            )
                        }
                    } label: {
                        Text(AppLocalization.string("Cancel This Transaction")).font(.headline).frame(maxWidth: .infinity).padding(
                            .vertical, SpectraLayout.Space.m)
                    }.buttonStyle(.glass)
                    Text(
                        AppLocalization.string(
                            pending.canSpeedUp
                                ? "This opens the Send composer with the same nonce and higher fee defaults so you can safely speed up or cancel the pending transaction."
                                : "This opens the Send composer with the same nonce and higher fee defaults so you can cancel the pending transfer. A token transfer cannot be rebuilt from its record, so it cannot be sped up."
                        )
                    ).font(.caption).foregroundStyle(.secondary)
                }
                if let replacementMessage {
                    Text(replacementMessage).font(.caption).foregroundStyle(.secondary)
                }
            }
        }
    }
    @ViewBuilder
    private var addressesCard: some View {
        let from = endpoints?.from
        let to = endpoints?.to
        if from != nil || to != nil {
            spectraDetailCard(title: "Addresses") {
                if let from { TransactionAddressRow(label: "From", endpoint: from) }
                if from != nil, to != nil { Divider().opacity(0.4) }
                if let to { TransactionAddressRow(label: "To", endpoint: to) }
            }
        }
    }
    /// The facts a reader checks: the network, what it cost, and the hash
    /// to look it up by. Status, time, block and confirmations are the
    /// timeline's; wallet, asset and amount are the hero's.
    private var detailsCard: some View {
        let tx = displayedTransaction
        return spectraDetailCard(title: "Details") {
            TransactionDetailRow(systemImage: "network", label: "Network", value: tx.chainName)
            if let networkFeeText {
                Divider().opacity(0.4)
                TransactionDetailRow(systemImage: "fuelpump.fill", label: "Network Fee", value: networkFeeText)
            }
            if let hash = nonEmptyAddress(tx.transactionHash) {
                Divider().opacity(0.4)
                CopyableValueRow(value: hash) {
                    TransactionDetailRow(systemImage: "number", label: "Transaction Hash", value: hash, isIdentifier: true)
                }
            }
            if let explorer = tx.explorerLink {
                Divider().opacity(0.4)
                Link(destination: explorer.url) {
                    HStack(spacing: SpectraLayout.Space.s) {
                        Image(systemName: "safari").font(.subheadline.weight(.semibold)).frame(width: 22)
                        Text(explorer.label).font(.subheadline.weight(.semibold))
                        Spacer(minLength: SpectraLayout.Space.m)
                        Image(systemName: "arrow.up.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                    }.contentShape(Rectangle())
                }
            }
        }
    }
    /// The receipt's fee when there is one, else the fee stored at
    /// confirmation. When both exist the other sits under Technical Details.
    private var networkFeeText: String? {
        store.amounts.receiptNetworkFeeText(for: displayedTransaction)
            ?? store.amounts.confirmedNetworkFeeText(for: displayedTransaction)
    }
    private struct TechnicalRow: Identifiable {
        let label: String
        let value: String
        let isIdentifier: Bool
        var id: String { label }
    }
    private var technicalRows: [TechnicalRow] {
        let tx = displayedTransaction
        let amounts = store.amounts
        var rows: [TechnicalRow] = []
        func add(_ label: String, _ value: String?, isIdentifier: Bool = false) {
            guard let value, !value.isEmpty else { return }
            rows.append(TechnicalRow(label: label, value: value, isIdentifier: isIdentifier))
        }
        add("History Source", amounts.historySourceText(for: tx))
        add("Gas Used", tx.receiptGasUsed)
        add("Effective Gas Price", amounts.receiptEffectiveGasPriceText(for: tx))
        if amounts.receiptNetworkFeeText(for: tx) != nil {
            add("Confirmed Fee", amounts.confirmedNetworkFeeText(for: tx))
        }
        add("Fee Rate", amounts.storedFeeRateText(for: tx))
        add("Used Change Output", tx.storedUsedChangeOutputText)
        add("Signed Payload Format", tx.rawTransactionFormatText)
        add("Source Path", tx.sourceDerivationPath)
        add("Change Path", tx.changeDerivationPath)
        // On an account chain the source is the From address above.
        if tx.sourceAddress != endpoints?.from?.address {
            add("Source Address", tx.sourceAddress, isIdentifier: true)
        }
        add("Change Address", tx.changeAddress, isIdentifier: true)
        return rows
    }
    @ViewBuilder
    private var technicalDetailsCard: some View {
        let rows = technicalRows
        let raw = displayedTransaction.rawTransactionText
        if !rows.isEmpty || raw != nil {
            VStack(alignment: .leading, spacing: 0) {
                Button {
                    withAnimation(reduceMotion ? nil : .snappy(duration: 0.25)) { isShowingTechnicalDetails.toggle() }
                } label: {
                    HStack(spacing: SpectraLayout.Space.s) {
                        Text(AppLocalization.string("Technical Details")).font(.headline).foregroundStyle(.primary)
                        Spacer(minLength: SpectraLayout.Space.m)
                        Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                            .rotationEffect(.degrees(isShowingTechnicalDetails ? 90 : 0))
                    }.frame(minHeight: 44).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(isShowingTechnicalDetails ? [.isButton, .isSelected] : .isButton)
                if isShowingTechnicalDetails {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                        ForEach(rows) { row in
                            Divider().opacity(0.4)
                            if row.isIdentifier {
                                CopyableValueRow(value: row.value) {
                                    TransactionDetailRow(label: row.label, value: row.value, isIdentifier: true)
                                }
                            } else {
                                TransactionDetailRow(label: row.label, value: row.value)
                            }
                        }
                        if let raw {
                            Divider().opacity(0.4)
                            CopyableValueRow(value: raw) {
                                Text(AppLocalization.string("Raw Transaction")).font(.subheadline).foregroundStyle(.secondary)
                            }
                            Text(verbatim: breakableAnywhere(raw)).font(.caption.monospaced()).foregroundStyle(.secondary)
                                .padding(SpectraLayout.Space.m).frame(maxWidth: .infinity, alignment: .leading)
                                .spectraInsetFill()
                        }
                    }.padding(.top, SpectraLayout.Space.xs)
                }
            }
            .padding(.horizontal, SpectraLayout.cardPadding).padding(.vertical, SpectraLayout.cardHeaderVertical).frame(maxWidth: .infinity, alignment: .leading)
            .spectraCardFill()
        }
    }
    /// The two revision counters the rebuild depends on, bundled because
    /// `.task(id:)` takes one `Equatable` value. One cancellable task replaces
    /// a `.task` plus two `onChange` closures that each spawned a detached one:
    /// a revision arriving mid-rebuild now cancels the stale pass instead of
    /// racing it to the `live*` assignments.
    private var refreshKey: RefreshKey {
        RefreshKey(transactions: store.transactionRevision, wallets: store.walletIdentityRevision)
    }
    private struct RefreshKey: Equatable {
        let transactions: UInt64
        let wallets: UInt64
    }
    private var transactionTimelineCard: some View {
        spectraDetailCard(title: "Timeline") {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(transactionTimelineItems.enumerated()), id: \.element.id) { index, item in
                    timelineRow(item, isLast: index == transactionTimelineItems.count - 1)
                }
            }
        }
    }
    private var transactionTimelineItems: [TransactionTimelineItem] {
        var items: [TransactionTimelineItem] = [
            TransactionTimelineItem(
                id: "recorded",
                title: displayedTransaction.isSubmittedOperation ? "Created" : "Recorded",
                detail: displayedTransaction.fullTimestampText,
                systemImage: displayedTransaction.isSubmittedOperation ? "paperplane.fill" : "arrow.down.circle.fill",
                tint: .accentColor,
                isComplete: true,
                isCurrent: false
            )
        ]

        if let transactionHash = nonEmptyAddress(displayedTransaction.transactionHash) {
            items.append(
                TransactionTimelineItem(
                    id: "network-hash",
                    title: displayedTransaction.isSubmittedOperation ? "Broadcast" : "Detected",
                    detail: AppLocalization.format("Hash %@", shortTransactionHash(transactionHash)),
                    systemImage: "link",
                    tint: .accentColor,
                    isComplete: true,
                    isCurrent: displayedTransaction.status == .pending
                )
            )
        } else {
            items.append(
                TransactionTimelineItem(
                    id: "network-hash",
                    title: "Awaiting Network Hash",
                    detail: "Spectra has not attached a network transaction hash yet.",
                    systemImage: "hourglass",
                    tint: .spectraWarning,
                    isComplete: false,
                    isCurrent: displayedTransaction.status == .pending
                )
            )
        }

        switch displayedTransaction.status {
        case .pending:
            items.append(
                TransactionTimelineItem(
                    id: "pending",
                    title: "Pending Confirmation",
                    detail: "Spectra will keep refreshing this transaction.",
                    systemImage: "clock.arrow.circlepath",
                    tint: .spectraWarning,
                    isComplete: false,
                    isCurrent: true
                )
            )
        case .confirmed:
            items.append(
                TransactionTimelineItem(
                    id: "confirmed",
                    title: "Confirmed",
                    detail: confirmedTimelineDetail,
                    systemImage: "checkmark.seal.fill",
                    tint: .green,
                    isComplete: true,
                    isCurrent: true
                )
            )
        case .failed:
            items.append(
                TransactionTimelineItem(
                    id: "failed",
                    title: "Failed",
                    detail: displayedTransaction.localizedFailureReason
                        ?? AppLocalization.string("Network or local validation failed."),
                    systemImage: "xmark.octagon.fill",
                    tint: .red,
                    isComplete: false,
                    isCurrent: true
                )
            )
        }
        return items
    }
    private var confirmedTimelineDetail: String {
        var parts: [String] = []
        if let receiptBlockNumberText = displayedTransaction.receiptBlockNumberText {
            parts.append(AppLocalization.format("Block %@", receiptBlockNumberText))
        }
        if let storedConfirmationCountText = displayedTransaction.storedConfirmationCountText {
            parts.append(storedConfirmationCountText)
        }
        return parts.isEmpty ? AppLocalization.string("Network has confirmed this transaction.") : parts.joined(separator: " - ")
    }
    private func shortTransactionHash(_ hash: String) -> String {
        guard hash.count > 20 else { return hash }
        return "\(hash.prefix(10))...\(hash.suffix(6))"
    }
    private func timelineRow(_ item: TransactionTimelineItem, isLast: Bool) -> some View {
        HStack(alignment: .top, spacing: SpectraLayout.Space.m) {
            VStack(spacing: SpectraLayout.Space.xs) {
                Image(systemName: item.systemImage)
                    .font(.caption.weight(.bold))
                    .foregroundStyle(item.isComplete || item.isCurrent ? item.tint : Color.secondary)
                    .frame(width: 30, height: 30)
                    .background(
                        Circle()
                            .fill((item.isComplete || item.isCurrent ? item.tint : Color.primary).opacity(0.14))
                    )
                if !isLast {
                    Rectangle()
                        .fill(item.isComplete ? item.tint.opacity(0.35) : Color.primary.opacity(0.12))
                        .frame(width: 2, height: 28)
                }
            }
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                HStack(spacing: SpectraLayout.Space.s) {
                    Text(AppLocalization.string(item.title))
                        .font(.subheadline.weight(.semibold))
                    if item.isCurrent {
                        Text(AppLocalization.string("Current"))
                            .font(.caption2.weight(.bold))
                            .foregroundStyle(item.tint)
                            .padding(.horizontal, SpectraLayout.Space.s)
                            .padding(.vertical, SpectraLayout.Space.xxs)
                            .background(item.tint.opacity(0.14), in: Capsule())
                    }
                }
                Text(AppLocalization.string(item.detail))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, SpectraLayout.Space.xxs)
    }
    private func nonEmptyAddress(_ value: String?) -> String? {
        guard let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmed.isEmpty else { return nil }
        return trimmed
    }
    private func rebuildDisplayedTransactionState() async {
        guard let projection = try? await loadTransactionDetailProjection(
            fallback: transaction,
            transaction: { try await store.bridge.ready().transaction(id: transaction.id) },
            endpoints: { try await store.bridge.ready().transactionEndpoints(transactionId: transaction.id) }
        ) else { return }
        liveTransaction = projection.transaction
        endpoints = projection.endpoints
    }
    private struct TransactionTimelineItem: Identifiable {
        let id: String
        let title: String
        let detail: String
        let systemImage: String
        let tint: Color
        let isComplete: Bool
        let isCurrent: Bool
    }
}

/// Read one detail projection before adopting either field. UniFFI reads can
/// finish after SwiftUI has cancelled the task for a newer revision.
@MainActor
func loadTransactionDetailProjection(
    fallback: TransactionRecord,
    transaction: () async throws -> TransactionRecord?,
    endpoints: () async throws -> TransactionEndpoints?
) async throws -> (transaction: TransactionRecord, endpoints: TransactionEndpoints?) {
    try Task.checkCancellation()
    let record = (try? await transaction()) ?? fallback
    try Task.checkCancellation()
    let parties = try? await endpoints()
    try Task.checkCancellation()
    return (record, parties)
}

/// A key/value row: an accent symbol, a secondary label and a primary value.
///
/// Side by side when both fit on one line, otherwise the value goes under
/// the label — neither is squeezed, so a value such as
/// `core.submission_json`, which has no space to wrap at, is never
/// hyphenated. Always stacked at accessibility sizes. An identifier — a hash
/// or an address — is the exception: it keeps the line beside its label and
/// is cut in the middle so both ends show; its row copies it in full.
private struct TransactionDetailRow: View {
    var systemImage: String? = nil
    let label: String
    let value: String
    var isIdentifier = false
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        if isIdentifier {
            HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.m) {
                labelView.layoutPriority(1)
                Spacer(minLength: SpectraLayout.Space.m)
                Text(emphasizedEnds(value)).font(.subheadline.monospaced())
                    .lineLimit(1).truncationMode(.middle)
            }
        } else if dynamicTypeSize.isAccessibilitySize {
            stacked
        } else {
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.m) {
                    labelView.fixedSize()
                    Spacer(minLength: SpectraLayout.Space.m)
                    valueText.fixedSize()
                }
                stacked
            }
        }
    }
    private var labelView: some View {
        HStack(spacing: SpectraLayout.Space.s) {
            if let systemImage {
                Image(systemName: systemImage).font(.subheadline.weight(.semibold)).foregroundStyle(.tint).frame(width: 22)
            }
            Text(AppLocalization.string(label)).font(.subheadline).foregroundStyle(.secondary)
        }
    }
    private var valueText: some View {
        Text(value).font(.subheadline.weight(.semibold)).foregroundStyle(.primary)
    }
    /// The value under the label, lined up with the label's text rather than
    /// its symbol.
    private var stacked: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
            labelView
            valueText.padding(.leading, systemImage == nil ? 0 : 22 + SpectraLayout.Space.s)
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// One end of the transfer: its label, who holds it — a wallet of the
/// user's or a saved contact, as core names it — and the address on one line
/// with its ends in full, the ends being what a reader compares against. The
/// row copies the whole address.
private struct TransactionAddressRow: View {
    let label: String
    let endpoint: TransactionEndpoint

    var body: some View {
        CopyableValueRow(value: endpoint.address) {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                HStack(spacing: SpectraLayout.Space.s) {
                    Text(AppLocalization.string(label)).font(.subheadline).foregroundStyle(.secondary)
                    if let holder = endpoint.holder {
                        EndpointHolderLabel(holder: holder)
                    } else if endpoint.isMine {
                        Text(AppLocalization.string("Mine")).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                            .padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.xs)
                            .spectraInsetFill()
                    }
                }
                Text(emphasizedEnds(endpoint.address)).font(.body.monospaced())
                    .lineLimit(1).truncationMode(.middle)
            }
        }
    }
}

/// A row that copies `value` when tapped. Its trailing glyph turns into a
/// checkmark for a moment; `.task(id:)` clears it and is cancelled with the
/// view.
private struct CopyableValueRow<Content: View>: View {
    let value: String
    @ViewBuilder let content: Content
    @State private var didCopy = false

    var body: some View {
        Button {
            UIPasteboard.general.string = value
            didCopy = true
            spectraHaptic(.light)
        } label: {
            HStack(spacing: SpectraLayout.Space.m) {
                content.frame(maxWidth: .infinity, alignment: .leading)
                Image(systemName: didCopy ? "checkmark" : "doc.on.doc")
                    .font(.subheadline.weight(.semibold)).foregroundStyle(.tint)
                    .contentTransition(.symbolEffect(.replace))
            }.contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityHint(AppLocalization.string(didCopy ? "Copied" : "Copy"))
        .task(id: didCopy) {
            guard didCopy else { return }
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            didCopy = false
        }
    }
}

/// `identifier` with its first and last characters primary and the middle
/// secondary. Middle truncation eats the secondary part first, so the ends
/// survive at any width.
private func emphasizedEnds(_ identifier: String) -> AttributedString {
    var text = AttributedString(identifier)
    let head = identifier.hasPrefix("0x") ? 6 : 4
    let tail = 4
    guard identifier.count > head + tail else {
        text.swiftUI.foregroundColor = .primary
        return text
    }
    text.swiftUI.foregroundColor = .secondary
    let headEnd = text.index(text.startIndex, offsetByCharacters: head)
    let tailStart = text.index(text.endIndex, offsetByCharacters: -tail)
    text[text.startIndex..<headEnd].swiftUI.foregroundColor = .primary
    text[tailStart..<text.endIndex].swiftUI.foregroundColor = .primary
    return text
}

