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

/// The catalog is the catalog: a merge brings every built-in and keeps the
/// tokens the user added.
#[tokio::test]
async fn merging_keeps_what_the_user_added() {
    let service = WalletService::new(Vec::new()).expect("service");
    service
        .apply_state_command(StateCommand::AddCustomToken {
            chain_id: crate::registry::Chain::Base,
            symbol: "MOON".into(),
            name: "Moon".into(),
            contract: format!("0x{}", "42".repeat(20)),
            coingecko_id: String::new(),
            coinpaprika_id: String::new(),
            decimals: 18,
        })
        .await
        .expect("add");
    let after = service
        .apply_state_command(StateCommand::MergeBuiltInTokens)
        .await
        .map(|transition| transition.state)
        .expect("merge");
    let built_ins = crate::store::built_in_token_preferences();
    assert_eq!(
        after
            .token_preferences
            .iter()
            .filter(|e| e.is_built_in)
            .count(),
        built_ins.len()
    );
    assert!(
        after
            .token_preferences
            .iter()
            .any(|e| !e.is_built_in && e.token.symbol == "MOON"),
        "the merge dropped a token the user added"
    );
}

/// A built-in row is not the user's to remove.
#[test]
fn a_built_in_cannot_be_removed() {
    use crate::store::state::{
        CoreAppState, StateEvent, TokenPreferenceRejection, reduce_state_in_place,
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
}
