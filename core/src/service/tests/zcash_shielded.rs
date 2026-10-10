use super::*;
use crate::derivation::setup::WalletSetupMethod;
use crate::derivation::setup::tests::{fixture, service};
use zcash_client_backend::data_api::chain::ChainState;
use zcash_primitives::block::BlockHash;

/// The wallet one import of the shared fixture stores.
async fn imported(service: &WalletService, chain: Chain, method: WalletSetupMethod) -> WalletState {
    let outcome = service
        .import_wallets(fixture(chain, method))
        .await
        .unwrap();
    outcome.wallets[0].to_wallet_state().unwrap()
}

/// `wallet` as if it had been imported at `path`.
fn at_path(wallet: &WalletState, path: &str) -> WalletState {
    let mut wallet = wallet.clone();
    wallet.derivation_path = Some(path.into());
    for address in &mut wallet.addresses {
        address.derivation_path = Some(path.into());
    }
    wallet
}

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/zcash-addresses.json")).unwrap()
}

/// The shielded account is the one the wallet's standard path names, so its
/// transparent receiver is the wallet's address; anything else has none.
#[tokio::test]
async fn the_shielded_account_is_the_one_the_standard_path_names() {
    let (service, directory) = service().await;
    let wallet = imported(&service, Chain::Zcash, WalletSetupMethod::ImportPhrase).await;
    assert_eq!(shielded_account(&wallet).unwrap(), 0);
    assert_eq!(
        shielded_account(&at_path(&wallet, "m/44'/133'/5'/0/0")).unwrap(),
        5
    );
    for path in [
        "m/44'/133'/0'/0/1",
        "m/44'/133'/0'/1/0",
        "m/44'/133'/0/0/0",
        "m/84'/133'/0'/0/0",
        "m/44'/1'/0'/0/0",
        "m/44'/133'/2147483648'/0/0",
        "m/44'/133'/0'/0",
    ] {
        assert!(shielded_account(&at_path(&wallet, path)).is_err(), "{path}");
    }
    let testnet = imported(
        &service,
        Chain::ZcashTestnet,
        WalletSetupMethod::ImportPhrase,
    )
    .await;
    assert_eq!(shielded_account(&testnet).unwrap(), 0);
    assert!(shielded_account(&at_path(&testnet, "m/44'/133'/0'/0/0")).is_err());

    let mut custom = wallet.clone();
    custom.derivation_overrides.hmac_key = Some("Spectra".into());
    assert!(shielded_account(&custom).is_err());
    let bitcoin = imported(&service, Chain::Bitcoin, WalletSetupMethod::ImportPhrase).await;
    assert_eq!(
        shielded_account(&bitcoin).unwrap_err().to_string(),
        "Only a Zcash wallet holds shielded funds."
    );
    std::fs::remove_dir_all(directory).unwrap();

    // The same address from its key, or watched: no seed, no account.
    for method in [
        WalletSetupMethod::ImportPrivateKey,
        WalletSetupMethod::WatchAddresses,
    ] {
        let (service, directory) = crate::derivation::setup::tests::service().await;
        let wallet = imported(&service, Chain::Zcash, method).await;
        assert_eq!(
            shielded_account(&wallet).unwrap_err().to_string(),
            "Shielded funds need a Zcash wallet restored from its seed phrase."
        );
        assert!(
            service
                .zcash_shielded_status(wallet.id.clone())
                .await
                .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}

/// Before its first sync a wallet has no shielded account: nothing is
/// spendable, the scan has not begun at its restore height, and nothing
/// counts toward its balance.
#[tokio::test]
async fn an_unsynced_wallet_holds_nothing_shielded() {
    let (service, directory) = service().await;
    let wallet = imported(&service, Chain::Zcash, WalletSetupMethod::ImportPhrase).await;
    // Restored with no height: from Sapling's activation, the first block
    // a ZIP-32 key could receive at.
    assert_eq!(wallet.restore_height, Some(419_200));
    let status = service
        .zcash_shielded_status(wallet.id.clone())
        .await
        .unwrap();
    assert_eq!(
        status,
        ZcashShieldedStatus {
            wallet_id: wallet.id.clone(),
            ready: false,
            restore_height: 419_200,
            scanned_height: 419_199,
            chain_tip_height: 0,
            progress_permille: 0,
            unread_transactions: 0,
            complete: false,
            spendable: "0".into(),
            pending: "0".into(),
            shieldable: "0".into(),
            address: None,
        }
    );
    assert_eq!(
        service.zcash_shielded_total(&wallet.id).await.unwrap(),
        None
    );
    std::fs::remove_dir_all(directory).unwrap();
}

/// The address a synced account shows is the reference implementation's for
/// its seed and account: Orchard and Sapling receivers, no transparent one.
#[test]
fn an_account_receives_at_its_reference_unified_address() {
    let vectors = vectors();
    let vector = &vectors["unified"]["vectors"][3];
    assert!(vector["p2pkh_bytes"].is_null() && vector["diversifier_index"] == 0);
    let network = Network::MainNetwork;
    let directory = std::env::temp_dir().join(crate::store::new_event_id());
    let path = directory.join("wallet.sqlite");
    let mut db = open_zcash_db(&path, network).unwrap();
    assert_eq!(unified_address(&db, &network).unwrap(), None);
    let seed = hex::decode(vector["root_seed"].as_str().unwrap()).unwrap();
    let account = u32::try_from(vector["account"].as_u64().unwrap()).unwrap();
    db.import_account_hd(
        "Spectra",
        &secrecy::SecretVec::new(seed),
        zip32::AccountId::try_from(account).unwrap(),
        &AccountBirthday::from_parts(
            ChainState::empty(BlockHeight::from_u32(3_430_000), BlockHash([0; 32])),
            None,
        ),
        None,
    )
    .unwrap();
    assert_eq!(
        unified_address(&db, &network).unwrap().as_deref(),
        vector["unified_addr"].as_str()
    );
    let status = status_of(&db, &network, "wallet", 3_430_001).unwrap();
    assert!(status.ready);
    assert_eq!(status.address.as_deref(), vector["unified_addr"].as_str());
    assert_eq!(
        (status.spendable.as_str(), status.pending.as_str()),
        ("0", "0")
    );
    assert!(shielded_history(&path, Chain::Zcash).unwrap().is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

/// With no lightwalletd server among the network's endpoints there is
/// nothing to scan from: the sync is refused and the wallet stays unsynced.
#[tokio::test]
async fn a_sync_with_no_lightwalletd_server_is_refused() {
    let service = crate::service::loopback_service::open().await;
    let wallet = service
        .import(fixture(Chain::Zcash, WalletSetupMethod::ImportPhrase))
        .await;
    service
        .apply_state_command(StateCommand::SetAppSetting {
            update: crate::store::state::AppSettingUpdate::CustomEndpointsOnly {
                chain_id: Chain::Zcash,
                value: true,
            },
        })
        .await
        .unwrap();
    let refusal = service
        .sync_zcash_shielded(wallet.clone(), None)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(
        refusal,
        "Zcash needs a lightwalletd server to scan shielded funds. Add one for it."
    );
    assert!(!service.zcash_shielded_status(wallet).await.unwrap().ready);
}
