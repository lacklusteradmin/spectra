import SwiftUI
import UIKit
private struct SpectraInputFieldChrome: ViewModifier {
    let cornerRadius: CGFloat
    let borderColor: Color?
    func body(content: Content) -> some View {
        content.spectraInsetFill(cornerRadius: cornerRadius).overlay {
            if let borderColor {
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous).stroke(borderColor, lineWidth: 1)
            }
        }
    }
}
extension View {
    func spectraBubbleFill() -> some View { frame(maxWidth: .infinity, alignment: .leading) }
    func spectraInputFieldStyle(cornerRadius: CGFloat = SpectraLayout.Radius.inner, borderColor: Color? = nil) -> some View {
        modifier(SpectraInputFieldChrome(cornerRadius: cornerRadius, borderColor: borderColor))
    }
}
extension Binding {
    static func isPresent<Wrapped: Sendable>(_ source: Binding<Wrapped?>) -> Binding<Bool> where Value == Bool {
        Binding<Bool>(
            get: { source.wrappedValue != nil },
            set: { if !$0 { source.wrappedValue = nil } }
        )
    }
}
@MainActor @ViewBuilder
func spectraDetailCard(title: String? = nil, @ViewBuilder content: () -> some View) -> some View {
    VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
        if let title { Text(AppLocalization.string(title)).font(.headline) }
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) { content() }
    }.padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
}
// MARK: — Typography helpers
extension View {
    func spectraHintText() -> some View { font(.caption).foregroundStyle(.secondary) }
}

// MARK: — Semantic status colors
extension Color {
    static func spectraTransactionStatusColor(_ status: TransactionStatus) -> Color {
        switch status {
        case .pending: return .spectraWarning
        case .confirmed: return .green
        case .failed: return .red
        }
    }
    static func spectraTransactionAmountColor(isReceive: Bool) -> Color { isReceive ? .green : .red }
    static func spectraPriceAlertStatusColor(isEnabled: Bool, hasTriggered: Bool) -> Color {
        if !isEnabled { return .gray }
        return hasTriggered ? .green : .spectraWarning
    }
}

// MARK: — Haptic helpers
//
// Generators are kept and re-prepared rather than built per tap. A generator
// constructed at the moment of the tap has to warm the Taptic Engine before it
// can fire, and that warm-up is the gap between the touch and the feedback;
// `prepare()` after each use means the next tap fires immediately.
//
// Nothing here checks whether haptics are wanted: `UIFeedbackGenerator` already
// honours the system haptics setting.
//
// These stay imperative on purpose. Almost every call sits inside a `Button`
// action, and `.sensoryFeedback` would need a `@State` trigger per site that
// exists only to be mutated — more state for the same tap. Feedback driven by
// a value changing rather than by a tap does use `.sensoryFeedback`; see the
// tag picker in `CryptoWikiViews`.
@MainActor
private enum SpectraHaptics {
    private static var impactByStyle: [UIImpactFeedbackGenerator.FeedbackStyle: UIImpactFeedbackGenerator] = [:]
    private static let notification = UINotificationFeedbackGenerator()

    static func impact(_ style: UIImpactFeedbackGenerator.FeedbackStyle) {
        let generator: UIImpactFeedbackGenerator
        if let existing = impactByStyle[style] {
            generator = existing
        } else {
            generator = UIImpactFeedbackGenerator(style: style)
            impactByStyle[style] = generator
        }
        generator.impactOccurred()
        generator.prepare()
    }

    static func notify(_ type: UINotificationFeedbackGenerator.FeedbackType) {
        notification.notificationOccurred(type)
        notification.prepare()
    }
}

@MainActor func spectraHaptic(_ style: UIImpactFeedbackGenerator.FeedbackStyle = .medium) {
    SpectraHaptics.impact(style)
}
@MainActor func spectraNotificationHaptic(_ type: UINotificationFeedbackGenerator.FeedbackType = .success) {
    SpectraHaptics.notify(type)
}

