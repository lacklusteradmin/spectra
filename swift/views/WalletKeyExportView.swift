import SwiftUI

extension WalletKeyKind {
    var title: String {
        switch self {
        case .privateKey: AppLocalization.string("Private Key")
        case .moneroSpendKey: AppLocalization.string("Spend Key")
        case .moneroViewKey: AppLocalization.string("View Key")
        case .accountPublicKey: AppLocalization.string("Account Public Key")
        }
    }

    var subtitle: String {
        switch self {
        case .privateKey: AppLocalization.string("Signs for this wallet in any wallet that imports it.")
        case .moneroSpendKey: AppLocalization.string("With the view key and the address, restores this wallet.")
        case .moneroViewKey: AppLocalization.string("With the address, shows this wallet's funds without spending them.")
        case .accountPublicKey: AppLocalization.string("Watches every address of this account without spending.")
        }
    }

    /// Whether the key can sign: shown only after a warning, never as a QR.
    var spends: Bool { self == .privateKey || self == .moneroSpendKey }
}

/// A wallet's keys, as other wallets import them. Core lists what this
/// wallet can export and writes each in its network's encoding; every export
/// is shown after device authentication, and a copy stays on this device for
/// a minute.
struct WalletKeyExportView: View {
    let store: AppState
    let wallet: WalletView
    @Environment(\.scenePhase) private var scenePhase
    @State private var kinds: [WalletKeyKind] = []
    @State private var password = ""
    @State private var shown: WalletKeyExport?
    @State private var didCopy = false
    @State private var error: String?
    @State private var isWorking = false

    private var needsPassword: Bool { wallet.signing.requiresPassword }

    var body: some View {
        Form {
            Section {
                ForEach(kinds, id: \.self) { kind in
                    Button {
                        Task { await export(kind) }
                    } label: {
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                            Text(kind.title).foregroundStyle(Color.primary)
                            Text(kind.subtitle).font(.caption).foregroundStyle(.secondary)
                        }
                    }.disabled(isWorking || (needsPassword && password.isEmpty))
                }
            } footer: {
                Text(AppLocalization.string("Anyone with a private or spend key controls this wallet's funds; anyone with a view or account key sees all of its activity."))
            }
            if needsPassword {
                Section {
                    SecureField(AppLocalization.string("Wallet Password"), text: $password)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().privacySensitive()
                }
            }
            if let error {
                Section { Text(error).foregroundStyle(.red) }
            }
        }
        .navigationTitle(WalletAction.exportKeys.title).navigationBarTitleDisplayMode(.inline)
        .task {
            kinds = (try? await store.bridge.ready().walletKeyExports(walletId: wallet.id)) ?? []
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .background {
                password = ""
                shown = nil
            }
        }
        .onDisappear {
            password = ""
            shown = nil
        }
        .sheet(item: $shown, onDismiss: { didCopy = false }) { export in
            exportSheet(export)
        }
    }

    private func export(_ kind: WalletKeyKind) async {
        isWorking = true
        defer { isWorking = false }
        error = nil
        if let failure = await store.authenticate(
            .secretMaterial, reason: AppLocalization.format("Authenticate to export a key of %@", wallet.name)) {
            error = failure
            return
        }
        do {
            shown = try await store.bridge.ready().exportWalletKey(
                walletId: wallet.id, kind: kind, password: needsPassword ? password : nil)
            password = ""
        } catch {
            self.error = userErrorMessage(error)
        }
    }

    private func exportSheet(_ export: WalletKeyExport) -> some View {
        NavigationStack {
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    Text(export.format.title).font(.subheadline).foregroundStyle(.secondary)
                    if !export.kind.spends {
                        QRCodeImage(address: export.value).frame(maxWidth: 220).frame(maxWidth: .infinity)
                            .padding(SpectraLayout.Space.m).background(Color.white, in: .rect(cornerRadius: SpectraLayout.Radius.inner))
                    }
                    Text(verbatim: breakableAnywhere(export.value)).font(.body.monospaced()).privacySensitive()
                        .padding(SpectraLayout.Space.m).frame(maxWidth: .infinity, alignment: .leading)
                        .spectraInputFieldStyle(cornerRadius: SpectraLayout.Radius.inner)
                    Button {
                        copySecretToPasteboard(export.value)
                        didCopy = true
                    } label: {
                        Label(AppLocalization.string(didCopy ? "Copied" : "Copy"), systemImage: didCopy ? "checkmark" : "doc.on.doc")
                            .font(.subheadline.weight(.semibold))
                    }.buttonStyle(.glass).tint(.accentColor)
                    Text(AppLocalization.string("A copied key stays on this device and leaves the clipboard after a minute."))
                        .font(.caption).foregroundStyle(.secondary)
                }.secretShield().padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill().padding(SpectraLayout.Space.l)
            }
            .navigationTitle(export.kind.title).navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(AppLocalization.string("Done")) { shown = nil }
                }
            }
        }
    }
}

extension WalletKeyExport: Identifiable {
    public var id: String { "\(kind)" }
}
