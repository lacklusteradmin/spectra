import Foundation

// Building, signing and broadcasting the staged send. Core does each step and
// keeps the artifact; `session` adopts its answers for the composer that asked.
extension SendFlowState {
    private func reviewInput() throws -> SendReviewInput {
        if let error = customEvmFeeValidationError ?? evmNonceValidationError {
            throw DisplayedError(error)
        }
        let nonce = try explicitEvmNonce().map(Int64.init)
        let fees = customEvmFeeConfiguration()
        let overrides = nonce == nil && fees == nil ? nil : EvmSendOverridesInput(
            nonce: nonce, customFees: fees, gasLimit: nil, calldataHex: nil,
            signOnly: nil, accessListJson: nil)
        return SendReviewInput(walletId: walletId, holdingKey: holdingKey,
            amount: amountInput, destination: address, overrides: overrides, memo: paymentMemo)
    }

    func build() async {
        guard session.artifact == nil, !session.isBusy else { return }
        do {
            let input = try reviewInput()
            await session.load(operation: .build, prepare: {
                let artifact = try await self.bridge.ready().buildOwnedSend(input: input)
                guard try self.reviewInput() == input else {
                    throw DisplayedError(AppLocalization.string("Send inputs changed. Build the transaction again."))
                }
                return artifact
            }, endpoints: {
                let choices = try await self.bridge.ready().sendEndpoints(chain: $0)
                guard try self.reviewInput() == input else {
                    throw DisplayedError(AppLocalization.string("Send inputs changed. Build the transaction again."))
                }
                return choices
            })
        } catch { session.error = userErrorMessage(error) }
    }

    /// Localize the immutable build-time advisories for both new and resumed sends.
    var pendingHighRiskReasons: [String] {
        guard let artifact = session.artifact else { return [] }
        var reasons = highRiskSendMessages(artifact.review.warnings)
            + evmRecipientMessages(artifact.review.recipientWarnings)
        if artifact.review.requiresSelfSendConfirmation {
            reasons.append(AppLocalization.string("This destination belongs to your wallet. Confirm intentional self-send."))
        }
        if let terms = artifact.review.transferTerms {
            if terms.fee != "0" {
                reasons.append(AppLocalization.format(
                    "The token takes %@ %@ of this amount on the way; the recipient receives %@ %@.",
                    AmountPresentation.localizedDecimal(terms.fee), artifact.symbol,
                    AmountPresentation.localizedDecimal(terms.received), artifact.symbol))
            }
            if let carried = terms.carriedNative {
                reasons.append(AppLocalization.format(
                    "The token goes with %@ %@, the least its output can hold; the recipient keeps it.",
                    AmountPresentation.localizedDecimal(carried), artifact.chainId.gasTokenSymbol))
            }
            if let registration = terms.recipientRegistration {
                reasons.append(AppLocalization.format(
                    "The token has not registered the recipient. This send registers it first for %@ %@, which the token keeps for the recipient.",
                    AmountPresentation.localizedDecimal(registration), artifact.chainId.gasTokenSymbol))
            }
            if let program = terms.hookProgram {
                reasons.append(AppLocalization.format(
                    "This token runs program %@ on every transfer, and it can refuse this one.", program))
            }
        }
        let network = artifact.chainId.displayName
        let amount = AmountPresentation.localizedDecimal(artifact.amount)
        reasons.insert("\(amount) \(artifact.symbol) → \(artifact.recipient) (\(network))", at: 0)
        return reasons
    }

    /// Signs only. Broadcasting is its own action, to the nodes the user
    /// selects on the signed transaction.
    func sign(password: String?, authenticate: () async -> String?) async {
        isShowingHighRiskConfirmation = false
        await session.sign(password: password, authenticate: authenticate, sign: {
            try await self.bridge.ready().signSend(id: $0, reviewDigest: $1, password: $2)
        })
    }

    /// The broadcast artifact, which belongs to the application even after
    /// this composer closes; `nil` if nothing was broadcast.
    func broadcast() async -> SendArtifact? {
        await session.broadcast(submit: { try await self.bridge.ready().broadcastSend(id: $0, endpoints: $1) })
    }

    func loadSavedArtifacts() async {
        let request = session.id
        do {
            let artifacts = try await bridge.ready().listSends()
            guard session.isCurrent(request) else { return }
            savedArtifacts = artifacts
        } catch { if session.isCurrent(request) { session.error = userErrorMessage(error) } }
    }

    @discardableResult
    func resume(id: String) async -> Bool {
        invalidateSession()
        return await session.load(operation: .resume,
            prepare: { try await self.bridge.ready().inspectSend(id: id) },
            endpoints: { try await self.bridge.ready().sendEndpoints(chain: $0) })
    }

    /// What core says about the last send, judged from its stored record.
    func updateVerificationNotice() async {
        let request = session.id
        guard let transactionId = session.artifact?.id else {
            clearVerificationNotice()
            return
        }
        guard let notice = try? await bridge.ready().sendVerificationNotice(transactionId: transactionId),
            session.isCurrent(request), session.artifact?.id == transactionId
        else { return }
        verificationNotice = notice.notice
        verificationNoticeIsWarning = notice.isWarning
    }
}
