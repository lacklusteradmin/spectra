import SwiftUI

/// A transaction's status as a pill: its word and a symbol, never colour
/// alone. `compact` is the list row's, beside the title.
struct TransactionStatusBadge: View {
    let status: TransactionStatus
    var compact = false
    private var statusColor: Color { Color.spectraTransactionStatusColor(status) }
    private var systemImage: String {
        switch status {
        case .pending: return "clock"
        case .confirmed: return "checkmark.circle.fill"
        case .failed: return "xmark.octagon.fill"
        }
    }
    var body: some View {
        Label(status.localizedTitle, systemImage: systemImage)
            .font((compact ? Font.caption2 : Font.caption).weight(.semibold))
            .lineLimit(1)
            .padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, compact ? SpectraLayout.Space.xxs : SpectraLayout.Space.xs)
            .background(statusColor.opacity(0.16), in: Capsule())
            .foregroundStyle(statusColor)
            .fixedSize()
    }
}
