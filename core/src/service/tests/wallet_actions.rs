use super::*;
use crate::derivation::setup::tests::{fixture, service};
use crate::derivation::setup::{WalletSetupMethod, wallet_setup_descriptor};
use crate::service::SeedPhraseReveal;

/// Whether core performs `action` for the stored wallet, asked of the
/// operation itself rather than of the predicate that lists it. Nothing here
/// reaches a network.
async fn performs(service: &WalletService, wallet: &WalletState, action: WalletAction) -> bool {
    let id = wallet.id.clone();
    let chain = wallet.chain_id;
    match action {
        WalletAction::Send => service.send_identity_address(id, chain, None).await.is_ok(),
        WalletAction::Receive => {
            matches!(service.receive_address(id, chain, false).await, Ok(Some(_)))
        }
        WalletAction::Stake => service.staking_owner(&id, chain).await.is_ok(),
        WalletAction::ScanBlocks => {
            matches!(service.monero_sync_status(id).await, Ok(Some(_)))
                && !wallet.signing.is_watch_only()
        }
        // Some network takes the copy; the copy's own test asks every one.
        WalletAction::AddToNetwork => {
            let targets = service.wallet_copy_targets(id.clone()).await.unwrap();
            match targets.first() {
                Some(target) => service
                    .preview_wallet_copy(crate::service::WalletCopyCommit {
                        source_wallet_id: id,
                        chain: *target,
                        wallet_name: String::new(),
                        password: None,
                        derivation_path: None,
                        restore_height: None,
                        ton_wallet_version: None,
                    })
                    .await
                    .is_ok(),
                None => false,
            }
        }
        WalletAction::ExportKeys => {
            let kinds = service.wallet_key_exports(id.clone()).await.unwrap();
            let mut exported = !kinds.is_empty();
            for kind in kinds {
                exported &= service
                    .export_wallet_key(id.clone(), kind, None)
                    .await
                    .is_ok();
            }
            exported
        }
        // Listing reads the network, which a test has none of: only the
        // refusal of a one-address wallet is an answer here.
        WalletAction::Coins => !matches!(
            service.wallet_coins(id).await,
            Err(crate::SpectraBridgeError::InvalidInput { .. })
        ),
        // Listing reads an indexer; a test asks only whether a wallet is
        // refused as not an EVM one.
        WalletAction::TokenApprovals => {
            chain.is_evm()
                || !service
                    .wallet_token_approvals(id)
                    .await
                    .is_err_and(|error| error.to_string().contains("Only an EVM wallet"))
        }
        // The status is read from the wallet's own shielded database.
        WalletAction::ShieldedFunds => service.zcash_shielded_status(id).await.is_ok(),
        // The status is read from the wallet's own scan cache.
        WalletAction::MwebFunds => service.litecoin_mweb_status(id).await.is_ok(),
        // The inventory is an indexer's; a test asks only whether a wallet
        // is refused as not an EVM one.
        WalletAction::Nfts => {
            chain.is_evm()
                || !service
                    .wallet_nfts(id)
                    .await
                    .is_err_and(|error| error.to_string().contains("Only an EVM wallet"))
        }
        // The account is read from a node; a test asks only for the refusal
        // of a network whose account is a balance.
        WalletAction::NetworkAccount => !service
            .wallet_network_account(id)
            .await
            .is_err_and(|error| error.to_string().contains("holds only its balance")),
        WalletAction::TokenAccounts => !service
            .wallet_empty_token_accounts(id)
            .await
            .is_err_and(|error| error.to_string().contains("Only a Solana wallet")),
        // The lines are read from a node; a test asks only for the refusal
        // of a network that keeps none.
        WalletAction::TrustLines => {
            matches!(chain.mainnet_counterpart(), Chain::Xrp | Chain::Stellar)
                || !service
                    .wallet_trust_lines(id)
                    .await
                    .is_err_and(|error| error.to_string().contains("keeps trust lines"))
        }
        WalletAction::GetTestCoins => {
            crate::registry::chain_faucet_url(chain).is_some_and(|url| url.starts_with("https://"))
        }
        WalletAction::CoinObjects => !service
            .wallet_coin_objects(id)
            .await
            .is_err_and(|error| error.to_string().contains("Only a Sui wallet")),
        WalletAction::AccessKeys => !service
            .wallet_access_keys(id)
            .await
            .is_err_and(|error| error.to_string().contains("Only a NEAR account")),
        WalletAction::SignMessage => service
            .sign_wallet_message(id, "proof".into(), None)
            .await
            .is_ok(),
        // Checking needs only the address's scheme; signing it needs a key
        // the watched wallet does not have.
        WalletAction::VerifyMessage => {
            matches!(service.wallet_message_scheme(id.clone()).await, Ok(Some(_)))
                && service
                    .sign_wallet_message(id, "proof".into(), None)
                    .await
                    .is_err()
        }
        WalletAction::RevealPhrase => matches!(
            service.reveal_seed_phrase(id, None),
            Ok(SeedPhraseReveal::Phrase { .. })
        ),
        WalletAction::OpenInExplorer => wallet
            .address_on(chain)
            .and_then(|address| crate::address_explorer_link(chain, address.into()))
            .is_some_and(|link| link.url.contains(wallet.address_on(chain).unwrap())),
        // The phrase every fixture derives from holds the watched address
        // or account; the import bound to the wallet would upgrade it.
        WalletAction::AddKeys => {
            let mut commit = fixture(chain, WalletSetupMethod::ImportPhrase);
            commit.upgrade_wallet_id = Some(id);
            service
                .preview_wallet_import(commit)
                .await
                .is_ok_and(|preview| preview.upgrades_wallet.as_ref() == Some(&wallet.name))
        }
        // Reading a wallet's history is a query over what is stored; a
        // name and a deletion are state commands every wallet takes.
        WalletAction::History => true,
        WalletAction::Rename => service
            .apply_state_command(StateCommand::RenameWallet {
                wallet_id: id,
                name: "Renamed".into(),
            })
            .await
            .is_ok(),
        WalletAction::Delete => true,
    }
}

