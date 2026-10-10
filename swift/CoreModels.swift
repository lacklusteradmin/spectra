import Foundation
import SwiftUI
#if canImport(UIKit)
    import UIKit
#endif
extension SendPreviewDetails {
    /// Whether the preview says anything the fee rows and the amount page
    /// have not: an EVM fee rate is already rows of its own, and the balance
    /// and maximum are the amount page's.
    func hasDetailRows(isEVM: Bool) -> Bool {
        (!isEVM && feeRateDescription != nil)
            || estimatedTransactionBytes != nil
            || selectedInputCount != nil
            || usesChangeOutput != nil
    }
}
/// A holding's `id` is the deployment id core derives; every projection
/// this app reads carries it.
extension AssetHolding: Identifiable {
    var color: Color { AssetPresentationCatalog.color(deploymentId: id) }
    var holdingKey: String { id }
    var chain: Chain { chainId }
    /// For text a person reads; identity is `chainId`.
    var chainName: String { chainId.displayName }
    var isEVMChain: Bool { chain.isEVM }
    /// The chain's own asset — `ETH` on Arbitrum, not `ARB` — by deployment
    /// identity, which the catalog names for each chain.
    var isNativeCoin: Bool { chain.entry?.nativeDeploymentId == id }
    /// Whether anything is held. Core stores amounts in canonical spelling,
    /// so zero is always `"0"`.
    var hasBalance: Bool { amount != "0" }
}
extension AssetWikiPlace {
    var chainName: String { chainId.displayName }
}
extension FundsFinderCandidate {
    var chainName: String { chainId.displayName }
}
extension DiagnosticLogInput {
    var chainName: String? { chainId?.displayName }
}
extension WalletView: Identifiable {}
extension WalletView {
    /// This wallet's address on a chain. Slot resolution (including "every
    /// EVM chain shares Ethereum's") lives in the Rust registry.
    func address(on chain: Chain) -> String? {
        let slot = chain.addressSlot
        guard !slot.isEmpty else { return nil }
        return addresses[slot]
    }
    /// The network this wallet is on.
    var chain: Chain { chainId }
    /// The mainnet whose family this wallet belongs to.
    var family: Chain { chain.mainnetCounterpart }
    /// The family's name, for text a person reads.
    var familyName: String { family.displayName }
    /// Whether the user hid `holding` from this wallet's total.
    func hides(_ holding: AssetHolding) -> Bool { hiddenHoldings.contains(holding.holdingKey) }
    /// The holdings with a balance the wallet shows and counts.
    var shownHoldings: [AssetHolding] { holdings.filter { $0.hasBalance && !hides($0) } }
}

extension DerivationProfile {
    /// The profile's name. A Bitcoin-family profile is its script type.
    var title: String {
        switch self {
        case .standard: AppLocalization.string("Standard")
        case .legacy: AppLocalization.string("Legacy")
        case .nestedSegWit: AppLocalization.string("Nested SegWit")
        case .nativeSegWit: AppLocalization.string("Native SegWit")
        case .taproot: AppLocalization.string("Taproot")
        }
    }
}
extension TonWalletVersion: CaseIterable {
    public static var allCases: [TonWalletVersion] { [.w5, .v4r2] }
    /// The version's name as TON wallets print it.
    var title: String {
        switch self {
        case .w5: AppLocalization.string("W5")
        case .v4r2: AppLocalization.string("v4R2")
        }
    }
}
extension FundsFinderCandidate {
    /// The profile and account the address is on, the wallet version on TON,
    /// or nothing for a network that derives one account.
    var profileTitle: String? {
        if let tonWalletVersion { return tonWalletVersion.title }
        return profile.map { AppLocalization.format("derivation.profile_account_format", $0.title, Int(account)) }
    }
}
extension TransactionStatus {
    var localizedTitle: String {
        switch self {
        case .pending: return AppLocalization.string("Pending")
        case .confirmed: return AppLocalization.string("Confirmed")
        case .failed: return AppLocalization.string("Failed")
        }
    }
}
/// Picker order and wording for core's history filter.
extension HistoryQueryFilter: CaseIterable, Identifiable {
    public static var allCases: [HistoryQueryFilter] { [.all, .send, .receive, .pending] }
    public var id: Self { self }
    var localizedTitle: String {
        switch self {
        case .all: return AppLocalization.string("All")
        case .send: return AppLocalization.string("Sends")
        case .receive: return AppLocalization.string("Receives")
        case .pending: return AppLocalization.string("Pending")
        }
    }
}
enum HistorySortOrder: String, CaseIterable, Identifiable {
    case newest = "Newest"
    case oldest = "Oldest"
    var id: String { rawValue }
    var localizedTitle: String { AppLocalization.string(rawValue) }
}
extension PriceAlertCondition {
    var displayName: String {
        switch self {
        case .above: return AppLocalization.string("Above")
        case .below: return AppLocalization.string("Below")
        }
    }
}
/// The alert rule core stores. Not a Swift copy of it — core owns the list,
/// the rule that a target must be positive, and the persistence. `id` is an
/// opaque core-assigned string, not a platform-minted `UUID`.
extension PriceAlertRule: Identifiable {}

