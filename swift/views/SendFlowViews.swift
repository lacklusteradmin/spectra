import Foundation
import SwiftUI
import VisionKit

private enum SendFlowStep: Int, CaseIterable {
    case from
    case recipient
    case amount
    case confirm

    var title: String {
        switch self {
        case .from: return "Choose an asset"
        case .recipient: return "Recipient"
        case .amount: return "Amount"
        case .confirm: return "Review transfer"
        }
    }

    var progressTitle: String {
        switch self {
        case .from: "Asset"
        case .recipient: "Recipient"
        case .amount: "Amount"
        case .confirm: "Review"
        }
    }
}

struct SendView: View {
    @Bindable var store: AppState
    @State private var isShowingQRScanner: Bool = false
    @State private var qrScannerErrorMessage: String?
    @State private var currentStep: SendFlowStep = .from
    @State private var flowDirection: Int = 1
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// Core's answer for the recipient as typed, keyed by what it answered.
    /// The amount page and the confirm gate read it rather than ask again;
    /// the preview and the build check the recipient for themselves.
    @State private var validatedRecipient: (key: String, resolution: SendDestinationResolution)?
    @State private var recipientError: String?
    @State private var isValidatingRecipient = false
    @State private var quotedInputKey: String?
    @State private var recipientValidationAttempt = 0
    @State private var sendWalletPassword = ""
    @State private var stagedTransaction: TransactionRecord?
    @State private var transactionError: String?

    private var isSendBusy: Bool { store.sendFlow.session.isBusy || store.sendFlow.isPreparingPreview }

    private var selectedNetworkSendCoin: AssetHolding? {
        store.availableSendCoins(for: store.sendFlow.walletId).first(where: { $0.holdingKey == store.sendFlow.holdingKey })
    }

