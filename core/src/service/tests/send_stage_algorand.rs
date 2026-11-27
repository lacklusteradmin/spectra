//! An Algorand payment built, signed and broadcast against a mock node.
use super::*;
use crate::service::send_stage_support::{Secret, Wallet, refusal};
use serde_json::{Value, json};
use sha2::Digest as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/algorand-vectors.json"
    ))
    .unwrap()
}

/// A 25-word mnemonic's account and another's, from algosdk.
fn mnemonic(index: usize) -> (String, String) {
    let row = &vectors()["mnemonics"][index];
    (
        row["mnemonic"].as_str().unwrap().into(),
        row["address"].as_str().unwrap().into(),
    )
}

const BIP39: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

/// What the mock node holds, which a test moves between stages.
struct Ledger {
    genesis_id: &'static str,
    round: AtomicU64,
    sender: Mutex<String>,
    sender_amount: AtomicU64,
    recipient_amount: AtomicU64,
    rekeyed: AtomicBool,
    /// The signed transaction last submitted.
    submitted: Mutex<Option<Vec<u8>>>,
    /// A transaction the node remembers committed, and one only the
    /// indexer does.
    committed: Mutex<Option<String>>,
    indexed: Mutex<Option<String>>,
}

impl Ledger {
    fn new(sender: &str) -> Arc<Self> {
        Arc::new(Self {
            genesis_id: "mainnet-v1.0",
            round: AtomicU64::new(67_257_603),
            sender: Mutex::new(sender.into()),
            sender_amount: AtomicU64::new(5_000_000),
            recipient_amount: AtomicU64::new(1_000_000),
            rekeyed: AtomicBool::new(false),
            submitted: Mutex::new(None),
            committed: Mutex::new(None),
            indexed: Mutex::new(None),
        })
    }
}

fn reply(value: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(value)
}

fn missing() -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_json(json!({"message": "not found"}))
}

/// The txn a signed transaction's msgpack carries after its signature, and
/// the signature, checked against the sender's key.
fn verified_txid(signed: &[u8], sender: &str) -> String {
    assert_eq!(&signed[..7], &[0x82, 0xa3, b's', b'i', b'g', 0xc4, 0x40]);
    assert_eq!(&signed[71..75], &[0xa3, b't', b'x', b'n']);
    let message = [b"TX".as_slice(), &signed[75..]].concat();
    let key = crate::derivation::algorand::decode_address(sender).unwrap();
    ed25519_dalek::VerifyingKey::from_bytes(&key)
        .unwrap()
        .verify_strict(
            &message,
            &ed25519_dalek::Signature::from_bytes(&signed[7..71].try_into().unwrap()),
        )
        .unwrap();
    data_encoding::BASE32_NOPAD.encode(&sha2::Sha512_256::digest(&message))
}

