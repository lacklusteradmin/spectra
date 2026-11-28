//! The state reducer: what each command changes, refuses and announces.
use super::*;

/// A state with no tokens at all, so a test of the user's tokens sees
/// only the ones it adds.
fn no_tokens() -> ResidentState {
    ResidentState {
        token_preferences: Vec::new(),
        ..ResidentState::default()
    }
}

/// A command applied to `state`: the whole state after it, and its events.
struct Reduced {
    state: ResidentState,
    events: Vec<StateEvent>,
}

fn reduce_state(mut state: ResidentState, command: StateCommand) -> Reduced {
    let events = reduce_state_in_place(&mut state, command);
    Reduced { state, events }
}

fn test_wallet(id: &str, chain: crate::registry::Chain) -> WalletState {
    WalletState {
        id: id.to_string(),
        name: "Main".to_string(),
        signing: crate::store::state::WalletSigning::SeedPhrase {
            password_protected: false,
        },
        chain_id: chain,
        include_in_portfolio_total: true,
        xpub: None,
        derivation_profile: None,
        derivation_flags: Default::default(),
        derivation_path: Some("m/84'/0'/0'/0/0".to_string()),
        holdings: Vec::new(),
        address: Some("bc1qexample".to_string()),
        restore_height: None,
        hidden_holdings: Vec::new(),
        icp_principal: None,
        near_account_key: None,
        starknet_public_key: None,
        multisig_policy: None,
        balances_read_at: None,
    }
}

fn add_token(
    chain: crate::registry::Chain,
    symbol: &str,
    contract: &str,
    decimals: u32,
) -> StateCommand {
    StateCommand::AddCustomToken {
        standard: None,
        chain_id: chain,
        symbol: symbol.to_string(),
        name: "A Token".to_string(),
        contract: contract.to_string(),
        coingecko_id: String::new(),
        coinpaprika_id: String::new(),
        decimals: Some(decimals),
    }
}

/// A token's places are its protocol's where the protocol fixes them,
/// and otherwise must be given: none are guessed.
#[test]
fn decimals_are_the_protocols_or_given() {
    let mut untyped = add_token(crate::registry::Chain::Base, "MOON", EVM_CONTRACT, 18);
    if let StateCommand::AddCustomToken { decimals, .. } = &mut untyped {
        *decimals = None;
    }
    assert_eq!(
        rejection(&reduce_state(no_tokens(), untyped.clone())),
        Some(TokenPreferenceRejection::DecimalsRequired)
    );
    let added = reduce_state(
        no_tokens(),
        add_token(crate::registry::Chain::Base, "MOON", EVM_CONTRACT, 18),
    );
    let mut edit = StateCommand::UpdateCustomToken {
        chain_id: crate::registry::Chain::Base,
        contract: EVM_CONTRACT.into(),
        symbol: "MOON".into(),
        name: "Moon".into(),
        coingecko_id: String::new(),
        coinpaprika_id: String::new(),
        decimals: None,
    };
    assert_eq!(
        rejection(&reduce_state(added.state.clone(), edit.clone())),
        Some(TokenPreferenceRejection::DecimalsRequired)
    );
    if let StateCommand::UpdateCustomToken { decimals, .. } = &mut edit {
        *decimals = Some(6);
    }
    let edited = reduce_state(added.state, edit);
    assert_eq!(rejection(&edited), None);
    assert_eq!(edited.state.token_preferences[0].token.decimals, 6);
}

fn rejection(transition: &Reduced) -> Option<TokenPreferenceRejection> {
    transition.events.iter().find_map(|event| match event {
        StateEvent::TokenPreferenceRejected { reason } => Some(*reason),
        _ => None,
    })
}

