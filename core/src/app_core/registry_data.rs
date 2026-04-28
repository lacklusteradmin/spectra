use super::{
    AppCoreAppChainDescriptor, AppCoreBroadcastProviderOption, AppCoreCatalog, AppCoreChainBackend,
    AppCoreChainIntegrationState,
};

// ── chain_backends ─────────────────────────────────────────────────────────
// Most chains are Live with full feature support, so the `live(...)` builder
// captures the default and lets each entry collapse to a single line.

fn live(name: &str, symbols: &[&str]) -> AppCoreChainBackend {
    AppCoreChainBackend {
        chain_name: name.to_string(),
        supported_symbols: symbols.iter().map(|s| s.to_string()).collect(),
        integration_state: AppCoreChainIntegrationState::Live,
        supports_seed_import: true,
        supports_balance_refresh: true,
        supports_receive_address: true,
        supports_send: true,
    }
}

const TRACKED_ERC20: &str = "Tracked ERC-20s";

pub(super) fn chain_backends() -> Vec<AppCoreChainBackend> {
    vec![
        live("Bitcoin", &["BTC"]),
        live("Bitcoin Cash", &["BCH"]),
        live("Bitcoin SV", &["BSV"]),
        live("Litecoin", &["LTC"]),
        live("Ethereum", &["ETH", "USDT", "USDC", "DAI"]),
        live("Arbitrum", &["ETH", TRACKED_ERC20]),
        live("Optimism", &["ETH", TRACKED_ERC20]),
        live("Ethereum Classic", &["ETC"]),
        live("Dogecoin", &["DOGE"]),
        live("BNB Chain", &["BNB"]),
        live("Avalanche", &["AVAX"]),
        live("Hyperliquid", &["HYPE", TRACKED_ERC20]),
        live("Tron", &["TRX", "USDT"]),
        live("Solana", &["SOL"]),
        live("XRP Ledger", &["XRP"]),
        live("Monero", &["XMR"]),
        live("Cardano", &["ADA"]),
        live("Sui", &["SUI"]),
        live("Aptos", &["APT"]),
        live("TON", &["TON", "Tracked Jettons"]),
        live("Internet Computer", &["ICP"]),
        live("NEAR", &["NEAR"]),
        live("Polkadot", &["DOT"]),
        live("Stellar", &["XLM"]),
        live("Polygon", &["POL", TRACKED_ERC20]),
        live("Base", &["ETH", TRACKED_ERC20]),
        live("Linea", &["ETH", TRACKED_ERC20]),
        live("Scroll", &["ETH", TRACKED_ERC20]),
        live("Blast", &["ETH", TRACKED_ERC20]),
        live("Mantle", &["MNT", TRACKED_ERC20]),
        live("Zcash", &["ZEC"]),
        live("Bitcoin Gold", &["BTG"]),
        live("Decred", &["DCR"]),
        live("Kaspa", &["KAS"]),
        live("Dash", &["DASH"]),
        live("Sei", &["SEI", TRACKED_ERC20]),
        live("Celo", &["CELO", TRACKED_ERC20]),
        live("Cronos", &["CRO", TRACKED_ERC20]),
        live("opBNB", &["BNB", TRACKED_ERC20]),
        live("zkSync Era", &["ETH", TRACKED_ERC20]),
        live("Sonic", &["S", TRACKED_ERC20]),
        live("Berachain", &["BERA", TRACKED_ERC20]),
        live("Unichain", &["ETH", TRACKED_ERC20]),
        live("Ink", &["ETH", TRACKED_ERC20]),
        live("X Layer", &["OKB", TRACKED_ERC20]),
        live("Bittensor", &["TAO"]),
    ]
}

pub(super) fn live_chain_names() -> Vec<String> {
    chain_backends()
        .into_iter()
        .filter(|b| matches!(b.integration_state, AppCoreChainIntegrationState::Live))
        .map(|b| b.chain_name)
        .collect()
}

// ── app_chain_descriptors ─────────────────────────────────────────────────

struct DescBuilder<'a> {
    id: &'a str,
    name: &'a str,
    label: &'a str,
    native: &'a str,
    keywords: &'a [&'a str],
    is_evm: bool,
    catalog: bool,
}

