//! Central chain + token registry.
//!
//! The canonical `Chain` enum identifies chains by stable string ids
//! (e.g. `"bitcoin"`, `"ethereum"`). `Chain::str_id()` returns the id;
//! `Chain::from_str_id()` parses one back. The numeric discriminants were
//! removed in favour of string-keyed lookups throughout the codebase.

/// Every chain Spectra knows about.
///
/// This crosses the FFI boundary as the one chain type every front end uses.
/// Before it did, each front end kept its own copy of this list — iOS had four
/// (`SpectraChainID`, `SeedDerivationChain`, `AppChainID`,
/// `StandardDiagnosticsChain`), each a different subset, each drifting.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum Chain {
    Bitcoin,
    Ethereum,
    Solana,
    Dogecoin,
    Xrp,
    Litecoin,
    BitcoinCash,
    Tron,
    Stellar,
    Cardano,
    Polkadot,
    Arbitrum,
    Optimism,
    Avalanche,
    Sui,
    Aptos,
    Ton,
    Near,
    Icp,
    Monero,
    Base,
    EthereumClassic,
    BitcoinSV,
    BnbChain,
    Hyperliquid,
    Polygon,
    Linea,
    Scroll,
    Blast,
    Mantle,
    Zcash,
    BitcoinGold,
    Decred,
    Kaspa,
    Sei,
    Celo,
    Cronos,
    OpBnb,
    ZkSyncEra,
    Sonic,
    Berachain,
    Unichain,
    Ink,
    Dash,
    XLayer,
    Bittensor,

    // ── Testnets ─────────────────────────────────────────────────────────────
    BitcoinTestnet,
    BitcoinTestnet4,
    BitcoinSignet,
    LitecoinTestnet,
    BitcoinCashTestnet,
    BitcoinSVTestnet,
    DogecoinTestnet,
    ZcashTestnet,
    DecredTestnet,
    KaspaTestnet,
    DashTestnet,
    EthereumSepolia,
    EthereumHoodi,
    ArbitrumSepolia,
    OptimismSepolia,
    BaseSepolia,
    BnbChainTestnet,
    AvalancheFuji,
    PolygonAmoy,
    HyperliquidTestnet,
    EthereumClassicMordor,
    TronNile,
    SolanaDevnet,
    XrpTestnet,
    StellarTestnet,
    CardanoPreprod,
    SuiTestnet,
    AptosTestnet,
    TonTestnet,
    NearTestnet,
    PolkadotWestend,
    MoneroStagenet,
}

/// Which endpoint-list slot to fetch for a given chain.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EndpointSlot {
    Primary,
    Secondary,
    Explorer,
}

// All variants in stable order. Used by Chain::all().
/// What a chain requires before a holding may be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendRule {
    /// Only the chain's native asset.
    NativeOnly,
    /// The native asset, or a token the app supports on this chain.
    NativeOrSupportedToken,
    /// Solana's own list of sendable coins.
    SupportedSolanaCoin,
    /// No extra restriction beyond the chain supporting sends at all.
    Any,
}

const ALL_CHAINS: &[Chain] = &[
    Chain::Bitcoin,
    Chain::Ethereum,
    Chain::Solana,
    Chain::Dogecoin,
    Chain::Xrp,
    Chain::Litecoin,
    Chain::BitcoinCash,
    Chain::Tron,
    Chain::Stellar,
    Chain::Cardano,
    Chain::Polkadot,
    Chain::Arbitrum,
    Chain::Optimism,
    Chain::Avalanche,
    Chain::Sui,
    Chain::Aptos,
    Chain::Ton,
    Chain::Near,
    Chain::Icp,
    Chain::Monero,
    Chain::Base,
    Chain::EthereumClassic,
    Chain::BitcoinSV,
    Chain::BnbChain,
    Chain::Hyperliquid,
    Chain::Polygon,
    Chain::Linea,
    Chain::Scroll,
    Chain::Blast,
    Chain::Mantle,
    Chain::Zcash,
    Chain::BitcoinGold,
    Chain::Decred,
    Chain::Kaspa,
    Chain::Sei,
    Chain::Celo,
    Chain::Cronos,
    Chain::OpBnb,
    Chain::ZkSyncEra,
    Chain::Sonic,
    Chain::Berachain,
    Chain::Unichain,
    Chain::Ink,
    Chain::Dash,
    Chain::XLayer,
    Chain::Bittensor,
    // Testnets
    Chain::BitcoinTestnet,
    Chain::BitcoinTestnet4,
    Chain::BitcoinSignet,
    Chain::LitecoinTestnet,
    Chain::BitcoinCashTestnet,
    Chain::BitcoinSVTestnet,
    Chain::DogecoinTestnet,
    Chain::ZcashTestnet,
    Chain::DecredTestnet,
    Chain::KaspaTestnet,
    Chain::DashTestnet,
    Chain::EthereumSepolia,
    Chain::EthereumHoodi,
    Chain::ArbitrumSepolia,
    Chain::OptimismSepolia,
    Chain::BaseSepolia,
    Chain::BnbChainTestnet,
    Chain::AvalancheFuji,
    Chain::PolygonAmoy,
    Chain::HyperliquidTestnet,
    Chain::EthereumClassicMordor,
    Chain::TronNile,
    Chain::SolanaDevnet,
    Chain::XrpTestnet,
    Chain::StellarTestnet,
    Chain::CardanoPreprod,
    Chain::SuiTestnet,
    Chain::AptosTestnet,
    Chain::TonTestnet,
    Chain::NearTestnet,
    Chain::PolkadotWestend,
    Chain::MoneroStagenet,
];

/// Where an EVM chain's transaction history can be read from.
///
/// Only keyless explorer sources are configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmHistorySource<'a> {
    /// A keyless endpoint. The base already identifies the chain, so no
    /// `chainid` is sent.
    Open(&'a str),
    /// No keyless indexer is configured. Asking is an error, not an empty list.
    Unavailable,
}

impl Chain {
    pub fn supports_derivation_passphrase(self) -> bool {
        self.mainnet_counterpart() != Self::Monero
    }
    pub fn supports_derivation_hmac_override(self) -> bool {
        matches!(
            self.mainnet_counterpart(),
            Self::Solana | Self::Stellar | Self::Polkadot
        )
    }

    /// Named accounts authorize keys on chain instead of encoding the key in their address.
    pub fn supports_named_sender_accounts(self) -> bool {
        matches!(self, Self::Near | Self::NearTestnet)
    }

