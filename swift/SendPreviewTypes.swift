import Foundation

// Preview types are UniFFI-generated from `core/src/send/`. What is left here
// is the composer's hold on the latest quote.

@MainActor
@Observable
final class SendPreviewStore {
    /// The latest quote core returned; it names the wallet, holding and
    /// network it was made for.
    private(set) var quote: OwnedSendPreview?

    func apply(_ quote: OwnedSendPreview?) { self.quote = quote }

    /// The quote, if it was made for this wallet's holding on its network.
    /// A quote for another holding, or a mainnet quote shown on a testnet,
    /// is none.
    func quote(walletId: String, coin: AssetHolding) -> OwnedSendPreview? {
        guard let quote, quote.walletId == walletId, quote.holdingKey == coin.holdingKey,
              quote.chainId == coin.chainId else { return nil }
        return quote
    }

    func reset() { quote = nil }
}
