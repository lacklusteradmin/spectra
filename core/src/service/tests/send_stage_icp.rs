//! An ICP transfer is built only at the ledger's own fee, to an account
//! identifier whose checksum holds.
use super::*;
use crate::send::SendExecutionRequest;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::state::WalletState;
use crate::store::wallet_secrets::store_seed_phrase;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

#[tokio::test]
async fn an_icp_transfer_is_built_only_at_the_ledger_fee_to_a_checksummed_account() {
    let suggested_fee = Arc::new(AtomicU64::new(10_001));
    let quoted = suggested_fee.clone();
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body = match request.url.path() {
                "/network/list" => json!({"network_identifiers": [
                    crate::api::icp_rosetta::network_identifier()
                ]}),
                "/construction/preprocess" => {
                    json!({"options": {"request_types": ["TRANSACTION"]}})
                }
                "/construction/metadata" => json!({"suggested_fee": [{
                    "value": quoted.load(Ordering::SeqCst).to_string(),
                    "currency": {"symbol": "ICP", "decimals": 8},
                }]}),
                path => panic!("unexpected Rosetta request {path}"),
            };
            ResponseTemplate::new(200).set_body_json(body)
        })
        .mount(&server)
        .await;
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: Chain::Icp,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    let database = std::env::temp_dir()
        .join(format!(
            "icp-stages-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    service.open_state(database).await.unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    store_seed_phrase(&*secrets, "w", SEED, None).unwrap();
    service.set_secret_store(secrets);
    let path = crate::derivation::path::default_path_from_catalog(Chain::Icp).unwrap();
    let address = crate::derivation::dispatch::derive_for_chain(
        Chain::Icp,
        SEED,
        &path,
        None,
        None,
        None,
        true,
        false,
        false,
    )
    .unwrap()
    .address
    .unwrap();
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: WalletState::single_address(
                "w",
                "ICP",
                Chain::Icp,
                address.clone(),
                Some(path),
                false,
            ),
        })
        .await
        .unwrap();
    let request = |to: &str| SendExecutionRequest {
        token_standard: None,
        chain_id: Chain::Icp,
        wallet_id: "w".into(),
        password: None,
        to_address: to.into(),
        amount_str: "0.001".into(),
        contract_address: None,
        token_decimals: None,
        fee_rate_svb: None,
        fee_sat: None,
        gas_budget: None,
        fee_amount: None,
        evm_overrides: None,
        sign_only: false,
        memo: None,
    };

    let refused = service.build_send(request(&address)).await.unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("ICP ledger fee changed or is unsupported"),
        "{refused}"
    );
    // Thirty-two zero bytes: the checksum their last 28 need is not zero.
    let read = server.received_requests().await.unwrap().len();
    let refused = service
        .build_send(request(&"00".repeat(32)))
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("Invalid destination for selected network"),
        "{refused}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), read);
    assert!(service.list_sends().await.unwrap().is_empty());

    suggested_fee.store(10_000, Ordering::SeqCst);
    let built = service.build_send(request(&address)).await.unwrap();
    let details: Value = serde_json::from_str(&built.prepared_details).unwrap();
    assert_eq!(details["Icp"]["fee"], 10_000);
}
