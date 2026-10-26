import Foundation
import SwiftUI

/// Immutable catalog presentation, indexed once by deployment id. A holding's
/// id is core's; this only looks up what the catalog draws for it.
enum AssetPresentationCatalog {
    private static let deployments = listAllBuiltinTokenDeployments()
    private static let artworkByDeployment = Dictionary(
        uniqueKeysWithValues: deployments.map { ($0.deploymentId, $0.artworkName) })
    /// A deployment's colour: the catalog token's own, or its chain's for a
    /// chain's native asset. Keyed by identity, never by ticker — a custom
    /// token calling itself `ETH` is not Ether.
    private static let colorByDeployment: [String: Color] = {
        var colors: [String: Color] = [:]
        for entry in Chain.all.compactMap(\.entry) {
            colors[entry.nativeDeploymentId] = entry.color.color
        }
        for token in deployments {
            if let color = token.color { colors[token.deploymentId] = color.color }
        }
        return colors
    }()

    static func artwork(deploymentId: String?) -> String {
        deploymentId.flatMap { artworkByDeployment[$0] } ?? ""
    }
    static func color(deploymentId: String) -> Color {
        colorByDeployment[deploymentId] ?? .gray
    }
}
