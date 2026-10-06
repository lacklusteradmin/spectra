import Foundation
import UIKit
import Testing

@testable import Spectra

/// Verify the identity-based core lookup reaches real bundled images through Swift.
@MainActor
struct CoinBadgeArtworkTests {
    /// A mark that is named has to load; a coin that names none draws its
    /// letter, which `CoinBadge` does for an empty name. The catalog carries
    /// tokens with no artwork on purpose, so "not nil" alone would fail on a
    /// row that is behaving correctly.
    private func assertDrawsItsMark(_ badge: CoinBadge, _ what: @autoclosure () -> String) {
        guard !badge.artworkName.isEmpty else { return }
        #expect(UIImage(named: badge.artworkName) != nil, "\(what()) drew a letter, not its mark")
    }

    @Test func catalogTokenIdentityLoadsItsArtwork() {
        for entry in CoreReferenceTables.assetWiki {
            let badge = CoinBadge(artworkName: entry.face.artworkName, fallbackText: entry.symbol, color: .orange)
            assertDrawsItsMark(badge, entry.symbol)
        }
    }

    @Test func heldDeploymentsUseTheirOwnArtwork() {
        for token in listAllBuiltinTokenDeployments() {
            let holding = AssetHolding(
                id: token.deploymentId, name: token.name, symbol: token.symbol, coingeckoId: token.coingeckoId,
                chainId: token.chainId, tokenStandard: token.tokenStandard,
                contractAddress: token.contract.isEmpty ? nil : token.contract, amount: "0")
            let badge = CoinBadge(artworkName: holding.artworkName, fallbackText: token.symbol, color: .orange)
            #expect(badge.artworkName == token.artworkName, "\(token.deploymentId)")
            assertDrawsItsMark(badge, token.deploymentId)
        }
        #expect(AssetPresentationCatalog.artwork(deploymentId: "base:native") == "ethereum")
        #expect(AssetPresentationCatalog.artwork(deploymentId: nil) == "")
    }

    /// Network wiki artwork must also reach real bundled images.
    @Test func everyNetworkWikiFaceLoadsItsMark() {
        for chain in CoreReferenceTables.chainWiki {
            let badge = CoinBadge(
                artworkName: chain.face.artworkName, fallbackText: chain.name, color: .orange)
            #expect(UIImage(named: badge.artworkName) != nil, "\(chain.name)'s wiki face drew a letter")
        }
    }

    /// A chain's own ticker draws the chain, not the coin it pays fees in.
    /// Base's gas is ETH, so these two live side by side and the badge must
    /// keep them apart.
    @Test func aChainBadgeStillDrawsTheChain() throws {
        for chain in Chain.all {
            let native = try #require(AssetHolding.nativeChainBadge(for: chain), "\(chain.displayName)")
            let badge = CoinBadge(
                artworkName: native.artworkName, fallbackText: chain.gasTokenSymbol, color: native.color)
            #expect(UIImage(named: badge.artworkName) != nil, "\(chain.displayName) drew a letter")
        }
        let base = CoinBadge(
            artworkName: Chain.base.entry?.artworkName,
            fallbackText: "BASE", color: .orange)
        let etherOnBase = CoinBadge(
            artworkName: AssetHolding.fixture(name: "", symbol: "ETH", chainId: Chain.base, amount: "0").artworkName,
            fallbackText: "ETH", color: .orange)
        #expect(base.artworkName == "base")
        #expect(etherOnBase.artworkName == "ethereum")
        let worldChain = CoinBadge(
            artworkName: Chain.worldChain.entry?.artworkName,
            fallbackText: "ETH", color: .orange)
        let etherOnWorldChain = CoinBadge(
            artworkName: AssetHolding.fixture(name: "", symbol: "ETH", chainId: Chain.worldChain, amount: "0").artworkName,
            fallbackText: "ETH", color: .orange)
        #expect(worldChain.artworkName == "worldcoin")
        #expect(etherOnWorldChain.artworkName == "ethereum")
        assertDrawsItsMark(worldChain, "World Chain")
        assertDrawsItsMark(etherOnWorldChain, "Ether on World Chain")
    }

    /// A custom contract cannot borrow USDC artwork by copying its symbol.
    @Test func anUnknownCoinFallsBackToItsLetter() {
        let unknown = CoinBadge(
            artworkName: AssetHolding.fixture(name: "USD Coin", symbol: "USDC", coingeckoId: "usd-coin", chainId: Chain.ethereum,
                tokenStandard: "ERC-20", contractAddress: "0xdead", amount: "0").artworkName,
            fallbackText: "USDCE", color: .orange)
        #expect(unknown.artworkName == "")
    }
}
