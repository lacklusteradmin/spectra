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
    @State private var isEditingPrivateKey = false
    /// A new phrase stays covered until asked for, so it is never on screen
    /// by surprise; leaving the page covers it again.
    @State private var isPhraseRevealed = false
    @State private var focusedVerificationSlot: Int?

    private var isCreateMode: Bool { draft.isCreateMode }
    private var isEditingWallet: Bool { draft.isEditingWallet }
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
        .onDisappear { isPhraseRevealed = false }
    }

    private func backupVerificationBinding(for index: Int) -> Binding<String> {
        Binding(
            get: {
                guard draft.backupVerificationEntries.indices.contains(index) else { return "" }
                return draft.backupVerificationEntries[index]
            }, set: { draft.updateBackupVerificationEntry(at: index, with: $0) }
        )
    }
    /// The new phrase: what it is for, then its words, covered until the
    /// user asks to see them. Length, account and path are under Advanced.
    @ViewBuilder
    private var createWalletSeedPhraseSection: some View {
        Label(copy.createSeedPhraseWarning, systemImage: "exclamationmark.triangle.fill")
            .font(.subheadline).foregroundStyle(.spectraWarning)
            .fixedSize(horizontal: false, vertical: true)
        SeedPhraseWordGrid(words: draft.seedPhraseWords)
            .blur(radius: isPhraseRevealed ? 0 : 10)
            .accessibilityHidden(!isPhraseRevealed)
            .overlay {
                if !isPhraseRevealed {
                    Button {
                        spectraHaptic(.light)
                        isPhraseRevealed = true
                    } label: {
                        Label(AppLocalization.string("Tap to Reveal"), systemImage: "eye.fill")
                            .font(.headline).padding(.horizontal, SpectraLayout.Space.l).padding(.vertical, SpectraLayout.Space.s)
                    }
                    .buttonStyle(.glassProminent)
                    .accessibilityHint(AppLocalization.string("secret.reveal.hint"))
                }
            }
            .secretShield()
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
        AddressInput(text: $draft.privateKeyInput, isFocused: $isEditingPrivateKey, prompt: copy.privateKeyPlaceholder)
            .frame(minHeight: 96, alignment: .topLeading)
            .padding(SpectraLayout.Space.m).spectraInputFieldStyle(borderColor: borderColor)
            .privacySensitive()
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
                        .frame(minWidth: 44, minHeight: 44)
                        .contentShape(Rectangle())
                }.buttonStyle(.plain).foregroundStyle(.secondary)
                .accessibilityLabel(AppLocalization.string("Clear"))
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
                Divider().opacity(0.4)
                derivationOptionsLink
            }
            .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
        } else if isPrivateKeyImportMode {
            privateKeyImportFields
                .secretShield()
                .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
                .keyboardAnchor()
            namedAccountCard
            tonWalletVersionCard
            WalletAddressPreviewCard(store: store, draft: draft)
        } else {
            SeedPhraseEntryView(entry: draft.seedEntry)
                .secretShield()
                .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
                .keyboardAnchor()
            if draft.asksRestoreHeight { RestoreHeightCard(draft: draft) }
            DerivationAccountCard(draft: draft) { findUsedAccountsButton }
            JunctionPathCard(draft: draft)
            namedAccountCard
            tonWalletVersionCard
            WalletAddressPreviewCard(store: store, draft: draft)
            advancedCard
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
    /// The words asked for back, each judged once it is left: a field moved
    /// past says whether it was right, so a wrong word is found by its row.
    @ViewBuilder
    private var backupVerificationStepSection: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            if !draft.backupVerificationPromptLabel.isEmpty {
                Text(draft.backupVerificationPromptLabel).font(.subheadline).foregroundStyle(.secondary)
            }
            if draft.backupVerificationWordIndices.isEmpty {
                Button(copy.backupVerificationButtonTitle) {
                    draft.prepareBackupVerificationChallenge()
                }.buttonStyle(.glass)
            } else {
                ForEach(Array(draft.backupVerificationWordIndices.enumerated()), id: \.offset) { offset, wordIndex in
                    verificationRow(offset: offset, wordIndex: wordIndex)
                }
                if draft.isBackupVerificationComplete {
                    Label(copy.backupVerifiedMessage, systemImage: "checkmark.circle.fill").font(.footnote).foregroundStyle(.green)
                } else {
                    Text(copy.backupVerificationHint).font(.footnote).foregroundStyle(.secondary)
                }
            }
        }
        .secretShield()
        .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
    }
    private func verificationRow(offset: Int, wordIndex: Int) -> some View {
        let label = AppLocalization.format("Word #%lld", wordIndex + 1)
        let entered = draft.backupVerificationEntries.indices.contains(offset) ? draft.backupVerificationEntries[offset] : ""
        let expected = draft.seedPhraseWords.indices.contains(wordIndex) ? draft.seedPhraseWords[wordIndex] : nil
        // Judged once left, never while typed.
        let verdict: Bool? = (focusedVerificationSlot == offset || entered.isEmpty) ? nil : entered == expected
        return HStack(spacing: SpectraLayout.Space.s) {
            Text(label).font(.caption.weight(.bold)).foregroundStyle(.secondary).fixedSize()
            SecretWordField(
                text: backupVerificationBinding(for: offset), isFocused: focusedVerificationSlot == offset,
                accessibilityLabel: label, isInvalid: verdict == false,
                onFocus: { focusedVerificationSlot = offset },
                onSubmit: {
                    let next = offset + 1
                    focusedVerificationSlot = next < draft.backupVerificationWordIndices.count ? next : nil
                })
            if let verdict {
                Image(systemName: verdict ? "checkmark.circle.fill" : "xmark.circle.fill")
                    .foregroundStyle(verdict ? Color.green : Color.red)
                    .accessibilityLabel(AppLocalization.string(verdict ? "Correct" : "Incorrect"))
            }
        }
        .padding(.horizontal, SpectraLayout.Space.m).padding(.vertical, SpectraLayout.Space.s)
        .frame(minHeight: 44)
        .spectraInputFieldStyle(cornerRadius: SpectraLayout.Radius.inner)
    }
    /// The derivation options the sheet has moved off their defaults, by
    /// name. The entry to the sheet is deliberately quiet, so it says when
    /// something behind it changes which addresses the seed derives.
    private var customizedDerivationOptionNames: [String] {
        var names: [String] = []
        if isCreateMode, let shortest = draft.createdLengths.first, draft.selectedSeedPhraseWordCount != Int(shortest.wordCount) {
            names.append(AppLocalization.string("Phrase Length"))
        }
        if draft.derivationAccount != 0 || draft.derivationProfile != draft.derivationProfiles.first {
            names.append(AppLocalization.string("Account"))
        }
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

/// A phrase's words as shown, numbered, each whole on one line: three
/// columns, two at accessibility sizes. A word shrinks to fit rather than
/// wrap, because a wrapped word takes a hyphen that reads as part of it, and
/// rather than be cut, which would hide letters.
struct SeedPhraseWordGrid: View {
    let words: [String]
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        let columns = Array(
            repeating: GridItem(.flexible(), spacing: SpectraLayout.Space.xs),
            count: dynamicTypeSize.isAccessibilitySize ? 2 : 3)
        LazyVGrid(columns: columns, spacing: SpectraLayout.Space.xs) {
            // Each cell is given its word, not an index into a list that may
            // be shorter by the time the cell is drawn.
            ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                SeedPhraseWordCell(index: index) {
                    Text(verbatim: word).font(.system(.callout, design: .monospaced).weight(.medium))
                        .foregroundStyle(Color.primary).lineLimit(1).minimumScaleFactor(0.5)
                        .privacySensitive()
                }
                .accessibilityElement(children: .combine)
            }
        }
    }
}

