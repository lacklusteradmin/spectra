use super::*;
use crate::send::keys::Ed25519Seed;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/solana-token-2022.json"
    ))
    .unwrap()
}

fn text(value: &Value) -> String {
    value.as_str().unwrap().to_string()
}

/// What the loopback node holds.
#[derive(Clone)]
struct Node {
    mint_extensions: Vec<Value>,
    source: Option<Value>,
    destination: Option<Value>,
    /// The validation account's owner and data, base64.
    validation: Option<(String, String)>,
    epoch: (u64, u64, u64),
    simulation_error: Option<Value>,
}

impl Node {
    fn new(fixture: &Value) -> Self {
        Self {
            mint_extensions: Vec::new(),
            source: Some(token_account(
                fixture,
                "owner",
                5_000_000,
                json!([]),
                "initialized",
            )),
            destination: None,
            validation: None,
            epoch: (700, 1_000, 432_000),
            simulation_error: None,
        }
    }

    fn with_fee(mut self, older: (u64, u64, u64), newer: (u64, u64, u64)) -> Self {
        let fee = |(epoch, bps, max): (u64, u64, u64)| json!({"epoch": epoch, "transferFeeBasisPoints": bps, "maximumFee": max});
        self.mint_extensions
            .push(json!({"extension": "transferFeeConfig", "state": {
            "olderTransferFee": fee(older), "newerTransferFee": fee(newer), "withheldAmount": 0}}));
        self
    }

    fn with_hook(mut self, fixture: &Value) -> Self {
        self.mint_extensions
            .push(json!({"extension": "transferHook", "state": {
            "authority": null, "programId": fixture["hook_program"]}}));
        self.validation = Some((
            text(&fixture["hook_program"]),
            text(&fixture["validation_data"]),
        ));
        self
    }
}

fn token_account(
    fixture: &Value,
    holder: &str,
    amount: u64,
    extensions: Value,
    state: &str,
) -> Value {
    json!({"owner": "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb", "data": {"parsed": {"type": "account", "info": {
        "mint": fixture["mint"], "owner": fixture[holder], "state": state,
        "tokenAmount": {"amount": amount.to_string(), "decimals": 6}, "extensions": extensions}}}})
}

async fn serve(fixture: &Value, node: Node) -> (MockServer, Arc<Mutex<Vec<Value>>>) {
    let server = MockServer::start().await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let fixture = fixture.clone();
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            seen.lock().unwrap().push(body.clone());
            let params = &body["params"];
            let result = match body["method"].as_str().unwrap() {
                "getAccountInfo" => {
                    let address = text(&params[0]);
                    let base64 = params[1]["encoding"] == "base64";
                    let value = if address == text(&fixture["mint"]) && !base64 {
                        json!({"owner": "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb", "data": {"parsed": {
                            "type": "mint", "info": {"isInitialized": true, "decimals": 6,
                            "extensions": node.mint_extensions}}}})
                    } else if address == text(&fixture["source"]) && base64 {
                        json!({"owner": "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
                               "data": [fixture["source_data"], "base64"]})
                    } else if address == text(&fixture["source"]) {
                        node.source.clone().unwrap_or(Value::Null)
                    } else if address == text(&fixture["destination"]) && !base64 {
                        node.destination.clone().unwrap_or(Value::Null)
                    } else if address == text(&fixture["validation_account"]) && base64 {
                        match &node.validation {
                            Some((owner, data)) => json!({"owner": owner, "data": [data, "base64"]}),
                            None => Value::Null,
                        }
                    } else {
                        Value::Null
                    };
                    json!({"context": {"slot": 1}, "value": value})
                }
                "getEpochInfo" => json!({"epoch": node.epoch.0, "slotIndex": node.epoch.1,
                                          "slotsInEpoch": node.epoch.2, "absoluteSlot": 1}),
                "getLatestBlockhash" => json!({"value": {"blockhash": bs58::encode([5u8; 32]).into_string(),
                                                         "lastValidBlockHeight": 100}}),
                "simulateTransaction" => json!({"value": {"err": node.simulation_error, "logs": []}}),
                other => panic!("unexpected request {other}"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": body["id"], "result": result}))
        })
        .mount(&server)
        .await;
    (server, calls)
}

