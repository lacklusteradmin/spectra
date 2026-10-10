import Foundation
import SwiftUI

/// One page of wallet setup. The network and the method were chosen before
/// the form opened; the form lives in the `@Bindable` draft, and the store
/// supplies app-wide state and performs the import.
struct SetupView: View {
    private let store: AppState
    @Bindable var draft: WalletImportDraft
    private let copy = ImportFlowContent.current
    /// The page this screen shows. Each page is its own pushed screen, so the
    /// system back button and the swipe step back one page at a time.
    private let setupPage: WalletSetupPage
    @State private var nextPage: WalletSetupPage?
    @State private var isEditingWatchedAddresses = false
    /// The name core gives a wallet whose name is left blank, shown where the
    /// name is typed.
    @State private var defaultWalletName: String?
    init(store: AppState, draft: WalletImportDraft, page: WalletSetupPage? = nil) {
        self.store = store
        self.draft = draft
        setupPage = page ?? draft.setupFlow.pages.first ?? .walletName
    }
    private var isEditingWallet: Bool { draft.isEditingWallet }
    private var pageCopy: WalletSetupPageCopy { setupPage.copy(copy, mode: draft.mode) }
    private var nextPageInFlow: WalletSetupPage? { draft.setupFlow.next(after: setupPage) }
    /// Whether this page's part of the form is complete enough to leave it.
    /// Pages before it were complete when they were left.
    private var isPageComplete: Bool {
        switch setupPage {
        case .seedPhrase: draft.isSecretComplete
        case .backupVerification: draft.isBackupVerificationComplete
        case .watchAddresses: store.canImportWallet
        case .walletName:
            store.canImportWallet && (!draft.mode.takesWalletPassword || draft.walletPasswordValidationError == nil)
        }
    }
    private var isPrimaryActionEnabled: Bool {
        guard !store.walletImport.isBusy, isPageComplete else { return false }
        return nextPageInFlow != nil || store.canImportWallet
    }
    /// "Next" while the flow has pages left, and what the submit does on the
    /// last one.
    private var primaryActionTitle: String {
        switch nextPageInFlow {
        case .backupVerification: return AppLocalization.string("import_flow.continue_to_backup_verification")
        case .some: return AppLocalization.string("import_flow.next")
        case nil: break
        }
        switch draft.mode {
        case .edit: return AppLocalization.string("import_flow.save_wallet")
        case .setup(.createPhrase): return AppLocalization.string("import_flow.create_wallet")
        case .setup(.watchAddresses), .setup(.watchAccountXpub), .setup(.watchViewKey), .setup(.watchMultisig):
            return AppLocalization.string("import_flow.watch_addresses")
        case .setup: return AppLocalization.string("import_flow.import_wallet")
        }
    }
    @ViewBuilder
    private func watchedAddressEditor(text: Binding<String>) -> some View {
        AddressInput(
            text: text, isFocused: $isEditingWatchedAddresses,
            prompt: AppLocalization.string("One address per line"), allowsNewlines: true)
            .frame(minHeight: 88, alignment: .topLeading)
            .padding(SpectraLayout.Space.m).spectraInputFieldStyle()
    }
    /// A flat card, not Liquid Glass: the setup screen stacks about ten of them.
    @ViewBuilder
    private func setupCard<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        content().padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
    }
    @ViewBuilder
    private var walletPasswordStepSection: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(AppLocalization.string("import_flow.wallet_password_optional")).font(.headline).foregroundStyle(Color.primary)
            Text(AppLocalization.string("import_flow.wallet_password_subtitle")).font(.subheadline).foregroundStyle(.secondary)
            SecureField(AppLocalization.string("import_flow.wallet_password_field"), text: $draft.walletPassword).textInputAutocapitalization(
                .never
            ).autocorrectionDisabled().padding(SpectraLayout.Space.m).spectraInputFieldStyle().foregroundStyle(Color.primary)
            SecureField(AppLocalization.string("import_flow.wallet_password_confirmation_field"), text: $draft.walletPasswordConfirmation)
                .textInputAutocapitalization(.never).autocorrectionDisabled().padding(SpectraLayout.Space.m).spectraInputFieldStyle().foregroundStyle(
                    Color.primary)
            if let walletPasswordValidationError = draft.walletPasswordValidationError {
                Text(walletPasswordValidationError).font(.caption).foregroundStyle(.red.opacity(0.9))
            } else if draft.walletPasswordInput?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false {
                Text(AppLocalization.string("import_flow.wallet_password_success")).font(.caption).foregroundStyle(.green.opacity(0.9))
            }
        }
    }
    @ViewBuilder
    private func watchedAddressSection(
        text: Binding<String>, caption: String? = nil, validationMessage: String? = nil, validationColor: Color? = nil
    ) -> some View {
        watchedAddressEditor(text: text)
        if let caption { Text(caption).font(.caption).foregroundStyle(.secondary) }
        if let validationMessage, !validationMessage.isEmpty {
            Text(validationMessage).font(.caption).foregroundStyle(validationColor ?? Color.secondary)
        }
    }
    private func watchedAddressValidationMessage(
        entries: [String], assetDisplayName: String, validator: (String) -> Bool
    ) -> (message: String, color: Color) {
        let localizedAssetName = assetDisplayName
        // Nothing typed yet: the page and the field already say what goes here.
        if entries.isEmpty { return ("", Color.secondary) }
        if !entries.allSatisfy(validator) {
            return (AppLocalization.format("Every line must contain a valid %@ address.", localizedAssetName), .red.opacity(0.9))
        }
        let count = entries.count
        return (
            AppLocalization.format("%lld valid %@ addresses ready to import.", count: count, count, localizedAssetName),
            .green.opacity(0.9)
        )
    }
    @ViewBuilder
    private var setupHeader: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            if !isEditingWallet, let chain = draft.chain, let entry = chain.entry {
                // The network this wallet is added on, chosen before the form,
                // and how far through the form this page is.
                HStack(spacing: SpectraLayout.Space.xs) {
                    CoinBadge(artworkName: entry.artworkName, fallbackText: entry.gasTokenSymbol, color: entry.color.color, size: 20)
                    Text(chain.displayName).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                    Spacer(minLength: SpectraLayout.Space.s)
                    stepIndicator
                }
            }
            Text(pageCopy.title).font(.largeTitle.weight(.bold)).foregroundStyle(Color.primary)
                .lineLimit(3).minimumScaleFactor(0.7).allowsTightening(true).fixedSize(horizontal: false, vertical: true)
            Text(pageCopy.subtitle).font(.subheadline).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
    /// "Step 2 of 3", with a segment per page: where this page sits in the flow.
    @ViewBuilder
    private var stepIndicator: some View {
        let pages = draft.setupFlow.pages
        if pages.count > 1, let index = draft.setupFlow.index(of: setupPage) {
            HStack(spacing: SpectraLayout.Space.xs) {
                ForEach(pages.indices, id: \.self) { position in
                    Capsule().fill(position <= index ? AnyShapeStyle(.tint) : AnyShapeStyle(SpectraLayout.insetFill))
                        .frame(width: 18, height: 4)
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(AppLocalization.format("Step %lld of %lld", index + 1, pages.count))
        }
    }
    /// Single rendering entry point for the page body: a switch over
    /// `setupPage` makes the page → content map structural.
    @ViewBuilder
    private var pageContent: some View {
        switch setupPage {
        case .watchAddresses:
            watchPageContent
        case .seedPhrase:
            WalletSecretStep(store: store, draft: draft, showsBackupVerification: false)
        case .backupVerification:
            WalletSecretStep(store: store, draft: draft, showsBackupVerification: true)
        case .walletName:
            walletNamePageContent
        }
    }
    /// A watch import's one field: addresses on the network, each judged by
    /// core's watch-only rule, or the account public key that stands in for
    /// the whole account.
    @ViewBuilder
    private var watchPageContent: some View {
        if let chain = draft.chain {
            setupCard {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    if draft.method == .watchAccountXpub {
                        Text(AppLocalization.string("Account Public Key")).font(.headline).foregroundStyle(Color.primary)
                        TextField(chain.accountKeyPrefixes.map { "\($0)…" }.joined(separator: " / "), text: $draft.accountXpubInput).textInputAutocapitalization(.never)
                            .autocorrectionDisabled().font(.system(.footnote, design: .monospaced))
                            .padding(SpectraLayout.Space.m).spectraInputFieldStyle().foregroundStyle(Color.primary)
                    } else if draft.method == .watchMultisig {
                        Text(AppLocalization.string("Multisig Policy")).font(.headline).foregroundStyle(Color.primary)
                        TextField(
                            walletSetupDescriptor(chain: chain).options.first { $0.method == .watchMultisig }?
                                .formats.first?.title ?? "",
                            text: $draft.descriptorInput, axis: .vertical
                        ).lineLimit(4...10)
                            .textInputAutocapitalization(.never).autocorrectionDisabled()
                            .font(.system(.footnote, design: .monospaced))
                            .padding(SpectraLayout.Space.m).spectraInputFieldStyle().foregroundStyle(Color.primary)
                        Text(AppLocalization.string("Spectra watches the account and builds its transactions for its signers to sign."))
                            .font(.caption).foregroundStyle(.secondary)
                    } else if draft.method == .watchViewKey {
                        Text(AppLocalization.string("Primary Address")).font(.headline).foregroundStyle(Color.primary)
                        TextField(chain.displayName, text: $draft.watchOnlyInput).textInputAutocapitalization(.never)
                            .autocorrectionDisabled().font(.system(.footnote, design: .monospaced))
                            .padding(SpectraLayout.Space.m).spectraInputFieldStyle().foregroundStyle(Color.primary)
                        Text(AppLocalization.string("Private View Key")).font(.headline).foregroundStyle(Color.primary)
                        TextField(AppLocalization.string("64 hex digits"), text: $draft.viewKeyInput).textInputAutocapitalization(.never)
                            .autocorrectionDisabled().font(.system(.footnote, design: .monospaced))
                            .padding(SpectraLayout.Space.m).spectraInputFieldStyle().foregroundStyle(Color.primary)
                        Text(AppLocalization.string("A view key shows what the wallet receives, not what it spends: its balance does not drop when it pays from another device."))
                            .font(.caption).foregroundStyle(.secondary)
                    } else {
                        let validation = watchedAddressValidationMessage(
                            entries: draft.watchOnlyEntries,
                            assetDisplayName: chain.displayName,
                            validator: { isValidWatchOnlyAddress(chain: chain, address: $0) }
                        )
                        watchedAddressSection(
                            text: $draft.watchOnlyInput,
                            validationMessage: validation.message, validationColor: validation.color
                        )
                    }
                }
            }
            if draft.method == .watchViewKey { RestoreHeightCard(draft: draft) }
            WalletAddressPreviewCard(store: store, draft: draft)
        }
    }
    /// The name first — the page is named for it — then the password a
    /// secret may be sealed under, then what the wallet will do and whom it
    /// asks, while the endpoints can still change.
    @ViewBuilder
    private var walletNamePageContent: some View {
        setupCard {
            HStack(spacing: SpectraLayout.Space.s) {
                TextField(
                    isEditingWallet
                        ? AppLocalization.string("import_flow.wallet_name")
                        : (defaultWalletName ?? AppLocalization.string("import_flow.wallet_name")),
                    text: $draft.walletName
                )
                .textInputAutocapitalization(.words).autocorrectionDisabled().foregroundStyle(Color.primary)
                .accessibilityLabel(AppLocalization.string("import_flow.wallet_name"))
                if !draft.walletName.isEmpty {
                    Button { draft.walletName = "" } label: {
                        Image(systemName: "xmark.circle.fill").font(.body.weight(.semibold))
                            .foregroundStyle(.secondary)
                    }.buttonStyle(.plain).accessibilityLabel(AppLocalization.string("Clear wallet name"))
                }
            }.padding(SpectraLayout.Space.m).spectraInputFieldStyle()
        }
        if draft.mode.takesWalletPassword {
            setupCard { walletPasswordStepSection }
        }
        if !isEditingWallet, let chain = draft.chain {
            WalletSetupSummaryCard(store: store, chain: chain)
        }
    }
    @ViewBuilder
    private var importStatusSection: some View {
        if let importError = store.walletImport.error {
            Text(importError).font(.footnote).foregroundStyle(.red.opacity(0.9))
        }
        if store.walletImport.isBusy {
            HStack(spacing: SpectraLayout.Space.s) {
                SpectraLoadingGlyph(size: 22, tint: .accentColor)
                Text(AppLocalization.string("import_flow.initializing_wallet_connections")).font(.footnote).foregroundStyle(.secondary)
            }.frame(maxWidth: .infinity, alignment: .leading)
        }
    }
    private func performPrimaryAction() {
        // Linear advance. `nil` from `next` means we're on the last page —
        // submit instead of routing.
        if let next = nextPageInFlow {
            // Backup verification checks a challenge drawn as it is entered.
            if next == .backupVerification { draft.prepareBackupVerificationChallenge() }
            nextPage = next
            return
        }
        let session = store.walletImport.id
        Task {
            guard store.walletImport.id == session else { return }
            await store.importWallet()
        }
    }
    var body: some View {
        ZStack {
            SpectraBackdrop().ignoresSafeArea()
            ScrollViewReader { proxy in
                ScrollView(showsIndicators: false) {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.l) {
                        setupHeader
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                            pageContent
                            importStatusSection
                        }
                    }
                    .spectraScreenPadding()
                    .scrollsToKeyboardAnchor(proxy)
                }
                .scrollBounceBehavior(.basedOnSize)
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .safeAreaBar(edge: .bottom) { setupBottomActionBar }
        .toolbar(.hidden, for: .tabBar)
        .navigationDestination(item: $nextPage) { page in
            SetupView(store: store, draft: draft, page: page)
        }
        .task(id: setupPage) {
            guard setupPage == .walletName, !isEditingWallet else { return }
            defaultWalletName = try? await store.bridge.ready().defaultWalletName()
        }
    }
    private var setupBottomActionBar: some View {
        SpectraBottomActionBar {
            Button(action: performPrimaryAction) {
                Text(primaryActionTitle)
                    .font(.body.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, SpectraLayout.Space.s)
            }.buttonStyle(.glassProminent).controlSize(.large).disabled(!isPrimaryActionEnabled)
        }
    }
}