    /// Stable string id matching `chains.toml` `id` field.
    /// This chain's row in the catalog.
    ///
    /// The enum is declared in `chains.toml` order, so a variant *is* an index
    /// into the catalog and every column it holds can be read without a second
    /// table. `chain_order_matches_the_catalog` fails the build if they drift.
    pub fn entry(self) -> &'static crate::chains::ChainEntry {
        crate::chains::catalog()
            .get(self as usize)
            .expect("enum declaration order is the catalog's order")
    }

    /// Stable string id — the catalog's `id`.
    pub fn str_id(self) -> &'static str {
        self.entry().id.as_str()
    }

    /// Parse a string id (from `chains.toml` or the FFI boundary) into a `Chain`.
    /// Parse a string id (from `chains.toml` or the FFI boundary) into a `Chain`.
    pub fn from_str_id(id: &str) -> Option<Self> {
        static BY_ID: std::sync::LazyLock<std::collections::HashMap<&'static str, Chain>> =
            std::sync::LazyLock::new(|| Chain::all().map(|c| (c.str_id(), c)).collect());
        BY_ID.get(id).copied()
    }

    /// Key under which this chain's address is stored during wallet import.
    ///
    /// Most chains own their address, so the slot is just [`Chain::str_id`].
    /// The exceptions are the EVM family: one derived secp256k1 address serves
    /// Ethereum, Arbitrum, Base and the rest, so they all share the
    /// `"ethereum"` slot rather than each carrying a copy.
    ///
    /// Ethereum Classic is EVM but keeps a slot of its own, because the import
    /// flow lets the user supply a distinct ETC address. A wallet on that chain
    /// is written into *both* slots — see `derivation::import::addresses_for_chain`.
    ///
    /// Testnets fall through to their own `str_id`. Import only ever populates
    /// mainnet slots, so a testnet lookup misses and yields no address — which
    /// is correct: a Bitcoin testnet address is not a Bitcoin address.
    pub fn address_slot(self) -> &'static str {
        match self {
            Chain::EthereumClassic => Chain::EthereumClassic.str_id(),
            _ if self.is_evm() && !self.is_testnet() => Chain::Ethereum.str_id(),
            _ => self.str_id(),
        }
    }

    /// Whether a raw private key yields an address on this chain.
    /// Shared by derivation and import eligibility. Testnets follow their
    /// mainnet; derivation handles network-specific address encoding.
    pub fn derives_from_private_key(self) -> bool {
        let chain = self.mainnet_counterpart();
        chain.is_evm()
            || matches!(
                chain,
                Chain::Bitcoin
                    | Chain::BitcoinCash
                    | Chain::Litecoin
                    | Chain::Dogecoin
                    | Chain::Decred
            )
    }

    /// The decode shape this chain's send preview comes back in, for the
    /// chains core estimates through one entry point.
    ///
    /// `None` means the chain has a preview path of its own — the UTXO family,
    /// Dogecoin, Tron and the EVM family each take different inputs and return
    /// a different record, which is why they are not one call.
    ///
    /// Swift held this as an eleven-entry `[String: SimpleChain]` table keyed
    /// by display name, and passed the result back to core beside the chain id
    /// core could have derived it from.
    ///
    /// Through `mainnet_counterpart` because the shape is what decoding needs
    /// and a testnet decodes like its mainnet; which network is reached is the
    /// chain id's business.
    pub fn simple_preview_chain(self) -> Option<crate::send::preview_decode::SimpleChain> {
        use crate::send::preview_decode::SimpleChain;
        Some(match self.mainnet_counterpart() {
            Chain::Solana => SimpleChain::Solana,
            Chain::Xrp => SimpleChain::Xrp,
            Chain::Stellar => SimpleChain::Stellar,
            Chain::Monero => SimpleChain::Monero,
            Chain::Cardano => SimpleChain::Cardano,
            Chain::Sui => SimpleChain::Sui,
            Chain::Aptos => SimpleChain::Aptos,
            Chain::Ton => SimpleChain::Ton,
            Chain::Icp => SimpleChain::Icp,
            Chain::Near => SimpleChain::Near,
            Chain::Polkadot => SimpleChain::Polkadot,
            Chain::Bittensor => SimpleChain::Bittensor,
            _ => return None,
        })
    }

    /// Extra transaction bytes beyond a plain output.
    /// Litecoin MWEB extension-block outputs add about a kilobyte, which must
    /// be included in fee and maximum-send calculations.
    pub fn extra_output_overhead_bytes(self, destination: &str) -> u64 {
        if self.is_extension_block_destination(destination) {
            1017
        } else {
            0
        }
    }

    /// Whether a destination on this chain is an extension-block output:
    /// Litecoin's MWEB, the only chain with one. The fee rule above and the
    /// composer's privacy badge both ask this, so it is asked one way.
    pub fn is_extension_block_destination(self, destination: &str) -> bool {
        let lowered = destination.trim().to_lowercase();
        self.mainnet_counterpart() == Chain::Litecoin
            && (lowered.starts_with("ltcmweb1") || lowered.starts_with("tmweb1"))
    }

    /// Wire contract implemented by the client consuming this service slot.
    /// Catalog URLs with a different API must never enter its fallback list.
    pub fn endpoint_api(self, slot: EndpointSlot) -> Option<crate::EndpointApi> {
        use crate::EndpointApi as Api;
        let chain = self.mainnet_counterpart();
        match slot {
            EndpointSlot::Primary => Some(match chain {
                c if c.is_evm() => Api::EvmJsonRpc,
                Chain::Bitcoin => Api::Esplora,
                Chain::BitcoinCash
                | Chain::Litecoin
                | Chain::Zcash
                | Chain::BitcoinGold
                | Chain::Dash => Api::Blockbook,
                Chain::BitcoinSV => Api::Whatsonchain,
                Chain::Dogecoin => Api::Blockcypher,
                Chain::Solana => Api::SolanaJsonRpc,
                Chain::Tron => Api::TronHttp,
                Chain::Stellar => Api::Horizon,
                Chain::Xrp => Api::XrplJsonRpc,
                Chain::Cardano => Api::Koios,
                Chain::Polkadot | Chain::Bittensor => Api::SubstrateJsonRpc,
                Chain::Sui => Api::SuiJsonRpc,
                Chain::Aptos => Api::AptosRest,
                Chain::Ton => Api::ToncenterV2,
                Chain::Near => Api::NearJsonRpc,
                Chain::Icp => Api::IcpRosetta,
                Chain::Monero => Api::MoneroDaemonRpc,
                Chain::Decred => Api::Insight,
                Chain::Kaspa => Api::KaspaRest,
                _ => return None,
            }),
            EndpointSlot::Secondary => match chain {
                Chain::Ton => Some(Api::ToncenterV3),
                _ => None,
            },
            EndpointSlot::Explorer => match chain {
                Chain::Near => Some(Api::Nearblocks),
                Chain::Xrp => Some(Api::Xrpscan),
                _ => None,
            },
        }
    }

    pub fn has_send_preview(self) -> bool {
        // The EVM family and the chains with a preview path of their own —
        // everything that does not go through the generic submit — always have
        // one. The rest need either a shared-path shape or a fee fallback.
        self.is_evm()
            || !self.uses_generic_send_submit()
            || self.simple_preview_chain().is_some()
            || self.send_execution_shape().fee_fallback > 0.0
    }

    /// How core moves a send here. Only meaningful where
    /// [`Chain::has_send_preview`] is true; that is the gate on the card this
    /// answers for.
    pub fn send_broadcast_mode(self) -> SendBroadcastMode {
        SendBroadcastMode::SignsAndBroadcasts
    }

    /// This chain's send builder can sign a transaction and stop, without
    /// putting it on the chain.
    ///
    /// Capabilities follow the staged builder; unsupported protocols refuse early.
    pub fn supports_sign_only(self) -> bool {
        self.has_send_preview() && self.transparent_send_unavailable_reason().is_none()
    }

    /// ICP currently presents a static neuron directory; it performs no endpoint query.
    pub fn staking_uses_endpoint(self) -> bool {
        self.supports_staking() && self != Self::Icp
    }

    pub fn supports_staking(self) -> bool {
        matches!(
            self,
            Chain::Solana | Chain::Sui | Chain::Aptos | Chain::Near | Chain::Polkadot | Chain::Icp
        )
    }

    /// Whether this chain's derivation reads a BIP-32 path.
    ///
    /// Read from the catalog rather than restated: `derivation_path = []` is
    /// how a row says its keys do not come from a path. Monero is the only
    /// mainnet that says it — its spend and view keys come from the seed
    /// directly — and the five chains whose derivation ignores the path it is
    /// handed still carry one, so this is not "does the arm use `p`".
    ///
    /// The distinction matters because "no default path" was an error
    /// everywhere it was asked, and Monero is a chain for which it is the
    /// answer. See `default_path_from_catalog`.
    pub fn uses_derivation_path(self) -> bool {
        crate::chains::default_derivation_path_template_by_id(self.str_id()).is_some()
    }

    /// Returns `true` for chains that are testnets.
    pub fn is_testnet(self) -> bool {
        self.entry().is_testnet
    }

    /// Maps a testnet variant to its mainnet counterpart. Returns `self` for mainnets.
    pub fn mainnet_counterpart(self) -> Chain {
        Chain::from_str_id(&self.entry().family).expect("validated network family")
    }

    /// `true` for every EVM-compatible chain (mainnet or testnet).
    /// Registry-owned and independent of the catalog: catalog initialization
    /// calls this to project the same fact to CLI and platform clients.
    pub fn is_evm(self) -> bool {
        matches!(
            self,
            Chain::Ethereum
                | Chain::Arbitrum
                | Chain::Optimism
                | Chain::Avalanche
                | Chain::Base
                | Chain::EthereumClassic
                | Chain::BnbChain
                | Chain::Hyperliquid
                | Chain::Polygon
                | Chain::Linea
                | Chain::Scroll
                | Chain::Blast
                | Chain::Mantle
                | Chain::Sei
                | Chain::Celo
                | Chain::Cronos
                | Chain::OpBnb
                | Chain::ZkSyncEra
                | Chain::Sonic
                | Chain::Berachain
                | Chain::Unichain
                | Chain::Ink
                | Chain::XLayer
                | Chain::EthereumSepolia
                | Chain::EthereumHoodi
                | Chain::ArbitrumSepolia
                | Chain::OptimismSepolia
                | Chain::BaseSepolia
                | Chain::BnbChainTestnet
                | Chain::AvalancheFuji
                | Chain::PolygonAmoy
                | Chain::HyperliquidTestnet
                | Chain::EthereumClassicMordor
        )
    }

    /// Why the current protocol adapter cannot safely expose separate stages.
    pub fn transparent_send_unavailable_reason(self) -> Option<&'static str> {
        None
    }

    /// Minimum retained change for the fixed-fee P2PKH send adapters.
    pub(crate) fn legacy_change_dust(self) -> Result<u64, String> {
        match self.mainnet_counterpart() {
            Self::BitcoinCash
            | Self::BitcoinSV
            | Self::BitcoinGold
            | Self::Dogecoin
            | Self::Litecoin
            | Self::Dash => Ok(546),
            _ => Err("Not a fixed-fee P2PKH send protocol".into()),
        }
    }

    pub(crate) fn monero_network_name(self) -> Result<&'static str, String> {
        match self {
            Self::Monero => Ok("mainnet"),
            Self::MoneroStagenet => Ok("stagenet"),
            _ => Err("Not a Monero network".into()),
        }
    }

    pub(crate) fn icp_ledger_id(self) -> Result<&'static str, String> {
        match self {
            Self::Icp => Ok("00000000000000020101"),
            _ => Err("Not the ICP ledger network".into()),
        }
    }

    /// Source: zcash/zcash src/chainparams.cpp and consensus/upgrades.cpp.
    pub(crate) fn zcash_consensus_branch(self, height: u32) -> Result<u32, String> {
        let activations = match self {
            Self::Zcash => [1_687_104, 2_726_400, 3_146_400, 3_364_600],
            Self::ZcashTestnet => [1_842_420, 2_976_000, 3_536_500, 4_052_000],
            _ => return Err("Not a Zcash network".into()),
        };
        let branches = [0xc2d6_d0b4, 0xc8e7_1055, 0x4dec_4df0, 0x5437_f330];
        activations
            .into_iter()
            .zip(branches)
            .rev()
            .find(|(activation, _)| height >= *activation)
            .map(|(_, branch)| branch)
            .ok_or("Zcash V5 is not active".into())
    }

    pub(crate) fn zcash_genesis(self) -> Result<&'static str, String> {
        match self {
            Self::Zcash => Ok("00040fe8ec8471911baa1db1266ea15dd06b4a8a5c453883c000b031973dce08"),
            Self::ZcashTestnet => {
                Ok("05a60a92d99d85997cce3b87616c089f6124d7342af37106edc76126334a2c38")
            }
            _ => Err("Not a Zcash network".into()),
        }
    }

    pub fn stellar_network_passphrase(self) -> Result<&'static str, String> {
        match self {
            Self::Stellar => Ok("Public Global Stellar Network ; September 2015"),
            Self::StellarTestnet => Ok("Test SDF Network ; September 2015"),
            _ => Err("Not a Stellar network".into()),
        }
    }

    /// Aptos network identity bound into each locally constructed transaction.
    pub fn aptos_chain_id(self) -> Option<u8> {
        match self {
            Self::Aptos => Some(1),
            Self::AptosTestnet => Some(2),
            _ => None,
        }
    }

    /// EIP-155 chain id. Refuses chains outside the EVM family.
    pub fn evm_chain_id(self) -> Result<u64, String> {
        Ok(match self {
            Chain::Ethereum => 1,
            Chain::Arbitrum => 42161,
            Chain::Optimism => 10,
            Chain::Avalanche => 43114,
            Chain::Base => 8453,
            Chain::EthereumClassic => 61,
            Chain::BnbChain => 56,
            Chain::Hyperliquid => 999,
            Chain::Polygon => 137,
            Chain::Linea => 59144,
            Chain::Scroll => 534352,
            Chain::Blast => 81457,
            Chain::Mantle => 5000,
            Chain::Sei => 1329,
            Chain::Celo => 42220,
            Chain::Cronos => 25,
            Chain::OpBnb => 204,
            Chain::ZkSyncEra => 324,
            Chain::Sonic => 146,
            Chain::Berachain => 80094,
            Chain::Unichain => 130,
            Chain::Ink => 57073,
            Chain::XLayer => 196,
            Chain::EthereumSepolia => 11155111,
            Chain::EthereumHoodi => 560048,
            Chain::ArbitrumSepolia => 421614,
            Chain::OptimismSepolia => 11155420,
            Chain::BaseSepolia => 84532,
            Chain::BnbChainTestnet => 97,
            Chain::AvalancheFuji => 43113,
            Chain::PolygonAmoy => 80002,
            Chain::HyperliquidTestnet => 998,
            Chain::EthereumClassicMordor => 63,
            _ => return Err(format!("{} is not an EVM chain", self.str_id())),
        })
    }

    /// The keyless explorer source for this EVM chain, if configured.
    pub fn evm_history_source(self) -> EvmHistorySource<'static> {
        crate::app_core::endpoint_catalog()
            .ok()
            .and_then(|catalog| {
                catalog.endpoint_records.iter().find(|record| {
                    record.chain_id == self.str_id()
                        && record.api == Some(crate::EndpointApi::Blockscout)
                        && record.capabilities.iter().any(|cap| cap == "history")
                })
            })
            .map(|record| EvmHistorySource::Open(record.endpoint.as_str()))
            .unwrap_or(EvmHistorySource::Unavailable)
    }

    /// Endpoint-table key for a given logical slot.
    /// Primary → chain str_id; Secondary → "id:secondary"; Explorer → "id:explorer".
    pub fn endpoint_str_id(self, slot: EndpointSlot) -> String {
        match slot {
            EndpointSlot::Primary => self.str_id().to_string(),
            EndpointSlot::Secondary => format!("{}:secondary", self.str_id()),
            EndpointSlot::Explorer => format!("{}:explorer", self.str_id()),
        }
    }

    // ── Native-coin metadata

    /// A zero-balance holding of this network's native coin: what a wallet
    /// starts with before its first refresh.
    pub fn native_holding_template(self) -> crate::store::wallet_domain::AssetHolding {
        crate::store::wallet_domain::AssetHolding {
            name: self.coin_name().to_string(),
            symbol: self.coin_symbol().to_string(),
            coingecko_id: self.coingecko_id().to_string(),
            chain_name: self.chain_display_name().to_string(),
            token_standard: "Native".to_string(),
            contract_address: None,
            amount: 0.0,
            price_usd: 0.0,
        }
    }

    pub fn coin_name(self) -> &'static str {
        self.entry().native_asset_display_name.as_str()
    }

    /// The native token's symbol, joined through the network's deployment reference.
    pub fn coin_symbol(self) -> &'static str {
        self.entry().gas_token_symbol.as_str()
    }

    /// The name shown to a user — the catalog's `name`.
    pub fn chain_display_name(self) -> &'static str {
        self.entry().name.as_str()
    }

    pub fn native_decimals(self) -> u8 {
        self.entry().native_decimals as u8
    }

    pub fn coingecko_id(self) -> &'static str {
        self.entry().native_coingecko_id.as_str()
    }

    /// Selectable networks: mainnet first, then testnets in registry order.
    pub fn network_choices(self) -> Vec<Chain> {
        let mainnet = self.mainnet_counterpart();
        std::iter::once(mainnet)
            .chain(
                Chain::all().filter(move |c| c.is_testnet() && c.mainnet_counterpart() == mainnet),
            )
            .collect()
    }

    /// The `bitcoin` crate's network for this chain.
    ///
    /// Replaces `bitcoin_network_for_mode(&str)`, which matched on the mode
    /// strings — one more table saying what the registry already knows.
    /// Non-UTXO chains answer `Bitcoin`, which is what the mode-string version
    /// did for anything it did not recognise.
    pub fn bitcoin_network(self) -> bitcoin::Network {
        match self {
            Chain::BitcoinTestnet
            | Chain::LitecoinTestnet
            | Chain::BitcoinCashTestnet
            | Chain::BitcoinSVTestnet
            | Chain::DogecoinTestnet
            | Chain::ZcashTestnet
            | Chain::DecredTestnet
            | Chain::DashTestnet => bitcoin::Network::Testnet,
            Chain::BitcoinTestnet4 => bitcoin::Network::Testnet4,
            Chain::BitcoinSignet => bitcoin::Network::Signet,
            _ => bitcoin::Network::Bitcoin,
        }
    }

    /// The JSON-RPC method that answers "is this node alive", or `None` for a
    /// chain whose endpoints are checked over plain HTTP.
    ///
    /// Three of these were spelled in three different Swift functions, and
    /// *which* endpoints were RPC was decided by two hand-written id lists
    /// (`NearBalanceService.rpcEndpointCatalog`,
    /// `PolkadotBalanceService.sidecarEndpointCatalog`) beside a catalog that
    /// already carries an `rpc` role per endpoint. Both agreed when this was
    /// written; adding a provider meant editing the JSON and remembering the
    /// Swift list, and forgetting the second probes a JSON-RPC node with a
    /// GET — which many of them answer 405, reported as unreachable.
    pub fn rpc_health_method(self) -> Option<&'static str> {
        self.endpoint_api(EndpointSlot::Primary)?
            .rpc_health_method()
    }

    /// Whether this chain's native send needs nothing beyond a destination, an
    /// amount and the fee its preview already supplied.
    ///
    /// The mainnets that answer no are the EVM family, which needs a nonce and
    /// gas overrides, and five that each need something only they have: a UTXO
    /// selection (Bitcoin, Dogecoin), a resolved source account (Internet
    /// Computer), a resource model (Tron) and a mint account (Solana).
    ///
    /// It is a chain fact and it lived as two lists of names in
    /// `AppState+SendExecution`, next to a comment saying the lists should not
    /// be there. A chain reaching the shared path had to be added to whichever
    /// of the two the author happened to be looking at.
    pub fn uses_generic_send_submit(self) -> bool {
        matches!(
            self.mainnet_counterpart(),
            Chain::Sui
                | Chain::Aptos
                | Chain::Ton
                | Chain::Xrp
                | Chain::Stellar
                | Chain::Cardano
                | Chain::Polkadot
                | Chain::Near
                | Chain::BitcoinCash
                | Chain::BitcoinSV
                | Chain::Litecoin
                | Chain::Zcash
                | Chain::BitcoinGold
                | Chain::Decred
                | Chain::Kaspa
                | Chain::Dash
                | Chain::Bittensor
                // Its Swift branch was `submitNativeChainSend` plus
                // `moneroPriority: 2` — the same default `build_send_params`
                // already applies with `unwrap_or(2)`, so the branch existed to
                // pass a value core would have supplied. It has a shared-path
                // preview like the rest.
                | Chain::Monero
        )
    }

    /// What a front end needs to assemble a send for this chain.
    ///
    /// Transcribed from the ten call sites that carried these inline; the
    /// values are theirs, not new decisions. The one thing worth noticing is
    /// that `fee_decimals` is 6 nearly everywhere and 7 for Stellar and 8 for
    /// the UTXO chains — a display choice, unrelated to native decimals, which
    /// is why it could not simply be looked up.
    pub fn send_execution_shape(self) -> SendExecutionShape {
        let chain = self.mainnet_counterpart();
        match chain {
            Chain::Sui => SendExecutionShape {
                fee_decimals: 6,
                supports_private_key: false,
                fee_field: SendFeeField::GasBudget,
                fee_fallback: 0.0,
            },
            Chain::Cardano => SendExecutionShape {
                fee_decimals: 6,
                supports_private_key: false,
                fee_field: SendFeeField::FeeAmount,
                fee_fallback: 0.0,
            },
            Chain::Stellar => SendExecutionShape {
                fee_decimals: 7,
                supports_private_key: true,
                fee_field: SendFeeField::None,
                fee_fallback: 0.0,
            },
            Chain::Xrp => SendExecutionShape {
                fee_decimals: 6,
                supports_private_key: true,
                fee_field: SendFeeField::None,
                fee_fallback: 0.0,
            },
            // Bitcoin was missing from a table whose own comment says the
            // UTXO chains are 8, because the ten call sites it was transcribed
            // from did not include Bitcoin's — Bitcoin has an arm of its own.
            // It fell to the default 6, which truncates a satoshi-denominated
            // fee by two digits.
            Chain::Bitcoin | Chain::BitcoinCash | Chain::BitcoinSV => SendExecutionShape {
                fee_decimals: 8,
                supports_private_key: false,
                fee_field: SendFeeField::FeeSats,
                fee_fallback: 0.00001,
            },
            Chain::Litecoin => SendExecutionShape {
                fee_decimals: 8,
                supports_private_key: false,
                fee_field: SendFeeField::FeeSats,
                fee_fallback: 0.0001,
            },
            // The five whose send existed but was unroutable. `fee_fallback`
            // is the default `execute_send` already applies when the request
            // carries no `fee_sat`, in the chain's own units — so the fee the
            // sheet shows and validates against is the fee core will use.
            // None of them has a shared-path preview, and without a fallback
            // the generic submit refuses for want of an estimate.
            Chain::Zcash | Chain::BitcoinGold | Chain::Kaspa => SendExecutionShape {
                fee_decimals: 8,
                supports_private_key: true,
                fee_field: SendFeeField::FeeSats,
                fee_fallback: 0.00001,
            },
            Chain::Decred | Chain::Dash => SendExecutionShape {
                fee_decimals: 8,
                supports_private_key: true,
                fee_field: SendFeeField::FeeSats,
                fee_fallback: 0.00002,
            },
            // e8s, like the UTXO chains. Same omission, same cause.
            Chain::Icp => SendExecutionShape {
                fee_decimals: 8,
                supports_private_key: true,
                fee_field: SendFeeField::None,
                fee_fallback: 0.0,
            },
            _ => SendExecutionShape {
                fee_decimals: 6,
                supports_private_key: false,
                fee_field: SendFeeField::None,
                fee_fallback: 0.0,
            },
        }
    }

    /// How this chain's pending transactions reach a final status.
    ///
    /// Resolved through the mainnet counterpart so a testnet cannot diverge.
    pub fn pending_status_poll(self) -> PendingStatusPoll {
        let chain = self.mainnet_counterpart();
        match chain {
            // Litecoin tracks receives too: its explorer confirms them on a
            // different cadence than the send path assumes.
            Chain::Litecoin => PendingStatusPoll::Utxo {
                require_send_kind: false,
            },
            Chain::Bitcoin | Chain::BitcoinCash | Chain::BitcoinSV | Chain::Dogecoin => {
                PendingStatusPoll::Utxo {
                    require_send_kind: true,
                }
            }
            Chain::Tron
            | Chain::Solana
            | Chain::Cardano
            | Chain::Xrp
            | Chain::Stellar
            | Chain::Monero
            | Chain::Sui
            | Chain::Aptos
            | Chain::Ton
            | Chain::Icp
            | Chain::Near
            | Chain::Polkadot => PendingStatusPoll::HistoryTxids,
            other if other.is_evm() => PendingStatusPoll::EvmReceipt,
            _ => PendingStatusPoll::None,
        }
    }

    /// The EVM family gates a non-native asset on it being a supported token.
    ///
    /// One rule now, on the side that refuses early: being told no is better
    /// than a signed transaction that cannot land.
    pub fn send_rule(self) -> SendRule {
        let chain = self.mainnet_counterpart();
        match chain {
            Chain::EthereumClassic | Chain::Hyperliquid => SendRule::NativeOnly,
            Chain::Solana => SendRule::SupportedSolanaCoin,
            _ if chain.is_evm() => SendRule::NativeOrSupportedToken,
            _ => SendRule::Any,
        }
    }

    /// How incoming history merges with stored history.
    /// Exhaustive so each new chain must specify its merge rule.
    pub fn transaction_merge_strategy(
        self,
    ) -> crate::fetch::transactions::TransactionMergeStrategy {
        use crate::fetch::transactions::TransactionMergeStrategy as S;
        // Resolved through the mainnet counterpart so a testnet can never merge
        // differently from the chain it mirrors — listing them separately is
        // how `zcash-testnet` silently ended up account-based.
        match self.mainnet_counterpart() {
            // Dogecoin's own variant: its explorer reports change outputs in a
            // shape the shared UTXO merge mishandles.
            Chain::Dogecoin => S::Dogecoin,
            Chain::Bitcoin
            | Chain::BitcoinCash
            | Chain::BitcoinSV
            | Chain::Litecoin
            | Chain::BitcoinGold
            | Chain::Dash
            | Chain::Decred
            | Chain::Zcash
            | Chain::Kaspa => S::StandardUtxo,
            other if other.is_evm() => S::Evm,
            _ => S::AccountBased,
        }
    }

    /// Whether the merge identity for this chain includes the asset symbol.
    ///
    /// Tron carries multiple assets on one transaction hash, so hash alone is
    /// not a unique key there.
    pub fn merge_identity_includes_symbol(self) -> bool {
        matches!(self.mainnet_counterpart(), Chain::Tron)
    }

    /// Encode a discovered public child using this chain's supported address format.
    pub(crate) fn encode_discovery_address(
        self,
        key: &secp256k1::PublicKey,
        script: crate::derivation::types::BitcoinScriptType,
    ) -> Result<String, String> {
        use crate::derivation::types::BitcoinScriptType;
        use crate::derivation::{
            bitcoin as btc, bitcoin_cash as bch, bitcoin_sv as bsv, dogecoin as doge,
            litecoin as ltc,
        };
        Ok(match self {
            Self::Bitcoin => return btc::encode_address_inner(btc::BTC_MAINNET, script, key),
            Self::BitcoinTestnet | Self::BitcoinTestnet4 | Self::BitcoinSignet => {
                return btc::encode_address_inner(btc::BTC_TESTNET, script, key)
            }
            Self::BitcoinCash => btc::encode_p2pkh(bch::BCH_MAINNET_VERSION, &key.serialize()),
            Self::BitcoinCashTestnet => {
                btc::encode_p2pkh(bch::BCH_TESTNET_VERSION, &key.serialize())
            }
            Self::BitcoinSV => btc::encode_p2pkh(bsv::BSV_MAINNET_VERSION, &key.serialize()),
            Self::BitcoinSVTestnet => btc::encode_p2pkh(bsv::BSV_TESTNET_VERSION, &key.serialize()),
            Self::Dogecoin => btc::encode_p2pkh(doge::DOGE_MAINNET_VERSION, &key.serialize()),
            Self::DogecoinTestnet => {
                btc::encode_p2pkh(doge::DOGE_TESTNET_VERSION, &key.serialize())
            }
            Self::Litecoin | Self::LitecoinTestnet => {
                if !matches!(script, BitcoinScriptType::P2pkh) {
                    return Err("Litecoin discovery only supports P2PKH paths".into());
                }
                btc::encode_p2pkh(
                    if self == Self::Litecoin {
                        ltc::LTC_MAINNET_VERSION
                    } else {
                        ltc::LTC_TESTNET_VERSION
                    },
                    &key.serialize(),
                )
            }
            _ => return Err("chain does not support UTXO discovery".into()),
        })
    }

    /// What a token send on this chain needs in the gas asset before it can
    /// land, when no preview estimates the fee for that path.
    ///
    /// NEAR is the one chain that routes a token send with no fee estimate to
    /// check against, so the floor is the whole check. It was `0.001` written
    /// into the iOS submit branch, next to the NEAR balance it was compared
    /// with — a number about a chain, held by the front end.
    pub fn token_send_gas_reserve(self) -> Option<f64> {
        match self {
            Chain::Near => Some(0.001),
            _ => None,
        }
    }

    pub fn supports_deep_utxo_discovery(self) -> bool {
        matches!(
            self,
            Chain::Bitcoin
                | Chain::BitcoinCash
                | Chain::BitcoinSV
                | Chain::Litecoin
                | Chain::Dogecoin
                | Chain::BitcoinTestnet
                | Chain::BitcoinTestnet4
                | Chain::BitcoinSignet
                | Chain::BitcoinCashTestnet
                | Chain::BitcoinSVTestnet
                | Chain::LitecoinTestnet
                | Chain::DogecoinTestnet
        )
    }

    /// Does a name typed as a destination resolve to an address on this chain?
    ///
    /// ENS is a registry deployed on Ethereum mainnet, so that is the only
    /// chain a `.eth` name is looked up for. The address it returns is a plain
    /// EVM address that would spend anywhere, but a name pointing at a mainnet
    /// contract need not point at anything on an L2 — a destination reached by
    /// name is accepted only where the registry that named it lives, which is
    /// the stricter of the two readings.
    ///
    /// Swift asked `chainName == "Ethereum"` for this in three places: the
    /// composer's recipient probe, the EVM preview and the submit path. One
    /// fact stated three times is three chances for them to disagree.
    pub fn resolves_ens_names(self) -> bool {
        matches!(self, Chain::Ethereum)
    }

    /// Canonical address normalization for storage and comparison.
    /// Testnets follow their mainnet; EVM addresses are lowercased.
    pub fn address_normalization(self) -> AddressNormalization {
        if self.is_evm() {
            return AddressNormalization::Lowercase;
        }
        match self.mainnet_counterpart() {
            Chain::Sui | Chain::Aptos => AddressNormalization::LowercaseHexPrefixed,
            Chain::Icp | Chain::Near => AddressNormalization::Lowercase,
            _ => AddressNormalization::None,
        }
    }

    /// Address-format key used by validation. Exhaustive so each new chain
    /// must declare its format; EVM chains share the network-agnostic format.
    pub fn address_validation_kind(self) -> &'static str {
        match self {
            // EVM: one format, network-agnostic on the wire.
            Chain::Ethereum
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Avalanche
            | Chain::Base
            | Chain::EthereumClassic
            | Chain::BnbChain
            | Chain::Hyperliquid
            | Chain::Polygon
            | Chain::Linea
            | Chain::Scroll
            | Chain::Blast
            | Chain::Mantle
            | Chain::Sei
            | Chain::Celo
            | Chain::Cronos
            | Chain::OpBnb
            | Chain::ZkSyncEra
            | Chain::Sonic
            | Chain::Berachain
            | Chain::Unichain
            | Chain::Ink
            | Chain::XLayer => "evm",
            Chain::EthereumSepolia
            | Chain::EthereumHoodi
            | Chain::ArbitrumSepolia
            | Chain::OptimismSepolia
            | Chain::BaseSepolia
            | Chain::BnbChainTestnet
            | Chain::AvalancheFuji
            | Chain::PolygonAmoy
            | Chain::HyperliquidTestnet
            | Chain::EthereumClassicMordor => "evmTestnet",

            Chain::Bitcoin => "bitcoin",
            Chain::BitcoinTestnet => "bitcoinTestnet",
            Chain::BitcoinTestnet4 => "bitcoinTestnet4",
            Chain::BitcoinSignet => "bitcoinSignet",
            Chain::BitcoinCash => "bitcoinCash",
            Chain::BitcoinCashTestnet => "bitcoinCashTestnet",
            Chain::BitcoinSV => "bitcoinSV",
            Chain::BitcoinSVTestnet => "bitcoinSVTestnet",
            Chain::Litecoin => "litecoin",
            Chain::LitecoinTestnet => "litecoinTestnet",
            Chain::Dogecoin => "dogecoin",
            Chain::DogecoinTestnet => "dogecoinTestnet",
            Chain::Tron => "tron",
            Chain::TronNile => "tronTestnet",
            Chain::Solana => "solana",
            Chain::SolanaDevnet => "solanaDevnet",
            Chain::Stellar => "stellar",
            Chain::StellarTestnet => "stellarTestnet",
            Chain::Xrp => "xrp",
            Chain::XrpTestnet => "xrpTestnet",
            Chain::Sui => "sui",
            Chain::SuiTestnet => "suiTestnet",
            Chain::Aptos => "aptos",
            Chain::AptosTestnet => "aptosTestnet",
            Chain::Ton => "ton",
            Chain::TonTestnet => "tonTestnet",
            Chain::Icp => "internetComputer",
            Chain::Near => "near",
            Chain::NearTestnet => "nearTestnet",
            Chain::Polkadot => "polkadot",
            Chain::PolkadotWestend => "polkadotTestnet",
            Chain::Monero => "monero",
            Chain::MoneroStagenet => "moneroStagenet",
            Chain::Cardano => "cardano",
            Chain::CardanoPreprod => "cardanoTestnet",
            Chain::Zcash => "zcash",
            Chain::ZcashTestnet => "zcashTestnet",
            Chain::BitcoinGold => "bitcoinGold",
            Chain::Decred => "decred",
            Chain::DecredTestnet => "decredTestnet",
            Chain::Kaspa => "kaspa",
            Chain::KaspaTestnet => "kaspaTestnet",
            Chain::Dash => "dash",
            Chain::DashTestnet => "dashTestnet",
            Chain::Bittensor => "bittensor",
        }
    }

    /// `true` when a wallet on this chain can be imported watch-only from an
    /// address alone.
    ///
    /// Monero is the notable exclusion: watching a Monero account needs the
    /// private view key, which an address does not carry. Testnets are excluded
    /// because import only populates mainnet slots — see [`Chain::address_slot`].
    pub fn supports_watch_only_import(self) -> bool {
        if self.is_testnet() {
            return false;
        }
        if self.is_evm() {
            return true;
        }
        matches!(
            self,
            Chain::Bitcoin
                | Chain::BitcoinCash
                | Chain::BitcoinSV
                | Chain::Litecoin
                | Chain::Dogecoin
                | Chain::Tron
                | Chain::Solana
                | Chain::Xrp
                | Chain::Stellar
                | Chain::Cardano
                | Chain::Sui
                | Chain::Aptos
                | Chain::Ton
                | Chain::Icp
                | Chain::Near
                | Chain::Polkadot
                | Chain::Zcash
                | Chain::BitcoinGold
                | Chain::Decred
                | Chain::Kaspa
                | Chain::Dash
                | Chain::Bittensor
        )
    }

    pub fn flags_evm_address_as_wrong_chain(self) -> bool {
        matches!(
            self,
            Chain::Bitcoin
                | Chain::BitcoinCash
                | Chain::Litecoin
                | Chain::Dogecoin
                | Chain::BitcoinTestnet
                | Chain::BitcoinTestnet4
                | Chain::BitcoinSignet
                | Chain::BitcoinCashTestnet
                | Chain::LitecoinTestnet
                | Chain::DogecoinTestnet
        )
    }

    pub fn static_fee_units(self) -> Option<u128> {
        match self {
            Chain::Solana => Some(5_000),
            Chain::Tron => Some(1_000_000),
            Chain::Cardano => Some(170_000),
            Chain::Polkadot => Some(160_000_000),
            Chain::Bittensor => Some(125_000),
            Chain::Sui => Some(1_000),
            Chain::Ton => Some(7_000_000),
            Chain::Icp => Some(10_000),
            Chain::Zcash | Chain::ZcashTestnet => Some(10_000),
            Chain::Monero => Some(500_000_000),
            Chain::Dogecoin => Some(1_000_000),
            Chain::Litecoin | Chain::BitcoinSV | Chain::BitcoinGold | Chain::Kaspa => Some(1_000),
            Chain::BitcoinCash | Chain::Decred | Chain::Dash => Some(2_000),
            Chain::SolanaDevnet => Some(5_000),
            Chain::TronNile => Some(1_000_000),
            Chain::CardanoPreprod => Some(170_000),
            Chain::PolkadotWestend => Some(160_000_000),
            Chain::SuiTestnet => Some(1_000),
            Chain::TonTestnet => Some(7_000_000),
            Chain::MoneroStagenet => Some(500_000_000),
            Chain::DogecoinTestnet => Some(1_000_000),
            Chain::LitecoinTestnet | Chain::BitcoinSVTestnet | Chain::KaspaTestnet => Some(1_000),
            Chain::BitcoinCashTestnet | Chain::DecredTestnet | Chain::DashTestnet => Some(2_000),
            Chain::Bitcoin | Chain::Xrp | Chain::Stellar | Chain::Aptos => None,
            Chain::BitcoinTestnet
            | Chain::BitcoinTestnet4
            | Chain::BitcoinSignet
            | Chain::XrpTestnet
            | Chain::StellarTestnet
            | Chain::AptosTestnet => None,
            Chain::Ethereum
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Avalanche
            | Chain::Base
            | Chain::EthereumClassic
            | Chain::BnbChain
            | Chain::Hyperliquid
            | Chain::Polygon
            | Chain::Linea
            | Chain::Scroll
            | Chain::Blast
            | Chain::Mantle
            | Chain::Sei
            | Chain::Celo
            | Chain::Cronos
            | Chain::OpBnb
            | Chain::ZkSyncEra
            | Chain::Sonic
            | Chain::Berachain
            | Chain::Unichain
            | Chain::Ink
            | Chain::XLayer => None,
            Chain::EthereumSepolia
            | Chain::EthereumHoodi
            | Chain::ArbitrumSepolia
            | Chain::OptimismSepolia
            | Chain::BaseSepolia
            | Chain::BnbChainTestnet
            | Chain::AvalancheFuji
            | Chain::PolygonAmoy
            | Chain::HyperliquidTestnet
            | Chain::EthereumClassicMordor => None,
            Chain::Near => None,
            Chain::NearTestnet => None,
        }
    }

    /// Iterator over every known chain.
    pub fn all() -> impl Iterator<Item = Self> {
        ALL_CHAINS.iter().copied()
    }

    /// Iterator over only mainnet chains.
    pub fn mainnets() -> impl Iterator<Item = Self> {
        Self::all().filter(|c| !c.is_testnet())
    }

    /// Iterator over only testnet chains.
    pub fn testnets() -> impl Iterator<Item = Self> {
        Self::all().filter(|c| c.is_testnet())
    }

    /// Resolve a chain from the display name used on the boundary.
    ///
    /// No special cases: the enum and `chains.toml` agree on every name, and
    /// `every_catalog_name_resolves` fails if they ever stop.
    pub fn from_display_name(name: &str) -> Option<Self> {
        Chain::all().find(|c| c.chain_display_name() == name)
    }
}

