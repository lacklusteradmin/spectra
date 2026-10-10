import Foundation
import SwiftUI
import VisionKit

private enum SendFlowStep: Int, CaseIterable {
    case from
    case recipient
    case amount
    case confirm

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
    /// What the last scanned code filled in, said for as long as the
    /// recipient is still the one it named.
    @State private var scannedPayment: ScannedPayment?
    @State private var currentStep: SendFlowStep = .from
    @State private var flowDirection: Int = 1
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// Core's answer for the recipient as typed, keyed by what it answered.
    /// The amount page and the confirm gate read it rather than ask again;
    /// the preview and the build check the recipient for themselves.
    @State private var validatedRecipient: (key: String, resolution: SendDestinationResolution)?
    @State private var recipientError: SendRecipientProblem?
    @State private var isValidatingRecipient = false
    @State private var quotedInputKey: String?
    @State private var quoteAttempt = 0
    @State private var recipientValidationAttempt = 0
    @State private var sendWalletPassword = ""
    @State private var stagedTransaction: TransactionRecord?
    @State private var transactionError: String?
    /// The artifact on screen was resumed from the list, not built from the
    /// form, which it does not fill: Back returns to that list.
    @State private var isShowingResumedSend = false

    private var isSendBusy: Bool { store.sendFlow.session.isBusy || store.sendFlow.isPreparingPreview }

    private var selectedNetworkSendCoin: AssetHolding? {
        store.availableSendCoins(for: store.sendFlow.walletId).first(where: { $0.holdingKey == store.sendFlow.holdingKey })
    }

