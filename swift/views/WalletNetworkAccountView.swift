import SwiftUI

/// What the wallet's account on its network holds and needs beyond a
/// balance, as core reads it from a node: Tron's resources, the XRP and
/// Stellar reserve, a TON contract's state, the parts of a Substrate balance,
/// the storage a NEAR account pays for, and a Cardano address's stake
/// address with its rewards and delegation. Read-only.
struct WalletNetworkAccountView: View {
    let store: AppState
    let wallet: WalletView
    @State private var account: WalletNetworkAccount?
    @State private var error: String?
    @State private var isLoading = false
    @State private var isClosing = false

    var body: some View {
        List {
            if let account {
                content(account.account, symbol: account.symbol)
                if account.closable {
                    Section {
                        Button(AppLocalization.string("Close Account"), role: .destructive) { isClosing = true }
                    } footer: {
                        Text(AppLocalization.string("Recover the reserve by closing this account into another one."))
                    }
                }
            } else if let error {
                WalletToolErrorSection(message: error)
            } else {
                WalletToolLoadingSection()
            }
        }
        .navigationTitle(WalletAction.networkAccount.title).navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .sheet(isPresented: $isClosing, onDismiss: { Task { await load() } }) {
            NavigationStack { CloseAccountView(store: store, wallet: wallet) }
        }
    }

    @ViewBuilder
    private func content(_ account: NetworkAccount, symbol: String) -> some View {
        switch account {
        case let .tron(
            activated, bandwidthAvailable, bandwidthLimit, energyAvailable, energyLimit,
            bandwidthPrice, energyPrice, transferBurn, tokenTransferBurn, tokenTransferCeiling):
            if !activated {
                notice(
                    AppLocalization.string("Not on the network yet: the first TRX sent to this address creates the account. Tokens alone do not."),
                    systemImage: "exclamationmark.circle")
            }
            Section {
                resource(available: bandwidthAvailable, limit: bandwidthLimit)
                amountRow(AppLocalization.string("Price per Point"), bandwidthPrice, symbol)
            } header: {
                Text(AppLocalization.string("Bandwidth"))
            } footer: {
                Text(AppLocalization.format(
                    "Every transaction spends bandwidth for its size. Without enough, it burns TRX instead: %@ %@ for a TRX transfer.",
                    transferBurn, symbol))
            }
            Section {
                resource(available: energyAvailable, limit: energyLimit)
                amountRow(AppLocalization.string("Price per Unit"), energyPrice, symbol)
            } header: {
                Text(AppLocalization.string("Energy"))
            } footer: {
                Text(AppLocalization.format(
                    "Token contracts spend energy. Without it, a token transfer burns about %@ %@, and at most %@ %@.",
                    tokenTransferBurn, symbol, tokenTransferCeiling, symbol))
            }
        case let .reserve(exists, balance, baseReserve, ownedObjects, objectReserve, totalReserve, spendable):
            if !exists {
                notice(
                    AppLocalization.format(
                        "Not on the network yet: a payment of at least %@ %@ creates the account.", baseReserve, symbol),
                    systemImage: "exclamationmark.circle")
            }
            Section {
                amountRow(AppLocalization.string("Balance"), balance, symbol)
                amountRow(AppLocalization.string("Base Reserve"), baseReserve, symbol)
                amountRow(
                    AppLocalization.format("Owned Objects (%lld)", Int(ownedObjects)), objectReserve, symbol)
                amountRow(AppLocalization.string("Total Reserve"), totalReserve, symbol)
                amountRow(AppLocalization.string("Spendable"), spendable, symbol, emphasized: true)
            } footer: {
                Text(AppLocalization.string("The network locks the reserve in the account; a send moves only what is above it. Each trust line, offer or other object the account owns locks more, and removing it frees that again."))
            }
        case let .ton(state, contract):
            Section {
                LabeledContent(AppLocalization.string("State"), value: state.title)
                if let contract {
                    LabeledContent(AppLocalization.string("Wallet Contract"), value: contract)
                }
            } footer: {
                Text(state.explanation)
            }
        case let .substrate(free, reserved, frozen, existentialDeposit, transferable):
            Section {
                amountRow(AppLocalization.string("Free"), free, symbol)
                amountRow(AppLocalization.string("Reserved"), reserved, symbol)
                amountRow(AppLocalization.string("Frozen"), frozen, symbol)
                amountRow(AppLocalization.string("Existential Deposit"), existentialDeposit, symbol)
                amountRow(AppLocalization.string("Transferable"), transferable, symbol, emphasized: true)
            } footer: {
                Text(AppLocalization.string("Reserved is held by the network for deposits and is not in the free balance. Frozen is free balance that staking or another lock keeps from moving. An account that falls below the existential deposit is removed, and what is left in it is lost."))
            }
        case let .near(storageBytes, storagePrice, locked, storageReserve):
            Section {
                LabeledContent(
                    AppLocalization.string("Storage Used"),
                    value: AppLocalization.format("%lld bytes", count: Int(storageBytes), Int(storageBytes)))
                amountRow(AppLocalization.string("Price per Byte"), storagePrice, symbol)
                amountRow(AppLocalization.string("Locked Stake"), locked, symbol)
                amountRow(AppLocalization.string("Kept for Storage"), storageReserve, symbol, emphasized: true)
            } footer: {
                Text(AppLocalization.string("Every byte an account stores must be covered by NEAR it holds, except for a small account within the network's free allowance. Locked stake counts toward it; the rest is kept back from the balance, and a send cannot move it."))
            }
        case let .cardano(stakeAddress, registered, rewards, delegatedPool, delegatedDrep):
            if !registered {
                notice(
                    AppLocalization.string("The stake key is not registered: this account delegates to no pool and earns no rewards."),
                    systemImage: "exclamationmark.circle")
            }
            Section {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                    Text(AppLocalization.string("Stake Address"))
                    Text(verbatim: stakeAddress).font(.caption.monospaced()).foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
                amountRow(AppLocalization.string("Rewards"), rewards, symbol, emphasized: true)
                LabeledContent(AppLocalization.string("Pool")) {
                    Text(verbatim: delegatedPool ?? AppLocalization.string("None")).font(.caption.monospaced())
                        .lineLimit(1).truncationMode(.middle)
                }
                LabeledContent(AppLocalization.string("Voting Delegation")) {
                    Text(verbatim: delegatedDrep ?? AppLocalization.string("None")).font(.caption.monospaced())
                        .lineLimit(1).truncationMode(.middle)
                }
            } footer: {
                Text(AppLocalization.string("Every address of this account shares one stake key. Delegating it to a pool earns rewards, which build up at the stake address and are not part of the balance until withdrawn."))
            }
        }
    }