/// A token is added under the protocol named for it — the network's only
/// one when none is — never one guessed from the identifier's shape. The
/// identifier is judged by that protocol, and the same contract under two
/// labels of one protocol is one token.
#[test]
fn custom_tokens_are_added_under_the_named_protocol() {
    use crate::registry::Chain;
    let mut state = no_tokens();
    let command = |chain_id, standard: Option<&str>, contract: &str| StateCommand::AddCustomToken {
        chain_id,
        standard: standard
            .map(|name| crate::registry::TokenStandard::parse(name).expect("a standard's name")),
        symbol: "CUSTOM".into(),
        name: "Custom".into(),
        contract: contract.into(),
        coingecko_id: String::new(),
        coinpaprika_id: String::new(),
        decimals: Some(6),
    };
    let rejected = |reason| [StateEvent::TokenPreferenceRejected { reason }];
    let added = |events: &[StateEvent]| {
        events
            .iter()
            .any(|e| matches!(e, StateEvent::TokenPreferencesChanged { .. }))
    };
    let trc20 = "TJRabPrwbZy45sbavfcjinPJC18kjpRTv8";

    // Several protocols and none named: refused, whatever the shape.
    for (chain, identifier) in [
        (Chain::Tron, trc20),
        (Chain::Tron, "1002000"),
        (Chain::Aptos, "0x1::coin::T"),
        (
            Chain::BnbChain,
            "0x1111111111111111111111111111111111111111",
        ),
    ] {
        let events = reduce_state_in_place(&mut state, command(chain, None, identifier));
        assert_eq!(
            events,
            rejected(TokenPreferenceRejection::StandardRequired),
            "{chain:?}"
        );
    }
    assert!(state.token_preferences.is_empty());

    // Named, each is kept under the protocol it was named under.
    assert!(added(&reduce_state_in_place(
        &mut state,
        command(Chain::Tron, Some("TRC-20"), trc20)
    )));
    assert!(added(&reduce_state_in_place(
        &mut state,
        command(Chain::Tron, Some("TRC-10"), "1002000")
    )));
    let address = "0x1111111111111111111111111111111111111111";
    assert!(added(&reduce_state_in_place(
        &mut state,
        command(Chain::BnbChain, Some("BEP-20"), address)
    )));
    assert!(added(&reduce_state_in_place(
        &mut state,
        command(Chain::Aptos, Some("Aptos Coin"), "0x001::coin::T")
    )));
    // The network's only protocol needs no naming.
    assert!(added(&reduce_state_in_place(
        &mut state,
        command(
            Chain::Solana,
            None,
            "So11111111111111111111111111111111111111112"
        )
    )));
    let standard_of = |state: &ResidentState, id: &str| {
        state
            .token_preferences
            .iter()
            .find(|p| p.token.deployment_id == id)
            .and_then(|p| p.token.token_standard)
            .map(|standard| standard.as_str())
    };
    assert_eq!(standard_of(&state, "tron:trc-10:1002000"), Some("TRC-10"));
    assert_eq!(
        standard_of(&state, "aptos:aptos coin:0x1::coin::T"),
        Some("Aptos Coin")
    );
    assert!(
        state
            .token_preferences
            .iter()
            .any(|p| p.token.chain_id == Chain::BnbChain
                && p.token.token_standard == Some(crate::registry::TokenStandard::Bep20))
    );
    assert!(
        state
            .token_preferences
            .iter()
            .any(|p| p.token.chain_id == Chain::Solana
                && p.token.token_standard == Some(crate::registry::TokenStandard::Spl))
    );

    // ERC-20 and BEP-20 are two labels of one protocol: one contract.
    let events = reduce_state_in_place(
        &mut state,
        command(Chain::BnbChain, Some("ERC-20"), address),
    );
    assert_eq!(events, rejected(TokenPreferenceRejection::DuplicateToken));

    // A protocol the network lacks, and an identifier the named protocol
    // does not take.
    for (chain, standard, identifier, reason) in [
        (
            Chain::Solana,
            "ERC-20",
            address,
            TokenPreferenceRejection::UnsupportedStandard,
        ),
        (
            Chain::Tron,
            "TRC-10",
            trc20,
            TokenPreferenceRejection::InvalidContract,
        ),
        (
            Chain::Tron,
            "TRC-20",
            "1002001",
            TokenPreferenceRejection::InvalidContract,
        ),
        (
            Chain::Aptos,
            "AIP-21",
            "0x1::coin::T",
            TokenPreferenceRejection::InvalidContract,
        ),
    ] {
        let events = reduce_state_in_place(&mut state, command(chain, Some(standard), identifier));
        assert_eq!(events, rejected(reason), "{chain:?} {standard}");
    }

    // A coin's own ERC-20 face is the native row the network already has,
    // whatever case it is typed in.
    for (chain, face) in [
        (Chain::Arc, "0x3600000000000000000000000000000000000000"),
        (Chain::Stable, "0x779DED0C9E1022225F8E0630B35A9B54BE713736"),
        (Chain::Metis, "0xdeaddeaddeaddeaddeaddeaddeaddeaddead0000"),
    ] {
        let events = reduce_state_in_place(&mut state, command(chain, None, face));
        assert_eq!(
            events,
            rejected(TokenPreferenceRejection::DuplicateToken),
            "{chain:?}"
        );
    }
}

const EVM_CONTRACT: &str = "0x742d35cc6634c0532925a3b844bc454e4438f44e";

