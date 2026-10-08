import Foundation
import Testing
@testable import Spectra

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct MoneroSyncViewModelTests {
    private func status(height: UInt64) -> MoneroSyncStatus {
        MoneroSyncStatus(walletId: "wallet", scannedHeight: height, targetHeight: 100,
                         unlockedPiconeros: 0, complete: false, spendsKnown: true, usedSubaddresses: [])
    }

    @Test func cancelledBatchCannotOverwriteOrFinishRestartedScan() async throws {
        let vm = MoneroSyncViewModel()
        vm.status = status(height: 0)
        vm.password = "old password"
        vm.begin()
        let oldRequest = try #require(vm.requestId)
        let oldGate = SuspensionGate<Void>()
        let currentGate = SuspensionGate<Void>()
        let old = Task {
            await vm.sync(request: oldRequest) { password, progress in
                #expect(password == "old password")
                await oldGate.wait()
                progress(self.status(height: 10))
                return "old failure"
            }
        }
        await oldGate.reached()
        vm.cancel()
        old.cancel()
        vm.password = "new password"
        vm.begin()
        let currentRequest = try #require(vm.requestId)
        let current = Task {
            await vm.sync(request: currentRequest) { password, progress in
                #expect(password == "new password")
                progress(self.status(height: 20))
                await currentGate.wait()
                progress(self.status(height: 30))
                return nil
            }
        }
        await currentGate.reached()
        oldGate.resume()
        await old.value
        #expect(vm.requestId == currentRequest)
        #expect(vm.isRunning)
        #expect(vm.password == "new password")
        #expect(vm.status?.scannedHeight == 20)
        #expect(vm.error == nil)
        currentGate.resume()
        await current.value
        #expect(!vm.isRunning)
        #expect(vm.password.isEmpty)
        #expect(vm.status?.scannedHeight == 30)
        #expect(vm.error == nil)
    }

    @Test func cancelledRunCannotSubmitAfterItsTaskWasQueued() async throws {
        let vm = MoneroSyncViewModel()
        vm.begin()
        let request = try #require(vm.requestId)
        vm.cancel()
        var invoked = false
        await vm.sync(request: request) { _, _ in
            invoked = true
            return nil
        }
        #expect(!invoked)
        #expect(!vm.isRunning)
    }
}
