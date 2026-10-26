import SwiftUI
import UIKit

extension View {
    /// Show `cover` for `item` in a window of its own, above everything this
    /// scene presents.
    ///
    /// An overlay inside the view tree sits under every sheet, alert and
    /// confirmation dialog, because UIKit presents those above the root view.
    /// A cover that must hide the app — the lock screen, the app-switcher
    /// snapshot — needs a window above the app's instead. The app's own tree
    /// is left alone, so nothing in it is rebuilt when the cover comes and goes.
    func sceneCover<Item: Equatable, Cover: View>(
        item: Item?, @ViewBuilder cover: @escaping (Item) -> Cover
    ) -> some View {
        modifier(SceneCoverModifier(item: item, cover: cover))
    }
}

private struct SceneCoverModifier<Item: Equatable, Cover: View>: ViewModifier {
    let item: Item?
    let cover: (Item) -> Cover
    @State private var window = SceneCoverWindow()

    func body(content: Content) -> some View {
        content
            .background(WindowSceneReader { window.scene = $0 })
            .onChange(of: item, initial: true) { _, item in
                window.show(item.map { AnyView(cover($0)) })
            }
    }
}

/// The cover's window. Created when a cover is shown and dropped when it is
/// hidden, so an unlocked app carries no second window.
@MainActor
private final class SceneCoverWindow {
    weak var scene: UIWindowScene? {
        didSet { if scene !== oldValue { apply() } }
    }
    private var content: AnyView?
    private var window: UIWindow?

    func show(_ content: AnyView?) {
        self.content = content
        apply()
    }

    private func apply() {
        guard let content else { return hide() }
        if let host = window?.rootViewController as? UIHostingController<AnyView> {
            host.rootView = content
            return
        }
        guard let scene else { return }
        // A focused field keeps the keyboard up, and the keyboard's window
        // sits above this one.
        scene.windows.forEach { $0.endEditing(true) }
        let host = UIHostingController(rootView: content)
        host.view.backgroundColor = .clear
        host.view.accessibilityViewIsModal = true
        let window = UIWindow(windowScene: scene)
        window.windowLevel = .alert + 1
        window.rootViewController = host
        window.makeKeyAndVisible()
        self.window = window
    }

    private func hide() {
        guard let window else { return }
        window.isHidden = true
        self.window = nil
        scene?.windows.first { $0.windowLevel == .normal }?.makeKey()
    }
}

/// Reports the window scene the view is placed in.
private struct WindowSceneReader: UIViewRepresentable {
    let onScene: (UIWindowScene?) -> Void

    func makeUIView(context: Context) -> ReaderView {
        let view = ReaderView()
        view.onScene = onScene
        return view
    }
    func updateUIView(_ view: ReaderView, context: Context) { view.onScene = onScene }

    final class ReaderView: UIView {
        var onScene: ((UIWindowScene?) -> Void)?
        override func didMoveToWindow() {
            super.didMoveToWindow()
            onScene?(window?.windowScene)
        }
    }
}
