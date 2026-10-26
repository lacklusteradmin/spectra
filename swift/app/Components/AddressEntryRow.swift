import SwiftUI
import VisionKit

/// An address typed, pasted, scanned or picked from the address book: the
/// field every page that asks for a recipient uses, so each takes the
/// clipboard, a QR code and a contact alike. A scanned code is read by core
/// against `chain`, so a payment URI yields its address and a code for
/// another network is refused rather than pasted in.
///
/// It is the row's content, in a `Form` row or on a card, and its buttons are
/// borderless so a list row does not fire them all at once.
struct AddressEntryRow: View {
    let title: String
    @Binding var text: String
    let chain: Chain
    /// Saved recipients on `chain`; none hides the contacts menu.
    var contacts: [AddressBookEntry] = []
    @State private var isScanning = false
    @State private var scanProblem: String?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
            TextField(title, text: $text, axis: .vertical)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.callout.monospaced())
                .lineLimit(1...4)
            // The system control reads the clipboard only on the user's tap,
            // so iOS does not ask permission to paste.
            PasteButton(payloadType: String.self) { pasted in
                guard let value = pasted.first else { return }
                text = value.trimmingCharacters(in: .whitespacesAndNewlines)
            }
            .labelStyle(.iconOnly)
            .buttonBorderShape(.circle)
            .controlSize(.small)
            if DataScannerViewController.isSupported {
                Button {
                    if DataScannerViewController.isAvailable {
                        isScanning = true
                    } else {
                        scanProblem = AppLocalization.string(
                            "QR scanning is unavailable right now. Check camera permission and try again.")
                    }
                } label: {
                    Image(systemName: "qrcode.viewfinder").frame(minWidth: 32, minHeight: 32)
                }
                .buttonStyle(.borderless)
                .accessibilityLabel(AppLocalization.string("Scan QR Code"))
            }
            if !contacts.isEmpty {
                Menu {
                    ForEach(contacts) { contact in
                        Button {
                            text = contact.address
                        } label: {
                            Text(contact.name)
                            Text(verbatim: shortAddress(contact.address))
                        }
                    }
                } label: {
                    Image(systemName: "person.crop.circle").frame(minWidth: 32, minHeight: 32)
                }
                .buttonStyle(.borderless)
                .accessibilityLabel(AppLocalization.string("Contacts"))
            }
        }
        .sheet(isPresented: $isScanning) {
            SendQRScannerSheet { payload in
                do {
                    text = try readScannedAddress(chain: chain, payload: payload)
                } catch {
                    scanProblem = userErrorMessage(error)
                }
            }
        }
        .alert(AppLocalization.string("Scan QR Code"), isPresented: .isPresent($scanProblem)) {
            Button(AppLocalization.string("OK"), role: .cancel) {}
        } message: {
            if let scanProblem { Text(verbatim: scanProblem) }
        }
    }
}

/// Both ends of an address, which is how one is recognised: a menu row has
/// no room for the middle.
func shortAddress(_ address: String) -> String {
    guard address.count > 16 else { return address }
    return "\(address.prefix(8))…\(address.suffix(6))"
}
