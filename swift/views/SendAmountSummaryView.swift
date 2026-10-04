import SwiftUI

/// Amounts supplied by the reviewed preview or immutable artifact, without a new quote.
struct SendAmountSummaryView: View {
    let artworkName: String?
    let symbol: String
    let chain: Chain
    let amount: String
    var fiatText: String? = nil
    var statusTitle: String? = nil
    var statusSystemImage: String? = nil
    var statusColor: Color = .accentColor
    var isReceipt = false

    var body: some View {
        VStack(spacing: SpectraLayout.Space.m) {
            if isReceipt, let statusTitle, let statusSystemImage {
                Image(systemName: statusSystemImage)
                    .font(.title.weight(.semibold))
                    .foregroundStyle(statusColor)
                    .frame(width: 64, height: 64)
                    .spectraInsetFill()
                    .accessibilityHidden(true)
                Text(AppLocalization.string(statusTitle))
                    .font(.title2.weight(.bold))
                    .multilineTextAlignment(.center)
            } else {
                networkLabel
            }

            // Every digit remains available during signing review, including amounts
            // too long for one line at the reader's chosen text size.
            Text("\(Text(verbatim: AmountPresentation.localizedDecimal(amount)).font(.largeTitle.weight(.semibold))) \(Text(verbatim: symbol).font(.title2.weight(.medium)).foregroundStyle(.secondary))")
            .monospacedDigit()
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityIdentifier("send.review.amount")

            if let fiatText {
                Text(verbatim: fiatText)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }

            if isReceipt {
                networkLabel
            } else if let statusTitle, let statusSystemImage {
                Label(AppLocalization.string(statusTitle), systemImage: statusSystemImage)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(statusColor)
                    .padding(.horizontal, SpectraLayout.Space.s)
                    .padding(.vertical, SpectraLayout.Space.xs)
                    .spectraInsetFill(cornerRadius: SpectraLayout.Radius.control)
                    .accessibilityIdentifier("send.review.status")
            }
        }
        .frame(maxWidth: .infinity)
        .padding(SpectraLayout.Space.xl)
        .spectraElevatedFill()
    }

    private var networkLabel: some View {
        HStack(spacing: SpectraLayout.Space.s) {
            CoinBadge(
                artworkName: artworkName, fallbackText: symbol,
                color: chain.entry?.color.color ?? .gray, size: 24)
                .accessibilityHidden(true)
            Text(verbatim: chain.displayName)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
    }
}
