import SwiftUI

/// Adds a wallet's key, or watched address, to another network as a wallet
/// of its own. Core lists the networks it can go to, reads the secret and
/// seals the copy; this page chooses the network, shows the address the copy
/// will hold and confirms.
struct WalletCopyView: View {
    let store: AppState
    let wallet: WalletView
    @Environment(\.scenePhase) private var scenePhase
    @State private var targets: [ChainSelectionDescriptor]?
    @State private var chain: Chain?
    @State private var name: String
    @State private var password = ""
    @State private var preview: WalletImportPreview?
    @State private var error: String?
    @State private var isWorking = false
    /// The new wallet's name once core has stored it.
    @State private var added: String?

    init(store: AppState, wallet: WalletView) {
        self.store = store
        self.wallet = wallet
        _name = State(initialValue: wallet.name)
    }

    private var holdsKey: Bool { !wallet.signing.isWatchOnly }
    private var needsPassword: Bool { wallet.signing.requiresPassword }

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                Text(AppLocalization.string(
                    holdsKey
                        ? "The new wallet seals its own copy of this wallet's key, under the same password. Deleting either leaves the other."
                        : "The new wallet watches this address on the network you choose."
                )).font(.subheadline).foregroundStyle(.secondary)
                if let added {
                    Label(AppLocalization.format("Added “%@”.", added), systemImage: "checkmark.circle.fill")
                        .font(.headline).foregroundStyle(.tint)
                        .padding(SpectraLayout.cardPadding).frame(maxWidth: .infinity, alignment: .leading)
                        .spectraCardFill()
                } else if let targets {
                    if let chain { form(chain) }
                    SpectraRowGroup(title: AppLocalization.string("Network"), data: targets) { descriptor in
                        ChainSelectionRow(descriptor: descriptor, isSelected: chain == descriptor.id) {
                            select(descriptor.id)
                        }
                    }.spectraCardFill()
                } else {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }.spectraScreenPadding()
        }
        .scrollDismissesKeyboard(.interactively)
        .background(SpectraBackdrop().ignoresSafeArea())
        .navigationTitle(WalletAction.addToNetwork.title).navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .task { await loadTargets() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .background { password = "" }
        }
        .onDisappear { password = "" }
    }

    @ViewBuilder
    private func form(_ chain: Chain) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(chain.displayName).font(.headline)
            TextField(AppLocalization.string("import_flow.wallet_name_placeholder"), text: $name)
                .padding(SpectraLayout.Space.m).spectraInputFieldStyle()
            if needsPassword {
                SecureField(AppLocalization.string("Wallet Password"), text: $password)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().privacySensitive()
                    .padding(SpectraLayout.Space.m).spectraInputFieldStyle()
            }
            if let preview {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                    Text(AppLocalization.string(preview.addresses.count == 1 ? "Address" : "Addresses"))
                        .font(.subheadline.weight(.semibold))
                    ForEach(preview.addresses, id: \.self) { address in
                        Text(verbatim: breakableAnywhere(address)).font(.footnote.monospaced())
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if let upgrades = preview.upgradesWallet {
                        Label(
                            AppLocalization.format("This adds keys to the watched wallet “%@”, keeping its history.", upgrades),
                            systemImage: "key.fill"
                        ).font(.caption.weight(.medium)).foregroundStyle(.tint)
                    }
                }
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle.fill").font(.footnote.weight(.medium))
                    .foregroundStyle(.red.opacity(0.92))
            }
            Button {
                Task { await (preview == nil ? showPreview(chain) : add(chain)) }
            } label: {
                HStack {
                    if isWorking { ProgressView() }
                    Text(AppLocalization.string(preview == nil ? "Check Address" : "Add Wallet"))
                        .font(.headline).frame(maxWidth: .infinity)
                }
            }
            .buttonStyle(.glassProminent)
            .disabled(isWorking || (needsPassword && password.isEmpty))
        }
        .padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading)
        .spectraBubbleFill().spectraCardFill()
    }

    private func select(_ target: Chain) {
        guard target != chain else { return }
        chain = target
        preview = nil
        error = nil
    }

    private func loadTargets() async {
        guard targets == nil else { return }
        do {
            let chains = try await store.bridge.ready().walletCopyTargets(walletId: wallet.id)
            targets = ChainSelectionDescriptor.popularOrder(chains)
        } catch {
            self.error = userErrorMessage(error)
            targets = []
        }
    }

    private func commit(_ chain: Chain) -> WalletCopyCommit {
        WalletCopyCommit(
            sourceWalletId: wallet.id, chain: chain,
            walletName: name.trimmingCharacters(in: .whitespacesAndNewlines),
            password: needsPassword ? password : nil,
            derivationPath: nil, restoreHeight: nil, tonWalletVersion: nil)
    }

    /// Reading the key needs the device owner; the address it gives is shown
    /// before anything is stored.
    private func showPreview(_ chain: Chain) async {
        isWorking = true
        defer { isWorking = false }
        error = nil
        if holdsKey, let failure = await store.authenticate(
            .secretMaterial, reason: AppLocalization.format("Authenticate to add %@ to another network", wallet.name)) {
            error = failure
            return
        }
        do {
            let result = try await store.bridge.ready().previewWalletCopy(commit: commit(chain))
            guard self.chain == chain else { return }
            preview = result
        } catch {
            self.error = userErrorMessage(error)
        }
    }

    private func add(_ chain: Chain) async {
        isWorking = true
        defer { isWorking = false }
        error = nil
        do {
            let outcome = try await store.copyWallet(commit(chain))
            password = ""
            added = outcome.wallets.first?.name ?? name
            spectraNotificationHaptic(.success)
        } catch {
            self.error = userErrorMessage(error)
            spectraNotificationHaptic(.error)
        }
    }
}
