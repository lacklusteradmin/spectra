import SwiftUI
import UIKit

/// An address, key or list of addresses being typed: it wraps between its
/// actual characters, and layout never adds a hyphen that would read as part
/// of it. UIKit retains the original editable string, including copy and
/// selection. One address takes Return as done; a list takes it as a line.
@MainActor
struct AddressInput: View {
    @Binding var text: String
    @Binding var isFocused: Bool
    let prompt: String
    var allowsNewlines = false
    @ScaledMetric(relativeTo: .subheadline) private var fontSize: CGFloat = 15

    var body: some View {
        AddressTextView(text: $text, isFocused: $isFocused, prompt: prompt, fontSize: fontSize, allowsNewlines: allowsNewlines)
            .overlay(alignment: .topLeading) {
                if text.isEmpty {
                    Text(verbatim: prompt)
                        .font(.subheadline.monospaced())
                        .foregroundStyle(.tertiary)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
    }
}

@MainActor
private struct AddressTextView: UIViewRepresentable {
    @Binding var text: String
    @Binding var isFocused: Bool
    let prompt: String
    let fontSize: CGFloat
    let allowsNewlines: Bool

    func makeCoordinator() -> Coordinator { Coordinator(input: self) }

    func makeUIView(context: Context) -> UITextView {
        // The paragraph style below explicitly disables system hyphenation;
        // TextKit lays out the raw address by characters instead of words.
        let view = UITextView(usingTextLayoutManager: true)
        view.delegate = context.coordinator
        view.backgroundColor = .clear
        view.isScrollEnabled = false
        view.textContainerInset = .zero
        view.textContainer.lineFragmentPadding = 0
        view.textContainer.lineBreakMode = .byCharWrapping
        view.autocapitalizationType = .none
        view.autocorrectionType = .no
        view.spellCheckingType = .no
        view.smartDashesType = .no
        view.smartQuotesType = .no
        view.smartInsertDeleteType = .no
        view.returnKeyType = allowsNewlines ? .default : .done
        view.adjustsFontForContentSizeCategory = false
        view.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        applyText(to: view)
        return view
    }

    func updateUIView(_ view: UITextView, context: Context) {
        context.coordinator.input = self
        applyText(to: view)
        if isFocused && !view.isFirstResponder {
            view.becomeFirstResponder()
        } else if !isFocused && view.isFirstResponder {
            view.resignFirstResponder()
        }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UITextView, context: Context) -> CGSize? {
        guard let width = proposal.width, width > 0 else { return nil }
        let size = uiView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude))
        return CGSize(width: width, height: ceil(size.height))
    }

    private func applyText(to view: UITextView) {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineBreakMode = .byCharWrapping
        paragraph.hyphenationFactor = 0
        paragraph.usesDefaultHyphenation = false
        let attributes: [NSAttributedString.Key: Any] = [
            .font: UIFont.monospacedSystemFont(ofSize: fontSize, weight: .regular),
            .foregroundColor: UIColor.label,
            .paragraphStyle: paragraph,
        ]
        // Replacing attributedText on every keystroke resets the selection and
        // marked text. Only outside edits and Dynamic Type changes need it.
        if view.text != text || view.font?.pointSize != fontSize {
            let selection = view.selectedRange
            view.attributedText = NSAttributedString(string: text, attributes: attributes)
            let length = (text as NSString).length
            let location = min(selection.location, length)
            view.selectedRange = NSRange(location: location, length: min(selection.length, length - location))
            view.invalidateIntrinsicContentSize()
        }
        view.typingAttributes = attributes
        view.accessibilityLabel = prompt
    }

    @MainActor
    final class Coordinator: NSObject, UITextViewDelegate {
        var input: AddressTextView

        init(input: AddressTextView) { self.input = input }

        func textViewDidChange(_ textView: UITextView) {
            input.text = textView.text
            textView.invalidateIntrinsicContentSize()
        }

        func textViewDidBeginEditing(_ textView: UITextView) {
            if !input.isFocused { input.isFocused = true }
        }

        func textViewDidEndEditing(_ textView: UITextView) {
            if input.isFocused { input.isFocused = false }
        }
    }
}
