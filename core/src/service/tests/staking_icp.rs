//! Building ICP neuron operations against a replica that answers with the
//! DFINITY SDK's encoded governance replies and a Rosetta node holding the
//! wallet's balance: what a stake refuses, the calls it reviews, and which
//! neurons the wallet may act on.

use super::*;
use crate::api::endpoint::EndpointApi;
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
                EndpointApi::IcpReplica,
                &[Staking, Verification, Broadcast][..],
            ),
            (
                EndpointApi::IcpRosetta,
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
    let fee = built.review.network_fee.unwrap();
    assert_eq!(
        (fee.amount.as_str(), fee.bound),
        ("0.0001", crate::send::stages::FeeBound::Exact)
    );
    let review = built.review.staking.unwrap();
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

#[test]
fn one_second_dissolve_delay_uses_execution_relative_official_sdk_argument() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/icp-staking-vectors.json"
    ))
    .unwrap();
    let mut calls = vec![];
    let governance = candid::Principal::from_text(Chain::Icp.icp_governance_id().unwrap()).unwrap();
    add_manage(
        &mut calls,
        &governance,
        NeuronSelector::Subaccount(hex::decode(fixture["subaccount"].as_str().unwrap()).unwrap()),
        increase_delay(1).unwrap(),
        IcpStakingCallKind::Configure,
        None,
    )
    .unwrap();
    // Candid permits different type-table ordering across implementations.
    // Decode the independently encoded SDK value and the local value into
    // the same official argument shape rather than pinning a table order.
    for argument in [
        calls[0].argument_hex.as_str(),
        fixture["calls"][1]["argument_hex"].as_str().unwrap(),
    ] {
        let command: ManageNeuron = candid::decode_one(&hex::decode(argument).unwrap()).unwrap();
        let Some(ManageCommand::Configure(Configure {
            operation: Some(ConfigureOperation::IncreaseDissolveDelay(delay)),
        })) = command.command
        else {
            panic!("relative dissolve delay")
        };
        assert_eq!(delay.additional_dissolve_delay_seconds, 1);
        let Some(NeuronSelector::Subaccount(account)) = command.neuron_id_or_subaccount else {
            panic!("derived neuron selector")
        };
        assert_eq!(hex::encode(account), fixture["subaccount"]);
        assert!(command.id.is_none());
    }
    assert!(increase_delay(0).is_err());
    assert!(increase_delay(63_115_201).is_err());
}

