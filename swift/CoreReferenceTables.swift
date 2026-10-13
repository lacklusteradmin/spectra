/// Process-lifetime copies of core's compile-time tables, fetched once on
/// first use. Do not cache secret inputs or helpers that are already cheap
/// local lookups.
enum CoreReferenceTables {
    static let assetWiki: [AssetWikiEntry] = listAssetWiki()
    static let chainWiki: [ChainWikiEntry] = listChainWiki()
    /// The bounds core holds edits to, for the controls that set them.
    static let bounds: InputBounds = inputBounds()
    /// What staking means on each chain that stakes.
    private static let stakingByChain = Dictionary(uniqueKeysWithValues: listStakingChains().map { ($0.chain, $0) })

    static func stakingEntry(for chain: Chain) -> StakingChainEntry? { stakingByChain[chain] }
}
