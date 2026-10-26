import SwiftUI

/// A disclosure as the app draws one: its label and a chevron that turns, the
/// whole row the tap target, and what it reveals at the full width of what
/// holds it. The system style inset the content, and inside a scroll view
/// laid a square plate behind the card it revealed.
struct SpectraDisclosureStyle: DisclosureGroupStyle {
    func makeBody(configuration: Configuration) -> some View {
        SpectraDisclosure(configuration: configuration)
    }
}

extension DisclosureGroupStyle where Self == SpectraDisclosureStyle {
    static var spectra: SpectraDisclosureStyle { SpectraDisclosureStyle() }
}

private struct SpectraDisclosure: View {
    let configuration: DisclosureGroupStyleConfiguration
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.s) {
            Button {
                withAnimation(reduceMotion ? nil : .snappy(duration: 0.25)) {
                    configuration.isExpanded.toggle()
                }
            } label: {
                HStack(spacing: SpectraLayout.Space.s) {
                    configuration.label
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Image(systemName: "chevron.right")
                        .font(.footnote.weight(.semibold))
                        .rotationEffect(.degrees(configuration.isExpanded ? 90 : 0))
                        .accessibilityHidden(true)
                }
                .foregroundStyle(.tint)
                .padding(.horizontal, SpectraLayout.Space.xs)
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityValue(AppLocalization.string(configuration.isExpanded ? "Expanded" : "Collapsed"))

            if configuration.isExpanded {
                configuration.content
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}
