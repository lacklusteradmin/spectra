use crate::service::WalletService;
use crate::store::state::StateCommand;
use crate::store::wallet_domain::CoreTokenHostingChain;

#[test]
fn every_built_in_has_a_unique_id() {
    let entries = crate::store::built_in_token_preferences();
    assert!(!entries.is_empty(), "the catalog produced nothing");
    let mut ids: Vec<String> = entries.iter().map(|e| e.id()).collect();
    ids.sort_unstable();
    let count = ids.len();
    ids.dedup();
    assert_eq!(count, ids.len(), "two built-ins share an id");
    assert!(
        entries.iter().all(|e| e.is_built_in),
        "a catalog entry is not marked built-in"
    );
}

/// The chain mapping used to exist four times. `tokens.toml` spells BNB
/// Chain `"bnb"`, which is the case a strict name match would drop.
#[test]
fn the_catalog_chain_names_all_resolve() {
    for token in crate::tokens::catalog() {
        if token.chain.eq_ignore_ascii_case("bnb") {
            assert_eq!(
                CoreTokenHostingChain::from_chain_name(&token.chain),
                Some(CoreTokenHostingChain::Bnb)
            );
        }
    }
    for chain in CoreTokenHostingChain::ALL {
        assert_eq!(
            CoreTokenHostingChain::from_chain_name(chain.chain_name()),
            Some(*chain),
            "{} does not round-trip",
            chain.chain_name()
        );
    }
}

/// A user's choices survive the merge; the build's additions arrive.
#[tokio::test]
async fn merging_keeps_what_the_user_chose() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let state = service
        .merge_built_in_token_preferences()
        .await
        .expect("merge");
    assert!(!state.token_preferences.is_empty());

    // Turn one off, then merge again.
    let target = state
        .token_preferences
        .iter()
        .find(|e| e.is_built_in && e.is_enabled)
        .expect("an enabled built-in");
    let id = target.id().clone();
    let key = crate::store::state::CoreTokenPreferenceKey {
        chain_name: target.token.chain.clone(),
        contract: target.token.contract.clone(),
    };
    service
        .apply_state_command(StateCommand::SetTokenPreferencesEnabled {
            tokens: vec![key],
            is_enabled: false,
        })
        .await
        .expect("store");

    let after = service
        .merge_built_in_token_preferences()
        .await
        .expect("merge again");
    let kept = after
        .token_preferences
        .iter()
        .find(|e| e.id() == id)
        .expect("the entry survived");
    assert!(
        !kept.is_enabled,
        "the merge re-enabled a token the user turned off"
    );
}