    var body: some View {
        let selectedCoin = selectedNetworkSendCoin
        ZStack {
            SpectraBackdrop().ignoresSafeArea()

            ScrollView(showsIndicators: false) {
                LazyVStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    if store.sendFlow.session.artifact == nil { stepProgress }

                    stepContent
                        .id(currentStep)
                        .transition(stepTransition)

                    SendStatusCards(store: store)
                }
                .spectraScreenPadding()

            }
            .scrollDismissesKeyboard(.interactively)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { flowBottomBar(selectedCoin: selectedCoin) }
        .navigationTitle(AppLocalization.string(navigationTitle))
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        // No keyboard toolbar: its floating Done sat on top of the primary
        // button, which already rides above the keyboard and dismisses it.
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    store.cancelSend()
                } label: {
                    Image(systemName: "xmark")
                }
                .accessibilityLabel(AppLocalization.string("Close"))
            }
        }
        .task(id: "\(recipientKey)|\(recipientValidationAttempt)") {
            let key = recipientKey
            validatedRecipient = nil
            recipientError = nil
            isValidatingRecipient = false
            guard let chain = selectedNetworkSendCoin?.chain,
                  !store.sendFlow.address.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
            isValidatingRecipient = true
            defer { if recipientKey == key { isValidatingRecipient = false } }
            do {
                try await Task.sleep(for: .milliseconds(350))
                let resolution = try await store.resolveSendDestination(input: store.sendFlow.address, on: chain)
                guard !Task.isCancelled, recipientKey == key else { return }
                validatedRecipient = (key, resolution)
            } catch {
                guard !Task.isCancelled, recipientKey == key else { return }
                recipientError = AppLocalization.string("Check the address and selected network, then try again.")
            }
        }
        .sheet(isPresented: $isShowingQRScanner) {
            SendQRScannerSheet { payload in applyScannedRecipientPayload(payload) }
        }
        .alert(AppLocalization.string("QR Scanner"), isPresented: .isPresent($qrScannerErrorMessage)) {
            Button(AppLocalization.string("OK"), role: .cancel) {}
        } message: {
            if let qrScannerErrorMessage { Text(verbatim: qrScannerErrorMessage) }
        }
        .task(id: currentStep) {
            if currentStep == .from { await store.sendFlow.loadSavedArtifacts() }
        }
        .onDisappear {
            sendWalletPassword = ""
            store.sendFlow.invalidateSession()
        }
        .onChange(of: store.sendFlow.isShowingHighRiskConfirmation) { _, showing in
            if !showing { sendWalletPassword = "" }
        }
        .task(id: previewRefreshKey) {
            let key = previewRefreshKey
            quotedInputKey = nil
            guard store.sendFlow.session.artifact == nil else { return }
            do {
                try await Task.sleep(for: .milliseconds(350))
                while store.sendFlow.isPreparingPreview {
                    try await Task.sleep(for: .milliseconds(100))
                }
                try Task.checkCancellation()
                await store.refreshSendPreview()
                guard !Task.isCancelled, previewRefreshKey == key else { return }
                quotedInputKey = key
            } catch { return }
        }
        .task(id: "\(store.sendFlow.session.id):\(store.sendFlow.session.artifact?.id ?? ""):\(store.sendFlow.session.artifact?.revision ?? 0):\(store.transactionRevision)") {
            guard let artifact = store.sendFlow.session.artifact, !artifact.attempts.isEmpty else {
                stagedTransaction = nil
                transactionError = nil
                return
            }
            if stagedTransaction?.id != artifact.id {
                stagedTransaction = nil
                transactionError = nil
            }
            let session = store.sendFlow.session.id
            let revision = store.transactionRevision
            do {
                let record = try await store.bridge.ready().transaction(id: artifact.id)
                guard store.sendFlow.session.isCurrent(session), store.transactionRevision == revision,
                      store.sendFlow.session.artifact?.id == artifact.id,
                      store.sendFlow.session.artifact?.revision == artifact.revision else { return }
                stagedTransaction = record
                transactionError = nil
            } catch {
                guard store.sendFlow.session.isCurrent(session), store.transactionRevision == revision,
                      store.sendFlow.session.artifact?.id == artifact.id,
                      store.sendFlow.session.artifact?.revision == artifact.revision else { return }
                transactionError = userErrorMessage(error)
            }
        }
        .alert(AppLocalization.string("Sign this transaction?"), isPresented: Bindable(store.sendFlow).isShowingHighRiskConfirmation) {
            if store.stagedSendRequiresPassword {
                SecureField(AppLocalization.string("Wallet Password"), text: $sendWalletPassword)
            }
            Button(AppLocalization.string("Cancel"), role: .cancel) {
                sendWalletPassword = ""
                store.sendFlow.isShowingHighRiskConfirmation = false
            }
            Button(AppLocalization.string("Sign Transaction"), role: .destructive) {
                let password = store.stagedSendRequiresPassword ? sendWalletPassword : nil
                sendWalletPassword = ""
                let session = store.sendFlow.session.id
                Task {
                    guard store.sendFlow.session.isCurrent(session) else { return }
                    await store.confirmSigning(password: password)
                }
            }
            .disabled(store.stagedSendRequiresPassword && sendWalletPassword.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        } message: {
            if let artifact = store.sendFlow.session.artifact {
                Text(verbatim: sendSigningConfirmationMessage(artifact: artifact))
            }
        }
    }

    // MARK: - Flow shell

    @ViewBuilder
    private var stepContent: some View {
        switch currentStep {
        case .from:
            SendFromPage(store: store)
            MoneroSyncView(store: store, walletId: store.sendFlow.walletId).id(store.sendFlow.walletId)
            if !store.sendFlow.savedArtifacts.isEmpty {
                DisclosureGroup(AppLocalization.string("Resume a transaction")) {
                    VStack(spacing: 0) {
                        ForEach(Array(store.sendFlow.savedArtifacts.enumerated()), id: \.element.id) { index, artifact in
                            if index > 0 { Divider().opacity(0.3) }
                            Button {
                                Task {
                                    if await store.sendFlow.resume(id: artifact.id) { go(to: .confirm) }
                                }
                            } label: {
                                SavedSendRow(artifact: artifact, walletName: store.wallet(for: artifact.walletId)?.name)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.horizontal, SpectraLayout.cardPadding)
                    .spectraCardFill()
                    .padding(.top, SpectraLayout.Space.s)
                }
                .font(.subheadline.weight(.semibold))
                .padding(.horizontal, SpectraLayout.Space.xs)
            }
        case .recipient:
            SendRecipientPage(
                store: store,
                isShowingQRScanner: $isShowingQRScanner,
                qrScannerErrorMessage: $qrScannerErrorMessage,
                validationError: recipientError,
                isValidating: isValidatingRecipient,
                validatedResolution: currentRecipientResolution,
                retryValidation: { recipientValidationAttempt += 1 }
            )
        case .amount:
            SendAmountPage(store: store, quoteIsCurrent: quotedInputKey == previewRefreshKey)
        case .confirm:
            if let artifact = store.sendFlow.session.artifact {
                SendStagesView(store: store, artifact: artifact,
                    transaction: stagedTransaction, transactionError: transactionError)
            } else {
                SendConfirmationStep(store: store, quoteIsCurrent: quotedInputKey == previewRefreshKey,
                    recipientAddress: currentRecipientResolution?.address ?? store.sendFlow.address)
            }
        }
    }

    private var stepTransition: AnyTransition {
        let insertionEdge: Edge = flowDirection >= 0 ? .trailing : .leading
        let removalEdge: Edge = flowDirection >= 0 ? .leading : .trailing
        return .asymmetric(
            insertion: .move(edge: insertionEdge).combined(with: .opacity),
            removal: .move(edge: removalEdge).combined(with: .opacity)
        )
    }

    private var stepProgress: some View {
        HStack(alignment: .top, spacing: SpectraLayout.Space.s) {
            ForEach(SendFlowStep.allCases, id: \.rawValue) { step in
                VStack(spacing: SpectraLayout.Space.xs) {
                    Image(systemName: step.rawValue < currentStep.rawValue ? "checkmark.circle.fill" : step == currentStep ? "circle.inset.filled" : "circle")
                    Text(AppLocalization.string(step.progressTitle))
                        .font(.caption.weight(step == currentStep ? .semibold : .regular))
                        .multilineTextAlignment(.center)
                }
                .foregroundStyle(step.rawValue <= currentStep.rawValue ? Color.accentColor : Color.secondary)
                .frame(maxWidth: .infinity)
            }
        }
        .padding(.vertical, SpectraLayout.Space.s)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(AppLocalization.format("Step %lld of %lld", currentStep.rawValue + 1, SendFlowStep.allCases.count))
        .accessibilityValue(AppLocalization.string(currentStep.progressTitle))
    }

    @ViewBuilder
    private func flowBottomBar(selectedCoin: AssetHolding?) -> some View {
        SpectraBottomActionBar {
            if currentStep != .from && store.sendFlow.session.artifact?.attempts.isEmpty != false {
                Button {
                    spectraHaptic(.light)
                    goBack()
                } label: {
                    Image(systemName: "chevron.left")
                        .font(.headline.weight(.semibold))
                        .frame(width: 46, height: 46)
                }
                .buttonStyle(.glass)
                .accessibilityLabel(AppLocalization.string("Back"))
            }

            if executionAction == .viewTransaction || executionAction == .done,
               let artifact = store.sendFlow.session.artifact,
               SendExecutionAction.canRetry(artifact: artifact, transaction: stagedTransaction) {
                Button {
                    startBroadcast()
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .font(.headline)
                        .frame(width: 46, height: 46)
                }
                .buttonStyle(.glass)
                .accessibilityLabel(AppLocalization.string("Retry Same Transaction"))
                .disabled(isSendBusy || store.sendFlow.session.selectedEndpoints.isEmpty)
            }

            Button {
                handlePrimaryAction()
            } label: {
                HStack(spacing: SpectraLayout.Space.s) {
                    if primaryShowsProgress {
                        SpectraLoadingGlyph(size: 20, tint: .white)
                    } else {
                        Image(systemName: primaryActionSystemImage)
                            .font(.system(size: 20, weight: .semibold))
                    }
                    Text(AppLocalization.string(primaryActionTitle))
                        .font(.headline)
                }
                .frame(maxWidth: .infinity)
                .frame(minHeight: 46)
            }
            .buttonStyle(.glassProminent)
            .disabled(!canUsePrimaryAction(selectedCoin: selectedCoin))
        }
    }

    private var primaryActionTitle: String {
        switch currentStep {
        case .from, .recipient: return "Next"
        case .amount: return "Review"
        case .confirm:
            // Three stages, each its own action (docs/ARCHITECTURE.md): what was built
            // is inspectable before signing, and a signed send waits for the
            // user to choose which nodes receive it.
            return executionAction.title
        }
    }

    private var primaryActionSystemImage: String {
        guard currentStep == .confirm else { return "chevron.right" }
        return executionAction.systemImage
    }

    private var primaryShowsProgress: Bool {
        currentStep == .confirm && isSendBusy
    }

    private func handlePrimaryAction() {
        switch currentStep {
        case .from:
            go(to: .recipient)
        case .recipient:
            go(to: .amount)
        case .amount:
            go(to: .confirm)
        case .confirm:
            spectraHaptic(executionAction == .done || executionAction == .viewTransaction ? .light : .heavy)
            switch executionAction {
            case .build:
                let session = store.sendFlow.session.id
                Task {
                    guard store.sendFlow.session.isCurrent(session) else { return }
                    await store.sendFlow.build()
                }
            case .sign: store.sendFlow.isShowingHighRiskConfirmation = true
            case .broadcast, .retry: startBroadcast()
            case .viewTransaction:
                if let url = stagedTransaction?.explorerLink?.url { UIApplication.shared.open(url) }
            case .done: store.cancelSend()
            }
        }
    }

    private func startBroadcast() {
        let session = store.sendFlow.session.id
        Task {
            guard store.sendFlow.session.isCurrent(session) else { return }
            await store.broadcastPreparedSend()
        }
    }

    private func canUsePrimaryAction(selectedCoin: AssetHolding?) -> Bool {
        switch currentStep {
        case .from:
            return store.selectedWalletForSend() != nil && selectedCoin != nil
        case .recipient:
            return currentRecipientResolution != nil
        case .amount:
            return store.sendAmountIsValid
        case .confirm:
            if store.sendFlow.session.artifact != nil {
                guard !isSendBusy else { return false }
                switch executionAction {
                case .broadcast: return !store.sendFlow.session.selectedEndpoints.isEmpty
                case .retry:
                    guard let artifact = store.sendFlow.session.artifact else { return false }
                    return !store.sendFlow.session.selectedEndpoints.isEmpty
                        && SendExecutionAction.canRetry(artifact: artifact, transaction: stagedTransaction)
                default: return true
                }
            }
            return !isSendBusy
                && store.selectedWalletForSend() != nil
                && selectedCoin != nil
                && currentRecipientResolution != nil
                && store.sendAmountIsValid
                && quotedInputKey == previewRefreshKey
                && store.sendFlow.customEvmFeeValidationError == nil
                && store.sendFlow.evmNonceValidationError == nil
        }
    }

    private var executionAction: SendExecutionAction {
        SendExecutionAction(artifact: store.sendFlow.session.artifact, transaction: stagedTransaction)
    }

    private var navigationTitle: String {
        guard currentStep == .confirm, let artifact = store.sendFlow.session.artifact else { return currentStep.title }
        switch executionAction {
        case .build: return "Review transfer"
        case .sign: return "Check and sign"
        case .broadcast: return "Submit transaction"
        case .retry: return "Submission results"
        case .viewTransaction, .done:
            if stagedTransaction?.id == artifact.id, stagedTransaction?.status == .confirmed { return "Transfer complete" }
            if stagedTransaction?.id == artifact.id, stagedTransaction?.status == .failed { return "Transaction failed" }
            return "Waiting for confirmation"
        }
    }

    private func goBack() {
        if currentStep == .confirm {
            store.sendFlow.invalidateSession()
        }
        guard let previous = SendFlowStep(rawValue: currentStep.rawValue - 1) else { return }
        go(to: previous)
    }

    private func go(to step: SendFlowStep) {
        flowDirection = step.rawValue >= currentStep.rawValue ? 1 : -1
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        withAnimation(reduceMotion ? nil : .snappy(duration: 0.28)) {
            currentStep = step
        }
    }

    private var currentRecipientResolution: SendDestinationResolution? {
        guard let validatedRecipient, validatedRecipient.key == recipientKey else { return nil }
        return validatedRecipient.resolution
    }

    private var recipientKey: String { [store.sendFlow.walletId, store.sendFlow.holdingKey, store.sendFlow.address].joined(separator: "|") }

    private var previewRefreshKey: String {
        [
            store.sendFlow.session.artifact?.id ?? "",
            store.sendFlow.walletId,
            store.sendFlow.holdingKey,
            store.sendFlow.address,
            store.sendFlow.amount,
            store.sendFlow.useCustomEvmFees.description,
            store.sendFlow.customEvmMaxFeeGwei,
            store.sendFlow.customEvmPriorityFeeGwei,
            store.sendFlow.evmManualNonceEnabled.description,
            store.sendFlow.evmManualNonce,
        ].joined(separator: "|")
    }

    private func applyScannedRecipientPayload(_ payload: String) {
        guard !payload.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            qrScannerErrorMessage = AppLocalization.string("The scanned QR code did not contain a usable address.")
            return
        }
        // Core reads the payload — bare address or payment URI — against the
        // network the wallet is on, and hands back the stored form. With no
        // network there is nothing to judge an address against, so nothing is
        // filled in.
        guard let network = scannedPayloadNetwork,
            let address = scannedSendAddress(chain: network, payload: payload)
        else {
            qrScannerErrorMessage = AppLocalization.string("The scanned QR code does not contain a valid address for the selected asset.")
            return
        }
        store.sendFlow.address = address
        qrScannerErrorMessage = nil
    }

    /// The network a scanned address must belong to: the one the sending wallet
    /// is on, or the selected asset's own.
    private var scannedPayloadNetwork: Chain? {
        store.selectedWalletForSend()?.chainId ?? store.selectedSendCoin?.chain
    }
}

