import Foundation

/// The send composer: its form, its preview and the send it stages.
///
/// Transient native flow state; persisted domain data remains in core, which
/// quotes, builds, signs and broadcasts. What only the wallet projection knows
/// — the selected holding, whether a wallet signs with a password, what a
/// completed send sets off — `AppState` supplies.
@MainActor
@Observable
final class SendFlowState {
    @ObservationIgnored let bridge: WalletServiceBridge // Service identity is not view state.
    var walletId: String = ""
    /// A memo belongs to the network it was typed for, so another holding clears it.
    var holdingKey: String = "" {
        didSet { if holdingKey != oldValue { clearMemo() } }
    }
    var amount: String = ""
    var address: String = ""
    /// The destination tag or memo for the recipient, where the network takes one.
    var memoKind: PaymentMemoKind? = nil
    var memoText: String = ""
    /// What core's recipient check says deserves a second look: an address
    /// this wallet has never sent to, one with no history on chain.
    var destinationWarnings: [String] = []
    var destinationInfoMessage: String? = nil
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
    let session = SendSession()
    var savedArtifacts: [SendArtifact] = []
    let previewStore = SendPreviewStore()
    /// An override turned on starts from the quote's own terms: an edit of
    /// what the network asks, not a blank field that reads as an error the
    /// moment it appears.
    var useCustomEvmFees: Bool = false {
        didSet {
            guard useCustomEvmFees, !oldValue, let terms = quotedEvmTerms,
                  customEvmMaxFeeGwei.isEmpty, customEvmPriorityFeeGwei.isEmpty else { return }
            customEvmMaxFeeGwei = AmountPresentation.decimalFieldText(terms.maxFeePerGasGwei)
            customEvmPriorityFeeGwei = AmountPresentation.decimalFieldText(terms.maxPriorityFeePerGasGwei)
        }
    }
    var customEvmMaxFeeGwei: String = ""
    var customEvmPriorityFeeGwei: String = ""
    var evmManualNonceEnabled: Bool = false {
        didSet {
            guard evmManualNonceEnabled, !oldValue, let terms = quotedEvmTerms, evmManualNonce.isEmpty else { return }
            evmManualNonce = String(terms.nonce)
        }
    }
    var evmManualNonce: String = ""
    /// The last quote's EVM terms, when it was an EVM quote.
    var quotedEvmTerms: EvmSendPreview? {
        guard case .ethereum(let terms)? = previewStore.quote?.preview else { return nil }
        return terms
    }
    @ObservationIgnored var previewRequestId = UUID() // Reject every completion of a superseded preview.
    var isPresented: Bool = false {
        didSet { if oldValue && !isPresented { resetComposer() } }
    }
    /// The tab whose stack the composer is pushed on: the one it was opened
    /// from, so Back returns to the page that opened it.
    var presentingTab: MainAppTab = .home

    init(bridge: WalletServiceBridge) { self.bridge = bridge }

    func clearVerificationNotice() {
        verificationNotice = nil
        verificationNoticeIsWarning = false
    }

    func invalidateSession() {
        session.reset()
        previewRequestId = UUID()
        isPreparingPreview = false
        isPreparingReplacement = false
        isShowingHighRiskConfirmation = false
        clearVerificationNotice()
    }

    func clearPreview() {
        previewRequestId = UUID()
        previewStore.reset()
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
        invalidateSession()
        clearPreview()
        amount = ""
        address = ""
        clearMemo()
        clearDestinationCheck()
        clearEvmOverrides()
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
