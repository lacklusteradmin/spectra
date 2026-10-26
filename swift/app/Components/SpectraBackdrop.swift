import SwiftUI

/// The app's wallpaper: a neutral gradient with three faint chroma clouds for
/// `.glassEffect` to refract, in the manner of Apple's own Liquid Glass hero
/// surfaces (Weather, Wallet, Maps) rather than painterly rainbow splatters.
///
/// The clouds drift. Each follows its own slow closed path, and their periods
/// share no common beat, so the motion never visibly repeats and is noticed
/// only when watched. Position is a pure function of wall-clock time, so every
/// instance — one per tab and per pushed screen — agrees, and a navigation
/// transition shows no jump between the backdrop leaving and the one arriving.
///
/// It holds still under Reduce Motion (at its rest layout), in Low Power Mode
/// and in the background.
struct SpectraBackdrop: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase
    @State private var isLowPowerMode = ProcessInfo.processInfo.isLowPowerModeEnabled

    /// Drift this slow reads as smooth at half the display's rate, and every
    /// frame also re-samples the glass above it.
    private static let frameInterval: TimeInterval = 1.0 / 30

    var body: some View {
        let isStill = isLowPowerMode || scenePhase == .background
        TimelineView(.animation(minimumInterval: Self.frameInterval, paused: reduceMotion || isStill)) { timeline in
            let time = timeline.date.timeIntervalSinceReferenceDate
            // The clouds overflow the screen, so they sit in an overlay: the
            // backdrop takes the size it is offered, whatever they span.
            LinearGradient(colors: backdropGradientColors, startPoint: .top, endPoint: .bottom).overlay {
                ZStack {
                    ForEach(Array(clouds.enumerated()), id: \.offset) { _, cloud in
                        cloud.view(at: time, drifting: !reduceMotion)
                    }
                }
            }.clipped()
        }
        .ignoresSafeArea()
        .task {
            for await _ in NotificationCenter.default.notifications(named: .NSProcessInfoPowerStateDidChange) {
                isLowPowerMode = ProcessInfo.processInfo.isLowPowerModeEnabled
            }
        }
    }

    /// Low-chroma on purpose: the backdrop is there for the glass to
    /// refract, not to be looked at. A saturated wash tints every card above
    /// it and pulls secondary text toward its own hue, and a second hue
    /// competes with the accent. So the clouds are the accent and one cool
    /// counterweight, both faint, over a neutral ground.
    private var clouds: [ChromaCloud] {
        let light = colorScheme == .light
        return [
            ChromaCloud(
                color: Color.accentColor.opacity(light ? 0.10 : 0.16), diameter: 340, softness: 110,
                rest: CGSize(width: 150, height: -240), drift: CGSize(width: 50, height: 50),
                periods: (x: 53, y: 41, breath: 31), phase: 1.7),
            ChromaCloud(
                color: (light ? Color.blue : Color.indigo).opacity(light ? 0.07 : 0.14), diameter: 320, softness: 110,
                rest: CGSize(width: -150, height: -60), drift: CGSize(width: 60, height: 60),
                periods: (x: 47, y: 37, breath: 29), phase: 0.0),
            ChromaCloud(
                color: Color.accentColor.opacity(light ? 0.05 : 0.08), diameter: 360, softness: 120,
                rest: CGSize(width: 120, height: 320), drift: CGSize(width: 60, height: 50),
                periods: (x: 61, y: 47, breath: 23), phase: 4.6),
        ]
    }

    private var backdropGradientColors: [Color] {
        if colorScheme == .light {
            return [
                Color(red: 0.97, green: 0.97, blue: 0.98), Color(red: 0.94, green: 0.94, blue: 0.95),
            ]
        }
        return [
            Color(red: 0.07, green: 0.07, blue: 0.08), Color(red: 0.04, green: 0.04, blue: 0.05),
        ]
    }
}

/// One soft cloud. Drawn as a radial gradient shaped like a blurred disc of
/// `diameter` and blur radius `softness`: moving a gradient only moves a layer,
/// where a live blur of this size would be re-rasterised every frame.
private struct ChromaCloud {
    let color: Color
    let diameter: CGFloat
    let softness: CGFloat
    /// The offset from the screen's centre where the cloud rests, and where it
    /// stays when motion is reduced.
    let rest: CGSize
    /// The furthest the cloud wanders from `rest` on each axis.
    let drift: CGSize
    /// Seconds per cycle of horizontal and vertical travel and of the slight
    /// swell in size.
    let periods: (x: Double, y: Double, breath: Double)
    /// Radians, so clouds that share a period still start apart.
    let phase: Double

    func view(at time: TimeInterval, drifting: Bool) -> some View {
        let reach = diameter / 2 + softness * 1.2
        return RadialGradient(
            stops: [
                .init(color: color.opacity(0.85), location: 0),
                .init(color: color.opacity(0.68), location: 0.3),
                .init(color: color.opacity(0.38), location: 0.55),
                .init(color: color.opacity(0.12), location: 0.78),
                .init(color: color.opacity(0), location: 1),
            ],
            center: .center, startRadius: 0, endRadius: reach
        )
        .frame(width: reach * 2, height: reach * 2)
        .scaleEffect(drifting ? 1 + 0.08 * wave(time, period: periods.breath) : 1)
        .offset(
            x: rest.width + (drifting ? drift.width * wave(time, period: periods.x) : 0),
            y: rest.height + (drifting ? drift.height * wave(time, period: periods.y) : 0)
        )
    }

    private func wave(_ time: TimeInterval, period: Double) -> CGFloat {
        CGFloat(sin(2 * .pi * time / period + phase))
    }
}
