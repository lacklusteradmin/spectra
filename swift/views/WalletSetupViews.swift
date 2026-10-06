import Foundation
import SwiftUI

/// One page of wallet setup. The form lives in the `@Bindable` draft; the
/// store supplies app-wide state and performs the import.
struct SetupView: View {
    /// Every chain, in the picker's popular order.
    private static let allChainDescriptors = ChainSelectionDescriptor.popularOrder(Chain.all)
    /// The chains this import can use, in popular order.
    private var chainSelectionDescriptors: [ChainSelectionDescriptor] {
        Self.allChainDescriptors.filter { draft.offers($0.id) }
    }
    /// Linear flow for the current mode. Drives the step counter, primary
    /// action routing, and back routing — replacing three separate switch
    /// statements that historically had to stay in sync.
    private var setupFlow: SetupFlow {
        if isEditingWallet { return .editWallet }
        if usesWatchAddressesFlow { return .watchOnly }
        if isCreateMode { return .createNewWallet }
        return .seedPhraseImport
    }
    private let store: AppState
    @Bindable var draft: WalletImportDraft
    private let copy = ImportFlowContent.current
    /// The page this screen shows. Each page is its own pushed screen, so the
    /// system back button and the swipe step back one page at a time.
    private let setupPage: WalletSetupPage
    @State private var nextPage: WalletSetupPage?
    @State private var chainSearchText: String = ""
    @State private var isShowingAllChainsPage: Bool = false
    init(store: AppState, draft: WalletImportDraft, page: WalletSetupPage? = nil) {
        self.store = store
        self.draft = draft
        setupPage = page ?? (draft.isEditingWallet ? .walletName : .details)
    }
    private var isEditingWallet: Bool { draft.isEditingWallet }
    private var isCreateMode: Bool { draft.isCreateMode }
    private var isWatchAddressesImportMode: Bool { !isEditingWallet && !isCreateMode && draft.isWatchOnlyMode }
    private var usesSeedPhraseFlow: Bool { !isEditingWallet && !draft.isWatchOnlyMode }
    private var isPrivateKeyImportMode: Bool { draft.isPrivateKeyImportMode }
    private var usesWatchAddressesFlow: Bool { !isEditingWallet && draft.isWatchOnlyMode }
    private var pageCopy: WalletSetupPageCopy {
        setupPage.copy(
            copy,
            mode: WalletSetupMode(
                isEditingWallet: isEditingWallet, isCreateMode: isCreateMode,
                isPrivateKeyImport: isPrivateKeyImportMode, isWatchOnly: draft.isWatchOnlyMode))
    }
    private var setupTitle: String { pageCopy.title }
    private var setupSubtitle: String { pageCopy.subtitle }
    private var canContinueFromSecretStep: Bool {
        draft.isSecretComplete && !store.walletImport.isBusy
    }
    private var canContinueToBackupVerification: Bool {
        canContinueFromSecretStep
            && draft.walletPasswordValidationError == nil
            && !store.walletImport.isBusy
    }
    private var canSubmitFromPasswordStep: Bool {
        draft.walletPasswordValidationError == nil
            && store.canImportWallet
            && !store.walletImport.isBusy
    }
    private var canAdvanceFromDetailsPage: Bool {
        if usesSeedPhraseFlow { return !draft.selectedChains.isEmpty && !store.walletImport.isBusy }
        if usesWatchAddressesFlow { return !draft.selectedChains.isEmpty && !store.walletImport.isBusy }
        return store.canImportWallet && !store.walletImport.isBusy
    }
    /// What the primary button says on the page that submits rather than
    /// advances. Shared by every page that reaches the end of its flow.
    private var submitActionTitle: String {
        if isEditingWallet { return AppLocalization.string("import_flow.save_wallet") }
        if isCreateMode { return AppLocalization.string("import_flow.create_wallet") }
        return isWatchAddressesImportMode
            ? AppLocalization.string("import_flow.watch_addresses") : AppLocalization.string("import_flow.import_wallet")
    }
    private var canSubmitSetup: Bool { store.canImportWallet && !store.walletImport.isBusy }
    private var primaryActionTitle: String {
        let next = AppLocalization.string("import_flow.next")
        switch setupPage {
        case .seedPhrase:
            return next
        case .details:
            return (usesSeedPhraseFlow || usesWatchAddressesFlow) ? next : submitActionTitle
        case .password:
            if isCreateMode { return AppLocalization.string("import_flow.continue_to_backup_verification") }
            return advancesToWalletName ? next : submitActionTitle
        // Both advance to the wallet-name step rather than submitting; that
        // step performs the final submit.
        case .watchAddresses, .backupVerification:
            return advancesToWalletName ? next : submitActionTitle
        case .walletName:
            return submitActionTitle
        }
    }
    private var isPrimaryActionEnabled: Bool {
        switch setupPage {
        case .seedPhrase:
            return canContinueFromSecretStep
        case .details:
            return (usesSeedPhraseFlow || usesWatchAddressesFlow) ? canAdvanceFromDetailsPage : canSubmitSetup
        case .password:
            return isCreateMode ? canContinueToBackupVerification : (canSubmitFromPasswordStep || advancesToWalletName)
        case .watchAddresses:
            return canAdvanceFromWatchAddressesPage
        case .backupVerification, .walletName:
            return canSubmitSetup
        }
    }
    /// True when the current page should advance to the `.walletName` step
    /// rather than submitting directly.
    private var advancesToWalletName: Bool {
        guard !isEditingWallet else { return false }
        switch setupPage {
        case .password: return isCreateMode ? false : canSubmitFromPasswordStep
        case .backupVerification: return true
        case .watchAddresses: return canAdvanceFromWatchAddressesPage
        case .details, .seedPhrase, .walletName: return false
        }
    }
    private var canAdvanceFromWatchAddressesPage: Bool {
        store.canImportWallet && !store.walletImport.isBusy
    }
    private var selectedChainSet: Set<Chain> { Set(draft.selectedChains) }
    private var selectedChainCount: Int { draft.selectedChains.count }
    private var chainSelectionSummary: String {
        switch selectedChainCount {
        case 0: return AppLocalization.string("import_flow.no_chains_selected")
        case 1: return AppLocalization.string("import_flow.one_chain_selected")
        default: return AppLocalization.format("import_flow.multiple_chains_selected_format", selectedChainCount)
        }
    }
    @ViewBuilder
    private func watchedAddressEditor(text: Binding<String>) -> some View {
        TextEditor(text: text).textInputAutocapitalization(.never).autocorrectionDisabled().scrollContentBackground(.hidden).frame(
            minHeight: 88
        ).padding(SpectraLayout.Space.s).spectraInputFieldStyle().foregroundStyle(Color.primary)
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
            Text(AppLocalization.string("import_flow.wallet_password_explanation")).font(.subheadline).foregroundStyle(.secondary)
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
        title: String, text: Binding<String>, caption: String? = nil, validationMessage: String? = nil, validationColor: Color? = nil
    ) -> some View {
        Text(AppLocalization.string(title)).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
        watchedAddressEditor(text: text)
        if let caption { Text(caption).font(.caption).foregroundStyle(.secondary) }
        if let validationMessage { Text(validationMessage).font(.caption).foregroundStyle(validationColor ?? Color.secondary) }
    }
    private func watchedAddressValidationMessage(
        entries: [String], assetDisplayName: String, validator: (String) -> Bool
    ) -> (message: String, color: Color) {
        let localizedAssetName = assetDisplayName
        if entries.isEmpty {
            return (AppLocalization.format("Enter one %@ address per line.", localizedAssetName), Color.secondary)
        }
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
            Text(setupTitle).font(.largeTitle.weight(.bold)).foregroundStyle(Color.primary)
                .lineLimit(3).minimumScaleFactor(0.7).allowsTightening(true).fixedSize(horizontal: false, vertical: true)
            Text(setupSubtitle).font(.subheadline).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
    /// Single rendering entry point for the page body. Replaces six
    /// separate `*PageSection` properties stacked in a VStack, each with
    /// their own internal "if isShowing<X>" gate that could drift out of
    /// sync with the page enum. A switch over `setupPage` makes the page
    /// → content map structural — adding a page is one new case rather
    /// than "remember to add the section *and* gate it correctly inside."
    @ViewBuilder
    private var pageContent: some View {
        switch setupPage {
        case .details:
            if !isEditingWallet { chainSelectionCard }
        case .watchAddresses:
            if !isEditingWallet, draft.isWatchOnlyMode { watchAddressesPageContent }
        case .seedPhrase:
            if !draft.isWatchOnlyMode {
                WalletSecretStep(store: store, draft: draft, showsBackupVerification: false)
            }
        case .password:
            passwordPageContent
        case .backupVerification:
            WalletSecretStep(store: store, draft: draft, showsBackupVerification: true)
        case .walletName:
            walletNamePageContent
        }
    }
    /// The most popular chains as rows, then the way to every chain. The
    /// count beside the title stands in for a header, since the page title
    /// above already names the step.
    @ViewBuilder
    private var chainSelectionCard: some View {
        let mainnets = chainSelectionDescriptors.filter { !$0.isTestnet }
        // The most popular mainnets are listed on the page itself; the rest
        // are one tap away behind "Browse all".
        let shortList = Array(mainnets.prefix(6))
        let shortListIDs = Set(shortList.map(\.id))
        let allowsMultipleSelection = draft.allowsMultipleChainSelection
        let extraSelectionCount = draft.selectedChains.filter { !shortListIDs.contains($0) }.count
        VStack(alignment: .leading, spacing: SpectraLayout.sectionSpacing) {
            SpectraRowGroup(
                title: AppLocalization.string("Popular chains"), trailing: chainSelectionSummary, data: shortList
            ) { descriptor in
                ChainSelectionRow(
                    descriptor: descriptor, isSelected: selectedChainSet.contains(descriptor.id),
                    allowsMultipleSelection: allowsMultipleSelection
                ) { draft.toggleChainSelection(descriptor.id) }
            }
            Button {
                chainSearchText = ""
                isShowingAllChainsPage = true
            } label: {
                HStack(spacing: SpectraLayout.Space.m) {
                    Text(AppLocalization.format("Browse all %lld chains", mainnets.count))
                        .font(.body.weight(.semibold))
                        .foregroundStyle(.tint)
                    Spacer(minLength: SpectraLayout.Space.s)
                    if extraSelectionCount > 0 {
                        Text("+\(extraSelectionCount)")
                            .font(.subheadline.weight(.semibold)).foregroundStyle(.secondary).monospacedDigit()
                    }
                    Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                }
                .spectraRowPadding()
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .spectraCardFill()
        }
        .navigationDestination(isPresented: $isShowingAllChainsPage) {
            AllChainsSelectionView(
                chainSearchText: $chainSearchText, descriptors: chainSelectionDescriptors,
                selectedChains: selectedChainSet,
                toggleSelection: { chain in
                    draft.toggleChainSelection(chain)
                    // A single choice is made once picked.
                    if !allowsMultipleSelection { isShowingAllChainsPage = false }
                },
                clearAllSelections: allowsMultipleSelection
                    ? { for chain in draft.selectedChains { draft.toggleChainSelection(chain) } } : nil
            )
        }
    }
    /// Page-level rendering contract: callers (the `pageContent` switch)
    /// have already verified the page is active. These `*PageContent`
    /// properties don't re-check `isShowing<X>` — they just render.
    @ViewBuilder
    private var watchAddressesPageContent: some View {
        setupCard {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                Text(copy.addressesToWatchTitle).font(.headline).foregroundStyle(Color.primary)
                Text(copy.addressesToWatchSubtitle).font(.subheadline).foregroundStyle(.secondary)
                watchAddressesInputsGroup
                watchAddressesEmptyNote
            }
        }
    }
    /// The one field a watch-only import has: addresses on the chain it is
    /// on, each judged by core's watch-only import rule.
    @ViewBuilder
    private var watchAddressesInputsGroup: some View {
        if let chain = draft.selectedChains.first {
            let validation = watchedAddressValidationMessage(
                entries: draft.watchOnlyEntries,
                assetDisplayName: chain.displayName,
                validator: { isValidWatchOnlyAddress(chain: chain, address: $0) }
            )
            watchedAddressSection(
                title: chain.displayName, text: $draft.watchOnlyInput,
                caption: chain.acceptsAccountXpub ? copy.bitcoinWatchCaption : nil,
                validationMessage: validation.message, validationColor: validation.color
            )
            // A chain whose import takes an account xpub has a second form: one
            // xpub instead of a list of addresses.
            if chain.acceptsAccountXpub {
                TextField("xpub... / zpub...", text: $draft.bitcoinXpubInput).textInputAutocapitalization(.never)
                    .autocorrectionDisabled().padding(SpectraLayout.Space.m).spectraInputFieldStyle().foregroundStyle(Color.primary)
            }
        }
    }

    @ViewBuilder
    private var watchAddressesEmptyNote: some View {
        if draft.selectedChains.isEmpty {
            Text(AppLocalization.string("Select a supported chain above to enter its address to watch.")).font(.caption)
                .foregroundStyle(.spectraWarning.opacity(0.9))
        }
    }
    @ViewBuilder
    private var walletNamePageContent: some View {
        setupCard {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                Text(
                    isEditingWallet
                        ? AppLocalization.string("import_flow.wallet_name")
                        : AppLocalization.string("import_flow.wallet_name_optional")
                ).font(.headline).foregroundStyle(Color.primary)
                if !isEditingWallet {
                    Text(AppLocalization.string("import_flow.wallet_name_hint")).font(.subheadline).foregroundStyle(.secondary)
                }
                HStack(spacing: SpectraLayout.Space.s) {
                    TextField(AppLocalization.string("import_flow.wallet_name_placeholder"), text: $draft.walletName)
                        .textInputAutocapitalization(.words).autocorrectionDisabled().foregroundStyle(Color.primary)
                    if !draft.walletName.isEmpty {
                        Button { draft.walletName = "" } label: {
                            Image(systemName: "xmark.circle.fill").font(.system(size: 18, weight: .semibold))
                                .foregroundStyle(.secondary)
                        }.buttonStyle(.plain).accessibilityLabel(AppLocalization.string("Clear wallet name"))
                    }
                }.padding(SpectraLayout.Space.m).spectraInputFieldStyle()
            }
        }
    }
    @ViewBuilder
    private var passwordPageContent: some View {
        setupCard { walletPasswordStepSection }
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
        if let next = setupFlow.next(after: setupPage) {
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
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.l) {
                    setupHeader
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                        pageContent
                        importStatusSection
                    }
                }.spectraScreenPadding()
            }.scrollBounceBehavior(.basedOnSize)
        }
        .navigationBarTitleDisplayMode(.inline)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            setupBottomActionBar
        }
        .navigationDestination(item: $nextPage) { page in
            SetupView(store: store, draft: draft, page: page)
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
