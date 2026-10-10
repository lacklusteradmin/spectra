import SwiftUI

/// Everything about a seed wallet that has a working default: a new phrase's
/// length, the account and path it derives, and the passphrase and HMAC
/// overrides. A sheet over the secret step rather than a page of the setup
/// flow, so the linear flow never routes through it and most people never
/// open it.
struct WalletDerivationOptionsView: View {
    let store: AppState
    @Bindable var draft: WalletImportDraft
    @Environment(\.dismiss) private var dismiss
    private let copy = ImportFlowContent.current
    var body: some View {
        NavigationStack {
            ScrollView(showsIndicators: false) {
                if !draft.isCreateMode {
                    SeedPhraseReadingSection(entry: draft.seedEntry)
                        .padding([.horizontal, .top], SpectraLayout.Space.l)
                } else {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                        NewPhraseLengthSection(draft: draft)
                        DerivationAccountCard(draft: draft)
                        JunctionPathCard(draft: draft)
                    }
                    .padding([.horizontal, .top], SpectraLayout.Space.l)
                }
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    Text(AppLocalization.string("Control the derivation path this wallet uses."))
                        .font(.subheadline).foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                        // Starts at the chosen profile's path; an edit makes it
                        // custom, and Reset returns to the profile.
                        if let profilePath = draft.profileDerivationPath {
                            SeedPathSlotEditor(
                                title: "Custom Path",
                                path: Binding(
                                    get: { draft.derivationPath ?? profilePath },
                                    set: { draft.customDerivationPath = $0 == profilePath ? "" : $0 }
                                ), defaultPath: profilePath
                            )
                        }
                        PowerUserOverridesSection(draft: draft)
                    }
                }.padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill().padding(SpectraLayout.Space.l)
            }
            .navigationTitle(copy.advancedTitle).navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(AppLocalization.string("Done")) { dismiss() }
                }
            }
        }
    }
}

extension SeedPhraseLanguage: Identifiable {
    public var id: String { code }
}

/// How an import reads its phrase: the length and the wordlist, each
/// inferred by core unless fixed here. A length is one BIP-39 defines: no
/// other has a checksum that can hold.
private struct SeedPhraseReadingSection: View {
    @Bindable var entry: SeedPhraseEntry
    private let columns = Array(repeating: GridItem(.flexible(), spacing: SpectraLayout.Space.xs), count: 3)
    private var wordlists: [SeedPhraseLanguage] { seedPhraseLanguages(chain: entry.chain) }

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.l) {
            wordCount
            // Only BIP-39's lists overlap enough to need choosing; Monero's
            // and Polyseed's are told apart by their words.
            if !wordlists.isEmpty {
                Divider().opacity(0.4)
                wordlist
            }
        }
        .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
    }

    private var wordCount: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            Text(AppLocalization.string("Word Count")).font(.subheadline.weight(.semibold))
            LazyVGrid(columns: columns, spacing: SpectraLayout.Space.xs) {
                chip(AppLocalization.string("Auto"), isSelected: entry.wordCountOverride == nil) {
                    entry.wordCountOverride = nil
                }
                ForEach(entry.lengths, id: \.wordCount) { length in
                    let count = Int(length.wordCount)
                    chip("\(count)", isSelected: entry.wordCountOverride == count) {
                        entry.wordCountOverride = count
                    }
                }
            }
            Text(AppLocalization.string("import_flow.word_count_footer")).font(.caption).foregroundStyle(.secondary)
        }
    }

    private var wordlist: some View {
        let detected = entry.verdict.language.map { AppLocalization.string($0.name) }
        let autoTitle =
            detected.map { AppLocalization.format("import_flow.wordlist_auto_detected_format", $0) }
            ?? AppLocalization.string("Auto-detect")
        return VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            HStack {
                Text(AppLocalization.string("Wordlist")).font(.subheadline.weight(.semibold))
                Spacer()
                Picker(AppLocalization.string("Wordlist"), selection: $entry.language) {
                    Text(autoTitle).tag(String?.none)
                    ForEach(wordlists) { language in
                        Text(AppLocalization.string(language.name)).tag(String?.some(language.code))
                    }
                }
                .pickerStyle(.menu)
                .tint(.secondary)
            }
            Text(AppLocalization.string("import_flow.wordlist_footer")).font(.caption).foregroundStyle(.secondary)
        }
    }

    private func chip(_ title: String, isSelected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.subheadline.weight(.semibold).monospacedDigit())
                .foregroundStyle(isSelected ? Color.white : Color.primary)
                .frame(maxWidth: .infinity, minHeight: 36)
                .spectraSelectableFill(isSelected: isSelected, accent: .accentColor, cornerRadius: SpectraLayout.Radius.control)
        }
        .buttonStyle(.plain)
    }
}

