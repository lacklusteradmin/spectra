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
                LabeledContent(AppLocalization.string("Proves"), value: "\(proof.amount) XMR")
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
                Text(verbatim: value).font(.caption.monospaced()).foregroundStyle(Color.primary)
                    .lineLimit(3).truncationMode(.middle)
            }
        }.buttonStyle(.plain)
    }
}