/// The bounds a control reads are the reducer's: the highest it offers
/// is accepted and one past it is not.
#[test]
fn input_bounds_are_the_reducers_bounds() {
    let bounds = input_bounds();
    let at = |decimals| {
        reduce_state(
            no_tokens(),
            add_token(
                crate::registry::Chain::Ethereum,
                "AT",
                EVM_CONTRACT,
                decimals,
            ),
        )
    };
    assert_eq!(rejection(&at(bounds.max_token_decimals)), None);
    assert_eq!(
        rejection(&at(bounds.max_token_decimals + 1)),
        Some(TokenPreferenceRejection::TooManyDecimals)
    );

    let percent = |value| {
        app_settings_applying(
            AppSettings::default(),
            AppSettingUpdate::LargeMovementAlertPercentThreshold { value },
        )
        .large_movement_alert_percent_threshold
    };
    assert_eq!(percent(0.0), bounds.large_movement_percent_min);
    assert_eq!(percent(1_000.0), bounds.large_movement_percent_max);
}

/// A contract is judged by the chain that would host it, not by whichever
/// arm a switch fell into. The composer's `default` assumed EVM and the
/// CLI checked nothing at all, so a Solana mint went into the Base list
/// and every balance read for it failed.
#[test]
fn a_contract_is_judged_by_the_chain_that_hosts_it() {
    let solana_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    let wrong_chain = reduce_state(
        no_tokens(),
        add_token(crate::registry::Chain::Base, "USDC", solana_mint, 6),
    );
    assert_eq!(
        rejection(&wrong_chain),
        Some(TokenPreferenceRejection::InvalidContract)
    );
    assert!(wrong_chain.state.token_preferences.is_empty());

    let wrong_way_round = reduce_state(
        no_tokens(),
        add_token(crate::registry::Chain::Solana, "USDC", EVM_CONTRACT, 6),
    );
    assert_eq!(
        rejection(&wrong_way_round),
        Some(TokenPreferenceRejection::InvalidContract)
    );

    let right = reduce_state(
        no_tokens(),
        add_token(crate::registry::Chain::Solana, "USDC", solana_mint, 6),
    );
    assert_eq!(rejection(&right), None);
    assert_eq!(right.state.token_preferences.len(), 1);
}

/// The symbol is trimmed and upper-cased, and a pasted name is refused
/// rather than stored as one.
#[test]
fn a_symbol_is_normalized_and_a_pasted_name_is_not_one() {
    let added = reduce_state(
        no_tokens(),
        add_token(crate::registry::Chain::Base, "  moon ", EVM_CONTRACT, 18),
    );
    assert_eq!(added.state.token_preferences[0].token.symbol, "MOON");
    // The deployment’s standard follows its identifier, not a network-wide default.
    assert_eq!(
        added.state.token_preferences[0].token.token_standard,
        Some(crate::registry::TokenStandard::Erc20)
    );
    assert!(!added.state.token_preferences[0].is_built_in);

    let pasted = reduce_state(
        no_tokens(),
        add_token(
            crate::registry::Chain::Base,
            "Moonbeam Network Token",
            EVM_CONTRACT,
            18,
        ),
    );
    assert_eq!(
        rejection(&pasted),
        Some(TokenPreferenceRejection::SymbolTooLong)
    );

    let empty = reduce_state(
        no_tokens(),
        add_token(crate::registry::Chain::Base, "  ", EVM_CONTRACT, 18),
    );
    assert_eq!(
        rejection(&empty),
        Some(TokenPreferenceRejection::EmptySymbol)
    );
}

/// A duplicate is the same *contract* on the same chain, under the chain's
/// own normalization — so an EVM address in another case is one and the
/// CLI's symbol compare was answering a different question.
#[test]
fn a_duplicate_is_the_same_contract_however_it_is_spelled() {
    let first = reduce_state(
        no_tokens(),
        add_token(crate::registry::Chain::Base, "MOON", EVM_CONTRACT, 18),
    );
    let again = reduce_state(
        first.state.clone(),
        add_token(
            crate::registry::Chain::Base,
            "SUN",
            &EVM_CONTRACT.to_uppercase(),
            18,
        ),
    );
    assert_eq!(
        rejection(&again),
        Some(TokenPreferenceRejection::DuplicateToken)
    );
    assert_eq!(again.state.token_preferences.len(), 1);

    // Same contract string, different chain: two different tokens.
    let elsewhere = reduce_state(
        first.state,
        add_token(crate::registry::Chain::Arbitrum, "MOON", EVM_CONTRACT, 18),
    );
    assert_eq!(rejection(&elsewhere), None);
    assert_eq!(elsewhere.state.token_preferences.len(), 2);
}

