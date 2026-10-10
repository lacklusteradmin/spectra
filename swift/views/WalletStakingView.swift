import SwiftUI

/// One wallet's staking: its positions, and staking from it. A wallet's page
/// opens it where core offers staking for the wallet's network; there is no
/// other way in.
struct WalletStakingView: View {
    let chain: Chain
    let walletId: String
    @Bindable var store: AppState
    @State private var vm: StakingViewModel
    @State private var showsValidators = false
    @State private var confirmsSigning = false
    @State private var confirmsBroadcast = false

    init(store: AppState, wallet: WalletView) {
        chain = wallet.chain
        walletId = wallet.id
        self.store = store
        _vm = State(wrappedValue: StakingViewModel(chain: wallet.chain, bridge: store.bridge))
    }

    private var wallet: WalletView? { store.wallet(for: vm.walletId) }
    /// The wallet's balance of the network's coin, as its rows show it.
    private var availableToStake: String? {
        guard let coin = store.availableSendCoins(for: vm.walletId).first(where: \.isNativeCoin) else { return nil }
        return store.amounts.formattedAssetAmount(coin.amount, symbol: coin.symbol, deploymentId: coin.holdingKey)
    }
    private var requiresPassword: Bool { wallet?.signing.requiresPassword ?? true }

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                if let entry = CoreReferenceTables.stakingEntry(for: chain) { mechanics(entry) }
                if wallet?.signing.isWatchOnly == true {
                    Text(AppLocalization.string("staking.watch_only")).font(.caption).foregroundStyle(.secondary)
                } else if requiresPassword {
                    passwordCard
                }
                if !vm.walletId.isEmpty {
                    if let artifact = vm.session.artifact {
                        StakingTransactionView(
                            store: store, vm: vm, artifact: artifact,
                            confirmsSigning: $confirmsSigning, confirmsBroadcast: $confirmsBroadcast)
                    } else {
                        StakingPositionsView(
                            vm: vm, canSign: wallet?.signing.isWatchOnly == false,
                            requiresPassword: requiresPassword)
                        preparation
                        savedTransactions
                    }
                }
            }.spectraScreenPadding()
        }
        .scrollDismissesKeyboard(.interactively)
        .background(SpectraBackdrop().ignoresSafeArea())
        .navigationTitle(AppLocalization.string("Staking")).navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .task {
            vm.selectWallet(walletId)
            await vm.loadValidators()
        }
        .task(id: vm.walletId) { await vm.loadWalletData() }
        .task(id: "saved:\(vm.session.id):\(vm.session.artifact?.revision ?? 0)") {
            await vm.loadSavedArtifacts()
        }
        .task(id: vm.request?.id) {
            if let id = vm.request?.id { await vm.perform(id, store: store) }
        }
        .task(id: "\(vm.session.artifact?.id ?? ""):\(store.transactionRevision)") {
            await vm.loadTransaction()
        }
        .onDisappear { vm.cancel() }
        .sheet(isPresented: $showsValidators) {
            StakingValidatorPicker(
                validators: vm.validators, isLoading: vm.isLoading,
                selected: $vm.validatorId)
        }
        .alert(AppLocalization.string("Sign this transaction?"), isPresented: $confirmsSigning) {
            // Asked here only for a review resumed before the password was
            // typed; otherwise the page's password signs.
            if requiresPassword && vm.password.isEmpty {
                SecureField(AppLocalization.string("Wallet Password"), text: $vm.password)
            }
            Button(AppLocalization.string("Cancel"), role: .cancel) {}
            Button(AppLocalization.string("Sign Transaction"), role: .destructive) { vm.begin(.sign) }
                .disabled(requiresPassword && vm.password.isEmpty)
        } message: {
            if let artifact = vm.session.artifact, let intent = artifact.staking {
                let validator = intent.validatorId.flatMap(vm.validatorName).map { "\($0)\n" } ?? ""
                Text(
                    verbatim:
                        "\(intent.action.localizedTitle) · \(chain.displayName)\n\(AmountPresentation.localizedDecimal(artifact.amount)) \(artifact.symbol)\n\(validator)\(artifact.recipient)\n\n\(AppLocalization.string("Signing authorizes this transaction. You will choose nodes and broadcast it separately."))"
                )
            }
        }
        .confirmationDialog(
            AppLocalization.string("Broadcast Transaction"), isPresented: $confirmsBroadcast,
            titleVisibility: .visible
        ) {
            Button(AppLocalization.string("Broadcast Transaction"), role: .destructive) {
                vm.begin(.broadcast)
            }
        } message: {
            Text(AppLocalization.string("staking.broadcast_confirmation"))
        }
    }

    /// The one place the password is typed; every step reads it.
    private var passwordCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            SecureField(AppLocalization.string("Wallet Password"), text: $vm.password)
                .textContentType(.password).padding(SpectraLayout.Space.m).spectraInputFieldStyle()
            Text(AppLocalization.string("staking.password_once")).font(.caption).foregroundStyle(.secondary)
        }.padding(SpectraLayout.cardPadding).spectraCardFill()
    }

    private func mechanics(_ entry: StakingChainEntry) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            Text(AppLocalization.string(entry.shortMechanic)).font(.headline)
            HStack(alignment: .top, spacing: SpectraLayout.Space.m) {
                Text(AppLocalization.string("Minimum Stake")).foregroundStyle(.secondary)
                Spacer()
                Text(AppLocalization.string(entry.minimumStake)).multilineTextAlignment(.trailing)
            }.font(.subheadline)
            HStack(alignment: .top, spacing: SpectraLayout.Space.m) {
                Text(AppLocalization.string("Unbonding")).foregroundStyle(.secondary)
                Spacer()
                Text(AppLocalization.string(entry.unbondingPeriod)).multilineTextAlignment(.trailing)
            }.font(.subheadline)
            DisclosureGroup(AppLocalization.string("How it works")) {
                Text(AppLocalization.string(entry.explanation)).font(.subheadline).foregroundStyle(
                    .secondary
                )
            }
            .disclosureGroupStyle(.spectra)
        }.padding(SpectraLayout.cardPadding).spectraElevatedFill()
    }

    private var preparation: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            HStack {
                Text(vm.action.localizedTitle).font(.headline)
                Spacer()
                if vm.positionId != nil {
                    Button(AppLocalization.string("staking.new_stake")) { vm.startStake() }.buttonStyle(
                        .glass)
                }
            }
            if let position = vm.positionId {
                Text(verbatim: position).font(.caption.monospaced()).foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            if vm.rules.validatorRequired {
                TextField(AppLocalization.string("staking.validator_identifier"), text: $vm.validatorId)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().padding(SpectraLayout.Space.m).spectraInputFieldStyle()
                if let name = vm.validatorName(vm.validatorId) {
                    Label(name, systemImage: "checkmark.seal").font(.caption.weight(.semibold))
                }
                Button(AppLocalization.string("staking.choose_validator")) { showsValidators = true }
                    .buttonStyle(.glass)
            }
            if vm.rules.amountAllowed {
                TextField(
                    AppLocalization.string(vm.rules.amountRequired ? "Amount" : "staking.optional_amount"),
                    text: $vm.amount
                )
                .keyboardType(.decimalPad).padding(SpectraLayout.Space.m).spectraInputFieldStyle()
                    .overlay(alignment: .trailing) {
                        Text(verbatim: chain.gasTokenSymbol).font(.subheadline).foregroundStyle(.secondary)
                            .padding(.trailing, SpectraLayout.Space.m)
                    }
                // What there is to stake. No Max: what a stake must leave for
                // its fee and the network's reserve differs by network, and
                // core does not yet answer it, so the figure is the user's.
                if vm.action == .stake, let available = availableToStake {
                    Text(AppLocalization.format("Available: %@", available)).font(.caption).foregroundStyle(.secondary)
                }
                if !vm.rules.amountRequired {
                    Text(AppLocalization.string("staking.full_withdrawal")).font(.caption).foregroundStyle(
                        .secondary)
                }
            }
            if vm.rules.lockupRequired {
                TextField(AppLocalization.string("staking.dissolve_delay"), text: $vm.lockupSeconds)
                    .keyboardType(.numberPad).padding(SpectraLayout.Space.m).spectraInputFieldStyle()
                Text(AppLocalization.string("staking.dissolve_delay_help")).font(.caption).foregroundStyle(
                    .secondary)
            }
            Text(AppLocalization.string("staking.build_explanation")).font(.caption).foregroundStyle(
                .secondary)
            Button {
                vm.begin(.build)
            } label: {
                Label(AppLocalization.string("Build Transaction"), systemImage: "hammer.fill")
            }.buttonStyle(.glassProminent)
                .disabled(wallet?.signing.isWatchOnly != false || (requiresPassword && vm.password.isEmpty))
                .accessibilityIdentifier("staking.build")
            if vm.isBusy { ProgressView() }
            if let error = vm.stepError {
                StakingErrorText(message: error)
            }
        }.padding(SpectraLayout.cardPadding).spectraCardFill().disabled(vm.isBusy)
    }

    @ViewBuilder private var savedTransactions: some View {
        if !vm.savedArtifacts.isEmpty {
            SpectraRowGroup(
                title: AppLocalization.string("staking.saved_transactions"), data: vm.savedArtifacts
            ) { saved in
                Button {
                    vm.begin(.resume(saved.id))
                } label: {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                        Text(saved.staking?.action.localizedTitle ?? AppLocalization.string("Staking")).font(
                            .subheadline.weight(.semibold))
                        Text(verbatim: "\(AmountPresentation.localizedDecimal(saved.amount)) \(saved.symbol)").font(.caption).foregroundStyle(
                            .secondary)
                        Text(AppLocalization.string(saved.stage == .prepared ? "Awaiting signing" : "Signed"))
                            .font(.caption).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, alignment: .leading).spectraRowPadding()
                }.buttonStyle(.plain).disabled(vm.isBusy)
            }
        }
    }
}
