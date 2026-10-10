import ActivityKit
import SwiftUI
import WidgetKit
@main
struct SpectraLiveActivityExtensionBundle: WidgetBundle {
    var body: some Widget { SendTransactionLiveActivityWidget() }
}
struct SendTransactionLiveActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: SendTransactionLiveActivityAttributes.self) { context in SendTransactionLockScreenView(state: context.state).activityBackgroundTint(SendTransactionActivityPalette.background(for: context.state.phase)).activitySystemActionForegroundColor(.white) } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    VStack(alignment: .leading, spacing: 8) {
                        SendTransactionStatusGlyph(phase: context.state.phase, size: 34)
                        if context.state.phase == .sending { Text(context.state.startedAt, style: .timer).font(.caption2.monospacedDigit()).foregroundStyle(.white.opacity(0.7)) }}}
                DynamicIslandExpandedRegion(.trailing) {
                    VStack(alignment: .trailing, spacing: 4) {
                        Text(context.state.symbol).font(.caption.weight(.bold))
                        Text(context.state.amountText).font(.headline.monospacedDigit().weight(.bold)).lineLimit(1)
                    }.foregroundStyle(.white)
                }
                DynamicIslandExpandedRegion(.center) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(context.state.walletName).font(.caption.weight(.semibold)).foregroundStyle(.white.opacity(0.72)).lineLimit(1)
                        Text(context.state.statusText).font(.headline.weight(.semibold)).foregroundStyle(.white).lineLimit(1)
                        Text(context.state.chainName).font(.caption).foregroundStyle(.white.opacity(0.75)).lineLimit(1)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 8) {
                            Image(systemName: "arrow.up.right").foregroundStyle(.white.opacity(0.7))
                            Text(context.state.destinationPreview).font(.caption.monospaced()).foregroundStyle(.white.opacity(0.86)).lineLimit(1)
                        }
                        Text(context.state.detailText).font(.caption).foregroundStyle(.white.opacity(0.72)).lineLimit(2)
                        if let transactionHashPreview = context.state.transactionHashPreview {
                            HStack(spacing: 8) {
                                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                                Text(transactionHashPreview).font(.caption.monospaced()).foregroundStyle(.white.opacity(0.7)).lineLimit(1)
                            }}}}} compactLeading: {
                SendTransactionStatusGlyph(phase: context.state.phase, size: 22)
            } compactTrailing: {
                Text(compactTrailingText(for: context.state)).font(.caption2.monospacedDigit().weight(.bold)).foregroundStyle(.white).lineLimit(1)
            } minimal: {
                SendTransactionStatusGlyph(phase: context.state.phase, size: 22)
            }.keylineTint(SendTransactionActivityPalette.tint(for: context.state.phase))
        }}
    private func compactTrailingText(for state: SendTransactionLiveActivityAttributes.ContentState) -> String {
        let compactAmount = state.amountText.replacingOccurrences(of: " ", with: "")
        if compactAmount.count <= 7 { return compactAmount }
        return state.symbol
    }
}
private struct SendTransactionLockScreenView: View {
    let state: SendTransactionLiveActivityAttributes.ContentState
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 12) {
                SendTransactionStatusGlyph(phase: state.phase, size: 30)
                VStack(alignment: .leading, spacing: 3) {
                    Text(state.walletName).font(.caption.weight(.semibold)).foregroundStyle(.white.opacity(0.68))
                    Text(state.statusText).font(.headline.weight(.semibold)).foregroundStyle(.white)
                    Text(state.chainName).font(.caption).foregroundStyle(.white.opacity(0.72))
                }
                Spacer()
                VStack(alignment: .trailing, spacing: 3) {
                    Text(state.symbol).font(.caption.weight(.bold)).foregroundStyle(.white.opacity(0.74))
                    Text(state.amountText).font(.title3.monospacedDigit().weight(.bold)).foregroundStyle(.white)
                }}
            if state.phase == .sending {
                HStack(spacing: 8) {
                    Image(systemName: "timer").foregroundStyle(.white.opacity(0.65))
                    Text(state.startedAt, style: .timer).font(.footnote.monospacedDigit()).foregroundStyle(.white.opacity(0.8))
                }}
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    Image(systemName: "wallet.bifold").foregroundStyle(.white.opacity(0.65))
                    Text(state.destinationPreview).font(.footnote.monospaced()).foregroundStyle(.white.opacity(0.82)).lineLimit(1)
                }
                Text(state.detailText).font(.footnote).foregroundStyle(.white.opacity(0.76)).lineLimit(2)
                if let transactionHashPreview = state.transactionHashPreview {
                    HStack(spacing: 8) {
                        Image(systemName: "number").foregroundStyle(.white.opacity(0.65))
                        Text(transactionHashPreview).font(.footnote.monospaced()).foregroundStyle(.white.opacity(0.72)).lineLimit(1)
                    }}}}.padding(.vertical, 4)
    }
}
private struct SendTransactionStatusGlyph: View {
    let phase: SendTransactionLiveActivityAttributes.ContentState.Phase
    let size: CGFloat
    var body: some View {
        ZStack {
            Circle().fill(backgroundColor.opacity(0.22))
            if phase == .sending { ProgressView().progressViewStyle(.circular).tint(foregroundColor).scaleEffect(0.85) } else { Image(systemName: systemImage).font(.caption.weight(.bold)).foregroundStyle(foregroundColor) }}.frame(width: size, height: size)
    }
    private var systemImage: String {
        switch phase {
        case .sending: return "arrow.up.circle.fill"
        case .complete: return "checkmark.circle.fill"
        case .failed: return "xmark.circle.fill"
        }}
    private var foregroundColor: Color {
        switch phase {
        case .sending: return .orange
        case .complete: return .green
        case .failed: return .red
        }}
    private var backgroundColor: Color {
        switch phase {
        case .sending: return .orange
        case .complete: return .green
        case .failed: return .red
        }}
}
private enum SendTransactionActivityPalette {
    static func background(for phase: SendTransactionLiveActivityAttributes.ContentState.Phase) -> Color {
        switch phase {
        case .sending: return Color.black.opacity(0.94)
        case .complete: return Color.green.opacity(0.22)
        case .failed: return Color.red.opacity(0.22)
        }}
    static func tint(for phase: SendTransactionLiveActivityAttributes.ContentState.Phase) -> Color {
        switch phase {
        case .sending: return .orange
        case .complete: return .green
        case .failed: return .red
        }}
}
