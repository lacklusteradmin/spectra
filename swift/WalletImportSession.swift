import Foundation

/// Owns only the native form and its lifetime. Closing does not undo a core commit.
@MainActor
@Observable
final class WalletImportSession {
    let draft = WalletImportDraft()
    private(set) var id = UUID()
    private(set) var isBusy = false
    private(set) var editingWalletId: String?
    var error: String?
    var isPresented = false {
        didSet { if oldValue && !isPresented { invalidate() } }
    }

    private func invalidate() {
        id = UUID()
        isBusy = false
        error = nil
        editingWalletId = nil
        draft.clear()
    }

    func begin(editing wallet: WalletView? = nil, configure: (WalletImportDraft) -> Void) {
        invalidate()
        editingWalletId = wallet?.id
        configure(draft)
        isPresented = true
    }

    func close() {
        if isPresented { isPresented = false }
        else { invalidate() }
    }

    /// Success, failure and cleanup may touch only the form that submitted them.
    /// The operation still refreshes committed domain projections after dismissal.
    @discardableResult
    func submit(operation: () async throws -> String?) async -> Bool {
        guard !isBusy else { return false }
        let request = id
        isBusy = true
        error = nil
        defer { if id == request { isBusy = false } }
        do {
            let notice = try await operation()
            guard id == request, !Task.isCancelled else { return false }
            close()
            error = notice
            return true
        } catch {
            if id == request, !Task.isCancelled { self.error = userErrorMessage(error) }
            return false
        }
    }
}
