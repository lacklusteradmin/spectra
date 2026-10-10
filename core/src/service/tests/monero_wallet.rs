//! A Monero wallet's scan from its restore height, and the keys that never
//! leave the device: what the wallet reports before it has scanned, what a
//! sync refuses without sending anything but the daemon's own question, and
//! what a view key alone can do until its phrase is given.
use super::*;
use crate::derivation::import::{WalletImportCommit, WalletImportKind, WalletImportRequest};
use crate::store::secret_backends::InMemorySecretStore;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};

const PASSWORD: &str = "monero-password";

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/monero-subaddresses.json"
    ))
    .unwrap()
}

fn commit(chain: Chain, kind: WalletImportKind, name: &str) -> WalletImportCommit {
    WalletImportCommit {
        password: None,
        request: WalletImportRequest {
            wallet_name: name.into(),
            chain,
            kind,
        },
        derivation_path: None,
        derivation_overrides: Default::default(),
        seed_phrase: None,
        private_key: None,
        restore_height: None,
        named_account: None,
        ton_wallet_version: None,
        upgrade_wallet_id: None,
    }
}

/// The fixture's phrase as a Monero wallet, sealed under `PASSWORD`, to
/// scan from `restore_height`.
fn phrase(restore_height: u64) -> WalletImportCommit {
    let mut phrase = commit(Chain::Monero, WalletImportKind::Phrase, "Phrase");
    phrase.seed_phrase = Some(fixture()["phrase"].as_str().unwrap().into());
    phrase.password = Some(PASSWORD.into());
    phrase.restore_height = Some(restore_height);
    phrase
}

/// A service whose Monero daemon is `daemon`.
async fn service(daemon: &MockServer) -> Arc<WalletService> {
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: Chain::Monero,
        endpoints: vec![daemon.uri()],
    }])
    .unwrap();
    service.set_secret_store(Arc::new(InMemorySecretStore::new()));
    let database = std::env::temp_dir()
        .join(format!(
            "monero-wallet-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    service.open_state(database).await.unwrap();
    service
}

/// A synchronized stagenet daemon.
async fn stagenet_daemon() -> MockServer {
    let daemon = MockServer::start().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"nettype": "stagenet", "synchronized": true})),
        )
        .mount(&daemon)
        .await;
    daemon
}

/// Only a Monero wallet, or a Zcash wallet restored from its phrase, scans
/// from a restore height; any other import naming one is refused before
/// anything is stored.
#[tokio::test]
async fn only_a_scanning_wallet_takes_a_restore_height() {
    let daemon = MockServer::start().await;
    let service = service(&daemon).await;
    let mut bitcoin = commit(Chain::Bitcoin, WalletImportKind::Phrase, "Bitcoin");
    bitcoin.seed_phrase = Some(crate::derivation::phrase::test_phrase(Chain::Bitcoin).into());
    bitcoin.restore_height = Some(1);
    let refused = service.import_wallets(bitcoin).await.unwrap_err();
    assert!(
        refused.to_string().contains(
            "Only Monero wallets and Zcash wallets restored from a phrase take a restore height"
        ),
        "{refused}"
    );
    assert!(service.app_state().await.wallets.is_empty());
}

/// Before its first scan a wallet reports the restore height it will scan
/// from. A sync without the wallet's password, or against a daemon on
/// another network, refuses having asked the daemon nothing but `get_info`,
/// so no key reaches it, and leaves that report as it was.
#[tokio::test]
async fn a_sync_refused_sends_no_key_and_changes_nothing() {
    let daemon = stagenet_daemon().await;
    let service = service(&daemon).await;
    let wallet = service
        .import_wallets(phrase(3_000_000))
        .await
        .unwrap()
        .wallets
        .remove(0);
    assert_eq!(wallet.restore_height, Some(3_000_000));
    let status = || service.monero_sync_status(wallet.id.clone());
    let before = status().await.unwrap().unwrap();
    assert_eq!(
        (
            before.scanned_height,
            before.target_height,
            before.complete,
            before.spends_known
        ),
        (3_000_000, 0, false, true)
    );
    for password in [None, Some("wrong".to_string())] {
        let refused = service
            .sync_monero_wallet(wallet.id.clone(), password)
            .await
            .unwrap_err();
        assert!(
            refused.to_string().to_lowercase().contains("password"),
            "{refused}"
        );
    }
    assert!(daemon.received_requests().await.unwrap().is_empty());
    let refused = service
        .sync_monero_wallet(wallet.id.clone(), Some(PASSWORD.into()))
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("wrong network"), "{refused}");
    let asked: Vec<_> = daemon
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| {
            (
                request.method.to_string(),
                request.url.path().to_string(),
                request.body.clone(),
            )
        })
        .collect();
    assert_eq!(asked, [("POST".into(), "/get_info".into(), b"{}".to_vec())]);
    assert_eq!(
        serde_json::to_value(status().await.unwrap()).unwrap(),
        serde_json::to_value(Some(before)).unwrap()
    );
}

/// A phrase wallet and a view-only wallet both hand out the primary
/// address without the password: the view key is stored beside the
/// wallet. A view-only wallet knows nothing it spends, until its phrase
/// gives it its spend key in place, keeping its id and name.
#[tokio::test]
async fn a_view_key_receives_without_a_password_and_its_phrase_upgrades_it_in_place() {
    let daemon = MockServer::start().await;
    let service = service(&daemon).await;
    let fixture = fixture();
    let mainnet = &fixture["networks"][0];
    let primary = mainnet["primary"].as_str().unwrap();
    let held = service
        .import_wallets(phrase(3_000_000))
        .await
        .unwrap()
        .wallets
        .remove(0);
    assert_eq!(
        service
            .receive_address(held.id.clone(), Chain::Monero, true)
            .await
            .unwrap()
            .as_deref(),
        Some(primary)
    );
    service
        .apply_state_command(StateCommand::RemoveWallet { wallet_id: held.id })
        .await
        .unwrap();

    let mut view = commit(
        Chain::Monero,
        WalletImportKind::WatchViewKey {
            address: primary.into(),
            view_key: mainnet["private_view_key"].as_str().unwrap().into(),
        },
        "View",
    );
    view.restore_height = Some(3_000_000);
    let watched = service
        .import_wallets(view)
        .await
        .unwrap()
        .wallets
        .remove(0);
    assert!(watched.signing.is_watch_only());
    assert_eq!(
        service
            .receive_address(watched.id.clone(), Chain::Monero, true)
            .await
            .unwrap()
            .as_deref(),
        Some(primary)
    );
    let status = service
        .monero_sync_status(watched.id.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(!status.spends_known && status.used_subaddresses.is_empty());

    let outcome = service.import_wallets(phrase(3_000_000)).await.unwrap();
    assert!(outcome.upgraded);
    let upgraded = &outcome.wallets[0];
    assert_eq!(
        (upgraded.id.as_str(), upgraded.name.as_str()),
        (watched.id.as_str(), "View")
    );
    assert!(!upgraded.signing.is_watch_only());
    assert_eq!(service.app_state().await.wallets.len(), 1);
    assert!(
        service
            .monero_sync_status(watched.id)
            .await
            .unwrap()
            .unwrap()
            .spends_known
    );
    assert!(daemon.received_requests().await.unwrap().is_empty());
}