const ACTIONS: [WalletAction; 25] = [
    WalletAction::Send,
    WalletAction::Receive,
    WalletAction::History,
    WalletAction::OpenInExplorer,
    WalletAction::AddKeys,
    WalletAction::Stake,
    WalletAction::ScanBlocks,
    WalletAction::Coins,
    WalletAction::TokenApprovals,
    WalletAction::Nfts,
    WalletAction::ShieldedFunds,
    WalletAction::MwebFunds,
    WalletAction::NetworkAccount,
    WalletAction::AccessKeys,
    WalletAction::CoinObjects,
    WalletAction::TokenAccounts,
    WalletAction::TrustLines,
    WalletAction::GetTestCoins,
    WalletAction::SignMessage,
    WalletAction::VerifyMessage,
    WalletAction::AddToNetwork,
    WalletAction::Rename,
    WalletAction::RevealPhrase,
    WalletAction::ExportKeys,
    WalletAction::Delete,
];

/// Each action's place in [`ACTIONS`]. A new action does not compile here
/// until it has one, so the list cannot leave an action unchecked, as it
/// once left three.
fn position(action: WalletAction) -> usize {
    match action {
        WalletAction::Send => 0,
        WalletAction::Receive => 1,
        WalletAction::History => 2,
        WalletAction::OpenInExplorer => 3,
        WalletAction::AddKeys => 4,
        WalletAction::Stake => 5,
        WalletAction::ScanBlocks => 6,
        WalletAction::Coins => 7,
        WalletAction::TokenApprovals => 8,
        WalletAction::Nfts => 9,
        WalletAction::ShieldedFunds => 10,
        WalletAction::MwebFunds => 11,
        WalletAction::NetworkAccount => 12,
        WalletAction::AccessKeys => 13,
        WalletAction::CoinObjects => 14,
        WalletAction::TokenAccounts => 15,
        WalletAction::TrustLines => 16,
        WalletAction::GetTestCoins => 17,
        WalletAction::SignMessage => 18,
        WalletAction::VerifyMessage => 19,
        WalletAction::AddToNetwork => 20,
        WalletAction::Rename => 21,
        WalletAction::RevealPhrase => 22,
        WalletAction::ExportKeys => 23,
        WalletAction::Delete => 24,
    }
}

#[test]
fn the_checked_actions_are_every_action() {
    for (index, action) in ACTIONS.into_iter().enumerate() {
        assert_eq!(position(action), index, "{action:?}");
    }
}

