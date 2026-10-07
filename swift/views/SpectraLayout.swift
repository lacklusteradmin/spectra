import SwiftUI

/// Shared layout and Liquid Glass tokens. [docs/IOS-UI.md](../../docs/IOS-UI.md)
/// is the authority for every value here; this file is where the document is
/// spelled in Swift, so a screen never restates a number the document owns.
enum SpectraLayout {
    /// The spacing scale. Every padding, stack spacing and spacer minimum in a
    /// view is one of these steps; the named layout values below are steps too.
    enum Space {
        /// Between two lines of one text pair (title over subtitle).
        static let xxs: CGFloat = 2
        static let xs: CGFloat = 4
        static let s: CGFloat = 8
        static let m: CGFloat = 12
        static let l: CGFloat = 16
        static let xl: CGFloat = 24
        static let xxl: CGFloat = 32
    }

    static let screenHorizontal: CGFloat = Space.l
    static let screenTop: CGFloat = Space.s
    static let screenBottom: CGFloat = Space.l
    static let sectionSpacing: CGFloat = Space.m
    static let cardPadding: CGFloat = Space.l
    static let cardHeaderVertical: CGFloat = Space.m
    static let rowHorizontal: CGFloat = Space.l
    static let rowVertical: CGFloat = Space.s
    /// Where a row divider starts: the row inset, a 36pt badge and the gap
    /// after it, so the line runs under the text rather than the badge.
    static let rowDividerInset: CGFloat = rowHorizontal + 36 + Space.m

    /// Corner radii. Three steps, named for the surface: a surface nested
    /// inside another takes the next step down.
    enum Radius {
        /// Cards of every kind: tab and hero cards, detail cards, row cards
        /// and inline notice banners.
        static let card: CGFloat = 20
        /// Surfaces inside a card: inputs, address blocks, chips and pills.
        static let inner: CGFloat = 14
        /// Dense controls, icon backplates and single-character slots.
        static let control: CGFloat = 10
    }

    /// Liquid Glass tints. Two neutral steps and no third: a card is either
    /// elevated or it is content. Coloured glass (an accent, warning or red
    /// notice) carries its own colour and is not one of these. Glass stops at
    /// the card: nothing inside a card is glass.
    enum GlassTint {
        /// Hero and header cards, and the bottom action bar.
        static let elevated: Color = .white.opacity(0.04)
        /// Ordinary content cards.
        static let content: Color = .white.opacity(0.03)
    }

    /// The flat fill of a surface inside a card — an input, an address
    /// block, a chip, a tile, an icon backplate. It recesses into the card
    /// rather than floating over it, which is what a second layer of glass
    /// did.
    static let insetFill: Color = .primary.opacity(0.06)
}

/// Semantic colours that must not follow the theme. The theme colour is the
/// asset catalog's AccentColor, written `.tint` or `Color.accentColor`; a
/// warning has to read as a warning whatever that colour is, so it is fixed
/// here. Declared on `ShapeStyle` so `.spectraWarning` works both where a
/// `Color` and where any shape style is expected.
extension ShapeStyle where Self == Color {
    /// Pending, in-progress and incomplete states, and warnings.
    static var spectraWarning: Color { .orange }
}

/// `text` with a zero-width space after every character, for display only.
/// Text hyphenates an unbroken run to wrap it, and a hyphen inside a hash,
/// payload or address reads as part of it (a TON address can hold a real
/// one); with a break allowed everywhere, lines wrap at the edge without one.
/// Copying must go through the original string.
func breakableAnywhere(_ text: String) -> String {
    text.map(String.init).joined(separator: "\u{200B}")
}

extension View {
    /// The screen inset every scrolling page uses, top-level tab or detail.
    func spectraScreenPadding() -> some View {
        padding(.horizontal, SpectraLayout.screenHorizontal)
            .padding(.top, SpectraLayout.screenTop)
            .padding(.bottom, SpectraLayout.screenBottom)
    }

    func spectraNumericTextLayout(minimumScaleFactor: CGFloat = 0.62) -> some View {
        lineLimit(1).minimumScaleFactor(minimumScaleFactor).allowsTightening(true)
    }

    /// Ordinary content card: the content tint on the card radius.
    ///
    /// Neither glass fill takes a radius. A card is the only glass surface,
    /// and it always has the card radius; a smaller radius would mean a
    /// surface nested inside a card, which takes `spectraInsetFill`.
    func spectraCardFill() -> some View {
        glassEffect(.regular.tint(SpectraLayout.GlassTint.content), in: .rect(cornerRadius: SpectraLayout.Radius.card))
    }

    /// Elevated card: hero and header cards.
    func spectraElevatedFill() -> some View {
        glassEffect(.regular.tint(SpectraLayout.GlassTint.elevated), in: .rect(cornerRadius: SpectraLayout.Radius.card))
    }

    /// A surface inside a card: flat, not glass. See `SpectraLayout.insetFill`.
    func spectraInsetFill(cornerRadius: CGFloat = SpectraLayout.Radius.inner) -> some View {
        background(SpectraLayout.insetFill, in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
    }

    /// A selectable surface inside a card: the inset fill, or an opaque
    /// accent fill when selected. The selected label is white text, so the
    /// fill behind it has to stay opaque to keep the label legible.
    @ViewBuilder
    func spectraSelectableFill(isSelected: Bool, accent: Color, cornerRadius: CGFloat) -> some View {
        if isSelected {
            background(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous).fill(accent))
        } else {
            spectraInsetFill(cornerRadius: cornerRadius)
        }
    }

    /// The inset and minimum height of a row in a `SpectraRowGroup`. Applied
    /// inside a row's button label, so the whole row is the tap target.
    func spectraRowPadding() -> some View {
        padding(.horizontal, SpectraLayout.rowHorizontal)
            .padding(.vertical, SpectraLayout.rowVertical)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .contentShape(Rectangle())
    }
}
