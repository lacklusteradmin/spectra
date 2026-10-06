import SwiftUI

struct StakingPositionsView: View {
    @Bindable var vm: StakingViewModel
    let canSign: Bool
    let requiresPassword: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            HStack {
                Text(AppLocalization.string("staking.positions")).font(.headline)
                Spacer()
                if vm.isBusy { ProgressView() }
            }
            TextField(AppLocalization.string("staking.additional_pool"), text: $vm.extraTarget)
                .textInputAutocapitalization(.never).autocorrectionDisabled().spectraInputFieldStyle()
            if vm.rules.positionsRequireAuthorization && requiresPassword {
                SecureField(AppLocalization.string("Wallet Password"), text: $vm.password)
                    .spectraInputFieldStyle()
            }
            Button(AppLocalization.string("staking.refresh_positions")) { vm.begin(.positions) }
                .buttonStyle(.glass)
                .disabled(
                    vm.isBusy
                        || (vm.rules.positionsRequireAuthorization
                            && (!canSign || (requiresPassword && vm.password.isEmpty)))
                )
                .accessibilityIdentifier("staking.refresh_positions")
            if vm.positions.isEmpty {
                Text(
                    AppLocalization.string(
                        vm.hasLoadedPositions ? "staking.no_positions" : "staking.positions_not_loaded")
                )
                .font(.subheadline).foregroundStyle(.secondary)
            } else {
                ForEach(vm.positions) { position in
                    Divider()
                    positionRow(position)
                }
            }
        }.padding(SpectraLayout.cardPadding).spectraCardFill().disabled(vm.isBusy)
    }

    private func positionRow(_ position: StakingPosition) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            Text(position.status.localizedTitle).font(.subheadline.weight(.semibold))
            Text(verbatim: position.id).font(.caption.monospaced()).foregroundStyle(.secondary)
                .textSelection(.enabled)
            amountRow("staking.staked_balance", units: position.stakedAmountSmallestUnit)
            amountRow("staking.unbonding_balance", units: position.unbondingAmountSmallestUnit)
            amountRow("staking.withdrawable_balance", units: position.withdrawableAmountSmallestUnit)
            if let rewards = position.claimableRewardsSmallestUnit {
                amountRow("staking.rewards", units: rewards)
            }
            if let pending = position.pendingRewardsSmallestUnit {
                amountRow("staking.pending_rewards", units: pending)
            }
            if let payoutTime = position.rewardsUnlockTimeUnix {
                HStack {
                    Text(AppLocalization.string("staking.rewards_unlock_time")).foregroundStyle(.secondary)
                    Spacer()
                    Text(
                        Date(timeIntervalSince1970: Double(payoutTime)),
                        format: .dateTime.year().month().day().hour().minute())
                }.font(.caption)
            }
            if let unlock = position.unlockTimeUnix {
                HStack {
                    Text(AppLocalization.string("staking.unlock_time")).foregroundStyle(.secondary)
                    Spacer()
                    Text(
                        Date(timeIntervalSince1970: Double(unlock)),
                        format: .dateTime.year().month().day().hour().minute())
                }.font(.caption)
            }
            if let epoch = position.unlockEpoch {
                Text(AppLocalization.format("staking.unlock_epoch", epoch)).font(.caption).foregroundStyle(
                    .secondary)
            }
            if !position.availableActions.isEmpty {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: SpectraLayout.Space.s) { actions(position) }
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.s) { actions(position) }
                }
            }
        }
    }

    private func amountRow(_ label: String, units: String) -> some View {
        HStack(alignment: .top, spacing: SpectraLayout.Space.s) {
            Text(AppLocalization.string(label)).foregroundStyle(.secondary)
            Spacer()
            Text(
                verbatim:
                    "\(formatStakingAmount(chain: vm.chain, smallestUnit: units).map(AmountPresentation.localizedDecimal) ?? "—") \(vm.chain.gasTokenSymbol)"
            )
            .multilineTextAlignment(.trailing).monospacedDigit()
        }.font(.caption)
    }

    @ViewBuilder private func actions(_ position: StakingPosition) -> some View {
        ForEach(position.availableActions, id: \.self) { action in
            Button(action.localizedTitle) { vm.selectPosition(position, action: action) }
                .buttonStyle(.glass).disabled(!canSign)
        }
    }
}

struct StakingValidatorPicker: View {
    let validators: [StakingValidator]
    let isLoading: Bool
    @Binding var selected: String
    @State private var search = ""
    @Environment(\.dismiss) private var dismiss

    private var filtered: [StakingValidator] {
        guard !search.isEmpty else { return validators }
        return validators.filter {
            $0.displayName.localizedCaseInsensitiveContains(search)
                || $0.identifier.localizedCaseInsensitiveContains(search)
        }
    }

    var body: some View {
        NavigationStack {
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    if isLoading { ProgressView() }
                    if filtered.isEmpty {
                        Text(AppLocalization.string("staking.no_validators")).foregroundStyle(.secondary)
                    } else {
                        SpectraRowGroup(data: filtered) { validator in
                            Button {
                                selected = validator.identifier
                                dismiss()
                            } label: {
                                HStack(spacing: SpectraLayout.Space.m) {
                                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                                        Text(validator.displayName).font(.subheadline.weight(.semibold))
                                            .foregroundStyle(.primary)
                                        Text(verbatim: validator.identifier).font(.caption.monospaced())
                                            .foregroundStyle(.secondary)
                                        if let commission = validator.commission {
                                            Text(AppLocalization.format("%.0f%% commission", commission * 100)).font(
                                                .caption
                                            ).foregroundStyle(.secondary)
                                        }
                                    }
                                    Spacer(minLength: SpectraLayout.Space.s)
                                    if selected == validator.identifier {
                                        Image(systemName: "checkmark").foregroundStyle(.tint)
                                    }
                                }.spectraRowPadding()
                            }.buttonStyle(.plain)
                        }
                    }
                }.spectraScreenPadding()
            }
            .background(SpectraBackdrop().ignoresSafeArea())
            .navigationTitle(AppLocalization.string("Validators"))
            .navigationBarTitleDisplayMode(.inline).toolbarBackground(.hidden, for: .navigationBar)
            .searchable(text: $search)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(AppLocalization.string("Done")) { dismiss() }
                }
            }
        }
    }
}

extension StakingPosition: Identifiable {}
extension StakingValidator: Identifiable { public var id: String { identifier } }
extension SendArtifact: Identifiable {}