async fn prepare(
    fixture: &Value,
    node: Node,
) -> Result<crate::send::solana::PreparedSolanaTransaction, SendError> {
    let (server, _) = serve(fixture, node).await;
    crate::send::solana::prepare_transfer(
        &SolanaClient::new(Arc::new(vec![server.uri()])),
        &text(&fixture["owner"]),
        &text(&fixture["recipient"]),
        fixture["amount"].as_str().unwrap().parse().unwrap(),
        Some((&text(&fixture["mint"]), 6)),
    )
    .await
}

/// A legacy message's instructions, each account with the signer and
/// writable flags the message gives it.
fn decompile(message: &[u8]) -> Vec<(String, Vec<(String, bool, bool)>, String)> {
    let compact = |at: &mut usize| {
        let (mut value, mut shift) = (0usize, 0);
        loop {
            let byte = message[*at];
            *at += 1;
            value |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return value;
            }
            shift += 7;
        }
    };
    let (signers, readonly_signed, readonly_unsigned) = (
        usize::from(message[0]),
        usize::from(message[1]),
        usize::from(message[2]),
    );
    let mut at = 3;
    let count = compact(&mut at);
    let keys: Vec<String> = (0..count)
        .map(|i| bs58::encode(&message[at + 32 * i..at + 32 * (i + 1)]).into_string())
        .collect();
    at += 32 * count + 32;
    let flags = |i: usize| {
        let signer = i < signers;
        let writable = if signer {
            i < signers - readonly_signed
        } else {
            i < count - readonly_unsigned
        };
        (keys[i].clone(), signer, writable)
    };
    let instructions = compact(&mut at);
    (0..instructions)
        .map(|_| {
            let program = keys[usize::from(message[at])].clone();
            at += 1;
            let accounts = compact(&mut at);
            let metas = message[at..at + accounts]
                .iter()
                .map(|i| flags(usize::from(*i)))
                .collect();
            at += accounts;
            let length = compact(&mut at);
            let data = hex::encode(&message[at..at + length]);
            at += length;
            (program, metas, data)
        })
        .collect()
}

/// The fixture's instructions with each account's flags merged across the
/// whole transaction, as a compiled message carries them.
fn expected(case: &Value) -> Vec<(String, Vec<(String, bool, bool)>, String)> {
    let instructions = case.as_array().unwrap();
    let mut merged = std::collections::HashMap::<String, (bool, bool)>::new();
    for key in instructions
        .iter()
        .flat_map(|ix| ix["keys"].as_array().unwrap())
    {
        let entry = merged.entry(text(&key["pubkey"])).or_default();
        entry.0 |= key["signer"].as_bool().unwrap();
        entry.1 |= key["writable"].as_bool().unwrap();
    }
    instructions
        .iter()
        .map(|ix| {
            let keys = ix["keys"]
                .as_array()
                .unwrap()
                .iter()
                .map(|key| {
                    let (signer, writable) = merged[&text(&key["pubkey"])];
                    (text(&key["pubkey"]), signer, writable)
                })
                .collect();
            (text(&ix["program"]), keys, text(&ix["data"]))
        })
        .collect()
}

#[test]
fn the_execute_discriminator_is_the_interfaces() {
    assert_eq!(
        execute_discriminator(),
        [105, 37, 101, 197, 75, 251, 102, 26]
    );
}

