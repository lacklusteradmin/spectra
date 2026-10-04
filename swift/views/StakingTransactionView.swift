import SwiftUI

/// Signing and submission are explicit steps over one immutable core review.
struct StakingTransactionView: View {
    @Bindable var store: AppState
    @Bindable var vm: StakingViewModel
    let artifact: SendArtifact
    @Binding var confirmsSigning: Bool
    @Binding var confirmsBroadcast: Bool

    private var action: SendExecutionAction {
        SendExecutionAction(artifact: artifact, transaction: vm.transaction)
    }
    private var canBroadcast: Bool {
        action == .broadcast
            || (action == .retry
                && SendExecutionAction.canRetry(artifact: artifact, transaction: vm.transaction))
    }
    private var offersRepair: Bool {
        guard let intent = artifact.staking else { return false }
        return stakingInputRules(chain: intent.chainId, action: intent.action).repairAllowed
            && artifact.stage == .signed && !artifact.attempts.isEmpty
            && vm.transaction?.status != .confirmed
    }
    private var canCheckStatus: Bool {
        !artifact.attempts.isEmpty && (vm.transaction == nil || vm.transaction?.status == .pending)
    }
    private var needsReadPassword: Bool {
        vm.rules.positionsRequireAuthorization
            && (store.wallet(for: artifact.walletId)?.signing.requiresPassword ?? true)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            reviewCard
            statusCard
            if canBroadcast { destinations }
            if let review = artifact.review.staking, action == .sign {
                Text(AppLocalization.string("staking.review_before_signing")).font(.caption)
                    .foregroundStyle(.secondary)
                Button(AppLocalization.string("Sign Transaction")) { confirmsSigning = true }
                    .buttonStyle(.glassProminent).disabled(vm.isBusy || review.networkFee.isEmpty)
                    .accessibilityIdentifier("staking.sign")
            } else if canBroadcast {
                Button(AppLocalization.string(action.title)) { confirmsBroadcast = true }
                    .buttonStyle(.glassProminent).disabled(vm.isBusy || vm.session.selectedEndpoints.isEmpty)
                    .accessibilityIdentifier("staking.broadcast")
            }
            if canCheckStatus || offersRepair {
                if needsReadPassword {
                    SecureField(AppLocalization.string("Wallet Password"), text: $vm.password).spectraInputFieldStyle()
                }
            }
            if canCheckStatus {
                Button(AppLocalization.string("staking.check_status")) { vm.begin(.recheck) }
                    .buttonStyle(.glass).disabled(vm.isBusy || (needsReadPassword && vm.password.isEmpty))
            }
            if offersRepair {
                Text(AppLocalization.string("staking.repair_explanation")).font(.caption).foregroundStyle(.secondary)
                Button(AppLocalization.string("staking.repair")) { vm.begin(.repair) }
                    .buttonStyle(.glass).disabled(vm.isBusy || (needsReadPassword && vm.password.isEmpty))
                    .accessibilityIdentifier("staking.repair")
            }
            if let link = vm.transaction?.explorerLink {
                Link(AppLocalization.string("View in block explorer"), destination: link.url).buttonStyle(
                    .glass)
            }
            Button(AppLocalization.string("staking.close_review")) { vm.startStake() }
                .buttonStyle(.glass).disabled(vm.isBusy)
            if vm.isBusy { ProgressView() }
        }
    }

    private var reviewCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(AppLocalization.string("staking.review")).font(.headline)
            if let intent = artifact.staking {
                reviewRow(AppLocalization.string("staking.action"), value: intent.action.localizedTitle)
                reviewRow(
                    AppLocalization.string("Amount"),
                    value: "\(AmountPresentation.localizedDecimal(artifact.amount)) \(artifact.symbol)")
                if intent.amount == nil {
                    Text(AppLocalization.string("staking.full_position_action")).font(.caption)
                        .foregroundStyle(.secondary)
                }
                if let validator = intent.validatorId {
                    reviewRow(AppLocalization.string("staking.validator_identifier"), value: validator)
                }
                if let position = intent.positionId {
                    reviewRow(AppLocalization.string("staking.position"), value: position)
                }
                if let delay = artifact.review.staking?.lockupSeconds {
                    reviewRow(
                        AppLocalization.string("staking.dissolve_delay"),
                        value: AppLocalization.format("staking.seconds", delay))
                }
            }
            reviewRow(
                AppLocalization.string("Wallet"),
                value: store.wallet(for: artifact.walletId)?.name ?? artifact.walletId)
            reviewRow(AppLocalization.string("From"), value: artifact.sender)
            if let entry = CoreReferenceTables.stakingEntry(for: artifact.chainId) {
                reviewRow(
                    AppLocalization.string("Unbonding"), value: AppLocalization.string(entry.unbondingPeriod))
            }
            if let review = artifact.review.staking {
                if review.fundingAlreadyCompleted {
                    Text(AppLocalization.string("staking.funding_completed")).font(.caption).foregroundStyle(.secondary)
                }
                let label = AppLocalization.string(
                    review.feeIsUpperBound ? "staking.fee_upper_bound" : "Network Fee")
                reviewRow(
                    label,
                    value:
                        "\(AmountPresentation.localizedDecimal(review.networkFee)) \(artifact.chainId.gasTokenSymbol)"
                )
                if review.feeIsDeductedFromAmount {
                    Text(AppLocalization.string("staking.fee_deducted")).font(.caption).foregroundStyle(.secondary)
                }
                if let deposit = review.refundableDeposit {
                    reviewRow(
                        AppLocalization.string("staking.refundable_deposit"),
                        value:
                            "\(AmountPresentation.localizedDecimal(deposit)) \(artifact.chainId.gasTokenSymbol)")
                }
                if review.rewardPayoutIsDelayed {
                    Text(AppLocalization.string("staking.delayed_reward_payout")).font(.caption)
                        .foregroundStyle(.secondary)
                }
            } else {
                Text(AppLocalization.string("staking.review_unavailable")).font(.caption).foregroundStyle(
                    .red)
            }
            Text(AppLocalization.string("staking.saved_review")).font(.caption).foregroundStyle(
                .secondary)
        }.padding(SpectraLayout.cardPadding).spectraElevatedFill()
    }

    private func reviewRow(_ title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            Text(verbatim: value).font(.subheadline).textSelection(.enabled)
        }
    }

    private var statusCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            if let transaction = vm.transaction {
                Text(transaction.status.localizedTitle).font(.headline)
                if let failure = transaction.localizedFailureReason {
                    Text(verbatim: failure).font(.subheadline).foregroundStyle(.red)
                }
            } else {
                Text(AppLocalization.string(artifact.stage == .prepared ? "Awaiting signing" : "Signed"))
                    .font(.headline)
            }
            if artifact.attempts.contains(where: { $0.outcome == .accepted }),
                vm.transaction == nil || vm.transaction?.status == .pending
            {
                Text(AppLocalization.string("staking.submission_pending")).font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            if let hash = artifact.transactionHash {
                Text(verbatim: hash).font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(
                    .enabled)
            }
            ForEach(Array(artifact.attempts.enumerated()), id: \.offset) { _, attempt in
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                    Text(verbatim: attempt.endpoint).font(.caption.monospaced())
                    Text(verbatim: attempt.detail).font(.caption).foregroundStyle(.secondary)
                }
            }
        }.padding(SpectraLayout.cardPadding).spectraCardFill()
    }

    private var destinations: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            Text(AppLocalization.string("Select submission nodes")).font(.headline)
            ForEach(vm.session.endpoints, id: \.self) { endpoint in
                Toggle(
                    isOn: Binding(
                        get: { vm.session.selectedEndpoints.contains(endpoint) },
                        set: { selected in
                            if selected {
                                vm.session.selectedEndpoints.insert(endpoint)
                            } else {
                                vm.session.selectedEndpoints.remove(endpoint)
                            }
                        })
                ) {
                    Text(verbatim: endpoint).font(.caption.monospaced()).textSelection(.enabled)
                }
            }
        }.padding(SpectraLayout.cardPadding).spectraCardFill().disabled(vm.isBusy)
    }
}
