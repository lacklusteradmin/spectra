import SwiftUI
import UIKit

/// One word of a recovery phrase being typed. UIKit rather than `TextField`
/// so every keyboard aid that reads or remembers what is typed is off:
/// autocorrection, spell checking and inline prediction, which together keep
/// the QuickType bar — and the keyboard's learned words — away from a seed.
/// The word fields of one phrase, by slot, so a space typed in one hands the
/// keyboard to the next in the same event. Through SwiftUI state the handoff
/// took a pass or two, and keys typed in between reached no field and were
/// lost.
@MainActor
final class SecretWordFieldGroup {
    private struct Entry { weak var view: UITextField? }
    private var fields: [Int: Entry] = [:]

    fileprivate func register(_ view: UITextField, at index: Int) {
        fields[index] = Entry(view: view)
    }

    /// Make slot `index` the field being typed in, now; `false` when it has
    /// no field yet, and the focus waits for the next update.
    @discardableResult
    func focus(_ index: Int) -> Bool {
        guard let view = fields[index]?.view, view.window != nil else { return false }
        return view.isFirstResponder || view.becomeFirstResponder()
    }
}

@MainActor
struct SecretWordField: UIViewRepresentable {
    @Binding var text: String
    let isFocused: Bool
    let accessibilityLabel: String
    var isInvalid = false
    /// The phrase's fields, and this one's slot among them.
    var group: SecretWordFieldGroup? = nil
    var index = 0
    /// The field became the one being typed in.
    let onFocus: () -> Void
    /// Return was pressed: move on.
    let onSubmit: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(field: self) }

    func makeUIView(context: Context) -> UITextField {
        let view = UITextField()
        view.delegate = context.coordinator
        view.autocorrectionType = .no
        view.spellCheckingType = .no
        view.inlinePredictionType = .no
        view.autocapitalizationType = .none
        view.smartDashesType = .no
        view.smartQuotesType = .no
        view.smartInsertDeleteType = .no
        view.returnKeyType = .next
        view.adjustsFontForContentSizeCategory = true
        view.font = UIFontMetrics(forTextStyle: .callout).scaledFont(
            for: .monospacedSystemFont(ofSize: UIFont.preferredFont(forTextStyle: .callout).pointSize, weight: .medium))
        view.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        view.addTarget(context.coordinator, action: #selector(Coordinator.changed(_:)), for: .editingChanged)
        group?.register(view, at: index)
        return view
    }

    func updateUIView(_ view: UITextField, context: Context) {
        context.coordinator.field = self
        group?.register(view, at: index)
        if view.text != text { view.text = text }
        view.textColor = isInvalid ? .systemRed : .label
        view.accessibilityLabel = accessibilityLabel
        if isFocused, !view.isFirstResponder {
            // Not during the update pass that asked for it — and only if this
            // field is still the one asked for when the task runs. Typing
            // faster than focus moves queues one of these per field; a stale
            // one pulled focus back to a slot already left, and the words
            // typed meanwhile were lost.
            let coordinator = context.coordinator
            Task { @MainActor in
                guard coordinator.field.isFocused, !view.isFirstResponder else { return }
                view.becomeFirstResponder()
            }
        }
    }

    @MainActor
    final class Coordinator: NSObject, UITextFieldDelegate {
        var field: SecretWordField
        init(field: SecretWordField) { self.field = field }

        @objc func changed(_ view: UITextField) { field.text = view.text ?? "" }

        func textFieldDidBeginEditing(_ textField: UITextField) { field.onFocus() }

        func textFieldShouldReturn(_ textField: UITextField) -> Bool {
            field.onSubmit()
            return false
        }
    }
}
