import SwiftUI

/// The wallet's name, recovery phrase, identifiers and deletion.
///
/// Revealing the phrase and deleting the wallet are rare, and one is sensitive
/// and the other irreversible, so each is a row a level down, and the details
/// page is only what the wallet holds.
struct WalletAdvancedDetailsView: View {
    let store: AppState
    let wallet: WalletView
    /// What core offers for managing the wallet, in its order.
    let manageOffers: [WalletActionOffer]
    let derivationPathsText: String?
    let firstActivityDateText: String
    @Environment(\.scenePhase) private var scenePhase
    @State private var seedReveal = SeedPhraseRevealState()
    @State private var isShowingDeleteWalletAlert: Bool = false
    private var displayedWallet: WalletView {
        store.wallet(for: wallet.id) ?? wallet
    }
    private func offers(_ action: WalletAction) -> Bool { manageOffers.contains { $0.action == action } }
    private func note(_ action: WalletAction) -> String {
        AppLocalization.string(manageOffers.first { $0.action == action }?.note ?? "")
    }
    private var isWatchOnly: Bool { displayedWallet.signing.isWatchOnly }
    private var isPrivateKeyWallet: Bool { displayedWallet.signing.isPrivateKey }
    private var requiresSeedPhrasePassword: Bool { displayedWallet.signing.requiresPassword }
    private var deleteWalletMessage: String {
        if isWatchOnly {
            return AppLocalization.string("You can't recover this wallet after deletion unless you still have this address.")
        }
        if isPrivateKeyWallet {
            return AppLocalization.string("Please keep this private key because you can't recover this wallet after deletion.")
        }
        return AppLocalization.string("Please take note of your seed phrase because you can't recover this wallet after deletion.")
    }
    var body: some View {
        Form {
            if offers(.rename) {
                Section {
                    Button {
                        spectraHaptic(.light)
                        store.beginEditingWallet(displayedWallet)
                    } label: {
                        HStack(spacing: SpectraLayout.Space.s) {
                            Text(AppLocalization.string("Name")).foregroundStyle(Color.primary)
                            Spacer(minLength: SpectraLayout.Space.s)
                            // Concrete label colours: a button's hierarchical
                            // styles derive from its tint, which drew the name orange.
                            Text(displayedWallet.name).foregroundStyle(Color(.secondaryLabel)).lineLimit(1)
                            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(Color(.tertiaryLabel))
                        }
                    }
                }
            }
            if offers(.revealPhrase) {
                Section(AppLocalization.string("Security")) {
                    Button {
                        spectraHaptic(.medium)
                        if requiresSeedPhrasePassword {
                            seedReveal.passwordInput = ""
                            seedReveal.isShowingPasswordPrompt = true
                        } else {
                            Task {
                                await revealSeedPhrase()
                            }
                        }
                    } label: {
                        Label(
                            seedReveal.isRevealing
                                ? AppLocalization.format("Checking %@…", DeviceBiometry.current.name)
                                : (requiresSeedPhrasePassword
                                    ? AppLocalization.string("Show Seed Phrase (Password)")
                                    : AppLocalization.string("Show Seed Phrase")),
                            systemImage: requiresSeedPhrasePassword ? "lock.shield" : DeviceBiometry.current.symbol
                        )
                    }.disabled(seedReveal.isRevealing || !displayedWallet.signing.hasSeedPhrase)
                }
            }
            if offers(.exportKeys) {
                Section {
                    NavigationLink {
                        WalletKeyExportView(store: store, wallet: displayedWallet)
                    } label: {
                        Label(WalletAction.exportKeys.title, systemImage: WalletAction.exportKeys.systemImage)
                    }
                } footer: {
                    Text(note(.exportKeys))
                }
            }
            if offers(.addToNetwork) {
                Section {
                    NavigationLink {
                        WalletCopyView(store: store, wallet: displayedWallet)
                    } label: {
                        Label(WalletAction.addToNetwork.title, systemImage: WalletAction.addToNetwork.systemImage)
                    }
                } footer: {
                    Text(note(.addToNetwork))
                }
            }
            Section(AppLocalization.string("Details")) {
                WalletDetailRow(label: "Wallet ID", value: wallet.id)
                if let derivationPathsText { WalletDetailRow(label: "Derivation Path", value: derivationPathsText) }
                WalletDetailRow(label: "First Activity", value: firstActivityDateText)
            }
            if offers(.delete) {
                Section {
                    Button(role: .destructive) {
                        spectraHaptic(.medium)
                        isShowingDeleteWalletAlert = true
                    } label: {
                        Label(AppLocalization.string("Delete Wallet"), systemImage: "trash").foregroundStyle(.red)
                    }
                }
            }
        }.navigationTitle(AppLocalization.string("Manage Wallet")).navigationBarTitleDisplayMode(.inline)
        .navigationDestination(
            isPresented: Binding(
                get: { store.walletImport.isPresented && store.walletImport.editingWalletId == wallet.id },
                set: { isPresented in
                    if !isPresented { store.walletImport.isPresented = false }
                }
            )
        ) {
            SetupView(store: store, draft: store.walletImport.draft)
        }.alert(AppLocalization.string("Delete Wallet?"), isPresented: $isShowingDeleteWalletAlert) {
            Button(AppLocalization.string("Delete"), role: .destructive) {
                Task { await store.deleteWallet(wallet) }
            }
            Button(AppLocalization.string("Cancel"), role: .cancel) {
                isShowingDeleteWalletAlert = false
            }
        } message: {
            Text(deleteWalletMessage)
        }.alert(
            AppLocalization.string("Cannot Reveal Seed Phrase"),
            isPresented: .isPresent($seedReveal.errorMessage)
        ) {
            Button(AppLocalization.string("OK"), role: .cancel) {}
        } message: {
            Text(seedReveal.errorMessage ?? AppLocalization.string("Something went wrong. Try again."))
        }.onChange(of: (store.wallet(for: wallet.id) != nil)) { _, walletStillExists in
            if !walletStillExists {
                isShowingDeleteWalletAlert = false
                seedReveal.invalidate()
            }
        }.onChange(of: scenePhase) { _, newPhase in
            seedReveal.setSceneIsActive(newPhase == .active)
            // Native authentication may briefly make the scene inactive.
            // Leaving the app invalidates the request as well as its presentation.
            if newPhase == .background { seedReveal.invalidate() }
        }
        .onAppear { seedReveal.activate(sceneIsActive: scenePhase == .active) }
        .onDisappear { seedReveal.deactivate() }
        .sheet(
            isPresented: $seedReveal.isShowingPasswordPrompt,
            onDismiss: {
                seedReveal.passwordInput = ""
            }
        ) {
            NavigationStack {
                ZStack {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                        Text(
                            AppLocalization.format("seedReveal.password_hint_format", DeviceBiometry.current.name)
                        ).font(.subheadline).foregroundStyle(.secondary)
                        SecureField(AppLocalization.string("Wallet Password"), text: $seedReveal.passwordInput)
                            .textInputAutocapitalization(.never).autocorrectionDisabled().privacySensitive().padding(SpectraLayout.Space.m)
                            .spectraInputFieldStyle().foregroundStyle(Color.primary)
                        Button {
                            spectraHaptic(.medium)
                            let password = seedReveal.passwordInput
                            seedReveal.isShowingPasswordPrompt = false
                            seedReveal.passwordInput = ""
                            Task {
                                await revealSeedPhrase(password: password)
                            }
                        } label: {
                            Text(AppLocalization.string("Reveal Seed Phrase")).font(.headline).frame(maxWidth: .infinity)
                        }.buttonStyle(.glassProminent).disabled(
                            seedReveal.passwordInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        Spacer()
                    }.padding(SpectraLayout.Space.l)
                }.navigationTitle(AppLocalization.string("Wallet Password")).navigationBarTitleDisplayMode(.inline).toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button(AppLocalization.string("Cancel")) {
                            seedReveal.isShowingPasswordPrompt = false
                        }
                    }
                }
            }
        }.sheet(
            isPresented: $seedReveal.isShowingPhraseSheet,
            onDismiss: {
                seedReveal.clearPhrase()
            }
        ) {
            NavigationStack {
                ZStack {
                    ScrollView(showsIndicators: false) {
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                            Text(
                                AppLocalization.string(
                                    "Write this down and keep it offline. Anyone with this phrase can control your funds.")
                            ).font(.subheadline).foregroundStyle(.secondary)
                            // Numbered as it was shown when created, so it is copied
                            // down in order. No Copy: Add to Another Network takes a
                            // phrase to another network without the clipboard.
                            SeedPhraseWordGrid(words: seedReveal.phrase.split(whereSeparator: \.isWhitespace).map(String.init))
                        }.secretShield().padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
                            .padding(SpectraLayout.Space.l)
                    }
                }.navigationTitle(AppLocalization.string("Seed Phrase")).navigationBarTitleDisplayMode(.inline).toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button(AppLocalization.string("Done")) {
                            seedReveal.isShowingPhraseSheet = false
                        }
                    }
                }
            }
        }
    }
    private func revealSeedPhrase(password: String? = nil) async {
        let result = await seedReveal.reveal(canPresent: {
            store.wallet(for: wallet.id) != nil
        }, operation: {
            try await store.revealSeedPhrase(for: wallet, password: password)
        })
        if let result { spectraNotificationHaptic(result ? .success : .error) }
    }
}
private struct WalletDetailRow: View {
    let label: String
    let value: String
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Text(AppLocalization.string(label)).font(.caption).foregroundStyle(.secondary)
            Text(value).font(.subheadline).foregroundStyle(Color.primary).textSelection(.enabled)
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}
