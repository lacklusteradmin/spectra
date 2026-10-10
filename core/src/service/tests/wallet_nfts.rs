use super::*;
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const OWNER: &str = "0x1111111111111111111111111111111111111111";
const OTHER: &str = "0x2222222222222222222222222222222222222222";
const COLLECTION: &str = "0x3333333333333333333333333333333333333333";

fn word(value: impl std::fmt::LowerHex) -> String {
    format!("0x{value:064x}")
}

/// A node whose `eth_call` answers come from `answer(contract, selector,
/// argument words)`: `Some` result, or `None` for a revert. Single calls and
/// batches alike.
async fn contracts(
    answer: impl Fn(&str, &str, &str) -> Option<String> + Send + Sync + 'static,
) -> (MockServer, EvmClient) {
    let server = MockServer::start().await;
    let reply = move |call: &Value| {
        assert_eq!(call["method"], "eth_call", "{call}");
        let to = call["params"][0]["to"].as_str().unwrap();
        let data = call["params"][0]["data"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x");
        match answer(to, &data[..8], &data[8..]) {
            Some(result) => json!({"jsonrpc": "2.0", "id": call["id"], "result": result}),
            None => json!({"jsonrpc": "2.0", "id": call["id"],
                "error": {"code": 3, "message": "execution reverted"}}),
        }
    };
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: Value = request.body_json().unwrap();
            ResponseTemplate::new(200).set_body_json(match body.as_array() {
                Some(batch) => json!(batch.iter().map(&reply).collect::<Vec<_>>()),
                None => reply(&body),
            })
        })
        .mount(&server)
        .await;
    let client = EvmClient::new(Arc::new(vec![server.uri()]), 1);
    (server, client)
}

/// [`contracts`] for one contract.
async fn node(
    answer: impl Fn(&str, &str) -> Option<String> + Send + Sync + 'static,
) -> (MockServer, EvmClient) {
    contracts(move |_, selector, argument| answer(selector, argument)).await
}

/// A collection's ERC-165 answers, by interface id.
fn claims(erc721: bool, erc1155: bool, everything: bool) -> impl Fn(&str, &str) -> Option<String> {
    move |selector, argument| {
        assert_eq!(selector, "01ffc9a7");
        Some(word(u8::from(match &argument[..8] {
            "80ac58cd" => erc721,
            "d9b67a26" => erc1155,
            "ffffffff" => everything,
            other => panic!("interface {other}"),
        })))
    }
}

/// The standard is what the contract reports, and only that: a contract
/// that claims every interface or reverts is no collection, and one that
/// claims both is refused.
#[tokio::test]
async fn the_standard_is_the_contracts_answer() {
    let read = |answer| async move {
        let (_server, client) = node(answer).await;
        client.fetch_nft_standard(COLLECTION).await
    };
    assert_eq!(
        read(claims(true, false, false)).await.unwrap(),
        Some(NftStandard::Erc721)
    );
    assert_eq!(
        read(claims(false, true, false)).await.unwrap(),
        Some(NftStandard::Erc1155)
    );
    assert_eq!(read(claims(false, false, false)).await.unwrap(), None);
    assert_eq!(read(claims(true, true, true)).await.unwrap(), None);
    assert!(read(claims(true, true, false)).await.is_err());
    let (_server, reverts) = node(|_, _| None).await;
    assert_eq!(reverts.fetch_nft_standard(COLLECTION).await.unwrap(), None);
    // A non-boolean answer claims nothing.
    let (_server, garbage) = node(|_, _| Some(word(2u8))).await;
    assert_eq!(garbage.fetch_nft_standard(COLLECTION).await.unwrap(), None);
}

/// No fungible read scales an NFT collection: its metadata read is refused
/// even when the contract answers `decimals()`, while a token that does not
/// implement ERC-165 at all reads as before.
#[tokio::test]
async fn a_collection_is_never_read_as_a_fungible_token() {
    let fungible_or = |erc721: bool| {
        move |selector: &str, argument: &str| match selector {
            "313ce567" => Some(word(18u8)),
            "95d89b41" => Some(format!("0x{:0<64}", hex::encode("APE"))),
            "01ffc9a7" if erc721 => claims(true, false, false)(selector, argument),
            "01ffc9a7" => None,
            other => panic!("selector {other}"),
        }
    };
    let (_server, collection) = node(fungible_or(true)).await;
    let error = collection
        .fetch_erc20_metadata(COLLECTION)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("NFT collection"), "{error}");
    let (_server, token) = node(fungible_or(false)).await;
    let token = token.fetch_erc20_metadata(COLLECTION).await.unwrap();
    assert_eq!((token.decimals, token.symbol.as_str()), (18, "APE"));
}