impl<'a> DescBuilder<'a> {
    fn build(self) -> AppCoreAppChainDescriptor {
        AppCoreAppChainDescriptor {
            id: self.id.to_string(),
            chain_name: self.name.to_string(),
            short_label: self.label.to_string(),
            native_symbol: self.native.to_string(),
            search_keywords: self.keywords.iter().map(|s| s.to_string()).collect(),
            supports_diagnostics: true,
            supports_endpoint_catalog: self.catalog,
            is_evm: self.is_evm,
        }
    }
}

fn evm(id: &str, name: &str, label: &str, native: &str, keywords: &[&str]) -> AppCoreAppChainDescriptor {
    DescBuilder {
        id,
        name,
        label,
        native,
        keywords,
        is_evm: true,
        catalog: true,
    }
    .build()
}

fn chain(id: &str, name: &str, label: &str, native: &str, keywords: &[&str]) -> AppCoreAppChainDescriptor {
    DescBuilder {
        id,
        name,
        label,
        native,
        keywords,
        is_evm: false,
        catalog: true,
    }
    .build()
}

fn chain_no_catalog(id: &str, name: &str, label: &str, native: &str, keywords: &[&str]) -> AppCoreAppChainDescriptor {
    DescBuilder {
        id,
        name,
        label,
        native,
        keywords,
        is_evm: false,
        catalog: false,
    }
    .build()
}

pub(super) fn app_chain_descriptors() -> Vec<AppCoreAppChainDescriptor> {
    vec![
        chain("bitcoin", "Bitcoin", "BTC", "BTC", &["Bitcoin", "BTC"]),
        chain("bitcoinCash", "Bitcoin Cash", "BCH", "BCH", &["Bitcoin Cash", "BCH"]),
        chain_no_catalog("bitcoinSV", "Bitcoin SV", "BSV", "BSV", &["Bitcoin SV", "BSV"]),
        chain("litecoin", "Litecoin", "LTC", "LTC", &["Litecoin", "LTC"]),
        chain("dogecoin", "Dogecoin", "DOGE", "DOGE", &["Dogecoin", "DOGE"]),
        evm("ethereum", "Ethereum", "ETH", "ETH", &["Ethereum", "ETH"]),
        evm("ethereumClassic", "Ethereum Classic", "ETC", "ETC", &["Ethereum Classic", "ETC"]),
        evm("arbitrum", "Arbitrum", "ARB", "ETH", &["Arbitrum", "ARB"]),
        evm("optimism", "Optimism", "OP", "ETH", &["Optimism", "OP"]),
        evm("bnb", "BNB Chain", "BNB", "BNB", &["BNB Chain", "BNB"]),
        evm("avalanche", "Avalanche", "AVAX", "AVAX", &["Avalanche", "AVAX"]),
        evm("hyperliquid", "Hyperliquid", "HYPE", "HYPE", &["Hyperliquid", "HYPE"]),
        chain("tron", "Tron", "TRX", "TRX", &["Tron", "TRX"]),
        chain("solana", "Solana", "SOL", "SOL", &["Solana", "SOL"]),
        chain("cardano", "Cardano", "ADA", "ADA", &["Cardano", "ADA"]),
        chain("xrp", "XRP Ledger", "XRP", "XRP", &["XRP Ledger", "XRP"]),
        chain("stellar", "Stellar", "XLM", "XLM", &["Stellar", "XLM"]),
        chain("monero", "Monero", "XMR", "XMR", &["Monero", "XMR"]),
        chain("sui", "Sui", "SUI", "SUI", &["Sui", "SUI"]),
        chain("aptos", "Aptos", "APT", "APT", &["Aptos", "APT"]),
        chain("ton", "TON", "TON", "TON", &["TON"]),
        chain("icp", "Internet Computer", "ICP", "ICP", &["Internet Computer", "ICP"]),
        chain("near", "NEAR", "NEAR", "NEAR", &["NEAR"]),
        chain("polkadot", "Polkadot", "DOT", "DOT", &["Polkadot", "DOT"]),
        evm("polygon", "Polygon", "POL", "POL", &["Polygon", "POL", "MATIC"]),
        evm("base", "Base", "BASE", "ETH", &["Base", "ETH"]),
        evm("linea", "Linea", "LINEA", "ETH", &["Linea"]),
        evm("scroll", "Scroll", "SCRL", "ETH", &["Scroll"]),
        evm("blast", "Blast", "BLAST", "ETH", &["Blast"]),
        evm("mantle", "Mantle", "MNT", "MNT", &["Mantle", "MNT"]),
        chain("zcash", "Zcash", "ZEC", "ZEC", &["Zcash", "ZEC"]),
        chain("bitcoinGold", "Bitcoin Gold", "BTG", "BTG", &["Bitcoin Gold", "BTG"]),
        chain("decred", "Decred", "DCR", "DCR", &["Decred", "DCR"]),
        chain("kaspa", "Kaspa", "KAS", "KAS", &["Kaspa", "KAS"]),
        chain("dash", "Dash", "DASH", "DASH", &["Dash", "DASH"]),
        evm("sei", "Sei", "SEI", "SEI", &["Sei", "SEI"]),
        evm("celo", "Celo", "CELO", "CELO", &["Celo", "CELO"]),
        evm("cronos", "Cronos", "CRO", "CRO", &["Cronos", "CRO"]),
        evm("opBNB", "opBNB", "opBNB", "BNB", &["opBNB", "BNB L2"]),
        evm("zkSyncEra", "zkSync Era", "zkSync", "ETH", &["zkSync Era", "zkSync"]),
        evm("sonic", "Sonic", "S", "S", &["Sonic", "S"]),
        evm("berachain", "Berachain", "BERA", "BERA", &["Berachain", "BERA"]),
        evm("unichain", "Unichain", "UNI L2", "ETH", &["Unichain"]),
        evm("ink", "Ink", "INK", "ETH", &["Ink"]),
        evm("xLayer", "X Layer", "X Layer", "OKB", &["X Layer", "OKB", "OKX"]),
        chain("bittensor", "Bittensor", "TAO", "TAO", &["Bittensor", "TAO", "subtensor"]),
    ]
}

