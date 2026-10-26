import SwiftUI

/// The fee and what leaves the wallet, under a transfer's parties: one style
/// on the review and on the built transaction, so the same fee does not read
/// as two different things a page apart.
struct SendCostRows: View {
    let fee: String
    /// The amount and the fee together, when the fee is paid in the sent asset.
    var total: String? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            SendSummaryRow(label: "Network Fee", value: fee)
            if let total {
                SendSummaryRow(label: "Total", value: total)
            }
        }
    }
}

/// Label and value side by side while they fit, stacked when they do not:
/// every digit of a fee stays on screen at any text size.
struct SendSummaryRow: View {
    let label: String
    let value: String

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.m) {
                Text(AppLocalization.string(label)).font(.subheadline).foregroundStyle(.secondary)
                Spacer(minLength: SpectraLayout.Space.s)
                Text(value).font(.subheadline.weight(.semibold)).multilineTextAlignment(.trailing)
                    .fixedSize()
            }
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(AppLocalization.string(label)).font(.subheadline).foregroundStyle(.secondary)
                Text(value).font(.subheadline.weight(.semibold)).fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
    }
}
