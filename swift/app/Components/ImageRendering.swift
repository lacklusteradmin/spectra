import Foundation
import SwiftUI

import UIKit

// MARK: ─ Icon helpers

/// Chain/token badge. Renders the bundled chain artwork when one exists;
/// otherwise falls back to a solid circle with the first letter of
/// `fallbackText`. No custom photos, no multi-letter disambiguation.
struct CoinBadge: View {
    /// The bundled artwork to draw, empty when the coin ships none. Not
    /// private: `CoinBadgeArtworkTests` asserts every coin the app can hold
    /// resolves to one that `UIImage(named:)` can load, and the resolution is
    /// the whole of what broke.
    let artworkName: String
    private let fallbackText: String
    private let color: Color
    private let size: CGFloat

    /// From artwork core already named — the wiki's rows carry one per coin.
    init(artworkName: String?, fallbackText: String, color: Color, size: CGFloat = 40) {
        self.artworkName = artworkName ?? ""
        self.fallbackText = fallbackText
        self.color = color
        self.size = size
    }

    var body: some View {
        let displayImage: UIImage? = artworkName.isEmpty ? nil : UIImage(named: artworkName)
        return Group {
            if let displayImage {
                Image(uiImage: displayImage).resizable().interpolation(.high).scaledToFit().frame(width: size, height: size)
            } else {
                letterFallback
            }
        }.shadow(color: color.opacity(0.18), radius: 6, y: 3)
    }
    private var letterFallback: some View {
        let letter = fallbackText.first.map { String($0).uppercased() } ?? "?"
        return Circle().fill(
            LinearGradient(colors: [color, color.opacity(0.75)], startPoint: .topLeading, endPoint: .bottomTrailing)
        ).frame(width: size, height: size).overlay {
            Text(letter).font(.system(size: size * 0.5, weight: .semibold, design: .rounded)).foregroundStyle(.white)
        }
    }
}
struct SpectraLogo: View {
    var size: CGFloat = 78
    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.28, style: .continuous).fill(Color.white.opacity(0.08)).frame(width: size, height: size)
                .background(
                    ZStack {
                        Circle().fill(Color.red.opacity(0.75)).frame(width: size * 0.7, height: size * 0.7).blur(radius: size * 0.14)
                            .offset(x: -size * 0.2, y: -size * 0.18)
                        Circle().fill(Color.yellow.opacity(0.72)).frame(width: size * 0.6, height: size * 0.6).blur(radius: size * 0.14)
                            .offset(x: size * 0.18, y: -size * 0.16)
                        Circle().fill(Color.green.opacity(0.62)).frame(width: size * 0.58, height: size * 0.58).blur(radius: size * 0.14)
                            .offset(x: -size * 0.16, y: size * 0.16)
                        Circle().fill(Color.blue.opacity(0.68)).frame(width: size * 0.62, height: size * 0.62).blur(radius: size * 0.15)
                            .offset(x: size * 0.2, y: size * 0.18)
                        Circle().fill(Color.purple.opacity(0.55)).frame(width: size * 0.52, height: size * 0.52).blur(radius: size * 0.16)
                    }
                ).overlay(
                    RoundedRectangle(cornerRadius: size * 0.28, style: .continuous).strokeBorder(Color.white.opacity(0.28), lineWidth: 1)
                ).glassEffect(.regular.tint(SpectraLayout.GlassTint.elevated), in: .rect(cornerRadius: size * 0.28)) // design-tokens: artwork
            Text("S").font(.system(size: size * 0.62, weight: .black, design: .rounded)).foregroundStyle(Color.primary).shadow(
                color: .black.opacity(0.18), radius: 8, y: 2
            ).rotationEffect(.degrees(-8))
        }.shadow(color: .black.opacity(0.18), radius: 16, y: 8)
    }
}
