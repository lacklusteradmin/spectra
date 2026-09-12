use crate::store::{
    state::{StateCommand, WalletSummary},
    wallet_domain::AssetHolding,
};
use crate::{registry::Chain, service::WalletService};

fn native(chain: Chain) -> AssetHolding {
    AssetHolding {
        name: chain.coin_name().into(),
        symbol: chain.coin_symbol().into(),
        chain_name: chain.chain_display_name().into(),
        token_standard: "Native".into(),
        coin_gecko_id: chain.coin_gecko_id().into(),
        contract_address: None,
        amount: 0.0,
        price_usd: 0.0,
    }
}

#[test]
fn native_and_protocol_mnt_share_a_token_but_not_a_deployment() {
    let native = crate::tokens::deployment("mantle:native").unwrap();
    let contract = crate::tokens::catalog()
        .iter()
        .find(|t| t.symbol == "MNT" && t.chain == "ethereum")
        .unwrap();
    assert!(native.is_native());
    assert!(!contract.is_native());
    assert_eq!(native.token_id, contract.token_id);
    assert_ne!(native.id, contract.id);
    assert_eq!(Chain::Arbitrum.coin_symbol(), "ETH");
    assert!(crate::registry::core_resolve_chain_id("ETH".into()).is_none());
}

#[test]
fn symbols_and_market_ids_never_identify_a_holding() {
    let eth = native(Chain::Ethereum);
    let base = native(Chain::Base);
    let mut lookalike = eth.clone();
    lookalike.token_standard = "ERC-20".into();
    lookalike.contract_address = Some("0x1111111111111111111111111111111111111111".into());
    assert!(!lookalike.is_native());
    assert_ne!(lookalike.deployment_key(), eth.deployment_key());
    assert_ne!(lookalike.token_identity(), eth.token_identity());
    assert_eq!(eth.token_identity(), base.token_identity());
    assert_ne!(eth.deployment_key(), base.deployment_key());
    lookalike.canonicalize().unwrap();
    assert!(lookalike.coin_gecko_id.is_empty());
}

#[test]
fn every_testnet_has_an_independent_unpriced_native_token() {
    for network in Chain::testnets() {
        let mut holding = native(network);
        holding.coin_gecko_id = network.mainnet_counterpart().coin_gecko_id().into();
        holding.price_usd = 100.0;
        holding.canonicalize().unwrap();
        assert!(holding.coin_gecko_id.is_empty());
        assert_eq!(holding.price_usd, 0.0);
        assert_ne!(
            holding.token_identity(),
            native(network.mainnet_counterpart()).token_identity()
        );
    }
}

#[tokio::test]
async fn invalid_protocol_identity_is_refused_before_storage() {
    let service = WalletService::new_typed(vec![]).unwrap();
    let mut wallet = WalletSummary::single_address(
        "w",
        "W",
        "Ethereum",
        "0x1111111111111111111111111111111111111111",
        None,
        true,
    );
    let mut holding = native(Chain::Ethereum);
    holding.token_standard = "ERC-20".into();
    wallet.holdings.push(holding);
    assert!(service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: wallet.clone()
        })
        .await
        .is_err());
    assert!(service.app_state().await.wallets.is_empty());
    wallet.holdings[0].contract_address = Some("not-an-address".into());
    assert!(service
        .apply_state_command(StateCommand::UpsertWallet { wallet })
        .await
        .is_err());
    assert!(service.app_state().await.wallets.is_empty());
}

#[test]
fn a_contract_called_eth_is_encoded_as_erc20() {
    use crate::send::ethereum::*;
    let contract = "0x1111111111111111111111111111111111111111";
    let assembly = prepare_evm_send_assembly(EvmSendAssemblyInput {
        chain_name: "Ethereum".into(),
        symbol: "ETH".into(),
        from_address: "0x2222222222222222222222222222222222222222".into(),
        resolved_destination: "0x3333333333333333333333333333333333333333".into(),
        amount: "1".into(),
        token: Some(EvmSupportedToken {
            symbol: "ETH".into(),
            contract_address: contract.into(),
            decimals: 6,
        }),
    })
    .unwrap();
    assert!(!assembly.is_native);
    assert_eq!(assembly.to_address, contract);
    assert_eq!(assembly.value_wei, "0");
    assert!(assembly.data_hex.starts_with("0xa9059cbb"));
}

#[test]
fn display_precision_is_deployment_specific() {
    use crate::tokens::token_display_decimals;
    assert_eq!(
        token_display_decimals(Some("ethereum:native".into()), Some(6)),
        18
    );
    let usdc = crate::tokens::catalog()
        .iter()
        .find(|t| t.chain == "ethereum" && t.token_id == "usd-coin")
        .unwrap();
    assert_eq!(token_display_decimals(Some(usdc.id.clone()), None), 6);
    assert_eq!(
        token_display_decimals(Some("custom:unlisted".into()), Some(2)),
        2
    );
    assert_eq!(token_display_decimals(None, None), 18);
}
