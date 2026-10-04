import SwiftUI

struct StakingView: View {
    @Bindable var store: AppState

    var body: some View {
        NavigationStack {
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                        Label(AppLocalization.string("Staking"), systemImage: "link.circle.fill")
                            .font(.title3.weight(.bold))
                        Text(AppLocalization.string("staking.intro"))
                            .font(.subheadline).foregroundStyle(.secondary)
                    }
                    .padding(SpectraLayout.cardPadding).frame(maxWidth: .infinity, alignment: .leading)
                    .spectraElevatedFill()
                    SpectraRowGroup(
                        title: AppLocalization.string("Supported Chains"),
                        trailing: "\(CoreReferenceTables.stakingChains.count)",
                        data: CoreReferenceTables.stakingChains
                    ) { entry in
                        NavigationLink(value: entry.chain) {
                            HStack(spacing: SpectraLayout.Space.m) {
                                CoinBadge(
                                    artworkName: AssetPresentationCatalog.artwork(
                                        deploymentId: entry.chain.entry?.nativeDeploymentId),
                                    fallbackText: entry.chain.gasTokenSymbol,
                                    color: entry.chain.entry?.color.color ?? .accentColor, size: 36)
                                Text(entry.chain.displayName).font(.headline).foregroundStyle(.primary)
                                Spacer(minLength: SpectraLayout.Space.s)
                                Image(systemName: "chevron.right").foregroundStyle(.tertiary)
                            }.spectraRowPadding()
                        }.buttonStyle(.plain)
                    }
                }.spectraScreenPadding()
            }
            .background(SpectraBackdrop().ignoresSafeArea())
            .navigationTitle(AppLocalization.string("Staking"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar)
            .navigationDestination(for: Chain.self) { ChainStakingDetailView(chain: $0, store: store) }
        }
    }
}

extension StakingChainEntry: Identifiable { public var id: Chain { chain } }

struct ChainStakingDetailView: View {
    let chain: Chain
    @Bindable var store: AppState
    @State private var vm: StakingViewModel
    @State private var showsValidators = false
    @State private var confirmsSigning = false
    @State private var confirmsBroadcast = false

    init(chain: Chain, store: AppState) {
        self.chain = chain
        self.store = store
        _vm = State(wrappedValue: StakingViewModel(chain: chain, bridge: store.bridge))
    }

    private var wallets: [WalletView] { store.wallets.filter { $0.address(on: chain) != nil } }
    private var wallet: WalletView? { store.wallet(for: vm.walletId) }
    private var requiresPassword: Bool { wallet?.signing.requiresPassword ?? true }

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                if let entry = CoreReferenceTables.stakingEntry(for: chain) { mechanics(entry) }
                walletPicker
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
                if let error = vm.error ?? vm.session.error {
                    Text(verbatim: error).font(.subheadline).foregroundStyle(.red)
                        .accessibilityIdentifier("staking.error")
                }
            }.spectraScreenPadding()
        }
        .scrollDismissesKeyboard(.interactively)
        .background(SpectraBackdrop().ignoresSafeArea())
        .navigationTitle(chain.displayName).navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .task {
            if vm.walletId.isEmpty, let first = wallets.first { vm.selectWallet(first.id) }
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
            if requiresPassword {
                SecureField(AppLocalization.string("Wallet Password"), text: $vm.password)
            }
            Button(AppLocalization.string("Cancel"), role: .cancel) { vm.password = "" }
            Button(AppLocalization.string("Sign Transaction"), role: .destructive) { vm.begin(.sign) }
                .disabled(requiresPassword && vm.password.isEmpty)
        } message: {
            if let artifact = vm.session.artifact, let intent = artifact.staking {
                Text(
                    verbatim:
                        "\(intent.action.localizedTitle) · \(chain.displayName)\n\(artifact.amount) \(artifact.symbol)\n\(artifact.recipient)\n\n\(AppLocalization.string("Signing authorizes this transaction. You will choose nodes and broadcast it separately."))"
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
        .onChange(of: confirmsSigning) { _, showing in
            if !showing && vm.request?.id == nil { vm.password = "" }
        }
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
                .padding(.top, SpectraLayout.Space.s)
            }
        }.padding(SpectraLayout.cardPadding).spectraElevatedFill()
    }

    private var walletPicker: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            if wallets.isEmpty {
                Text(AppLocalization.string("staking.no_wallet")).font(.subheadline).foregroundStyle(
                    .secondary)
            } else {
                Picker(
                    AppLocalization.string("Wallet"),
                    selection: Binding(get: { vm.walletId }, set: { vm.selectWallet($0) })
                ) {
                    ForEach(wallets) { wallet in Text(wallet.name).tag(wallet.id) }
                }.disabled(vm.isBusy)
                if let wallet, let address = wallet.address(on: chain) {
                    Text(verbatim: address).font(.caption.monospaced()).foregroundStyle(.secondary)
                        .textSelection(.enabled)
                    if wallet.signing.isWatchOnly {
                        Text(AppLocalization.string("staking.watch_only")).font(.caption).foregroundStyle(
                            .secondary)
                    }
                }
            }
        }.padding(SpectraLayout.cardPadding).spectraCardFill()
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
                    .textInputAutocapitalization(.never).autocorrectionDisabled().spectraInputFieldStyle()
                Button(AppLocalization.string("staking.choose_validator")) { showsValidators = true }
                    .buttonStyle(.glass)
            }
            if vm.rules.amountAllowed {
                TextField(
                    AppLocalization.string(vm.rules.amountRequired ? "Amount" : "staking.optional_amount"),
                    text: $vm.amount
                )
                .keyboardType(.decimalPad).spectraInputFieldStyle()
                Text(verbatim: chain.gasTokenSymbol).font(.caption).foregroundStyle(.secondary)
                if !vm.rules.amountRequired {
                    Text(AppLocalization.string("staking.full_withdrawal")).font(.caption).foregroundStyle(
                        .secondary)
                }
            }
            if vm.rules.lockupRequired {
                TextField(AppLocalization.string("staking.dissolve_delay"), text: $vm.lockupSeconds)
                    .keyboardType(.numberPad).spectraInputFieldStyle()
                Text(AppLocalization.string("staking.dissolve_delay_help")).font(.caption).foregroundStyle(
                    .secondary)
            }
            if requiresPassword {
                SecureField(AppLocalization.string("Wallet Password"), text: $vm.password)
                    .spectraInputFieldStyle()
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
                        Text(verbatim: "\(saved.amount) \(saved.symbol)").font(.caption).foregroundStyle(
                            .secondary)
                        Text(AppLocalization.string(saved.stage == .prepared ? "Awaiting signing" : "Signed"))
                            .font(.caption).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, alignment: .leading).spectraRowPadding()
                }.buttonStyle(.plain).disabled(vm.isBusy)
            }
        }
    }
}
