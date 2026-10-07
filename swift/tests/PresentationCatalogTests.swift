import Foundation
import SwiftUI
import Testing
@testable import Spectra

struct PresentationCatalogTests {
    @Test func wikiAndPickerShareTheSameChainTagsAcrossTheBridge() throws {
        for wiki in CoreReferenceTables.chainWiki {
            let chain = try #require(Chain(id: wiki.id))
            let entry = try #require(chain.entry)
            let picker = ChainSelectionDescriptor(chain: chain, entry: entry)
            #expect(!chain.isTestnet)
            #expect(wiki.tags == picker.tags, "\(wiki.id)")
            #expect(wiki.tags.map(\.title).joined(separator: " · ") == picker.tagLine)
        }
    }

    @Test func peercoinReachesThePickerAndWikiWithItsNativeIdentity() throws {
        let rows = ChainSelectionDescriptor.popularOrder(Chain.all)
        let mainnetRows = rows.picked(filter: .tag(.utxo), query: "Peercoin", order: .name, showsTestNetworks: false)
        let mainnet = try #require(mainnetRows.first { $0.id == .peercoin })
        #expect(mainnet.symbol == "PPC")
        #expect(mainnet.artworkName == "peercoin")
        #expect(!mainnet.isTestnet)
        #expect(!mainnetRows.contains { $0.id == .peercoinTestnet })
        let testnetRows = rows.picked(filter: .all, query: "Peercoin", order: .name, showsTestNetworks: true)
        let testnet = try #require(testnetRows.first { $0.id == .peercoinTestnet })
        #expect(testnet.isTestnet)
        #expect(testnet.artworkName == "peercoin")
        for chain in [Chain.peercoin, .peercoinTestnet] {
            #expect(chain.nativeDecimals == 6)
            #expect(chain.mainnetCounterpart == .peercoin)
            let methods = walletSetupDescriptor(chain: chain).options.map(\.method)
            #expect(methods.contains(.watchAddresses))
            #expect(methods.contains(.importPrivateKey))
            #expect(chain.hasSendPreview)
        }
        let coin = try #require(CoreReferenceTables.assetWikiEntry(tokenId: "peercoin"))
        #expect(coin.symbol == "PPC")
        #expect(coin.artworkName == "peercoin")
        #expect(!coin.comment.isEmpty)
        #expect(coin.livesOn.contains { $0.chainId == .peercoin && $0.isNative && $0.decimals == 6 })
        #expect(!coin.livesOn.contains { $0.chainId == .peercoinTestnet })
        #expect(CoreReferenceTables.chainWikiEntry(id: "peercoin") != nil)
    }

    @Test func newEvmNetworksReachTheChainPickerWithTheirOwnIdentity() throws {
        let rows = ChainSelectionDescriptor.popularOrder(Chain.mainnets)
            .picked(filter: .tag(.evm), query: "", order: .name, showsTestNetworks: false)
        for (chain, id, symbol, artwork) in [
            (Chain.plasma, "plasma", "XPL", "plasma"),
            (.monad, "monad", "MON", "monad"),
            (.worldChain, "world-chain", "ETH", "worldcoin")
        ] {
            let row = try #require(rows.first { $0.id == chain }, "\(id)")
            #expect(chain.id == id)
            #expect(row.symbol == symbol)
            #expect(row.artworkName == artwork)
            #expect(row.matches(chain.displayName))
            #expect(!row.isTestnet)
        }
    }

    /// Colour follows deployment identity, never the ticker: a custom token
    /// that calls itself `ETH` is grey, not Ether's colour.
    @Test func coinColorsFollowDeploymentIdentity() throws {
        for token in listTokenDeployments(chain: nil) {
            if let color = token.color {
                #expect(AssetPresentationCatalog.color(deploymentId: token.deploymentId) == color.color)
            }
        }
        let ethereum = try #require(Chain.ethereum.entry)
        #expect(AssetPresentationCatalog.color(deploymentId: ethereum.nativeDeploymentId) == ethereum.color.color)
        #expect(AssetPresentationCatalog.color(deploymentId: "ethereum:erc-20:0xnot-in-the-catalog") == .gray)
    }

    /// A native asset's display name is its chain's, so naming the pair
    /// unconditionally read "Solana on Solana". The subtitle spells the chain
    /// only when it is not already the asset, and the expectations are built
    /// from the shipped formats so the assertion is about that shape rather
    /// than about English.
    @Test func transactionSubtitleNamesTheChainOnlyWhenItIsNotTheAsset() {
        let copy = CommonLocalizationContent.current
        func subtitle(asset: String, chainId: Chain) -> String {
            TransactionRecord(
                id: "tx", kind: .receive, status: .confirmed, walletName: "Main Wallet",
                assetDisplayName: asset, symbol: "SOL", chainId: chainId, amount: "0.1", address: "address"
            ).subtitleText
        }
        func wallet(_ asset: String) -> String { String(format: copy.transactionSubtitleFormat, asset, "Main Wallet") }
        func onChain(_ asset: String, _ chain: String) -> String { String(format: copy.assetOnChainFormat, asset, chain) }

        #expect(subtitle(asset: "Solana", chainId: Chain.solana) == wallet("Solana"))
        #expect(subtitle(asset: "solana", chainId: Chain.solana) == wallet("solana"))
        #expect(subtitle(asset: "USD Coin", chainId: Chain.solana) == wallet(onChain("USD Coin", "Solana")))
        #expect(subtitle(asset: "Bitcoin", chainId: Chain.bitcoinTestnet4) == wallet(onChain("Bitcoin", Chain.bitcoinTestnet4.displayName)))
    }

    /// One string table per declared locale, every table with the same keys —
    /// read from the manifest rather than listed here, so adding a locale
    /// cannot leave a table silently missing.
    @Test func everyDeclaredLocaleShipsTheSameStringTable() throws {
        let locales = try declaredLocales()
        #expect(locales.contains("en"), "the source language must ship")
        let source = try Set(table("en").keys)
        for locale in locales {
            #expect(try Set(table(locale).keys) == source, "\(locale)")
        }
    }

    /// A localized string ships in a locale's table and nothing else; a
    /// locale-independent data file ships once, unsuffixed.
    @Test func localeIndependentDataShipsOnce() {
        for name in ["AppLinks", "BuyProviders"] {
            #expect(Bundle.main.url(forResource: name, withExtension: "json") != nil, "\(name)")
        }
        for name in ["CommonContent", "DiagnosticsContent", "DonationsContent",
                     "EndpointsContent", "ImportFlowContent", "SettingsContent"] {
            #expect(Bundle.main.url(forResource: "\(name).en", withExtension: "json") == nil, "\(name)")
        }
    }

    private struct Manifest: Decodable {
        let availableLocales: [String]
    }

    private func declaredLocales() throws -> [String] {
        let url = try #require(Bundle.main.url(forResource: "RuntimeStrings.manifest", withExtension: "json"), "RuntimeStrings.manifest.json")
        return try JSONDecoder().decode(Manifest.self, from: Data(contentsOf: url)).availableLocales
    }

    private func table(_ locale: String) throws -> [String: String] {
        let name = "RuntimeStrings.\(locale)"
        let url = try #require(Bundle.main.url(forResource: name, withExtension: "json"), "\(name)")
        return try JSONDecoder().decode([String: String].self, from: Data(contentsOf: url))
    }
}
