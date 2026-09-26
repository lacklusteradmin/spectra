use crate::store::state::{CoreAppState, StateCommand};
use crate::wallet_db;

/// One database per test. Keyed by thread id as well as pid: two tests in
/// the same process share a pid, and the first version of this helper did
/// not, so the second test read the first one's tokens and "passed" on
/// data it never wrote.
fn tmp_db() -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "spectra-tokens-{}-{:?}.sqlite",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

fn add_custom(symbol: &str, decimals: u32) -> StateCommand {
    StateCommand::AddCustomToken {
        chain_id: crate::registry::Chain::Ethereum.str_id().to_string(),
        symbol: symbol.to_string(),
        name: symbol.to_string(),
        contract: "0x0000000000000000000000000000000000000001".to_string(),
        coingecko_id: symbol.to_lowercase(),
        coinpaprika_id: String::new(),
        decimals,
    }
}

/// A precision no token has is refused, where it used to be clamped.
///
/// A clamp stores a number the user did not type and reads every later
/// balance at that scale; nothing downstream can tell it from a real one.
/// The round trip is checked on the entry that *was* accepted, so this
/// still covers what the clamp test covered.
#[test]
fn an_impossible_precision_is_refused_rather_than_clamped() {
    let db = tmp_db();
    let mut state = CoreAppState::default();
    let events = crate::store::state::reduce_state_in_place(&mut state, add_custom("USDT", 99));
    assert_eq!(
        events.first(),
        Some(&crate::store::state::StateEvent::TokenPreferenceRejected {
            reason: crate::store::state::TokenPreferenceRejection::TooManyDecimals
        })
    );
    assert!(
        state.token_preferences.is_empty(),
        "a refused token must not be stored at any precision"
    );

    crate::store::state::reduce_state_in_place(
        &mut state,
        add_custom("USDT", crate::store::state::MAX_TOKEN_DECIMALS as u32),
    );
    assert_eq!(state.token_preferences.len(), 1);
    assert_eq!(
        state.token_preferences[0].token.decimals,
        crate::store::state::MAX_TOKEN_DECIMALS as u32
    );
    wallet_db::app_state_save(&crate::wallet_db::WalletDatabase::new(&db), &state).expect("save");
    let reloaded =
        wallet_db::app_state_load(&crate::wallet_db::WalletDatabase::new(&db)).expect("load");
    assert_eq!(reloaded.token_preferences, state.token_preferences);
}

#[test]
fn new_network_deployments_inherit_the_tokens_saved_choice() {
    use crate::store::state::{CoreTokenPreferenceKey, reduce_state_in_place};
    let mut state = CoreAppState::default();
    reduce_state_in_place(&mut state, StateCommand::MergeBuiltInTokens);
    let token = state
        .token_preferences
        .iter()
        .find(|e| e.token.symbol == "USDC" && e.token.chain_id == "ethereum")
        .unwrap()
        .token
        .clone();
    reduce_state_in_place(
        &mut state,
        StateCommand::SetTokenPreferencesEnabled {
            tokens: vec![CoreTokenPreferenceKey {
                chain_id: "ethereum".into(),
                contract: token.contract,
            }],
            is_enabled: false,
        },
    );
    // A later catalog introduces deployments absent from the saved preferences.
    state
        .token_preferences
        .retain(|e| e.token.token_id != token.token_id || e.token.chain_id == "ethereum");
    reduce_state_in_place(&mut state, StateCommand::MergeBuiltInTokens);
    let deployments: Vec<_> = state
        .token_preferences
        .iter()
        .filter(|e| e.token.token_id == token.token_id)
        .collect();
    assert!(deployments.len() > 2);
    assert!(deployments.iter().all(|e| !e.is_enabled));
}
