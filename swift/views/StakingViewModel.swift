import Foundation

/// A transient form over core's persisted positions, reviews and broadcast journal.
@MainActor @Observable final class StakingViewModel {
    enum Operation {
        case positions, build, sign, broadcast
        case resume(String)
        case recheck
        case repair
    }
    struct Request {
        let id = UUID()
        let operation: Operation
    }

    let chain: Chain
    let session = SendSession()
    var validators: [StakingValidator] = []
    var positions: [StakingPosition] = []
    var savedArtifacts: [SendArtifact] = []
    var walletId = ""
    var action: StakingAction = .stake
    var validatorId = ""
    var positionId: String?
    var amount = ""
    var lockupSeconds = ""
    var extraTarget = ""
    var password = ""
    var transaction: TransactionRecord?
    var request: Request?
    var isLoading = false
    var hasLoadedPositions = false
    var error: String?
    @ObservationIgnored private let operations: StakingOperations
    @ObservationIgnored private let authentication: (@MainActor (String) async -> String?)?
    @ObservationIgnored private let broadcastCompletion: (@MainActor (SendArtifact) async -> Void)?
    @ObservationIgnored private var generation = UUID()
    @ObservationIgnored private var positionsRead = UUID()
    @ObservationIgnored private var savedRead = UUID()
    @ObservationIgnored private var transactionRead = UUID()

    init(
        chain: Chain, bridge: WalletServiceBridge, operations: StakingOperations? = nil,
        authentication: (@MainActor (String) async -> String?)? = nil,
        broadcastCompletion: (@MainActor (SendArtifact) async -> Void)? = nil
    ) {
        self.chain = chain
        self.operations = operations ?? .live(bridge: bridge)
        self.authentication = authentication
        self.broadcastCompletion = broadcastCompletion
    }

    private func authorize(_ reason: String, store: AppState) async -> String? {
        if let authentication { return await authentication(reason) }
        return await store.authenticate(.send, reason: reason)
    }

    var rules: StakingInputRules { stakingInputRules(chain: chain, action: action) }
    var isBusy: Bool { request != nil || session.isBusy }

    func selectWallet(_ id: String) {
        guard id != walletId else { return }
        cancel()
        walletId = id
        positions = []
        savedArtifacts = []
        hasLoadedPositions = false
        startStake()
    }

    func startStake() {
        session.reset()
        transaction = nil
        action = .stake
        validatorId = ""
        positionId = nil
        amount = ""
        lockupSeconds = ""
        password = ""
        error = nil
    }

    func selectPosition(_ position: StakingPosition, action: StakingAction) {
        session.reset()
        transaction = nil
        self.action = action
        validatorId = position.validatorIdentifier
        positionId = position.id
        let units =
            action == .withdraw
            ? position.withdrawableAmountSmallestUnit : position.stakedAmountSmallestUnit
        amount =
            stakingInputRules(chain: chain, action: action).amountAllowed
            ? formatNativeAmount(chain: chain, smallestUnit: units) ?? "" : ""
        lockupSeconds = ""
        password = ""
        error = nil
    }

    func begin(_ operation: Operation) {
        guard !isBusy else { return }
        error = nil
        session.error = nil
        request = Request(operation: operation)
    }

    func cancel() {
        generation = UUID()
        request = nil
        password = ""
        session.reset()
        transaction = nil
    }

    private func current(_ token: UUID) -> Bool { generation == token && !Task.isCancelled }