/// A built or signed send that survived, as the From page offers it back:
/// what it moves, on which network, to whom, and which stage it is waiting in.
private struct SavedSendRow: View {
    let artifact: SendArtifact
    let walletName: String?

    private var stageText: String {
        switch artifact.stage {
        case .prepared: return AppLocalization.string("Ready to sign")
        default: return AppLocalization.string(artifact.attempts.isEmpty ? "Ready to broadcast" : "Submitted")
        }
    }

    var body: some View {
        let chain = artifact.chainId
        let badge = AssetHolding.nativeChainBadge(for: chain) ?? (nil, Color.secondary)
        HStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(artworkName: badge.artworkName, fallbackText: artifact.symbol, color: badge.color, size: 28)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(verbatim: "\(AmountPresentation.localizedDecimal(artifact.amount)) \(artifact.symbol)")
                    .font(.subheadline.weight(.semibold))
                    .spectraNumericTextLayout()
                Text(verbatim: artifact.chainId.displayName)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if let walletName {
                    Label(walletName, systemImage: "wallet.pass")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Text(verbatim: "→ \(artifact.recipient)")
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: SpectraLayout.Space.s)
            VStack(alignment: .trailing, spacing: SpectraLayout.Space.xxs) {
                Text(stageText)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tint)
                Text(Date(timeIntervalSince1970: artifact.createdAt), format: .relative(presentation: .named))
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }
        .padding(.vertical, SpectraLayout.Space.m)
        .contentShape(Rectangle())
    }
}
