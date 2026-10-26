import Foundation

// Core assembles the bundle from what it recorded; Swift supplies what only
// the platform knows about itself and owns the files.

extension AppState {
    private var diagnosticsPlatformInfo: DiagnosticsPlatformInfo {
        let info = Bundle.main.infoDictionary ?? [:]
        return DiagnosticsPlatformInfo(
            appVersion: (info["CFBundleShortVersionString"] as? String) ?? "unknown",
            buildNumber: (info["CFBundleVersion"] as? String) ?? "unknown",
            osVersion: ProcessInfo.processInfo.operatingSystemVersionString,
            localeIdentifier: Locale.current.identifier,
            timeZoneIdentifier: TimeZone.current.identifier)
    }

    // MARK: File I/O

    func exportDiagnosticsBundle() async throws -> URL {
        let json = try await bridge.ready().diagnosticsBundle(platform: diagnosticsPlatformInfo)
        let stamp = Self.exportFilenameTimestampFormatter.string(from: Date()).replacingOccurrences(of: ":", with: "-")
        let fileURL = try diagnosticsBundleExportsDirectoryURL()
            .appendingPathComponent("spectra-diagnostics-\(stamp)")
            .appendingPathExtension("json")
        try Data(json.utf8).write(to: fileURL, options: .atomic)
        return fileURL
    }
    func diagnosticsBundleExportsDirectoryURL() throws -> URL {
        let base = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first ?? FileManager.default.temporaryDirectory
        let directory = base.appendingPathComponent("Diagnostics Bundles", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }
    func diagnosticsBundleExportURLs() -> [URL] {
        guard let directory = try? diagnosticsBundleExportsDirectoryURL(),
            let urls = try? FileManager.default.contentsOfDirectory(
                at: directory, includingPropertiesForKeys: [.contentModificationDateKey], options: [.skipsHiddenFiles])
        else { return [] }
        return urls.filter { $0.pathExtension.lowercased() == "json" }.sorted { lhs, rhs in
            let l = (try? lhs.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
            let r = (try? rhs.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
            return l > r
        }
    }
    func deleteDiagnosticsBundleExport(at url: URL) throws { try FileManager.default.removeItem(at: url) }
    @discardableResult
    func importDiagnosticsBundle(from url: URL) throws -> DiagnosticsBundlePayload {
        let data = try Data(contentsOf: url)
        guard let json = String(data: data, encoding: .utf8),
            let payload = diagnosticsBundleFromJson(json: json)
        else { throw DiagnosticsBundleError.invalidBundle }
        return payload
    }
}

enum DiagnosticsBundleError: LocalizedError {
    case invalidBundle
    var errorDescription: String? {
        switch self {
        case .invalidBundle: return AppLocalization.string("That file is not a Spectra diagnostics bundle.")
        }
    }
}

extension DiagnosticsBundlePayload {
    var generatedAtDate: Date { Date(timeIntervalSince1970: generatedAt) }

}
