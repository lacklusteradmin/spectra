import Foundation
import Testing
import SwiftUI
import UIKit

@testable import Spectra

@MainActor
@Suite(.isolatedAppState, .serialized)
struct SendComposerViewTests: IsolatedAppStateSuite {
    private let recipient = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

    @Test func composerScreensRenderAcrossAppearanceAndLargeType() async throws {
        let state = try await makeComposerFixture()
        let resolution = SendDestinationResolution(address: recipient, usedEns: false)
        let cases: [(String, ComposerPage, ColorScheme, DynamicTypeSize, String)] = [
            ("01 Asset light", .asset, .light, .large, "0.25"),
            ("02 Recipient light", .recipient, .light, .large, "0.25"),
            ("03 Amount light", .amount, .light, .large, "0.25"),
            ("04 Pre-build review dark", .review, .dark, .large, "0.25"),
            ("05 Recipient accessibility3 dark", .recipient, .dark, .accessibility3, "0.25"),
            ("06 Long amount accessibility3 dark", .amount, .dark, .accessibility3, "0.250000000000000001")
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

        for (name, page, appearance, typeSize, amount) in cases {
            state.sendFlow.amount = amount
            applyPreviewFixture(to: state)
            #expect(state.sendAmountIsValid, "The layout fixture must exercise a valid amount")
            let content = composerPage(page, state: state, resolution: resolution)
            let view = NavigationStack {
                ZStack {
                    SpectraBackdrop().ignoresSafeArea()
                    ScrollView {
                        content.spectraScreenPadding()
                    }
                }
                .navigationTitle(AppLocalization.string("Send"))
                .navigationBarTitleDisplayMode(.inline)
                .toolbarBackground(.hidden, for: .navigationBar)
            }
            .environment(\.colorScheme, appearance)
            .environment(\.dynamicTypeSize, typeSize)
            let controller = UIHostingController(rootView: view)
            controller.overrideUserInterfaceStyle = appearance == .dark ? .dark : .light
            window.rootViewController = controller
            window.makeKeyAndVisible()
            try await Task.sleep(for: .milliseconds(350))
            window.layoutIfNeeded()
            // Flush the first native render, then give the local core holder
            // lookup and its SwiftUI update a separate layout pass.
            _ = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                _ = window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
            }
            try await Task.sleep(for: .milliseconds(750))
            window.layoutIfNeeded()
            try recordRender(window: window, name: name)

            if typeSize.isAccessibilitySize {
                let scrollView = try #require(scrollViews(in: controller.view)
                    .filter { $0.contentSize.height > $0.bounds.height }
                    .max { $0.bounds.height < $1.bounds.height })
                let bottom = max(0, scrollView.contentSize.height - scrollView.bounds.height + scrollView.adjustedContentInset.bottom)
                scrollView.setContentOffset(CGPoint(x: scrollView.contentOffset.x, y: bottom), animated: false)
                try await Task.sleep(for: .milliseconds(200))
                window.layoutIfNeeded()
                try recordRender(window: window, name: "\(name) bottom")
            }

            #expect(state.sendFlow.amount == amount, "Rendering must leave the entered precision intact")
            #expect(state.sendFlow.address == recipient, "Rendering must leave the full recipient intact")
            #expect(state.sendFlow.session.artifact == nil, "Rendering must not build or sign a transaction")
            #expect(state.sendFlow.session.selectedEndpoints.isEmpty, "Rendering must not choose broadcast destinations")
        }
    }

    private func recordRender(window: UIWindow, name: String) throws {
        let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
            #expect(window.drawHierarchy(in: window.bounds, afterScreenUpdates: true))
        }
        let pixels = try #require(image.cgImage?.dataProvider?.data) as Data
        #expect(Set(pixels).count > 16, "The attachment must contain rendered content")
        Attachment.record(image, named: name)
    }

    private func scrollViews(in view: UIView) -> [UIScrollView] {
        let own = (view as? UIScrollView).map { [$0] } ?? []
        return own + view.subviews.flatMap { scrollViews(in: $0) }
    }

    private enum ComposerPage {
        case asset, recipient, amount, review
    }

    private func composerPage(_ page: ComposerPage, state: AppState, resolution: SendDestinationResolution) -> AnyView {
        switch page {
        case .asset:
            AnyView(SendFromPage(store: state))
        case .recipient:
            AnyView(SendRecipientPage(
                store: state, isShowingQRScanner: .constant(false), qrScannerErrorMessage: .constant(nil),
                validationError: nil, isValidating: false, validatedResolution: resolution, retryValidation: {}))
        case .amount:
            AnyView(SendAmountPage(store: state, quoteIsCurrent: true))
        case .review:
            AnyView(SendConfirmationStep(store: state, quoteIsCurrent: true, recipientAddress: recipient))
        }
    }

    private func makeComposerFixture() async throws -> AppState {
        let state = makeState()
        state.isNetworkReachable = false
        let holdings: [AssetHolding] = [
            .fixture(name: "Ethereum", symbol: "ETH", coingeckoId: "ethereum", chainId: .ethereum, amount: "1.5"),
            .fixture(name: "USD Coin", symbol: "USDC", coingeckoId: "usd-coin", chainId: .ethereum,
                tokenStandard: "ERC-20", contractAddress: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", amount: "2500"),
            .fixture(name: "Tether", symbol: "USDT", coingeckoId: "tether", chainId: .ethereum,
                tokenStandard: "ERC-20", contractAddress: "0xdac17f958d2ee523a2206206994597c13d831ec7", amount: "1000")
        ]
        // The capability flag is render-fixture metadata. No secret is stored
        // and this suite never calls signing, preview transport or broadcast.
        let wallet = WalletView(
            name: "Main Wallet", chainId: .ethereum,
            addresses: [.ethereum: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"], holdings: holdings,
            signing: .privateKey(passwordProtected: false))
        _ = try await bridge.ready().applyStateCommand(command: .mergeBuiltInTokens)
        _ = try await bridge.ready().applyStateCommand(command: .upsertWallet(wallet: wallet.walletState()))
        _ = try await bridge.ready().applyStateCommand(command: .addAddressBookEntry(
            name: "Alex", chainId: .ethereum, address: recipient, note: ""))
        let snapshot = try await bridge.ready().portfolioSnapshot()
        state.applyPortfolioSnapshot(snapshot)
        let coin = try #require(state.availableSendCoins(for: wallet.id).first { $0.symbol == "ETH" })
        #expect(state.sendEnabledWallets.contains { $0.id == wallet.id })
        #expect(coin.amount == "1.5")
        state.sendFlow.walletId = wallet.id
        state.sendFlow.holdingKey = coin.holdingKey
        state.sendFlow.address = recipient
        state.sendFlow.amount = "0.25"
        return state
    }

    private func applyPreviewFixture(to state: AppState) {
        let preview = EvmSendPreview(
            nonce: 12, gasLimit: 21_000, maxFeePerGasGwei: "20", maxPriorityFeePerGasGwei: "1",
            estimatedNetworkFee: "0.00042", spendableBalance: "1.5", feeRateDescription: nil,
            maxSendable: "1.49958")
        state.sendFlow.previewStore.apply(OwnedSendPreview(
            walletId: state.sendFlow.walletId, holdingKey: state.sendFlow.holdingKey, chainId: .ethereum,
            amount: state.sendFlow.amountInput, preview: .ethereum(preview: preview), networkFee: "0.00042",
            networkFeeValue: 1.26, amountValue: 750, details: SendPreviewDetails(
                spendableBalance: "1.5", feeRateDescription: nil, estimatedTransactionBytes: nil,
                selectedInputCount: nil, usesChangeOutput: nil, maxSendable: "1.49958"),
            shortcuts: [25: "0.374895", 50: "0.74979", 75: "1.124685", 100: "1.49958"],
            recipient: RecipientCheck(activity: nil, isOwnAddress: false)))
    }
}
