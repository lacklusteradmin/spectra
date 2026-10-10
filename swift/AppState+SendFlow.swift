import Foundation
import SwiftUI
extension AppState {
    /// Open the composer on `walletId`, or on the first wallet that can send,
    /// and on `holdingKey` when that wallet can send it.
    func beginSend(walletId: String? = nil, holdingKey: String? = nil) {
        let wallets = sendEnabledWallets
        guard let wallet = wallets.first(where: { $0.id == walletId }) ?? wallets.first else { return }
        sendFlow.walletId = wallet.id
        let coins = availableSendCoins(for: sendFlow.walletId)
        sendFlow.holdingKey = coins.first(where: { $0.holdingKey == holdingKey })?.holdingKey ?? coins.first?.holdingKey ?? ""
        sendFlow.resetComposer()
        syncSendAssetSelection()
        presentSendFlow()
    }
    /// Push the composer on the tab the user is on — Home and History each
    /// have it as a destination; Settings has none, so it opens on Home.
    private func presentSendFlow() {
        if selectedMainTab == .settings { selectedMainTab = .home }
        sendFlow.presentingTab = selectedMainTab
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
    var sendAmountIsValid: Bool {
        guard let decimals = sendAmountDecimals else { return false }
        return isValidAmountInput(text: sendFlow.amountInput, maxDecimals: decimals)
    }
    // A provisional quote can load before the user types; it never changes the
    // amount field and is replaced by a quote for the entered amount.
    var sendPreviewAmountInput: String {
        guard sendFlow.amountInput.isEmpty,
              let coin = selectedSendCoin, let decimals = sendAmountDecimals else { return sendFlow.amountInput }
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
              quote.amount == sendFlow.amountInput else { return nil }
        return quote
    }
    /// Why the entered amount cannot be sent, from core's quote for it: more
    /// than the balance, or too little left for the fee.
    var sendAmountRefusal: String? {
        sendQuoteForEnteredAmount?.amountRefusal?.localizedText
    }
    func sendShortcutAmount(percentage: UInt32) -> String? {
        guard !sendFlow.isPreparingPreview else { return nil }
        return sendQuote?.shortcuts[percentage]
    }
    func sendPreviewDetails(for coin: AssetHolding) -> SendPreviewDetails? {
        sendFlow.previewStore.quote(walletId: sendFlow.walletId, coin: coin)?.details
    }
    var sendPreviewTarget: SendPreviewTarget {
        SendPreviewTarget(coin: selectedSendCoin, amount: sendPreviewAmountInput)
    }
    func refreshSendPreview() async {
        await sendFlow.refreshPreview { self.sendPreviewTarget }
    }
    var sendAddressBookEntries: [AddressBookEntry] {
        guard let selectedSendCoin else { return [] }
        return addressBook.entries.filter { $0.chainId == selectedSendCoin.chainId }
    }
    func selectedWalletForSend() -> WalletView? { wallet(for: sendFlow.walletId) }
    /// Whether the composer is pushed on `tab`'s stack.
    func sendFlowBinding(on tab: MainAppTab) -> Binding<Bool> {
        Binding(
            get: { self.sendFlow.isPresented && self.sendFlow.presentingTab == tab },
            set: { self.sendFlow.isPresented = $0 })
    }
    func replaceableSend(forTransaction transactionId: String) -> ReplaceableSend? {
        replaceableSends.first { $0.transactionId == transactionId }
    }
    func openReplacementComposer(for transactionId: String, cancel: Bool) async -> String? {
        guard let pending = replaceableSend(forTransaction: transactionId) else {
            let message = AppLocalization.string(
                "This transaction is no longer pending, so replacement and cancel are unavailable.")
            sendFlow.session.error = message
            return message
        }
        presentSendFlow()
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
    /// The address this send is going to, from whatever is in the field.
    /// Core owns resolution.
    func resolveSendDestination(input: String, on chain: Chain) async throws -> SendDestinationResolution {
        try await self.bridge.ready().resolveSendDestination(chainId: chain, input: input)
    }
    func confirmSigning(password: String?) async {
        await sendFlow.sign(password: password) {
            await self.authenticate(.send, reason: AppLocalization.string("Authorize transaction signing"))
        }
    }
    var stagedSendRequiresPassword: Bool {
        guard let artifact = sendFlow.session.artifact else { return false }
        // An unknown wallet asks for a password rather than signing without one.
        return wallet(for: artifact.walletId)?.signing.requiresPassword ?? true
    }
    func broadcastPreparedSend() async {
        guard let submitted = await sendFlow.broadcast() else { return }
        await handleBroadcastCompletion(submitted)
    }

    /// Application completion is independent of the originating form's lifetime.
    func handleBroadcastCompletion(_ submitted: SendArtifact) async {
        await refreshTransactionProjection()
        // Resolve the completed operation by its own ID, never the current
        // composer or the bounded history summary.
        if let transaction = try? await bridge.ready().transaction(id: submitted.id),
           submitted.attempts.contains(where: { $0.outcome == .accepted }) {
            notifications.startSendLiveActivity(for: transaction, amounts: amounts)
            notifications.requestPermissionAfterSend(settings: committedAppSettings)
            // Broadcast acceptance alone does not establish confirmation.
            await performCoreRefresh(.afterSend(chainId: transaction.chainId))
        }
    }

    func availableSendCoins(for walletId: String) -> [AssetHolding] { walletDerivedCache.availableSendCoinsByWalletId[walletId] ?? [] }
    var sendEnabledWallets: [WalletView] { walletDerivedCache.sendEnabledWallets }
    var canBeginSend: Bool { !sendEnabledWallets.isEmpty }
}