    func loadValidators() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            let result = try await operations.validators(chain)
            guard !Task.isCancelled else { return }
            validators = result
        } catch { if !Task.isCancelled { self.error = userErrorMessage(error) } }
    }

    func loadWalletData() async {
        let token = generation
        let wallet = walletId
        guard !wallet.isEmpty else { return }
        await loadSavedArtifacts()
        guard current(token), wallet == walletId else { return }
        do {
            if !rules.positionsRequireAuthorization {
                try await readPositions(wallet: wallet, password: nil, token: token)
            }
        } catch {
            if current(token), wallet == walletId { self.error = userErrorMessage(error) }
        }
    }

    func loadSavedArtifacts() async {
        let token = generation
        let wallet = walletId
        guard !wallet.isEmpty else { return }
        let query = UUID()
        savedRead = query
        do {
            let saved = try await operations.saved()
            guard current(token), wallet == walletId, savedRead == query else { return }
            savedArtifacts = saved.filter { $0.staking != nil && $0.chainId == chain && $0.walletId == wallet }
        } catch {
            if current(token), wallet == walletId, savedRead == query { self.error = userErrorMessage(error) }
        }
    }

    private func readPositions(wallet: String, password: String?, token: UUID) async throws {
        let targets = extraTarget.isEmpty ? [] : [extraTarget]
        let query = UUID()
        positionsRead = query
        do {
            let result = try await operations.positions(wallet, chain, targets, password)
            guard current(token), walletId == wallet, positionsRead == query else { return }
            positions = result
            hasLoadedPositions = true
        } catch {
            guard current(token), walletId == wallet, positionsRead == query else { return }
            throw error
        }
    }

    private func intent() throws -> StakingRequest {
        let delay: UInt64?
        if rules.lockupRequired {
            guard let parsed = UInt64(lockupSeconds) else {
                throw DisplayedError(AppLocalization.string("staking.delay_integer"))
            }
            delay = parsed
        } else {
            delay = nil
        }
        return StakingRequest(
            walletId: walletId, chainId: chain, action: action,
            validatorId: rules.validatorRequired && !validatorId.isEmpty ? validatorId : nil,
            positionId: positionId,
            amount: rules.amountAllowed && !amount.isEmpty
                ? AmountPresentation.canonicalDecimalInput(amount) : nil,
            lockupSeconds: delay)
    }

    func perform(_ id: UUID, store: AppState) async {
        guard let request, request.id == id else { return }
        let token = generation
        let wallet = walletId
        let requiresPassword =
            store.wallet(for: session.artifact?.walletId ?? wallet)?.signing.requiresPassword ?? true
        let suppliedPassword = requiresPassword ? password : nil
        password = ""
        defer { if self.request?.id == id { self.request = nil } }
        do {
            switch request.operation {
            case .positions:
                if rules.positionsRequireAuthorization {
                    if let failure = await authorize(
                        AppLocalization.string("staking.authorize_positions"), store: store)
                    {
                        throw DisplayedError(failure)
                    }
                    guard current(token) else { return }
                }
                try await readPositions(wallet: wallet, password: suppliedPassword, token: token)
            case .build:
                let input = try intent()
                if let failure = await authorize(AppLocalization.string("staking.authorize_build"), store: store) {
                    throw DisplayedError(failure)
                }
                guard current(token), try intent() == input else { return }
                await session.load(
                    operation: .build,
                    prepare: {
                        let result = try await self.operations.build(input, suppliedPassword)
                        guard self.current(token), try self.intent() == input else {
                            throw DisplayedError(AppLocalization.string("staking.inputs_changed"))
                        }
                        return result
                    }, endpoints: { try await self.operations.endpoints($0) })
            case .sign:
                await session.sign(
                    password: suppliedPassword,
                    authenticate: {
                        await self.authorize(AppLocalization.string("Authorize transaction signing"), store: store)
                    },
                    sign: { try await self.operations.sign($0, $1, $2) })
            case .broadcast:
                if let submitted = await session.broadcast(submit: {
                    try await self.operations.broadcast($0, $1)
                }) {
                    if let broadcastCompletion {
                        await broadcastCompletion(submitted)
                    } else {
                        await store.handleBroadcastCompletion(submitted)
                    }
                    if current(token) { await loadTransaction() }
                }
            case .resume(let savedId):
                session.reset()
                await session.load(
                    operation: .resume,
                    prepare: { try await self.operations.inspect(savedId) },
                    endpoints: { try await self.operations.endpoints($0) })
                if current(token) { await loadTransaction() }
            case .recheck:
                if let artifact = session.artifact, !artifact.attempts.isEmpty {
                    if rules.positionsRequireAuthorization {
                        if let failure = await authorize(
                            AppLocalization.string("staking.authorize_status"), store: store)
                        {
                            throw DisplayedError(failure)
                        }
                        guard current(token), session.artifact?.id == artifact.id else { return }
                    }
                    let inspected = try await operations.recheck(artifact.id, suppliedPassword)
                    await store.refreshTransactionProjection()
                    if current(token), session.artifact?.id == artifact.id {
                        session.artifact = inspected
                        await loadTransaction()
                    }
                }
            case .repair:
                guard let artifact = session.artifact else { return }
                if let failure = await authorize(AppLocalization.string("staking.authorize_repair"), store: store) {
                    throw DisplayedError(failure)
                }
                guard current(token), session.artifact?.reviewDigest == artifact.reviewDigest else { return }
                if await session.load(
                    operation: .build,
                    prepare: { try await self.operations.repair(artifact.id, suppliedPassword) },
                    endpoints: { try await self.operations.endpoints($0) })
                {
                    transaction = nil
                    transactionRead = UUID()
                }
            }
        } catch { if current(token) { self.error = userErrorMessage(error) } }
    }

    func loadTransaction() async {
        let query = UUID()
        transactionRead = query
        guard let artifact = session.artifact, !artifact.attempts.isEmpty else {
            transaction = nil
            return
        }
        let token = generation
        do {
            let result = try await operations.transaction(artifact.id)
            guard current(token), session.artifact?.reviewDigest == artifact.reviewDigest, transactionRead == query
            else {
                return
            }
            transaction = result
        } catch {
            if current(token), session.artifact?.reviewDigest == artifact.reviewDigest, transactionRead == query {
                self.error = userErrorMessage(error)
            }
        }
    }
}

extension StakingAction {
    var localizedTitle: String {
        switch self {
        case .stake: AppLocalization.string("staking.stake")
        case .unstake: AppLocalization.string("staking.unstake")
        case .withdraw: AppLocalization.string("staking.withdraw")
        case .claimRewards: AppLocalization.string("staking.claim_rewards")
        }
    }
}

extension StakingPositionStatus {
    var localizedTitle: String {
        switch self {
        case .active: AppLocalization.string("staking.active")
        case .activating: AppLocalization.string("staking.activating")
        case .unbonding: AppLocalization.string("staking.unbonding")
        case .withdrawable: AppLocalization.string("staking.withdrawable")
        case .inactive: AppLocalization.string("staking.inactive")
        }
    }
}
