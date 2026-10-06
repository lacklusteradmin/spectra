import Foundation

/// Saved recipients, as core last committed them.
///
/// Core owns the list, the rules about what may be saved, and the
/// persistence. Every change here is a command through the shared queue, and
/// what core commits comes back through `adopt`.
@MainActor
@Observable
final class AddressBookState {
    @ObservationIgnored private let commands: StateCommandQueue // Where edits go; not view state.
    /// Core's list as last adopted. `private(set)`, because assigning to it
    /// would only desynchronise it from core.
    private(set) var entries: [AddressBookEntry] = []
    /// Why core refused the last address-book change, if it did.
    var error: String?

    init(commands: StateCommandQueue) { self.commands = commands }

    /// The only writer of `entries`, called with each committed core state.
    func adopt(_ entries: [AddressBookEntry]) {
        if self.entries != entries { self.entries = entries }
    }

    /// Save a recipient. Core trims, normalizes the address, validates it,
    /// rejects duplicates and assigns the entry's id.
    func add(name: String, address: String, chain: Chain, note: String = "") {
        send(.addAddressBookEntry(name: name, chainId: chain, address: address, note: note))
    }
    func saveRecipient(of transaction: TransactionRecord) {
        guard transaction.kind == .send else { return }
        add(
            name: AppLocalization.format("%@ Recipient", transaction.symbol), address: transaction.address,
            chain: transaction.chain, note: AppLocalization.string("Saved from recent send"))
    }
    func rename(id: String, to newName: String) {
        send(.renameAddressBookEntry(id: id, name: newName))
    }
    func remove(id: String) {
        send(.removeAddressBookEntry(id: id))
    }

    /// A refusal arrives as an `addressBookRejected` event carrying the reason
    /// core decided on.
    private func send(_ command: StateCommand) {
        commands.enqueue(command) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let transition):
                self.error = nil
                for case .addressBookRejected(let reason) in transition.events {
                    self.error = addressBookRejectionMessage(reason)
                }
            case .failure(let error):
                self.error = userErrorMessage(error)
            }
        }
    }
}

private func addressBookRejectionMessage(_ reason: AddressBookRejection) -> String {
    switch reason {
    case .emptyName: return AppLocalization.string("Enter a name for this contact.")
    case .invalidAddress: return AppLocalization.string("That address is not valid for this chain.")
    case .duplicateAddress: return AppLocalization.string("That address is already saved.")
    }
}

/// Enables the save button. Core still validates the address and refuses a
/// duplicate — whether two addresses are the same recipient is its rule.
func canSaveAddressBookEntry(name: String, address: String, chain: Chain) -> Bool {
    let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
    return !trimmedName.isEmpty && isValidSendAddress(chain: chain, address: address)
}

func addressBookAddressValidationMessage(for address: String, chain: Chain) -> String {
    let trimmed = address.trimmingCharacters(in: .whitespacesAndNewlines)
    let isEmpty = trimmed.isEmpty
    if !isEmpty, isValidSendAddress(chain: chain, address: trimmed) {
        return AppLocalization.format("Valid %@ address.", chain.displayName)
    }

    // The sentence a chain has of its own, looked up by id. These are
    // content, so they live in the locale files keyed by chain id; a chain
    // with none falls back to a template built from the catalog's
    // `address_prefix_hint`.
    let key = "addressHint.\(chain.id).\(isEmpty ? "empty" : "invalid")"
    let localized = AppLocalization.string(key)
    if localized != key { return localized }

    let hint = chain.addressPrefixHint
    guard !hint.isEmpty else {
        return isEmpty
            ? AppLocalization.string("Enter an address for the selected chain.")
            : AppLocalization.format("Enter a valid %@ address.", chain.displayName)
    }
    return isEmpty
        ? AppLocalization.format("%@ addresses look like %@", chain.displayName, hint)
        : AppLocalization.format("Enter a valid %@ address — they look like %@", chain.displayName, hint)
}
