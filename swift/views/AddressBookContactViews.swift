import Foundation
import SwiftUI
import UIKit

/// Every mainnet, because core validates every mainnet, as the rows the shared
/// chain picker draws.
@MainActor private let addressBookChainDescriptors = ChainSelectionDescriptor.popularOrder(Chain.mainnets)

/// Adding a recipient, on its own page behind the address book's `+`.
struct NewAddressBookContactView: View {
    let addressBook: AddressBookState
    @Environment(\.dismiss) private var dismiss
    @State private var name: String = ""
    @State private var chain: Chain? = Chain.mainnets.first
    @State private var address: String = ""
    @State private var note: String = ""
    @State private var isChoosingChain = false
    @State private var chainSearchText: String = ""
    @State private var isSaving = false
    /// Core's reason for refusing the save, shown here with what was typed.
    @State private var refusal: String?

    private var trimmedAddress: String { address.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var canSave: Bool {
        guard let chain else { return false }
        return canSaveAddressBookEntry(name: name, address: address, chain: chain)
    }

    var body: some View {
        ZStack {
            SpectraBackdrop().ignoresSafeArea()

            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    spectraPageHeader(
                        title: "New Contact",
                        subtitle: "Name a recipient and save an address you send to often.",
                        systemImage: "person.crop.circle.badge.plus"
                    )
                    contactCard
                    destinationCard
                }
                .spectraScreenPadding()
            }
            .scrollDismissesKeyboard(.interactively)
        }
        .safeAreaBar(edge: .bottom) {
            VStack(spacing: SpectraLayout.Space.s) {
                if let refusal {
                    Label(refusal, systemImage: "exclamationmark.triangle.fill")
                        .font(.subheadline).foregroundStyle(.red)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, SpectraLayout.screenHorizontal)
                }
                SpectraBottomActionBar {
                    Button(action: save) {
                        Label(AppLocalization.string("Save Contact"), systemImage: "checkmark")
                            .font(.headline)
                            .frame(maxWidth: .infinity)
                            .frame(minHeight: 46)
                    }
                    .buttonStyle(.glassProminent)
                    .disabled(!canSave || isSaving)
                }
            }
        }
        .navigationTitle(AppLocalization.string("New Contact"))
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .navigationDestination(isPresented: $isChoosingChain) {
            AllChainsSelectionView(
                chainSearchText: $chainSearchText,
                title: AppLocalization.string("import_flow.all_chains_title"),
                descriptors: addressBookChainDescriptors,
                selectedChains: chain.map { Set([$0]) } ?? [],
                toggleSelection: { picked in
                    chain = picked
                    isChoosingChain = false
                }
            )
        }
    }

    /// Closes only once core has stored the contact; a refusal keeps the
    /// form, and what was typed, on screen with core's reason.
    private func save() {
        guard let chain else { return }
        isSaving = true
        Task {
            let refused = await addressBook.add(name: name, address: address, chain: chain, note: note)
            isSaving = false
            if let refused {
                refusal = refused
                // Said here; the list need not say it again on the way back.
                addressBook.error = nil
                spectraNotificationHaptic(.error)
            } else {
                spectraNotificationHaptic(.success)
                dismiss()
            }
        }
    }

    private var contactCard: some View {
        spectraDetailCard(title: "Contact") {
            TextField(AppLocalization.string("Name"), text: $name)
                .textInputAutocapitalization(.words)
                .autocorrectionDisabled()
                .padding(SpectraLayout.Space.m)
                .spectraInputFieldStyle()
                .foregroundStyle(Color.primary)

            TextField(AppLocalization.string("Note (Optional)"), text: $note)
                .textInputAutocapitalization(.sentences)
                .padding(SpectraLayout.Space.m)
                .spectraInputFieldStyle()
                .foregroundStyle(Color.primary)
        }
    }

    private var destinationCard: some View {
        spectraDetailCard(title: "Saved Address") {
            chainRow

            // The placeholder is the chain's own format hint; the address
            // wraps rather than scrolling out of sight, and can be pasted or
            // scanned as on the send page.
            if let chain {
                AddressEntryRow(title: addressPrompt, text: $address, chain: chain)
                    .padding(SpectraLayout.Space.m)
                    .spectraInputFieldStyle()
                    .foregroundStyle(Color.primary)
            }

            // Only show validation feedback once there is input to judge, and
            // never by colour alone.
            if !trimmedAddress.isEmpty {
                Label(addressValidationMessage, systemImage: isAddressValid ? "checkmark.circle.fill" : "exclamationmark.circle")
                    .font(.caption)
                    .foregroundStyle(isAddressValid ? Color.green : Color.spectraWarning)
            }
        }
    }

    private var chainRow: some View {
        let badge = AssetHolding.nativeChainBadge(for: chain) ?? (nil, Color.mint)
        let chainName = chain?.displayName ?? AppLocalization.string("Select a chain")

        return Button {
            spectraHaptic(.light)
            isChoosingChain = true
        } label: {
            HStack(spacing: SpectraLayout.Space.m) {
                CoinBadge(
                    artworkName: badge.artworkName,
                    fallbackText: chainName,
                    color: badge.color,
                    size: 34
                )
                VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                    Text(AppLocalization.string("Chain"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text(chainName)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(Color.primary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(SpectraLayout.Space.m)
            .contentShape(Rectangle())
            .spectraInputFieldStyle(cornerRadius: SpectraLayout.Radius.inner)
        }
        .buttonStyle(.plain)
    }

    /// A terse example of what an address on this chain looks like.
    private var addressPrompt: String {
        let hint = chain?.addressPrefixHint ?? ""
        return hint.isEmpty ? AppLocalization.string("Address") : hint
    }

    private var addressValidationMessage: String {
        guard let chain else { return AppLocalization.string("Select a chain first.") }
        return addressBookAddressValidationMessage(for: address, chain: chain)
    }

    private var isAddressValid: Bool {
        guard let chain else { return false }
        return isValidSendAddress(chain: chain, address: trimmedAddress)
    }
}

/// A saved recipient with rename and delete actions.
struct AddressBookContactView: View {
    let store: AppState
    let entry: AddressBookEntry
    private var addressBook: AddressBookState { store.addressBook }
    @Environment(\.dismiss) private var dismiss
    @State private var editedName: String = ""
    @State private var isConfirmingDelete = false
    @State private var didCopy = false

    /// The stored contact, so a rename is reflected here rather than leaving
    /// the pushed page showing the name it was opened with.
    private var contact: AddressBookEntry {
        addressBook.entries.first { $0.id == entry.id } ?? entry
    }

    private var canRename: Bool {
        let trimmed = editedName.trimmingCharacters(in: .whitespacesAndNewlines)
        return !trimmed.isEmpty && trimmed != contact.name
    }

    var body: some View {
        ZStack {
            SpectraBackdrop().ignoresSafeArea()

            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    contactHero
                    sendButton
                    labelCard
                    deleteButton
                }
                .spectraScreenPadding()
            }
            .scrollDismissesKeyboard(.interactively)
        }
        .navigationTitle(AppLocalization.string("Contact"))
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(AppLocalization.string("Save")) {
                    addressBook.rename(id: contact.id, to: editedName)
                    spectraHaptic(.light)
                }
                .disabled(!canRename)
            }
        }
        .onAppear { editedName = contact.name }
        .task(id: didCopy) {
            guard didCopy else { return }
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            didCopy = false
        }
    }

    private var contactHero: some View {
        let badge = AssetHolding.nativeChainBadge(for: contact.chainId) ?? (nil, Color.mint)

        return VStack(spacing: SpectraLayout.Space.m) {
            CoinBadge(
                artworkName: badge.artworkName,
                fallbackText: contact.chainName,
                color: badge.color,
                size: 56
            )

            VStack(spacing: SpectraLayout.Space.xs) {
                Text(contact.name)
                    .font(.title3.weight(.semibold))
                    .multilineTextAlignment(.center)
                Text(contact.subtitleText)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }

            // Whole and wrapped here, where there is room for it: the list row
            // shows the same address elided.
            Text(contact.address)
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .multilineTextAlignment(.center)
                .textSelection(.enabled)

            Button {
                UIPasteboard.general.string = contact.address
                didCopy = true
                spectraHaptic(.light)
            } label: {
                Label(
                    AppLocalization.string(didCopy ? "Copied" : "Copy"),
                    systemImage: didCopy ? "checkmark" : "doc.on.doc"
                )
                .font(.subheadline.weight(.semibold))
                .frame(maxWidth: .infinity)
                .padding(.vertical, SpectraLayout.Space.s)
            }
            .buttonStyle(.glass)
        }
        .frame(maxWidth: .infinity)
        .padding(SpectraLayout.Space.l)
        .spectraElevatedFill()
    }

    /// A wallet that can pay this contact: one on its network that signs.
    private var payingWallet: WalletView? {
        store.sendEnabledWallets.first { $0.chain == contact.chainId }
    }

    /// Opens the composer from that wallet with this address filled in.
    @ViewBuilder
    private var sendButton: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Button {
                guard let payingWallet else { return }
                spectraHaptic(.light)
                store.beginSend(walletId: payingWallet.id)
                store.sendFlow.address = contact.address
            } label: {
                Label(AppLocalization.format("Send to %@", contact.name), systemImage: "arrow.up")
                    .font(.subheadline.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, SpectraLayout.Space.s)
            }
            .buttonStyle(.glassProminent)
            .disabled(payingWallet == nil)
            if payingWallet == nil {
                Text(AppLocalization.format("No wallet on %@ can send yet.", contact.chainName))
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private var labelCard: some View {
        spectraDetailCard(title: "Label") {
            Text(
                AppLocalization.string(
                    "You can update the label for this saved address. The chain, address, and note stay fixed.")
            )
            .spectraHintText()

            TextField(AppLocalization.string("Name"), text: $editedName)
                .textInputAutocapitalization(.words)
                .autocorrectionDisabled()
                .padding(SpectraLayout.Space.m)
                .spectraInputFieldStyle()
                .foregroundStyle(Color.primary)
        }
    }

    /// The dialog hangs off the button rather than the page, so the popover it
    /// becomes points at what it is asking about instead of at the title bar.
    private var deleteButton: some View {
        Button(role: .destructive) {
            spectraHaptic(.light)
            isConfirmingDelete = true
        } label: {
            Label(AppLocalization.string("Delete Contact"), systemImage: "trash")
                .font(.subheadline.weight(.semibold))
                .frame(maxWidth: .infinity)
                .padding(.vertical, SpectraLayout.Space.m)
        }
        .buttonStyle(.glass)
        .tint(.red)
        .confirmationDialog(
            AppLocalization.string("Delete Contact"),
            isPresented: $isConfirmingDelete,
            titleVisibility: .visible
        ) {
            Button(AppLocalization.string("Delete"), role: .destructive) {
                spectraHaptic(.medium)
                addressBook.remove(id: contact.id)
                dismiss()
            }
            Button(AppLocalization.string("Cancel"), role: .cancel) {}
        }
    }
}

/// Naming a recipient just sent to before it is saved: a name of the user's,
/// prefilled, and core's answer shown here rather than on a page the user is
/// not on.
struct SaveContactSheet: View {
    let addressBook: AddressBookState
    let chain: Chain
    let address: String
    @State var name: String
    @Environment(\.dismiss) private var dismiss
    @State private var refusal: String?
    @State private var isSaving = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(AppLocalization.string("Name"), text: $name)
                        .textInputAutocapitalization(.words)
                        .autocorrectionDisabled()
                } footer: {
                    Text(verbatim: address).font(.caption.monospaced())
                }
                if let refusal {
                    Section {
                        Label(refusal, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle(AppLocalization.string("Save Contact"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(AppLocalization.string("Cancel")) { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(AppLocalization.string("Save"), action: save)
                        .disabled(isSaving || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
        .presentationDetents([.medium])
    }

    private func save() {
        isSaving = true
        Task {
            let refused = await addressBook.add(
                name: name, address: address, chain: chain, note: AppLocalization.string("Saved from recent send"))
            isSaving = false
            if let refused {
                refusal = refused
                addressBook.error = nil
            } else {
                spectraNotificationHaptic(.success)
                dismiss()
            }
        }
    }
}
