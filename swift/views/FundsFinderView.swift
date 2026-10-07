import SwiftUI

/// One account the scan found used: funded, or with history.
struct FundsFinderHit: Identifiable {
    let id = UUID()
    let candidate: FundsFinderCandidate
    /// The balance, or `nil` for an account used and since emptied.
    let balanceDisplay: String?
}

/// Scan a seed's derivation paths for funded addresses.
///
/// The scan's state is this screen's: nothing else reads it, and leaving the
/// screen cancels it. It sat on `AppState` as six underscore-prefixed
/// properties behind six forwarding ones, with a comment claiming
/// `@Observable` required that shape.
struct FundsFinderView: View {
    let store: AppState
    /// The same entry the import page reads a phrase with, so a phrase the
    /// import would refuse is not scanned here either.
    @State private var seedEntry = SeedPhraseEntry()
    @State private var passphrase: String = ""
    @State private var showPassphrase: Bool = false
    @State private var hasStarted: Bool = false
    @State private var isScanning = false
    @State private var progress: Double = 0
    @State private var hits: [FundsFinderHit] = []
    @State private var checkedCount = 0
    @State private var totalCount = 0
    /// Why the scan itself stopped.
    @State private var scanError: String?
    /// Why a chain's addresses could not be read: the first failure on each
    /// chain, worded like any other failed call.
    @State private var unreadChains: [Chain: String] = [:]
    @State private var scanTask: Task<Void, Never>?

    private var canStart: Bool { seedEntry.verdict.isValid && !isScanning }

