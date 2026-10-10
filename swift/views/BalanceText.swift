import SwiftUI

/// A balance — an amount held, or what it is worth — or a mask while Hide
/// Balances is on. The mask reads "Hidden" to VoiceOver, not six bullets.
/// A price is public and never goes through this.
struct BalanceText: View {
    let text: String
    let isHidden: Bool

    var body: some View {
        if isHidden {
            Text(verbatim: "••••••").accessibilityLabel(AppLocalization.string("Hidden"))
        } else {
            Text(verbatim: text)
        }
    }
}