async fn node(ledger: Arc<Ledger>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let path = request.url.path().to_string();
            let sender = ledger.sender.lock().unwrap().clone();
            let account = |address: &str, amount: u64| {
                let mut value = json!({
                    "address": address, "amount": amount, "min-balance": 100_000,
                    "round": ledger.round.load(Ordering::SeqCst), "status": "Offline",
                });
                if address == sender && ledger.rekeyed.load(Ordering::SeqCst) {
                    value["auth-addr"] = json!(mnemonic(3).1);
                }
                value
            };
            match path.as_str() {
                "/v2/transactions/params" => reply(json!({
                    "fee": 0, "min-fee": 1000,
                    "genesis-id": ledger.genesis_id,
                    "genesis-hash": "wGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit8=",
                    "last-round": ledger.round.load(Ordering::SeqCst),
                })),
                "/v2/transactions" => {
                    assert_eq!(request.method.as_str(), "POST");
                    let txid = verified_txid(&request.body, &sender);
                    *ledger.submitted.lock().unwrap() = Some(request.body.clone());
                    reply(json!({"txId": txid}))
                }
                _ if path == format!("/v2/accounts/{sender}") => {
                    assert_eq!(request.url.query(), Some("exclude=all"));
                    reply(account(
                        &sender,
                        ledger.sender_amount.load(Ordering::SeqCst),
                    ))
                }
                _ if path.starts_with("/v2/accounts/") => {
                    let address = path.trim_start_matches("/v2/accounts/");
                    reply(account(
                        address,
                        ledger.recipient_amount.load(Ordering::SeqCst),
                    ))
                }
                _ if path.starts_with("/v2/transactions/pending/") => {
                    let txid = path.trim_start_matches("/v2/transactions/pending/");
                    if ledger.committed.lock().unwrap().as_deref() == Some(txid) {
                        reply(json!({"confirmed-round": 67_257_700, "pool-error": "", "txn": {}}))
                    } else {
                        missing()
                    }
                }
                _ if path.starts_with("/v2/transactions/") => {
                    let txid = path.trim_start_matches("/v2/transactions/");
                    if ledger.indexed.lock().unwrap().as_deref() == Some(txid) {
                        reply(json!({"current-round": 67_300_000, "transaction": {
                            "id": txid, "confirmed-round": 67_257_650,
                            "genesis-id": "mainnet-v1.0",
                            "genesis-hash": "wGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit8=",
                        }}))
                    } else {
                        missing()
                    }
                }
                other => panic!("unexpected request {other}"),
            }
        })
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a_payment_is_priced_signed_and_accepted() {
    let (phrase, address) = mnemonic(2);
    let (_, recipient) = mnemonic(3);
    let ledger = Ledger::new(&address);
    let server = node(ledger.clone()).await;
    let wallet = Wallet::import(Chain::Algorand, &server.uri(), Secret::Phrase(&phrase)).await;
    assert_eq!(wallet.address, address);
    // A 25-word mnemonic is one key and stores no path.
    let stored = wallet.service.stored_wallet(&wallet.id).await.unwrap();
    assert_eq!(stored.derivation_path, None);

    let built = wallet
        .build(wallet.request(&recipient, "1.5"))
        .await
        .unwrap();
    let artifact = wallet
        .service
        .load_send_artifact(built.id.clone())
        .await
        .unwrap();
    let PreparedPayload::Algorand(prepared) = &artifact.prepared else {
        panic!("not an Algorand payment");
    };
    assert_eq!(prepared.amount, 1_500_000);
    assert_eq!(prepared.fee, 1_000);
    assert_eq!(prepared.first_valid, 67_257_603);
    assert_eq!(prepared.last_valid, 67_257_603 + 1_000);
    assert_eq!(prepared.receiver, recipient);
    assert_eq!(artifact.request.fee_amount.as_deref(), Some("0.001"));
    assert_eq!(
        built.signing_payload_hex,
        hex::encode(prepared.bytes_to_sign())
    );

    let signed = wallet.sign(&built).await.unwrap();
    let txid = signed.transaction_hash.clone().unwrap();
    assert_eq!(txid, prepared.txid());
    let broadcast = wallet.broadcast(&signed).await.unwrap();
    assert_eq!(broadcast.transaction_hash.as_deref(), Some(txid.as_str()));
    assert!(
        broadcast
            .attempts
            .iter()
            .any(|attempt| attempt.outcome == SubmissionOutcome::Accepted)
    );
    assert!(ledger.submitted.lock().unwrap().is_some());

    // Committed in the node's memory; then only in the indexer's.
    let status = |txid: String| {
        let service = wallet.service.clone();
        async move {
            service
                .algorand_transaction_status(Chain::Algorand, &txid)
                .await
                .unwrap()
        }
    };
    *ledger.committed.lock().unwrap() = Some(txid.clone());
    assert_eq!(
        status(txid.clone()).await,
        TransactionStatus::Confirmed {
            succeeded: true,
            block: Some(67_257_700)
        }
    );
    *ledger.committed.lock().unwrap() = None;
    wallet.add_endpoint("algod", &server.uri()).await;
    wallet.add_endpoint("algorand-indexer", &server.uri()).await;
    assert_eq!(status(txid.clone()).await, TransactionStatus::Pending);
    *ledger.indexed.lock().unwrap() = Some(txid.clone());
    assert_eq!(
        status(txid).await,
        TransactionStatus::Confirmed {
            succeeded: true,
            block: Some(67_257_650)
        }
    );
}

#[tokio::test]
async fn a_bip39_wallet_signs_with_its_arc52_key() {
    let account = &vectors()["bip39_accounts"][0];
    let address = account["address"].as_str().unwrap();
    let ledger = Ledger::new(address);
    let server = node(ledger.clone()).await;
    let wallet = Wallet::import(Chain::Algorand, &server.uri(), Secret::Phrase(BIP39)).await;
    assert_eq!(wallet.address, address);
    let stored = wallet.service.stored_wallet(&wallet.id).await.unwrap();
    assert_eq!(stored.derivation_path.as_deref(), Some("m/44'/283'/0'/0/0"));
    let built = wallet
        .build(wallet.request(&mnemonic(3).1, "0.25"))
        .await
        .unwrap();
    let signed = wallet.sign(&built).await.unwrap();
    wallet.broadcast(&signed).await.unwrap();
    // The node checked the signature against the account's public key.
    assert!(ledger.submitted.lock().unwrap().is_some());
}