    var body: some View {
        let selectedCoin = selectedNetworkSendCoin
        ZStack {
            SpectraBackdrop().ignoresSafeArea()

            ScrollViewReader { proxy in
                ScrollView(showsIndicators: false) {
                    LazyVStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                        if store.sendFlow.session.artifact == nil { stepProgress }
                        if store.sendFlow.isPreparingReplacement {
                            SpectraLoadingRow(title: "Preparing replacement/cancel context...")
                        }

                        stepContent
                            .id(currentStep)
                            .transition(stepTransition)
                    }
                    .spectraScreenPadding()
                    .scrollsToKeyboardAnchor(proxy)
                }
                .scrollDismissesKeyboard(.interactively)
            }
        }
        // The build's, signature's and broadcast's errors ride with the
        // button that raised them, where they are seen whatever the scroll.
        .safeAreaBar(edge: .bottom) {
            VStack(spacing: SpectraLayout.Space.s) {
                SendStatusCards(store: store).padding(.horizontal, SpectraLayout.Space.l)
                flowBottomBar(selectedCoin: selectedCoin)
            }
        }
        .navigationTitle(AppLocalization.string(navigationTitle))
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .toolbar(.hidden, for: .tabBar)
        // Back is a step back, never the whole flow: the system button and
        // its edge swipe would pop the composer and wipe what was typed.
        .navigationBarBackButtonHidden(true)
        // No keyboard toolbar: its floating Done sat on top of the primary
        // button, which already rides above the keyboard and dismisses it.
        .toolbar {
            if canStepBack {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        spectraHaptic(.light)
                        goBack()
                    } label: {
                        Image(systemName: "chevron.left")
                    }
                    .accessibilityLabel(AppLocalization.string("Back"))
                }
            }
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
            do {
                // Typing is not checking: the row appears once typing pauses,
                // not on every keystroke.
                try await Task.sleep(for: .milliseconds(350))
                isValidatingRecipient = true
                defer { if recipientKey == key { isValidatingRecipient = false } }
                let resolution = try await store.resolveSendDestination(input: store.sendFlow.address, on: chain)
                guard !Task.isCancelled, recipientKey == key else { return }
                validatedRecipient = (key, resolution)
            } catch {
                guard !Task.isCancelled, recipientKey == key else { return }
                recipientError = SendRecipientProblem(error)
            }
        }
        .sheet(isPresented: $isShowingQRScanner) {
            SendQRScannerSheet { payload in applyScannedRecipientPayload(payload) }
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
        .task(id: "\(previewRefreshKey)#\(quoteAttempt)") {
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
                // A quote that failed leaves the fee unknown: nothing is
                // current until a retry lands.
                guard !Task.isCancelled, previewRefreshKey == key, store.sendFlow.previewError == nil else { return }
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
                Text(verbatim: sendSigningConfirmationMessage(artifact: artifact, amounts: store.amounts))
            }
        }
    }

    // MARK: - Flow shell

    @ViewBuilder
    private var stepContent: some View {
        switch currentStep {
        case .from:
            SendFromPage(store: store)
            if !store.sendFlow.savedArtifacts.isEmpty {
                DisclosureGroup(AppLocalization.string("Resume a transaction")) {
                    VStack(spacing: 0) {
                        ForEach(Array(store.sendFlow.savedArtifacts.enumerated()), id: \.element.id) { index, artifact in
                            if index > 0 { Divider().opacity(0.3) }
                            Button {
                                Task {
                                    if await store.sendFlow.resume(id: artifact.id) {
                                        isShowingResumedSend = true
                                        go(to: .confirm)
                                    }
                                }
                            } label: {
                                SavedSendRow(artifact: artifact, walletName: store.wallet(for: artifact.walletId)?.name)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.horizontal, SpectraLayout.cardPadding)
                    .spectraCardFill()
                }
                .font(.subheadline.weight(.semibold))
                .disclosureGroupStyle(.spectra)
            }
        case .recipient:
            SendRecipientPage(
                store: store,
                isShowingQRScanner: $isShowingQRScanner,
                qrScannerErrorMessage: $qrScannerErrorMessage,
                scanNotice: scanNotice,
                validationError: recipientError,
                isValidating: isValidatingRecipient,
                validatedResolution: currentRecipientResolution,
                retryValidation: { recipientValidationAttempt += 1 }
            )
        case .amount:
            SendAmountPage(store: store, quoteIsCurrent: quotedInputKey == previewRefreshKey,
                retryQuote: { quoteAttempt += 1 })
        case .confirm:
            if let artifact = store.sendFlow.session.artifact {
                SendStagesView(store: store, artifact: artifact,
                    transaction: stagedTransaction, transactionError: transactionError)
            } else {
                SendConfirmationStep(store: store, quoteIsCurrent: quotedInputKey == previewRefreshKey,
                    recipientAddress: currentRecipientResolution?.address ?? store.sendFlow.address,
                    retryQuote: { quoteAttempt += 1 })
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
            if executionAction == .done,
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
                            .font(.title3.weight(.semibold))
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
            spectraHaptic(executionAction == .done ? .light : .heavy)
            switch executionAction {
            case .build:
                isShowingResumedSend = false
                let session = store.sendFlow.session.id
                Task {
                    guard store.sendFlow.session.isCurrent(session) else { return }
                    await store.sendFlow.build()
                }
            case .sign: store.sendFlow.isShowingHighRiskConfirmation = true
            case .broadcast, .retry: startBroadcast()
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
            return store.sendAmountIsValid && store.sendAmountRefusal == nil && store.sendFlow.previewError == nil
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
                && store.sendAmountRefusal == nil
                && quotedInputKey == previewRefreshKey
                && store.sendFlow.customEvmFeeValidationError == nil
                && store.sendFlow.evmNonceValidationError == nil
        }
    }

    private var executionAction: SendExecutionAction {
        SendExecutionAction(artifact: store.sendFlow.session.artifact, transaction: stagedTransaction)
    }

    /// The flow's name while it is composed — the step bar and the page's
    /// own heading say which step this is — and the stage once it is built,
    /// in title case as every navigation title is.
    private var navigationTitle: String {
        guard currentStep == .confirm, let artifact = store.sendFlow.session.artifact else { return "Send" }
        switch executionAction {
        case .build: return "Review Transfer"
        case .sign: return "Check and Sign"
        case .broadcast: return "Submit Transaction"
        case .retry: return "Submission Results"
        case .done:
            if stagedTransaction?.id == artifact.id, stagedTransaction?.status == .confirmed { return "Transfer Complete" }
            if stagedTransaction?.id == artifact.id, stagedTransaction?.status == .failed { return "Transaction Failed" }
            return "Waiting for Confirmation"
        }
    }

    /// A step to go back to: none on the first page, and none once the
    /// transaction has gone to a node.
    private var canStepBack: Bool {
        currentStep != .from && store.sendFlow.session.artifact?.attempts.isEmpty != false
    }

    private func goBack() {
        // Back from a built transaction is the review it was built from: the
        // build is let go (it stays resumable), the form is not. A resumed
        // one goes back to the list it came from.
        if currentStep == .confirm, store.sendFlow.session.artifact != nil {
            if isShowingResumedSend {
                isShowingResumedSend = false
                store.sendFlow.invalidateSession()
                go(to: .from)
                return
            }
            withAnimation(reduceMotion ? nil : .snappy(duration: 0.28)) {
                store.sendFlow.invalidateSession()
            }
            return
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
        // Core reads the payload — bare address or payment URI — against the
        // network and the asset being sent, and hands back the stored form
        // of the address with the amount and memo the code asks for. Without
        // an asset there is nothing to judge the code against.
        guard let coin = selectedNetworkSendCoin else {
            qrScannerErrorMessage = AppLocalization.string("The scanned QR code does not contain a valid address for the selected asset.")
            return
        }
        do {
            let payment = try readScannedPayment(chain: coin.chain, tokenContract: coin.contractAddress, payload: payload)
            store.sendFlow.address = payment.address
            if let amount = payment.amount { store.sendFlow.amount = AmountPresentation.decimalFieldText(amount) }
            if let memo = payment.memo {
                store.sendFlow.memoKind = memo.kind
                store.sendFlow.memoText = memo.value
            }
            scannedPayment = payment
            qrScannerErrorMessage = nil
        } catch {
            scannedPayment = nil
            qrScannerErrorMessage = userErrorMessage(error)
        }
    }

    /// "From the code: 0.1 BTC and Destination Tag 123456." — what the scan
    /// filled in besides the address, while that address stands.
    private var scanNotice: String? {
        guard let payment = scannedPayment, payment.address == store.sendFlow.address else { return nil }
        var filled: [String] = []
        if let amount = payment.amount, let coin = selectedNetworkSendCoin {
            filled.append(AppLocalization.format("scan.filled.part_format", amount, coin.symbol))
        }
        if let memo = payment.memo {
            filled.append(AppLocalization.format("scan.filled.part_format", memo.kind.localizedTitle, memo.value))
        }
        guard !filled.isEmpty else { return nil }
        return AppLocalization.format(
            "scan.filled_format", filled.formatted(.list(type: .and).locale(AppLocalization.locale)))
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
                    Label(walletName, systemImage: "wallet.bifold")
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
