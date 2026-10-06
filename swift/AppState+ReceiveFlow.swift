import Foundation
import SwiftUI
extension AppState {
    /// Open with no wallet selected unless there is only one to choose.
    func beginReceive() {
        let wallets = receiveEnabledWallets
        guard !wallets.isEmpty else { return }
        receiveFlow.walletId = wallets.count == 1 ? wallets[0].id : ""
        syncReceiveAssetSelection()
        receiveFlow.isPresented = true
    }
    func syncReceiveAssetSelection() {
        receiveFlow.holdingKey = selectedReceiveCoin(for: receiveFlow.walletId)?.holdingKey ?? ""
        receiveFlow.clearAddress()
    }
    func cancelReceive() {
        receiveFlow.isPresented = false
        receiveFlow.clearAddress()
    }
    func refreshReceiveAddress() async {
        let requestId = UUID()
        receiveFlow.requestId = requestId
        receiveFlow.resolvedAddress = ""
        receiveFlow.error = nil
        receiveFlow.isResolving = false
        guard let wallet = wallet(for: receiveFlow.walletId),
            let coin = selectedReceiveCoin(for: receiveFlow.walletId) else { return }
        let chain = coin.chain
        receiveFlow.isResolving = true
        defer {
            if receiveFlow.requestId == requestId { receiveFlow.isResolving = false }
        }
        do {
            let address = try await self.bridge.ready().receiveAddress(
                walletId: wallet.id, chain: chain, reserve: true)
            guard !Task.isCancelled, receiveFlow.requestId == requestId,
                receiveFlow.walletId == wallet.id, receiveFlow.holdingKey == coin.holdingKey else { return }
            receiveFlow.resolvedAddress = address ?? ""
            if address == nil { receiveFlow.error = AppLocalization.string("No receive address is available for this wallet and network.") }
        } catch {
            guard !Task.isCancelled, receiveFlow.requestId == requestId else { return }
            receiveFlow.error = userErrorMessage(error)
        }
    }
    func availableReceiveCoins(for walletId: String) -> [AssetHolding] { walletDerivedCache.availableReceiveCoinsByWalletId[walletId] ?? [] }
    /// Choose the native holding, or the first token if none is native,
    /// for the receive screen's symbol and icon. This does not select an address.
    func selectedReceiveCoin(for walletId: String) -> AssetHolding? {
        let receiveCoins = availableReceiveCoins(for: walletId)
        return receiveCoins.first { $0.contractAddress == nil } ?? receiveCoins.first
    }
    var receiveEnabledWallets: [WalletView] { walletDerivedCache.receiveEnabledWallets }
    var canBeginReceive: Bool { !receiveEnabledWallets.isEmpty }
}
