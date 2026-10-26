import Foundation
enum AppEndpointDirectory {
    /// The endpoint catalog as settings groups it, read once. Core networking
    /// uses its own catalog.
    private static let byChainId: [Chain: ChainEndpointSettings] = Dictionary(
        uniqueKeysWithValues: endpointSettings().map { ($0.chainId, $0) })

    static func groupedSettingsEntries(for chain: Chain) -> [EndpointSettingsGroup] {
        byChainId[chain]?.groups ?? []
    }
}
