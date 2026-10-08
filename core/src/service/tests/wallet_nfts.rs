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
