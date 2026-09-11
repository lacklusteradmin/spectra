use super::*;
use crate::derivation::types::BitcoinScriptType;
use crate::store::{
    secret_backends::InMemorySecretStore, state::WalletSummary, wallet_secrets::store_seed_phrase,
};
use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

fn tx(i: u64, address: &str) -> serde_json::Value {
    json!({"txid":format!("{i:064x}"),"vin":[],"vout":[{"scriptpubkey_address":address,"value":100}],
        "fee":1,"status":{"confirmed":true,"block_height":i,"block_time":1700000000+i}})
}

#[tokio::test]
async fn stored_testnet_hd_history_uses_its_network_script_and_all_pages() {
    use crate::derivation::chains::bitcoin::{derive_from_seed_phrase, BTC_TESTNET};
    let path = "m/84'/1'/0'/0/0";
    let address = derive_from_seed_phrase(
        BTC_TESTNET,
        BitcoinScriptType::P2wpkh,
        SEED,
        path,
        None,
        true,
        false,
        false,
    )
    .unwrap()
    .0
    .unwrap();
    let server = MockServer::start().await;
    let funded = address.clone();
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let parts: Vec<_> = request.url.path().split('/').collect();
            assert_eq!(parts[1], "address");
            assert!(
                parts[2].starts_with("tb1q"),
                "HD address must use testnet and BIP84"
            );
            let rows = if parts[2] == funded {
                let upper = if parts.len() == 6 {
                    u64::from_str_radix(parts[5], 16).unwrap() - 1
                } else {
                    61
                };
                (1..=upper)
                    .rev()
                    .take(25)
                    .map(|i| tx(i, &funded))
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };
            ResponseTemplate::new(200).set_body_json(rows)
        })
        .mount(&server)
        .await;
    let service = WalletService::new_typed(vec![
        ChainEndpoints {
            chain_id: Chain::BitcoinTestnet4.str_id().into(),
            endpoints: vec![server.uri()],
            api_key: None,
        },
        ChainEndpoints {
            chain_id: "bitcoin".into(),
            endpoints: vec![format!("{}/WRONG-NETWORK", server.uri())],
            api_key: None,
        },
    ])
    .unwrap();
    let db = std::env::temp_dir().join(format!(
        "spectra-hd-pager-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(db.to_string_lossy().into_owned())
        .await
        .unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    store_seed_phrase(&*secrets, "w", SEED, None).unwrap();
    service.set_secret_store(secrets);
    let mut wallet =
        WalletSummary::single_address("w", "Test", "Bitcoin", &address, Some(path.into()), false);
    wallet.network_mode = Some(Chain::BitcoinTestnet4.str_id().into());
    wallet.addresses[0].chain_name = Chain::BitcoinTestnet4.chain_display_name().into();
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet })
        .await
        .unwrap();
    let mut total = 0;
    for page in 0..7 {
        let result = service
            .refresh_bitcoin_history(vec!["w".into()], page > 0, Some(10))
            .await
            .unwrap();
        assert_eq!(result.wallets_failed, 0, "{:?}", result.diagnostics);
        assert_eq!(result.updated, 0);
        total += result.added;
        assert_eq!(result.exhausted, page == 6);
    }
    assert_eq!(total, 61);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        32,
        "30 initial address reads, two provider continuation pages"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().contains("/txs/chain/"))
            .count(),
        2
    );
}

#[test]
fn hd_history_addresses_match_individual_derivation_for_every_supported_purpose() {
    use crate::derivation::{chains::bitcoin::*, xpub_walker::*};
    for (purpose, hd_script, script) in [
        (44, HdScriptType::P2pkh, BitcoinScriptType::P2pkh),
        (49, HdScriptType::P2shP2wpkh, BitcoinScriptType::P2shP2wpkh),
        (84, HdScriptType::P2wpkh, BitcoinScriptType::P2wpkh),
        (86, HdScriptType::P2tr, BitcoinScriptType::P2tr),
    ] {
        for (coin, network, params) in [
            (0, HdNetwork::Mainnet, BTC_MAINNET),
            (1, HdNetwork::Testnet, BTC_TESTNET),
        ] {
            let account = format!("m/{purpose}'/{coin}'/0'");
            let xpub = derive_account_xpub(SEED, "", &account).unwrap();
            let actual =
                derive_children_on_network(&xpub, 0, 0, 1, network, Some(hd_script)).unwrap();
            let expected = derive_from_seed_phrase(
                params,
                script,
                SEED,
                &format!("{account}/0/0"),
                None,
                true,
                false,
                false,
            )
            .unwrap()
            .0
            .unwrap();
            assert_eq!(actual[0].address, expected);
        }
    }
}
