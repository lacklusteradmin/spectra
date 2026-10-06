import SwiftUI
enum WalletDraftMode {
    case importExisting
    case createNew
    case editExisting
}
/// Mutation contract for `WalletImportDraft`:
///
///   * **Derived state is computed, not stored.** `selectedChains` and the
///     verdicts are read from the fields they depend on, so no field needs a
///     hook to keep them current and any field may be bound directly.
///   * **Fields with a `didSet` reshape other fields** — a created phrase's
///     length regenerates it. The phrase grid is `seedEntry`'s, which follows
///     core's verdict as words land. Chain selection goes through
///     `toggleChainSelection`, which applies the mode's one-chain rule.
@MainActor
@Observable
final class WalletImportDraft {

    var mode: WalletDraftMode = .importExisting
    var isEditingWallet: Bool { mode == .editExisting }
    var walletName: String = ""
    /// The phrase grid: typed when importing, generated when creating.
    let seedEntry = SeedPhraseEntry()
    /// The phrase as core reads it.
    var seedPhrase: String { seedEntry.phrase }
    var walletPassword: String = ""
    var walletPasswordConfirmation: String = ""
    /// An import from a raw private key rather than a phrase. Chosen on the
    /// Add Wallet page, before the chains, because it decides which chains
    /// the import can use.
    var importsPrivateKey: Bool = false
    var privateKeyInput: String = ""
    var seedDerivationPreset: SeedDerivationPreset = .standard
    var seedDerivationPaths: SeedDerivationPaths = .defaults
    // Power-user derivation overrides (Advanced Options sheet). Each field is
    // a user-entered string; blank/empty-picker means "use chain preset default".
    // These are converted to WalletDerivationOverrides at import time via
    // `resolvedDerivationOverrides`.
    var overridePassphrase: String = ""
    var overrideHmacKey: String = ""
    /// The length a created phrase is generated at: one of core's lengths.
    var selectedSeedPhraseWordCount: Int = SeedPhraseEntry.initialSlotCount {
        didSet { if selectedSeedPhraseWordCount != oldValue { regenerateSeedPhrase() } }
    }
    var isWatchOnlyMode: Bool = false
    /// The watched addresses, one per line, for the one chain a watch-only
    /// import is on.
    var watchOnlyInput: String = ""
    /// Not an address: an account xpub stands in for the whole account and
    /// plans one wallet rather than one per line. Read only on a chain that
    /// `acceptsAccountXpub`.
    var bitcoinXpubInput: String = ""
    /// Every chain ticked, in the order ticked.
    var selectedChainsStorage: [Chain] = []
    var backupVerificationWordIndices: [Int] = []
    var backupVerificationEntries: [String] = []
    /// The chains the import uses: those ticked that this mode can use, all
    /// of them or only the first where the mode allows one — editing,
    /// watch-only and private-key imports.
    var selectedChains: [Chain] {
        let usable = selectedChainsStorage.filter(offers)
        return allowsMultipleChainSelection ? usable : Array(usable.prefix(1))
    }
    /// Whether this mode's chain picker lists `chain`: a private key derives
    /// an address on only some chains, and only some chains can be watched.
    func offers(_ chain: Chain) -> Bool {
        if isPrivateKeyImportMode { return chain.derivesFromPrivateKey }
        if isWatchOnlyMode { return chain.supportsWatchOnlyImport }
        return true
    }
    var isCreateMode: Bool { mode == .createNew }
    var isPrivateKeyImportMode: Bool { mode == .importExisting && !isWatchOnlyMode && importsPrivateKey }
    var allowsMultipleChainSelection: Bool { !isEditingWallet && !isWatchOnlyMode && !isPrivateKeyImportMode }
    func isSelected(_ chain: Chain) -> Bool { selectedChainsStorage.contains(chain) }
    /// The phrase as words, as core reads them.
    var seedPhraseWords: [String] { seedEntry.verdict.words }
    /// The password as typed, or `nil` for an empty field: the one way to
    /// say "no password". Core owns what counts as a password — it ignores
    /// surrounding whitespace and refuses a blank one rather than storing the
    /// wallet unsealed — so this side does not reshape it.
    var walletPasswordInput: String? { walletPassword.isEmpty ? nil : walletPassword }
    var walletPasswordValidationError: String? {
        guard let reason = validateWalletPassword(password: walletPassword, confirmation: walletPasswordConfirmation) else { return nil }
        switch reason {
        case .tooShort(let minChars):
            return AppLocalization.format("Wallet password must be at least %lld characters, or leave it blank.", Int(minChars))
        case .confirmationMismatch: return AppLocalization.string("Wallet password confirmation does not match.")
        }
    }
    /// Core interprets exact secret input and refuses unsupported overrides.
    var resolvedDerivationOverrides: WalletDerivationOverrides {
        parseWalletDerivationInput(input: WalletDerivationInput(
            passphrase: overridePassphrase, hmacKey: overrideHmacKey))
    }
    /// The selected chains, in catalog order rather than selection order.
    var selectableDerivationChains: [Chain] {
        let selected = Set(selectedChains)
        return Chain.all.filter(selected.contains)
    }
    /// The watched addresses, one per non-blank line.
    var watchOnlyEntries: [String] {
        watchOnlyInput.split(whereSeparator: \.isNewline).map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }
    /// The watch-only inputs as core reads them, keyed by the chain the import
    /// is on — the chain core plans the wallets for. Empty outside watch-only
    /// mode or before a chain is chosen.
    var watchOnlyImportEntries: WalletImportWatchOnlyEntries {
        guard isWatchOnlyMode, let chain = selectedChains.first else {
            return WalletImportWatchOnlyEntries(byChainId: [:], bitcoinXpub: nil)
        }
        let entries = watchOnlyEntries
        let trimmedXpub = bitcoinXpubInput.trimmingCharacters(in: .whitespacesAndNewlines)
        return WalletImportWatchOnlyEntries(
            byChainId: entries.isEmpty ? [:] : [chain: entries],
            bitcoinXpub: chain.acceptsAccountXpub && !trimmedXpub.isEmpty ? trimmedXpub : nil)
    }
    /// Form completeness is view state. Domain validation remains mandatory
    /// in core's import/rename operations even when a client skips this check.
    var canImportWallet: Bool {
        if isEditingWallet { return !walletName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        guard !selectedChains.isEmpty else { return false }
        if isWatchOnlyMode {
            let entries = watchOnlyImportEntries
            return !entries.byChainId.isEmpty || entries.bitcoinXpub != nil
        }
        return isSecretComplete && (!requiresBackupVerification || isBackupVerificationComplete)
    }
    /// Whether the secret step — a seed phrase or a private key — is complete
    /// enough to move on. The one definition both the step and the submit use;
    /// a private key's single-chain rule is the selection's own.
    var isSecretComplete: Bool {
        guard !selectedChains.isEmpty else { return false }
        if isPrivateKeyImportMode {
            return isPrivateKeyHex(rawValue: privateKeyInput)
        }
        return seedEntry.verdict.checksumValid
    }
    var requiresBackupVerification: Bool { isCreateMode }
    var isBackupVerificationComplete: Bool {
        guard requiresBackupVerification else { return true }
        guard backupVerificationWordIndices.count == backupVerificationEntries.count, !backupVerificationWordIndices.isEmpty else {
            return false
        }
        let words = seedPhraseWords
        guard words.count == selectedSeedPhraseWordCount else { return false }
        for (offset, index) in backupVerificationWordIndices.enumerated() {
            guard words.indices.contains(index) else { return false }
            let expected = words[index].trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            let entered = backupVerificationEntries[offset].trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if expected != entered { return false }
        }
        return true
    }
    var backupVerificationPromptLabel: String {
        guard requiresBackupVerification else { return "" }
        if backupVerificationWordIndices.isEmpty { return AppLocalization.string("Generate a backup verification challenge to continue.") }
        return ""
    }
    func configureForNewWallet() {
        mode = .importExisting
        reset()
    }
    func configureForPrivateKeyImport() {
        mode = .importExisting
        reset()
        importsPrivateKey = true
    }
    func configureForWatchAddressesImport() {
        mode = .importExisting
        reset()
        isWatchOnlyMode = true
    }
    func configureForCreatedWallet() {
        // Reset outside create mode: resetting the word count regenerates a
        // phrase in create mode, and this generates exactly one.
        mode = .importExisting
        reset()
        mode = .createNew
        regenerateSeedPhrase()
    }
    func configureForEditing(wallet: WalletView) {
        mode = .editExisting
        reset()
        walletName = wallet.name
    }
    func reset() {
        walletName = ""
        seedEntry.reset()
        walletPassword = ""
        walletPasswordConfirmation = ""
        importsPrivateKey = false
        privateKeyInput = ""
        seedDerivationPreset = .standard
        seedDerivationPaths = .defaults
        overridePassphrase = ""
        overrideHmacKey = ""
        selectedSeedPhraseWordCount = SeedPhraseEntry.initialSlotCount
        isWatchOnlyMode = false
        watchOnlyInput = ""
        bitcoinXpubInput = ""
        selectedChainsStorage = []
        backupVerificationWordIndices = []
        backupVerificationEntries = []
    }
    func toggleChainSelection(_ chain: Chain) { setSelectedChain(chain, isEnabled: !isSelected(chain)) }
    private func setSelectedChain(_ chain: Chain, isEnabled: Bool) {
        if isEnabled {
            if allowsMultipleChainSelection {
                if !selectedChainsStorage.contains(chain) { selectedChainsStorage.append(chain) }
            } else {
                selectedChainsStorage = [chain]
            }
        } else {
            selectedChainsStorage.removeAll { $0 == chain }
        }
    }
    func regenerateSeedPhrase() {
        guard isCreateMode else { return }
        backupVerificationWordIndices = []
        backupVerificationEntries = []
        // The length is one of core's, so generating it cannot be refused;
        // if it were, the grid shows no phrase rather than a guessed one.
        let generatedPhrase = (try? generateMnemonic(wordCount: UInt32(selectedSeedPhraseWordCount))) ?? ""
        let generatedWords = generatedPhrase.lowercased().split(whereSeparator: \.isWhitespace).map(String.init)
        seedEntry.load(generatedWords, wordCount: selectedSeedPhraseWordCount)
    }
    func prepareBackupVerificationChallenge() {
        guard requiresBackupVerification else {
            backupVerificationWordIndices = []
            backupVerificationEntries = []
            return
        }
        let words = seedPhraseWords
        guard words.count == selectedSeedPhraseWordCount else {
            backupVerificationWordIndices = []
            backupVerificationEntries = []
            return
        }
        var indices: Set<Int> = []
        while indices.count < min(3, selectedSeedPhraseWordCount) {
            indices.insert(Int.random(in: 0..<selectedSeedPhraseWordCount))
        }
        let sortedIndices = indices.sorted()
        backupVerificationWordIndices = sortedIndices
        backupVerificationEntries = Array(repeating: "", count: sortedIndices.count)
    }
    func updateBackupVerificationEntry(at index: Int, with value: String) {
        guard backupVerificationEntries.indices.contains(index) else { return }
        backupVerificationEntries[index] = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }
}
