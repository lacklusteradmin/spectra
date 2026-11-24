use crate::registry::Chain;
use crate::store::state::WalletState;
use crate::store::wallet_domain::AssetHolding;

/// A served wallet names each holding by its deployment and the profile its
/// path is on; its one address is where every network sharing its slot reads
/// it, and nowhere else.
#[test]
fn a_served_wallet_carries_its_ids_profile_and_one_address() {
    let mut wallet = WalletState::single_address(
        "w1",
        "Cold",
        Chain::Bitcoin,
        "bc1qexample",
        Some("m/84'/0'/2'/0/0".to_string()),
        false,
    );
    wallet.holdings = vec![AssetHolding {
        id: String::new(),
        name: "Bitcoin".to_string(),
        symbol: "BTC".to_string(),
        coingecko_id: "bitcoin".to_string(),
        chain_id: Chain::Bitcoin,
        token_standard: None,
        contract_address: None,
        amount: "1.5".into(),
    }];
    let served = wallet.served();
    assert_eq!(served.holdings[0].id, "bitcoin:native");
    assert_eq!(
        served.derivation_profile,
        Some(crate::derivation::path::DerivationProfileChoice {
            profile: crate::catalog::chains::DerivationProfile::NativeSegWit,
            account: 2,
        })
    );
    assert_eq!(served.address_on(Chain::Bitcoin), Some("bc1qexample"));
    assert_eq!(served.address_on(Chain::Litecoin), None);

    let evm = WalletState::single_address("w2", "Hot", Chain::Base, "0xabc", None, true);
    assert_eq!(evm.address_on(Chain::Ethereum), Some("0xabc"));
    assert_eq!(evm.address_on(Chain::Solana), None);
}