/// The passphrase and HMAC overrides, below the per-chain paths.
private struct PowerUserOverridesSection: View {
    @Bindable var draft: WalletImportDraft
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            header
            stage1Overrides
        }.padding(SpectraLayout.Space.m).background(
            RoundedRectangle(cornerRadius: SpectraLayout.Radius.inner, style: .continuous).fill(Color.spectraWarning.opacity(0.08))
        ).overlay(
            RoundedRectangle(cornerRadius: SpectraLayout.Radius.inner, style: .continuous).stroke(Color.spectraWarning.opacity(0.35), lineWidth: 1)
        )
    }
    private var header: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            HStack(spacing: SpectraLayout.Space.s) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.caption.weight(.bold)).foregroundStyle(.spectraWarning)
                Text(AppLocalization.string("Power-User Overrides"))
                    .font(.subheadline.weight(.semibold)).foregroundStyle(Color.primary)
            }
            Text(
                AppLocalization.string(
                    "Secret text is used exactly as entered, including spaces. Unsupported chain overrides are refused. Leave blank to use the chain default."
                )
            ).font(.caption).foregroundStyle(.spectraWarning.opacity(0.9))
        }
    }
    private var stage1Overrides: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            AdvancedOverrideTextField(
                title: AppLocalization.string("Passphrase"),
                detail: AppLocalization.string("BIP-39 passphrase (“25th word”). Blank = none."),
                text: $draft.overridePassphrase, isSecure: true)
            // A new wallet's passphrase is typed twice: a typo in it makes a
            // wallet no one can restore, and nothing else would catch it.
            if draft.isCreateMode, !draft.overridePassphrase.isEmpty {
                AdvancedOverrideTextField(
                    title: AppLocalization.string("Confirm Passphrase"),
                    detail: AppLocalization.string("Type the passphrase again, exactly."),
                    text: $draft.overridePassphraseConfirmation, isSecure: true)
                if let error = draft.passphraseConfirmationError {
                    Label(error, systemImage: "exclamationmark.triangle.fill").font(.caption).foregroundStyle(.red)
                }
            }
            AdvancedOverrideTextField(
                title: AppLocalization.string("HMAC Master Key"),
                detail: AppLocalization.string(
                    "Custom master HMAC key for supported chains. Blank uses the chain default."),
                text: $draft.overrideHmacKey)
        }
    }
}

private struct AdvancedOverrideTextField: View {
    let title: String
    let detail: String
    @Binding var text: String
    var isSecure: Bool = false
    var keyboard: UIKeyboardType = .default
    /// A secret field can be shown: its spaces count, and a hidden field
    /// hides them.
    @State private var isRevealed = false
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Text(title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            HStack(spacing: SpectraLayout.Space.s) {
                inputField.font(.subheadline.monospaced())
                if isSecure {
                    Button { isRevealed.toggle() } label: {
                        Image(systemName: isRevealed ? "eye.slash" : "eye").font(.subheadline)
                            .frame(minWidth: 32, minHeight: 32)
                    }
                    .buttonStyle(.plain).foregroundStyle(.secondary)
                    .accessibilityLabel(AppLocalization.string(isRevealed ? "Hide" : "Show"))
                }
            }
            .padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.s)
            .spectraInsetFill(cornerRadius: SpectraLayout.Radius.control)
            .overlay(
                RoundedRectangle(cornerRadius: SpectraLayout.Radius.control, style: .continuous).stroke(Color.primary.opacity(0.1), lineWidth: 1))
            Text(detail).font(.caption2).foregroundStyle(.secondary)
        }
    }
    @ViewBuilder
    private var inputField: some View {
        if isSecure && !isRevealed {
            SecureField(AppLocalization.string("(default)"), text: $text)
        } else if isSecure {
            TextField(AppLocalization.string("(default)"), text: $text)
                .textInputAutocapitalization(.never).autocorrectionDisabled().privacySensitive()
        } else {
            TextField(AppLocalization.string("(default)"), text: $text)
                .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(keyboard)
        }
    }
}

/// A new phrase's length, as core offers the network's created format, and a
/// fresh phrase at it. Changing either replaces the phrase on the page.
private struct NewPhraseLengthSection: View {
    @Bindable var draft: WalletImportDraft

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            HStack(alignment: .firstTextBaseline) {
                Text(AppLocalization.string("Phrase Length")).font(.subheadline.weight(.semibold))
                Spacer()
                Button {
                    draft.regenerateSeedPhrase()
                } label: {
                    Label(AppLocalization.string("Regenerate"), systemImage: "arrow.clockwise").font(.caption.weight(.semibold))
                }.buttonStyle(.glass).tint(.accentColor)
            }
            // A network whose created format has one length — Monero's 25,
            // TON's 24 — has nothing to choose.
            if draft.createdLengths.count > 1 {
                HStack(spacing: SpectraLayout.Space.xs) {
                    ForEach(draft.createdLengths, id: \.wordCount) { length in
                        let count = Int(length.wordCount)
                        let isSelected = draft.selectedSeedPhraseWordCount == count
                        Button {
                            draft.selectedSeedPhraseWordCount = count
                        } label: {
                            Text(AppLocalization.format("%lld words", count: count, count))
                                .font(.subheadline.weight(.semibold).monospacedDigit())
                                .foregroundStyle(isSelected ? Color.white : Color.primary)
                                .lineLimit(1).minimumScaleFactor(0.8)
                                .frame(maxWidth: .infinity, minHeight: 44)
                                .spectraSelectableFill(isSelected: isSelected, accent: .accentColor, cornerRadius: SpectraLayout.Radius.inner)
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
                    }
                }
            }
            Text(AppLocalization.string("Twelve words are enough for most wallets; longer phrases are as safe and longer to write down."))
                .font(.caption).foregroundStyle(.secondary)
        }
        .padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
    }
}
