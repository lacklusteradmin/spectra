import Foundation
import Testing
import SwiftUI
import UIKit

@testable import Spectra

@MainActor
@Suite(.isolatedAppState)
struct TokenRegistryPresentationTests: IsolatedAppStateSuite {
    @Test func customEditorPersistsBothPriceSourcesAndKeepsIdentity() async throws {
        let state = makeState()
        let identifier = "0x1111111111111111111111111111111111111111"
        let added = await state.tokenPreferences.addCustom(chain: .ethereum, symbol: "DEMO", name: "Demo",
            contractAddress: identifier, coinpaprikaId: "demo-token", decimals: 6)
        #expect(added == nil)
        let entry = try #require(state.tokenPreferences.entries.first { !$0.isBuiltIn })
        #expect(entry.token.coinpaprikaId == "demo-token")
        #expect(entry.token.coingeckoId == "")
        let edited = await state.tokenPreferences.addCustom(chain: .ethereum, symbol: "NEW", name: "New Demo",
            contractAddress: identifier, coingeckoId: "demo", coinpaprikaId: "demo-new", decimals: 8, editing: entry)
        #expect(edited == nil)
        let current = try #require(state.tokenPreferences.entries.first { $0.id == entry.id })
        #expect(current.token.tokenId == entry.token.tokenId)
        #expect(current.token.name == "New Demo")
        #expect(current.token.coinpaprikaId == "demo-new")
        #expect(current.token.coingeckoId == "demo")
        let reopened = WalletServiceBridge(databasePath: directory.appendingPathComponent("state.sqlite").path,
            service: try WalletService(endpoints: []))
        let persisted = try await reopened.ready().appState()
        #expect(persisted.tokenPreferences.first { $0.id == entry.id }?.token == current.token)
    }

    /// The form names the protocol: Tron offers two, so none named is refused
    /// in words, and a TRC-10 ID is kept as TRC-10 when named.
    @Test func aTokenIsAddedUnderTheStandardTheFormNames() async throws {
        let state = makeState()
        let standards = try #require(Chain.tron.entry?.tokenStandards)
        #expect(standards.map(\.standard) == ["TRC-10", "TRC-20"])
        #expect(standards.map(\.identifierPrompt) == ["Token ID", "Contract Address"])
        let unnamed = await state.tokenPreferences.addCustom(
            chain: .tron, symbol: "OLD", name: "Old", contractAddress: "1009999", decimals: 6)
        #expect(unnamed == AppLocalization.string("Choose the standard the token was issued under."))
        let named = await state.tokenPreferences.addCustom(
            chain: .tron, standard: "TRC-10", symbol: "OLD", name: "Old", contractAddress: "1009999", decimals: 6)
        #expect(named == nil)
        #expect(state.tokenPreferences.entries.contains { $0.token.tokenStandard == "TRC-10" && $0.token.contract == "1009999" })
    }

    @Test func tokenManagementScreensRenderInARealWindow() async throws {
        let state = makeState()
        let seeded = try await bridge.ready().applyStateCommand(command: .mergeBuiltInTokens)
        state.applyCoreState(seeded.state)
        let entry = try #require(state.tokenPreferences.entries.first { $0.token.symbol == "USDC" })
        let views: [(String, AnyView)] = [
            ("Known tokens", AnyView(NavigationStack { TokenRegistrySettingsView(tokens: state.tokenPreferences) })),
            ("Token detail", AnyView(NavigationStack { TokenRegistryDetailView(tokens: state.tokenPreferences, groupKey: entry.token.tokenId) })),
            ("Custom token form", AnyView(NavigationStack { AddCustomTokenView(tokens: state.tokenPreferences) }))
        ]
        let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previousWindow = scene.windows.first(where: \.isKeyWindow)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 393, height: 852)
        defer {
            window.isHidden = true
            window.rootViewController = nil
            previousWindow?.makeKey()
        }
        for (name, view) in views {
            window.rootViewController = UIHostingController(rootView: view)
            window.makeKeyAndVisible()
            try await Task.sleep(for: .milliseconds(300))
            window.layoutIfNeeded()
            let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                #expect(window.drawHierarchy(in: window.bounds, afterScreenUpdates: true))
            }
            let pixels = try #require(image.cgImage?.dataProvider?.data) as Data
            #expect(Set(pixels).count > 16)
            Attachment.record(image, named: name)
        }
    }
}
