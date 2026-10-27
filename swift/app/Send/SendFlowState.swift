import Foundation

/// The send composer: its form, its preview and the send it stages.
///
/// Transient native flow state; persisted domain data remains in core, which
/// resolves the recipient, quotes, builds, signs and broadcasts. Every
/// request the composer makes starts here, when what it depends on changes
/// while the composer is on screen, and only its latest answer is adopted.
/// What only the wallet projection knows — the selected holding, whether a
/// wallet signs with a password, what a completed send sets off — `AppState`
/// supplies.
@MainActor
@Observable
final class SendFlowState {
    @ObservationIgnored let bridge: WalletServiceBridge // Service identity is not view state.
    /// The holding the composer has selected, as the wallet projection has it
    /// now. `AppState` supplies it; requests read it when they start and
    /// when they land.
    @ObservationIgnored var selectedHolding: @MainActor () -> AssetHolding? = { nil }
    var walletId: String = "" {
        didSet { if walletId != oldValue { recipientChanged() } }
    }
    /// A memo belongs to the network it was typed for, so another holding clears it.
    var holdingKey: String = "" {
        didSet {
            guard holdingKey != oldValue else { return }
            clearMemo()
            recipientChanged()
        }
    }
    var amount: String = "" {
        didSet { if amount != oldValue { schedulePreview() } }
    }
    var address: String = "" {
        didSet { if address != oldValue { recipientChanged() } }
    }
    /// The destination tag or memo for the recipient, where the network takes one.
    var memoKind: PaymentMemoKind? = nil
    var memoText: String = ""
    /// What the last scanned code filled in, said for as long as the
    /// recipient is still the one it named.
    var scannedPayment: ScannedPayment?
    /// What core's recipient check says deserves a second look: an address
    /// this wallet has never sent to, one with no history on chain.
    var destinationWarnings: [String] = []
    var destinationInfoMessage: String? = nil
    /// Core's resolution of the recipient as typed, once it has answered
    /// for the address in the field now.
    private(set) var resolvedRecipient: SendDestinationResolution?
    /// Why core turned the recipient as typed down.
    private(set) var recipientProblem: SendRecipientProblem?
    private(set) var isResolvingRecipient = false
    /// The recipient is checked inside the preview request.
    var isCheckingDestination: Bool {
        isPreparingPreview && !address.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
    var isShowingHighRiskConfirmation: Bool = false
    var verificationNotice: String? = nil
    var verificationNoticeIsWarning: Bool = false
    var isPreparingReplacement: Bool = false
    var isPreparingPreview: Bool = false
    /// Why the last quote failed. Separate from `session.error`, which is the
    /// build's, the signature's or the broadcast's: a quote that did not land
    /// leaves the fee unknown, and no build goes ahead without one.
    var previewError: String? = nil
    /// The form the last quote answered. A quote is current only while the
    /// form still reads the same.
    var quotedInput: SendPreviewInputSnapshot?
    let session: SendSession
    var savedArtifacts: [SendArtifact] = []
    let previewStore = SendPreviewStore()
    /// An override turned on starts from the quote's own terms: an edit of
    /// what the network asks, not a blank field that reads as an error the
    /// moment it appears.
    var useCustomEvmFees: Bool = false {
        didSet {
            guard useCustomEvmFees != oldValue else { return }
            schedulePreview()
            guard useCustomEvmFees, let terms = quotedEvmTerms,
                  customEvmMaxFeeGwei.isEmpty, customEvmPriorityFeeGwei.isEmpty else { return }
            customEvmMaxFeeGwei = AmountPresentation.decimalFieldText(terms.maxFeePerGasGwei)
            customEvmPriorityFeeGwei = AmountPresentation.decimalFieldText(terms.maxPriorityFeePerGasGwei)
        }
    }
    var customEvmMaxFeeGwei: String = "" {
        didSet { if customEvmMaxFeeGwei != oldValue { schedulePreview() } }
    }
    var customEvmPriorityFeeGwei: String = "" {
        didSet { if customEvmPriorityFeeGwei != oldValue { schedulePreview() } }
    }
    var evmManualNonceEnabled: Bool = false {
        didSet {
            guard evmManualNonceEnabled != oldValue else { return }
            schedulePreview()
            guard evmManualNonceEnabled, let terms = quotedEvmTerms, evmManualNonce.isEmpty else { return }
            evmManualNonce = String(terms.nonce)
        }
    }
    var evmManualNonce: String = "" {
        didSet { if evmManualNonce != oldValue { schedulePreview() } }
    }
    /// The last quote's EVM terms, when it was an EVM quote.
    var quotedEvmTerms: EvmFeeTerms? {
        guard case .evm(let terms)? = previewStore.quote?.preview.terms else { return nil }
        return terms
    }
    @ObservationIgnored var previewRequestId = UUID() // Reject every completion of a superseded preview.
    @ObservationIgnored private var previewTask: Task<Void, Never>?
    @ObservationIgnored private var recipientTask: Task<Void, Never>?
    @ObservationIgnored private var progressTask: Task<Void, Never>?
    var isPresented: Bool = false {
        didSet {
            guard isPresented != oldValue else { return }
            if isPresented { recipientChanged() } else { resetComposer() }
        }
    }
    /// The tab whose stack the composer is pushed on: the one it was opened
    /// from, so Back returns to the page that opened it.
    var presentingTab: MainAppTab = .home

    init(bridge: WalletServiceBridge) {
        self.bridge = bridge
        session = SendSession(bridge: bridge)
    }

    func clearVerificationNotice() {
        verificationNotice = nil
        verificationNoticeIsWarning = false
    }

    /// Let the staged send go. The form stays, and is quoted again: the
    /// quote the send was built from may be stale by now.
    func invalidateSession() {
        session.reset()
        progressTask?.cancel()
        previewRequestId = UUID()
        quotedInput = nil
        isPreparingPreview = false
        isPreparingReplacement = false
        isShowingHighRiskConfirmation = false
        clearVerificationNotice()
        schedulePreview()
    }

    /// Drop the last quote. A quote already scheduled still goes ahead: the
    /// form it reads is the one now on screen.
    func clearPreview() {
        previewRequestId = UUID()
        previewStore.reset()
        quotedInput = nil
        previewError = nil
        isPreparingPreview = false
        isShowingHighRiskConfirmation = false
    }

    func clearDestinationCheck() {
        destinationWarnings = []
        destinationInfoMessage = nil
    }

    func clearEvmOverrides() {
        useCustomEvmFees = false
        customEvmMaxFeeGwei = ""
        customEvmPriorityFeeGwei = ""
        evmManualNonceEnabled = false
        evmManualNonce = ""
    }

    func clearMemo() {
        memoKind = nil
        memoText = ""
    }

    func resetComposer() {
        recipientTask?.cancel()
        progressTask?.cancel()
        invalidateSession()
        clearPreview()
        amount = ""
        address = ""
        clearMemo()
        clearDestinationCheck()
        clearEvmOverrides()
        clearRecipientResolution()
        scannedPayment = nil
    }

    /// The memo as core reads it: none until something is typed.
    var paymentMemo: PaymentMemo? {
        guard let memoKind, !memoText.isEmpty else { return nil }
        return PaymentMemo(kind: memoKind, value: memoText)
    }

    /// Dismissing resets the composer; one that is not on screen is reset here.
    func close() {
        if isPresented { isPresented = false } else { resetComposer() }
    }

    func reset() {
        close()
        walletId = ""
        holdingKey = ""
        savedArtifacts = []
    }

    // MARK: - The recipient

    /// The wallet, holding and address core's resolution answers for.
    private struct RecipientKey: Equatable {
        let walletId: String
        let holdingKey: String
        let address: String
    }

    private var recipientKey: RecipientKey {
        RecipientKey(walletId: walletId, holdingKey: holdingKey, address: address)
    }

    @ObservationIgnored private var resolvedRecipientKey: RecipientKey?

    /// Core's resolution of the recipient, if it answered for the field as
    /// it reads now.
    var currentRecipient: SendDestinationResolution? {
        resolvedRecipientKey == recipientKey ? resolvedRecipient : nil
    }

    private func clearRecipientResolution() {
        resolvedRecipient = nil
        resolvedRecipientKey = nil
        recipientProblem = nil
        isResolvingRecipient = false
    }

    /// The wallet, holding or address changed: ask core about the recipient
    /// once typing pauses, and quote the form again.
    private func recipientChanged() {
        scheduleRecipientResolution()
        schedulePreview()
    }

    /// Ask core again, as when a failure to reach the network may since
    /// have passed.
    func retryRecipientResolution() { scheduleRecipientResolution() }

    /// Read a scanned code — a bare address or a payment URI — against the
    /// network and the asset being sent, and fill in the amount and memo it
    /// asks for. Returns the address in core's stored form. Without an asset
    /// there is nothing to judge the code against.
    func applyScannedPayment(_ payload: String) throws -> String {
        guard let coin = selectedHolding() else {
            throw DisplayedError(AppLocalization.string(
                "The scanned QR code does not contain a valid address for the selected asset."))
        }
        do {
            let payment = try readScannedPayment(chain: coin.chain, tokenContract: coin.contractAddress, payload: payload)
            address = payment.address
            if let amount = payment.amount { self.amount = AmountPresentation.decimalFieldText(amount) }
            if let memo = payment.memo {
                memoKind = memo.kind
                memoText = memo.value
            }
            scannedPayment = payment
            return payment.address
        } catch {
            scannedPayment = nil
            throw error
        }
    }

    private func scheduleRecipientResolution() {
        recipientTask?.cancel()
        clearRecipientResolution()
        let key = recipientKey
        guard isPresented, let chain = selectedHolding()?.chain,
              !address.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        recipientTask = Task { [weak self] in
            // Typing is not checking: the answer comes once typing pauses,
            // not on every keystroke.
            try? await Task.sleep(for: .milliseconds(350))
            guard let self, !Task.isCancelled, recipientKey == key else { return }
            isResolvingRecipient = true
            defer { if recipientKey == key { isResolvingRecipient = false } }
            do {
                let resolution = try await bridge.ready().resolveSendDestination(chainId: chain, input: key.address)
                guard !Task.isCancelled, recipientKey == key else { return }
                resolvedRecipient = resolution
                resolvedRecipientKey = key
            } catch {
                guard !Task.isCancelled, recipientKey == key else { return }
                recipientProblem = SendRecipientProblem(error)
            }
        }
    }

    // MARK: - The quote

    /// Quote the form once editing pauses. A newer edit cancels the wait,
    /// and the request it starts supersedes any still in flight.
    func schedulePreview() {
        previewTask?.cancel()
        quotedInput = nil
        guard isPresented, session.artifact == nil else { return }
        previewTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(350))
            guard let self, !Task.isCancelled, session.artifact == nil else { return }
            await refreshPreview()
        }
    }

    /// Whether the last quote answers the form as it reads now.
    var quoteIsCurrent: Bool { quotedInput != nil && quotedInput == previewInput }

    // MARK: - The staged send

    /// Read core's answer for the staged send again once it was submitted:
    /// its history record moves while the artifact does not.
    func refreshProgress() {
        guard session.artifact?.attempts.isEmpty == false else { return }
        progressTask?.cancel()
        progressTask = Task { [weak self] in await self?.session.refresh() }
    }

    // MARK: - The form as core reads it

    /// The amount field as core reads it.
    var amountInput: String { AmountPresentation.canonicalDecimalInput(amount) }

    private var parsedCustomEvmFees: Result<EvmCustomFeeConfiguration, Error>? {
        // The toggle is cleared outside the EVM family; only EVM preview and
        // submit paths read these fees.
        guard useCustomEvmFees else { return nil }
        return Result {
            try parseEvmCustomFees(
                maxFeeGweiRaw: AmountPresentation.canonicalDecimalInput(customEvmMaxFeeGwei),
                priorityFeeGweiRaw: AmountPresentation.canonicalDecimalInput(customEvmPriorityFeeGwei))
        }
    }
    var customEvmFeeValidationError: String? {
        guard case .failure(let error)? = parsedCustomEvmFees else { return nil }
        switch error {
        case EvmCustomFeeError.InvalidMaxFee: return AppLocalization.string("Enter a valid Max Fee in gwei.")
        case EvmCustomFeeError.InvalidPriorityFee: return AppLocalization.string("Enter a valid Priority Fee in gwei.")
        case EvmCustomFeeError.MaxBelowPriority: return AppLocalization.string("Max Fee must be greater than or equal to Priority Fee.")
        default: return userErrorMessage(error)
        }
    }
    func customEvmFeeConfiguration() -> EvmCustomFeeConfiguration? {
        guard case .success(let fees)? = parsedCustomEvmFees else { return nil }
        return fees
    }
    var evmNonceValidationError: String? {
        do {
            _ = try explicitEvmNonce()
            return nil
        } catch EvmNonceError.Empty {
            return AppLocalization.string("Enter a nonce value for manual nonce mode.")
        } catch EvmNonceError.InvalidInteger {
            return AppLocalization.string("Nonce must be a non-negative integer.")
        } catch EvmNonceError.TooLarge {
            return AppLocalization.string("Nonce value is too large.")
        } catch {
            return userErrorMessage(error)
        }
    }
    func explicitEvmNonce() throws -> Int? {
        guard evmManualNonceEnabled else { return nil }
        return Int(try parseEvmNonce(raw: evmManualNonce))
    }
}

/// Why core turned the recipient down, in its words, and whether asking again
/// could change the answer: only a failure to reach or read the network can,
/// so only that offers Retry.
struct SendRecipientProblem: Equatable {
    let message: String
    let canRetry: Bool

    @MainActor
    init(_ error: Error) {
        message = userErrorMessage(error)
        switch error as? SpectraBridgeError {
        case .some(.Network), .some(.Decode): canRetry = true
        default: canRetry = false
        }
    }
}