/// How a chain's estimated fee enters its signing request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SendFeeField {
    /// Sui: the fee becomes the transaction's gas budget.
    GasBudget,
    /// Cardano: the fee is passed as an explicit amount.
    FeeAmount,
    /// UTXO chains: the fee is converted to satoshis.
    FeeSats,
    /// The chain computes its own fee at signing time.
    None,
}

/// How core moves a send on this chain — which is what the send screen's
/// network card tells the user it is about to do.
///
/// The card's sentence was twelve strings in the iOS view, one per chain, and
/// they had drifted into four verbs for two behaviours: "signs and broadcasts
/// X transfers", "signs and broadcasts X payments", "signs and broadcasts ADA
/// transfers" (the symbol, not the chain) and Monero's, which is the only one
/// that describes something different. Two behaviours, so two variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SendBroadcastMode {
    /// Core signs on the device and broadcasts to this chain's endpoints.
    SignsAndBroadcasts,
    /// Core prepares the transfer against the configured backend, which also
    /// quotes the fee. Monero has no in-process wallet to sign with, so the
    /// backend does both halves and the card must not claim otherwise.
    PreparesWithBackend,
}

/// What a front end needs to assemble a send for this chain, beyond the
/// amount and the destination.
#[derive(Debug, Clone, Copy, uniffi::Record)]
pub struct SendExecutionShape {
    /// Decimal places to show when reporting that the balance cannot cover
    /// the fee. A display precision, not the chain's native decimals.
    pub fee_decimals: u8,
    /// Whether a wallet holding only a private key (no seed) can sign here.
    pub supports_private_key: bool,
    pub fee_field: SendFeeField,
    /// Fee to assume when no preview is available, in native units. Zero
    /// where the chain always has a preview by the time a send is submitted.
    pub fee_fallback: f64,
}