/// The descriptor is honest: on every network and for every way a wallet
/// can be added there, each action it lists is one core performs for that
/// wallet, and each it leaves out is one core refuses.
#[tokio::test]
async fn every_listed_action_is_performed_and_every_other_refused() {
    for chain in Chain::all() {
        let (service, directory) = service().await;
        for option in wallet_setup_descriptor(chain).options {
            if option.method == WalletSetupMethod::CreatePhrase {
                continue;
            }
            let outcome = service
                .import_wallets(fixture(chain, option.method))
                .await
                .unwrap_or_else(|error| panic!("{chain} {:?}: {error}", option.method));
            let wallet = outcome.wallets[0].to_wallet_state().unwrap();
            let offered = service.wallet_actions(wallet.id.clone()).await.unwrap();
            assert_eq!(offered.chain, chain);
            assert_eq!(offered.summary.chain, chain);
            for action in ACTIONS {
                let listed = offered.actions.iter().any(|offer| offer.action == action);
                assert_eq!(
                    listed,
                    performs(&service, &wallet, action).await,
                    "{chain} {:?} {action:?}",
                    option.method
                );
            }
            service
                .apply_state_command(StateCommand::RemoveWallet {
                    wallet_id: wallet.id,
                })
                .await
                .unwrap();
        }
        drop(service);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

/// A watch-only wallet lists nothing that needs its key. Its empty token
/// accounts are read from its address; closing them is refused.
#[tokio::test]
async fn a_watched_wallet_lists_only_what_needs_no_key() {
    let (service, directory) = service().await;
    let outcome = service
        .import_wallets(fixture(Chain::Solana, WalletSetupMethod::WatchAddresses))
        .await
        .unwrap();
    let actions: Vec<_> = service
        .wallet_actions(outcome.wallets[0].id.clone())
        .await
        .unwrap()
        .actions
        .into_iter()
        .map(|offer| offer.action)
        .collect();
    assert_eq!(
        actions,
        [
            WalletAction::Receive,
            WalletAction::History,
            WalletAction::OpenInExplorer,
            WalletAction::AddKeys,
            WalletAction::Stake,
            WalletAction::TokenAccounts,
            WalletAction::VerifyMessage,
            WalletAction::AddToNetwork,
            WalletAction::Rename,
            WalletAction::Delete,
        ]
    );
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn a_removed_wallet_has_no_actions() {
    let (service, directory) = service().await;
    assert!(service.wallet_actions("missing".into()).await.is_err());
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn every_action_has_a_sentence_and_a_section() {
    for action in ACTIONS {
        let note = action.note();
        assert!(note.ends_with('.'), "{action:?}");
        assert!(note.starts_with(char::is_uppercase), "{action:?}");
        let _ = action.section();
    }
}

/// An import bound to a watched wallet gives that wallet its keys, keeping
/// its id and name, or is refused and stores nothing: a secret holding
/// another address, a watch, and a wallet that already signs.
#[tokio::test]
async fn a_bound_import_upgrades_its_wallet_or_is_refused() {
    let (service, directory) = service().await;
    let watched = service
        .import_wallets(fixture(Chain::Ethereum, WalletSetupMethod::WatchAddresses))
        .await
        .unwrap()
        .wallets[0]
        .clone();
    let bound = |phrase: &str| {
        let mut commit = fixture(Chain::Ethereum, WalletSetupMethod::ImportPhrase);
        commit.seed_phrase = Some(phrase.into());
        commit.upgrade_wallet_id = Some(watched.id.clone());
        commit
    };
    let other = "legal winner thank year wave sausage worth useful legal winner thank yellow";
    assert!(service.import_wallets(bound(other)).await.is_err());
    let mut watch = fixture(Chain::Ethereum, WalletSetupMethod::WatchAddresses);
    watch.upgrade_wallet_id = Some(watched.id.clone());
    assert!(service.import_wallets(watch).await.is_err());
    let state = service.app_state().await;
    assert_eq!(state.wallets.len(), 1);
    assert!(state.wallets[0].signing.is_watch_only());
    assert_eq!(
        service
            .wallet_upgrade_methods(watched.id.clone())
            .await
            .unwrap(),
        [
            WalletSetupMethod::ImportPhrase,
            WalletSetupMethod::ImportPrivateKey
        ]
    );

    let phrase = crate::derivation::setup::tests::phrase(Chain::Ethereum);
    let outcome = service.import_wallets(bound(phrase)).await.unwrap();
    assert!(outcome.upgraded);
    assert_eq!(outcome.wallets[0].id, watched.id);
    assert_eq!(outcome.wallets[0].name, watched.name);
    assert!(
        service
            .wallet_upgrade_methods(watched.id.clone())
            .await
            .unwrap()
            .is_empty()
    );
    // It signs now, so a second bound import has nothing to give it.
    assert!(service.import_wallets(bound(phrase)).await.is_err());
    assert_eq!(service.app_state().await.wallets.len(), 1);
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}
