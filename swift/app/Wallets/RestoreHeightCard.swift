import SwiftUI

/// Where a restored or watched wallet's scan starts: a Monero wallet's, or a
/// Zcash wallet's for its shielded funds. Blank reads a Polyseed's birthday,
/// or scans from the first block the wallet could have received at.
struct RestoreHeightCard: View {
    @Bindable var draft: WalletImportDraft

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            Text(AppLocalization.string("Restore Height (Optional)")).font(.subheadline.weight(.semibold))
            TextField(AppLocalization.string("Block height"), text: $draft.restoreHeightInput)
                .keyboardType(.numberPad).font(.body.monospacedDigit())
                .padding(SpectraLayout.Space.m).spectraInputFieldStyle()
            if draft.isRestoreHeightValid {
                Text(AppLocalization.string(
                    "The scan starts here and cannot find funds received earlier. Leave it blank to use the phrase's creation date when it records one (a Polyseed), or to scan from the first block the wallet could have received at, which takes longest."
                )).font(.caption).foregroundStyle(.secondary)
            } else {
                Text(AppLocalization.string("A restore height is a whole block number.")).font(.caption)
                    .foregroundStyle(.red.opacity(0.9))
            }
        }
        .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
    }
}
