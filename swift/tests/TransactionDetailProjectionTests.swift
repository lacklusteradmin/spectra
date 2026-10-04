import Foundation
import Testing
@testable import Spectra

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct TransactionDetailProjectionTests {
    private func record(status: TransactionStatus) -> TransactionRecord {
        TransactionRecord(id: "send", walletId: "wallet", kind: .send, status: status,
            walletName: "Wallet", assetDisplayName: "Ether", symbol: "ETH", chainId: .ethereum,
            amount: "1", address: "recipient")
    }

    private func endpoints(holder: String) -> TransactionEndpoints {
        TransactionEndpoints(from: nil,
            to: TransactionEndpoint(address: "recipient", isMine: false, holder: .contact(name: holder)))
    }

    @Test(arguments: [false, true])
    func cancelledReadCannotReplaceNewerStatusOrAddressHolder(waitingForEndpoints: Bool) async throws {
        let gate = SuspensionGate<Void>()
        var displayed = (transaction: record(status: .pending), endpoints: Optional(endpoints(holder: "original")))
        var oldEndpointsRead = false
        let old = Task {
            do {
                displayed = try await loadTransactionDetailProjection(
                    fallback: self.record(status: .pending),
                    transaction: {
                        if !waitingForEndpoints { await gate.wait() }
                        return self.record(status: .pending)
                    }, endpoints: {
                        oldEndpointsRead = true
                        if waitingForEndpoints { await gate.wait() }
                        return self.endpoints(holder: "obsolete")
                    })
                Issue.record("A cancelled detail read must not return a projection")
            } catch is CancellationError {
                // SwiftUI cancels the previous .task when the revision changes.
            } catch { Issue.record(error) }
        }
        await gate.reached()
        old.cancel()
        displayed = try await loadTransactionDetailProjection(
            fallback: record(status: .pending), transaction: { self.record(status: .confirmed) },
            endpoints: { self.endpoints(holder: "current") })
        gate.resume()
        await old.value
        #expect(displayed.transaction.status == .confirmed)
        #expect(displayed.endpoints?.to?.holder == .contact(name: "current"))
        #expect(oldEndpointsRead == waitingForEndpoints, "A cancelled transaction read must not start an endpoint read")
    }
}
