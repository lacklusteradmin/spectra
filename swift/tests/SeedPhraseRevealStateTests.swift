import Foundation
import Testing
@testable import Spectra

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct SeedPhraseRevealStateTests {
    @Test func hiddenScreenRejectsLatePhraseAndOldCleanupCannotFinishNewReveal() async {
        let state = SeedPhraseRevealState()
        state.activate()
        let oldGate = SuspensionGate<String>()
        let currentGate = SuspensionGate<String>()
        let old = Task {
            await state.reveal(canPresent: { true }, operation: { await oldGate.wait() })
        }
        await oldGate.reached()
        state.deactivate()
        #expect(state.phrase.isEmpty)
        #expect(!state.isShowingPhraseSheet)
        state.activate()
        state.passwordInput = "new password"
        let current = Task {
            await state.reveal(canPresent: { true }, operation: { await currentGate.wait() })
        }
        await currentGate.reached()
        oldGate.resume("obsolete secret")
        #expect(await old.value == nil)
        #expect(state.phrase.isEmpty)
        #expect(!state.isShowingPhraseSheet)
        #expect(state.isRevealing, "Old cleanup must not finish the new request")
        #expect(state.passwordInput == "new password")
        currentGate.resume("current secret")
        #expect(await current.value == true)
        #expect(state.phrase == "current secret")
        #expect(state.isShowingPhraseSheet)
        #expect(!state.isRevealing)
        #expect(state.passwordInput.isEmpty)
        state.invalidate()
        #expect(state.phrase.isEmpty)
        #expect(!state.isShowingPhraseSheet)
    }

    @Test func removedWalletRejectsCompletionWithoutRepopulatingSecret() async {
        let state = SeedPhraseRevealState()
        state.activate()
        var canPresent = true
        let gate = SuspensionGate<String>()
        let work = Task {
            await state.reveal(canPresent: { canPresent }, operation: { await gate.wait() })
        }
        await gate.reached()
        canPresent = false
        gate.resume("secret of a removed wallet")
        #expect(await work.value == nil)
        #expect(state.phrase.isEmpty)
        #expect(!state.isShowingPhraseSheet)
        #expect(state.errorMessage == nil)
    }

    /// Face ID makes the scene inactive and can answer before it is active
    /// again: the phrase waits, hidden, and is shown on return.
    @Test func anAnswerDuringFaceIDIsShownOnReturn() async {
        let state = SeedPhraseRevealState()
        state.activate()
        let gate = SuspensionGate<String>()
        let work = Task {
            await state.reveal(canPresent: { true }, operation: { await gate.wait() })
        }
        await gate.reached()
        state.setSceneIsActive(false)
        gate.resume("secret")
        #expect(await work.value == true)
        #expect(!state.isShowingPhraseSheet, "nothing is presented while the scene is inactive")
        state.setSceneIsActive(false)
        #expect(state.phrase == "secret", "a second inactive pass keeps the waiting answer")
        state.setSceneIsActive(true)
        #expect(state.isShowingPhraseSheet)
        #expect(state.phrase == "secret")
    }

    /// A refusal that arrives during Face ID is said on return too.
    @Test func anErrorDuringFaceIDIsSaidOnReturn() async {
        let state = SeedPhraseRevealState()
        state.activate()
        let gate = SuspensionGate<Void>()
        let work = Task {
            await state.reveal(canPresent: { true }, operation: {
                await gate.wait()
                throw DisplayedError("refused")
            })
        }
        await gate.reached()
        state.setSceneIsActive(false)
        gate.resume()
        #expect(await work.value == false)
        #expect(state.errorMessage == nil)
        state.setSceneIsActive(true)
        #expect(state.errorMessage == "refused")
    }

    /// Going to the background drops a waiting answer: nothing is shown
    /// when the app comes back.
    @Test func backgroundDropsAWaitingAnswer() async {
        let state = SeedPhraseRevealState()
        state.activate()
        let gate = SuspensionGate<String>()
        let work = Task {
            await state.reveal(canPresent: { true }, operation: { await gate.wait() })
        }
        await gate.reached()
        state.setSceneIsActive(false)
        gate.resume("secret")
        #expect(await work.value == true)
        state.invalidate()
        state.setSceneIsActive(true)
        #expect(!state.isShowingPhraseSheet)
        #expect(state.phrase.isEmpty)
    }

    @Test func backgroundedRequestStaysInvalidAfterReturningToActiveScene() async {
        let state = SeedPhraseRevealState()
        state.activate()
        let gate = SuspensionGate<String>()
        let work = Task {
            await state.reveal(canPresent: { true }, operation: { await gate.wait() })
        }
        await gate.reached()
        state.setSceneIsActive(false)
        state.invalidate()
        state.setSceneIsActive(true)
        gate.resume("secret from previous foreground")
        #expect(await work.value == nil)
        #expect(state.phrase.isEmpty)
        #expect(!state.isShowingPhraseSheet)
        #expect(!state.isRevealing)
    }

    @Test func dismissedScreenRejectsQueuedWorkAndLateErrors() async {
        let state = SeedPhraseRevealState()
        var invoked = false
        #expect(await state.reveal(canPresent: { true }, operation: {
            invoked = true
            return "secret"
        }) == nil)
        #expect(!invoked)
        state.activate()
        let gate = SuspensionGate<Void>()
        let work = Task {
            await state.reveal(canPresent: { true }, operation: {
                await gate.wait()
                throw NSError(domain: "obsolete", code: 1)
            })
        }
        await gate.reached()
        state.invalidate()
        state.errorMessage = "current message"
        gate.resume()
        #expect(await work.value == nil)
        #expect(state.errorMessage == "current message")
        #expect(state.phrase.isEmpty)
    }
}
