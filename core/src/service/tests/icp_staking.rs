//! Building ICP neuron operations against a replica that answers with the
//! DFINITY SDK's encoded governance replies and a Rosetta node holding the
//! wallet's balance: what a stake refuses, the calls it reviews, and which
//! neurons the wallet may act on.

use super::*;
use crate::derivation::setup::WalletSetupMethod;
use crate::send::icp_staking::IcpStakingCallKind;
use crate::send::stages::SendArtifact;
use serde_json::{Value, json};
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/icp-staking-vectors.json"
    ))
    .unwrap()
}

/// The method an IC query envelope calls.
fn query_method(body: &[u8]) -> String {
    let mut value: ciborium::Value = ciborium::from_reader(body).unwrap();
    // The self-describing CBOR tag.
    if let ciborium::Value::Tag(_, inner) = value {
        value = *inner;
    }
    let field = |value: &ciborium::Value, key: &str| {
        value
            .as_map()
            .unwrap()
            .iter()
            .find(|(name, _)| name.as_text() == Some(key))
            .unwrap()
            .1
            .clone()
    };
    field(&field(&value, "content"), "method_name")
        .as_text()
        .unwrap()
        .to_string()
}

/// A wallet holding the fixture's controller key, and a node serving both
/// the replica and Rosetta, with the wallet's ICP balance in e8s.
struct Neurons {
    _server: MockServer,
    balance: Arc<Mutex<u64>>,
    service: crate::service::loopback_service::OpenService,
    wallet: String,
}

impl Neurons {
    async fn new() -> Self {
        let service = crate::service::loopback_service::open().await;
        let mut commit = crate::derivation::setup::tests::fixture(
            Chain::Icp,
            WalletSetupMethod::ImportPrivateKey,
        );
        commit.private_key = Some("01".repeat(32));
        let wallet = service.import(commit).await;
        let balance = Arc::new(Mutex::new(300_010_000u64));
        let server = MockServer::start().await;
        let held = balance.clone();
        Mock::given(any())
            .respond_with(move |request: &Request| {
                let path = request.url.path();
                if path.starts_with("/api/v2/canister/") {
                    assert!(path.ends_with("/query"), "{path}");
                    let reply = match query_method(&request.body).as_str() {
                        "get_network_economics_parameters" => "economics",
                        "list_known_neurons" => "known",
                        "list_neurons" => "active",
                        other => panic!("unexpected governance query {other}"),
                    };
                    return ResponseTemplate::new(200).set_body_raw(
                        hex::decode(fixture()["query_replies"][reply].as_str().unwrap()).unwrap(),
                        "application/cbor",
                    );
                }
                let icp = json!({"symbol": "ICP", "decimals": 8});
                ResponseTemplate::new(200).set_body_json(match path {
                    "/network/list" => json!({"network_identifiers": [{
                        "blockchain": "Internet Computer", "network": "00000000000000020101"}]}),
                    "/account/balance" => json!({"balances": [{
                        "value": held.lock().unwrap().to_string(), "currency": icp}]}),
                    "/construction/preprocess" => json!({"options": {}}),
                    "/construction/metadata" => {
                        json!({"suggested_fee": [{"value": "10000", "currency": icp}]})
                    }
                    other => panic!("unexpected Rosetta request {other}"),
                })
            })
            .mount(&server)
            .await;
        use EndpointCapability::*;
        for (api, capabilities) in [
            (
                crate::EndpointApi::IcpReplica,
                &[Staking, Verification, Broadcast][..],
            ),
            (
                crate::EndpointApi::IcpRosetta,
                &[Balance, Fee, Verification, Broadcast],
            ),
        ] {
            service
                .use_endpoint(Chain::Icp, api, capabilities, &server.uri())
                .await;
        }
        Self {
            _server: server,
            balance,
            service,
            wallet,
        }
    }

