import SwiftUI

/// The addresses an account-discovery UTXO wallet has handed out and the
/// coins they hold, as core reads them from the network. Read-only.
struct WalletCoinsView: View {
    let store: AppState
    let wallet: WalletView
    @State private var coins: WalletCoins?
    @State private var error: String?
    @State private var isLoading = false
    @State private var copiedAddress: String?

    var body: some View {
        List {
            if let coins {
                content(coins)
            } else if let error {
                Section { Text(error).foregroundStyle(.red) }
            } else {
                Section { ProgressView().frame(maxWidth: .infinity) }
            }
        }
        .navigationTitle(WalletAction.coins.title).navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
    }

    @ViewBuilder
    private func content(_ coins: WalletCoins) -> some View {
        if let next = coins.nextReceiveAddress {
            Section {
                addressText(next)
            } header: {
                Text(AppLocalization.string("Next Receive Address"))
            } footer: {
                Text(AppLocalization.string("Unused: a payment to it does not link to your other addresses."))
            }
        }
        if coins.maturing != "0" {
            Section {
                Label(
                    AppLocalization.format(
                        "%@ %@ in minting rewards is still maturing: counted in the balance, not yet spendable.",
                        coins.maturing, coins.symbol),
                    systemImage: "hourglass")
                    .font(.subheadline)
            }
        }
        addressSection(AppLocalization.string("Receive Addresses"), coins.addresses.filter { $0.branch == .receive }, coins)
        addressSection(AppLocalization.string("Change Addresses"), coins.addresses.filter { $0.branch == .change }, coins)
        addressSection(AppLocalization.string("Other Addresses"), coins.addresses.filter { $0.branch == nil }, coins)
        Section {
            if coins.outputs.isEmpty {
                Text(AppLocalization.string("No unspent coins.")).foregroundStyle(.secondary)
            }
            ForEach(coins.outputs, id: \.outpoint) { output in
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    HStack {
                        Text(verbatim: "\(output.amount) \(coins.symbol)").font(.subheadline.weight(.semibold))
                            .monospacedDigit()
                        Spacer()
                        Text(output.confirmationsText).font(.caption).foregroundStyle(.secondary)
                    }
                    Text(verbatim: output.outpoint).font(.caption2.monospaced()).foregroundStyle(.secondary)
                        .lineLimit(1).truncationMode(.middle)
                    if !output.spendable {
                        Label(AppLocalization.string("Maturing"), systemImage: "hourglass")
                            .font(.caption).foregroundStyle(Color.spectraWarning)
                    }
                }
            }
        } header: {
            Text(AppLocalization.format("Coins (%lld)", coins.outputs.count))
        }
    }

    @ViewBuilder
    private func addressSection(_ title: String, _ rows: [OwnedAddressCoins], _ coins: WalletCoins) -> some View {
        if !rows.isEmpty {
            Section(title) {
                ForEach(rows, id: \.address) { row in
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                        HStack {
                            if let index = row.index {
                                Text(verbatim: "#\(index)").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Text(verbatim: "\(row.balance) \(coins.symbol)").font(.subheadline).monospacedDigit()
                        }
                        addressText(row.address)
                        if !row.used {
                            Text(AppLocalization.string("Unused")).font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
    }

    private func addressText(_ address: String) -> some View {
        Button {
            UIPasteboard.general.string = address
            copiedAddress = address
            spectraHaptic(.light)
        } label: {
            HStack {
                Text(verbatim: address).font(.footnote.monospaced()).foregroundStyle(Color.primary)
                    .lineLimit(1).truncationMode(.middle)
                Spacer()
                Image(systemName: copiedAddress == address ? "checkmark" : "doc.on.doc").font(.caption)
                    .foregroundStyle(.tint)
            }
        }.buttonStyle(.plain)
    }

    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            coins = try await store.bridge.ready().walletCoins(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}

extension UnspentOutput {
    var outpoint: String { "\(txid):\(vout)" }
    var confirmationsText: String {
        confirmations == 0
            ? AppLocalization.string("Unconfirmed")
            : AppLocalization.format("%lld confirmations", count: Int(confirmations), Int(confirmations))
    }
}
