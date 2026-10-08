use super::*;
use crate::send::zcash_shielded::{PreparedZcashShielded, ZcashPayment};

const SENDER: &str = "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F";
const RECIPIENT: &str = "u1hekqggren38vzwhnny8vewspf9j59nxaly64ese097lmedgnyzv979k9rh00wj7lyhfxjkvwee5rw44vppm656k4qyp6fxq99588xm68mryat0nejep54e2g2ge23vpn0u5u23gpnzc63teksttlk455q8xxm0j9m8e76mg4lqh752r4";
const OWN: &str = "u14hxpxnwfzfujl6n36gfpr2e90mxzupmaksy3ly7ztgcey8qa06703yutk3m3u46tfkeywk9w69atu6zmdlp0lekh8jpsxmpzngrspf379dtgyvyfggtqlyvcq3wjr5y5xngex2qtsxheywdyzdfalk5clnms8wy3fje0j560wvsjm6d8";

/// A stored shielded artifact as `store_zcash_shielded` stores one: a
/// payment of 0.3 ZEC with a memo, or shielding 0.49985 ZEC.
fn stored(shielding: bool) -> StoredSend {
    let prepared = if shielding {
        PreparedZcashShielded {
            proposal_hex: "00".into(),
            payments: vec![],
            fee_zat: 15_000,
            transparent_in_zat: 50_000_000,
            shielded_in_zat: 0,
            change_zat: 49_985_000,
            spends_sapling: false,
            uses_sapling: false,
        }
    } else {
        PreparedZcashShielded {
            proposal_hex: "00".into(),
            payments: vec![ZcashPayment {
                address: RECIPIENT.into(),
                zatoshis: 30_000_000,
                memo: Some("thanks".into()),
            }],
            fee_zat: 10_000,
            transparent_in_zat: 0,
            shielded_in_zat: 150_000_000,
            change_zat: 119_990_000,
            spends_sapling: false,
            uses_sapling: false,
        }
    };
    let (recipient, amount) = if shielding {
        (OWN, "0.49985")
    } else {
        (RECIPIENT, "0.3")
    };
    let mut stored = StoredSend {
        view: SendArtifact {
            id: "zec".into(),
            revision: 0,
            stage: SendStage::Prepared,
            wallet_id: "wallet".into(),
            chain_id: crate::registry::Chain::Zcash,
            sender: SENDER.into(),
            recipient: recipient.into(),
            amount: amount.into(),
            asset: "ZEC".into(),
            symbol: "ZEC".into(),
            staking: None,
            operation: Some(if shielding {
                WalletOperation::ShieldTransparent {
                    amount: amount.into(),
                    network_fee: "0.00015".into(),
                }
            } else {
                WalletOperation::ShieldedPayment {
                    memo: Some("thanks".into()),
                    network_fee: "0.0001".into(),
                }
            }),
            created_at: 1.0,
            review_digest: String::new(),
            review: SendArtifactReview::default(),
            prepared_details: String::new(),
            signing_payload_hex: String::new(),
            signed_payload: None,
            transaction_hash: None,
            attempts: vec![],
            selected_endpoints: vec![],
            memo: None,
        },
        request: super::super::SendExecutionRequest {
            chain_id: crate::registry::Chain::Zcash,
            wallet_id: "wallet".into(),
            password: None,
            to_address: recipient.into(),
            amount_str: amount.into(),
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
        prepared: PreparedPayload::ZcashShielded(prepared),
        submission: None,
        signed_digest: None,
        substrate_verified_through: None,
        icp_staking_receipts: vec![],
    };
    reseal(&mut stored);
    stored
}

/// What a tamperer with the database would do after an edit: make the
/// stored artifact consistent with itself again.
fn reseal(stored: &mut StoredSend) {
    stored.view.prepared_details = serde_json::to_string_pretty(&stored.prepared).unwrap();
    stored.view.review_digest = stored.digest().unwrap();
}

fn prepared(stored: &mut StoredSend) -> &mut PreparedZcashShielded {
    match &mut stored.prepared {
        PreparedPayload::ZcashShielded(prepared) => prepared,
        _ => unreachable!(),
    }
}

fn memo(stored: &mut StoredSend) -> &mut Option<String> {
    match stored.view.operation.as_mut().unwrap() {
        WalletOperation::ShieldedPayment { memo, .. } => memo,
        _ => unreachable!(),
    }
}

fn refused(shielding: bool, what: &str, tamper: impl Fn(&mut StoredSend)) {
    let mut stored = stored(shielding);
    tamper(&mut stored);
    reseal(&mut stored);
    let error = stored.validate().unwrap_err().to_string();
    assert!(error.contains("operation was altered"), "{what}: {error}");
}

/// A shielded payment shows what its proposal pays: the one recipient, the
/// amount, the memo and the fee, from shielded notes alone.
#[test]
fn a_shielded_payment_shows_what_it_pays() {
    stored(false).validate().unwrap();
    refused(false, "another recipient", |s| {
        s.view.recipient = OWN.into();
        s.request.to_address = OWN.into();
    });
    refused(false, "another amount", |s| {
        s.view.amount = "0.31".into();
        s.request.amount_str = "0.31".into();
    });
    refused(false, "another memo", |s| {
        *memo(s) = Some("thank you".into())
    });
    refused(false, "a memo hidden", |s| *memo(s) = None);
    refused(false, "another fee", |s| prepared(s).fee_zat = 15_000);
    refused(false, "a second payment", |s| {
        let payment = prepared(s).payments[0].clone();
        prepared(s).payments.push(payment);
    });
    refused(false, "transparent funds spent", |s| {
        prepared(s).transparent_in_zat = 1
    });
    refused(false, "named a shielding", |s| {
        s.view.operation = Some(WalletOperation::ShieldTransparent {
            amount: "0.3".into(),
            network_fee: "0.0001".into(),
        })
    });
    refused(false, "unnamed", |s| s.view.operation = None);
}

/// Shielding pays no one: every transparent input moves to the wallet's own
/// shielded pool, and the amount shown is what arrives there.
#[test]
fn shielding_moves_only_the_wallets_own_funds() {
    stored(true).validate().unwrap();
    refused(true, "a payment", |s| {
        prepared(s).payments.push(ZcashPayment {
            address: RECIPIENT.into(),
            zatoshis: 1,
            memo: None,
        })
    });
    refused(true, "another amount", |s| {
        s.view.amount = "0.5".into();
        s.request.amount_str = "0.5".into();
        if let Some(WalletOperation::ShieldTransparent { amount, .. }) = &mut s.view.operation {
            *amount = "0.5".into();
        }
    });
    refused(true, "another fee", |s| prepared(s).fee_zat = 10_000);
    refused(true, "shielded notes spent", |s| {
        prepared(s).shielded_in_zat = 1
    });
    refused(true, "nothing to shield", |s| {
        prepared(s).transparent_in_zat = 0
    });
    refused(true, "named a payment", |s| {
        s.view.operation = Some(WalletOperation::ShieldedPayment {
            memo: None,
            network_fee: "0.00015".into(),
        })
    });
}
