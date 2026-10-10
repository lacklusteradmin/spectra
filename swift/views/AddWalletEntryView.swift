import SwiftUI

/// Add Wallet: choose the network first. Every page after it — the ways of
/// adding a wallet, the formats they accept, the options they offer — is that
/// network's own: a wallet is on one network, and Add to Another Network
/// takes its phrase to the next. Funds Finder sits under the list, for a
/// phrase whose network is unknown.
struct AddWalletEntryView: View {
    let store: AppState
    private static let descriptors = ChainSelectionDescriptor.popularOrder(Chain.all)
    @State private var searchText = ""
    @State private var chosenChain: Chain?
    @State private var isShowingFundsFinder = false
    var body: some View {
        AllChainsSelectionView(
            chainSearchText: $searchText, title: AppLocalization.string("Add Wallet"), descriptors: Self.descriptors,
            selectedChains: [], accessory: .disclosure, toggleSelection: { chosenChain = $0 }
        ) {
            fundsFinderRow
        }
        .navigationDestination(item: $chosenChain) { chain in
            WalletSetupMethodsView(store: store, chain: chain)
        }
        .navigationDestination(isPresented: $isShowingFundsFinder) {
            FundsFinderView(store: store)
        }
    }
    private var fundsFinderRow: some View {
        Button {
            isShowingFundsFinder = true
        } label: {
            HStack(spacing: SpectraLayout.Space.m) {
                Image(systemName: "magnifyingglass.circle.fill").font(.system(size: 28, weight: .semibold))
                    .foregroundStyle(.tint).frame(width: 36, height: 36)
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(AppLocalization.string("Find Lost Funds")).font(.headline).foregroundStyle(Color.primary)
                    Text(AppLocalization.string("Check every network's derivation profiles for accounts a phrase has used."))
                        .font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.leading)
                }
                Spacer(minLength: SpectraLayout.Space.s)
                Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
            }
            .spectraRowPadding()
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .spectraCardFill()
    }
}

/// One network's ways of adding a wallet, as core's setup descriptor lists
/// them, each with the formats it accepts there. Opened from a watched
/// wallet's page, the ways of giving that wallet its keys instead, as core
/// offers them for it.
struct WalletSetupMethodsView: View {
    let store: AppState
    let chain: Chain
    /// The watched wallet the methods give their keys to, and what doing so
    /// means, when the page was opened from that wallet's page.
    var upgrading: (wallet: WalletView, note: String)? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var upgradeMethods: [WalletSetupMethod] = []
    private var descriptor: WalletSetupDescriptor { walletSetupDescriptor(chain: chain) }
    private var options: [WalletSetupOption] {
        guard upgrading != nil else { return descriptor.options }
        return descriptor.options.filter { upgradeMethods.contains($0.method) }
    }
    private var upgradedIsWatchOnly: Bool? {
        upgrading.flatMap { store.wallet(for: $0.wallet.id)?.signing.isWatchOnly }
    }
    var body: some View {
        ZStack {
            SpectraBackdrop().ignoresSafeArea()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.sectionSpacing) {
                    networkHeader
                    if let upgrading {
                        Text(AppLocalization.string(upgrading.note)).font(.subheadline).foregroundStyle(.secondary)
                    }
                    SpectraRowGroup(data: options) { option in
                        Button { begin(option.method) } label: {
                            methodRow(option)
                        }.buttonStyle(.plain)
                    }
                }.spectraScreenPadding()
            }
        }
        .task(id: upgrading?.wallet.id) {
            guard let id = upgrading?.wallet.id else { return }
            upgradeMethods = (try? await store.bridge.ready().walletUpgradeMethods(walletId: id)) ?? []
        }
        // Once the wallet has its keys there is nothing left to give it.
        .onChange(of: upgradedIsWatchOnly) { _, isWatchOnly in
            if isWatchOnly == false { dismiss() }
        }
        // The header names the network; the bar would say it a second time.
        .navigationTitle(upgrading == nil ? "" : WalletAction.addKeys.title)
        .navigationBarTitleDisplayMode(.inline)
        .navigationDestination(
            isPresented: Binding(
                get: { store.walletImport.isPresented && store.walletImport.editingWalletId == nil },
                set: { isPresented in
                    if !isPresented { store.walletImport.isPresented = false }
                }
            )
        ) {
            SetupView(store: store, draft: store.walletImport.draft)
        }
    }
    private func begin(_ method: WalletSetupMethod) {
        if let upgrading {
            store.beginWalletUpgrade(walletId: upgrading.wallet.id, chain: chain, method: method)
        } else {
            store.beginWalletSetup(chain: chain, method: method)
        }
    }
    @ViewBuilder
    private var networkHeader: some View {
        if let entry = chain.entry {
            let row = ChainSelectionDescriptor(chain: chain, entry: entry)
            HStack(spacing: SpectraLayout.Space.m) {
                CoinBadge(artworkName: row.artworkName, fallbackText: row.symbol, color: row.color, size: 48)
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(row.title).font(.title2.weight(.bold)).foregroundStyle(Color.primary)
                    Text(row.tagLine).font(.subheadline).foregroundStyle(.secondary)
                }
            }
        }
    }
    private func methodRow(_ option: WalletSetupOption) -> some View {
        HStack(spacing: SpectraLayout.Space.m) {
            Image(systemName: option.method.icon).font(.system(size: 28, weight: .semibold)).foregroundStyle(.tint)
                .frame(width: 36, height: 36)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(option.method.title).font(.headline).foregroundStyle(Color.primary)
                Text(option.method.subtitle).font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.leading)
                Text(option.formats.map(\.title).joined(separator: " · ")).font(.caption2.weight(.medium))
                    .foregroundStyle(.tertiary).multilineTextAlignment(.leading)
            }
            Spacer(minLength: SpectraLayout.Space.s)
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }.spectraRowPadding()
    }
}

