import SwiftUI

/// A short-lived confirmation message, such as "Copied 12 log entries".
///
/// Carries an identity separate from its text so that two notices are distinct
/// values even when they read the same. Copying the same thing twice has to
/// restart the dismiss timer; if the notice were a bare `String?` the second
/// copy would leave the value unchanged, and the timer already running would
/// keep its original deadline and clear the second notice early.
struct SpectraTransientNotice: Equatable {
    let id = UUID()
    let text: String
    init(_ text: String) { self.text = text }
}

extension View {
    /// Clears `notice` after `seconds`.
    ///
    /// `.task(id:)` rather than a detached `Task { sleep }`: it cancels the
    /// pending timer when the notice changes and when the view goes away, so
    /// two notices in quick succession cannot race and a dismissed screen
    /// leaves nothing running. The cancellation check matters because a
    /// cancelled `Task.sleep` throws, and clearing the notice at that point
    /// would wipe the notice that just replaced it.
    func spectraTransientNotice(_ notice: Binding<SpectraTransientNotice?>, seconds: Double = 2) -> some View {
        task(id: notice.wrappedValue) {
            guard notice.wrappedValue != nil else { return }
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled else { return }
            notice.wrappedValue = nil
        }
    }
}
