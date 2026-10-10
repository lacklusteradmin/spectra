import SwiftUI
import UIKit

/// The card a page brings into view when the keyboard comes up: the one
/// holding the field being typed in, scrolled to the top of what the keyboard
/// leaves. A flow's action bar rides on the keyboard, and without this it
/// covered whatever sat under the field — the amount's balance, the address's
/// warning, the seed grid's last row.
///
/// One per page: the marked card is the page's input. The top, not the
/// bottom, goes to the edge, so at large text sizes the field stays in view
/// and the rest of the card scrolls.
extension View {
    func keyboardAnchor() -> some View {
        id(KeyboardAnchor.id)
    }

    /// For a page's scroll content — the stack inside the `ScrollView`, so
    /// its height is the content's — given its `ScrollViewReader`'s proxy.
    func scrollsToKeyboardAnchor(_ proxy: ScrollViewProxy) -> some View {
        modifier(KeyboardAnchorScroller(proxy: proxy))
    }
}

private enum KeyboardAnchor {
    static let id = "keyboard.anchor"
}

/// Scrolls when the keyboard comes up, and again while it is up whenever the
/// content changes height: a check row or a warning appearing under the
/// field would otherwise grow the card down behind the bar.
private struct KeyboardAnchorScroller: ViewModifier {
    let proxy: ScrollViewProxy
    @State private var isKeyboardUp = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        content
            .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardDidShowNotification)) { _ in
                isKeyboardUp = true
                scrollToAnchor()
            }
            .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in
                isKeyboardUp = false
            }
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { _ in
                if isKeyboardUp { scrollToAnchor() }
            }
    }

    private func scrollToAnchor() {
        withAnimation(reduceMotion ? nil : .snappy(duration: 0.25)) {
            proxy.scrollTo(KeyboardAnchor.id, anchor: .top)
        }
    }
}
