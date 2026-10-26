import SwiftUI

/// One card holding a list, one row per element, with inset dividers between
/// rows — the list the home and history tabs draw.
///
/// A list is rows in a card, never a card per row: a card of cards stacks
/// glass on glass, and a column of cards spends a card's padding and gap on
/// every row. Each row applies `spectraRowPadding()` inside its own button
/// label, so the padding is part of the tap target.
///
/// The header is a title with an optional trailing count, or a view of the
/// caller's own (the home card's page switch). The footer sits under the
/// rows, inside the card: an empty state, a loading row, or an action on the
/// list as a whole. It draws its own divider when it wants one.
struct SpectraRowGroup<Data: RandomAccessCollection, Row: View, Header: View, Footer: View>: View
where Data.Element: Identifiable {
    var title: String? = nil
    /// Right of the title, such as a count.
    var trailing: String? = nil
    let data: Data
    /// Where a divider starts. The default runs it under the text of a row
    /// that leads with a 36pt badge.
    var dividerInset: CGFloat = SpectraLayout.rowDividerInset
    @ViewBuilder var header: Header
    @ViewBuilder var footer: Footer
    @ViewBuilder let row: (Data.Element) -> Row

    var body: some View {
        VStack(spacing: 0) {
            if let title {
                HStack(spacing: SpectraLayout.Space.s) {
                    Text(title).font(.headline)
                    Spacer()
                    if let trailing {
                        Text(trailing).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary).monospacedDigit()
                    }
                }
                .padding(.horizontal, SpectraLayout.rowHorizontal)
                .padding(.vertical, SpectraLayout.cardHeaderVertical)
                Divider().opacity(0.25)
            } else if Header.self != EmptyView.self {
                header
                Divider().opacity(0.25)
            }
            if !data.isEmpty {
                // Lazy so a long list, such as the wiki's, builds only the rows on screen.
                LazyVStack(spacing: 0) {
                    ForEach(Array(data.enumerated()), id: \.element.id) { index, element in
                        row(element)
                        if index < data.count - 1 {
                            Divider().padding(.leading, dividerInset).opacity(0.25)
                        }
                    }
                }
                .padding(.vertical, SpectraLayout.Space.xs)
            }
            footer
        }
        .frame(maxWidth: .infinity)
        .spectraCardFill()
    }
}

extension SpectraRowGroup where Header == EmptyView, Footer == EmptyView {
    init(
        title: String? = nil, trailing: String? = nil, data: Data,
        dividerInset: CGFloat = SpectraLayout.rowDividerInset,
        @ViewBuilder row: @escaping (Data.Element) -> Row
    ) {
        self.init(
            title: title, trailing: trailing, data: data, dividerInset: dividerInset,
            header: { EmptyView() }, footer: { EmptyView() }, row: row)
    }
}

extension SpectraRowGroup where Header == EmptyView {
    init(
        title: String? = nil, trailing: String? = nil, data: Data,
        dividerInset: CGFloat = SpectraLayout.rowDividerInset,
        @ViewBuilder footer: () -> Footer,
        @ViewBuilder row: @escaping (Data.Element) -> Row
    ) {
        self.init(
            title: title, trailing: trailing, data: data, dividerInset: dividerInset,
            header: { EmptyView() }, footer: footer, row: row)
    }
}

/// A `SpectraRowGroup` of fixed rows rather than data: each subview of
/// `content` is a row, with the same card, header and inset dividers. The
/// settings tab is a column of these. A footer, when given, explains the
/// card from under it, outside the glass.
struct SpectraRowSection<Content: View>: View {
    var title: String? = nil
    var footer: String? = nil
    var dividerInset: CGFloat = SpectraLayout.rowDividerInset
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            VStack(spacing: 0) {
                if let title {
                    Text(title).font(.headline)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, SpectraLayout.rowHorizontal)
                        .padding(.vertical, SpectraLayout.cardHeaderVertical)
                    Divider().opacity(0.25)
                }
                Group(subviews: content) { rows in
                    VStack(spacing: 0) {
                        ForEach(rows) { row in
                            row
                            if row.id != rows.last?.id {
                                Divider().padding(.leading, dividerInset).opacity(0.25)
                            }
                        }
                    }
                    .padding(.vertical, SpectraLayout.Space.xs)
                }
            }
            .frame(maxWidth: .infinity)
            .spectraCardFill()
            if let footer {
                Text(footer).font(.footnote).foregroundStyle(.secondary)
                    .padding(.horizontal, SpectraLayout.rowHorizontal)
            }
        }
    }
}
