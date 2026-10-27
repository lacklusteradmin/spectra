import SwiftUI
import VisionKit

/// An address typed, pasted, scanned or picked from the address book: the
/// field every page that asks for a recipient uses — the send composer, a
/// wallet tool's sheet, a contact — so each takes the clipboard, a QR code
/// and a contact alike. A scanned code is read by core against `chain`, so a
/// payment URI yields its address and a code for another network is refused
/// rather than pasted in.
///
/// It is the row's content, in a `Form` row or on a card, and its buttons are
/// borderless so a list row does not fire them all at once.
struct AddressEntryRow: View {
    let title: String
    @Binding var text: String
    let chain: Chain
    /// Saved recipients on `chain`; none hides the contacts menu.
    var contacts: [AddressBookEntry] = []
    /// Reads a scanned code into the address it names. The default reads the
    /// address alone; the send composer reads the whole payment request.
    var readScan: (@MainActor (String) throws -> String)? = nil
    @State private var isFocused = false
    @State private var isScanning = false
    @State private var scanProblem: String?

    var body: some View {
        // No maximum line count: a long address remains visible at every
        // Dynamic Type size instead of scrolling inside a small field.
        HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
            AddressInput(text: $text, isFocused: $isFocused, prompt: title)
                .onChange(of: text) { _, address in
                    // A multi-line field takes Return as a newline; here it
                    // means done, and no address holds one.
                    guard address.contains(where: \.isNewline) else { return }
                    text = address.filter { !$0.isNewline }
                    isFocused = false
                }
                // A text view's own width is its text's: empty, none.
                .frame(maxWidth: .infinity, alignment: .leading)
            // The system control reads the clipboard only on the user's tap,
            // so iOS does not ask permission to paste.
            PasteButton(payloadType: String.self) { pasted in
                guard let value = pasted.first else { return }
                text = value.trimmingCharacters(in: .whitespacesAndNewlines)
                isFocused = false
            }
            .labelStyle(.iconOnly)
            .buttonBorderShape(.circle)
            .controlSize(.small)
            if DataScannerViewController.isSupported {
                Button {
                    if DataScannerViewController.isAvailable {
                        isFocused = false
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
                            isFocused = false
                        } label: {
                            // The list is this network's already; the address
                            // tells two contacts of one name apart.
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
                    text = try readScan?(payload) ?? readScannedAddress(chain: chain, payload: payload)
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
