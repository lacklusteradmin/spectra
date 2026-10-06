import ActivityKit
import Foundation

// The send Live Activity: started when a broadcast is accepted, ended when core
// says the transaction reached a terminal status. The extension target renders
// `SendTransactionLiveActivityAttributes`; the three moments below are the
// whole lifecycle.
//
// No status is decided here. Core owns whether a transaction is pending,
// confirmed or failed; this turns the status it already reports into the
// phase, text and truncations a lock-screen row shows.

// MARK: - Content

/// Middle-truncate a long identifier so both ends stay readable on one line.
///
/// A reader checking an address or a hash reads its ends, and every row in the
/// activity is one line wide.
func sendLiveActivityPreview(_ value: String, keepingEachEnd keep: Int) -> String {
    guard value.count > keep * 2 + 1 else { return value }
    return "\(value.prefix(keep))…\(value.suffix(keep))"
}

/// What the activity shows for `transaction` in `phase`.
///
/// `amountText` is passed in rather than formatted here: the asset's precision
/// is a store lookup, and this stays a pure function of the record.
func sendLiveActivityContentState(
    for transaction: TransactionRecord,
    phase: SendTransactionLiveActivityAttributes.ContentState.Phase,
    amountText: String
) -> SendTransactionLiveActivityAttributes.ContentState {
    let statusText: String
    let detailText: String
    switch phase {
    case .sending:
        statusText = AppLocalization.string("Sending")
        detailText = AppLocalization.format("Waiting for %@ to confirm this send.", transaction.chainName)
    case .complete:
        statusText = AppLocalization.string("Sent")
        detailText = transaction.sendOutcomeDetail(for: .confirmed) ?? ""
    case .failed:
        statusText = AppLocalization.string("Send failed")
        detailText = transaction.sendOutcomeDetail(for: .failed) ?? ""
    }
    return SendTransactionLiveActivityAttributes.ContentState(
        phase: phase,
        walletName: transaction.walletName,
        chainName: transaction.chainName,
        symbol: transaction.symbol,
        amountText: amountText,
        statusText: statusText,
        detailText: detailText,
        destinationPreview: sendLiveActivityPreview(transaction.address, keepingEachEnd: 6),
        transactionHashPreview: transaction.transactionHash.map {
            sendLiveActivityPreview($0, keepingEachEnd: 8)
        },
        startedAt: transaction.createdDate)
}

/// A send that never resolves should read as stale rather than keep claiming to
/// be in flight; the system dims the activity once this much time has passed
/// with no update.
private let sendLiveActivityStaleAfter: TimeInterval = 60 * 60

/// How long a finished activity stays on the lock screen before the system
/// clears it, so the outcome is still there when the phone is next picked up.
private let sendLiveActivityLingerAfterFinish: TimeInterval = 60 * 5

// MARK: - The running activities

/// Start, end and enumerate the send activities.
///
/// Stateless on purpose. ActivityKit already holds the running activities and
/// hands them back after a relaunch, so a Swift-side dictionary of handles
/// would be a second copy of that list which any termination silently
/// invalidates — which is how a "sending" row survives its own transaction.
enum SendLiveActivityStore {
    private static var running: [Activity<SendTransactionLiveActivityAttributes>] {
        Activity<SendTransactionLiveActivityAttributes>.activities
    }

    static var runningTransactionIds: Set<String> {
        Set(running.map(\.attributes.transactionId))
    }

    private static func activity(for transactionId: String) -> Activity<
        SendTransactionLiveActivityAttributes
    >? {
        running.first { $0.attributes.transactionId == transactionId }
    }

    /// Ask the system to show an activity for `transactionId`.
    ///
    /// Silent when the user has Live Activities switched off, and a no-op when
    /// one is already running for this transaction — a resubmitted send must
    /// not stack two rows for the same record.
    static func start(
        transactionId: String, state: SendTransactionLiveActivityAttributes.ContentState
    ) {
        guard ActivityAuthorizationInfo().areActivitiesEnabled else { return }
        guard activity(for: transactionId) == nil else { return }
        _ = try? Activity.request(
            attributes: SendTransactionLiveActivityAttributes(transactionId: transactionId),
            content: ActivityContent(
                state: state, staleDate: Date().addingTimeInterval(sendLiveActivityStaleAfter)))
    }