/// How a chain's pending transactions are polled for confirmation.
///
/// A per-chain fact, so it lives here rather than as one wrapper function per
/// chain in the shell — there were eighteen of those, each naming a chain, a
/// chain id, an address resolver and up to two flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PendingStatusPoll {
    /// Ask the chain's own status endpoint for a txid.
    Utxo {
        /// Only sends are tracked; receives confirm on their own.
        require_send_kind: bool,
    },
    /// Fetch the address's history and treat any txid in it as confirmed.
    HistoryTxids,
    /// Receipt-based, through the EVM history path.
    EvmReceipt,
    /// Not polled.
    None,
}

/// The networks available for a chain's family, mainnet first.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NetworkChoice {
    /// Registry id — what `SelectChainForFamily` takes.
    pub chain_id: String,
    /// What to show in a picker: "Bitcoin", "Bitcoin Testnet4", …
    pub title: String,
    pub is_testnet: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The network card claims in-app signing for every chain that has one
    /// except the backend-prepared one. Monero's testnet answers like Monero:
    /// the backend is what does the work on either network.
    /// Every EVM chain carries its EIP-155 id, and Ethereum's test networks
    /// carry theirs. These were asserted in the iOS suite through an
    /// `EVMChainContext` wrapper the app no longer has; the facts are the
    /// registry's, so this is where they are checked.
    #[test]
    fn evm_chains_carry_their_eip155_ids() {
        for chain in Chain::all().filter(|chain| chain.is_evm()) {
            assert!(
                chain.evm_chain_id().unwrap() > 0,
                "{} has no chain id",
                chain.str_id()
            );
        }
        assert_eq!(Chain::EthereumSepolia.evm_chain_id().unwrap(), 11_155_111);
        assert_eq!(Chain::EthereumHoodi.evm_chain_id().unwrap(), 560_048);
        assert_eq!(Chain::EthereumClassic.evm_chain_id().unwrap(), 61);
    }

    #[test]
    fn non_evm_chains_have_no_eip155_identity() {
        for chain in Chain::all().filter(|chain| !chain.is_evm()) {
            assert!(chain.evm_chain_id().is_err(), "{}", chain.str_id());
        }
    }

    #[test]
    fn sends_are_signed_on_device() {
        for chain in Chain::all().filter(|c| c.has_send_preview()) {
            assert_eq!(
                chain.send_broadcast_mode(),
                SendBroadcastMode::SignsAndBroadcasts
            );
        }
    }

    /// Exhaustive address validators independently check registry EVM membership.
    #[test]
    fn evm_validation_kind_agrees_with_is_evm() {
        for chain in Chain::all() {
            let kind = chain.address_validation_kind();
            let kind_says_evm = kind == "evm" || kind == "evmTestnet";
            assert_eq!(
                kind_says_evm,
                chain.is_evm(),
                "{} : is_evm()={} but address_validation_kind()={kind:?}",
                chain.str_id(),
                chain.is_evm(),
            );
            if chain.is_evm() {
                assert_eq!(
                    kind,
                    if chain.is_testnet() {
                        "evmTestnet"
                    } else {
                        "evm"
                    },
                    "{} has the wrong EVM flavour",
                    chain.str_id(),
                );
            }
        }
    }

    /// Every chain's kind must be one `validate_address` actually dispatches
    /// on. A kind it doesn't know falls through to `invalid_result()`, which
    /// silently rejects every address on that chain — the failure mode the
    /// old per-module copies of this table had.
    #[test]
    fn every_chain_has_a_kind_validate_address_recognises() {
        use crate::validation::address::{validate_address, AddressValidationRequest};
        for chain in Chain::all() {
            let kind = chain.address_validation_kind();
            assert!(!kind.is_empty(), "{} has an empty kind", chain.str_id());
            // A syntactically impossible address: a recognised kind still
            // reports `is_valid == false`, so this can't distinguish on its
            // own. What it does catch is a kind that panics or is blank.
            let result = validate_address(AddressValidationRequest {
                kind: kind.to_string(),
                value: "!".to_string(),
            });
            assert!(!result.is_valid, "{kind} accepted a bogus address");
        }
    }

    /// Address slots: every chain has one, and the EVM family shares Ethereum's.
    #[test]
    fn address_slots_are_shared_across_the_evm_family_only() {
        for chain in Chain::all() {
            let slot = chain.address_slot();
            assert!(!slot.is_empty(), "{} has no slot", chain.str_id());
            if chain.is_evm() && !chain.is_testnet() && chain != Chain::EthereumClassic {
                assert_eq!(
                    slot,
                    Chain::Ethereum.str_id(),
                    "{} should share the ethereum slot",
                    chain.str_id()
                );
            } else {
                assert_eq!(
                    slot,
                    chain.str_id(),
                    "{} should own its slot",
                    chain.str_id()
                );
            }
        }
    }

    /// Watch-only support must never be claimed for a chain with no slot to
    /// read, and Monero must stay excluded.
    #[test]
    fn watch_only_support_excludes_monero_and_testnets() {
        assert!(!Chain::Monero.supports_watch_only_import());
        assert!(!Chain::BitcoinTestnet.supports_watch_only_import());
        assert!(Chain::Bitcoin.supports_watch_only_import());
        assert!(Chain::Polygon.supports_watch_only_import());
        assert!(Chain::EthereumClassic.supports_watch_only_import());
        for chain in Chain::all() {
            if chain.supports_watch_only_import() {
                assert!(!chain.is_testnet(), "{} is a testnet", chain.str_id());
            }
        }
    }

    /// No two chains answer to the same hosting variant, and no variant goes
    /// unclaimed. The count was written as `18`, so growing the enum failed
    /// here with a number rather than a name; `ALL` is the same assertion
    /// without the copy.
    #[test]
    fn token_hosting_chains_map_one_to_one() {
        let mut seen = std::collections::HashMap::new();
        for identity in chain_identities() {
            if let Some(t) = identity.token_hosting_chain {
                if let Some(prev) = seen.insert(format!("{t:?}"), identity.name.clone()) {
                    panic!("{t:?} claimed by both {prev} and {}", identity.name);
                }
            }
        }
        assert_eq!(
            seen.len(),
            crate::store::wallet_domain::CoreTokenHostingChain::ALL.len(),
            "a hosting variant is claimed by no chain"
        );
    }

    /// Monero is the only mainnet the flag excludes, and one piece of iOS copy
    /// depends on that: the watch-only footer note names Monero while its
    /// condition reads the flag. A second excluded chain means generalising the
    /// string, which is a localisation edit rather than something to discover
    /// from a screenshot.
    #[test]
    fn only_monero_is_excluded_from_watch_only_import() {
        let excluded: Vec<&str> = Chain::all()
            .filter(|c| !c.is_testnet() && !c.supports_watch_only_import())
            .map(|c| c.chain_display_name())
            .collect();
        assert_eq!(excluded, vec!["Monero"]);
    }

    #[test]
    fn testnet_derivation_metadata_names_the_concrete_network() {
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            assert_eq!(
                crate::send::flow::seed_derivation_chain_raw(chain).as_deref(),
                Some(chain.chain_display_name())
            );
            assert_eq!(
                crate::registry::evm_seed_derivation_chain(chain).as_deref(),
                chain.is_evm().then_some(chain.chain_display_name())
            );
        }
    }

    #[test]
    fn str_id_roundtrips() {
        for chain in Chain::all() {
            let id = chain.str_id();
            let back = Chain::from_str_id(id).expect("str_id must round-trip");
            assert_eq!(chain, back, "round-trip failed for {id}");
        }
        assert!(Chain::from_str_id("not-a-chain").is_none());
    }

    #[test]
    fn evm_group_includes_mainnets_and_testnets() {
        let mainnet_ids: Vec<&str> = vec![
            "ethereum",
            "arbitrum",
            "optimism",
            "avalanche",
            "base",
            "ethereum-classic",
            "bnb",
            "hyperliquid",
            "polygon",
            "linea",
            "scroll",
            "blast",
            "mantle",
            "sei",
            "celo",
            "cronos",
            "opbnb",
            "zksync-era",
            "sonic",
            "berachain",
            "unichain",
            "ink",
            "x-layer",
        ];
        let testnet_ids: Vec<&str> = vec![
            "ethereum-sepolia",
            "ethereum-hoodi",
            "arbitrum-sepolia",
            "optimism-sepolia",
            "base-sepolia",
            "bnb-testnet",
            "avalanche-fuji",
            "polygon-amoy",
            "hyperliquid-testnet",
            "ethereum-classic-mordor",
        ];
        let mut expected: Vec<&str> = [mainnet_ids, testnet_ids].concat();
        expected.sort();
        let mut actual: Vec<&str> = Chain::all()
            .filter(|c| c.is_evm())
            .map(|c| c.str_id())
            .collect();
        actual.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn testnet_mainnet_counterparts_are_mainnets() {
        for testnet in Chain::testnets() {
            let counterpart = testnet.mainnet_counterpart();
            assert!(
                !counterpart.is_testnet(),
                "{:?} mainnet_counterpart returned testnet {:?}",
                testnet,
                counterpart
            );
        }
    }

    #[test]
    fn evm_mainnet_chain_ids_match_eip155() {
        assert_eq!(Chain::Ethereum.evm_chain_id().unwrap(), 1);
        assert_eq!(Chain::Arbitrum.evm_chain_id().unwrap(), 42161);
        assert_eq!(Chain::Optimism.evm_chain_id().unwrap(), 10);
        assert_eq!(Chain::Avalanche.evm_chain_id().unwrap(), 43114);
        assert_eq!(Chain::Base.evm_chain_id().unwrap(), 8453);
        assert_eq!(Chain::EthereumClassic.evm_chain_id().unwrap(), 61);
        assert_eq!(Chain::BnbChain.evm_chain_id().unwrap(), 56);
        assert_eq!(Chain::Hyperliquid.evm_chain_id().unwrap(), 999);
        assert_eq!(Chain::Polygon.evm_chain_id().unwrap(), 137);
        assert_eq!(Chain::Linea.evm_chain_id().unwrap(), 59144);
        assert_eq!(Chain::Scroll.evm_chain_id().unwrap(), 534352);
        assert_eq!(Chain::Blast.evm_chain_id().unwrap(), 81457);
        assert_eq!(Chain::Mantle.evm_chain_id().unwrap(), 5000);
        assert_eq!(Chain::Sei.evm_chain_id().unwrap(), 1329);
        assert_eq!(Chain::Celo.evm_chain_id().unwrap(), 42220);
        assert_eq!(Chain::Cronos.evm_chain_id().unwrap(), 25);
        assert_eq!(Chain::OpBnb.evm_chain_id().unwrap(), 204);
        assert_eq!(Chain::ZkSyncEra.evm_chain_id().unwrap(), 324);
        assert_eq!(Chain::Sonic.evm_chain_id().unwrap(), 146);
        assert_eq!(Chain::Berachain.evm_chain_id().unwrap(), 80094);
        assert_eq!(Chain::Unichain.evm_chain_id().unwrap(), 130);
        assert_eq!(Chain::Ink.evm_chain_id().unwrap(), 57073);
        assert_eq!(Chain::XLayer.evm_chain_id().unwrap(), 196);
    }

    #[test]
    fn endpoint_slots_use_string_suffixes() {
        assert_eq!(
            Chain::Polkadot.endpoint_str_id(EndpointSlot::Primary),
            "polkadot"
        );
        assert_eq!(
            Chain::Polkadot.endpoint_str_id(EndpointSlot::Secondary),
            "polkadot:secondary"
        );
        assert_eq!(
            Chain::Ethereum.endpoint_str_id(EndpointSlot::Explorer),
            "ethereum:explorer"
        );
        assert_eq!(
            Chain::Tron.endpoint_str_id(EndpointSlot::Explorer),
            "tron:explorer"
        );
        assert_eq!(
            Chain::Near.endpoint_str_id(EndpointSlot::Explorer),
            "near:explorer"
        );
    }
}