// ── broadcast_provider_options ─────────────────────────────────────────────

pub(super) fn broadcast_provider_options(chain_name: &str) -> Vec<AppCoreBroadcastProviderOption> {
    let pairs: &[(&str, &str)] = match chain_name {
        "Bitcoin" => &[("esplora", "Esplora"), ("maestro-esplora", "Maestro Esplora")],
        "Bitcoin Cash" => &[("blockchair", "Blockchair"), ("actorforth", "ActorForth REST")],
        "Bitcoin SV" => &[("whatsonchain", "WhatsOnChain"), ("blockchair", "Blockchair")],
        "Litecoin" => &[("litecoinspace", "LitecoinSpace"), ("blockcypher", "BlockCypher")],
        "Dogecoin" => &[("blockcypher", "BlockCypher")],
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism" | "BNB Chain" | "Avalanche"
        | "Hyperliquid" | "Polygon" | "Base" | "Linea" | "Scroll" | "Blast" | "Mantle" => {
            &[("rpc", "RPC Broadcast")]
        }
        "Tron" => &[
            ("trongrid-io", "TronGrid"),
            ("trongrid-pro", "TronGrid Pro"),
            ("trongrid-network", "TronGrid Network"),
        ],
        "Solana" => &[
            ("solana-mainnet-beta", "Solana Mainnet RPC"),
            ("solana-ankr", "Ankr Solana RPC"),
        ],
        "Cardano" => &[
            ("koios", "Koios"),
            ("xray-koios", "Xray Koios"),
            ("happystaking-koios", "HappyStake Koios"),
        ],
        "XRP Ledger" => &[
            ("ripple-s1", "Ripple RPC S1"),
            ("ripple-s2", "Ripple RPC S2"),
            ("xrplcluster", "XRPL Cluster"),
        ],
        "Stellar" => &[
            ("stellar-horizon", "Stellar Horizon"),
            ("lobstr-horizon", "LOBSTR Horizon"),
        ],
        "Monero" => &[
            ("edge-lws-1", "Edge Monero LWS 1"),
            ("edge-lws-2", "Edge Monero LWS 2"),
            ("edge-lws-3", "Edge Monero LWS 3"),
        ],
        "Sui" => &[
            ("sui-mainnet", "Sui Mainnet"),
            ("sui-publicnode", "PublicNode Sui"),
            ("sui-blockvision", "BlockVision Sui"),
            ("sui-blockpi", "BlockPI Sui"),
            ("sui-suiscan", "SuiScan RPC"),
        ],
        "Aptos" => &[
            ("aptoslabs-api", "Aptos Labs API"),
            ("blastapi-aptos", "BlastAPI Aptos"),
            ("aptoslabs-mainnet", "Aptos Mainnet"),
        ],
        "TON" => &[("ton-api-v2", "TON API v2")],
        "Internet Computer" => &[("rosetta", "Rosetta")],
        "NEAR" => &[
            ("near-mainnet-rpc", "NEAR Mainnet RPC"),
            ("fastnear-rpc", "FastNEAR RPC"),
            ("lava-near-rpc", "Lava NEAR RPC"),
        ],
        "Polkadot" => &[("sidecar", "Sidecar")],
        "Zcash" => &[("trezor-blockbook", "Trezor Blockbook")],
        "Bitcoin Gold" => &[("trezor-blockbook", "Trezor Blockbook")],
        "Decred" => &[("dcrdata", "dcrdata Insight")],
        "Kaspa" => &[("kaspaorg", "api.kaspa.org")],
        "Dash" => &[("trezor-blockbook", "Trezor Blockbook")],
        "Bittensor" => &[("opentensor", "OpenTensor RPC")],
        "Sei" | "Celo" | "Cronos" | "opBNB" | "zkSync Era" | "Sonic" | "Berachain"
        | "Unichain" | "Ink" | "X Layer" => &[("rpc", "RPC Broadcast")],
        _ => &[],
    };
    pairs
        .iter()
        .map(|(id, title)| AppCoreBroadcastProviderOption {
            id: (*id).to_string(),
            title: (*title).to_string(),
        })
        .collect()
}