#[tokio::test]
async fn what_the_network_would_refuse_is_refused_before_signing() {
    let (phrase, address) = mnemonic(2);
    let (_, recipient) = mnemonic(3);
    let ledger = Ledger::new(&address);
    let server = node(ledger.clone()).await;
    let wallet = Wallet::import(Chain::Algorand, &server.uri(), Secret::Phrase(&phrase)).await;

    // An asset, a memo, more than the account holds above its minimum.
    let token = wallet.token_request(&recipient, "1", "31566704", 6);
    assert!(wallet.build(token).await.is_err());
    let mut memo = wallet.request(&recipient, "1");
    memo.memo = Some(crate::send::payment_memo::PaymentMemo {
        kind: crate::registry::PaymentMemoKind::MemoText,
        value: "deposit".into(),
    });
    assert!(wallet.build(memo).await.is_err());
    // 5 ALGO held, 0.1 kept: 4.9 less the fee is the most it sends.
    assert!(
        refusal(wallet.build(wallet.request(&recipient, "4.9")).await).contains("Insufficient")
    );
    // A Stellar account is no Algorand address.
    let stellar = "GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN7";
    assert!(wallet.build(wallet.request(stellar, "1")).await.is_err());
    // A new account must receive at least its minimum balance.
    ledger.recipient_amount.store(0, Ordering::SeqCst);
    assert!(refusal(wallet.build(wallet.request(&recipient, "0.05")).await).contains("recipient"));
    assert!(wallet.built_nothing().await);
    assert!(
        wallet
            .build(wallet.request(&recipient, "0.1"))
            .await
            .is_ok()
    );
    ledger.recipient_amount.store(1_000_000, Ordering::SeqCst);

    // The most it can send goes through.
    let built = wallet
        .build(wallet.request(&recipient, "4.899"))
        .await
        .unwrap();
    // Rekeyed between review and signing: its key no longer signs for it.
    ledger.rekeyed.store(true, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("rekeyed"));
    ledger.rekeyed.store(false, Ordering::SeqCst);
    // The network reaches its last valid round before signing.
    ledger.round.store(67_257_603 + 1_000, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("expired"));
}

#[tokio::test]
async fn another_network_is_refused_and_a_mnemonic_takes_no_path() {
    let (phrase, address) = mnemonic(2);
    let ledger = Arc::new(Ledger {
        genesis_id: "testnet-v1.0",
        ..Arc::try_unwrap(Ledger::new(&address)).ok().unwrap()
    });
    let server = node(ledger).await;
    let wallet = Wallet::import(Chain::Algorand, &server.uri(), Secret::Phrase(&phrase)).await;
    assert!(
        wallet
            .build(wallet.request(&mnemonic(3).1, "1"))
            .await
            .is_err()
    );
    assert!(wallet.built_nothing().await);
    assert!(
        wallet
            .service
            .fetch_shared_send_preview(Chain::Algorand, address.clone())
            .await
            .is_err()
    );

    // A path given with a 25-word mnemonic is refused, not ignored.
    let commit = crate::derivation::import::WalletImportCommit {
        password: None,
        request: crate::derivation::import::WalletImportRequest {
            wallet_name: "Pathed".into(),
            chain: Chain::Algorand,
            kind: crate::derivation::import::WalletImportKind::Phrase,
        },
        derivation_path: Some("m/44'/283'/0'/0/0".into()),
        derivation_overrides: Default::default(),
        seed_phrase: Some(phrase),
        private_key: None,
        restore_height: None,
        named_account: None,
        ton_wallet_version: None,
        upgrade_wallet_id: None,
        created: false,
    };
    assert!(refusal(wallet.service.import_wallets(commit).await).contains("derivation path"));
}

#[tokio::test]
async fn a_preview_keeps_the_minimum_balance() {
    let (phrase, address) = mnemonic(2);
    let ledger = Ledger::new(&address);
    let server = node(ledger).await;
    let wallet = Wallet::import(Chain::Algorand, &server.uri(), Secret::Phrase(&phrase)).await;
    let preview = wallet
        .service
        .fetch_shared_send_preview(Chain::Algorand, address)
        .await
        .unwrap();
    assert_eq!(preview.estimated_network_fee, "0.001");
    assert_eq!(preview.spendable_balance, "4.9");
    assert_eq!(preview.max_sendable, "4.899");
    let reserve = wallet
        .service
        .account_reserve(Chain::Algorand)
        .await
        .unwrap();
    assert_eq!(reserve.amount, "0.1");
    assert_eq!(reserve.symbol, "ALGO");
}