#[test]
fn the_validation_account_lists_its_metas_and_seeds() {
    use base64::Engine;
    let fixture = fixture();
    let data = base64::engine::general_purpose::STANDARD
        .decode(text(&fixture["validation_data"]))
        .unwrap();
    let metas = extra_account_metas(&data).unwrap();
    assert_eq!(metas.len(), 8);
    assert_eq!(
        seeds(&metas[1].address_config).unwrap(),
        [
            Seed::Literal(b"counter".to_vec()),
            Seed::AccountKey { index: 1 }
        ]
    );
    assert_eq!(
        seeds(&metas[2].address_config).unwrap(),
        [
            Seed::AccountKey { index: 0 },
            Seed::InstructionData {
                index: 8,
                length: 8
            }
        ]
    );
    assert_eq!(
        seeds(&metas[3].address_config).unwrap(),
        [Seed::AccountData {
            account: 0,
            offset: 32,
            length: 32
        }]
    );
    assert_eq!(metas[5].discriminator, 0x80 + 9);
    // Another entry first is skipped; a truncated list is refused.
    let mut prefixed = vec![9; 8];
    prefixed.extend(2u32.to_le_bytes());
    prefixed.extend([0, 0]);
    prefixed.extend(&data);
    assert_eq!(extra_account_metas(&prefixed).unwrap(), metas);
    assert!(extra_account_metas(&data[..data.len() - 1]).is_err());
    assert!(extra_account_metas(&[]).is_err());
    let mut literal = [0u8; 32];
    literal[..3].copy_from_slice(&[1, 40, 7]);
    assert!(seeds(&literal).is_err(), "a literal longer than its config");
}

/// TransferCheckedWithFee stating the fee, a hook's resolved accounts, and
/// both, as @solana/spl-token builds them; each signed by the owner.
#[tokio::test]
async fn transfers_match_spl_token() {
    let fixture = fixture();
    let key = Ed25519Seed::from_hex(&text(&fixture["seed"])).unwrap();
    for (case, fee, hook) in [
        ("fee", true, false),
        ("hook", false, true),
        ("fee_and_hook", true, true),
    ] {
        let mut node = Node::new(&fixture);
        if fee {
            // 0.5% of 1 000 000 is the fixture's 5000.
            node = node.with_fee((0, 50, 1_000_000_000), (0, 50, 1_000_000_000));
        }
        if hook {
            node = node.with_hook(&fixture);
        }
        let prepared = prepare(&fixture, node).await.unwrap();
        assert_eq!(
            decompile(&prepared.message),
            expected(&fixture["cases"][case]),
            "{case}"
        );
        let token = prepared.token.as_ref().unwrap();
        assert_eq!(token.fee, fee.then_some(5000), "{case}");
        let terms = token.terms().unwrap();
        assert_eq!(
            (
                terms.debited.as_str(),
                terms.received.as_str(),
                terms.fee.as_str()
            ),
            (
                "1",
                if fee { "0.995" } else { "1" },
                if fee { "0.005" } else { "0" }
            ),
            "{case}"
        );
        assert_eq!(terms.hook_program.is_some(), hook);
        let signed = prepared.sign(&key).unwrap();
        ed25519_dalek::VerifyingKey::from_bytes(&prepared.payer)
            .unwrap()
            .verify_strict(
                &signed[65..],
                &ed25519_dalek::Signature::from_slice(&signed[1..65]).unwrap(),
            )
            .unwrap();
        assert_eq!(
            token
                .message(
                    &prepared.payer,
                    &crate::derivation::solana::decode_b58_32(&text(&fixture["recipient"]))
                        .unwrap(),
                    &prepared.blockhash
                )
                .unwrap(),
            prepared.message
        );
    }
}

/// A fee that changes in the epoch about to start is refused near the
/// boundary, and charged at the current epoch's rate otherwise.
#[tokio::test]
async fn a_fee_changing_at_the_next_epoch_is_refused_near_it() {
    let fixture = fixture();
    let changing = || Node::new(&fixture).with_fee((0, 50, u64::MAX), (701, 100, u64::MAX));
    let near = Node {
        epoch: (700, 431_800, 432_000),
        ..changing()
    };
    let error = prepare(&fixture, near).await.unwrap_err().to_string();
    assert!(error.contains("epoch about to start"), "{error}");
    let prepared = prepare(&fixture, changing()).await.unwrap();
    assert_eq!(prepared.token.unwrap().fee, Some(5000));
    let later = Node {
        epoch: (701, 1_000, 432_000),
        ..changing()
    };
    assert_eq!(
        prepare(&fixture, later).await.unwrap().token.unwrap().fee,
        Some(10_000)
    );
    // The same fee either side of the boundary is no reason to wait.
    let steady = Node {
        epoch: (700, 431_999, 432_000),
        ..Node::new(&fixture).with_fee((0, 50, u64::MAX), (701, 50, u64::MAX))
    };
    assert!(prepare(&fixture, steady).await.is_ok());
}

