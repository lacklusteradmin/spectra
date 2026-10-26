import SwiftUI
import UIKit

/// What a recovery phrase or a private key sits behind on screen.
///
/// While the screen is recorded, mirrored or shared the secret is blurred and
/// its `.privacySensitive()` text redacted, so it reaches no other screen.
/// A screenshot cannot be stopped, but it lands in Photos, where backups and
/// other apps can read it, so taking one is answered with a warning.
private struct SecretShield: ViewModifier {
    @Environment(\.isSceneCaptured) private var isCaptured
    @State private var tookScreenshot = false

    func body(content: Content) -> some View {
        content
            .redacted(reason: isCaptured ? .privacy : [])
            .blur(radius: isCaptured ? 14 : 0)
            .overlay {
                if isCaptured {
                    Label(AppLocalization.string("secret.hiddenWhileCaptured"), systemImage: "eye.slash.fill")
                        .font(.subheadline.weight(.semibold))
                        .multilineTextAlignment(.center)
                        .padding(SpectraLayout.cardPadding)
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: UIApplication.userDidTakeScreenshotNotification)) { _ in
                tookScreenshot = true
            }
            .alert(AppLocalization.string("secret.screenshot.title"), isPresented: $tookScreenshot) {
                Button(AppLocalization.string("OK"), role: .cancel) {}
            } message: {
                Text(AppLocalization.string("secret.screenshot.message"))
            }
    }
}

extension View {
    /// Shield a secret on screen; see `SecretShield`. Mark the secret's own
    /// text `.privacySensitive()` so the redaction reaches it.
    func secretShield() -> some View { modifier(SecretShield()) }
}