    private func resource(available: UInt64, limit: UInt64) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            HStack {
                Text(AppLocalization.string("Available"))
                Spacer()
                Text(verbatim: "\(available) / \(limit)").monospacedDigit().foregroundStyle(.secondary)
            }
            if limit > 0 {
                ProgressView(value: Double(min(available, limit)), total: Double(limit))
            }
        }
    }

    private func amountRow(_ title: String, _ amount: String, _ symbol: String, emphasized: Bool = false) -> some View {
        LabeledContent(title) {
            Text(verbatim: "\(amount) \(symbol)").monospacedDigit()
                .fontWeight(emphasized ? .semibold : .regular)
                .foregroundStyle(emphasized ? Color.primary : Color.secondary)
        }
    }

    private func notice(_ text: String, systemImage: String) -> some View {
        Section {
            Label(text, systemImage: systemImage).font(.subheadline).foregroundStyle(Color.spectraWarning)
        }
    }

    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            account = try await store.bridge.ready().walletNetworkAccount(walletId: wallet.id)
            error = nil
        } catch {
            self.error = userErrorMessage(error)
        }
    }
}

extension TonAccountState {
    var title: String {
        switch self {
        case .active: AppLocalization.string("Active")
        case .uninitialized: AppLocalization.string("Not Deployed")
        case .frozen: AppLocalization.string("Frozen")
        }
    }

