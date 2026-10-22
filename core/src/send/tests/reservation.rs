//! One resource is one key, however it was spelled, and a second signed
//! artifact claiming it is refused when it is stored.

use super::*;
use crate::api::utxo::UtxoInput;
use crate::send::account_utxo::{AccountProtocol, PreparedAccountTransfer};
use crate::send::payload::PreparedSubmission;
use crate::send::stages::{
    PreparedAccountUtxoTransaction, SendArtifact, SendArtifactReview, SendStage, UtxoSendSource,
};
use crate::wallet_db::{WalletDatabase, send_exists, send_save};

const TXID: &str = "00ab11cd22ef33aa44bb55cc66dd77ee88ff99aabbccddeeff00112233445566";

/// The one reviewed input, its txid spelled as an indexer might.
fn spending(txid: &str) -> Vec<UtxoPreparedInput> {
    let script = vec![0x76, 0xa9, 0x14];
    vec![UtxoPreparedInput {
        source: UtxoSendSource {
            address: "source".into(),
            derivation_path: None,
            script_pubkey: script.clone(),
        },
        utxo: UtxoInput::fixture(txid, 3, 50_000, script),
    }]
}

/// A signed artifact of `prepared`, consistent enough to store.
fn signed(id: &str, chain: Chain, sender: &str, prepared: PreparedPayload) -> StoredSend {
    let payload = format!("signed {id}");
    let mut stored = StoredSend {
        view: SendArtifact {
            id: id.into(),
            revision: 0,
            stage: SendStage::Signed,
            wallet_id: "wallet".into(),
            chain_id: chain,
            sender: sender.into(),
            recipient: "recipient".into(),
            amount: "0.0004".into(),
            asset: chain.coin_symbol().into(),
            symbol: chain.coin_symbol().into(),
            staking: None,
            operation: None,
            created_at: 1.0,
            review_digest: String::new(),
            review: SendArtifactReview::default(),
            prepared_details: serde_json::to_string_pretty(&prepared).unwrap(),
            signing_payload_hex: String::new(),
            signed_payload: Some(payload.clone()),
            transaction_hash: None,
            attempts: vec![],
            selected_endpoints: vec![],
            memo: None,
        },
        request: crate::send::SendExecutionRequest {
            chain_id: chain,
            wallet_id: "wallet".into(),
            password: None,
            to_address: "recipient".into(),
            amount_str: "0.0004".into(),
            contract_address: None,
            token_standard: None,
            token_decimals: None,
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: None,
            evm_overrides: None,
            sign_only: false,
            memo: None,
        },
        prepared,
        submission: Some(PreparedSubmission {
            payload: crate::send::payload::SignedPayload::Hex(payload),
            transaction_hash: None,
            nonce: None,
        }),
        signed_digest: None,
        substrate_verified_through: None,
        icp_staking_receipts: vec![],
    };
    stored.view.review_digest = stored.digest().unwrap();
    stored.signed_digest = stored.submission_digest().unwrap();
    stored
}

fn keys(stored: &StoredSend) -> Vec<String> {
    stored
        .reservations()
        .unwrap()
        .iter()
        .map(|reservation| reservation.key(stored.view.chain_id))
        .collect()
}

/// An account transfer (the Bitcoin family's), a Litecoin and a Peercoin
/// transaction each spending one outpoint, spelled in lowercase by one
/// indexer and uppercase by another: the second is refused, not signed twice.
#[test]
fn one_outpoint_in_either_case_is_reserved_once_on_every_utxo_protocol() {
    let account = |inputs| {
        PreparedPayload::AccountTransfer(PreparedAccountTransfer {
            inputs,
            outputs: vec![(vec![0x51], 40_000)],
            fee: 10_000,
            protocol: AccountProtocol::Bitcoin,
        })
    };
    let utxo = |inputs| PreparedAccountUtxoTransaction {
        inputs,
        amount: 40_000,
        fee: 10_000,
        recipient_script: vec![0x51],
    };
    let protocols: [(Chain, &dyn Fn(Vec<UtxoPreparedInput>) -> PreparedPayload); 4] = [
        (Chain::Bitcoin, &account),
        (Chain::Dogecoin, &account),
        (Chain::Litecoin, &|inputs| {
            PreparedPayload::Litecoin(utxo(inputs))
        }),
        (Chain::Peercoin, &|inputs| {
            PreparedPayload::Peercoin(utxo(inputs))
        }),
    ];
    for (chain, payload) in protocols {
        let db = WalletDatabase::new(":memory:");
        let first = signed("first", chain, "sender", payload(spending(TXID)));
        let second = signed(
            "second",
            chain,
            "sender",
            payload(spending(&TXID.to_ascii_uppercase())),
        );
        assert_eq!(keys(&first), keys(&second), "{chain}");
        assert_eq!(
            keys(&first),
            vec![format!("{}:outpoint:{TXID}:3", chain.str_id())]
        );
        send_save(&db, &first, &first.reservations().unwrap()).unwrap();
        assert!(
            send_save(&db, &second, &second.reservations().unwrap()).is_err(),
            "{chain}"
        );
        assert!(!send_exists(&db, "second").unwrap(), "{chain}");
    }

    // A Litecoin peg-in spends transparent outputs too: the same outpoint
    // as a payment's is the same key.
    let pegin = signed(
        "pegin",
        Chain::Litecoin,
        "sender",
        PreparedPayload::LitecoinPegIn(crate::send::litecoin_mweb::prepared::PreparedPegIn {
            inputs: spending(&TXID.to_ascii_uppercase()),
            recipient: "recipient".into(),
            amount: 40_000,
            mweb_fee: 2_100,
            canonical_fee: 500,
        }),
    );
    let payment = signed(
        "payment",
        Chain::Litecoin,
        "sender",
        PreparedPayload::Litecoin(utxo(spending(TXID))),
    );
    assert_eq!(keys(&pegin), keys(&payment));

    // One txid on two networks is two outputs.
    let bitcoin = signed("bitcoin", Chain::Bitcoin, "sender", account(spending(TXID)));
    assert_ne!(keys(&bitcoin), keys(&payment));
}

/// An account's sequence is keyed on its sender as the chain normalizes it,
/// on EVM as on every other chain: a checksummed and a lowercase spelling of
/// one sender hold one nonce.
#[test]
fn an_account_sequence_is_keyed_on_the_normalized_sender() {
    let sender = "0x52908400098527886E0F7030069857D2E4169EE7";
    let reservation = |sender: &str| Reservation::AccountSequence {
        sender: sender.into(),
        sequence: 7,
    };
    assert_eq!(
        reservation(sender).key(Chain::Ethereum),
        reservation(&sender.to_ascii_lowercase()).key(Chain::Ethereum)
    );
    assert_eq!(
        reservation(sender).key(Chain::Ethereum),
        format!("ethereum:sequence:{}:7", sender.to_ascii_lowercase())
    );
    // Stellar's account ids are uppercase; a lowercase spelling is the same.
    let stellar = "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H";
    assert_eq!(
        reservation(stellar).key(Chain::Stellar),
        reservation(&stellar.to_ascii_lowercase()).key(Chain::Stellar)
    );
}
