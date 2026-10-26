import Foundation

/// Ordered wallet-setup pages. Navigation and the step indicator derive
/// from this list.
struct SetupFlow {
    let pages: [WalletSetupPage]

    /// Linear index of `page` within this flow, or `nil` for a page this
    /// flow does not visit.
    func index(of page: WalletSetupPage) -> Int? {
        pages.firstIndex(of: page)
    }

    /// Page to advance to from `current`, or `nil` if already at the end
    /// (indicating the primary action should submit instead of route).
    func next(after current: WalletSetupPage) -> WalletSetupPage? {
        guard let i = index(of: current), i + 1 < pages.count else { return nil }
        return pages[i + 1]
    }

}

/// Linear pages for the wallet-setup flow. Lifted out of `SetupView`'s
/// private enum so `SetupFlow` can reference it.
enum WalletSetupPage: Hashable {
    case watchAddresses
    case seedPhrase
    case backupVerification
    /// The name, and the optional password a wallet with a secret may take.
    case walletName
}

extension SetupFlow {
    /// The pages for adding a wallet by `method`. The network was chosen
    /// before the form opened, so no flow asks for it.
    static func forMethod(_ method: WalletSetupMethod) -> SetupFlow {
        // A new phrase is checked straight after it is written down, while
        // the paper is still in hand; the name and password come last.
        switch method {
        case .createPhrase: SetupFlow(pages: [.seedPhrase, .backupVerification, .walletName])
        case .importPhrase, .importPrivateKey: SetupFlow(pages: [.seedPhrase, .walletName])
        case .watchAddresses, .watchAccountXpub, .watchViewKey, .watchMultisig:
            SetupFlow(pages: [.watchAddresses, .walletName])
        }
    }

    /// Edit existing wallet — single-page (just the name field).
    static let editWallet = SetupFlow(pages: [.walletName])
}

/// What a page calls itself: the heading and the line under it.
struct WalletSetupPageCopy {
    let title: String
    let subtitle: String
}

extension WalletSetupPage {
    /// Page wording. Exhaustive so every new page must supply a title.
    ///
    /// The page alone does not decide what it is called — "seed phrase" reads
    /// as *record* one when creating a wallet and *enter* one when importing —
    /// so the copy resolver takes the mode alongside the page.
    func copy(_ content: ImportFlowContent, mode: WalletDraftMode) -> WalletSetupPageCopy {
        switch self {
        case .walletName:
            if mode == .edit {
                return WalletSetupPageCopy(title: content.editWalletTitle, subtitle: content.editWalletSubtitle)
            }
            if mode.takesWalletPassword {
                return WalletSetupPageCopy(
                    title: AppLocalization.string("import_flow.name_your_wallet"),
                    subtitle: AppLocalization.string("import_flow.name_and_password_hint"))
            }
            return WalletSetupPageCopy(
                title: AppLocalization.string("import_flow.name_your_wallet"),
                subtitle: AppLocalization.string("import_flow.wallet_name_hint"))
        case .backupVerification:
            return WalletSetupPageCopy(
                title: content.backupVerificationTitle, subtitle: content.backupVerificationSubtitle)
        case .watchAddresses:
            if mode == .setup(.watchAccountXpub) {
                return WalletSetupPageCopy(
                    title: AppLocalization.string("Watch Account"),
                    subtitle: AppLocalization.string("Enter the account's extended public key."))
            }
            if mode == .setup(.watchMultisig) {
                return WalletSetupPageCopy(
                    title: AppLocalization.string("Watch Multisig"),
                    subtitle: AppLocalization.string("Enter the account's descriptor: wsh(sortedmulti(…)) with each key's origin."))
            }
            if mode == .setup(.watchViewKey) {
                return WalletSetupPageCopy(
                    title: AppLocalization.string("Watch with View Key"),
                    subtitle: AppLocalization.string("Enter the wallet's primary address and its private view key."))
            }
            return WalletSetupPageCopy(
                title: content.watchAddressesTitle, subtitle: content.watchAddressesSubtitle)
        case .seedPhrase:
            if mode == .setup(.importPrivateKey) {
                return WalletSetupPageCopy(
                    title: content.enterPrivateKeyTitle, subtitle: content.privateKeySubtitle)
            }
            let isCreating = mode == .setup(.createPhrase)
            return WalletSetupPageCopy(
                title: isCreating ? content.recordSeedPhraseTitle : content.enterSeedPhraseTitle,
                subtitle: isCreating ? content.saveRecoveryPhraseSubtitle : content.enterRecoveryPhraseSubtitle)
        }
    }
}

extension WalletDraftMode {
    /// A wallet added with a phrase or a key may seal it under a password;
    /// a watched one has nothing to seal, and a rename changes no secret.
    var takesWalletPassword: Bool {
        switch self {
        case .setup(.createPhrase), .setup(.importPhrase), .setup(.importPrivateKey): true
        case .setup, .edit: false
        }
    }
}