/// A tracked token's balance read never scales an NFT count: a collection
/// that holds something is refused, while a fungible token and a collection
/// holding nothing (zero either way, and asked nothing more) read as before.
#[tokio::test]
async fn a_balance_read_never_counts_tokens_as_an_amount() {
    const TOKEN: &str = "0x4444444444444444444444444444444444444444";
    const EMPTY: &str = "0x5555555555555555555555555555555555555555";
    let asked = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = asked.clone();
    let (_server, client) = contracts(move |to, selector, argument| {
        if selector == "01ffc9a7" {
            seen.lock().unwrap().push(to.to_string());
            return match to {
                COLLECTION | EMPTY => claims(true, false, false)(selector, argument),
                _ => None,
            };
        }
        Some(match (to, selector) {
            (EMPTY, "70a08231") => word(0u8),
            (_, "70a08231") => word(3u8),
            (TOKEN, "313ce567") => word(6u8),
            (_, "313ce567") => word(0u8),
            other => panic!("call {other:?}"),
        })
    })
    .await;
    let reads = client
        .fetch_erc20_balances(OWNER, &[TOKEN.into(), COLLECTION.into(), EMPTY.into()])
        .await;
    assert_eq!(reads[0], Ok((3, 6)));
    assert!(
        reads[1]
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("NFT collection")
    );
    assert_eq!(reads[2], Ok((0, 0)));
    let mut asked = asked.lock().unwrap().clone();
    asked.sort();
    asked.dedup();
    assert_eq!(asked, [COLLECTION, TOKEN]);
}

/// An ERC-721 token is sent one at a time by its owner; an ERC-1155 id in
/// any whole quantity up to what the wallet holds, read live and compared
/// as whole numbers past u128.
#[tokio::test]
async fn only_what_the_wallet_holds_is_sent() {
    let owned_by = |holder: &'static str| {
        move |selector: &str, argument: &str| {
            assert_eq!(selector, "6352211e");
            match u64::from_str_radix(&argument[48..], 16).unwrap() {
                1234 => Some(format!("0x{:0>64}", &holder[2..])),
                _ => None,
            }
        }
    };
    let erc721 = |holder, id: &'static str, quantity: &'static str| async move {
        let (_server, client) = node(owned_by(holder)).await;
        validate_nft_holding(
            &client,
            NftStandard::Erc721,
            COLLECTION,
            OWNER,
            id,
            quantity,
        )
        .await
    };
    erc721(OWNER, "1234", "1").await.unwrap();
    for (holder, id, quantity, words) in [
        (OTHER, "1234", "1", "does not own token 1234"),
        (OWNER, "99", "1", "does not own token 99"),
        (OWNER, "1234", "2", "one at a time"),
        (OWNER, "1234", "0", "one at a time"),
    ] {
        let error = erc721(holder, id, quantity).await.unwrap_err().to_string();
        assert!(error.contains(words), "{words}: {error}");
    }

    let large = num_bigint::BigUint::from(1u8) << 200u32;
    let holds = |amount: num_bigint::BigUint| {
        move |selector: &str, argument: &str| {
            assert_eq!(selector, "00fdd58e");
            assert_eq!(&argument[24..64], &OWNER[2..]);
            Some(format!("0x{amount:064x}"))
        }
    };
    let erc1155 = |amount: num_bigint::BigUint, quantity: String| async move {
        let (_server, client) = node(holds(amount)).await;
        validate_nft_holding(
            &client,
            NftStandard::Erc1155,
            COLLECTION,
            OWNER,
            "7",
            &quantity,
        )
        .await
    };
    erc1155(5u8.into(), "5".into()).await.unwrap();
    erc1155(large.clone(), large.to_string()).await.unwrap();
    let error = erc1155(5u8.into(), "6".into())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("holds 5 of token 7, fewer than 6"),
        "{error}"
    );
    let more = (&large + 1u8).to_string();
    assert!(erc1155(large, more).await.is_err());
    let error = erc1155(5u8.into(), "0".into())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("at least one"), "{error}");
}

