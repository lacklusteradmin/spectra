use super::*;
use crate::derivation::setup::WalletSetupMethod;
use crate::derivation::setup::tests::{fixture, service};

async fn import(
    service: &WalletService,
    chain: Chain,
    method: WalletSetupMethod,
    password: Option<&str>,
) -> WalletState {
    let mut commit = fixture(chain, method);
    commit.password = password.map(str::to_string);
    service.import_wallets(commit).await.unwrap().wallets[0]
        .to_wallet_state()
        .unwrap()
}

fn copy(source: &WalletState, chain: Chain, password: Option<&str>) -> WalletCopyCommit {
    WalletCopyCommit {
        source_wallet_id: source.id.clone(),
        chain,
        wallet_name: String::new(),
        password: password.map(str::to_string),
        derivation_path: None,
        restore_height: None,
        ton_wallet_version: None,
    }
}

/// The list is honest: for a source of every kind of secret — each phrase
/// format, each key scheme, a watched address and a watched account key —
/// every network listed takes the copy, and every other is refused.
#[tokio::test]
async fn every_listed_network_takes_the_copy_and_every_other_refuses_it() {
    let sources = [
        (Chain::Bitcoin, WalletSetupMethod::ImportPhrase),
        (Chain::Monero, WalletSetupMethod::ImportPhrase),
        (Chain::Ton, WalletSetupMethod::ImportPhrase),
        (Chain::Ethereum, WalletSetupMethod::ImportPrivateKey),
        (Chain::Solana, WalletSetupMethod::ImportPrivateKey),
        (Chain::Polkadot, WalletSetupMethod::ImportPrivateKey),
        (Chain::Cardano, WalletSetupMethod::ImportPrivateKey),
        (Chain::Ethereum, WalletSetupMethod::WatchAddresses),
        (Chain::BitcoinTestnet, WalletSetupMethod::WatchAccountXpub),
    ];
    for (chain, method) in sources {
        let (service, directory) = service().await;
        let source = import(&service, chain, method, None).await;
        let targets = service
            .wallet_copy_targets(source.id.clone())
            .await
            .unwrap();
        assert!(!targets.is_empty(), "{chain} {method:?}");
        assert!(!targets.contains(&chain), "{chain} {method:?}");
        for target in Chain::all() {
            let preview = service
                .preview_wallet_copy(copy(&source, target, None))
                .await;
            assert_eq!(
                preview.is_ok(),
                targets.contains(&target),
                "{chain} {method:?} → {target}: {preview:?}"
            );
        }
        drop(service);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[tokio::test]
async fn the_lists_follow_the_secret() {
    let (service, directory) = service().await;
    let targets = |chain, method| {
        let service = service.clone();
        async move {
            let source = import(&service, chain, method, None).await;
            let targets = service
                .wallet_copy_targets(source.id.clone())
                .await
                .unwrap();
            service
                .apply_state_command(StateCommand::RemoveWallet {
                    wallet_id: source.id,
                })
                .await
                .unwrap();
            targets
        }
    };
    // A phrase stays within its format.
    let monero = targets(Chain::Monero, WalletSetupMethod::ImportPhrase).await;
    assert!(monero.contains(&Chain::MoneroStagenet));
    assert!(
        monero
            .iter()
            .all(|chain| chain.mainnet_counterpart() == Chain::Monero)
    );
    let bip39 = targets(Chain::Ethereum, WalletSetupMethod::ImportPhrase).await;
    assert!(bip39.contains(&Chain::Bitcoin) && bip39.contains(&Chain::Solana));
    assert!(!bip39.contains(&Chain::Monero) && !bip39.contains(&Chain::Ton));
    // A key stays within its signature scheme.
    let secp = targets(Chain::Ethereum, WalletSetupMethod::ImportPrivateKey).await;
    assert!(secp.contains(&Chain::Arbitrum) && secp.contains(&Chain::Bitcoin));
    assert!(!secp.contains(&Chain::Solana) && !secp.contains(&Chain::Polkadot));
    // An EVM address is watched on the other EVM networks only.
    let watched = targets(Chain::Ethereum, WalletSetupMethod::WatchAddresses).await;
    assert!(watched.contains(&Chain::Base));
    assert!(watched.iter().all(|chain| chain.is_evm()));
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}

/// The copy signs with the same password as its source, holds the same
/// account where the networks share one, and keeps signing after the
/// source is deleted — and the source after the copy is.
#[tokio::test]
async fn a_copy_signs_and_stands_alone() {
    const PASSWORD: &str = "copy password";
    for method in [
        WalletSetupMethod::ImportPhrase,
        WalletSetupMethod::ImportPrivateKey,
    ] {
        let (service, directory) = service().await;
        let source = import(&service, Chain::Ethereum, method, Some(PASSWORD)).await;
        let outcome = service
            .copy_wallet_to_network(copy(&source, Chain::Arbitrum, Some(PASSWORD)))
            .await
            .unwrap();
        let copied = outcome.wallets[0].to_wallet_state().unwrap();
        assert_ne!(copied.id, source.id);
        assert_eq!(copied.chain_id, Chain::Arbitrum);
        assert!(copied.signing.requires_password(), "{method:?}");
        let address = |wallet: &WalletState, chain| {
            let service = service.clone();
            let id = wallet.id.clone();
            async move {
                service
                    .send_identity_address(id, chain, Some(PASSWORD.into()))
                    .await
            }
        };
        assert_eq!(
            address(&copied, Chain::Arbitrum).await.unwrap(),
            address(&source, Chain::Ethereum).await.unwrap()
        );
        service
            .apply_state_command(StateCommand::RemoveWallet {
                wallet_id: source.id.clone(),
            })
            .await
            .unwrap();
        assert!(
            address(&copied, Chain::Arbitrum).await.is_ok(),
            "{method:?}"
        );
        drop(service);
        std::fs::remove_dir_all(directory).unwrap();
    }
    // The other way round: the source outlives its copy.
    let (service, directory) = service().await;
    let source = import(
        &service,
        Chain::Solana,
        WalletSetupMethod::ImportPhrase,
        None,
    )
    .await;
    let copied = service
        .copy_wallet_to_network(copy(&source, Chain::Sui, None))
        .await
        .unwrap()
        .wallets[0]
        .id
        .clone();
    service
        .apply_state_command(StateCommand::RemoveWallet { wallet_id: copied })
        .await
        .unwrap();
    assert!(
        service
            .send_identity_address(source.id, Chain::Solana, None)
            .await
            .is_ok()
    );
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}

/// The source's seal is opened only with its password, and a refusal
/// stores nothing.
#[tokio::test]
async fn a_copy_needs_the_sources_password() {
    let (service, directory) = service().await;
    let source = import(
        &service,
        Chain::Ethereum,
        WalletSetupMethod::ImportPhrase,
        Some("right"),
    )
    .await;
    for password in [None, Some("wrong")] {
        assert!(
            service
                .copy_wallet_to_network(copy(&source, Chain::Base, password))
                .await
                .is_err()
        );
    }
    // A key does not become a key of another signature scheme.
    let key = import(
        &service,
        Chain::Bitcoin,
        WalletSetupMethod::ImportPrivateKey,
        None,
    )
    .await;
    assert!(
        service
            .copy_wallet_to_network(copy(&key, Chain::Solana, None))
            .await
            .is_err()
    );
    assert_eq!(service.app_state().await.wallets.len(), 2);
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}