/// Every account the transfer touches is checked before anything is built.
#[tokio::test]
async fn accounts_the_network_would_refuse_are_refused_first() {
    let fixture = fixture();
    let base = || Node::new(&fixture);
    let account = |holder: &str, amount: u64, extensions: Value, state: &str| {
        Some(token_account(&fixture, holder, amount, extensions, state))
    };
    let cases: Vec<(Node, &str)> = vec![
        (
            Node {
                source: None,
                ..base()
            },
            "Insufficient token balance",
        ),
        (
            Node {
                source: account("owner", 999_999, json!([]), "initialized"),
                ..base()
            },
            "Insufficient token balance",
        ),
        (
            Node {
                source: account("owner", 5_000_000, json!([]), "frozen"),
                ..base()
            },
            "Your token account is frozen",
        ),
        (
            Node {
                source: account("recipient", 5_000_000, json!([]), "initialized"),
                ..base()
            },
            "another owner",
        ),
        (
            Node {
                destination: account("recipient", 0, json!([]), "frozen"),
                ..base()
            },
            "recipient's token account is frozen",
        ),
        (
            Node {
                destination: account(
                    "recipient",
                    0,
                    json!([{"extension": "memoTransfer", "state": {"requireIncomingTransferMemos": true}}]),
                    "initialized",
                ),
                ..base()
            },
            "memo",
        ),
        (
            Node {
                destination: account(
                    "recipient",
                    0,
                    json!([{"extension": "confidentialTransferAccount", "state": {"allowNonConfidentialCredits": false}}]),
                    "initialized",
                ),
                ..base()
            },
            "confidential",
        ),
        (
            Node {
                destination: account("owner", 0, json!([]), "initialized"),
                ..base()
            },
            "another owner",
        ),
        (
            Node {
                mint_extensions: vec![
                    json!({"extension": "defaultAccountState", "state": {"accountState": "frozen"}}),
                ],
                ..base()
            },
            "starts frozen",
        ),
    ];
    for (node, words) in cases {
        let error = prepare(&fixture, node).await.unwrap_err().to_string();
        assert!(error.contains(words), "{words}: {error}");
    }
    // A recipient with a thawed account receives even when new ones start frozen.
    let thawed = Node {
        mint_extensions: vec![
            json!({"extension": "defaultAccountState", "state": {"accountState": "frozen"}}),
        ],
        destination: account("recipient", 0, json!([]), "initialized"),
        ..base()
    };
    assert!(prepare(&fixture, thawed).await.is_ok());
}

/// A hook that would sign, has no validation account, keeps it under another
/// program, or refuses the transfer when it runs, is refused before signing.
#[tokio::test]
async fn hooks_that_cannot_run_as_reviewed_are_refused() {
    use base64::Engine;
    let fixture = fixture();
    let hooked = || Node::new(&fixture).with_hook(&fixture);
    let engine = base64::engine::general_purpose::STANDARD;
    let mut data = engine.decode(text(&fixture["validation_data"])).unwrap();
    // The first extra account asks to sign.
    data[8 + 4 + 4 + 33] = 1;
    let signing = Node {
        validation: Some((text(&fixture["hook_program"]), engine.encode(&data))),
        ..hooked()
    };
    let foreign = Node {
        validation: Some((
            text(&fixture["external_program"]),
            text(&fixture["validation_data"]),
        )),
        ..hooked()
    };
    let refusing = Node {
        simulation_error: Some(json!({"InstructionError": [1, {"Custom": 6000}]})),
        ..hooked()
    };
    for (node, words) in [
        (signing, "signature"),
        (
            Node {
                validation: None,
                ..hooked()
            },
            "no account listing",
        ),
        (foreign, "cannot read"),
        (refusing, "refused this transfer"),
    ] {
        let error = prepare(&fixture, node).await.unwrap_err().to_string();
        assert!(error.contains(words), "{words}: {error}");
    }
}