#[test]
fn signing_rechecks_current_controller_unlock_amount_and_claimable_maturity() {
    let (key, mut prepared) = icp_staking::tests::new_neuron_fixture();
    let owner = controller(&key);
    let mut request = StakingRequest {
        chain_id: Chain::Icp,
        wallet_id: "wallet".into(),
        action: StakingAction::Unstake,
        position_id: Some("42".into()),
        validator_id: None,
        amount: None,
        lockup_seconds: None,
    };
    let mut neuron = Neuron {
        id: Some(NeuronId { id: 42 }),
        controller: Some(owner),
        account: hex::decode(&prepared.subaccount_hex).unwrap(),
        cached_neuron_stake_e8s: prepared.amount,
        neuron_fees_e8s: 0,
        maturity_e8s_equivalent: 200_000_000,
        staked_maturity_e8s_equivalent: None,
        dissolve_state: Some(DissolveState::DissolveDelaySeconds(600)),
        spawn_at_timestamp_seconds: None,
        maturity_disbursements_in_progress: None,
    };
    fn set_packet(prepared: &mut PreparedIcpStaking, request: &StakingRequest, neuron: &Neuron) {
        let (command, kind) = match request.action {
            StakingAction::Unstake => (
                ManageCommand::Configure(Configure {
                    operation: Some(ConfigureOperation::StartDissolving(Empty {})),
                }),
                IcpStakingCallKind::Configure,
            ),
            StakingAction::Withdraw => (
                withdraw_command(neuron, &prepared.sender, None).unwrap(),
                IcpStakingCallKind::Disburse,
            ),
            StakingAction::ClaimRewards => (
                claim_maturity_command(&prepared.sender).unwrap(),
                IcpStakingCallKind::DisburseMaturity,
            ),
            _ => panic!("existing action"),
        };
        prepared.calls.clear();
        add_manage(
            &mut prepared.calls,
            &candid::Principal::from_text(Chain::Icp.icp_governance_id().unwrap()).unwrap(),
            NeuronSelector::NeuronId(neuron.id.clone().unwrap()),
            command,
            kind,
            None,
        )
        .unwrap();
    }
    prepared.fee = 0;
    set_packet(&mut prepared, &request, &neuron);
    validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 0).unwrap();
    neuron.controller = Some(candid::Principal::anonymous());
    assert!(validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 0).is_err());
    neuron.controller = Some(owner);
    request.action = StakingAction::Withdraw;
    prepared.fee = 10_000;
    set_packet(&mut prepared, &request, &neuron);
    assert!(
        validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 10_000).is_err()
    );
    neuron.dissolve_state = Some(DissolveState::DissolveDelaySeconds(0));
    validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 10_000).unwrap();
    neuron.cached_neuron_stake_e8s -= 1;
    assert!(
        validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 10_000).is_err()
    );
    request.action = StakingAction::ClaimRewards;
    prepared.amount = neuron.maturity_e8s_equivalent;
    prepared.fee = 0;
    set_packet(&mut prepared, &request, &neuron);
    validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 10_000).unwrap();
    neuron.maturity_e8s_equivalent = 0;
    assert!(
        validate_existing_neuron_intent(&request, &prepared, &neuron, owner, 10, 10_000).is_err()
    );
}
#[test]
fn full_and_partial_withdrawal_do_not_deduct_neuron_fees_twice() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/icp-staking-vectors.json"
    ))
    .unwrap();
    let (_, prepared) = icp_staking::tests::new_neuron_fixture();
    let neuron = Neuron {
        id: Some(NeuronId { id: 42 }),
        controller: None,
        account: hex::decode(&prepared.subaccount_hex).unwrap(),
        cached_neuron_stake_e8s: 200_000_000,
        neuron_fees_e8s: 10_000,
        maturity_e8s_equivalent: 0,
        staked_maturity_e8s_equivalent: None,
        dissolve_state: Some(DissolveState::DissolveDelaySeconds(0)),
        spawn_at_timestamp_seconds: None,
        maturity_disbursements_in_progress: None,
    };
    let full_sdk: ManageNeuron = candid::decode_one(
        &hex::decode(fixture["full_disburse_argument_hex"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let partial_sdk: ManageNeuron = candid::decode_one(
        &hex::decode(fixture["calls"][4]["argument_hex"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    for (command, expected) in [
        (
            withdraw_command(&neuron, &prepared.sender, None).unwrap(),
            None,
        ),
        (full_sdk.command.unwrap(), None),
        (
            withdraw_command(&neuron, &prepared.sender, Some(100_000_000)).unwrap(),
            Some(100_010_000),
        ),
        (partial_sdk.command.unwrap(), Some(100_010_000)),
    ] {
        let ManageCommand::Disburse(disburse) = command else {
            panic!("Disburse")
        };
        assert_eq!(disburse.amount.as_ref().map(|amount| amount.e8s), expected);
        assert_eq!(
            hex::encode(disburse.to_account.unwrap().hash),
            prepared.sender
        );
        let principal = disburse
            .amount
            .as_ref()
            .map_or(net_stake(&neuron).unwrap(), |amount| {
                amount.e8s - neuron.neuron_fees_e8s
            });
        assert_eq!(
            principal - 10_000,
            if expected.is_some() {
                99_990_000
            } else {
                199_980_000
            }
        );
    }
    assert!(withdraw_command(&neuron, &prepared.sender, Some(u64::MAX)).is_err());
}

#[test]
fn maturity_claims_enforce_the_official_minimum_queue_limit_and_spawn_state() {
    let mut neuron = Neuron {
        id: Some(NeuronId { id: 42 }),
        controller: None,
        account: vec![0; 32],
        cached_neuron_stake_e8s: 0,
        neuron_fees_e8s: 0,
        maturity_e8s_equivalent: 100_000_000,
        staked_maturity_e8s_equivalent: None,
        dissolve_state: Some(DissolveState::DissolveDelaySeconds(0)),
        spawn_at_timestamp_seconds: None,
        maturity_disbursements_in_progress: None,
    };
    assert!(maturity_can_be_disbursed(&neuron).unwrap());
    neuron.maturity_e8s_equivalent -= 1;
    assert!(!maturity_can_be_disbursed(&neuron).unwrap());
    assert!(
        !position(&neuron, "owner", 10)
            .unwrap()
            .available_actions
            .contains(&StakingAction::ClaimRewards)
    );
    neuron.maturity_e8s_equivalent += 1;
    neuron.spawn_at_timestamp_seconds = Some(10);
    assert!(!maturity_can_be_disbursed(&neuron).unwrap());
    neuron.spawn_at_timestamp_seconds = None;
    let pending = MaturityDisbursement {
        amount_e8s: Some(100_000_000),
        timestamp_of_disbursement_seconds: Some(1),
        finalize_disbursement_timestamp_seconds: Some(604_801),
    };
    neuron.maturity_disbursements_in_progress = Some(vec![pending; 10]);
    assert!(!maturity_can_be_disbursed(&neuron).unwrap());
    neuron
        .maturity_disbursements_in_progress
        .as_mut()
        .unwrap()
        .pop();
    assert!(maturity_can_be_disbursed(&neuron).unwrap());
}
#[test]
fn processing_and_unexpired_absent_ingress_cannot_be_renewed() {
    assert!(require_repairable_ingress(&IngressStatus::Processing, 100, 200).is_err());
    assert!(require_repairable_ingress(&IngressStatus::Pending, 100, 99).is_err());
    assert!(require_repairable_ingress(&IngressStatus::Pending, 100, 100).is_ok());
    // Pruning permits a fresh repair-state check. Neither state proves
    // funding success; repair still needs its independent exact proof.
    assert!(require_repairable_ingress(&IngressStatus::Unknown, 100, 200).is_ok());
    assert!(require_repairable_ingress(&IngressStatus::Done, 100, 200).is_ok());
}

#[test]
fn repair_does_not_relock_an_existing_dissolving_or_unlocked_neuron() {
    let mut neuron = Neuron {
        id: Some(NeuronId { id: 42 }),
        controller: None,
        account: vec![1; 32],
        cached_neuron_stake_e8s: 100,
        neuron_fees_e8s: 0,
        maturity_e8s_equivalent: 0,
        staked_maturity_e8s_equivalent: None,
        dissolve_state: Some(DissolveState::WhenDissolvedTimestampSeconds(100)),
        spawn_at_timestamp_seconds: None,
        maturity_disbursements_in_progress: None,
    };
    assert_eq!(existing_neuron_delay(&neuron, 10).unwrap(), Some(90));
    assert_eq!(existing_neuron_delay(&neuron, 101).unwrap(), Some(0));
    neuron.dissolve_state = Some(DissolveState::DissolveDelaySeconds(0));
    assert_eq!(existing_neuron_delay(&neuron, 10).unwrap(), None);
    neuron.dissolve_state = None;
    assert!(existing_neuron_delay(&neuron, 10).is_err());
}
#[test]
fn independent_governance_queries_keep_pending_maturity_separate_from_claimable_rewards() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/icp-staking-vectors.json"
    ))
    .unwrap();
    let bytes = hex::decode(fixture["query_replies"]["queued"].as_str().unwrap()).unwrap();
    let envelope: ciborium::Value = ciborium::from_reader(bytes.as_slice()).unwrap();
    let envelope = match envelope {
        ciborium::Value::Tag(_, value) => *value,
        value => value,
    };
    let ciborium::Value::Map(map) = envelope else {
        panic!("query envelope")
    };
    let reply = map
        .iter()
        .find(|(key, _)| key == &ciborium::Value::Text("reply".into()))
        .unwrap();
    let ciborium::Value::Map(reply) = &reply.1 else {
        panic!("query reply")
    };
    let arg = reply
        .iter()
        .find(|(key, _)| key == &ciborium::Value::Text("arg".into()))
        .unwrap();
    let ciborium::Value::Bytes(arg) = &arg.1 else {
        panic!("Candid reply")
    };
    let response: ListNeuronsResponse = candid::decode_one(arg).unwrap();
    let pending = position(
        &response.full_neurons[0],
        fixture["owner"].as_str().unwrap(),
        1_800_000_000,
    )
    .unwrap();
    assert_eq!(
        pending.claimable_rewards_smallest_unit.as_deref(),
        Some("0")
    );
    assert_eq!(
        pending.pending_rewards_smallest_unit.as_deref(),
        Some("1000000")
    );
    assert!(pending.rewards_unlock_time_unix.is_some());
    assert!(
        !pending
            .available_actions
            .contains(&StakingAction::ClaimRewards)
    );
}

#[test]
fn neuron_positions_follow_real_dissolve_state_and_control() {
    let mut n = Neuron {
        id: Some(NeuronId { id: 42 }),
        controller: None,
        account: vec![1; 32],
        cached_neuron_stake_e8s: 100,
        neuron_fees_e8s: 2,
        maturity_e8s_equivalent: 3,
        staked_maturity_e8s_equivalent: None,
        dissolve_state: Some(DissolveState::DissolveDelaySeconds(100)),
        spawn_at_timestamp_seconds: None,
        maturity_disbursements_in_progress: None,
    };
    let active = position(&n, "owner", 10).unwrap();
    assert_eq!(active.staked_amount_smallest_unit, "98");
    assert!(active.available_actions.contains(&StakingAction::Unstake));
    n.dissolve_state = Some(DissolveState::WhenDissolvedTimestampSeconds(100));
    assert_eq!(
        position(&n, "owner", 10).unwrap().status,
        StakingPositionStatus::Unbonding
    );
    assert_eq!(
        position(&n, "owner", 101)
            .unwrap()
            .withdrawable_amount_smallest_unit,
        "98"
    );
    n.neuron_fees_e8s = 101;
    assert!(position(&n, "owner", 101).is_err());
}