// ── FFI surface ──────────────────────────────────────────────────────────

/// What identifies one chain to a front end: the enum value and the three
/// facts every screen needs to go with it.
/// The canonical form an address is folded to before storage or comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressNormalization {
    /// Case and shape are significant — a Bitcoin or Solana address is used
    /// exactly as the user typed it.
    None,
    Lowercase,
    /// Lowercase, and prefixed with `0x` when the input omitted it.
    LowercaseHexPrefixed,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChainIdentity {
    pub chain: Chain,
    /// The catalog's `id` — what endpoint tables and the FFI boundary key on.
    pub id: String,
    /// The catalog's `name` — the one spelling of this chain.
    pub name: String,
    pub is_testnet: bool,
    pub is_evm: bool,
    /// Which chain's slot this chain's address is stored under. The EVM family
    /// shares Ethereum's.
    pub address_slot: String,
    /// The address format family `validate_address` dispatches on.
    pub address_validation_kind: String,
    /// HD discovery walks this chain's addresses past the last used one.
    pub supports_deep_utxo_discovery: bool,
    /// A watch-only import can carry addresses for this chain.
    pub supports_watch_only_import: bool,
    /// A private key alone yields an address on this chain.
    ///
    /// Was `core_supported_private_key_chain_names`, an export whose whole
    /// body was `Chain::all().filter(…).map(display_name)` — a filter over
    /// this column, made into a call.
    pub derives_from_private_key: bool,
    /// The chain has protocol-native staking the staking tab can drive.
    pub supports_staking: bool,
    /// The send screen has a network card to show for this chain — a fee, a
    /// preview, or both. False only where core routes no send at all.
    pub has_send_preview: bool,
    /// Which `CoreTokenHostingChain` this chain is, if it can host known
    /// tokens. `None` for the chains that cannot.
    ///
    /// `CoreTokenHostingChain::chain_name` and its inverse already collapsed
    /// four copies of this mapping inside Rust; publishing it here removes the
    /// three that were left in Swift, which hand-wrote `rawValue`,
    /// `init?(rawValue:)` and `allCases` for an enum core owns.
    pub token_hosting_chain: Option<crate::store::wallet_domain::CoreTokenHostingChain>,
    pub send_execution_shape: SendExecutionShape,
    /// How core moves a send here, which is what the network card says.
    pub send_broadcast_mode: SendBroadcastMode,
    /// The JSON-RPC method that answers "is this node alive", or `None` for a
    /// chain whose endpoints are checked over plain HTTP.
    pub rpc_health_method: Option<String>,
    pub pending_status_poll: PendingStatusPoll,
    /// Which chain's derivation path this chain reuses, as a display name.
    /// `None` for a chain with no BIP-32 path.
    pub seed_derivation_chain: Option<String>,
    /// The EVM chain whose derivation this chain reuses, or `None` off the
    /// EVM family.
    pub evm_seed_derivation_chain: Option<String>,
    /// The mainnet this chain belongs to, or itself.
    pub mainnet_counterpart: Chain,
    /// The networks this chain's family offers, mainnet first.
    pub network_choices: Vec<NetworkChoice>,
}

