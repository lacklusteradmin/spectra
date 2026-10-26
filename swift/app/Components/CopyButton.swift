import SwiftUI
import UIKit

/// Copies `value` and says so for a moment: the glyph turns to a checkmark,
/// and VoiceOver hears "Copied". With a title it is a glass button; without,
/// a tinted glyph with a full-size tap target, for the end of a row.
struct CopyButton: View {
    let value: String
    var title: String? = nil
    @State private var didCopy = false

    var body: some View {
        Group {
            if let title {
                Button(action: copy) {
                    Label(AppLocalization.string(didCopy ? "Copied" : title), systemImage: glyph)
                        .font(.caption.weight(.semibold))
                        .contentTransition(.symbolEffect(.replace))
                }
                .buttonStyle(.glass).tint(.accentColor)
            } else {
                Button(action: copy) {
                    Image(systemName: glyph).font(.subheadline.weight(.semibold)).foregroundStyle(.tint)
                        .contentTransition(.symbolEffect(.replace))
                        .frame(minWidth: 44, minHeight: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(AppLocalization.string(didCopy ? "Copied" : "Copy"))
            }
        }
        .task(id: didCopy) {
            guard didCopy else { return }
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            didCopy = false
        }
    }

    private var glyph: String { didCopy ? "checkmark" : "doc.on.doc" }

    private func copy() {
        UIPasteboard.general.string = value
        didCopy = true
        spectraHaptic(.light)
    }
}
