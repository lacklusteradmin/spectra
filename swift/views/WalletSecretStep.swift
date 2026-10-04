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
            HStack(spacing: SpectraLayout.Space.xs) {
                ForEach(CoreReferenceTables.standardSeedPhraseLengths, id: \.wordCount) { length in
                    seedPhraseLengthChip(length)
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
                UIPasteboard.general.string = draft.seedPhraseWords.joined(separator: " ")
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
        let isLikelyValid = !trimmed.isEmpty && isPrivateKeyHex(rawValue: draft.privateKeyInput)
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
    /// The shape is core's to judge (`isPrivateKeyHex`, which also takes a
    /// `0x` prefix), so this row only describes it; the border and the
    /// feedback below say whether the input fits.
    private var privateKeyMetadataRow: some View {
        HStack(spacing: SpectraLayout.Space.s) {
            Text(AppLocalization.string("Private key hex (64 or 128 characters)")).font(.caption2).foregroundStyle(.secondary)
            Spacer()
            if !draft.privateKeyInput.isEmpty {
                Button(role: .destructive) { draft.privateKeyInput = "" } label: {
                    Image(systemName: "xmark.circle.fill").font(.caption.weight(.semibold))
                }.buttonStyle(.plain).foregroundStyle(.secondary)
            }
        }
    }
    private var privateKeyValidationFeedback: (message: String, icon: String, color: Color)? {
        let trimmed = draft.privateKeyInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if !isPrivateKeyHex(rawValue: draft.privateKeyInput) {
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
        } else if isPrivateKeyImportMode {
            privateKeyImportFields
                .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
        } else {
            SeedPhraseEntryView(entry: draft.seedEntry)
                .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
            advancedCard
        }
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
        let presetPaths = SeedDerivationPaths.forPreset(draft.seedDerivationPreset)
        if draft.selectableDerivationChains.contains(where: {
            draft.seedDerivationPaths.path(for: $0) != presetPaths.path(for: $0)
        }) {
            names.append(AppLocalization.string("Derivation Paths"))
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
            if verdict.checksumValid, let language = verdict.language {
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
