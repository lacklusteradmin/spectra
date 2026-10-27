import SwiftUI

/// The composer's recipient: the address, entered as on every page that asks
/// for one, and what core said of it.
@MainActor
struct SendRecipientPage: View {
    @Bindable var store: AppState

    private var presentation: SendComposerPresentation { SendComposerPresentation(store: store) }
    private var flow: SendFlowState { store.sendFlow }

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            SendAssetContextView(store: store)
            FlowPageHeading(title: "Who are you sending to?", subtitle: "Enter an address or choose a saved contact.")
            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                toCard
                Label(AppLocalization.string("Check the full address. A valid address does not verify the recipient's identity."), systemImage: "info.circle")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .keyboardAnchor()
        }
    }

    private var toCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            SendRecipientIdentityView(
                store: store, chain: presentation.selectedCoin?.chainId,
                address: flow.currentRecipient?.address ?? flow.address)
            sendComposerSectionLabel("Recipient address")

            if let coin = presentation.selectedCoin {
                AddressEntryRow(
                    title: AppLocalization.string("Recipient address"), text: Bindable(flow).address,
                    chain: coin.chain, contacts: presentation.addressBookEntries,
                    readScan: flow.applyScannedPayment)
                    .padding(.horizontal, SpectraLayout.Space.m)
                    .padding(.vertical, SpectraLayout.Space.m)
                    .frame(minHeight: 44)
                    .spectraInsetFill(cornerRadius: SpectraLayout.Radius.inner)
            }

            if let chain = presentation.selectedCoin?.chainId,
               case let kinds = paymentMemoKinds(chain: chain), !kinds.isEmpty {
                SendPaymentMemoField(
                    kinds: kinds, kind: Bindable(flow).memoKind,
                    text: Bindable(flow).memoText)
            }

            // One row while anything about the recipient is being asked:
            // the address check and the preview's look at its history.
            if flow.isResolvingRecipient || flow.isCheckingDestination {
                SpectraLoadingRow(title: "Checking recipient...")
            } else if let problem = flow.recipientProblem {
                HStack(spacing: SpectraLayout.Space.s) {
                    Label(problem.message, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption).foregroundStyle(.red)
                    Spacer(minLength: 0)
                    if problem.canRetry {
                        Button(AppLocalization.string("Retry"), action: flow.retryRecipientResolution)
                            .font(.caption.weight(.semibold))
                    }
                }
            } else if let resolution = flow.currentRecipient {
                Label(
                    resolution.usedEns
                        ? AppLocalization.format(
                            "Resolved ENS %@ to %@.",
                            flow.address.trimmingCharacters(in: .whitespacesAndNewlines),
                            resolution.address)
                        : AppLocalization.string("Valid address for this network"),
                    systemImage: "checkmark.circle.fill"
                )
                .font(.caption).foregroundStyle(.green)
            }
            recipientMessages
        }
        .padding(SpectraLayout.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraElevatedFill()
    }

    /// "From the code: 0.1 BTC and Destination Tag 123456." — what the scan
    /// filled in besides the address, while that address stands.
    private var scanNotice: String? {
        guard let payment = flow.scannedPayment, payment.address == flow.address else { return nil }
        var filled: [String] = []
        if let amount = payment.amount, let coin = presentation.selectedCoin {
            filled.append(AppLocalization.format("scan.filled.part_format", amount, coin.symbol))
        }
        if let memo = payment.memo {
            filled.append(AppLocalization.format("scan.filled.part_format", memo.kind.localizedTitle, memo.value))
        }
        guard !filled.isEmpty else { return nil }
        return AppLocalization.format(
            "scan.filled_format", filled.formatted(.list(type: .and).locale(AppLocalization.locale)))
    }

    @ViewBuilder
    private var recipientMessages: some View {
        if let scanNotice {
            Label(scanNotice, systemImage: "qrcode.viewfinder").font(.caption).foregroundStyle(.secondary)
        }

        // What the recipient's history says is about a valid address only;
        // beside a refusal or mid-check it would describe nothing.
        if flow.currentRecipient != nil, flow.recipientProblem == nil, !flow.isResolvingRecipient,
           !flow.isCheckingDestination {
            ForEach(flow.destinationWarnings, id: \.self) { warning in
                Label(warning, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption).foregroundStyle(.spectraWarning)
            }
            if let info = flow.destinationInfoMessage {
                Label(info, systemImage: "info.circle").font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}
