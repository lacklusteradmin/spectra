use super::*;
use crate::derivation::import::{WalletImportCommit, WalletImportKind, WalletImportRequest};
use crate::derivation::setup::WalletSetupMethod;
use crate::derivation::setup::tests::{fixture, service};

const ABANDON: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

async fn import(service: &WalletService, commit: WalletImportCommit) -> WalletState {
    service.import_wallets(commit).await.unwrap().wallets[0]
        .to_wallet_state()
        .unwrap()
}

fn watch(chain: Chain, kind: WalletImportKind) -> WalletImportCommit {
    let mut commit = fixture(chain, WalletSetupMethod::WatchAddresses);
    commit.request = WalletImportRequest {
        wallet_name: String::new(),
        chain,
        kind,
    };
    commit
}

/// The address a stored wallet shows: its own, or its account key's first.
fn shown_address(wallet: &WalletState) -> String {
    match wallet.address_on(wallet.chain_id) {
        Some(address) => address.to_string(),
        None => crate::derivation::account_key::first_receive_address(
            wallet.chain_id,
            wallet.xpub.as_deref().unwrap(),
        )
        .unwrap(),
    }
}

/// On every network, each export a phrase or key wallet offers reads back
/// through Spectra's own import as the same wallet: the key as a key import
/// holding the same address, the account key as a watch whose first address
/// is the wallet's.
#[tokio::test]
async fn every_export_reads_back_as_the_same_wallet() {
    for chain in Chain::all() {
        for method in [
            WalletSetupMethod::ImportPhrase,
            WalletSetupMethod::ImportPrivateKey,
        ] {
            if crate::derivation::setup::wallet_setup_descriptor(chain)
                .option(method)
                .is_none()
            {
                continue;
            }
            let (source, source_directory) = service().await;
            let wallet = import(&source, fixture(chain, method)).await;
            for kind in source.wallet_key_exports(wallet.id.clone()).await.unwrap() {
                let export = source
                    .export_wallet_key(wallet.id.clone(), kind, None)
                    .await
                    .unwrap_or_else(|error| panic!("{chain} {method:?} {kind:?}: {error}"));
                let commit = match kind {
                    WalletKeyKind::PrivateKey => {
                        let mut commit = fixture(chain, WalletSetupMethod::ImportPrivateKey);
                        commit.private_key = Some(export.value.clone());
                        commit
                    }
                    WalletKeyKind::AccountPublicKey => watch(
                        chain,
                        WalletImportKind::WatchAccountXpub {
                            xpub: export.value.clone(),
                        },
                    ),
                    // Monero's keys are checked against its own wallet below.
                    WalletKeyKind::MoneroSpendKey | WalletKeyKind::MoneroViewKey => continue,
                };
                let (target, target_directory) = service().await;
                let restored = import(&target, commit).await;
                assert_eq!(
                    shown_address(&restored),
                    shown_address(&wallet),
                    "{chain} {method:?} {kind:?} {:?}",
                    export.format
                );
                drop(target);
                std::fs::remove_dir_all(target_directory).unwrap();
            }
            drop(source);
            std::fs::remove_dir_all(source_directory).unwrap();
        }
    }
}

/// Monero's keys are the ones monero-wallet-cli prints for the seed.
#[tokio::test]
async fn monero_exports_the_spend_and_view_keys_its_wallets_print() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/monero-phrases.json")).unwrap();
    let vector = &fixtures["electrum"][0];
    let (service, directory) = service().await;
    let mut commit = fixture(Chain::Monero, WalletSetupMethod::ImportPhrase);
    commit.seed_phrase = Some(vector["phrase"].as_str().unwrap().into());
    let wallet = import(&service, commit).await;
    for (kind, field) in [
        (WalletKeyKind::MoneroSpendKey, "spend_key"),
        (WalletKeyKind::MoneroViewKey, "view_key"),
    ] {
        let export = service
            .export_wallet_key(wallet.id.clone(), kind, None)
            .await
            .unwrap();
        assert_eq!(export.value, vector[field].as_str().unwrap(), "{kind:?}");
    }
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}

