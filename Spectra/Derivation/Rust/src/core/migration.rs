use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::state::{AppSettings, AssetHolding, CoreAppState, WalletAddress, WalletSummary};

const LEGACY_WALLET_STORE_VERSION: i32 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyPersistedCoin {
    pub name: String,
    pub symbol: String,
    pub market_data_id: String,
    pub coin_gecko_id: String,
    pub chain_name: String,
    pub token_standard: String,
    pub contract_address: Option<String>,
    pub amount: f64,
    pub price_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacySeedDerivationPaths {
    pub is_custom_enabled: bool,
    pub bitcoin: String,
    pub bitcoin_cash: String,
    pub bitcoin_sv: String,
    pub litecoin: String,
    pub dogecoin: String,
    pub ethereum: String,
    pub ethereum_classic: String,
    pub arbitrum: String,
    pub optimism: String,
    pub avalanche: String,
    pub hyperliquid: String,
    pub tron: String,
    pub solana: String,
    pub stellar: String,
    pub xrp: String,
    pub cardano: String,
    pub sui: String,
    pub aptos: String,
    pub ton: String,
    pub internet_computer: String,
    pub near: String,
    pub polkadot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyPersistedWallet {
    pub id: String,
    pub name: String,
    pub bitcoin_network_mode: String,
    pub dogecoin_network_mode: String,
    pub bitcoin_address: Option<String>,
    pub bitcoin_x_pub: Option<String>,
    pub bitcoin_cash_address: Option<String>,
    pub bitcoin_sv_address: Option<String>,
    pub litecoin_address: Option<String>,
    pub dogecoin_address: Option<String>,
    pub ethereum_address: Option<String>,
    pub tron_address: Option<String>,
    pub solana_address: Option<String>,
    pub stellar_address: Option<String>,
    pub xrp_address: Option<String>,
    pub monero_address: Option<String>,
    pub cardano_address: Option<String>,
    pub sui_address: Option<String>,
    pub aptos_address: Option<String>,
    pub ton_address: Option<String>,
    pub icp_address: Option<String>,
    pub near_address: Option<String>,
    pub polkadot_address: Option<String>,
    pub seed_derivation_preset: String,
    pub seed_derivation_paths: LegacySeedDerivationPaths,
    pub selected_chain: String,
    pub holdings: Vec<LegacyPersistedCoin>,
    pub include_in_portfolio_total: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyPersistedWalletStore {
    pub version: i32,
    pub wallets: Vec<LegacyPersistedWallet>,
}

pub fn legacy_wallet_store_to_core_state(json: &str) -> Result<CoreAppState, String> {
    let store = serde_json::from_str::<LegacyPersistedWalletStore>(json).map_err(display_error)?;
    let selected_wallet_id = store.wallets.first().map(|wallet| wallet.id.clone());
    Ok(CoreAppState {
        schema_version: 1,
        wallets: store
            .wallets
            .into_iter()
            .map(core_wallet_from_legacy)
            .collect(),
        selected_wallet_id,
        settings: AppSettings::default(),
    })
}

pub fn core_state_to_legacy_wallet_store_json(state: &CoreAppState) -> Result<String, String> {
    let store = LegacyPersistedWalletStore {
        version: LEGACY_WALLET_STORE_VERSION,
        wallets: state.wallets.iter().map(legacy_wallet_from_core).collect(),
    };
    serde_json::to_string(&store).map_err(display_error)
}

fn core_wallet_from_legacy(wallet: LegacyPersistedWallet) -> WalletSummary {
    let addresses = addresses_from_legacy_wallet(&wallet);
    let derivation_paths = BTreeMap::from([
        ("Bitcoin".to_string(), wallet.seed_derivation_paths.bitcoin),
        (
            "Bitcoin Cash".to_string(),
            wallet.seed_derivation_paths.bitcoin_cash,
        ),
        (
            "Bitcoin SV".to_string(),
            wallet.seed_derivation_paths.bitcoin_sv,
        ),
        (
            "Litecoin".to_string(),
            wallet.seed_derivation_paths.litecoin,
        ),
        (
            "Dogecoin".to_string(),
            wallet.seed_derivation_paths.dogecoin,
        ),
        (
            "Ethereum".to_string(),
            wallet.seed_derivation_paths.ethereum,
        ),
        (
            "Ethereum Classic".to_string(),
            wallet.seed_derivation_paths.ethereum_classic,
        ),
        (
            "Arbitrum".to_string(),
            wallet.seed_derivation_paths.arbitrum,
        ),
        (
            "Optimism".to_string(),
            wallet.seed_derivation_paths.optimism,
        ),
        (
            "Avalanche".to_string(),
            wallet.seed_derivation_paths.avalanche,
        ),
        (
            "Hyperliquid".to_string(),
            wallet.seed_derivation_paths.hyperliquid,
        ),
        ("Tron".to_string(), wallet.seed_derivation_paths.tron),
        ("Solana".to_string(), wallet.seed_derivation_paths.solana),
        ("Stellar".to_string(), wallet.seed_derivation_paths.stellar),
        ("XRP Ledger".to_string(), wallet.seed_derivation_paths.xrp),
        ("Cardano".to_string(), wallet.seed_derivation_paths.cardano),
        ("Sui".to_string(), wallet.seed_derivation_paths.sui),
        ("Aptos".to_string(), wallet.seed_derivation_paths.aptos),
        ("TON".to_string(), wallet.seed_derivation_paths.ton),
        (
            "Internet Computer".to_string(),
            wallet.seed_derivation_paths.internet_computer,
        ),
        ("NEAR".to_string(), wallet.seed_derivation_paths.near),
        (
            "Polkadot".to_string(),
            wallet.seed_derivation_paths.polkadot,
        ),
    ]);

    WalletSummary {
        id: wallet.id,
        name: wallet.name,
        is_watch_only: false,
        selected_chain: Some(wallet.selected_chain),
        include_in_portfolio_total: wallet.include_in_portfolio_total,
        bitcoin_network_mode: wallet.bitcoin_network_mode,
        dogecoin_network_mode: wallet.dogecoin_network_mode,
        bitcoin_xpub: wallet.bitcoin_x_pub.clone(),
        derivation_preset: wallet.seed_derivation_preset,
        derivation_paths,
        holdings: wallet
            .holdings
            .into_iter()
            .map(|holding| AssetHolding {
                name: holding.name,
                symbol: holding.symbol,
                market_data_id: holding.market_data_id,
                coin_gecko_id: holding.coin_gecko_id,
                chain_name: holding.chain_name,
                token_standard: holding.token_standard,
                contract_address: holding.contract_address,
                amount: holding.amount,
                price_usd: holding.price_usd,
            })
            .collect(),
        addresses,
    }
}

fn legacy_wallet_from_core(wallet: &WalletSummary) -> LegacyPersistedWallet {
    let derivation_paths = LegacySeedDerivationPaths {
        is_custom_enabled: true,
        bitcoin: wallet_path(wallet, "Bitcoin"),
        bitcoin_cash: wallet_path(wallet, "Bitcoin Cash"),
        bitcoin_sv: wallet_path(wallet, "Bitcoin SV"),
        litecoin: wallet_path(wallet, "Litecoin"),
        dogecoin: wallet_path(wallet, "Dogecoin"),
        ethereum: wallet_path(wallet, "Ethereum"),
        ethereum_classic: wallet_path(wallet, "Ethereum Classic"),
        arbitrum: wallet_path(wallet, "Arbitrum"),
        optimism: wallet_path(wallet, "Optimism"),
        avalanche: wallet_path(wallet, "Avalanche"),
        hyperliquid: wallet_path(wallet, "Hyperliquid"),
        tron: wallet_path(wallet, "Tron"),
        solana: wallet_path(wallet, "Solana"),
        stellar: wallet_path(wallet, "Stellar"),
        xrp: wallet_path(wallet, "XRP Ledger"),
        cardano: wallet_path(wallet, "Cardano"),
        sui: wallet_path(wallet, "Sui"),
        aptos: wallet_path(wallet, "Aptos"),
        ton: wallet_path(wallet, "TON"),
        internet_computer: wallet_path(wallet, "Internet Computer"),
        near: wallet_path(wallet, "NEAR"),
        polkadot: wallet_path(wallet, "Polkadot"),
    };

    LegacyPersistedWallet {
        id: wallet.id.clone(),
        name: wallet.name.clone(),
        bitcoin_network_mode: wallet.bitcoin_network_mode.clone(),
        dogecoin_network_mode: wallet.dogecoin_network_mode.clone(),
        bitcoin_address: wallet_address(wallet, "Bitcoin", "address"),
        bitcoin_x_pub: wallet
            .bitcoin_xpub
            .clone()
            .or_else(|| wallet_address(wallet, "Bitcoin", "xpub")),
        bitcoin_cash_address: wallet_address(wallet, "Bitcoin Cash", "address"),
        bitcoin_sv_address: wallet_address(wallet, "Bitcoin SV", "address"),
        litecoin_address: wallet_address(wallet, "Litecoin", "address"),
        dogecoin_address: wallet_address(wallet, "Dogecoin", "address"),
        ethereum_address: wallet_address(wallet, "Ethereum", "address"),
        tron_address: wallet_address(wallet, "Tron", "address"),
        solana_address: wallet_address(wallet, "Solana", "address"),
        stellar_address: wallet_address(wallet, "Stellar", "address"),
        xrp_address: wallet_address(wallet, "XRP Ledger", "address"),
        monero_address: wallet_address(wallet, "Monero", "address"),
        cardano_address: wallet_address(wallet, "Cardano", "address"),
        sui_address: wallet_address(wallet, "Sui", "address"),
        aptos_address: wallet_address(wallet, "Aptos", "address"),
        ton_address: wallet_address(wallet, "TON", "address"),
        icp_address: wallet_address(wallet, "Internet Computer", "address"),
        near_address: wallet_address(wallet, "NEAR", "address"),
        polkadot_address: wallet_address(wallet, "Polkadot", "address"),
        seed_derivation_preset: wallet.derivation_preset.clone(),
        seed_derivation_paths: derivation_paths,
        selected_chain: wallet
            .selected_chain
            .clone()
            .unwrap_or_else(|| "Bitcoin".to_string()),
        holdings: wallet
            .holdings
            .iter()
            .map(|holding| LegacyPersistedCoin {
                name: holding.name.clone(),
                symbol: holding.symbol.clone(),
                market_data_id: holding.market_data_id.clone(),
                coin_gecko_id: holding.coin_gecko_id.clone(),
                chain_name: holding.chain_name.clone(),
                token_standard: holding.token_standard.clone(),
                contract_address: holding.contract_address.clone(),
                amount: holding.amount,
                price_usd: holding.price_usd,
            })
            .collect(),
        include_in_portfolio_total: wallet.include_in_portfolio_total,
    }
}

fn addresses_from_legacy_wallet(wallet: &LegacyPersistedWallet) -> Vec<WalletAddress> {
    let mut addresses = Vec::new();

    push_legacy_address(
        &mut addresses,
        "Bitcoin",
        "address",
        wallet.bitcoin_address.clone(),
        Some(wallet.seed_derivation_paths.bitcoin.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Bitcoin",
        "xpub",
        wallet.bitcoin_x_pub.clone(),
        None,
    );
    push_legacy_address(
        &mut addresses,
        "Bitcoin Cash",
        "address",
        wallet.bitcoin_cash_address.clone(),
        Some(wallet.seed_derivation_paths.bitcoin_cash.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Bitcoin SV",
        "address",
        wallet.bitcoin_sv_address.clone(),
        Some(wallet.seed_derivation_paths.bitcoin_sv.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Litecoin",
        "address",
        wallet.litecoin_address.clone(),
        Some(wallet.seed_derivation_paths.litecoin.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Dogecoin",
        "address",
        wallet.dogecoin_address.clone(),
        Some(wallet.seed_derivation_paths.dogecoin.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Ethereum",
        "address",
        wallet.ethereum_address.clone(),
        Some(wallet.seed_derivation_paths.ethereum.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Tron",
        "address",
        wallet.tron_address.clone(),
        Some(wallet.seed_derivation_paths.tron.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Solana",
        "address",
        wallet.solana_address.clone(),
        Some(wallet.seed_derivation_paths.solana.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Stellar",
        "address",
        wallet.stellar_address.clone(),
        Some(wallet.seed_derivation_paths.stellar.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "XRP Ledger",
        "address",
        wallet.xrp_address.clone(),
        Some(wallet.seed_derivation_paths.xrp.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Monero",
        "address",
        wallet.monero_address.clone(),
        None,
    );
    push_legacy_address(
        &mut addresses,
        "Cardano",
        "address",
        wallet.cardano_address.clone(),
        Some(wallet.seed_derivation_paths.cardano.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Sui",
        "address",
        wallet.sui_address.clone(),
        Some(wallet.seed_derivation_paths.sui.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Aptos",
        "address",
        wallet.aptos_address.clone(),
        Some(wallet.seed_derivation_paths.aptos.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "TON",
        "address",
        wallet.ton_address.clone(),
        Some(wallet.seed_derivation_paths.ton.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Internet Computer",
        "address",
        wallet.icp_address.clone(),
        Some(wallet.seed_derivation_paths.internet_computer.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "NEAR",
        "address",
        wallet.near_address.clone(),
        Some(wallet.seed_derivation_paths.near.clone()),
    );
    push_legacy_address(
        &mut addresses,
        "Polkadot",
        "address",
        wallet.polkadot_address.clone(),
        Some(wallet.seed_derivation_paths.polkadot.clone()),
    );

    addresses
}

fn push_legacy_address(
    addresses: &mut Vec<WalletAddress>,
    chain_name: &str,
    kind: &str,
    address: Option<String>,
    derivation_path: Option<String>,
) {
    let Some(address) = address.map(|value| value.trim().to_string()) else {
        return;
    };
    if address.is_empty() {
        return;
    }
    addresses.push(WalletAddress {
        chain_name: chain_name.to_string(),
        address,
        kind: kind.to_string(),
        derivation_path,
    });
}

fn wallet_path(wallet: &WalletSummary, chain_name: &str) -> String {
    wallet
        .derivation_paths
        .get(chain_name)
        .cloned()
        .unwrap_or_default()
}

fn wallet_address(wallet: &WalletSummary, chain_name: &str, kind: &str) -> Option<String> {
    wallet
        .addresses
        .iter()
        .find(|address| address.chain_name == chain_name && address.kind == kind)
        .map(|address| address.address.clone())
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_legacy_wallet_store() {
        let legacy = LegacyPersistedWalletStore {
            version: 5,
            wallets: vec![LegacyPersistedWallet {
                id: "wallet-1".to_string(),
                name: "Main".to_string(),
                bitcoin_network_mode: "mainnet".to_string(),
                dogecoin_network_mode: "mainnet".to_string(),
                bitcoin_address: Some("bc1qexample".to_string()),
                bitcoin_x_pub: Some("xpubexample".to_string()),
                bitcoin_cash_address: Some("bitcoincash:qexample".to_string()),
                bitcoin_sv_address: None,
                litecoin_address: None,
                dogecoin_address: None,
                ethereum_address: Some("0x1234".to_string()),
                tron_address: None,
                solana_address: None,
                stellar_address: None,
                xrp_address: None,
                monero_address: None,
                cardano_address: None,
                sui_address: None,
                aptos_address: None,
                ton_address: None,
                icp_address: None,
                near_address: None,
                polkadot_address: None,
                seed_derivation_preset: "standard".to_string(),
                seed_derivation_paths: LegacySeedDerivationPaths {
                    is_custom_enabled: false,
                    bitcoin: "m/84'/0'/0'/0/0".to_string(),
                    bitcoin_cash: "m/44'/145'/0'/0/0".to_string(),
                    bitcoin_sv: String::new(),
                    litecoin: String::new(),
                    dogecoin: String::new(),
                    ethereum: "m/44'/60'/0'/0/0".to_string(),
                    ethereum_classic: String::new(),
                    arbitrum: String::new(),
                    optimism: String::new(),
                    avalanche: String::new(),
                    hyperliquid: String::new(),
                    tron: String::new(),
                    solana: String::new(),
                    stellar: String::new(),
                    xrp: String::new(),
                    cardano: String::new(),
                    sui: String::new(),
                    aptos: String::new(),
                    ton: String::new(),
                    internet_computer: String::new(),
                    near: String::new(),
                    polkadot: String::new(),
                },
                selected_chain: "Bitcoin".to_string(),
                holdings: vec![LegacyPersistedCoin {
                    name: "Bitcoin".to_string(),
                    symbol: "BTC".to_string(),
                    market_data_id: "bitcoin".to_string(),
                    coin_gecko_id: "bitcoin".to_string(),
                    chain_name: "Bitcoin".to_string(),
                    token_standard: "native".to_string(),
                    contract_address: None,
                    amount: 1.25,
                    price_usd: 65000.0,
                }],
                include_in_portfolio_total: true,
            }],
        };

        let json = serde_json::to_string(&legacy).expect("legacy json");
        let state = legacy_wallet_store_to_core_state(&json).expect("core state");
        let roundtrip = core_state_to_legacy_wallet_store_json(&state).expect("roundtrip");
        let decoded =
            serde_json::from_str::<LegacyPersistedWalletStore>(&roundtrip).expect("decoded");

        assert_eq!(decoded.version, 5);
        assert_eq!(
            decoded.wallets[0].bitcoin_x_pub.as_deref(),
            Some("xpubexample")
        );
        assert_eq!(decoded.wallets[0].holdings[0].symbol, "BTC");
    }
}
