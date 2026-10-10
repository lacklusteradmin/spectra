import SwiftUI

/// The row carrying a flow's primary actions, placed with
/// `.safeAreaBar(edge: .bottom)`.
///
/// The bar draws nothing behind its buttons. The buttons are glass, and
/// `safeAreaBar` gives the content scrolling under them the system's scroll
/// edge effect, so the bar reads as part of the screen rather than a slab
/// laid over it. A background of its own — a material, a glass rectangle, a
/// divider — is what made it one.
struct SpectraBottomActionBar<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        HStack(spacing: SpectraLayout.Space.m) { content }
            .padding(.horizontal, SpectraLayout.Space.l)
            .padding(.vertical, SpectraLayout.Space.s)
    }
}
