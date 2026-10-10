import Foundation

/// Transient secret presentation. A dismissed screen, or one the app left for
/// the background, cannot adopt a late reveal.
///
/// An inactive scene is not one the app left: the Face ID or passcode sheet a
/// reveal asks for makes the scene inactive, and its answer can arrive before
/// the scene is active again. That answer waits and is shown on return. Until
/// then the app's snapshot cover is over the screen, so nothing is visible.
@MainActor
@Observable
final class SeedPhraseRevealState {
    var isShowingPasswordPrompt = false
    var isShowingPhraseSheet = false
    var passwordInput = ""
    private(set) var phrase = ""
    var errorMessage: String?
    private(set) var isRevealing = false
    @ObservationIgnored private var requestId = UUID() // Binds results and cleanup to one reveal.
    @ObservationIgnored private var isVisible = false // Rejects tasks queued before the screen disappeared.
    @ObservationIgnored private var isSceneActive = true // Completion reads live lifecycle state.
    /// An answer that arrived while the scene was inactive, shown when it is active.
    @ObservationIgnored private var pending: PendingOutcome?
    private enum PendingOutcome { case phrase, error(String) }

    func activate(sceneIsActive: Bool = true) {
        isVisible = true
        isSceneActive = sceneIsActive
    }

    func deactivate() {
        isVisible = false
        invalidate()
    }

    /// Leaving the scene hides what is shown; returning shows an answer that
    /// arrived meanwhile. Going to the background invalidates the request
    /// besides (`invalidate`), so nothing from it is waiting on return.
    func setSceneIsActive(_ active: Bool) {
        isSceneActive = active
        guard active else {
            // An answer already waiting outlives this; one on screen is hidden.
            if pending == nil { clearPresentation() }
            return
        }
        guard isVisible, let outcome = pending else { return }
        pending = nil
        switch outcome {
        case .phrase: isShowingPhraseSheet = true
        case .error(let message): errorMessage = message
        }
    }

    func clearPresentation() {
        pending = nil
        isShowingPasswordPrompt = false
        isShowingPhraseSheet = false
        passwordInput = ""
        phrase = ""
        errorMessage = nil
    }

    func invalidate() {
        requestId = UUID()
        isRevealing = false
        clearPresentation()
    }

    func clearPhrase() { phrase = "" }

    /// `nil` means the request no longer belongs to a visible screen.
    func reveal(canPresent: @MainActor () -> Bool,
                operation: @MainActor () async throws -> String) async -> Bool? {
        guard isVisible, isSceneActive, !isRevealing, canPresent(), !Task.isCancelled else { return nil }
        let request = requestId
        isRevealing = true
        defer { if requestId == request { isRevealing = false } }
        do {
            let revealed = try await operation()
            guard requestId == request, isVisible, !Task.isCancelled, canPresent() else { return nil }
            phrase = revealed
            passwordInput = ""
            errorMessage = nil
            if isSceneActive { isShowingPhraseSheet = true } else { pending = .phrase }
            return true
        } catch {
            guard requestId == request, isVisible, !Task.isCancelled, canPresent() else { return nil }
            let message = userErrorMessage(error)
            if isSceneActive { errorMessage = message } else { pending = .error(message) }
            return false
        }
    }
}
