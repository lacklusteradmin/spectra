import SwiftUI

/// The destination tag or memo a payment carries to a recipient that shares
/// its account, such as an exchange's deposit address. Offered where core's
/// `paymentMemoKinds(chain:)` names any; core checks the value and refuses a
/// send without one to an account that asks for it.
struct SendPaymentMemoField: View {
    let kinds: [PaymentMemoKind]
    @Binding var kind: PaymentMemoKind?
    @Binding var text: String

    private var selected: PaymentMemoKind { kind.flatMap { kinds.contains($0) ? $0 : nil } ?? kinds[0] }

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            sendComposerSectionLabel(kinds == [.destinationTag] ? "Destination Tag" : "Memo")
            if kinds.count > 1 {
                Picker(AppLocalization.string("Memo"), selection: Binding(get: { selected }, set: { kind = $0 })) {
                    ForEach(kinds, id: \.self) { Text($0.localizedTitle).tag($0) }
                }
                .pickerStyle(.segmented)
            }
            TextField(AppLocalization.string("Optional"), text: $text)
                .keyboardType(selected == .memoText ? .default : .numberPad)
                .textInputAutocapitalization(.never).autocorrectionDisabled().font(.body.monospaced())
                .padding(.horizontal, SpectraLayout.Space.m)
                .frame(minHeight: 44)
                .spectraInsetFill(cornerRadius: SpectraLayout.Radius.inner)
            Text(AppLocalization.string("An exchange or other shared account gives one to tell whose deposit it is. Leave it empty unless the recipient gave you one."))
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear { if kind != selected { kind = selected } }
        .onChange(of: kinds) { if kind != selected { kind = selected } }
    }
}

extension PaymentMemoKind {
    /// What the field is called on review and in the picker.
    var localizedTitle: String {
        switch self {
        case .destinationTag: AppLocalization.string("Destination Tag")
        case .memoText: AppLocalization.string("Text Memo")
        case .memoId: AppLocalization.string("ID Memo")
        }
    }
}

/// A reviewed transaction's destination tag or memo, exactly as core built it.
struct PaymentMemoRow: View {
    let memo: PaymentMemo

    var body: some View {
        LabeledContent(memo.kind.localizedTitle) {
            Text(verbatim: memo.value).font(.subheadline.monospaced()).textSelection(.enabled)
        }
    }
}
