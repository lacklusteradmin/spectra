import SwiftUI
/// What a draft is for: adding a wallet one way on one network, or renaming
/// one. The network and the method are chosen before the form opens.
enum WalletDraftMode: Equatable {
    case setup(WalletSetupMethod)
    case edit
}
/// Mutation contract for `WalletImportDraft`:
///
///   * **Derived state is computed, not stored.** `chain` and the
///     verdicts are read from the fields they depend on, so no field needs a
///     hook to keep them current and any field may be bound directly.
///   * **Fields with a `didSet` reshape other fields** — a created phrase's
///     length regenerates it. The phrase grid is `seedEntry`'s, which follows
///     core's verdict as words land.
@MainActor
@Observable
final class WalletImportDraft {

    var mode: WalletDraftMode = .setup(.importPhrase)
    /// The network the wallet is added on, chosen before the form opens.
    var chain: Chain?
    /// The watched wallet this form gives its keys to, when it was opened
    /// from that wallet's page. Core refuses a secret that does not hold its
    /// address rather than adding a wallet.
    var upgradeWalletId: String?
    var isEditingWallet: Bool { mode == .edit }
    var method: WalletSetupMethod? {
        if case .setup(let method) = mode { return method }
        return nil
    }
    var walletName: String = ""
    /// The phrase grid: typed when importing, generated when creating.
    let seedEntry = SeedPhraseEntry()
    /// The phrase as core reads it.
    var seedPhrase: String { seedEntry.phrase }
    var walletPassword: String = ""
    var walletPasswordConfirmation: String = ""
    var privateKeyInput: String = ""
    /// The derivation profile a phrase wallet uses: one of the network's, its
    /// first by default. `nil` on a network that derives without a path.
    var derivationProfile: DerivationProfile?
    /// The account index on the profile.
    var derivationAccount: UInt32 = 0
    /// A path typed under Advanced. When set it replaces the profile's.
    var customDerivationPath: String = ""
    // Power-user derivation overrides (Advanced Options sheet). Each field is
    // a user-entered string; blank/empty-picker means "use chain preset default".
    // These are converted to WalletDerivationOverrides at import time via
    // `resolvedDerivationOverrides`.
    var overridePassphrase: String = ""
    var overrideHmacKey: String = ""
    /// The length a created phrase is generated at: one of `createdLengths`.
    var selectedSeedPhraseWordCount: Int = 12 {
        didSet { if selectedSeedPhraseWordCount != oldValue { regenerateSeedPhrase() } }
    }
    /// A restored Monero wallet's restore height as typed; blank lets core
    /// read a Polyseed's birthday or scan a 25-word seed from the start.
    var restoreHeightInput: String = ""
    /// The typed restore height, or `nil` when blank. Not a number is not a
    /// height, and is refused by `isSecretComplete`.
    var restoreHeight: UInt64? { UInt64(restoreHeightInput.trimmingCharacters(in: .whitespaces)) }
    var isRestoreHeightValid: Bool {
        restoreHeightInput.trimmingCharacters(in: .whitespaces).isEmpty || restoreHeight != nil
    }
    /// This method's entry in the network's setup descriptor.
    private var setupOption: WalletSetupOption? {
        guard let chain, let method else { return nil }
        return walletSetupDescriptor(chain: chain).options.first { $0.method == method }
    }
    /// Whether this method on this network asks for a restore height.
    var asksRestoreHeight: Bool { setupOption?.fields.contains(.restoreHeight) ?? false }
    /// A named account the key controls, as typed; blank keeps the key's
    /// implicit account.
    var namedAccountInput: String = ""
    /// Whether this method on this network takes a named account.
    var asksNamedAccount: Bool { setupOption?.fields.contains(.namedAccount) ?? false }
    /// The wallet contract a TON key's account is under.
    var tonWalletVersion: TonWalletVersion = .w5
    /// Whether this method on this network asks which wallet version to hold.
    var asksTonWalletVersion: Bool { setupOption?.fields.contains(.tonWalletVersion) ?? false }
    /// The profiles this method offers on the network, default first; empty
    /// where the network derives without a path.
    var derivationProfiles: [DerivationProfile] { setupOption?.profiles ?? [] }
    /// The profile's path at the chosen account, as core renders it.
    var profileDerivationPath: String? {
        guard let chain, let derivationProfile else { return nil }
        return try? derivationProfilePath(chain: chain, profile: derivationProfile, account: derivationAccount)
    }
    /// The path the import derives along: the custom one when typed, else the
    /// profile's. Core refuses one that does not parse.
    var derivationPath: String? {
        let custom = customDerivationPath.trimmingCharacters(in: .whitespacesAndNewlines)
        return custom.isEmpty ? profileDerivationPath : custom
    }
    /// The watched addresses, one per line, on the import's chain.
    var watchOnlyInput: String = ""
    /// Not an address: an account xpub stands in for the whole account and
    /// imports one wallet rather than one per line.
    var accountXpubInput: String = ""
    var backupVerificationWordIndices: [Int] = []
    var backupVerificationEntries: [String] = []
    var isCreateMode: Bool { method == .createPhrase }
    var isPrivateKeyImportMode: Bool { method == .importPrivateKey }
    var isWatchOnlyMode: Bool { method == .watchAddresses || method == .watchAccountXpub }
    /// The pages this draft's form walks through.
    var setupFlow: SetupFlow {
        switch mode {
        case .edit: .editWallet
        case .setup(let method): .forMethod(method)
        }
    }
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
    /// The watched addresses, one per non-blank line.
    var watchOnlyEntries: [String] {
        watchOnlyInput.split(whereSeparator: \.isNewline).map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }
    /// How core reads this import. Creating and restoring a phrase are one
    /// import: core does not need to know the phrase was just generated.
    var importKind: WalletImportKind {
        switch method {
        case .importPrivateKey: .privateKey
        case .watchAddresses: .watchAddresses(addresses: watchOnlyEntries)
        case .watchAccountXpub: .watchAccountXpub(xpub: accountXpubInput.trimmingCharacters(in: .whitespacesAndNewlines))
        case .createPhrase, .importPhrase, nil: .phrase
        }
    }
    /// The commit core imports from, built from the form as it stands. The
    /// one place the form becomes a request, so whatever reads it before the
    /// import sees what the import will.
    func importCommit(name: String) -> WalletImportCommit? {
        guard let chain else { return nil }
        let kind = importKind
        // A new Monero wallet scans from near now; a restored one from the
        // height typed, if any.
        let restoreHeight: UInt64? =
            isCreateMode ? newWalletRestoreHeight(chain: chain) : self.restoreHeight
        return WalletImportCommit(
            password: walletPasswordInput,
            request: WalletImportRequest(walletName: name, chain: chain, kind: kind),
            derivationPath: kind == .phrase ? derivationPath : nil,
            derivationOverrides: resolvedDerivationOverrides,
            seedPhrase: kind == .phrase ? seedPhrase : nil,
            privateKey: kind == .privateKey ? privateKeyInput : nil,
            restoreHeight: restoreHeight,
            namedAccount: asksNamedAccount && !namedAccountInput.trimmingCharacters(in: .whitespaces).isEmpty
                ? namedAccountInput : nil,
            tonWalletVersion: asksTonWalletVersion ? tonWalletVersion : nil,
            upgradeWalletId: upgradeWalletId)
    }
    /// The commit an address preview reads: the form's, without the name and
    /// password, which change no address. `nil` until there is enough to
    /// derive from — a complete secret, or a line or key to watch.
    var previewCommit: WalletImportCommit? {
        switch importKind {
        case .watchAddresses(let addresses): guard !addresses.isEmpty else { return nil }
        case .watchAccountXpub(let xpub): guard !xpub.isEmpty else { return nil }
        case .phrase, .privateKey: guard isSecretComplete else { return nil }
        }
        guard var commit = importCommit(name: "") else { return nil }
        commit.password = nil
        return commit
    }
    /// Form completeness is view state. Domain validation remains mandatory
    /// in core's import/rename operations even when a client skips this check.
    var canImportWallet: Bool {
        if isEditingWallet { return !walletName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        guard chain != nil else { return false }
        switch importKind {
        case .watchAddresses(let addresses): return !addresses.isEmpty
        case .watchAccountXpub(let xpub): return !xpub.isEmpty
        case .phrase, .privateKey: break
        }
        return isSecretComplete && (!requiresBackupVerification || isBackupVerificationComplete)
    }
    /// Whether the secret step — a seed phrase or a private key — is complete
    /// enough to move on. The one definition both the step and the submit use.
    var isSecretComplete: Bool {
        guard chain != nil else { return false }
        if isPrivateKeyImportMode {
            guard let chain else { return false }
            return isValidPrivateKey(chain: chain, rawValue: privateKeyInput)
        }
        return seedEntry.verdict.isValid && isRestoreHeightValid
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
    /// An empty form, on no network.
    func clear() {
        // Reset outside create mode: resetting the word count regenerates a
        // phrase in create mode.
        mode = .setup(.importPhrase)
        reset()
    }
    /// A fresh form for adding a wallet on `chain` by `method`. Creating
    /// generates exactly one phrase.
    func configure(chain: Chain, method: WalletSetupMethod, upgrading walletId: String? = nil) {
        clear()
        self.chain = chain
        upgradeWalletId = walletId
        // The grid judges the phrase in the network's own formats.
        seedEntry.chain = chain
        if let shortest = createdLengths.first { selectedSeedPhraseWordCount = Int(shortest.wordCount) }
        mode = .setup(method)
        derivationProfile = derivationProfiles.first
        if method == .createPhrase { regenerateSeedPhrase() }
    }
    /// The lengths a phrase created on the network can have: those of the
    /// format its setup descriptor creates in — BIP-39's five, Monero's 25,
    /// TON's 24.
    var createdLengths: [SeedPhraseLength] {
        guard let chain else { return [] }
        let created = walletSetupDescriptor(chain: chain).options.first { $0.method == .createPhrase }?.formats.first
        return seedEntry.lengths.filter { $0.format == created }
    }
    func configureForEditing(wallet: WalletView) {
        clear()
        mode = .edit
        walletName = wallet.name
    }
    func reset() {
        upgradeWalletId = nil
        walletName = ""
        seedEntry.reset()
        walletPassword = ""
        walletPasswordConfirmation = ""
        privateKeyInput = ""
        derivationProfile = nil
        derivationAccount = 0
        customDerivationPath = ""
        overridePassphrase = ""
        overrideHmacKey = ""
        selectedSeedPhraseWordCount = 12
        watchOnlyInput = ""
        accountXpubInput = ""
        restoreHeightInput = ""
        namedAccountInput = ""
        tonWalletVersion = .w5
        chain = nil
        seedEntry.chain = nil
        backupVerificationWordIndices = []
        backupVerificationEntries = []
    }
    func regenerateSeedPhrase() {
        guard isCreateMode, let chain else { return }
        backupVerificationWordIndices = []
        backupVerificationEntries = []
        // The length is one of core's for the network's created format, so
        // generating it cannot be refused; if it were, the grid shows no
        // phrase rather than a guessed one.
        let generatedPhrase =
            (try? generateSeedPhrase(chain: chain, wordCount: UInt32(selectedSeedPhraseWordCount))) ?? ""
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
