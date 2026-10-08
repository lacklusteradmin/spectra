//! The parts of a network account core computes rather than reads.

use super::*;

/// XRP: one base reserve and an increment per owned object; what is above
/// both is spendable.
#[test]
fn an_xrp_reserve_is_its_base_and_an_increment_per_object() {
    // 25 XRP held, 1 XRP base, 3 objects at 0.2 XRP.
    assert_eq!(
        reserve_account(true, 25_000_000, 1_000_000, 3, 200_000, 6),
        NetworkAccount::Reserve {
            exists: true,
            balance: "25".into(),
            base_reserve: "1".into(),
            owned_objects: 3,
            object_reserve: "0.6".into(),
            total_reserve: "1.6".into(),
            spendable: "23.4".into(),
        }
    );
}

/// Stellar: two base reserves, then one per subentry and sponsoring, less
/// one per sponsored entry (CAP-33); an account below its reserve spends
/// nothing rather than a negative amount.
#[test]
fn a_stellar_reserve_counts_sponsorships() {
    let base = 5_000_000; // 0.5 XLM
    let objects = (2u64 + 1).saturating_sub(1); // two trust lines, sponsoring one, one sponsored
    let NetworkAccount::Reserve {
        base_reserve,
        total_reserve,
        spendable,
        ..
    } = reserve_account(true, 15_000_000, 2 * u128::from(base), objects, base, 7)
    else {
        unreachable!()
    };
    assert_eq!(base_reserve, "1");
    assert_eq!(total_reserve, "2");
    assert_eq!(spendable, "0");
}

/// An account the network has not created holds nothing and is still told
/// what creates it.
#[test]
fn a_missing_account_reports_its_base_reserve() {
    let NetworkAccount::Reserve {
        exists,
        base_reserve,
        spendable,
        ..
    } = reserve_account(false, 0, 1_000_000, 0, 200_000, 6)
    else {
        unreachable!()
    };
    assert!(!exists);
    assert_eq!(base_reserve, "1");
    assert_eq!(spendable, "0");
}

#[test]
fn ton_states_and_contracts_take_their_wallet_names() {
    assert_eq!(ton_account_state("active"), Some(TonAccountState::Active));
    for state in ["uninitialized", "uninit", "nonexist"] {
        assert_eq!(
            ton_account_state(state),
            Some(TonAccountState::Uninitialized)
        );
    }
    assert_eq!(ton_account_state("frozen"), Some(TonAccountState::Frozen));
    assert_eq!(ton_account_state("deleted"), None);
    assert_eq!(ton_contract_name("wallet v5 r1".into()), "W5");
    assert_eq!(ton_contract_name("wallet v4 r2".into()), "v4R2");
    assert_eq!(ton_contract_name("wallet v3 r2".into()), "wallet v3 r2");
}

#[test]
fn only_networks_that_keep_more_than_a_balance_have_an_account() {
    let wallet = |chain, address: &str| {
        crate::store::state::WalletState::single_address("w", "W", chain, address, None, false)
    };
    for chain in [
        Chain::Tron,
        Chain::Xrp,
        Chain::Stellar,
        Chain::Ton,
        Chain::Polkadot,
        Chain::Bittensor,
        Chain::Near,
    ] {
        assert!(has_network_account(&wallet(chain, "address")), "{chain}");
    }
    for chain in [Chain::Bitcoin, Chain::Ethereum, Chain::Solana, Chain::Sui] {
        assert!(!has_network_account(&wallet(chain, "address")), "{chain}");
    }
    // Cardano: a base address names its stake key; a raw key's enterprise
    // address names none.
    assert!(has_network_account(&wallet(
        Chain::Cardano,
        "addr1qy8ac7qqy0vtulyl7wntmsxc6wex80gvcyjy33qffrhm7sh927ysx5sftuw0dlft05dz3c7revpf7jx0xnlcjz3g69mq4afdhv"
    )));
    assert!(has_network_account(&wallet(
        Chain::CardanoPreprod,
        "addr_test1qq8ac7qqy0vtulyl7wntmsxc6wex80gvcyjy33qffrhm7sh927ysx5sftuw0dlft05dz3c7revpf7jx0xnlcjz3g69mqkt5dmn"
    )));
    assert!(!has_network_account(&wallet(
        Chain::Cardano,
        "addr1vy8ac7qqy0vtulyl7wntmsxc6wex80gvcyjy33qffrhm7ss7lxrqp"
    )));
}
