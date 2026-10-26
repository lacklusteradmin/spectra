import Foundation
import SwiftUI
import UIKit

/// Saved recipients, with adding a contact as a toolbar action.
struct AddressBookView: View {
    let store: AppState
    private var addressBook: AddressBookState { store.addressBook }
    @State private var isAddingContact = false
    @State private var openContact: AddressBookEntry?

    var body: some View {
        ZStack {
            SpectraBackdrop().ignoresSafeArea()

            ScrollView(showsIndicators: false) {
                LazyVStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
                    spectraPageHeader(
                        title: "Address Book",
                        subtitle: "Pick a saved recipient during a send instead of pasting an address.",
                        systemImage: "person.crop.circle"
                    )

                    rejectionNotice

                    if addressBook.entries.isEmpty {
                        SpectraEmptyStateCard(
                            title: "No saved addresses yet",
                            message: "Save frequent recipients here so future sends are faster.",
                            systemImage: "person.crop.circle.badge.plus",
                            actionTitle: "New Contact",
                            actionSystemImage: "plus",
                            action: {
                                spectraHaptic(.light)
                                isAddingContact = true
                            }
                        )
                    } else {
                        // One card, a row per contact.
                        SpectraRowGroup(data: addressBook.entries) { entry in
                            AddressBookContactRow(entry: entry) { openContact = entry }
                        }
                    }
                }
                .spectraScreenPadding()
            }
        }
        .navigationTitle(AppLocalization.string("Address Book"))
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    spectraHaptic(.light)
                    isAddingContact = true
                } label: {
                    Image(systemName: "plus")
                }
                .accessibilityLabel(AppLocalization.string("New Contact"))
            }
        }
        .navigationDestination(isPresented: $isAddingContact) {
            NewAddressBookContactView(addressBook: addressBook)
        }
        .navigationDestination(item: $openContact) { entry in
            AddressBookContactView(store: store, entry: entry)
        }
    }

    /// Core's reason for refusing a contact. `addressBook.error` has been set
    /// since address-book commands moved to core and no screen showed it, so a
    /// refused save was indistinguishable from a save that did nothing.
    @ViewBuilder
    private var rejectionNotice: some View {
        if let addressBookError = addressBook.error {
            HStack(alignment: .top, spacing: SpectraLayout.Space.m) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.red)
                Text(verbatim: addressBookError)
                    .font(.subheadline)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button {
                    addressBook.error = nil
                } label: {
                    Image(systemName: "xmark")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(AppLocalization.string("Close"))
            }
            .padding(SpectraLayout.Space.l)
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassEffect(
                .regular.tint(Color.red.opacity(0.12)),
                in: .rect(cornerRadius: SpectraLayout.Radius.card)
            )
        }
    }
}

/// One saved recipient. The row opens the contact; copy is its own button.
private struct AddressBookContactRow: View {
    let entry: AddressBookEntry
    let onOpen: () -> Void

    var body: some View {
        let badge = AssetHolding.nativeChainBadge(for: entry.chainId) ?? (nil, Color.mint)

        HStack(spacing: SpectraLayout.Space.xs) {
            Button(action: onOpen) {
                HStack(spacing: SpectraLayout.Space.m) {
                    CoinBadge(
                        artworkName: badge.artworkName,
                        fallbackText: entry.chainName,
                        color: badge.color,
                        size: 36
                    )

                    VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                        Text(entry.name)
                            .font(.headline)
                            .foregroundStyle(Color.primary)
                            .lineLimit(1)
                        Text(entry.subtitleText)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                        // One line, elided in the middle: an address is
                        // recognised by both ends, and wrapping it to two
                        // monospaced lines made it, rather than the name, the
                        // largest thing in the row.
                        Text(entry.address)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }

                    Spacer(minLength: 0)
                }
                .padding(.leading, SpectraLayout.rowHorizontal)
                .padding(.vertical, SpectraLayout.rowVertical)
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            CopyButton(value: entry.address)
                .padding(.trailing, SpectraLayout.Space.xs)
        }
    }
}
