import Foundation

/// Raw form identity, including invalid edits that may parse to the same nil value.
struct SendPreviewInputSnapshot: Equatable {
    let walletId: String
    let holdingKey: String
    let amount: String
    let destination: String
    let nonceEnabled: Bool
    let nonce: String
    let feesEnabled: Bool
    let maxFee: String
    let priorityFee: String
}

extension SendFlowState {
    /// The form as a quote reads it.
    var previewInput: SendPreviewInputSnapshot {
        SendPreviewInputSnapshot(walletId: walletId, holdingKey: holdingKey,
            amount: amountInput, destination: address,
            nonceEnabled: evmManualNonceEnabled, nonce: evmManualNonce,
            feesEnabled: useCustomEvmFees, maxFee: customEvmMaxFeeGwei,
            priorityFee: customEvmPriorityFeeGwei)
    }

    /// Every completion, including errors and loading cleanup, belongs to one
    /// request. Core quotes and checks the recipient in the same call, and
    /// quotes an empty amount provisionally, for a share of the holding.
    ///
    /// The selected holding is read when the request starts and again when
    /// its answer lands: an answer for a form the composer has since moved
    /// off is not adopted.
    func refreshPreview() async {
        let requestId = UUID()
        previewRequestId = requestId
        let input = previewInput
        previewError = nil
        // An override still being typed is pointed out beside its field.
        // Until it parses there is nothing to quote, and a refused quote
        // would only say so again, worse worded. The last quote and its
        // recipient check stay as what the network said — the fields'
        // placeholders, the estimate rows, the destination warnings — and
        // the quote is not current, so nothing builds from it.
        guard evmNonceValidationError == nil, customEvmFeeValidationError == nil else {
            isPreparingPreview = false
            return
        }
        clearDestinationCheck()
        guard selectedHolding() != nil else {
            isPreparingPreview = false
            previewStore.reset()
            return
        }
        isPreparingPreview = true
        defer { if previewRequestId == requestId { isPreparingPreview = false } }
        do {
            // Capture all inputs before the first suspension.
            let nonce = try explicitEvmNonce().map(Int64.init)
            let fees = customEvmFeeConfiguration()
            let preview = try await bridge.ready().previewOwnedSend(
                walletId: input.walletId, holdingKey: input.holdingKey, amount: input.amount,
                destination: input.destination, explicitNonce: nonce, customFees: fees)
            adoptPreviewResult(.success(preview), requestId: requestId, input: input)
        } catch {
            adoptPreviewResult(.failure(error), requestId: requestId, input: input)
        }
    }

    func adoptPreviewResult(_ result: Result<OwnedSendPreview?, Error>, requestId: UUID,
                            input: SendPreviewInputSnapshot) {
        guard !Task.isCancelled, previewRequestId == requestId, previewInput == input else { return }
        switch result {
        case .success(let preview):
            previewStore.apply(preview)
            previewError = nil
            quotedInput = input
            clearVerificationNotice()
            adoptRecipientCheck(preview?.recipient, coin: selectedHolding())
        case .failure(let error):
            guard !(error is CancellationError) else { return }
            previewStore.reset()
            previewError = userErrorMessage(error)
        }
    }

    /// Core checks the destination beside the quote; this only words it.
    /// An own address says so and nothing else: whether it is funded or
    /// fresh is no reason to double-check a transfer between the user's
    /// own wallets. A new destination is said here, before the build, in
    /// the words the build's review uses for it.
    private func adoptRecipientCheck(_ check: RecipientCheck?, coin: AssetHolding?) {
        guard let check, let coin else { return }
        if check.isOwnAddress {
            destinationWarnings = []
            destinationInfoMessage = AppLocalization.string("This address belongs to one of your wallets.")
            return
        }
        var warnings = check.isNewToWallet ? [HighRiskSendWarning.newAddress.localizedText] : []
        if let activity = check.activity {
            let messages = chainRiskProbeMessages(chainName: coin.chainName, symbol: coin.symbol, activity: activity)
            warnings += messages.warning.map { [$0] } ?? []
            destinationInfoMessage = messages.info
        } else {
            destinationInfoMessage = AppLocalization.string("Unable to verify this address's activity. Try again later.")
        }
        destinationWarnings = warnings
    }
}