// ── Bitcoin URL groups ────────────────────────────────────────────────────

pub(super) fn bitcoin_esplora_base_urls(
    catalog: &AppCoreCatalog,
    network: &str,
) -> Result<Vec<String>, String> {
    let ids: &[&str] = match network {
        "mainnet" => &[
            "bitcoin.mainnet.blockstream",
            "bitcoin.mainnet.mempool",
            "bitcoin.mainnet.mempool_emzy",
            "bitcoin.mainnet.maestro",
        ],
        "testnet" => &["bitcoin.testnet.blockstream", "bitcoin.testnet.mempool"],
        "testnet4" => &["bitcoin.testnet4.mempool"],
        "signet" => &["bitcoin.signet.blockstream", "bitcoin.signet.mempool"],
        _ => return Err(format!("Unsupported Bitcoin network mode: {network}")),
    };
    endpoints_for_known_ids(catalog, ids)
}

pub(super) fn bitcoin_wallet_store_default_base_urls(
    catalog: &AppCoreCatalog,
    network: &str,
) -> Result<Vec<String>, String> {
    let ids: &[&str] = match network {
        "mainnet" => &[
            "bitcoin.mainnet.blockstream",
            "bitcoin.mainnet.mempool",
            "bitcoin.mainnet.maestro",
        ],
        "testnet" => &["bitcoin.testnet.blockstream", "bitcoin.testnet.mempool"],
        "testnet4" => &["bitcoin.testnet4.mempool"],
        "signet" => &["bitcoin.signet.mempool"],
        _ => return Err(format!("Unsupported Bitcoin network mode: {network}")),
    };
    endpoints_for_known_ids(catalog, ids)
}

fn endpoints_for_known_ids(catalog: &AppCoreCatalog, ids: &[&str]) -> Result<Vec<String>, String> {
    ids.iter()
        .map(|id| {
            catalog
                .endpoint_records
                .iter()
                .find(|r| r.id == *id)
                .map(|r| r.endpoint.clone())
                .ok_or_else(|| format!("Missing endpoint record for id: {id}"))
        })
        .collect()
}