/// Neither a network without EVM tokens nor a wallet that is gone is asked
/// anything.
#[tokio::test]
async fn only_an_evm_wallet_holds_nfts() {
    let (service, directory) = crate::derivation::setup::tests::service().await;
    let wallet = service
        .import_wallets(crate::derivation::setup::tests::fixture(
            Chain::Solana,
            crate::derivation::setup::WalletSetupMethod::ImportPhrase,
        ))
        .await
        .unwrap()
        .wallets[0]
        .id
        .clone();
    let error = service.wallet_nfts(wallet.clone()).await.unwrap_err();
    assert!(error.to_string().contains("Only an EVM wallet"), "{error}");
    let error = service
        .build_nft_transfer(
            wallet,
            COLLECTION.into(),
            "1".into(),
            "1".into(),
            OTHER.into(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Only an EVM wallet"), "{error}");
    assert!(service.wallet_nfts("missing".into()).await.is_err());
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}

mod transfers {
    use super::*;
    use crate::send::stages::{SendArtifact, SendStage};
    use crate::service::send_stage_protocols::send_stage_support::{Secret, Wallet, node, refusal};
    use std::sync::Mutex;

    const KEY: &str = "0101010101010101010101010101010101010101010101010101010101010101";

    fn vectors() -> Value {
        serde_json::from_str::<Value>(include_str!(
            "../../../tests/fixtures/nft-transfer-vectors.json"
        ))
        .unwrap()["signed"]
            .clone()
    }

    fn text(value: &Value) -> String {
        value.as_str().unwrap().to_string()
    }

    fn abi_string(text: &str) -> String {
        format!("0x{:064x}{:064x}{:0<64}", 32, text.len(), hex::encode(text))
    }

    /// An Ethereum node where the wallet owns the ERC-721 collection's token
    /// 1234 and holds `items` of the ERC-1155 collection's id 7.
    async fn ethereum(items: Arc<Mutex<u64>>) -> wiremock::MockServer {
        let vectors = vectors();
        let apes = text(&vectors["erc721"]["to"]);
        let collection = text(&vectors["erc1155"]["to"]);
        let owner = text(&vectors["signer"]);
        node(move |_, body| {
            let answer = |call: &Value| {
                let params = &call["params"];
                let result = match call["method"].as_str().unwrap() {
                    "eth_call" => {
                        let to = params[0]["to"].as_str().unwrap().to_ascii_lowercase();
                        let data = params[0]["data"].as_str().unwrap();
                        let (selector, argument) = (&data[2..10], &data[10..]);
                        let number = |index: usize| {
                            u64::from_str_radix(&argument[64 * index + 48..64 * (index + 1)], 16)
                                .unwrap()
                        };
                        let answer = match selector {
                            "01ffc9a7" => Some(word(u8::from(matches!(
                                (to == apes, to == collection, &argument[..8]),
                                (true, _, "80ac58cd") | (_, true, "d9b67a26")
                            )))),
                            "6352211e" if to == apes && number(0) == 1234 => {
                                Some(format!("0x{:0>64}", &owner[2..]))
                            }
                            "00fdd58e" if to == collection && number(1) == 7 => {
                                Some(word(*items.lock().unwrap()))
                            }
                            "06fdde03" if to == apes => Some(abi_string("Apes")),
                            "95d89b41" if to == apes => Some(abi_string("APE")),
                            _ => None,
                        };
                        match answer {
                            Some(result) => json!(result),
                            None => {
                                return json!({"jsonrpc": "2.0", "id": call["id"],
                                "error": {"code": 3, "message": "execution reverted"}});
                            }
                        }
                    }
                    "eth_sendRawTransaction" => json!(format!("0x{}", "ab".repeat(32))),
                    "eth_chainId" => json!("0x1"),
                    "eth_getBalance" => json!(format!("0x{:x}", 10u128 * 10u128.pow(18))),
                    "eth_getCode" => json!("0x"),
                    "eth_getTransactionCount" => json!("0x3"),
                    "eth_estimateGas" => json!("0x15f90"),
                    "eth_blockNumber" => json!("0x20"),
                    "eth_feeHistory" => {
                        json!({"baseFeePerGas": ["0x3b9aca00"], "reward": [["0x77359400"]]})
                    }
                    other => panic!("unexpected request {other}"),
                };
                json!({"jsonrpc": "2.0", "id": call["id"], "result": result})
            };
            Some(match body.as_array() {
                Some(batch) => json!(batch.iter().map(answer).collect::<Vec<_>>()),
                None => answer(body),
            })
        })
        .await
    }

    async fn collector() -> (Wallet, wiremock::MockServer, Arc<Mutex<u64>>) {
        let items = Arc::new(Mutex::new(5));
        let server = ethereum(items.clone()).await;
        let wallet = Wallet::import(Chain::Ethereum, &server.uri(), Secret::Key(KEY)).await;
        assert_eq!(
            wallet.address.to_ascii_lowercase(),
            text(&vectors()["signer"])
        );
        (wallet, server, items)
    }

    async fn send_nft(
        wallet: &Wallet,
        contract: &str,
        token_id: &str,
        quantity: &str,
        to: &str,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        wallet
            .service
            .build_nft_transfer(
                wallet.id.clone(),
                contract.into(),
                token_id.into(),
                quantity.into(),
                to.into(),
            )
            .await
    }

    /// An NFT sent to its own wallet would cost a fee to move nothing, and
    /// one sent to the zero address is gone: both are refused before the
    /// network is asked anything, and nothing is stored.
    #[tokio::test]
    async fn an_nft_goes_neither_to_its_own_wallet_nor_to_the_zero_address() {
        let (wallet, server, _) = collector().await;
        let apes = text(&vectors()["erc721"]["to"]);
        let own = wallet.address.to_ascii_uppercase().replacen("0X", "0x", 1);
        for (to, words) in [
            (own.as_str(), "own address"),
            (ZERO_ADDRESS, "zero address"),
        ] {
            let error = refusal(send_nft(&wallet, &apes, "1234", "1", to).await);
            assert!(error.contains(words), "{words}: {error}");
        }
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(wallet.built_nothing().await);
    }

    /// An ERC-721 transfer is reviewed as the collection's token, at the
    /// most its gas can cost, and recorded once broadcast as that one token:
    /// its own asset, named by its collection and id, its quantity the
    /// amount.
    #[tokio::test]
    async fn an_nft_transfer_is_reviewed_and_recorded_as_its_collections_token() {
        let (wallet, _server, _) = collector().await;
        let vectors = vectors();
        let apes = text(&vectors["erc721"]["to"]);
        let recipient = text(&vectors["recipient"]);
        let built = send_nft(&wallet, &apes, "1234", "1", &recipient)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&built.operation).unwrap(),
            json!({"kind": "transfer_nft", "contract": apes, "standard": "ERC-721",
                "token_id": "1234", "quantity": "1", "collection": "Apes",
                "network_fee": "0.000432"})
        );
        assert_eq!(
            (
                built.recipient.as_str(),
                built.amount.as_str(),
                built.symbol.as_str(),
                built.asset.as_str()
            ),
            (recipient.as_str(), "1", "APE", apes.as_str())
        );
        let signed = wallet.sign(&built).await.unwrap();
        wallet.broadcast(&signed).await.unwrap();
        let record = wallet
            .service
            .fetch_all_history_records()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.id == built.id)
            .unwrap()
            .payload;
        assert_eq!(
            (
                record.deployment_id.as_deref(),
                record.asset_display_name.as_str(),
                record.symbol.as_str(),
                record.amount.as_str()
            ),
            (
                Some(format!("ethereum:erc-721:{apes}:1234").as_str()),
                "Apes #1234",
                "APE",
                "1"
            )
        );
    }

    /// The quantity of an ERC-1155 id is read again before signing: holding
    /// fewer than reviewed by then is refused, and leaves it unsigned.
    #[tokio::test]
    async fn an_erc1155_quantity_no_longer_held_at_signing_is_refused() {
        let (wallet, _server, items) = collector().await;
        let vectors = vectors();
        let built = send_nft(
            &wallet,
            &text(&vectors["erc1155"]["to"]),
            "7",
            "3",
            &text(&vectors["recipient"]),
        )
        .await
        .unwrap();
        *items.lock().unwrap() = 2;
        let error = refusal(wallet.sign(&built).await);
        assert!(
            error.contains("holds 2 of token 7, fewer than 3"),
            "{error}"
        );
        assert_eq!(wallet.stored(&built).await.stage, SendStage::Prepared);
        *items.lock().unwrap() = 5;
        assert_eq!(wallet.sign(&built).await.unwrap().stage, SendStage::Signed);
    }
}