/// The catalog's rows are not the user's to edit or delete.
#[test]
fn a_built_in_token_is_not_editable() {
    let state = ResidentState::default();
    let built_in = state
        .token_preferences
        .iter()
        .find(|entry| entry.is_built_in)
        .expect("the catalog ships tokens")
        .clone();
    let count = state.token_preferences.len();

    let removed = reduce_state(
        state.clone(),
        StateCommand::RemoveCustomToken {
            chain_id: built_in.token.chain_id,
            contract: built_in.token.contract.clone(),
        },
    );
    assert_eq!(
        rejection(&removed),
        Some(TokenPreferenceRejection::BuiltInToken)
    );
    assert_eq!(removed.state.token_preferences.len(), count);

    let rescaled = reduce_state(
        state,
        StateCommand::UpdateCustomToken {
            chain_id: built_in.token.chain_id,
            contract: built_in.token.contract.clone(),
            symbol: built_in.token.symbol.clone(),
            name: built_in.token.name.clone(),
            coingecko_id: String::new(),
            coinpaprika_id: String::new(),
            decimals: Some(2),
        },
    );
    assert_eq!(
        rejection(&rescaled),
        Some(TokenPreferenceRejection::BuiltInToken)
    );
}

/// A custom token's price sources are provider ids, edited in place under
/// the token's own identity: a URL is not an id and changes nothing, and
/// an edit naming none clears both.
#[test]
fn a_custom_tokens_price_ids_are_edited_in_place_and_a_url_is_refused() {
    let chain = crate::registry::Chain::Base;
    let edit = |coingecko_id: &str, coinpaprika_id: &str| StateCommand::UpdateCustomToken {
        chain_id: chain,
        contract: EVM_CONTRACT.into(),
        symbol: "USDC".into(),
        name: "Independent token".into(),
        coingecko_id: coingecko_id.into(),
        coinpaprika_id: coinpaprika_id.into(),
        decimals: Some(6),
    };
    let ids = |state: &ResidentState| {
        let token = &state.token_preferences[0].token;
        (
            token.token_id.clone(),
            token.coingecko_id.clone(),
            token.coinpaprika_id.clone(),
        )
    };
    let mut state = no_tokens();
    reduce_state_in_place(
        &mut state,
        StateCommand::AddCustomToken {
            standard: None,
            chain_id: chain,
            symbol: "USDC".into(),
            name: "Independent token".into(),
            contract: EVM_CONTRACT.into(),
            coingecko_id: String::new(),
            coinpaprika_id: " Custom-Independent ".into(),
            decimals: Some(6),
        },
    );
    let token_id = format!("custom:base:erc-20:{EVM_CONTRACT}");
    assert_eq!(
        ids(&state),
        (token_id.clone(), String::new(), "custom-independent".into())
    );

    let edited = reduce_state(state, edit("usd-coin", "usdc-usd-coin"));
    assert_eq!(rejection(&edited), None);
    assert_eq!(
        ids(&edited.state),
        (token_id.clone(), "usd-coin".into(), "usdc-usd-coin".into())
    );

    let url = reduce_state(
        edited.state.clone(),
        edit("usd-coin", "https://coinpaprika.com/coin/example"),
    );
    assert_eq!(
        rejection(&url),
        Some(TokenPreferenceRejection::InvalidPriceId)
    );
    assert_eq!(url.state.token_preferences, edited.state.token_preferences);

    let cleared = reduce_state(edited.state, edit("", ""));
    assert_eq!(rejection(&cleared), None);
    assert_eq!(
        ids(&cleared.state),
        (token_id, String::new(), String::new())
    );
}

/// A reset goes back to the catalog and takes the custom rows with it.
#[test]
fn a_reset_drops_what_the_user_added() {
    let added = reduce_state(
        no_tokens(),
        add_token(crate::registry::Chain::Base, "MOON", EVM_CONTRACT, 18),
    );
    let reset = reduce_state(added.state, StateCommand::ResetTokenPreferences);
    assert!(
        !reset
            .state
            .token_preferences
            .iter()
            .any(|entry| entry.token.symbol == "MOON"),
        "a custom token survived the reset"
    );
    assert!(reset.state.token_preferences.iter().all(|e| e.is_built_in));
}

#[test]
fn stored_settings_require_every_current_field() {
    let complete = serde_json::to_value(AppSettings::default()).unwrap();
    for key in complete.as_object().unwrap().keys() {
        let mut partial = complete.clone();
        partial.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<AppSettings>(partial).is_err(),
            "{key}"
        );
    }
    assert!(serde_json::from_value::<AppSettings>(complete).is_ok());
}

#[test]
fn upsert_wallet_adds_a_new_wallet() {
    let mut state = no_tokens();
    let events = upsert_wallet_in_place(
        &mut state,
        test_wallet("wallet-1", crate::registry::Chain::Bitcoin),
    );

    assert_eq!(state.wallets.len(), 1);
    assert!(matches!(events[0], StateEvent::WalletAdded { .. }));
}
