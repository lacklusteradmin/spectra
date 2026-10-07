import Testing
import UIKit

@testable import Spectra

@MainActor
struct SecretPasteboardTests {
    /// A copied phrase is on the pasteboard as plain text, so the setup
    /// page's paste button reads it back whole.
    @Test func aCopiedSecretReadsBackAsText() throws {
        let pasteboard = try #require(UIPasteboard(name: UIPasteboard.Name("spectra.tests.\(UUID())"), create: true))
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        copySecretToPasteboard("legal winner thank year", pasteboard: pasteboard)
        #expect(pasteboard.string == "legal winner thank year")
    }
}
