//! Each prerequisite the network enforces on closing an account, refused
//! before anything is built.

use super::*;
use crate::api::horizon::StellarMergeableAccount;
use crate::api::xrpl_json_rpc::XrpDeletableAccount;

const DESTINATION: &str = "rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe";

fn xrp_state() -> XrpDeletionState {
    XrpDeletionState {
        source: Some(XrpDeletableAccount {
            balance_drops: 25_000_000,
            sequence: 1_000,
            owner_count: 2,
            minted_nft_sequence: None,
            blockers: 0,
        }),
        destination_flags: Some(0),
        ledger_index: 1_256,
        reserve_base: 1_000_000,
        reserve_increment: 200_000,
    }
}

fn refusal<T: std::fmt::Debug>(result: Result<T, SpectraBridgeError>) -> String {
    result.unwrap_err().to_string()
}

#[test]
fn an_xrp_account_with_nothing_blocking_deletes_for_one_owner_reserve() {
    // Two deletable objects go with it; its reserve is the base and theirs.
    assert_eq!(
        xrp_deletion(&xrp_state(), DESTINATION).unwrap(),
        (25_000_000, 1_000, 2, 1_400_000, 200_000)
    );
}

#[test]
fn xrp_refuses_what_the_network_would() {
    let mut missing = xrp_state();
    missing.source = None;
    assert!(refusal(xrp_deletion(&missing, DESTINATION)).contains("nothing to close"));

    let mut blocked = xrp_state();
    blocked.source.as_mut().unwrap().blockers = 1;
    assert!(refusal(xrp_deletion(&blocked, DESTINATION)).contains("trust lines"));

    let mut too_many = xrp_state();
    too_many.source.as_mut().unwrap().owner_count = 1_001;
    assert!(refusal(xrp_deletion(&too_many, DESTINATION)).contains("1001 objects"));

    // Sequence + 256 must not pass the validated ledger, nor must the
    // minted-NFT sequence.
    let mut recent = xrp_state();
    recent.ledger_index = 1_255;
    assert!(refusal(xrp_deletion(&recent, DESTINATION)).contains("after ledger 1256"));
    let mut minted = xrp_state();
    minted.source.as_mut().unwrap().minted_nft_sequence = Some(1_100);
    assert!(refusal(xrp_deletion(&minted, DESTINATION)).contains("after ledger 1356"));

    let mut absent = xrp_state();
    absent.destination_flags = None;
    assert!(refusal(xrp_deletion(&absent, DESTINATION)).contains("not on the network"));
    let mut tagged = xrp_state();
    tagged.destination_flags = Some(XRP_REQUIRE_DEST_TAG);
    assert!(refusal(xrp_deletion(&tagged, DESTINATION)).contains("destination tag"));
    let mut authorized = xrp_state();
    authorized.destination_flags = Some(XRP_DEPOSIT_AUTH);
    assert!(refusal(xrp_deletion(&authorized, DESTINATION)).contains("authorized"));

    let mut poor = xrp_state();
    poor.source.as_mut().unwrap().balance_drops = 200_000;
    assert!(refusal(xrp_deletion(&poor, DESTINATION)).contains("does not cover the fee"));
}

fn stellar_state() -> StellarMergeState {
    StellarMergeState {
        source: Some(StellarMergeableAccount {
            balance_stroops: 50_000_000,
            sequence: 4_294_967_296 * 10 + 5,
            subentries: 0,
            sponsoring: 0,
            sponsored: 0,
            auth_immutable: false,
        }),
        destination_memo_required: Some(false),
        ledger: 11,
        base_reserve: 5_000_000,
    }
}

#[test]
fn a_bare_stellar_account_merges_with_the_next_sequence() {
    assert_eq!(
        stellar_merge(&stellar_state(), DESTINATION).unwrap(),
        (50_000_000, 4_294_967_296 * 10 + 6, 10_000_000)
    );
    // A sponsored account's own reserve was its sponsor's.
    let mut sponsored = stellar_state();
    sponsored.source.as_mut().unwrap().sponsored = 2;
    assert_eq!(stellar_merge(&sponsored, DESTINATION).unwrap().2, 0);
}

#[test]
fn stellar_refuses_what_the_network_would() {
    let mut missing = stellar_state();
    missing.source = None;
    assert!(refusal(stellar_merge(&missing, DESTINATION)).contains("nothing to close"));
    let mut entries = stellar_state();
    entries.source.as_mut().unwrap().subentries = 1;
    assert!(refusal(stellar_merge(&entries, DESTINATION)).contains("trust lines"));
    let mut sponsor = stellar_state();
    sponsor.source.as_mut().unwrap().sponsoring = 1;
    assert!(refusal(stellar_merge(&sponsor, DESTINATION)).contains("sponsors"));
    let mut immutable = stellar_state();
    immutable.source.as_mut().unwrap().auth_immutable = true;
    assert!(refusal(stellar_merge(&immutable, DESTINATION)).contains("immutable"));
    // The merge's sequence must stay below the ledger's starting sequence.
    let mut ahead = stellar_state();
    ahead.ledger = 10;
    assert!(refusal(stellar_merge(&ahead, DESTINATION)).contains("later ledger"));
    let mut absent = stellar_state();
    absent.destination_memo_required = None;
    assert!(refusal(stellar_merge(&absent, DESTINATION)).contains("not on the network"));
    let mut memo = stellar_state();
    memo.destination_memo_required = Some(true);
    assert!(refusal(stellar_merge(&memo, DESTINATION)).contains("memo"));
}
