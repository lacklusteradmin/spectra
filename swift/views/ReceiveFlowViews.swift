import Foundation
import SwiftUI
import UIKit

/// Receive starts on the wallet list; choosing a wallet pushes its address,
/// so the navigation bar's back button and swipe return to the list.
struct ReceiveView: View {
    @Bindable var store: AppState
    @State private var isShowingAddress = false

    var body: some View {
        // One wallet leaves nothing to choose: `beginReceive` selected it.
        if store.receiveEnabledWallets.count == 1 {
            ReceiveAddressView(store: store)
        } else {
            walletList
                .navigationDestination(isPresented: $isShowingAddress) {
                    ReceiveAddressView(store: store)
                }
        }
    }

    private var walletList: some View {
        ReceiveScreen(store: store, title: "Receive") {
            if store.receiveEnabledWallets.isEmpty {
                SpectraEmptyStateCard(
                    title: "No receive wallets",
                    message: "Import a wallet to generate receive addresses.",
                    systemImage: "wallet.pass"
                )
            } else {
                SpectraRowGroup(data: store.receiveEnabledWallets) { wallet in
                    WalletReceiveRow(
                        wallet: wallet,
                        isSelected: wallet.id == store.receiveFlow.walletId
                    ) {
                        select(wallet)
                    }
                }
            }
        }
    }

    private func select(_ wallet: WalletView) {
        spectraHaptic(.light)
        if store.receiveFlow.walletId != wallet.id {
            store.receiveFlow.walletId = wallet.id
            store.syncReceiveAssetSelection()
        }
        isShowingAddress = true
    }
}

private struct ReceiveAddressView: View {
    @Bindable var store: AppState
    @State private var didCopy: Bool = false
    @State private var isShowingShareSheet: Bool = false
    @State private var qrExportMessage: String?
    @State private var qrImageSaver: PhotoLibraryImageSaver?

    private var selectedWallet: WalletView? {
        store.receiveEnabledWallets.first(where: { $0.id == store.receiveFlow.walletId })
    }

    private var selectedCoin: AssetHolding? {
        store.selectedReceiveCoin(for: store.receiveFlow.walletId)
    }

    private var resolvedAddress: String {
        store.receiveFlow.resolvedAddress
    }

    private var canUseResolvedAddress: Bool {
        !resolvedAddress.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !store.receiveFlow.isResolving
    }

    /// The QR as an image, for sharing and saving. `nil` until an address
    /// resolves, which is what disables both buttons.
    private var qrImage: UIImage? {
        let address = resolvedAddress.trimmingCharacters(in: .whitespacesAndNewlines)
        return canUseResolvedAddress ? QRCodeRenderer.makeImage(from: address) : nil
    }

    var body: some View {
        ReceiveScreen(store: store, title: "Receive") {
            receiveAddressHero
            receiveActionCard
        }
        .sheet(isPresented: $isShowingShareSheet) {
            if let qrImage { ActivityItemSheet(activityItems: [qrImage]) }
        }
        .alert(
            AppLocalization.string("QR Code Export"),
            isPresented: .isPresent($qrExportMessage)
        ) {
            Button(AppLocalization.string("OK"), role: .cancel) { qrExportMessage = nil }
        } message: {
            if let qrExportMessage { Text(verbatim: qrExportMessage) }
        }
        .task(id: "\(store.receiveFlow.walletId)|\(store.receiveFlow.holdingKey)") {
            await store.refreshReceiveAddress()
        }
    }