/// The whole catalog as identities, in declaration order.
///
/// One call rather than an accessor per column: a front end builds its lookups
/// from this once and then reads them locally, and there is no way to ask for
/// an id without the name that goes with it. `Chain` deliberately has no
/// `CaseIterable` on the Swift side — the order that matters is the catalog's.
#[uniffi::export]
pub fn chain_identities() -> Vec<ChainIdentity> {
    Chain::all()
        .map(|chain| ChainIdentity {
            chain,
            id: chain.str_id().to_string(),
            name: chain.chain_display_name().to_string(),
            is_testnet: chain.is_testnet(),
            is_evm: chain.is_evm(),
            address_slot: chain.address_slot().to_string(),
            address_validation_kind: chain.address_validation_kind().to_string(),
            supports_deep_utxo_discovery: chain.supports_deep_utxo_discovery(),
            supports_watch_only_import: chain.supports_watch_only_import(),
            derives_from_private_key: chain.derives_from_private_key(),
            supports_staking: chain.supports_staking(),
            has_send_preview: chain.has_send_preview(),
            token_hosting_chain:
                crate::store::wallet_domain::CoreTokenHostingChain::from_chain_name(
                    chain.chain_display_name(),
                ),
            send_execution_shape: chain.send_execution_shape(),
            send_broadcast_mode: chain.send_broadcast_mode(),
            rpc_health_method: chain.rpc_health_method().map(str::to_string),
            pending_status_poll: chain.pending_status_poll(),
            seed_derivation_chain: crate::send::flow::seed_derivation_chain_raw(chain),
            evm_seed_derivation_chain: chain
                .is_evm()
                .then(|| evm_seed_derivation_chain(chain))
                .flatten(),
            mainnet_counterpart: chain.mainnet_counterpart(),
            network_choices: chain
                .network_choices()
                .into_iter()
                .map(|c| NetworkChoice {
                    chain_id: c.str_id().to_string(),
                    title: c.chain_display_name().to_string(),
                    is_testnet: c.is_testnet(),
                })
                .collect(),
        })
        .collect()
}

