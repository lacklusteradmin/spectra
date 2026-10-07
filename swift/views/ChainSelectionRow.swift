import SwiftUI

/// One chain in a picker: badge, name, tags, gas token and a trailing mark.
///
/// Drawn inside a `SpectraRowGroup`, by the setup page's short list and by the
/// full list behind it, so the two cannot drift into different shapes.
struct ChainSelectionRow: View {
    /// What sits at the end of the row.
    enum Accessory {
        /// A choice: the chosen row is ticked.
        case checkmark
        /// A way into the chain's own page.
        case disclosure
    }

    let descriptor: ChainSelectionDescriptor
    let isSelected: Bool
    var accessory: Accessory = .checkmark
    let toggle: () -> Void

    var body: some View {
        Button {
            spectraHaptic(.light)
            toggle()
        } label: {
            HStack(spacing: SpectraLayout.Space.m) {
                CoinBadge(
                    artworkName: descriptor.artworkName, fallbackText: descriptor.symbol,
                    color: descriptor.color, size: 36
                )
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(descriptor.title)
                        .font(.body.weight(.semibold))
                        .foregroundStyle(Color.primary)
                        .lineLimit(1)
                    Text(descriptor.tagLine)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: SpectraLayout.Space.s)
                Text(descriptor.symbol)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, SpectraLayout.Space.s)
                    .padding(.vertical, SpectraLayout.Space.xs)
                    .background(Capsule(style: .continuous).fill(SpectraLayout.insetFill))
                selectionMark
            }
            .spectraRowPadding()
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    @ViewBuilder
    private var selectionMark: some View {
        switch accessory {
        case .checkmark:
            if isSelected {
                Image(systemName: "checkmark.circle.fill").font(.title3).foregroundStyle(.tint)
            }
        case .disclosure:
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }
    }
}