extension WalletSetupOption: Identifiable {
    public var id: WalletSetupMethod { method }
}

extension WalletSetupMethod {
    var title: String {
        switch self {
        case .createPhrase: AppLocalization.string("Create New Wallet")
        case .importPhrase: AppLocalization.string("Import Seed Phrase")
        case .importPrivateKey: AppLocalization.string("Import Private Key")
        case .watchAddresses: AppLocalization.string("Watch Addresses")
        case .watchAccountXpub: AppLocalization.string("Watch Account")
        case .watchViewKey: AppLocalization.string("Watch with View Key")
        case .watchMultisig: AppLocalization.string("Watch Multisig")
        }
    }
    var subtitle: String {
        switch self {
        case .createPhrase: AppLocalization.string("Generate a new seed phrase and set up your wallet.")
        case .importPhrase: AppLocalization.string("Restore a wallet from its recovery phrase.")
        case .importPrivateKey: AppLocalization.string("Add one account from its private key.")
        case .watchAddresses: AppLocalization.string("Track public addresses without adding private keys.")
        case .watchAccountXpub: AppLocalization.string("Track a whole account from its extended public key.")
        case .watchViewKey: AppLocalization.string("Scan what a wallet receives from its address and private view key.")
        case .watchMultisig: AppLocalization.string("Watch a multisig account from the policy its address derives from.")
        }
    }
    var icon: String {
        switch self {
        case .createPhrase: "plus.circle.fill"
        case .importPhrase: "arrow.down.circle.fill"
        case .importPrivateKey: "key.circle.fill"
        case .watchAddresses: "eye.circle.fill"
        case .watchAccountXpub: "binoculars.circle.fill"
        case .watchViewKey: "eye.square.fill"
        case .watchMultisig: "person.2.circle.fill"
        }
    }
}

extension WalletSecretFormat {
    var title: String {
        switch self {
        case .bip39Phrase: AppLocalization.string("BIP-39 phrase, 12 to 24 words")
        case .moneroPhrase: AppLocalization.string("Monero seed, 25 words")
        case .polyseed: AppLocalization.string("Polyseed, 16 words")
        case .tonMnemonic: AppLocalization.string("TON mnemonic, 24 words")
        case .hexSecret32: AppLocalization.string("32-byte key in hex")
        case .cardanoExtendedKey: AppLocalization.string("64-byte extended key in hex")
        case .wif: AppLocalization.string("WIF")
        case .solanaKeypair: AppLocalization.string("Base58 or JSON keypair")
        case .stellarSecretSeed: AppLocalization.string("Secret seed (S…)")
        case .suiPrivateKey: AppLocalization.string("suiprivkey1…")
        case .aptosPrivateKey: AppLocalization.string("AIP-80 key (ed25519-priv-0x…)")
        case .nearSecretKey: AppLocalization.string("Key string (ed25519:…)")
        case .address: AppLocalization.string("Addresses, one per line")
        case .accountXpub: AppLocalization.string("Account public key")
        case .moneroViewKey: AppLocalization.string("Private view key, 64 hex digits")
        case .multisigDescriptor: AppLocalization.string("Output descriptor, sortedmulti(…)")
        case .suiMultisigPublicKey: AppLocalization.string("Threshold and weighted public keys, as Sui's SDK takes them")
        case .aptosMultiKey: AppLocalization.string("Signatures required and public keys, as Aptos's SDK takes them")
        case .cardanoNativeScript: AppLocalization.string("Native script, as cardano-cli writes it")
        case .substrateMultisig: AppLocalization.string("Threshold and signatory addresses")
        }
    }
}