/// Not exported: it is a column of `chain_identities` now.
pub fn evm_seed_derivation_chain(chain: Chain) -> Option<String> {
    if chain.is_testnet() && chain.is_evm() {
        return Some(chain.chain_display_name().to_string());
    }
    Some(
        match chain {
            Chain::Ethereum => "Ethereum",
            Chain::EthereumClassic => "Ethereum Classic",
            Chain::Arbitrum => "Arbitrum",
            Chain::BnbChain => "Ethereum",
            Chain::Avalanche => "Avalanche",
            Chain::Hyperliquid => "Hyperliquid",
            _ => return None,
        }
        .to_string(),
    )
}

#[cfg(test)]
mod catalog_agreement_tests {
    use super::*;

    /// A chain id spelled from the variant's own name, so the check below has
    /// a source independent of the catalog it is checking.
    ///
    /// The six exceptions are the whole list of places where the enum and
    /// `chains.toml` spell a chain differently, which is worth being able to
    /// read in one place.
    fn expected_id(chain: Chain) -> String {
        const EXCEPTIONS: &[(Chain, &str)] = &[
            (Chain::Icp, "internet-computer"),
            (Chain::BnbChain, "bnb"),
            (Chain::OpBnb, "opbnb"),
            (Chain::ZkSyncEra, "zksync-era"),
            (Chain::BitcoinTestnet4, "bitcoin-testnet-4"),
            (Chain::BnbChainTestnet, "bnb-testnet"),
        ];
        if let Some((_, id)) = EXCEPTIONS.iter().find(|(c, _)| *c == chain) {
            return (*id).to_string();
        }
        let name = format!("{chain:?}");
        let mut out = String::with_capacity(name.len() + 4);
        let bytes: Vec<char> = name.chars().collect();
        for (i, ch) in bytes.iter().enumerate() {
            let starts_word = ch.is_uppercase()
                && i > 0
                && (bytes[i - 1].is_lowercase()
                    || bytes[i - 1].is_ascii_digit()
                    || bytes.get(i + 1).is_some_and(|n| n.is_lowercase()));
            if starts_word {
                out.push('-');
            }
            out.extend(ch.to_lowercase());
        }
        out
    }