/// BIP-84's own test vector: the native SegWit account of its phrase.
#[tokio::test]
async fn a_native_segwit_account_exports_bip84s_zpub() {
    let (service, directory) = service().await;
    let mut commit = fixture(Chain::Bitcoin, WalletSetupMethod::ImportPhrase);
    commit.seed_phrase = Some(ABANDON.into());
    let wallet = import(&service, commit).await;
    let export = service
        .export_wallet_key(wallet.id.clone(), WalletKeyKind::AccountPublicKey, None)
        .await
        .unwrap();
    assert_eq!(
        export.value,
        "zpub6rFR7y4Q2AijBEqTUquhVz398htDFrtymD9xYYfG1m4wAcvPhXNfE3EfH1r1ADqtfSdVCToUG868RvUUkgDKf31mGDtKsAYz2oz2AGutZYs"
    );
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}

/// A phrase spread over many addresses has no one key; Taproot has no
/// account-key encoding; a Cardano phrase's base address needs its stake
/// key too; a watched address has nothing to export.
#[tokio::test]
async fn what_cannot_be_exported_is_not_offered_and_is_refused() {
    let (service, directory) = service().await;
    let bitcoin = import(
        &service,
        fixture(Chain::Bitcoin, WalletSetupMethod::ImportPhrase),
    )
    .await;
    assert_eq!(
        service
            .wallet_key_exports(bitcoin.id.clone())
            .await
            .unwrap(),
        [WalletKeyKind::AccountPublicKey]
    );
    assert!(
        service
            .export_wallet_key(bitcoin.id.clone(), WalletKeyKind::PrivateKey, None)
            .await
            .is_err()
    );
    let mut taproot = fixture(Chain::BitcoinTestnet, WalletSetupMethod::ImportPhrase);
    taproot.derivation_path = Some(
        crate::derivation::path::derivation_profile_path(
            Chain::BitcoinTestnet,
            crate::chains::DerivationProfile::Taproot,
            0,
        )
        .unwrap(),
    );
    let taproot = import(&service, taproot).await;
    assert!(
        service
            .wallet_key_exports(taproot.id)
            .await
            .unwrap()
            .is_empty()
    );
    let cardano = import(
        &service,
        fixture(Chain::Cardano, WalletSetupMethod::ImportPhrase),
    )
    .await;
    assert!(
        service
            .wallet_key_exports(cardano.id.clone())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        service
            .export_wallet_key(cardano.id, WalletKeyKind::PrivateKey, None)
            .await
            .is_err()
    );
    let watched = import(
        &service,
        fixture(Chain::Solana, WalletSetupMethod::WatchAddresses),
    )
    .await;
    assert!(
        service
            .wallet_key_exports(watched.id)
            .await
            .unwrap()
            .is_empty()
    );
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}

/// The seal opens only with the wallet's password.
#[tokio::test]
async fn an_export_needs_the_wallets_password() {
    let (service, directory) = service().await;
    let mut commit = fixture(Chain::Solana, WalletSetupMethod::ImportPhrase);
    commit.password = Some("right password".into());
    let wallet = import(&service, commit).await;
    for password in [None, Some("wrong".to_string())] {
        assert!(
            service
                .export_wallet_key(wallet.id.clone(), WalletKeyKind::PrivateKey, password)
                .await
                .is_err()
        );
    }
    let export = service
        .export_wallet_key(
            wallet.id.clone(),
            WalletKeyKind::PrivateKey,
            Some("right password".into()),
        )
        .await
        .unwrap();
    assert_eq!(export.format, WalletSecretFormat::SolanaKeypair);
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}

