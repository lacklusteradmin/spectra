import Foundation

/// Transient native workflow identity. Core owns every transaction and signature,
/// and says what may be done with a staged one next: every artifact this session
/// adopts comes with core's `SendProgress` for it, read after the step that
/// produced it, so the two never disagree.
@MainActor
@Observable
final class SendSession {
    enum Operation: Equatable { case build, resume, sign, broadcast }
    private(set) var id = UUID()
    private(set) var operation: Operation?
    /// The staged transaction, its history record and what core allows next.
    var progress: SendProgress?
    var endpoints: [String] = []
    var selectedEndpoints: Set<String> = []
    var error: String?
    var isBusy: Bool { operation != nil }
    var artifact: SendArtifact? { progress?.artifact }
    /// Core's answer for a staged artifact, read by its id.
    @ObservationIgnored private let readProgress: @MainActor (SendArtifact) async throws -> SendProgress

    init(progress: @escaping @MainActor (SendArtifact) async throws -> SendProgress) {
        readProgress = progress
    }

    convenience init(bridge: WalletServiceBridge) {
        self.init(progress: { try await bridge.ready().sendProgress(id: $0.id) })
    }

    func reset() {
        id = UUID()
        operation = nil
        progress = nil
        endpoints = []
        selectedEndpoints = []
        error = nil
    }

    func isCurrent(_ request: UUID) -> Bool { id == request && !Task.isCancelled }

    private func finish(_ request: UUID) {
        if id == request { operation = nil }
    }

    /// Adopt a complete artifact, progress and endpoint set together, never
    /// part of one.
    @discardableResult
    func load(operation: Operation, prepare: () async throws -> SendArtifact,
              endpoints loadEndpoints: (Chain) async throws -> [String]) async -> Bool {
        guard self.operation == nil else { return false }
        let request = id
        self.operation = operation
        defer { finish(request) }
        do {
            let prepared = try await prepare()
            guard isCurrent(request) else { return false }
            let progress = try await readProgress(prepared)
            guard isCurrent(request) else { return false }
            let choices = try await loadEndpoints(prepared.chainId)
            guard isCurrent(request) else { return false }
            self.progress = progress
            endpoints = choices
            selectedEndpoints = Set(progress.artifact.selectedEndpoints).intersection(choices)
            error = nil
            return true
        } catch {
            if isCurrent(request) { self.error = userErrorMessage(error) }
            return false
        }
    }

    func sign(password: String?, authenticate: () async -> String?,
              sign: (String, String, String?) async throws -> SendArtifact) async {
        guard operation == nil, let progress, progress.next == .sign else { return }
        let artifact = progress.artifact
        let request = id
        operation = .sign
        defer { finish(request) }
        let failure = await authenticate()
        guard isCurrent(request), self.artifact?.id == artifact.id else { return }
        if let failure {
            error = failure
            return
        }
        do {
            let signed = try await sign(artifact.id, artifact.reviewDigest, password)
            guard isCurrent(request), self.artifact?.id == artifact.id else { return }
            let next = try await readProgress(signed)
            guard isCurrent(request), self.artifact?.id == artifact.id else { return }
            self.progress = next
            error = nil
        } catch {
            if isCurrent(request) { self.error = userErrorMessage(error) }
        }
    }

    /// Submit the signed transaction to the selected nodes, the first time
    /// or again. Returns the submitted artifact, which belongs to the
    /// application even after this session's form closes; `nil` if nothing
    /// was submitted.
    func broadcast(submit: (String, [String]) async throws -> SendArtifact) async -> SendArtifact? {
        guard operation == nil, let progress,
              progress.next == .broadcast || progress.canRebroadcast else { return nil }
        let artifact = progress.artifact
        let request = id
        operation = .broadcast
        defer { finish(request) }
        let result: SendArtifact
        do {
            result = try await submit(artifact.id, endpoints.filter(selectedEndpoints.contains))
        } catch {
            if isCurrent(request) { self.error = userErrorMessage(error) }
            return nil
        }
        // A committed broadcast belongs to the application even after its
        // composer closes. Only the form's adoption is session-scoped.
        do {
            let next = try await readProgress(result)
            if isCurrent(request), self.artifact?.id == artifact.id {
                self.progress = next
                error = nil
            }
        } catch {
            if isCurrent(request) { self.error = userErrorMessage(error) }
        }
        return result
    }

    /// Read core's answer for the staged transaction again: its history
    /// record moves without the artifact changing.
    func refresh() async {
        guard operation == nil, let artifact else { return }
        let request = id
        do {
            let next = try await readProgress(artifact)
            guard isCurrent(request), operation == nil, self.artifact?.id == artifact.id else { return }
            progress = next
        } catch {
            if isCurrent(request), self.artifact?.id == artifact.id { self.error = userErrorMessage(error) }
        }
    }
}