    var explanation: String {
        switch self {
        case .active: AppLocalization.string("The wallet contract is deployed and runs every send.")
        case .uninitialized:
            AppLocalization.string("The wallet contract is not deployed yet. The first send deploys it with the transfer, for a slightly higher fee. Receiving does not need it.")
        case .frozen:
            AppLocalization.string("Frozen for unpaid storage fees: it cannot send until the debt is paid and the contract is restored.")
        }
    }
}

/// Closing the wallet's XRP or Stellar account into another existing
/// account: core checks every prerequisite and builds the transaction; this
/// sheet shows what is given up and asks for the user's confirmation before
/// signing.
private struct CloseAccountView: View {
    let store: AppState
    let wallet: WalletView
    @Environment(\.dismiss) private var dismiss
    @State private var session = SendSession()
    @State private var destination = ""
    @State private var memoKind: PaymentMemoKind?
    @State private var memoText = ""
    @State private var understood = false

    private var memoKinds: [PaymentMemoKind] { paymentMemoKinds(chain: wallet.chainId) }

    var body: some View {
        Form {
            if let artifact = session.artifact,
               case let .closeAccount(destination, reserve, removedObjects, networkFee) = artifact.operation {
                Section {
                    LabeledContent(AppLocalization.string("To")) {
                        Text(verbatim: destination).font(.caption.monospaced()).lineLimit(1)
                            .truncationMode(.middle)
                    }
                    if let memo = artifact.memo { PaymentMemoRow(memo: memo) }
                    LabeledContent(AppLocalization.string("Amount"), value: "\(artifact.amount) \(artifact.symbol)")
                    LabeledContent(
                        AppLocalization.string("Reserve Recovered"), value: "\(reserve) \(artifact.symbol)")
                    if removedObjects > 0 {
                        LabeledContent(AppLocalization.string("Objects Deleted"), value: "\(removedObjects)")
                    }
                    LabeledContent(
                        AppLocalization.string("Network Fee"), value: "\(networkFee) \(artifact.symbol)")
                } footer: {
                    Text(AppLocalization.string("Everything the account holds when the transaction executes goes to this account, less the fee. The amount shown is its balance now."))
                }
                if artifact.stage == .prepared {
                    Section {
                        Toggle(
                            AppLocalization.string("I understand the account is deleted and this cannot be undone."),
                            isOn: $understood)
                    }
                }
            } else {
                Section {
                    AddressEntryRow(
                        title: AppLocalization.string("Destination Address"), text: $destination, chain: wallet.chain,
                        contacts: store.addressBook.entries.filter { $0.chainId == wallet.chain })
                    if !memoKinds.isEmpty {
                        SendPaymentMemoField(kinds: memoKinds, kind: $memoKind, text: $memoText)
                    }
                } footer: {
                    Text(AppLocalization.string("Closing deletes this account on the network and sends everything it holds, its reserve included, to another account that already exists. It cannot be undone. The wallet stays in Spectra, and funding its address again later opens a new account."))
                }
                Section {
                    Button(AppLocalization.string("Review")) { Task { await review() } }
                        .disabled(destination.trimmingCharacters(in: .whitespaces).isEmpty || session.isBusy)
                }
            }
            SendArtifactStageSections(
                store: store, wallet: wallet, session: session,
                authenticationReason: AppLocalization.format("Authenticate to close the account of %@", wallet.name),
                submittedText: AppLocalization.string("Submitted. The account is closed once it confirms."),
                canSign: understood)
        }
        .navigationTitle(AppLocalization.string("Close Account")).navigationBarTitleDisplayMode(.inline)
        .sendSheetDismissal(session: session) { dismiss() }
    }

    private func review() async {
        let destination = destination.trimmingCharacters(in: .whitespacesAndNewlines)
        let memo = memoKind.flatMap { memoText.isEmpty ? nil : PaymentMemo(kind: $0, value: memoText) }
        await session.load(
            operation: .build,
            prepare: {
                try await store.bridge.ready().buildAccountClosing(
                    walletId: wallet.id, destination: destination, memo: memo)
            },
            endpoints: { try await store.bridge.ready().sendEndpoints(chain: $0) })
    }
}