// MARK: — Shimmer loading placeholder
struct SpectraShimmer: View {
    /// A placeholder bar's own rounding, not a step on
    /// `SpectraLayout.Radius`: the scale describes surfaces in the
    /// hierarchy, and this is a 12-14pt bar standing in for a line of text.
    private static let cornerRadius: CGFloat = 6
    var height: CGFloat = 16
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var phase: CGFloat = -1
    var body: some View {
        GeometryReader { geo in
            ZStack {
                RoundedRectangle(cornerRadius: Self.cornerRadius, style: .continuous).fill(Color.primary.opacity(0.08))
                if !reduceMotion {
                    RoundedRectangle(cornerRadius: Self.cornerRadius, style: .continuous).fill(
                        LinearGradient(colors: [.clear, Color.white.opacity(0.18), .clear], startPoint: .leading, endPoint: .trailing)
                    ).offset(x: geo.size.width * (phase + 1))
                }
            }
        }
        .frame(height: height)
        .clipShape(RoundedRectangle(cornerRadius: Self.cornerRadius, style: .continuous))
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.linear(duration: 1.4).repeatForever(autoreverses: false)) { phase = 1 }
        }
    }
}

struct SpectraLoadingGlyph: View {
    var size: CGFloat = 28
    var tint: Color = .accentColor
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isPulsing = false
    @State private var isSpinning = false

    var body: some View {
        ZStack {
            Circle()
                .fill(tint.opacity(0.14))
            Circle()
                .trim(from: 0.18, to: 0.84)
                .stroke(tint.opacity(0.82), style: StrokeStyle(lineWidth: max(2, size * 0.09), lineCap: .round))
                .rotationEffect(.degrees(isSpinning ? 360 : 0))
            Text("S")
                .font(.system(size: size * 0.44, weight: .black, design: .rounded))
                .foregroundStyle(tint)
        }
        .frame(width: size, height: size)
        .scaleEffect(reduceMotion ? 1 : (isPulsing ? 1.06 : 0.94))
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.easeInOut(duration: 1.05).repeatForever(autoreverses: true)) {
                isPulsing = true
            }
            withAnimation(.linear(duration: 1.35).repeatForever(autoreverses: false)) {
                isSpinning = true
            }
        }
    }
}

struct SpectraLoadingRow: View {
    let title: String
    var subtitle: String? = nil

    var body: some View {
        HStack(spacing: SpectraLayout.Space.m) {
            SpectraLoadingGlyph(size: 30)
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xxs) {
                Text(AppLocalization.string(title))
                    .font(.subheadline.weight(.semibold))
                if let subtitle {
                    Text(AppLocalization.string(subtitle))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 0)
        }
    }
}

/// An empty state on its own, between the cards of a glass page. Inside a
/// card, or in a `Form` row, the same words go in `SpectraEmptyStateContent`
/// — a card in a card stacks glass on glass — and an empty search or a whole
/// empty list screen is `ContentUnavailableView`.
struct SpectraEmptyStateCard: View {
    let title: String
    let message: String
    var systemImage: String = "tray"
    var actionTitle: String? = nil
    var actionSystemImage: String = "arrow.right"
    var action: (() -> Void)? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            SpectraEmptyStateContent(title: title, message: message, systemImage: systemImage)
            if let actionTitle, let action {
                Button(action: action) {
                    Label(AppLocalization.string(actionTitle), systemImage: actionSystemImage)
                        .font(.subheadline.weight(.semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, SpectraLayout.Space.s)
                }
                .buttonStyle(.glassProminent)
            }
        }
        .padding(SpectraLayout.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .spectraCardFill()
    }
}

/// An empty state's words and symbol, without a surface: for inside a card
/// or a `Form` row. See `SpectraEmptyStateCard`.
struct SpectraEmptyStateContent: View {
    let title: String
    let message: String
    var systemImage: String = "tray"

    var body: some View {
        HStack(alignment: .top, spacing: SpectraLayout.Space.m) {
            Image(systemName: systemImage)
                .font(.title3.weight(.semibold))
                .foregroundStyle(.tint)
                .frame(width: 40, height: 40)
                .background(Color.accentColor.opacity(0.14), in: Circle())
            VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
                Text(AppLocalization.string(title))
                    .font(.headline)
                Text(AppLocalization.string(message))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The icon-plus-title-plus-subtitle header the send, receive and primary send
/// pages all open with.
@ViewBuilder
func spectraPageHeader(title: String, subtitle: String, systemImage: String) -> some View {
    HStack(alignment: .top, spacing: SpectraLayout.Space.m) {
        Image(systemName: systemImage)
            .font(.title2.weight(.semibold))
            .foregroundStyle(.tint)
            .frame(width: 42, height: 42)
            .background(Color.accentColor.opacity(0.14), in: Circle())

        VStack(alignment: .leading, spacing: SpectraLayout.Space.xs) {
            Text(AppLocalization.string(title)).font(.title2.weight(.bold))
            Text(AppLocalization.string(subtitle)).font(.subheadline).foregroundStyle(.secondary)
        }
    }
}
