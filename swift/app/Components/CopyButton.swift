import SwiftUI
import UIKit

/// Copies `value` and says so for a moment: the glyph turns to a checkmark,
/// and VoiceOver hears "Copied". With a title it is a glass button, drawn as
/// its `Style` says; without, a tinted glyph with a full-size tap target, for
/// the end of a row. Given row content, the whole row copies and ends in that
/// glyph.
struct CopyButton<Row: View>: View {
    /// Where a titled button sits.
    enum Style {
        /// Beside a heading.
        case compact
        /// Across its card.
        case wide
        /// Across its card as the page's main action, as tall as the
        /// buttons beside it.
        case prominent
    }

    let value: String
    private let title: String?
    private let style: Style
    private let row: Row?
    @State private var didCopy = false

    var body: some View {
        button
            .task(id: didCopy) {
                guard didCopy else { return }
                try? await Task.sleep(for: .seconds(1.5))
                guard !Task.isCancelled else { return }
                didCopy = false
            }
    }

    @ViewBuilder
    private var button: some View {
        if let row {
            Button(action: copy) {
                HStack(spacing: SpectraLayout.Space.m) {
                    row
                    Spacer(minLength: SpectraLayout.Space.s)
                    glyphImage
                }
                .spectraRowPadding()
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityHint(spokenState)
        } else if let title {
            switch style {
            case .compact:
                Button(action: copy) {
                    titledLabel(title).font(.caption.weight(.semibold))
                }
                .buttonStyle(.glass).tint(.accentColor)
            case .wide:
                Button(action: copy) {
                    titledLabel(title).font(.subheadline.weight(.semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, SpectraLayout.Space.s)
                }
                .buttonStyle(.glass)
            case .prominent:
                Button(action: copy) {
                    titledLabel(title).font(.headline)
                        .frame(maxWidth: .infinity)
                        .frame(minHeight: 46)
                }
                .buttonStyle(.glassProminent)
            }
        } else {
            Button(action: copy) {
                glyphImage
                    .frame(minWidth: 44, minHeight: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(spokenState)
        }
    }

    private func titledLabel(_ title: String) -> some View {
        Label(AppLocalization.string(didCopy ? "Copied" : title), systemImage: glyph)
            .contentTransition(.symbolEffect(.replace))
    }

    private var glyphImage: some View {
        Image(systemName: glyph).font(.subheadline.weight(.semibold)).foregroundStyle(.tint)
            .contentTransition(.symbolEffect(.replace))
    }

    private var glyph: String { didCopy ? "checkmark" : "doc.on.doc" }

    private var spokenState: String { AppLocalization.string(didCopy ? "Copied" : "Copy") }

    private func copy() {
        UIPasteboard.general.string = value
        didCopy = true
        spectraHaptic(.light)
    }
}

extension CopyButton where Row == EmptyView {
    /// A glass button when titled, else a glyph for the end of a row.
    init(value: String, title: String? = nil, style: Style = .compact) {
        self.value = value
        self.title = title
        self.style = style
        self.row = nil
    }
}

extension CopyButton {
    /// A row of a `SpectraRowGroup` that copies wherever it is tapped and
    /// ends in the glyph.
    init(value: String, @ViewBuilder row: () -> Row) {
        self.value = value
        self.title = nil
        self.style = .compact
        self.row = row()
    }
}
