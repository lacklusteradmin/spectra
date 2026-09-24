use crate::service::WalletService;
use crate::store::state::StateCommand;

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

/// Every deployment references a registered chain; token-hosting projections
/// must resolve the registry identity rather than depend on display spelling.
#[test]
fn the_catalog_chain_ids_all_resolve() {
    for token in crate::tokens::catalog() {
        let chain = crate::registry::Chain::from_str_id(&token.chain_id).unwrap();
        if !token.is_native() {
            assert!(chain.hosts_tokens(), "{}", token.deployment_id);
        }
    }
}

/// A user's choices survive the merge; the build's additions arrive.
#[tokio::test]
async fn merging_keeps_what_the_user_chose() {
    let service = WalletService::new(Vec::new()).expect("service");
    let state = service
        .apply_state_command(StateCommand::MergeBuiltInTokens)
        .await
        .map(|transition| transition.state)
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
        chain_id: target.token.chain_id.clone(),
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
        .apply_state_command(StateCommand::MergeBuiltInTokens)
        .await
        .map(|transition| transition.state)
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