/// A Polkadot or Bittensor phrase imports along a Substrate junction path
/// to polkadot.js's account for it (`substrate-paths.json`): the preview,
/// the stored path, the signing identity and a message signature are all
/// that account's. A hard path exports a seed that imports back as the same
/// wallet; a soft junction's key has no seed, so it offers none.
#[tokio::test]
async fn a_substrate_phrase_derives_along_its_junction_path() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/substrate-paths.json")).unwrap();
    for chain in Chain::all().filter(|chain| chain.derives_along_junctions()) {
        let field = if chain == Chain::Polkadot {
            "polkadot"
        } else {
            "substrate"
        };
        for vector in fixtures["vectors"].as_array().unwrap() {
            let path = vector["path"].as_str().unwrap();
            if path.is_empty() {
                continue;
            }
            let expected = vector[field].as_str().unwrap();
            let (source, directory) = service().await;
            let mut commit =
                crate::derivation::setup::tests::fixture(chain, WalletSetupMethod::ImportPhrase);
            commit.seed_phrase = Some(fixtures["phrase"].as_str().unwrap().into());
            commit.derivation_path = Some(format!(" {path} "));
            let passphrase = vector["passphrase"].as_str().unwrap();
            if !passphrase.is_empty() {
                commit.derivation_overrides.passphrase = Some(passphrase.into());
            }
            let preview = source.preview_wallet_import(commit.clone()).await.unwrap();
            assert_eq!(preview.addresses, [expected], "{chain} {path:?}");
            let wallet = import(&source, commit).await;
            assert_eq!(wallet.derivation_path.as_deref(), Some(path), "{chain}");
            assert_eq!(wallet.address_on(chain), Some(expected), "{chain} {path:?}");
            assert_eq!(
                source
                    .send_identity_address(wallet.id.clone(), chain, None)
                    .await
                    .unwrap(),
                expected,
                "{chain} {path:?}"
            );
            let signed = source
                .sign_wallet_message(wallet.id.clone(), "Spectra".into(), None)
                .await
                .unwrap();
            assert!(
                crate::send::message::verify_message(
                    chain,
                    expected.into(),
                    "Spectra".into(),
                    signed.signature
                ),
                "{chain} {path:?}"
            );
            let exports = source.wallet_key_exports(wallet.id.clone()).await.unwrap();
            if vector["hardOnly"].as_bool().unwrap() {
                assert_eq!(exports, [WalletKeyKind::PrivateKey], "{chain} {path:?}");
                let export = source
                    .export_wallet_key(wallet.id.clone(), WalletKeyKind::PrivateKey, None)
                    .await
                    .unwrap();
                let (target, target_directory) = service().await;
                let mut key = crate::derivation::setup::tests::fixture(
                    chain,
                    WalletSetupMethod::ImportPrivateKey,
                );
                key.private_key = Some(export.value);
                assert_eq!(
                    import(&target, key).await.address_on(chain),
                    Some(expected),
                    "{chain} {path:?}"
                );
                drop(target);
                std::fs::remove_dir_all(target_directory).unwrap();
            } else {
                assert!(exports.is_empty(), "{chain} {path:?}");
                assert!(
                    source
                        .export_wallet_key(wallet.id.clone(), WalletKeyKind::PrivateKey, None)
                        .await
                        .is_err()
                );
            }
            drop(source);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }
}

/// A path the two Substrate implementations read differently, a BIP-32
/// path and a secret URI's `///password` are refused before anything is
/// stored; so is a Polkadot HMAC override that would change nothing.
#[tokio::test]
async fn a_substrate_import_refuses_paths_it_cannot_read_one_way() {
    let (service, directory) = service().await;
    for path in [
        "//0x1234",
        "m/44'/354'/0'",
        "//polkadot///secret",
        "polkadot",
    ] {
        let mut commit = crate::derivation::setup::tests::fixture(
            Chain::Polkadot,
            WalletSetupMethod::ImportPhrase,
        );
        commit.derivation_path = Some(path.into());
        assert!(service.import_wallets(commit).await.is_err(), "{path:?}");
    }
    let mut commit =
        crate::derivation::setup::tests::fixture(Chain::Polkadot, WalletSetupMethod::ImportPhrase);
    commit.derivation_overrides.hmac_key = Some("Bitcoin seed".into());
    assert!(service.import_wallets(commit).await.is_err());
    assert!(service.app_state().await.wallets.is_empty());
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}
