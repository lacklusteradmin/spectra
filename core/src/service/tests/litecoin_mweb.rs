//! What a Litecoin wallet's MWEB funds refuse before any node is read.

use super::*;
use crate::derivation::setup::WalletSetupMethod;

async fn litecoin_wallet(
    service: &crate::service::loopback_service::OpenService,
    profile: crate::chains::DerivationProfile,
) -> String {
    let mut commit =
        crate::derivation::setup::tests::fixture(Chain::Litecoin, WalletSetupMethod::ImportPhrase);
    commit.derivation_path = Some(
        crate::derivation::path::derivation_profile_path(Chain::Litecoin, profile, 0).unwrap(),
    );
    service.import(commit).await
}

fn only_mine(chain: Chain) -> StateCommand {
    StateCommand::SetAppSetting {
        update: crate::store::state::AppSettingUpdate::CustomEndpointsOnly {
            chain_id: chain,
            value: true,
        },
    }
}

/// With no Litecoin node among the network's endpoints there is nothing to
/// scan MWEB funds from.
#[tokio::test]
async fn a_sync_with_no_litecoin_node_is_refused() {
    let service = crate::service::loopback_service::open().await;
    let wallet = litecoin_wallet(&service, crate::chains::DerivationProfile::Legacy).await;
    service
        .apply_state_command(only_mine(Chain::Litecoin))
        .await
        .unwrap();
    let refusal = service
        .sync_litecoin_mweb(wallet.clone(), None)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(
        refusal,
        "Litecoin needs a Litecoin node to scan MWEB funds. Add one for it."
    );
    assert!(!service.litecoin_mweb_status(wallet).await.unwrap().ready);
}

/// Before its first sync a wallet holds no MWEB funds and has no MWEB
/// address, and neither pays from them nor moves funds into them; nothing
/// is stored.
#[tokio::test]
async fn an_unsynced_wallet_holds_no_mweb_funds_and_moves_none() {
    let service = crate::service::loopback_service::open().await;
    let wallet = litecoin_wallet(&service, crate::chains::DerivationProfile::Legacy).await;
    let status = service.litecoin_mweb_status(wallet.clone()).await.unwrap();
    assert_eq!(
        (
            status.ready,
            status.scanned_height,
            status.complete,
            status.spendable.as_str(),
            status.pending.as_str(),
            status.address
        ),
        (false, 0, false, "0", "0", None)
    );
    let recipient = service.address(&wallet, Chain::Litecoin).await;
    let refusal = service
        .build_litecoin_mweb_send(wallet.clone(), recipient, "0.1".into())
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(refusal, "Sync the MWEB funds before sending from them.");
    let refusal = service
        .build_litecoin_mweb_pegin(wallet, "0.1".into())
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(
        refusal,
        "Sync the MWEB funds before moving funds into them."
    );
    assert!(service.list_sends().await.unwrap().is_empty());
}

/// One wallet of a phrase holds its MWEB funds: once a wallet's first sync
/// has claimed the phrase's keys, another wallet of the same phrase, at
/// another path, is refused them.
#[tokio::test]
async fn a_second_wallet_of_one_phrase_is_refused_its_mweb_funds() {
    let service = crate::service::loopback_service::open().await;
    let first = litecoin_wallet(&service, crate::chains::DerivationProfile::Legacy).await;
    let second = litecoin_wallet(&service, crate::chains::DerivationProfile::NativeSegWit).await;
    // A node that is not there: the first sync claims the keys, then fails
    // to connect.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    service
        .use_endpoint(
            Chain::Litecoin,
            crate::EndpointApi::LitecoinP2p,
            &[EndpointCapability::History],
            &format!("tcp://127.0.0.1:{closed}"),
        )
        .await;
    let unreachable = service
        .sync_litecoin_mweb(first.clone(), None)
        .await
        .unwrap_err()
        .to_string();
    assert!(!unreachable.contains("already holds"), "{unreachable}");
    let name = service
        .app_state()
        .await
        .wallets
        .into_iter()
        .find(|w| w.id == first)
        .unwrap()
        .name;
    let refusal = service
        .sync_litecoin_mweb(second, None)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(
        refusal,
        format!("Another wallet, {name}, already holds this phrase's MWEB funds.")
    );
}
