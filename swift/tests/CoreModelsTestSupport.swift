// Conversions from the Swift view models back into the core records, used by
// the bridge tests to seed state through the same commands the app issues.
//
// They live in the test target because nothing in the app converts in this
// direction: the app renders core's records, and core builds its own. Swift has
// no `#[cfg(test)]`, so the target boundary is the gate.
import Foundation

@testable import Spectra

extension WalletView {
    /// A wallet record with every field a test does not name defaulted. The
    /// app never builds one; core returns them. `addresses` is keyed by chain
    /// id and stored under that chain's slot, as core stores it.
    init(
        id: UUID = UUID(),
        name: String,
        chainId: Chain,
        addresses: [Chain: String] = [:],
        bitcoinXpub: String? = nil,
        derivationPath: String? = nil,
        derivationOverrides: WalletDerivationOverrides = WalletDerivationOverrides(passphrase: nil, hmacKey: nil),
        holdings: [AssetHolding] = [],
        includeInPortfolioTotal: Bool = true,
        signing: WalletSigning = .watchOnly
    ) {
        self.init(
            id: id.uuidString, name: name, chainId: chainId,
            addresses: Dictionary(
                addresses.map { chain, address in (chain.addressSlot, address) },
                uniquingKeysWith: { first, _ in first }),
            accountXpub: bitcoinXpub,
            derivationPath: derivationPath,
            derivationOverrides: derivationOverrides,
            holdings: holdings,
            includeInPortfolioTotal: includeInPortfolioTotal,
            signing: signing,
            restoreHeight: nil
        )
    }

    /// Set this wallet's address on a chain. Passing `nil` clears it.
    ///
    /// Spares the bridge tests rebuilding the record to change one field. The
    /// app never edits a wallet record in place — it renders what core sends
    /// and issues commands back.
    mutating func setAddress(_ address: String?, on chain: Chain) {
        let slot = chain.addressSlot
        guard !slot.isEmpty else { return }
        if let address, !address.isEmpty {
            addresses[slot] = address
        } else {
            addresses.removeValue(forKey: slot)
        }
    }

    /// The authoritative model this view model was rendered from, the same
    /// mapping as core's `WalletView::to_wallet_state`, for seeding a test
    /// through the command the app issues.
    func walletState() -> WalletState {
        // The wallet's own slot first: core reads the first receive address as
        // the primary one.
        let ownSlot = family.addressSlot
        let slots = addresses.keys.sorted { ($0 == ownSlot ? 0 : 1, $0) < ($1 == ownSlot ? 0 : 1, $1) }
        return WalletState(
            id: id, name: name, signing: signing, chainId: chainId,
            includeInPortfolioTotal: includeInPortfolioTotal, xpub: accountXpub,
            derivationPath: derivationPath, derivationOverrides: derivationOverrides,
            holdings: holdings,
            addresses: slots.compactMap { slot in
                guard let address = addresses[slot] else { return nil }
                let owner: Chain
                if slot == chain.addressSlot {
                    owner = chain
                } else {
                    guard let slotOwner = Chain.all.first(where: { $0.addressSlot == slot }) else { return nil }
                    owner = slotOwner
                }
                return WalletAddress(
                    chainId: owner, address: address, kind: "receive", derivationPath: owner == chain ? derivationPath : nil)
            },
            restoreHeight: restoreHeight)
    }
}

extension TransactionRecord {
    /// A record with every field a test does not name left empty. The app never
    /// builds one — core records what was sent and fetched.
    init(
        id: String, walletId: String? = nil, deploymentId: String? = nil, kind: TransactionKind,
        status: TransactionStatus, walletName: String, assetDisplayName: String, symbol: String,
        chainId: Chain, amount: String, address: String, transactionHash: String? = nil,
        nonce: Int64? = nil, failureReason: TransactionFailure? = nil
    ) {
        self.init(
            actions: TransactionActions(recheckUnavailableReason: "Not evaluated", rebroadcastUnavailableReason: "Not evaluated"),
            deploymentId: deploymentId, id: id, walletId: walletId, kind: kind, status: status,
            walletName: walletName, assetDisplayName: assetDisplayName, symbol: symbol,
            chainId: chainId, amount: amount, address: address, transactionHash: transactionHash,
            nonce: nonce, receiptBlockNumber: nil, receiptGasUsed: nil,
            receiptEffectiveGasPriceGwei: nil, receiptNetworkFee: nil,
            feeRateDescription: nil, confirmationCount: nil, confirmedNetworkFee: nil,
            usedChangeOutput: nil, sourceDerivationPath: nil,
            changeDerivationPath: nil, sourceAddress: nil, changeAddress: nil,
            signedTransactionPayload: nil, signedTransactionPayloadFormat: nil,
            failureReason: failureReason, transactionHistorySource: nil,
            createdAtUnix: Date().timeIntervalSince1970)
    }
}

extension AssetHolding {
    /// A holding as core would project it. The id follows core's
    /// `deployment_id` for the EVM-style contracts these tests use.
    static func fixture(
        name: String, symbol: String, coingeckoId: String = "", chainId: Chain, tokenStandard: String = "Native",
        contractAddress: String? = nil, amount: String
    ) -> AssetHolding {
        let id = contractAddress.map { "\(chainId.id):\(tokenStandard.lowercased()):\($0.lowercased())" } ?? "\(chainId.id):native"
        return AssetHolding(
            id: id, name: name, symbol: symbol, coingeckoId: coingeckoId, chainId: chainId,
            tokenStandard: tokenStandard, contractAddress: contractAddress, amount: amount)
    }
}

@MainActor
extension WalletDiagnosticsState {
    func flushPendingPersistence() async { await pendingCommand?.value }
}
