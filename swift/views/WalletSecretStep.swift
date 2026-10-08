import SwiftUI

/// The wallet-setup secret step: enter or record a seed, paste a private key,
/// and verify a backup.
struct WalletSecretStep: View {
    let store: AppState
    @Bindable var draft: WalletImportDraft
    /// `true` renders the backup-verification page instead of the secret page.
    let showsBackupVerification: Bool

    private let copy = ImportFlowContent.current
    @State private var isShowingDerivationOptions = false
    @State private var isFindingUsedAccounts = false

    private var isCreateMode: Bool { draft.isCreateMode }
    private var isEditingWallet: Bool { draft.isEditingWallet }
    private let seedPhraseGridColumns = [
        GridItem(.flexible(), spacing: SpectraLayout.Space.xs), GridItem(.flexible(), spacing: SpectraLayout.Space.xs), GridItem(.flexible(), spacing: SpectraLayout.Space.xs),
    ]
    private var isPrivateKeyImportMode: Bool { draft.isPrivateKeyImportMode }
    private var canContinueFromSecretStep: Bool {
        draft.isSecretComplete && !store.walletImport.isBusy
    }

    var body: some View {
        Group {
            if showsBackupVerification {
                backupVerificationStepSection
            } else {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) { walletSecretStepSection }
            }
        }
        .sheet(isPresented: $isShowingDerivationOptions) {
            WalletDerivationOptionsView(store: store, draft: draft)
        }
        .sheet(isPresented: $isFindingUsedAccounts) {
            UsedAccountsSheet(store: store, draft: draft)
        }
    }

    private func backupVerificationBinding(for index: Int) -> Binding<String> {
        Binding(
            get: {
                guard draft.backupVerificationEntries.indices.contains(index) else { return "" }
                return draft.backupVerificationEntries[index]
            }, set: { draft.updateBackupVerificationEntry(at: index, with: $0) }
        )
    }
    @ViewBuilder
    private func seedPhraseLengthPicker(title: String, subtitle: String, showsRegenerateButton: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                    Text(AppLocalization.string(title)).font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary)
                    Text(AppLocalization.string(subtitle)).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if showsRegenerateButton {
                    Button {
                        draft.regenerateSeedPhrase()
                    } label: {
                        Label(AppLocalization.string("Regenerate"), systemImage: "arrow.clockwise").font(.caption.weight(.semibold))
                    }.buttonStyle(.glass).tint(.accentColor)
                }
            }
            // A network whose created format has one length — Monero's 25,
            // TON's 24 — has nothing to choose.
            if draft.createdLengths.count > 1 {
                HStack(spacing: SpectraLayout.Space.xs) {
                    ForEach(draft.createdLengths, id: \.wordCount) { length in
                        seedPhraseLengthChip(length)
                    }
                }
            }
        }
    }
    /// One chip per core-defined length, labelled with its entropy.
    @ViewBuilder
    private func seedPhraseLengthChip(_ length: SeedPhraseLength) -> some View {
        let wordCount = Int(length.wordCount)
        let isSelected = draft.selectedSeedPhraseWordCount == wordCount
        Button {
            draft.selectedSeedPhraseWordCount = wordCount
        } label: {
            VStack(spacing: SpectraLayout.Space.xxs) {
                Text("\(wordCount)").font(.title3.weight(.bold).monospacedDigit()).foregroundStyle(
                    isSelected ? Color.white : Color.primary)
                Text("\(length.entropyBits)b").font(.caption2.weight(.semibold)).foregroundStyle(
                    isSelected ? Color.white.opacity(0.8) : .secondary)
            }.frame(maxWidth: .infinity, minHeight: 56).spectraSelectableFill(isSelected: isSelected, accent: .accentColor, cornerRadius: SpectraLayout.Radius.inner)
        }.buttonStyle(.plain)
    }
    @ViewBuilder
    private var createWalletSeedPhraseSection: some View {
        seedPhraseLengthPicker(
            title: copy.createSeedLengthTitle, subtitle: copy.createSeedLengthSubtitle, showsRegenerateButton: true
        )
        Text(copy.createSeedPhraseWarning).font(.footnote).foregroundStyle(.secondary)
        seedPhraseDisplayHeader
        LazyVGrid(columns: seedPhraseGridColumns, spacing: SpectraLayout.Space.xs) {
            ForEach(draft.seedPhraseWords.indices, id: \.self) { index in
                SeedPhraseWordCell(index: index) {
                    Text(draft.seedPhraseWords[index]).font(.system(.callout, design: .monospaced).weight(.medium))
                        .foregroundStyle(Color.primary).lineLimit(1).minimumScaleFactor(0.7)
                }
            }
        }
    }
    @ViewBuilder
    private var seedPhraseDisplayHeader: some View {
        HStack(spacing: SpectraLayout.Space.s) {
            Label(AppLocalization.string("Recovery Phrase"), systemImage: "key.fill").font(.caption.weight(.semibold)).foregroundStyle(
                .tint
            ).padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.xs).background(
                Capsule(style: .continuous).fill(Color.accentColor.opacity(0.12)))
            Spacer()
            Button {
                copySecretToPasteboard(draft.seedPhraseWords.joined(separator: " "))
            } label: {
                Label(AppLocalization.string("Copy"), systemImage: "doc.on.doc").font(.caption.weight(.semibold))
            }.buttonStyle(.glass).tint(.accentColor).disabled(draft.seedPhraseWords.isEmpty)
        }
    }
    @ViewBuilder
    private var privateKeyImportFields: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            HStack(spacing: SpectraLayout.Space.s) {
                Text(copy.privateKeyTitle).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                Spacer()
                PasteButton(payloadType: String.self) { pasted in
                    guard let text = pasted.first?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else { return }
                    draft.privateKeyInput = text
                }
                .buttonBorderShape(.capsule)
                .labelStyle(.titleAndIcon)
                .controlSize(.small)
                .tint(.accentColor)
            }
            Text(copy.privateKeyPrompt).font(.footnote).foregroundStyle(.secondary)
            privateKeyEditor
            privateKeyMetadataRow
            if let validation = privateKeyValidationFeedback {
                Label(validation.message, systemImage: validation.icon).font(.footnote.weight(.medium)).foregroundStyle(validation.color)
            }
        }
    }
    @ViewBuilder
    private var privateKeyEditor: some View {
        let trimmed = draft.privateKeyInput.trimmingCharacters(in: .whitespacesAndNewlines)
        let isLikelyValid = !trimmed.isEmpty && draft.isSecretComplete
        let isInvalidShape = !trimmed.isEmpty && !isLikelyValid
        let borderColor: Color? =
            isInvalidShape
            ? Color.red.opacity(0.85) : (isLikelyValid ? Color.green.opacity(0.55) : nil)
        ZStack(alignment: .topLeading) {
            TextEditor(text: $draft.privateKeyInput).textInputAutocapitalization(.never).autocorrectionDisabled().scrollContentBackground(
                .hidden
            ).font(.system(.footnote, design: .monospaced)).foregroundStyle(Color.primary).frame(minHeight: 96).padding(.horizontal, SpectraLayout.Space.s)
                .padding(.vertical, SpectraLayout.Space.s).spectraInputFieldStyle(borderColor: borderColor)
            if trimmed.isEmpty {
                Text(copy.privateKeyPlaceholder).font(.system(.footnote, design: .monospaced)).foregroundStyle(.secondary).padding(
                    .horizontal, SpectraLayout.Space.l).padding(.vertical, SpectraLayout.Space.l).allowsHitTesting(false)
            }
        }
    }
    /// The formats the network takes, from its setup descriptor. The shape is
    /// core's to judge, so this row only names it; the border and the feedback
    /// below say whether the input fits.
    private var privateKeyMetadataRow: some View {
        HStack(spacing: SpectraLayout.Space.s) {
            Text(privateKeyFormats).font(.caption2).foregroundStyle(.secondary)
            Spacer()
            if !draft.privateKeyInput.isEmpty {
                Button(role: .destructive) { draft.privateKeyInput = "" } label: {
                    Image(systemName: "xmark.circle.fill").font(.caption.weight(.semibold))
                }.buttonStyle(.plain).foregroundStyle(.secondary)
            }
        }
    }
    private var privateKeyFormats: String {
        guard let chain = draft.chain else { return "" }
        let option = walletSetupDescriptor(chain: chain).options.first { $0.method == .importPrivateKey }
        return (option?.formats ?? []).map(\.title).joined(separator: " · ")
    }
    private var privateKeyValidationFeedback: (message: String, icon: String, color: Color)? {
        let trimmed = draft.privateKeyInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if !draft.isSecretComplete {
            return (
                AppLocalization.string("Enter a valid private key for the selected network."), "exclamationmark.triangle.fill",
                .red.opacity(0.92)
            )
        }
        return (AppLocalization.string("Looks like a valid private key."), "checkmark.seal.fill", .green.opacity(0.92))
    }
    /// Creating a wallet shows its phrase in one card. Importing a phrase
    /// shows the entry, then the one way into everything a default already
    /// answers; importing a private key shows the key.
    @ViewBuilder
    private var walletSecretStepSection: some View {
        if isCreateMode {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                createWalletSeedPhraseSection
                derivationOptionsLink
            }
            .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
            derivationAccountCard
            WalletAddressPreviewCard(store: store, draft: draft)
        } else if isPrivateKeyImportMode {
            privateKeyImportFields
                .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
            namedAccountCard
            tonWalletVersionCard
            WalletAddressPreviewCard(store: store, draft: draft)
        } else {
            SeedPhraseEntryView(entry: draft.seedEntry)
                .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
            if draft.asksRestoreHeight { restoreHeightCard }
            derivationAccountCard
            namedAccountCard
            tonWalletVersionCard
            WalletAddressPreviewCard(store: store, draft: draft)
            advancedCard
        }
    }
    /// Which account the phrase derives: the network's profile, chosen when
    /// it has several, and the account index on it — both core's, from the
    /// setup descriptor. A custom path under Advanced replaces them.
    @ViewBuilder
    private var derivationAccountCard: some View {
        if !draft.derivationProfiles.isEmpty {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                Text(AppLocalization.string("Account")).font(.subheadline.weight(.semibold))
                if draft.customDerivationPath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    if draft.derivationProfiles.count > 1 {
                        HStack {
                            Text(AppLocalization.string("Address Type")).font(.subheadline)
                            Spacer()
                            Picker(AppLocalization.string("Address Type"), selection: $draft.derivationProfile) {
                                ForEach(draft.derivationProfiles, id: \.self) { profile in
                                    Text(profile.title).tag(Optional(profile))
                                }
                            }.pickerStyle(.menu).tint(.secondary)
                        }
                    }
                    Stepper(value: $draft.derivationAccount, in: 0...UInt32(Int32.max)) {
                        Text(AppLocalization.format("Account %lld", Int(draft.derivationAccount))).font(.subheadline)
                    }
                } else {
                    Text(AppLocalization.string("The custom path under Advanced replaces the account."))
                        .font(.caption).foregroundStyle(.spectraWarning)
                }
                if let path = draft.derivationPath {
                    Text(path).font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                }
                findUsedAccountsButton
            }
            .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
        }
    }
    /// Restoring, the phrase may have been used on another account or wallet
    /// version; a scan finds it, on request.
    @ViewBuilder
    private var findUsedAccountsButton: some View {
        if !isCreateMode, !isPrivateKeyImportMode, draft.seedEntry.verdict.isValid {
            Button {
                isFindingUsedAccounts = true
            } label: {
                Label(AppLocalization.string("Find Used Accounts"), systemImage: "magnifyingglass")
                    .font(.subheadline.weight(.semibold))
            }.buttonStyle(.glass)
        }
    }
    /// A named account the key controls, on a network that has them. The
    /// import confirms on the network that the key is one of its full-access
    /// keys before the wallet is added.
    @ViewBuilder
    private var namedAccountCard: some View {
        if draft.asksNamedAccount {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                Text(AppLocalization.string("Named Account (Optional)")).font(.subheadline.weight(.semibold))
                TextField(AppLocalization.string("Named account"), text: $draft.namedAccountInput)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().font(.body.monospaced())
                    .padding(SpectraLayout.Space.m).spectraInputFieldStyle()
                Text(AppLocalization.string(
                    "If this key controls a named account, enter it. Adding the wallet checks on the network that the key is one of its full-access keys."
                )).font(.caption).foregroundStyle(.secondary)
            }
            .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
        }
    }
    /// The wallet contract a restored TON key's account is under. Each
    /// version is a different address, which the preview below shows.
    @ViewBuilder
    private var tonWalletVersionCard: some View {
        if draft.asksTonWalletVersion {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                Text(AppLocalization.string("Wallet Version")).font(.subheadline.weight(.semibold))
                Picker(AppLocalization.string("Wallet Version"), selection: $draft.tonWalletVersion) {
                    ForEach(TonWalletVersion.allCases, id: \.self) { version in
                        Text(version.title).tag(version)
                    }
                }.pickerStyle(.segmented)
                Text(AppLocalization.string(
                    "Each version is a different address for the same key. Wallets created before 2024 usually use v4R2."
                )).font(.caption).foregroundStyle(.secondary)
                findUsedAccountsButton
            }
            .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
        }
    }
    /// Where a restored wallet's scan starts: a Monero wallet's, or a Zcash
    /// wallet's for its shielded funds. Blank reads a Polyseed's birthday, or
    /// scans from the first block the wallet could have received at.
    private var restoreHeightCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            Text(AppLocalization.string("Restore Height (Optional)")).font(.subheadline.weight(.semibold))
            TextField(AppLocalization.string("Block height"), text: $draft.restoreHeightInput)
                .keyboardType(.numberPad).font(.body.monospacedDigit())
                .padding(SpectraLayout.Space.m).spectraInputFieldStyle()
            if draft.isRestoreHeightValid {
                Text(AppLocalization.string(
                    "The scan starts here and cannot find funds received earlier. Leave it blank to use the phrase's creation date when it records one (a Polyseed), or to scan from the first block the wallet could have received at, which takes longest."
                )).font(.caption).foregroundStyle(.secondary)
            } else {
                Text(AppLocalization.string("A restore height is a whole block number.")).font(.caption)
                    .foregroundStyle(.red.opacity(0.9))
            }
        }
        .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
    }
    @ViewBuilder
    private var backupVerificationStepSection: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(copy.backupVerificationTitle).font(.headline).foregroundStyle(Color.primary)
            if !draft.backupVerificationPromptLabel.isEmpty {
                Text(draft.backupVerificationPromptLabel).font(.subheadline).foregroundStyle(.secondary)
            }
            if draft.backupVerificationWordIndices.isEmpty {
                Button(copy.backupVerificationButtonTitle) {
                    draft.prepareBackupVerificationChallenge()
                }.buttonStyle(.glass)
            } else {
                ForEach(draft.backupVerificationWordIndices.indices, id: \.self) { offset in
                    let wordIndex = draft.backupVerificationWordIndices[offset]
                    HStack(spacing: SpectraLayout.Space.s) {
                        Text(AppLocalization.format("Word #%lld", wordIndex + 1)).font(.caption.weight(.bold)).foregroundStyle(.secondary).frame(width: 72, alignment: .leading)
                        TextField("", text: backupVerificationBinding(for: offset)).textInputAutocapitalization(
                            .never
                        ).autocorrectionDisabled()
                        .font(.system(.footnote, design: .monospaced).weight(.medium))
                        .foregroundStyle(Color.primary)
                    }.padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.s).spectraInputFieldStyle(cornerRadius: SpectraLayout.Radius.inner)
                }
                if draft.isBackupVerificationComplete {
                    Text(copy.backupVerifiedMessage).font(.footnote).foregroundStyle(.green.opacity(0.9))
                } else {
                    Text(copy.backupVerificationHint).font(.footnote).foregroundStyle(.secondary)
                }
            }
        }.padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
    }
    /// The derivation options the sheet has moved off their defaults, by
    /// name. The entry to the sheet is deliberately quiet, so it says when
    /// something behind it changes which addresses the seed derives.
    private var customizedDerivationOptionNames: [String] {
        var names: [String] = []
        if !isCreateMode, draft.seedEntry.wordCountOverride != nil { names.append(AppLocalization.string("Word Count")) }
        if !isCreateMode, draft.seedEntry.language != nil { names.append(AppLocalization.string("Wordlist")) }
        if !draft.customDerivationPath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            names.append(AppLocalization.string("Custom Path"))
        }
        if !draft.overridePassphrase.isEmpty { names.append(AppLocalization.string("Passphrase")) }
        if !draft.overrideHmacKey.isEmpty { names.append(AppLocalization.string("HMAC Master Key")) }
        return names
    }
    private var derivationOptionsSummary: String? {
        let names = customizedDerivationOptionNames
        guard !names.isEmpty else { return nil }
        let list = ListFormatter()
        list.locale = AppLocalization.locale
        return AppLocalization.format(
            "import_flow.advanced_customized_format", list.string(from: names) ?? names.joined(separator: ", "))
    }
    /// The import's Advanced entry, as a row in its own card. Says what is
    /// in effect when nothing was changed, and what was changed otherwise.
    private var advancedCard: some View {
        let summary = derivationOptionsSummary
        let verdict = draft.seedEntry.verdict
        let inEffect: String =
            if verdict.isValid, let language = verdict.language {
                AppLocalization.format(
                    "import_flow.advanced_in_effect_format", Int(verdict.wordCount), AppLocalization.string(language.name))
            } else {
                AppLocalization.string("import_flow.advanced_seed_subtitle")
            }
        return Button {
            isShowingDerivationOptions = true
        } label: {
            HStack(spacing: SpectraLayout.Space.m) {
                Image(systemName: "slider.horizontal.3").font(.body.weight(.semibold))
                    .foregroundStyle(summary == nil ? AnyShapeStyle(.tint) : AnyShapeStyle(Color.spectraWarning))
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(copy.advancedTitle).font(.body.weight(.semibold)).foregroundStyle(Color.primary)
                    Text(summary ?? inEffect).font(.caption)
                        .foregroundStyle(summary == nil ? Color.secondary : .spectraWarning)
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
    private var derivationOptionsLink: some View {
        let summary = derivationOptionsSummary
        return Button {
            isShowingDerivationOptions = true
        } label: {
            HStack(spacing: SpectraLayout.Space.s) {
                Image(systemName: "slider.horizontal.3").font(.footnote.weight(.semibold))
                    .foregroundStyle(summary == nil ? Color.secondary : .spectraWarning)
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(copy.advancedTitle).font(.footnote.weight(.semibold)).foregroundStyle(Color.primary)
                    Text(summary ?? copy.advancedSubtitle).font(.caption2)
                        .foregroundStyle(summary == nil ? Color.secondary : .spectraWarning)
                }
                Spacer()
                Image(systemName: "chevron.right").font(.caption2.weight(.bold)).foregroundStyle(.tertiary)
            }.padding(.vertical, SpectraLayout.Space.xs).contentShape(Rectangle())
        }.buttonStyle(.plain)
    }
}
