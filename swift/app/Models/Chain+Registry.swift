import Foundation

/// `Chain` is generated from `registry::Chain` in `core/data/chains.toml` order.
/// Its identity, display name, symbol, and capabilities come from that catalog,
/// which crosses the FFI once, as `listAllChains()`.
extension Chain: Identifiable {
    /// Every chain, in catalog order.
    static let all: [Chain] = entries.map(\.chain)

    /// Only the chains that are not testnets. Ordered as the catalog is.
    static let mainnets: [Chain] = entries.filter { !$0.isTestnet }.map(\.chain)

    /// Chains that can hold tracked tokens, in catalog order.
    static let tokenHostingChains: [Chain] = all.filter(\.hostsTokens)

    private static let entries: [ChainEntry] = listAllChains()
    private static let entryByChain: [Chain: ChainEntry] = Dictionary(
        uniqueKeysWithValues: entries.map { ($0.chain, $0) })
    private static let chainById: [String: Chain] = Dictionary(
        uniqueKeysWithValues: entries.map { ($0.id, $0.chain) })

    /// This chain's catalog row. `nil` only if the enum and the catalog have
    /// drifted, which core's `chain_order_matches_the_catalog` fails on.
    var entry: ChainEntry? { Self.entryByChain[self] }

    /// The catalog's stable `id` — `"bitcoin"`, `"bitcoin-cash"`, `"bnb"`.
    /// This is what crosses the FFI boundary and what endpoint tables key on.
    public var id: String { entry?.id ?? "" }

    /// The catalog's `name` — `"Bitcoin Cash"`, `"XRP Ledger"`, `"BNB Smart Chain"`.
    /// One spelling per chain: the registry has a test that says so.
    var displayName: String { entry?.name ?? "" }

    /// Which chain's slot this chain's address is stored under. The EVM family
    /// shares Ethereum's.
    var addressSlot: String { entry?.addressSlot ?? "" }
    /// A wallet on this chain is an account of many addresses, which a
    /// rescan can walk.
    var usesAccountUTXO: Bool { entry?.usesAccountUtxo ?? false }
    /// The prefixes a watched account public key starts with on this network.
    var accountKeyPrefixes: [String] { entry?.accountKeyPrefixes ?? [] }
    /// The send screen has a network card to show for this chain.
    var hasSendPreview: Bool { entry?.hasSendPreview ?? false }
    /// The chain can hold tracked tokens.
    var hostsTokens: Bool { entry?.hostsTokens ?? false }
    /// The ledger creates an account only once it holds the network's reserve.
    var requiresAccountReserve: Bool { entry?.requiresAccountReserve ?? false }
    /// The mainnet this chain belongs to, or itself.
    var mainnetCounterpart: Chain { entry?.mainnet ?? self }
    /// A test network's faucet page, where its coins are free.
    var faucetURL: URL? { entry?.faucetUrl.flatMap(URL.init(string:)) }

    /// The network’s native token symbol, derived by core.
    var gasTokenSymbol: String { entry?.gasTokenSymbol ?? "" }

    /// The chain's native asset decimals, from the catalog. `nil` only if the
    /// enum and the catalog have drifted; a guessed precision would misstate
    /// every fee on the chain.
    var nativeDecimals: UInt32? { entry?.nativeDecimals }

    /// A terse example of what an address on this chain looks like, or "" for
    /// a chain the catalog has no example for.
    var addressPrefixHint: String { entry?.addressPrefixHint ?? "" }
    var isEVM: Bool { entry?.isEvm ?? false }
    var searchKeywords: [String] { entry?.searchKeywords ?? [] }

    init?(id: String) {
        guard let chain = Self.chainById[id] else { return nil }
        self = chain
    }
}
