use crate::store::state::{CoreAppState, StateCommand};
use crate::store::wallet_db;
use crate::store::wallet_domain::CoreTokenHostingChain;

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
        chain_name: CoreTokenHostingChain::Ethereum.chain_name().to_string(),
        symbol: symbol.to_string(),
        name: symbol.to_string(),
        contract: "0x0000000000000000000000000000000000000001".to_string(),
        coingecko_id: symbol.to_lowercase(),
        decimals,
    }
}

#[test]
fn a_tracked_token_survives_a_reopen() {
    let db = tmp_db();
    let mut state = CoreAppState::default();
    crate::store::state::reduce_state_in_place(&mut state, add_custom("USDC", 6));
    wallet_db::app_state_save(&db, &state).expect("save");

    let reloaded = wallet_db::app_state_load(&db).expect("load");
    assert_eq!(reloaded.token_preferences.len(), 1, "known token was lost");
    assert_eq!(reloaded.token_preferences[0].token.symbol, "USDC");
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
        events.first().map(|e| e.kind.as_str()),
        Some("tokenPreferenceRejected")
    );
    assert_eq!(
        events.first().and_then(|e| e.subject_id.as_deref()),
        Some("tooManyDecimals")
    );
    assert!(
        state.token_preferences.is_empty(),
        "a refused token must not be stored at any precision"
    );

    crate::store::state::reduce_state_in_place(
        &mut state,
        add_custom("USDT", crate::store::state::MAX_TOKEN_DECIMALS as u32),
    );
    wallet_db::app_state_save(&db, &state).expect("save");
    let reloaded = wallet_db::app_state_load(&db).expect("load");
    assert_eq!(
        reloaded.token_preferences[0].token.decimals,
        crate::store::state::MAX_TOKEN_DECIMALS as u32
    );
}
