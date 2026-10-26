import SwiftUI

@main
struct SpectraApp: App {
    @State private var store = AppState()

    var body: some Scene {
        WindowGroup {
            ContentView(store: store).toggleStyle(AccentSwitchToggleStyle())
        }
    }
}

/// The asset catalog's AccentColor (system orange) is the app's one theme
/// colour, and most controls read it on their own. A switch's on state does
/// not — it stays system green. A root `.tint` would fix that but also colours
/// the toolbar glyphs, which stay monochrome; a toggle style reaches only
/// toggles. `UISwitch.appearance()` did too, but SwiftUI re-applies its own
/// tint when a toggle re-renders, so a switch turned green after an update.
private struct AccentSwitchToggleStyle: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        Toggle(configuration).toggleStyle(.switch).tint(.accentColor)
    }
}