    fn request(
        &self,
        action: StakingAction,
        position: Option<&str>,
        amount: Option<&str>,
        lockup: Option<u64>,
    ) -> StakingRequest {
        StakingRequest {
            wallet_id: self.wallet.clone(),
            chain_id: Chain::Icp,
            action,
            validator_id: (action == StakingAction::Stake).then(|| "1".into()),
            position_id: position.map(Into::into),
            amount: amount.map(Into::into),
            lockup_seconds: lockup,
        }
    }

    async fn build(&self, request: StakingRequest) -> Result<SendArtifact, SpectraBridgeError> {
        self.service.build_staking(request, None).await
    }

    /// `request` refused with `words`, and nothing stored.
    async fn refused(&self, request: StakingRequest, words: &str) {
        let error = self.build(request).await.unwrap_err().to_string();
        assert!(error.contains(words), "{error}");
        assert!(self.service.list_sends().await.unwrap().is_empty());
    }
}

/// A new neuron is refused below the live minimum stake, without a
/// dissolve delay between a second and two years, and beyond the balance
/// with the ledger fee; nothing is stored.
#[tokio::test]
async fn a_new_neuron_needs_the_minimum_stake_a_dissolve_delay_and_the_balance() {
    let neurons = Neurons::new().await;
    let stake = |amount, lockup| neurons.request(StakingAction::Stake, None, Some(amount), lockup);
    neurons
        .refused(
            stake("0.5", Some(600)),
            "Amount is below the live minimum neuron stake",
        )
        .await;
    for lockup in [None, Some(0), Some(63_115_201)] {
        neurons
            .refused(
                stake("2", lockup),
                "Review an ICP dissolve delay between 1 second and 2 years",
            )
            .await;
    }
    // Two ICP and the 0.0001 fee, less one e8s.
    *neurons.balance.lock().unwrap() = 200_009_999;
    neurons
        .refused(
            stake("2", Some(600)),
            "Insufficient ICP balance for stake and network fee",
        )
        .await;
    *neurons.balance.lock().unwrap() = 200_010_000;
    neurons.build(stake("2", Some(600))).await.unwrap();
}

/// A new neuron is reviewed as six calls in the order they must run: fund
/// its subaccount, claim it, set its dissolve delay, then follow the chosen
/// neuron on the three topics; the fee and delay are shown.
#[tokio::test]
async fn a_new_neuron_is_funded_claimed_configured_and_set_to_follow() {
    let neurons = Neurons::new().await;
    let built = neurons
        .build(neurons.request(StakingAction::Stake, None, Some("2"), Some(600)))
        .await
        .unwrap();
    let review = built.review.staking.unwrap();
    assert_eq!(review.network_fee, "0.0001");
    assert_eq!(review.lockup_seconds, Some(600));
    assert!(!review.reward_payout_is_delayed);
    let prepared: crate::send::stages::PreparedPayload =
        serde_json::from_str(&built.prepared_details).unwrap();
    let crate::send::stages::PreparedPayload::IcpStaking(prepared) = prepared else {
        panic!("not an ICP staking payload")
    };
    assert_eq!(
        prepared
            .calls
            .iter()
            .map(|call| call.kind.clone())
            .collect::<Vec<_>>(),
        [
            IcpStakingCallKind::Fund,
            IcpStakingCallKind::Claim,
            IcpStakingCallKind::Configure,
            IcpStakingCallKind::Follow,
            IcpStakingCallKind::Follow,
            IcpStakingCallKind::Follow,
        ]
    );
}

/// Only a neuron the wallet's key controls is acted on; claiming its
/// maturity is reviewed as a payout that arrives later.
#[tokio::test]
async fn only_a_controlled_neuron_is_acted_on_and_its_rewards_arrive_later() {
    let neurons = Neurons::new().await;
    neurons
        .refused(
            neurons.request(StakingAction::Unstake, Some("43"), None, None),
            "Select an owned staking position",
        )
        .await;
    let claim = neurons
        .build(neurons.request(StakingAction::ClaimRewards, Some("42"), None, None))
        .await
        .unwrap();
    assert!(claim.review.staking.unwrap().reward_payout_is_delayed);
}