    var body: some View {
        ZStack {
            SpectraBackdrop().ignoresSafeArea()
            ScrollView(showsIndicators: false) {
                VStack(spacing: SpectraLayout.Space.m) {
                    if !hasStarted {
                        inputSection
                    } else {
                        scanProgressSection
                        if !hits.isEmpty {
                            hitsSection
                        }
                        ForEach(Chain.all.filter { unreadChains[$0] != nil }) { chain in
                            errorBanner(AppLocalization.format(
                                "%@ addresses could not be checked: %@", chain.displayName, unreadChains[chain] ?? ""))
                        }
                        if let error = scanError {
                            errorBanner(error)
                        }
                        // Unread addresses are not empty ones.
                        if !isScanning && hits.isEmpty && scanError == nil && unreadChains.isEmpty {
                            emptyResultsSection
                        }
                    }
                }
                .padding(.horizontal, SpectraLayout.Space.l)
                .padding(.top, SpectraLayout.Space.l)
                .padding(.bottom, SpectraLayout.Space.xxl)
            }
        }
        .navigationTitle(AppLocalization.string("Funds Finder"))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if hasStarted && !isScanning {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(AppLocalization.string("New Scan")) {
                        resetScan()
                        hasStarted = false
                        seedEntry.reset()
                        passphrase = ""
                    }
                }
            }
        }
        .onDisappear {
            resetScan()
        }
        .navigationDestination(
            isPresented: Binding(
                get: { store.walletImport.isPresented && store.walletImport.editingWalletId == nil },
                set: { isPresented in
                    if !isPresented { store.walletImport.isPresented = false }
                }
            )
        ) {
            SetupView(store: store, draft: store.walletImport.draft)
        }
    }

    /// A found account opens the single-wallet import on its network, with
    /// the phrase, passphrase, profile and account it was found at.
    private func importHit(_ hit: FundsFinderHit) {
        let words = seedEntry.verdict.words
        let phrasePassphrase = passphrase
        store.beginWalletSetup(chain: hit.candidate.chainId, method: .importPhrase)
        let draft = store.walletImport.draft
        draft.seedEntry.load(words, wordCount: words.count)
        draft.overridePassphrase = phrasePassphrase
        if let profile = hit.candidate.profile {
            draft.derivationProfile = profile
            draft.derivationAccount = hit.candidate.account
        }
    }

    // MARK: - Scan

    private func startScan(seedPhrase: String, passphrase: String?) {
        guard !isScanning else { return }
        resetScan()
        isScanning = true
        scanTask = Task { @MainActor in
            do {
                let service = try store.bridge.service()
                let request = FundsFinderRequest(seedPhrase: seedPhrase, passphrase: passphrase)
                // Deriving every candidate address from the seed is synchronous
                // work; keep it off the main actor.
                let scan = try await Task.detached {
                    try service.beginFundsScan(request: request, chainId: nil)
                }.value
                guard !Task.isCancelled else { return }
                repeat {
                    let batch = await scan.nextBatch()
                    guard !Task.isCancelled else { return }
                    totalCount = Int(batch.total)
                    checkedCount = Int(batch.checked)
                    progress = batch.total == 0 ? 1 : Double(batch.checked) / Double(batch.total)
                    for read in batch.reads {
                        if let error = read.error, unreadChains[read.candidate.chainId] == nil {
                            unreadChains[read.candidate.chainId] = userErrorMessage(error)
                        }
                        if read.used {
                            hits.append(FundsFinderHit(
                                candidate: read.candidate,
                                balanceDisplay: read.funded ? read.balance?.amountDisplay : nil))
                        }
                    }
                    if batch.complete { break }
                } while !Task.isCancelled
            } catch {
                if !Task.isCancelled { scanError = userErrorMessage(error) }
            }
            isScanning = false
        }
    }

    private func resetScan() {
        scanTask?.cancel()
        scanTask = nil
        isScanning = false
        progress = 0
        hits = []
        checkedCount = 0
        totalCount = 0
        scanError = nil
        unreadChains = [:]
    }

    // MARK: - Input section

    private var inputSection: some View {
        VStack(spacing: SpectraLayout.Space.m) {
            headerCard
            seedPhraseCard
            passphraseCard
            disclaimerCard
            startButton
        }
    }

    private var headerCard: some View {
        HStack(alignment: .top, spacing: SpectraLayout.Space.m) {
            Image(systemName: "magnifyingglass.circle.fill")
                .font(.system(size: 32, weight: .semibold))
                .foregroundStyle(.tint)
                .frame(width: 36, height: 36)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                Text(AppLocalization.string("Scan All Derivation Paths"))
                    .font(.headline)
                Text(AppLocalization.string("Enter your seed phrase to check every network's derivation profiles at their first three accounts, revealing which accounts hold funds or have been used."))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.leading)
            }
        }
        .padding(SpectraLayout.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }

    private var seedPhraseCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(AppLocalization.string("Seed Phrase")).font(.subheadline.weight(.semibold))
            SeedPhraseEntryView(entry: seedEntry)
        }
        .padding(SpectraLayout.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }

    private var passphraseCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            HStack {
                Text(AppLocalization.string("BIP-39 Passphrase")).font(.subheadline.weight(.semibold))
                Spacer()
                Text(AppLocalization.string("Optional")).font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Group {
                    if showPassphrase {
                        TextField(AppLocalization.string("Leave blank if none"), text: $passphrase)
                    } else {
                        SecureField(AppLocalization.string("Leave blank if none"), text: $passphrase)
                    }
                }
                .font(.body)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                Button {
                    showPassphrase.toggle()
                } label: {
                    Image(systemName: showPassphrase ? "eye.slash" : "eye")
                        .foregroundStyle(.secondary)
                        .font(.subheadline)
                }
            }
            .padding(SpectraLayout.Space.m)
            .spectraInputFieldStyle(cornerRadius: SpectraLayout.Radius.inner)
            Text(AppLocalization.string("A passphrase creates a different wallet. Leave blank unless you set one up."))
                .font(.caption).foregroundStyle(.secondary)
        }
        .padding(SpectraLayout.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }

    private var disclaimerCard: some View {
        HStack(alignment: .top, spacing: SpectraLayout.Space.s) {
            Image(systemName: "lock.shield.fill")
                .foregroundStyle(.green)
                .font(.subheadline)
            Text(AppLocalization.string("Your seed phrase never leaves this device. Derivation and balance checks happen locally and via your configured RPC endpoints."))
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.leading)
        }
        .padding(SpectraLayout.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }

    private var startButton: some View {
        Button {
            guard canStart else { return }
            hasStarted = true
            startScan(
                seedPhrase: seedEntry.phrase,
                passphrase: passphrase.isEmpty ? nil : passphrase
            )
        } label: {
            Label(AppLocalization.string("Start Scan"), systemImage: "magnifyingglass")
                .font(.headline)
                .frame(maxWidth: .infinity)
                .padding(.vertical, SpectraLayout.Space.l)
        }
        .buttonStyle(.borderedProminent)
        .disabled(!canStart)
        .clipShape(RoundedRectangle(cornerRadius: SpectraLayout.Radius.inner, style: .continuous))
    }

    // MARK: - Progress section

    private var scanProgressSection: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            HStack {
                if isScanning {
                    SpectraLoadingGlyph(size: 26, tint: .accentColor)
                    Text(AppLocalization.string("Scanning…"))
                        .font(.headline)
                } else {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                    Text(AppLocalization.string("Scan Complete"))
                        .font(.headline)
                }
                Spacer()
                if totalCount > 0 {
                    Text(AppLocalization.format("%lld / %lld",
                        checkedCount, totalCount))
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
            if totalCount > 0 {
                ProgressView(value: progress)
                    .progressViewStyle(.linear)
            }
            if isScanning {
                Text(AppLocalization.string("Checking addresses across all derivation paths…"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else if hits.isEmpty, unreadChains.isEmpty {
                Text(AppLocalization.format("Checked %lld paths — no funds found", count: checkedCount, checkedCount))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else if !hits.isEmpty {
                Text(AppLocalization.format("Found %lld paths with funds across %lld checked",
                    count: hits.count, hits.count, checkedCount))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(SpectraLayout.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }

    // MARK: - Hits section

    private var hitsSection: some View {
        SpectraRowGroup(
            title: AppLocalization.format("%lld paths with funds found", count: hits.count, hits.count),
            data: hits, dividerInset: SpectraLayout.rowHorizontal
        ) { hit in
            FundsFinderHitRow(hit: hit) { importHit(hit) }
        }
    }

    private var emptyResultsSection: some View {
        VStack(spacing: SpectraLayout.Space.s) {
            Image(systemName: "tray").font(.system(size: 32)).foregroundStyle(.secondary)
            Text(AppLocalization.string("No funds found")).font(.headline)
            Text(AppLocalization.string("No balance was detected at any of the scanned derivation paths. Double-check your seed phrase and try with a BIP-39 passphrase if you set one."))
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(SpectraLayout.Space.l)
        .frame(maxWidth: .infinity)
        .spectraCardFill()
    }

    @ViewBuilder
    private func errorBanner(_ message: String) -> some View {
        HStack(alignment: .top, spacing: SpectraLayout.Space.s) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.spectraWarning)
            Text(message).font(.caption).foregroundStyle(.secondary)
        }
        .padding(SpectraLayout.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }
}

// MARK: - Hit row

private struct FundsFinderHitRow: View {
    let hit: FundsFinderHit
    let importAccount: () -> Void
    @State private var isCopied = false

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            HStack {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(hit.candidate.chainName)
                        .font(.subheadline.weight(.semibold))
                    if let profile = hit.candidate.profileTitle {
                        Text(profile)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer()
                Text(hit.balanceDisplay ?? AppLocalization.string("Used"))
                    .font(.subheadline.weight(.bold))
                    .foregroundStyle(.tint)
            }
            HStack(spacing: SpectraLayout.Space.xs) {
                Text(hit.candidate.derivationPath)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer(minLength: SpectraLayout.Space.xs)
                Button {
                    UIPasteboard.general.string = hit.candidate.address
                    isCopied = true
                    Task {
                        try? await Task.sleep(nanoseconds: 1_500_000_000)
                        isCopied = false
                    }
                } label: {
                    Label(
                        isCopied ? AppLocalization.string("Copied") : AppLocalization.string("Copy Address"),
                        systemImage: isCopied ? "checkmark" : "doc.on.doc"
                    )
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(isCopied ? .green : .secondary)
                }
                .buttonStyle(.plain)
                .animation(.spring(duration: 0.25), value: isCopied)
            }
            Text(hit.candidate.address)
                .font(.caption.monospaced())
                .foregroundStyle(.tertiary)
                .lineLimit(1)
                .truncationMode(.middle)
            Button(AppLocalization.string("Import This Account"), action: importAccount)
                .font(.caption.weight(.semibold)).buttonStyle(.glass)
        }
        .spectraRowPadding()
    }
}
