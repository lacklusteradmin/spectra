import SwiftUI

/// The proof of a Monero payment, as core derives and checks it: the
/// transaction, its key and the address it paid. The recipient, or anyone
/// they show it to, checks it with monero-wallet-cli's `check_tx_key`.
struct MoneroPaymentProofView: View {
    let proof: MoneroPaymentProof
    @Environment(\.dismiss) private var dismiss
    @State private var copied: String?

    private var command: String { "check_tx_key \(proof.txid) \(proof.txKey) \(proof.address)" }

    var body: some View {
        Form {
            Section {
                copyRow(AppLocalization.string("Transaction ID"), proof.txid)
                copyRow(AppLocalization.string("Transaction Key"), proof.txKey)
                copyRow(AppLocalization.string("Address"), proof.address)
                LabeledContent(AppLocalization.string("Proves"), value: "\(AmountPresentation.localizedDecimal(proof.amount)) XMR")
            } footer: {
                Text(AppLocalization.string("Anyone with these three can see that this address received the amount, and nothing else about the wallet. Share them only with whoever needs to check the payment."))
            }
            Section {
                copyRow(AppLocalization.string("Command"), command)
            } footer: {
                Text(AppLocalization.string("Any Monero wallet checks the proof with its transaction-key check, such as this command in monero-wallet-cli."))
            }
        }
        .navigationTitle(AppLocalization.string("Payment Proof")).navigationBarTitleDisplayMode(.inline)
        // "Copied" says so for a moment, then the row is ready to copy again.
        .task(id: copied) {
            guard copied != nil else { return }
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            copied = nil
        }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(AppLocalization.string("Done")) { dismiss() }
            }
        }
    }

    private func copyRow(_ title: String, _ value: String) -> some View {
        Button {
            UIPasteboard.general.string = value
            copied = value
            spectraHaptic(.light)
        } label: {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                HStack {
                    Text(title).font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Image(systemName: copied == value ? "checkmark" : "doc.on.doc").font(.caption).foregroundStyle(.tint)
                }
                // Whole: a key or a command cut in the middle cannot be read
                // back or checked against what was pasted.
                Text(verbatim: breakableAnywhere(value)).font(.caption.monospaced()).foregroundStyle(Color.primary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityLabel(Text(verbatim: value))
            }
        }.buttonStyle(.plain)
    }
}
