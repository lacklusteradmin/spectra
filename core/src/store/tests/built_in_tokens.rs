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
        let chain = token.chain_id;
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
        chain_id: target.token.chain_id,
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

/// A built-in row is not the user's to remove, and toggling one deployment
/// toggles the token on every network it is deployed to.
#[test]
fn built_ins_toggle_as_one_token_and_cannot_be_removed() {
    use crate::store::state::{
        CoreAppState, CoreTokenPreferenceKey, StateEvent, TokenPreferenceRejection,
        reduce_state_in_place,
    };
    let mut state = CoreAppState::default();
    reduce_state_in_place(&mut state, StateCommand::MergeBuiltInTokens);
    let usdc = state
        .token_preferences
        .iter()
        .find(|e| e.token.symbol == "USDC" && e.token.chain_id == crate::registry::Chain::Ethereum)
        .expect("USDC is built in")
        .token
        .clone();
    let count = state.token_preferences.len();

    let events = reduce_state_in_place(
        &mut state,
        StateCommand::RemoveCustomToken {
            chain_id: usdc.chain_id,
            contract: usdc.contract.clone(),
        },
    );
    assert_eq!(
        events.first(),
        Some(&StateEvent::TokenPreferenceRejected {
            reason: TokenPreferenceRejection::BuiltInToken
        })
    );
    assert_eq!(state.token_preferences.len(), count);

    reduce_state_in_place(
        &mut state,
        StateCommand::SetTokenPreferencesEnabled {
            tokens: vec![CoreTokenPreferenceKey {
                chain_id: usdc.chain_id,
                contract: usdc.contract,
            }],
            is_enabled: false,
        },
    );
    assert_eq!(
        state.token_preferences.len(),
        count,
        "untracking is not deleting"
    );
    let deployments: Vec<_> = state
        .token_preferences
        .iter()
        .filter(|e| e.token.token_id == usdc.token_id)
        .collect();
    assert!(deployments.len() > 2);
    assert!(deployments.iter().all(|e| !e.is_enabled));
}