/// Which account the phrase derives: the network's profile, chosen when it
/// has several, and the account index on it — both core's, from the setup
/// descriptor. A custom path under Advanced replaces them.
struct DerivationAccountCard<Accessory: View>: View {
    @Bindable var draft: WalletImportDraft
    @ViewBuilder var accessory: Accessory

    var body: some View {
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
                    Stepper(value: $draft.derivationAccount, in: 0...99) {
                        Text(AppLocalization.format("Account %lld", Int(draft.derivationAccount))).font(.subheadline)
                    }
                } else {
                    Text(AppLocalization.string("The custom path under Advanced replaces the account."))
                        .font(.caption).foregroundStyle(.spectraWarning)
                }
                if let path = draft.derivationPath {
                    Text(path).font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                }
                accessory
            }
            .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
        }
    }
}

extension DerivationAccountCard where Accessory == EmptyView {
    init(draft: WalletImportDraft) { self.init(draft: draft) { EmptyView() } }
}

/// A Substrate network's account below the phrase's root: hard and soft
/// junctions, as its wallets write them. Core refuses a path it cannot read
/// one way.
struct JunctionPathCard: View {
    @Bindable var draft: WalletImportDraft

    var body: some View {
        if draft.asksJunctionPath {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                Text(AppLocalization.string("Derivation Path (Optional)")).font(.subheadline.weight(.semibold))
                TextField("//polkadot//0", text: $draft.junctionPathInput)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().font(.body.monospaced())
                    .padding(SpectraLayout.Space.m).spectraInputFieldStyle()
                Text(AppLocalization.string(
                    "Hard (//) and soft (/) junctions, as Polkadot wallets write them. Leave it blank for the phrase's root account."
                )).font(.caption).foregroundStyle(.secondary)
            }
            .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
        }
    }
}
