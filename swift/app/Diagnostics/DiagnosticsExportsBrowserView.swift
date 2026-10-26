import Foundation
import SwiftUI

/// The diagnostics bundles this device exported, newest first.
struct DiagnosticsExportsBrowserView: View {
    let store: AppState
    @Environment(\.dismiss) private var dismiss
    @State private var exportURLs: [URL] = []
    var body: some View {
        NavigationStack {
            List {
                if exportURLs.isEmpty {
                    Text(AppLocalization.string("No diagnostics exports yet.")).foregroundStyle(.secondary)
                } else {
                    ForEach(exportURLs, id: \.self) { url in
                        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                            Text(url.lastPathComponent).font(.subheadline.weight(.semibold))
                            Text(exportTimestamp(for: url)).font(.caption).foregroundStyle(.secondary)
                            ShareLink(item: url) {
                                Label(AppLocalization.string("Share"), systemImage: "square.and.arrow.up")
                            }.font(.caption)
                        }.padding(.vertical, SpectraLayout.Space.xs)
                    }.onDelete(perform: deleteExports)
                }
            }.navigationTitle(AppLocalization.string("Past Exports")).navigationBarTitleDisplayMode(.inline).toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(AppLocalization.string("Done")) {
                        dismiss()
                    }
                }
            }.onAppear(perform: reloadExports)
        }
    }
    private func reloadExports() { exportURLs = store.diagnosticsBundleExportURLs() }
    private func deleteExports(at offsets: IndexSet) {
        for index in offsets {
            try? store.deleteDiagnosticsBundleExport(at: exportURLs[index])
        }
        reloadExports()
    }
    private func exportTimestamp(for url: URL) -> String {
        let date = (try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? Date.distantPast
        return date == .distantPast ? AppLocalization.string("Unknown date") : date.appFormatted(time: .shortened)
    }
}
