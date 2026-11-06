import Foundation

// Localized display messages. AmountPresentation owns native number rendering.

extension EvmRecipientPreflightWarning {
    /// The warning, worded. Exhaustive, with no `default`: a reason core adds
    /// does not compile until it has words.
    var localizedText: String {
        switch self {
        case .recipientIsContract(let chain, let symbol):
            AppLocalization.format(
                "Recipient is a smart contract on %@. Confirm it can receive %@ safely.", chain.displayName, symbol)
        case .recipientCodeUnknown(let chain):
            AppLocalization.format(
                "Could not verify recipient contract state on %@. Review destination carefully.", chain.displayName)
        case .tokenContractMissing(let chain, let tokenSymbol):
            AppLocalization.format(
                "Token contract %@ appears missing on %@. This may be a wrong-network token selection.",
                tokenSymbol, chain.displayName)
        case .tokenCodeUnknown(let chain, let tokenSymbol):
            AppLocalization.format("Could not verify %@ contract bytecode on %@.", tokenSymbol, chain.displayName)
        }
    }
}

extension HighRiskSendWarning {
    /// The reason, worded. Exhaustive for the same reason as
    /// `EvmRecipientPreflightWarning.localizedText`.
    var localizedText: String {
        switch self {
        case .invalidFormat(let chain):
            AppLocalization.format("The destination address format does not match %@.", chain.displayName)
        case .newAddress:
            AppLocalization.string("This is a new destination address with no prior history in this wallet.")
        case .ensResolved(let name, let address):
            AppLocalization.format(
                "ENS name '%@' resolved to %@. Confirm this resolved address before sending.", name, address)
        case .largeSend(let percent, let symbol):
            AppLocalization.format(
                "This send is %@ of your %@ balance.",
                (Double(percent) / 100.0).formatted(.percent.precision(.fractionLength(0)).locale(AppLocalization.locale)),
                symbol)
        case .nonEvmOnEvm(let chain):
            AppLocalization.format("Destination appears to be a non-EVM address while sending on %@.", chain.displayName)
        case .ensOffEthereum(let chain):
            AppLocalization.format(
                "ENS names are Ethereum-specific. For %@, verify the resolved EVM address very carefully.", chain.displayName)
        case .ethOnUtxo(let chain):
            AppLocalization.format("Destination appears to be an Ethereum-style address while sending on %@.", chain.displayName)
        case .foreignAddressFormat(let chain):
            AppLocalization.format("Destination appears to be another network's address format while sending on %@.", chain.displayName)
        case .chainMismatch:
            AppLocalization.string("Wallet-chain context mismatch detected for this send.")
        }
    }
}

extension SendArtifact {
    /// What core's review says before this transaction is signed, worded:
    /// the same list wherever an artifact is reviewed. Exhaustive, so a new
    /// warning does not compile until it has words.
    @MainActor
    var reviewWarningMessages: [String] {
        review.warnings.map { warning in
            switch warning {
            case .risk(let warning): warning.localizedText
            case .contract(let warning): warning.localizedText
            case .ownAddress:
                AppLocalization.string("This destination belongs to your wallet. Confirm intentional self-send.")
            case .tokenFee(let fee, let received):
                AppLocalization.format(
                    "The token takes %@ %@ of this amount on the way; the recipient receives %@ %@.",
                    AmountPresentation.localizedDecimal(fee), symbol,
                    AmountPresentation.localizedDecimal(received), symbol)
            case .carriesNative(let amount):
                AppLocalization.format(
                    "The token goes with %@ %@, the least its output can hold; the recipient keeps it.",
                    AmountPresentation.localizedDecimal(amount), chainId.gasTokenSymbol)
            case .registersRecipient(let deposit):
                AppLocalization.format(
                    "The token has not registered the recipient. This send registers it first for %@ %@, which the token keeps for the recipient.",
                    AmountPresentation.localizedDecimal(deposit), chainId.gasTokenSymbol)
            case .transferHook(let program):
                AppLocalization.format("This token runs program %@ on every transfer, and it can refuse this one.", program)
            }
        }
    }
}

/// Localized title and message for a destination verdict.
func chainRiskProbeMessages(chainName: String, symbol: String, activity: SendDestinationActivity) -> (
    warning: String?, info: String?
) {
    switch activity {
    case .unused:
        return (AppLocalization.format(
            "Warning: this %@ address has zero %@ balance and no transaction history. Double-check recipient details.",
            chainName, symbol), nil)
    case .emptyPreviouslyUsed:
        return (nil, AppLocalization.format(
            "Note: this %@ address has transaction history but currently zero %@ balance.", chainName, symbol))
    case .funded: return (nil, nil)
    }
}
extension LocalizableMessage {
    /// Core's sentence in the reader's language: its English template is the
    /// key into the string tables, and its values go in where the template
    /// says. Text with no table entry — a node's or a library's own words —
    /// reads as core sent it.
    var localizedText: String {
        let text = args.isEmpty ? AppLocalization.string(template) : AppLocalization.format(template, arguments: args)
        return text.prefix(1).uppercased() + text.dropFirst()
    }
}

/// What the user reads when a call fails. UniFFI words its errors as their
/// Swift debug description — type, case and field names — so the
/// `localizedDescription` of a bridge error never belongs on screen. Core's
/// sentences for refused input and failed requests are translated here; a
/// network or decoding failure carries a transport or parser message meant
/// for the operational log, so it gets a fixed sentence instead.
func userErrorMessage(_ error: Error) -> String {
    if let error = error as? SpectraBridgeError {
        switch error {
        case .InvalidInput(let message), .Failure(let message):
            return message.localizedText
        case .Network:
            return AppLocalization.string("Couldn't reach the network. Check your connection and try again.")
        case .Decode:
            return AppLocalization.string("Spectra received data it couldn't read. Try again later.")
        case .StoreUnreadable:
            return AppLocalization.string("content.unreadable.subtitle")
        }
    }
    let description = error.localizedDescription
    // Every other UniFFI error is worded the same way and has no sentence.
    return description == String(reflecting: error)
        ? AppLocalization.string("Something went wrong. Try again.") : description
}

extension Date {
    /// This date in the app's language and the reader's region. The words
    /// around a date are in the app's language, which need not be the
    /// system's, so the date's month names and order are too.
    func appFormatted(time: Date.FormatStyle.TimeStyle) -> String {
        formatted(Date.FormatStyle(date: .abbreviated, time: time).locale(AppLocalization.locale))
    }
}