extension PriceAlertRule {
    var chainName: String { chainId.displayName }
    var titleText: String { String(format: CommonLocalizationContent.current.assetOnChainFormat, assetDisplayName, chainName) }
    var statusText: String {
        if !isEnabled { return AppLocalization.string("Paused") }
        return hasTriggered ? AppLocalization.string("Triggered") : AppLocalization.string("Watching")
    }
}
// `AddressBookEntry` is the Rust record — core owns saved recipients, including
// the rules about which ones are acceptable. Only display helpers live here.
extension AddressBookEntry: Identifiable {
    var chainName: String { chainId.displayName }
    var subtitleText: String {
        guard !note.isEmpty else { return chainName }
        return String(format: CommonLocalizationContent.current.addressBookSubtitleFormat, chainName, note)
    }
}
/// A stored transaction, as core keeps it.
extension TransactionRecord: Identifiable {
    /// History with no deployment identity draws its letter.
    var artworkName: String { AssetPresentationCatalog.artwork(deploymentId: deploymentId) }
    var chain: Chain { chainId }
    var chainName: String { chainId.displayName }
    /// When it was recorded. Core stores Unix seconds.
    var createdDate: Date { Date(timeIntervalSince1970: createdAtUnix) }
    var titleText: String {
        let copy = CommonLocalizationContent.current
        switch kind {
        case .send: return String(format: copy.transactionSentTitleFormat, symbol)
        case .receive: return String(format: copy.transactionReceivedTitleFormat, symbol)
        case .stake: return AppLocalization.format("transaction.title.stake", symbol)
        case .unstake: return AppLocalization.format("transaction.title.unstake", symbol)
        case .withdraw: return AppLocalization.format("transaction.title.withdraw", symbol)
        case .claimRewards: return AppLocalization.format("transaction.title.claimRewards", symbol)
        case .revokeApproval: return AppLocalization.format("Revoke %@ Approval", symbol)
        case .deleteAccessKey: return AppLocalization.string("Delete Access Key")
        case .mergeCoins: return AppLocalization.format("Merge %@ Coins", symbol)
        case .closeTokenAccounts: return AppLocalization.string("Close Token Accounts")
        case .refundTokenStorage: return AppLocalization.string("Refund Token Storage")
        case .trustAsset: return AppLocalization.string("Trust Asset")
        case .removeTrustLine: return AppLocalization.string("Remove Trust Line")
        case .shield: return AppLocalization.format("Shield %@", symbol)
        }
    }
    /// Name the network and wallet, prefixing the asset name for token transfers.
    var subtitleText: String {
        let copy = CommonLocalizationContent.current
        let asset =
            assetDisplayName.caseInsensitiveCompare(chainName) == .orderedSame
            ? assetDisplayName : String(format: copy.assetOnChainFormat, assetDisplayName, chainName)
        return String(format: copy.transactionSubtitleFormat, asset, walletName)
    }
    var statusText: String { status.localizedTitle }
    var badgeColor: Color {
        switch kind {
        case .send, .stake: return .red
        case .receive, .withdraw, .claimRewards: return .green
        case .unstake, .revokeApproval, .deleteAccessKey, .mergeCoins, .closeTokenAccounts, .refundTokenStorage,
             .trustAsset, .removeTrustLine, .shield: return .secondary
        }
    }
    var isSubmittedOperation: Bool { transactionKindIsSubmitted(kind: kind) }
    var amountDirection: TransactionDirection { transactionAmountDirection(kind: kind, amount: amount) }
    var amountSign: String {
        switch amountDirection {
        case .incoming: return "+"
        case .outgoing: return "-"
        case .neutral: return ""
        }
    }
    var amountColor: Color {
        switch amountDirection {
        case .incoming: return .spectraTransactionAmountColor(isReceive: true)
        case .outgoing: return .spectraTransactionAmountColor(isReceive: false)
        case .neutral: return .secondary
        }
    }
    var receiptBlockNumberText: String? {
        guard let receiptBlockNumber else { return nil }
        return String(receiptBlockNumber)
    }
    var storedConfirmationCountText: String? {
        guard let confirmationCount else { return nil }
        return AppLocalization.format("%lld confirmations", count: Int(confirmationCount), confirmationCount)
    }
    var storedUsedChangeOutputText: String? {
        guard let usedChangeOutput else { return nil }
        return AppLocalization.string(usedChangeOutput ? "Yes" : "No")
    }
    /// The signed payload as stored, whatever its encoding; the format row
    /// beside it says which.
    var rawTransactionText: String? {
        let trimmed = signedTransactionPayload?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }
    var rawTransactionFormatText: String? {
        guard let signedTransactionPayloadFormat else { return nil }
        let trimmed = signedTransactionPayloadFormat.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
    /// Core stores a time it does not know as a date before any chain existed.
    var hasKnownDate: Bool { createdAtUnix > 0 }
    /// When it happened, in the app's language: the date and the second,
    /// for the transaction page.
    var fullTimestampText: String {
        guard hasKnownDate else { return AppLocalization.string("Unknown date") }
        return createdDate.formatted(
            Date.FormatStyle(date: .abbreviated, time: .standard).locale(AppLocalization.locale))
    }
    /// When it happened, as a list row shows it: the time alone under a day
    /// header, the date and time under a month's.
    func timestampText(showsDate: Bool) -> String {
        guard hasKnownDate else { return AppLocalization.string("Unknown date") }
        return createdDate.formatted(
            Date.FormatStyle(date: showsDate ? .abbreviated : .omitted, time: .shortened).locale(AppLocalization.locale))
    }
    /// The button that opens this transaction on its network's explorer.
    var explorerLink: (label: String, url: URL)? {
        guard let transactionHash,
              let link = transactionExplorerLink(chainId: chainId, transactionHash: transactionHash),
              let url = URL(string: link.url) else { return nil }
        return (AppLocalization.format("Open In %@", link.name), url)
    }
    /// The sentence a finished send reports, on the lock screen and in a
    /// notification alike; `nil` while it is still pending.
    func sendOutcomeDetail(for status: TransactionStatus) -> String? {
        switch status {
        case .pending: return nil
        case .confirmed:
            return AppLocalization.format("Your %@ send from %@ is now confirmed on %@.", symbol, walletName, chainName)
        case .failed:
            return localizedFailureReason
                ?? AppLocalization.format("Your %@ send from %@ failed on %@.", symbol, walletName, chainName)
        }
    }
    /// The failure reason to show, localized. Core stores the reason; the
    /// words are made here so they follow the reader's language.
    var localizedFailureReason: String? {
        guard let failureReason else { return nil }
        switch failureReason {
        case .executionFailed:
            return AppLocalization.string("The transaction failed during on-chain execution.")
        case .submissionOutcomeUnknown:
            return AppLocalization.string("Submission outcome unknown; check network status before sending again.")
        case .rebroadcastOutcomeUnknown:
            return AppLocalization.string("Rebroadcast outcome unknown; check network status before retrying.")
        case .reported(let message):
            return message
        }
    }
}

extension WalletView {
    var networkTitle: String { chainId.displayName }
}
