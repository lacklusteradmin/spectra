import Foundation
import UserNotifications

/// What this platform has for telling the user something: a notification and
/// the send Live Activity.
///
/// Core decides what is worth telling the user — a price alert, a large
/// portfolio movement, a send reaching a terminal status — and logs it. No view
/// reads anything here, so nothing is kept: the notification center holds its
/// requests and ActivityKit the running activities.
@MainActor
struct PlatformNotifications {
    let bridge: WalletServiceBridge
    let diagnostics: WalletDiagnosticsState // Where a failed delivery is logged.

    func requestPermission() {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound]) { _, _ in }
    }
    /// A send asks only when an alert that needs the permission is on.
    func requestPermissionAfterSend(settings: AppSettings) {
        guard settings.useTransactionStatusNotifications || settings.useLargeMovementNotifications else { return }
        requestPermission()
    }

    private func post(identifier: String, title: String, body: String) async {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        let request = UNNotificationRequest(identifier: identifier, content: content, trigger: nil)
        do { try await UNUserNotificationCenter.current().add(request) }
        catch { diagnostics.appendOperationalLog(category: "Notifications", message: error.localizedDescription) }
    }

    func deliverPriceAlerts(_ notifications: [PriceAlertNotification], amounts: AmountPresentation) async {
        for notification in notifications { await postPriceAlert(notification, amounts: amounts) }
    }
    /// One sentence per condition, so each language words both directions.
    private func postPriceAlert(_ notification: PriceAlertNotification, amounts: AmountPresentation) async {
        let template: String
        switch notification.condition {
        case .above: template = "%@ on %@ is now %@, above your target of %@."
        case .below: template = "%@ on %@ is now %@, below your target of %@."
        }
        await post(
            identifier: "price-alert-\(notification.id)-\(UUID().uuidString)",
            title: AppLocalization.format("%@ price alert", notification.symbol),
            body: AppLocalization.format(
                template, notification.assetDisplayName, notification.chainId.displayName,
                amounts.formattedFiat(notification.livePrice, currency: notification.currency),
                amounts.formattedFiat(notification.targetPrice, currency: notification.currency)))
    }

    func deliverPortfolioMovement(_ evaluation: LargeMovementEvaluation, amounts: AmountPresentation) async {
        let percent = evaluation.ratio.formatted(.percent.precision(.fractionLength(0)).locale(AppLocalization.locale))
        await post(
            identifier: "portfolio-movement-\(UUID().uuidString)",
            title: AppLocalization.string("Large portfolio movement detected"),
            body: AppLocalization.format(
                evaluation.directionUp
                    ? "Your portfolio rose by %@ (%@) since the last sync."
                    : "Your portfolio fell by %@ (%@) since the last sync.",
                amounts.formattedFiat(evaluation.absoluteDelta, currency: evaluation.currency), percent))
    }

    /// Deliver native effects after the caller adopts the transaction projection.
    /// A Live Activity follows every change; a notification only what core
    /// says is worth one.
    func deliverPendingStatusChanges(_ changes: [TransactionStatusChange], amounts: AmountPresentation) async {
        for change in changes where change.statusChanged {
            guard let transaction = try? await bridge.ready().transaction(id: change.id) else { continue }
            if change.notify { await postTransactionStatus(transaction, newStatus: change.newStatus) }
            await finishSendLiveActivity(for: transaction, newStatus: change.newStatus, amounts: amounts)
        }
    }
    private func postTransactionStatus(_ transaction: TransactionRecord, newStatus: TransactionStatus) async {
        guard let body = transaction.sendOutcomeDetail(for: newStatus) else { return }
        let title = newStatus == .confirmed
            ? AppLocalization.format("%@ transaction confirmed", transaction.symbol)
            : AppLocalization.format("%@ transaction failed", transaction.symbol)
        await post(identifier: "transaction-status-\(transaction.id)-\(newStatus)", title: title, body: body)
    }
}
