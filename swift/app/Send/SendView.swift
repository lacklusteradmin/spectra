import Foundation
import SwiftUI

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
    @State private var currentStep: SendFlowStep = .from
    @State private var flowDirection: Int = 1
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var sendWalletPassword = ""
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
            SendRecipientPage(store: store)
        case .amount:
            SendAmountPage(store: store)
        case .confirm:
            if let progress = store.sendFlow.session.progress {
                SendStagesView(store: store, progress: progress)
            } else {
                SendConfirmationStep(store: store)
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
            if let progress = store.sendFlow.session.progress, progress.next == .done, progress.canRebroadcast {
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

    /// Core's next step for the staged send; none until one is built.
    private var nextStep: SendNextStep? { store.sendFlow.session.progress?.next }

    private var primaryActionTitle: String {
        switch currentStep {
        case .from, .recipient: return "Next"
        case .amount: return "Review"
        case .confirm:
            // Three stages, each its own action (docs/ARCHITECTURE.md): what was built
            // is inspectable before signing, and a signed send waits for the
            // user to choose which nodes receive it.
            return nextStep?.title ?? "Build Transaction"
        }
    }

    private var primaryActionSystemImage: String {
        guard currentStep == .confirm else { return "chevron.right" }
        return nextStep?.systemImage ?? "hammer.fill"
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
            spectraHaptic(nextStep == .done ? .light : .heavy)
            switch nextStep {
            case nil:
                isShowingResumedSend = false
                let session = store.sendFlow.session.id
                Task {
                    guard store.sendFlow.session.isCurrent(session) else { return }
                    await store.sendFlow.build()
                }
            case .sign: store.sendFlow.isShowingHighRiskConfirmation = true
            case .broadcast, .rebroadcast: startBroadcast()
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
            return store.sendFlow.currentRecipient != nil
        case .amount:
            return store.sendAmountIsValid && store.sendAmountRefusal == nil && store.sendFlow.previewError == nil
        case .confirm:
            if let progress = store.sendFlow.session.progress {
                guard !isSendBusy else { return false }
                switch progress.next {
                case .broadcast: return !store.sendFlow.session.selectedEndpoints.isEmpty
                case .rebroadcast:
                    return !store.sendFlow.session.selectedEndpoints.isEmpty && progress.canRebroadcast
                case .sign, .done: return true
                }
            }
            return !isSendBusy
                && store.selectedWalletForSend() != nil
                && selectedCoin != nil
                && store.sendFlow.currentRecipient != nil
                && store.sendAmountIsValid
                && store.sendAmountRefusal == nil
                && store.sendFlow.quoteIsCurrent
                && store.sendFlow.customEvmFeeValidationError == nil
                && store.sendFlow.evmNonceValidationError == nil
        }
    }

    /// The flow's name while it is composed — the step bar and the page's
    /// own heading say which step this is — and the stage once it is built,
    /// in title case as every navigation title is.
    private var navigationTitle: String {
        guard currentStep == .confirm else { return "Send" }
        guard let progress = store.sendFlow.session.progress else { return "Review Transfer" }
        switch progress.next {
        case .sign: return "Check and Sign"
        case .broadcast: return "Submit Transaction"
        case .rebroadcast: return "Submission Results"
        case .done:
            switch progress.transaction?.status {
            case .confirmed: return "Transfer Complete"
            case .failed: return "Transaction Failed"
            case .pending, nil: return "Waiting for Confirmation"
            }
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
