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

/// What the wallet projection says a preview is for: the holding the composer
/// has selected, if any, and the amount to quote.
struct SendPreviewTarget {
    let coin: AssetHolding?
    let amount: String
}

extension SendFlowState {
    func previewInput(for target: SendPreviewTarget) -> SendPreviewInputSnapshot {
        SendPreviewInputSnapshot(walletId: walletId, holdingKey: holdingKey,
            amount: target.amount, destination: address,
            nonceEnabled: evmManualNonceEnabled, nonce: evmManualNonce,
            feesEnabled: useCustomEvmFees, maxFee: customEvmMaxFeeGwei,
            priorityFee: customEvmPriorityFeeGwei)
    }

    /// Every completion, including errors and loading cleanup, belongs to one
    /// request. Core quotes and checks the recipient in the same call.
    ///
    /// `target` is read when the request starts and again when its answer
    /// lands: an answer for a holding or amount the projection has since
    /// moved off is not adopted.
    func refreshPreview(target: () -> SendPreviewTarget) async {
        let requestId = UUID()
        previewRequestId = requestId
        let start = target()
        let input = previewInput(for: start)
        clearDestinationCheck()
        guard start.coin != nil else {
            isPreparingPreview = false
            previewStore.reset()
            return
        }
        isPreparingPreview = true
        defer { if previewRequestId == requestId { isPreparingPreview = false } }
        do {
            // Capture and validate all inputs before the first suspension.
            let nonce = try explicitEvmNonce().map(Int64.init)
            let fees = customEvmFeeConfiguration()
            if let error = customEvmFeeValidationError {
                throw DisplayedError(error)
            }
            let preview = try await bridge.ready().previewOwnedSend(
                walletId: input.walletId, holdingKey: input.holdingKey, amount: input.amount,
                destination: input.destination, explicitNonce: nonce, customFees: fees)
            adoptPreviewResult(.success(preview), requestId: requestId, input: input, target: target)
        } catch {
            adoptPreviewResult(.failure(error), requestId: requestId, input: input, target: target)
        }
    }

    func adoptPreviewResult(_ result: Result<OwnedSendPreview?, Error>, requestId: UUID,
                            input: SendPreviewInputSnapshot, target: () -> SendPreviewTarget) {
        let current = target()
        guard !Task.isCancelled, previewRequestId == requestId, previewInput(for: current) == input else { return }
        switch result {
        case .success(let preview):
            previewStore.apply(preview)
            session.error = nil
            clearVerificationNotice()
            adoptRecipientCheck(preview?.recipient, coin: current.coin)
        case .failure(let error):
            guard !(error is CancellationError) else { return }
            previewStore.reset()
            session.error = userErrorMessage(error)
        }
    }

    /// Core checks the destination beside the quote; this only words it.
    /// An own address says so and nothing else: whether it is funded or
    /// fresh is no reason to double-check a transfer between the user's
    /// own wallets.
    private func adoptRecipientCheck(_ check: RecipientCheck?, coin: AssetHolding?) {
        guard let check, let coin else { return }
        if check.isOwnAddress {
            destinationRiskWarning = nil
            destinationInfoMessage = AppLocalization.string("This address belongs to one of your wallets.")
        } else if let activity = check.activity {
            let messages = chainRiskProbeMessages(chainName: coin.chainName, symbol: coin.symbol, activity: activity)
            destinationRiskWarning = messages.warning
            destinationInfoMessage = messages.info
        } else {
            destinationInfoMessage = AppLocalization.string("Unable to verify this address's activity. Try again later.")
        }
    }
}