    private var receiveAddressHero: some View {
        let wallet = selectedWallet
        let coin = selectedCoin
        return VStack(spacing: SpectraLayout.Space.m) {
            // The network is named here and on the wallet line under the
            // code; a third mark above this sentence said it again.
            if let coin {
                Text(AppLocalization.format("Receive only %@ assets on this network. Check the sender's network before transferring.", coin.chainName))
                    .font(.subheadline)
                    .multilineTextAlignment(.center)
            }
            if canUseResolvedAddress {
                QRCodeImage(address: resolvedAddress)
                    .frame(width: 184, height: 184)
                    .padding(SpectraLayout.Space.l)
                    .background(Color.white, in: RoundedRectangle(cornerRadius: SpectraLayout.Radius.card, style: .continuous))
            } else {
                receiveQRCodePlaceholder(size: 216)
            }

            // The address takes any asset on the chain, so the line names the
            // chain and draws its mark, not the gas token's.
            HStack(spacing: SpectraLayout.Space.m) {
                if let coin {
                    let badge = AssetHolding.nativeChainBadge(for: coin.chain) ?? (nil, coin.color)
                    CoinBadge(
                        artworkName: badge.artworkName,
                        fallbackText: coin.chainName,
                        color: badge.color,
                        size: 36
                    )
                }
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(wallet?.name ?? AppLocalization.string("Wallet"))
                        .font(.headline)
                    Text(coin?.chainName ?? AppLocalization.string("Select a chain"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            Text(canUseResolvedAddress ? resolvedAddress : (store.receiveFlow.error ?? AppLocalization.string("Loading receive address…")))
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .multilineTextAlignment(.center)
                .textSelection(.enabled)
        }
        .frame(maxWidth: .infinity)
        .padding(SpectraLayout.Space.l)
        .spectraElevatedFill()
    }

    private var receiveActionCard: some View {
        VStack(spacing: SpectraLayout.Space.s) {
            Button {
                guard canUseResolvedAddress else { return }
                UIPasteboard.general.string = resolvedAddress
                didCopy = true
                spectraHaptic(.light)
                Task {
                    try? await Task.sleep(for: .seconds(1.5))
                    didCopy = false
                }
            } label: {
                Label(
                    AppLocalization.string(didCopy ? "Copied" : "Copy Address"),
                    systemImage: didCopy ? "checkmark" : "doc.on.doc"
                )
                .font(.headline)
                .frame(maxWidth: .infinity)
                .frame(minHeight: 46)
            }
            .buttonStyle(.glassProminent)
            .disabled(!canUseResolvedAddress)

            Button {
                guard qrImage != nil else { return }
                isShowingShareSheet = true
            } label: {
                Label(AppLocalization.string("Share QR Code"), systemImage: "square.and.arrow.up")
                    .font(.subheadline.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, SpectraLayout.Space.s)
            }
            .buttonStyle(.glass)
            .disabled(qrImage == nil)

            Button {
                guard let qrImage else { return }
                let saver = PhotoLibraryImageSaver { result in
                    switch result {
                    case .success: qrExportMessage = AppLocalization.string("QR code saved to Photos.")
                    case .failure(let error): qrExportMessage = userErrorMessage(error)
                    }
                    qrImageSaver = nil
                }
                qrImageSaver = saver
                saver.save(qrImage)
            } label: {
                Label(AppLocalization.string("Save QR Code"), systemImage: "square.and.arrow.down")
                    .font(.subheadline.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, SpectraLayout.Space.s)
            }
            .buttonStyle(.glass)
            .disabled(qrImage == nil)
        }
        .padding(SpectraLayout.Space.l)
        .frame(maxWidth: .infinity)
        .spectraCardFill()
    }

    /// Choosing a wallet is the step forward; there is no separate Continue.
}

/// The backdrop, scrolling column and close button both receive pages share.
private struct ReceiveScreen<Content: View>: View {
    let store: AppState
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        ZStack {
            SpectraBackdrop().ignoresSafeArea()

            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) { content }
                    .spectraScreenPadding()
            }
        }
        .navigationTitle(AppLocalization.string(title))
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    store.cancelReceive()
                } label: {
                    Image(systemName: "xmark")
                }
                .accessibilityLabel(AppLocalization.string("Close"))
            }
        }
    }
}

/// Open a wallet's receive address. Addresses come from core on the next
/// step, where UTXO addresses are reserved and registered as owned.
private struct WalletReceiveRow: View {
    let wallet: WalletView
    let isSelected: Bool
    let onSelect: () -> Void

    var body: some View {
        let badge = AssetHolding.nativeChainBadge(for: wallet.family) ?? (nil, Color.mint)

        Button(action: onSelect) {
            HStack(spacing: SpectraLayout.Space.m) {
                CoinBadge(
                    artworkName: badge.artworkName,
                    fallbackText: wallet.familyName,
                    color: badge.color,
                    size: 36
                )

                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(wallet.name)
                        .font(.headline)
                        .foregroundStyle(Color.primary)
                        .lineLimit(1)
                    // The network, not the family: a Sepolia wallet read
                    // "Ethereum" and looked like a mainnet one.
                    Text(wallet.networkTitle)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                Spacer(minLength: 0)

                if isSelected {
                    Image(systemName: "checkmark")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.tint)
                }
                Image(systemName: "chevron.right")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
            .spectraRowPadding()
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
    }
}

/// `SpectraLoadingGlyph` and `SpectraShimmer` carry `@State`, so their
/// memberwise initializers are main-actor isolated; a nonisolated free
/// function cannot call them.
@MainActor private func receiveQRCodePlaceholder(size: CGFloat) -> some View {
    ZStack {
        RoundedRectangle(cornerRadius: SpectraLayout.Radius.card, style: .continuous)
            .fill(Color.white.opacity(0.82))
        VStack(spacing: SpectraLayout.Space.m) {
            SpectraLoadingGlyph(size: 42, tint: .accentColor)
            VStack(spacing: SpectraLayout.Space.s) {
                SpectraShimmer(height: 14)
                    .frame(width: size * 0.58)
                SpectraShimmer(height: 14)
                    .frame(width: size * 0.42)
            }
        }
    }
    .frame(width: size, height: size)
}
