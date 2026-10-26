import UIKit
import UniformTypeIdentifiers

/// How long a copied secret stays on the pasteboard if nothing pastes it.
let secretPasteboardLifetime: Duration = .seconds(60)

/// Put a secret — a seed phrase — on the pasteboard for this device only and
/// for a minute. `.localOnly` keeps it off Universal Clipboard, so it never
/// reaches the user's other devices, and the expiration takes it off the
/// pasteboard rather than leaving it for whatever reads the pasteboard next.
@MainActor
func copySecretToPasteboard(_ secret: String, pasteboard: UIPasteboard = .general) {
    pasteboard.setItems(
        [[UTType.utf8PlainText.identifier: secret]],
        options: [
            .localOnly: true,
            .expirationDate: Date.now.addingTimeInterval(TimeInterval(secretPasteboardLifetime.components.seconds)),
        ])
}