    /// Show the outcome and let the system retire the activity.
    ///
    /// `state` is `nil` when the record behind the activity is gone, which
    /// leaves nothing true to display and dismisses it at once.
    static func end(
        transactionId: String,
        state: SendTransactionLiveActivityAttributes.ContentState?,
        lingering: Bool
    ) async {
        guard let activity = activity(for: transactionId) else { return }
        let content = state.map { ActivityContent(state: $0, staleDate: nil) }
        let policy: ActivityUIDismissalPolicy =
            lingering
            ? .after(Date().addingTimeInterval(sendLiveActivityLingerAfterFinish)) : .immediate
        await activity.end(content, dismissalPolicy: policy)
    }
}

// MARK: - The three moments

extension PlatformNotifications {
    private func sendLiveActivityState(
        for transaction: TransactionRecord,
        phase: SendTransactionLiveActivityAttributes.ContentState.Phase,
        amounts: AmountPresentation
    ) -> SendTransactionLiveActivityAttributes.ContentState {
        sendLiveActivityContentState(
            for: transaction, phase: phase,
            amountText: amounts.formattedAssetAmountValue(
                transaction.amount, deploymentId: transaction.deploymentId))
    }

    /// A broadcast was accepted and the transaction is waiting on the chain.
    func startSendLiveActivity(for transaction: TransactionRecord, amounts: AmountPresentation) {
        guard transaction.isSubmittedOperation, transaction.status == .pending else { return }
        SendLiveActivityStore.start(
            transactionId: transaction.id,
            state: sendLiveActivityState(for: transaction, phase: .sending, amounts: amounts))
    }

    /// Core reported a terminal status for a transaction.
    func finishSendLiveActivity(
        for transaction: TransactionRecord, newStatus: TransactionStatus, amounts: AmountPresentation
    ) async {
        let phase: SendTransactionLiveActivityAttributes.ContentState.Phase
        switch newStatus {
        case .confirmed: phase = .complete
        case .failed: phase = .failed
        case .pending: return
        }
        await SendLiveActivityStore.end(
            transactionId: transaction.id,
            state: sendLiveActivityState(for: transaction, phase: phase, amounts: amounts), lingering: true)
    }

    /// Retire activities whose transaction stopped being in flight while the app
    /// was not running to see it.
    ///
    /// Without this, a send that confirmed after the app was terminated leaves a
    /// spinner on the lock screen until the staleness date passes.
    func reconcileSendLiveActivities(amounts: AmountPresentation) async {
        let runningIds = SendLiveActivityStore.runningTransactionIds
        guard !runningIds.isEmpty else { return }
        await reconcileSendActivities(transactionIds: runningIds,
            lookup: { try await bridge.ready().transaction(id: $0) },
            finish: { id, transaction in
                if let transaction {
                    await finishSendLiveActivity(for: transaction, newStatus: transaction.status, amounts: amounts)
                } else {
                    await SendLiveActivityStore.end(transactionId: id, state: nil, lingering: false)
                }
            }, failed: {
                diagnostics.appendOperationalLog(.error, category: "Live Activity", message: $0.localizedDescription)
            })
    }
}

/// Reconcile native activities by their stored IDs. A read failure retains the
/// activity for retry; an absent record removes it without inventing a status.
@MainActor
func reconcileSendActivities(
    transactionIds: Set<String>,
    lookup: (String) async throws -> TransactionRecord?,
    finish: (String, TransactionRecord?) async -> Void,
    failed: (Error) -> Void
) async {
    for id in transactionIds {
        do {
            let transaction = try await lookup(id)
            guard transaction?.status != .pending else { continue }
            await finish(id, transaction)
        } catch { failed(error) }
    }
}
