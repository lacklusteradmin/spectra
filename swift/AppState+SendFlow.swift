import Foundation
import SwiftUI
extension AppState {
    func beginSend() {
        guard let firstWallet = sendEnabledWallets.first else { return }
        sendFlow.walletId = firstWallet.id
        sendFlow.holdingKey = availableSendCoins(for: sendFlow.walletId).first?.holdingKey ?? ""
        sendFlow.resetComposer()
        syncSendAssetSelection()
        sendFlow.isPresented = true
    }
    func syncSendAssetSelection() {
        let availableHoldingKeys = availableSendCoins(for: sendFlow.walletId).map(\.holdingKey)
        if !availableHoldingKeys.contains(sendFlow.holdingKey) { sendFlow.holdingKey = availableHoldingKeys.first ?? "" }
        // Keep EIP-1559 fees and manual nonce when switching within the EVM
        // family; clear them when leaving it.
        if selectedSendCoin?.isEVMChain != true {
            sendFlow.clearEvmOverrides()
        }
        sendFlow.invalidateSession()
        sendFlow.clearPreview()
        sendFlow.clearDestinationCheck()
    }
    func cancelSend() { sendFlow.close() }
    var selectedSendCoin: AssetHolding? {
        availableSendCoins(for: sendFlow.walletId).first(where: { $0.holdingKey == sendFlow.holdingKey })
    }
    var sendAmountDecimals: UInt32? {
        guard let coin = selectedSendCoin else { return nil }
        return assetPrecision?.byDeploymentId[coin.holdingKey]
    }
    /// The amount field as core reads it.
    var sendAmountInput: String { AmountPresentation.canonicalDecimalInput(sendFlow.amount) }
    var sendAmountIsValid: Bool {
        guard let decimals = sendAmountDecimals else { return false }
        return isValidAmountInput(text: sendAmountInput, maxDecimals: decimals)
    }
    // A provisional quote can load before the user types; it never changes the
    // amount field and is replaced by a quote for the entered amount.
    var sendPreviewAmountInput: String {
        guard sendAmountInput.isEmpty,
              let coin = selectedSendCoin, let decimals = sendAmountDecimals else { return sendAmountInput }
        return sendAmountShortcut(maximum: coin.amount, decimals: decimals, percentage: 10) ?? "0"
    }
    /// The quote core made for the selected holding, if it is current.
    var sendQuote: OwnedSendPreview? {
        guard let coin = selectedSendCoin else { return nil }
        return sendFlow.previewStore.quote(walletId: sendFlow.walletId, coin: coin)
    }
    /// The quote for the amount on screen. A quote for another amount — the
    /// provisional one, or one still in flight — says nothing about this one.
    var sendQuoteForEnteredAmount: OwnedSendPreview? {
        guard let quote = sendQuote,
              quote.amount == sendAmountInput else { return nil }
        return quote
    }
    func sendShortcutAmount(percentage: UInt32) -> String? {
        guard !sendFlow.isPreparingPreview else { return nil }
        return sendQuote?.shortcuts[percentage]
    }
    func sendPreviewDetails(for coin: AssetHolding) -> SendPreviewDetails? {
        sendFlow.previewStore.quote(walletId: sendFlow.walletId, coin: coin)?.details
    }
    private var parsedCustomEvmFees: Result<EvmCustomFeeConfiguration, Error>? {
        // The toggle is cleared outside the EVM family; only EVM preview and
        // submit paths read these fees.
        guard sendFlow.useCustomEvmFees else { return nil }
        return Result {
            try parseEvmCustomFees(
                maxFeeGweiRaw: AmountPresentation.canonicalDecimalInput(sendFlow.customEvmMaxFeeGwei),
                priorityFeeGweiRaw: AmountPresentation.canonicalDecimalInput(sendFlow.customEvmPriorityFeeGwei))
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
        guard sendFlow.evmManualNonceEnabled else { return nil }
        return Int(try parseEvmNonce(raw: sendFlow.evmManualNonce))
    }
    func selectedWalletForSend() -> WalletView? { wallet(for: sendFlow.walletId) }
    /// The pending send the composer can replace as it stands: core's rule,
    /// scoped to the wallet and chain the composer is on.
    ///
    /// The rule is `replaceable_sends`, derived where the records are; what is
    /// left here is the lookup. The chain has to match: a replacement is the
    /// pending send's nonce re-signed *on its own chain*, and the composer
    /// signs for whichever chain it is showing.
    var replaceableSendForSelectedWallet: ReplaceableSend? {
        guard let selectedSendCoin else { return nil }
        return replaceableSends.first {
            $0.walletId == sendFlow.walletId && $0.chainId == selectedSendCoin.chainId
        }
    }
    func replaceableSend(forTransaction transactionId: String) -> ReplaceableSend? {
        replaceableSends.first { $0.transactionId == transactionId }
    }
    func prepareReplacementContext(cancel: Bool) async {
        guard let pending = replaceableSendForSelectedWallet else {
            sendFlow.session.error = AppLocalization.string("No pending transaction found for this wallet.")
            return
        }
        await prepareReplacementContext(pending: pending, cancel: cancel)
    }
    func openReplacementComposer(for transactionId: String, cancel: Bool) async -> String? {
        guard let pending = replaceableSend(forTransaction: transactionId) else {
            let message = AppLocalization.string(
                "This transaction is no longer pending, so replacement and cancel are unavailable.")
            sendFlow.session.error = message
            return message
        }
        selectedMainTab = .home
        await Task.yield()
        sendFlow.isPresented = true
        await prepareReplacementContext(pending: pending, cancel: cancel)
        return sendFlow.session.error
    }
    func prepareReplacementContext(pending: ReplaceableSend, cancel: Bool) async {
        sendFlow.invalidateSession()
        let session = sendFlow.session.id
        sendFlow.isPreparingReplacement = true
        defer { if sendFlow.session.id == session { sendFlow.isPreparingReplacement = false } }
        do {
            let draft = try await self.bridge.ready().replacementDraft(
                transactionId: pending.transactionId, cancel: cancel)
            guard sendFlow.session.isCurrent(session) else { return }
            sendFlow.walletId = draft.walletId
            sendFlow.holdingKey = draft.holdingKey
            sendFlow.address = draft.destination
            sendFlow.amount = draft.amount
            sendFlow.evmManualNonceEnabled = true
            sendFlow.evmManualNonce = String(draft.nonce)
            sendFlow.useCustomEvmFees = true
            sendFlow.customEvmMaxFeeGwei = draft.maxFeeGwei
            sendFlow.customEvmPriorityFeeGwei = draft.priorityFeeGwei
            await refreshSendPreview()
        } catch {
            guard sendFlow.session.isCurrent(session) else { return }
            sendFlow.session.error = AppLocalization.format("Unable to prepare replacement context: %@", userErrorMessage(error))
        }
    }
    func prepareSpeedUpContext() async { await prepareReplacementContext(cancel: false) }
    func prepareCancelContext() async { await prepareReplacementContext(cancel: true) }
    func isValidAddress(_ address: String, on chain: Chain) -> Bool {
        isValidSendAddress(chain: chain, address: address)
    }
    /// The address this send is going to, from whatever is in the field.
    /// Core owns resolution.
    func resolveSendDestination(input: String, on chain: Chain) async throws -> SendDestinationResolution {
        try await self.bridge.ready().resolveSendDestination(chainId: chain, input: input)
    }
    func clearHighRiskSendConfirmation() { sendFlow.isShowingHighRiskConfirmation = false }
    /// Signs only. Broadcasting is its own action, to the nodes the user
    /// selects on the signed transaction.
    func confirmSigning(password: String?) async {
        sendFlow.isShowingHighRiskConfirmation = false
        await signPreparedSend(password: password)
    }

    func availableSendCoins(for walletId: String) -> [AssetHolding] { walletDerivedCache.availableSendCoinsByWalletId[walletId] ?? [] }
    var sendEnabledWallets: [WalletView] { walletDerivedCache.sendEnabledWallets }
    var canBeginSend: Bool { !sendEnabledWallets.isEmpty }
    var replacementNonceStateMessage: String? {
        guard let selectedSendCoin, selectedSendCoin.isEVMChain else { return nil }
        guard let pending = replaceableSendForSelectedWallet else {
            return AppLocalization.format(
                "No pending %@ send found for this wallet. Replacement and cancel are available only for pending transactions.",
                selectedSendCoin.chainName)
        }
        var message = AppLocalization.format("Pending %@ transaction detected", pending.symbol)
        if let nonce = pending.recordedNonce {
            message += AppLocalization.format("send.replacement.pendingNonceSuffix", nonce)
        } else {
            message += "."
        }
        let hash = pending.transactionHash
        let shortHash = hash.count > 14 ? "\(hash.prefix(10))...\(hash.suffix(4))" : hash
        message += AppLocalization.format("send.replacement.transactionSuffix", shortHash)
        message += AppLocalization.string(
            pending.canSpeedUp
                ? " Use Speed Up to resend with higher fees or Cancel to submit a 0-value self-transfer using the same nonce."
                : " Use Cancel to submit a 0-value self-transfer using the same nonce. A token transfer cannot be rebuilt from its record, so it cannot be sped up.")
        return message
    }
}
