import Foundation
import SwiftUI
struct AboutView: View {
    @State private var isAnimatingHero = false
    private let copy = SettingsContentCopy.current
    var body: some View {
        ZStack {
            SpectraBackdrop().ignoresSafeArea()
            ScrollView(showsIndicators: false) {
                LazyVStack(spacing: SpectraLayout.Space.l) {
                    aboutHero
                    aboutCard(title: copy.aboutEthosTitle, lines: copy.aboutEthosLines)
                    aboutNarrativeCard
                }.spectraScreenPadding()
            }
        }.navigationTitle(AppLocalization.string("About Spectra")).navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.hidden, for: .navigationBar).onAppear {
            isAnimatingHero = true
        }.onDisappear {
            // Stop the infinite rotation so the GPU isn't animating an
            // off-screen layer when the user navigates away from About.
            isAnimatingHero = false
        }
    }
    private var aboutHero: some View {
        VStack(spacing: SpectraLayout.Space.m) {
            ZStack {
                Circle().fill(
                    AngularGradient(
                        colors: [
                            .red.opacity(0.85), .orange.opacity(0.92), .yellow.opacity(0.9), .green.opacity(0.82), .blue.opacity(0.82),  // design-tokens: artwork
                            .indigo.opacity(0.82), .pink.opacity(0.88), .red.opacity(0.85),
                        ], center: .center
                    )
                ).frame(width: 220, height: 220).blur(radius: 26).rotationEffect(.degrees(isAnimatingHero ? 360 : 0)).animation(
                    .linear(duration: 18).repeatForever(autoreverses: false), value: isAnimatingHero)
                Circle().fill(Color.white.opacity(0.08)).frame(width: 178, height: 178).glassEffect(.regular.tint(SpectraLayout.GlassTint.elevated), in: .circle)  // design-tokens: artwork
                SpectraLogo(size: 96)
            }
            VStack(spacing: SpectraLayout.Space.s) {
                Text(copy.aboutTitle).font(.largeTitle.weight(.bold)).foregroundStyle(Color.primary)
                Text(copy.aboutSubtitle).font(.subheadline).multilineTextAlignment(.center).foregroundStyle(.secondary)
            }.frame(maxWidth: .infinity)
        }.padding(SpectraLayout.Space.l).spectraBubbleFill().spectraCardFill()
    }
    private var aboutNarrativeCard: some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(copy.aboutNarrativeTitle).font(.headline).foregroundStyle(Color.primary)
            ForEach(copy.aboutNarrativeParagraphs, id: \.self) { paragraph in
                Text(paragraph).font(.subheadline).foregroundStyle(.secondary)
            }
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading).spectraBubbleFill().spectraCardFill()
    }
    private func aboutCard(title: String, lines: [String]) -> some View {
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            Text(title).font(.headline).foregroundStyle(Color.primary)
            ForEach(lines, id: \.self) { line in
                HStack(alignment: .top, spacing: SpectraLayout.Space.s) {
                    Circle().fill(Color.primary.opacity(0.5)).frame(width: 6, height: 6).padding(.top, SpectraLayout.Space.s)
                    Text(line).font(.subheadline).foregroundStyle(.secondary)
                }
            }
        }.padding(SpectraLayout.Space.l).frame(maxWidth: .infinity, alignment: .leading).spectraBubbleFill().spectraCardFill()
    }
}