    /// The enum is an index into `chains.toml`, so the two orders must match
    /// exactly — position by position, not merely as sets.
    ///
    /// Asserting `chain.str_id() == entry.id` would prove nothing: `str_id`
    /// *reads* the catalog now, so the two agree by construction. The variant
    /// name is the independent source, which is why [`expected_id`] exists.
    ///
    /// This replaces `display_names_match_the_catalog`, which asked whether two
    /// tables agreed on a name. There is one table now. The question worth
    /// asking is whether the index is sound — if it is not, every chain
    /// silently becomes a different chain, and unlike a rename that is
    /// invisible from the outside.
    #[test]
    fn chain_order_matches_the_catalog() {
        let catalog = crate::chains::list_all_chains();
        assert_eq!(
            ALL_CHAINS.len(),
            catalog.len(),
            "{} chains in the enum, {} in chains.toml",
            ALL_CHAINS.len(),
            catalog.len()
        );
        for (index, entry) in catalog.iter().enumerate() {
            let chain = ALL_CHAINS[index];
            assert_eq!(
                chain as usize, index,
                "ALL_CHAINS[{index}] is {chain:?}, whose discriminant is {}",
                chain as usize
            );
            assert_eq!(
                expected_id(chain),
                entry.id,
                "position {index}: the enum has {chain:?}, chains.toml has \"{}\"",
                entry.id
            );
        }
    }

    /// Every catalog entry is reachable from the name it publishes, with no
    /// special case in the resolver.
    #[test]
    fn every_catalog_name_resolves() {
        for entry in crate::chains::list_all_chains() {
            assert_eq!(
                Chain::from_display_name(&entry.name).map(Chain::str_id),
                Some(entry.id.as_str()),
                "{} does not resolve from \"{}\"",
                entry.id,
                entry.name
            );
        }
    }
}

#[cfg(test)]
mod fee_decimals_match_the_asset {
    /// A fee is shown and validated at the asset's own precision.
    ///
    /// Bitcoin and Internet Computer fell to the default six while the send
    /// sheet formatted them at eight: satoshis and e8s both need eight, and a
    /// six-decimal fee drops the last two digits.
    #[test]
    fn utxo_and_e8s_chains_use_eight() {
        for name in [
            "Bitcoin",
            "Bitcoin Cash",
            "Bitcoin SV",
            "Litecoin",
            "Internet Computer",
        ] {
            let c = super::Chain::from_display_name(name).unwrap();
            assert_eq!(c.send_execution_shape().fee_decimals, 8, "{name}");
        }
        assert_eq!(super::Chain::Stellar.send_execution_shape().fee_decimals, 7);
    }
}

#[cfg(test)]
mod the_post_send_refresh_set_is_the_registrys {
    use super::{Chain, PendingStatusPoll};

    /// After a send, a chain either polls for a pending status or refreshes
    /// history. Which it does is `pending_status_poll`, and a testnet does what
    /// its mainnet does.
    ///
    /// Swift held `utxoPostSendChains`, a five-name `Set<String>` of the
    /// mainnets, so a send on any of the seven UTXO testnets took the history
    /// refresh instead of the pending one. Same shape as the five-name table
    /// that made address discovery dead on those chains — a hand-written list
    /// shorter than the registry.
    #[test]
    fn every_utxo_testnet_polls_the_way_its_mainnet_does() {
        let utxo = |c: Chain| matches!(c.pending_status_poll(), PendingStatusPoll::Utxo { .. });
        for (mainnet, testnets) in [
            (
                Chain::Bitcoin,
                vec![
                    Chain::BitcoinTestnet,
                    Chain::BitcoinTestnet4,
                    Chain::BitcoinSignet,
                ],
            ),
            (Chain::Litecoin, vec![Chain::LitecoinTestnet]),
            (Chain::BitcoinCash, vec![Chain::BitcoinCashTestnet]),
            (Chain::BitcoinSV, vec![Chain::BitcoinSVTestnet]),
            (Chain::Dogecoin, vec![Chain::DogecoinTestnet]),
        ] {
            assert!(utxo(mainnet), "{mainnet:?}");
            for testnet in testnets {
                assert!(utxo(testnet), "{testnet:?} must poll like {mainnet:?}");
            }
        }
        assert_eq!(
            Chain::all().filter(|c| utxo(*c)).count(),
            12,
            "five mainnets and seven testnets"
        );
    }
}

#[cfg(test)]
mod monero_takes_the_shared_submit_path {
    use super::Chain;

    /// Monero's send is the generic one.
    ///
    /// Its Swift branch was four lines — `submitNativeChainSend(...,
    /// moneroPriority: 2)` — and that `2` is the same default
    /// `build_send_params` applies with `req.monero_priority.unwrap_or(2)`.
    /// Two copies of one default, and a branch whose only job was to carry
    /// one of them.
    ///
    /// It has a shared-path preview, which is what the flag needs: without one
    /// `has_send_preview` would answer through the `!uses_generic_send_submit`
    /// branch and go false the moment the flag flipped.
    #[test]
    fn it_has_the_preview_the_shared_path_needs() {
        assert!(Chain::Monero.uses_generic_send_submit());
        assert!(Chain::Monero.simple_preview_chain().is_some());
        assert!(Chain::Monero.has_send_preview());
    }
}

/// History service strategy; callers never select a protocol implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryRefreshKind {
    Bitcoin,
    Evm,
    Utxo,
    Normalized,
}
impl Chain {
    pub(crate) fn history_refresh_kind(self) -> HistoryRefreshKind {
        if self.mainnet_counterpart() == Chain::Bitcoin {
            HistoryRefreshKind::Bitcoin
        } else if self.is_evm() {
            HistoryRefreshKind::Evm
        } else if self.supports_deep_utxo_discovery() {
            HistoryRefreshKind::Utxo
        } else {
            HistoryRefreshKind::Normalized
        }
    }
}
