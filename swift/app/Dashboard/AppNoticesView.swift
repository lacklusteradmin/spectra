import SwiftUI

struct AppNoticesView: View {
    let store: AppState
    var body: some View {
        let notices = store.appNoticeItems
        return List {
            if notices.isEmpty {
                Section {
                    VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                        Text(AppLocalization.string("No active notices")).font(.headline)
                        Text(AppLocalization.string("Current wallet, pricing, and chain-state warnings will appear here.")).font(
                            .subheadline
                        ).foregroundStyle(.secondary)
                    }.padding(.vertical, SpectraLayout.Space.xs)
                }
            } else {
                Section(AppLocalization.string("Active Notices")) {
                    ForEach(notices) { notice in noticeRow(notice) }
                }
            }
        }.navigationTitle(AppLocalization.string("Notices"))
    }

    /// A notice with its way forward: a row that opens where it is fixed, or
    /// a button under it. Borderless, so the row does not fire it.
    @ViewBuilder
    private func noticeRow(_ notice: AppNoticeItem) -> some View {
        switch notice.action {
        case .openEndpoints:
            NavigationLink { EndpointCatalogSettingsView(store: store) } label: {
                DashboardNoticeCardView(notice: notice)
            }
        case .retry(let retry):
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                DashboardNoticeCardView(notice: notice)
                Button(AppLocalization.string("Retry"), systemImage: "arrow.clockwise") { Task { await retry() } }
                    .buttonStyle(.borderless).font(.subheadline.weight(.semibold))
            }
        case .dismiss(let dismiss):
            VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
                DashboardNoticeCardView(notice: notice)
                Button(AppLocalization.string("Dismiss"), systemImage: "xmark") { dismiss() }
                    .buttonStyle(.borderless).font(.subheadline.weight(.semibold))
            }
        case nil:
            DashboardNoticeCardView(notice: notice)
        }
    }
}

struct DashboardNoticeCardView: View {
    let notice: AppNoticeItem
    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            HStack(spacing: SpectraLayout.Space.s) {
                Image(systemName: notice.systemImage).foregroundStyle(notice.severity.tint)
                Text(notice.title).font(.headline)
                Spacer()
                Text(notice.severity.label).font(.caption.weight(.semibold)).foregroundStyle(notice.severity.tint)
            }
            Text(notice.message).font(.subheadline).foregroundStyle(.primary)
            if let timestamp = notice.timestamp {
                Text(
                    AppLocalization.format(
                        "Last known healthy sync: %@", timestamp.appFormatted(time: .shortened))
                ).font(.caption).foregroundStyle(.secondary)
            }
        }.padding(.vertical, SpectraLayout.Space.xs)
    }
}

enum AppNoticeSeverity {
    case warning
    case error
    var tint: Color {
        switch self {
        case .warning: return .spectraWarning
        case .error: return .red
        }
    }
    var label: String {
        switch self {
        case .warning: return AppLocalization.string("Warning")
        case .error: return AppLocalization.string("Error")
        }
    }
}
struct AppNoticeItem: Identifiable {
    /// The notice's own words: `appNoticeItems` is recomputed on every read,
    /// so a generated id would make every notice a new row each time.
    var id: String { "\(title)\u{1F}\(message)" }
    let title: String
    let message: String
    let severity: AppNoticeSeverity
    let systemImage: String
    var timestamp: Date? = nil
    /// What can be done about it from the notices page, if anything.
    var action: AppNoticeAction? = nil
}
/// A notice's way forward: ask again, go where it is fixed, or put it away.
enum AppNoticeAction {
    /// Run the read that failed again.
    case retry(@MainActor () async -> Void)
    /// Where a network's endpoints are chosen.
    case openEndpoints
    /// Seen, with nothing else to do.
    case dismiss(@MainActor () -> Void)
}
