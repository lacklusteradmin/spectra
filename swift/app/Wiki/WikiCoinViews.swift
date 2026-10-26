import SwiftUI

/// Artwork and identity supplied by core, shared by coin and chain wiki pages.
struct WikiCoinFace: Equatable {
    let name: String
    let symbol: String
    let artworkName: String
    let color: Color
}

struct WikiCoinBadge: View {
    let face: WikiCoinFace
    let size: CGFloat
    var body: some View {
        CoinBadge(
            artworkName: face.artworkName, fallbackText: face.symbol,
            color: face.color, size: size
        )
    }
}
