//! Staking on Solana, Sui, Aptos and NEAR against a node that answers as
//! each network does: which positions a wallet owns, what a build refuses
//! before anything is stored, and what signing reads again.

use super::*;
use crate::derivation::setup::WalletSetupMethod;
use crate::send::stages::SendStage;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const STAKE_PROGRAM: &str = "Stake11111111111111111111111111111111111111";

/// What the node holds. Each test changes one fact.
#[derive(Clone)]
struct Node {
    chain: Chain,
    owner: String,
    balance: u128,
    /// Solana's fee for a message, in lamports.
    fee: u64,
    /// Aptos' gas unit price in octas, NEAR's in yoctoNEAR.
    gas_price: u128,
    /// The stake's authority (Solana), the objects' owner (Sui) or the
    /// signing key's permission (NEAR) is another account's.
    foreign: bool,
    simulation: bool,
    unlocked: bool,
    /// Lamports in the stake account beyond its delegation and rent.
    surplus: u64,
    /// The node cannot run a Solana simulation at all.
    simulation_unavailable: bool,
    /// Aptos reports an amount other than the one asked for.
    adjusted: bool,
    /// NEAR's protocol charges twice for a function call's execution.
    fee_schedule_doubled: bool,
}

fn b58(byte: u8) -> String {
    bs58::encode([byte; 32]).into_string()
}

fn target(chain: Chain) -> String {
    match chain {
        Chain::Solana => b58(0x22),
        Chain::Near => "validator.poolv1.near".into(),
        _ => format!("0x{}", "22".repeat(32)),
    }
}

fn position(chain: Chain) -> String {
    match chain {
        Chain::Solana => b58(0x44),
        Chain::Sui => format!("0x{}", "44".repeat(32)),
        _ => target(chain),
    }
}

fn solana(node: &Node, method: &str, params: &Value) -> Value {
    let authority = if node.foreign {
        target(Chain::Solana)
    } else {
        node.owner.clone()
    };
    let deactivation = if node.unlocked {
        "9".to_string()
    } else {
        u64::MAX.to_string()
    };
    let stake = json!({"lamports": 2_002_282_880 + node.surplus, "owner": STAKE_PROGRAM,
        "data": {"parsed": {"type": "delegated", "info": {
            "meta": {"rentExemptReserve": "2282880",
                "authorized": {"staker": authority, "withdrawer": authority},
                "lockup": {"epoch": 0, "unixTimestamp": 0}},
            "stake": {"delegation": {"voter": target(Chain::Solana), "stake": "2000000000",
                "activationEpoch": "8", "deactivationEpoch": deactivation}}}}}});
    match method {
        "getGenesisHash" => json!(Chain::Solana.solana_genesis_hash().unwrap()),
        "getLatestBlockhash" => {
            json!({"value": {"blockhash": b58(0x33), "lastValidBlockHeight": 1000}})
        }
        "isBlockhashValid" => json!({"value": true}),
        "getEpochInfo" => json!({"epoch": 10}),
        "getVoteAccounts" => json!({"current": [{"votePubkey": target(Chain::Solana),
            "commission": 5, "activatedStake": 2_000_000_000u64}], "delinquent": []}),
        "getStakeMinimumDelegation" => json!({"value": 1_000_000_000u64}),
        "getMinimumBalanceForRentExemption" => json!(2_282_880),
        "getBalance" => json!({"value": node.balance}),
        "getFeeForMessage" => json!({"value": node.fee}),
        "simulateTransaction" => {
            if node.simulation_unavailable {
                return json!({"value": {"err": "InsufficientFundsForFee"}});
            }
            let raw = STANDARD.decode(params[0].as_str().unwrap()).unwrap();
            // A Withdraw (instruction 4) ends the message with its lamports.
            let (rest, lamports) = raw.split_at(raw.len() - 8);
            let withdrawn = if rest.ends_with(&4u32.to_le_bytes()) {
                u64::from_le_bytes(lamports.try_into().unwrap())
            } else {
                0
            };
            let allowed =
                node.simulation && (withdrawn == 0 || node.unlocked || withdrawn <= node.surplus);
            json!({"value": {"err": if allowed {
                Value::Null
            } else {
                json!({"InstructionError": [0, "Custom"]})
            }}})
        }
        "getProgramAccounts" => json!([{"pubkey": position(Chain::Solana), "account": stake}]),
        "getAccountInfo" => json!({"value": stake}),
        other => panic!("unexpected Solana call {other}"),
    }
}

fn sui(node: &Node, method: &str, params: &Value) -> Value {
    let (identifier, checkpoint) = Chain::Sui.sui_network_identity().unwrap();
    let owner = if node.foreign {
        target(Chain::Sui)
    } else {
        node.owner.clone()
    };
    let pool = format!("0x{}", "99".repeat(32));
    let coin = format!("0x{}", "33".repeat(32));
    match method {
        "sui_getChainIdentifier" => json!(identifier),
        "sui_getCheckpoint" => json!({"sequenceNumber": "0", "digest": checkpoint}),
        "suix_getReferenceGasPrice" => json!("1000"),
        "suix_getLatestSuiSystemState" => json!({"activeValidators": [{
            "suiAddress": target(Chain::Sui), "name": "Fixture validator", "description": "",
            "projectUrl": "", "commissionRate": "500", "stakingPoolSuiBalance": "2000000000"}]}),
        "suix_getStakes" => json!([{"validatorAddress": target(Chain::Sui), "stakingPool": pool,
            "stakes": [{"stakedSuiId": position(Chain::Sui), "stakeActiveEpoch": "8",
                "principal": "2000000000", "status": "Active", "estimatedReward": "1234567"}]}]),
        "suix_getCoins" => json!({"data": [{"coinObjectId": coin, "version": "7",
            "digest": "1".repeat(32), "balance": node.balance.to_string()}],
            "hasNextPage": false, "nextCursor": null}),
        "sui_getObject" if params[0] == "0x5" => json!({"data": {
            "type": "0x3::sui_system::SuiSystemState",
            "owner": {"Shared": {"initial_shared_version": 1}}}}),
        "sui_getObject" => {
            let stake = params[0] == position(Chain::Sui);
            assert!(stake || params[0] == coin.as_str(), "{params}");
            let (version, kind, fields) = if stake {
                (
                    "8",
                    "0x3::staking_pool::StakedSui",
                    json!({"principal": "2000000000", "pool_id": pool,
                        "stake_activation_epoch": "8"}),
                )
            } else {
                (
                    "7",
                    "0x2::coin::Coin<0x2::sui::SUI>",
                    json!({"balance": node.balance.to_string()}),
                )
            };
            json!({"data": {"objectId": params[0], "version": version, "digest": "1".repeat(32),
                "owner": {"AddressOwner": owner}, "type": kind, "content": {"fields": fields}}})
        }
        "sui_dryRunTransactionBlock" => json!({"effects": {"status": {
            "status": if node.simulation { "success" } else { "failure" }}}}),
        other => panic!("unexpected Sui call {other}"),
    }
}

fn aptos(node: &Node, request: &Request, body: &Value) -> Value {
    if let Some(variables) = body.get("variables") {
        // The indexer, whose one page of pools follows the empty cursor.
        let rows = if variables["after"] == "" {
            json!([{"pool_address": target(Chain::Aptos)}])
        } else {
            json!([])
        };
        return json!({"data": {"ledger_infos": [{"chain_id": 1}],
            "current_delegator_balances": rows}});
    }
    let path = request.url.path();
    match (request.method.as_str(), path) {
        ("GET", "/") => json!({"chain_id": 1, "ledger_version": "100",
            "ledger_timestamp": "1800000000000000"}),
        ("GET", "/estimate_gas_price") => json!({"gas_estimate": node.gas_price}),
        ("GET", path) if path.contains("/resource/") => {
            json!({"data": {"locked_until_secs": "1900000000"}})
        }
        ("GET", path) if path.starts_with("/accounts/") => json!({"sequence_number": "7"}),
        ("POST", "/view") => {
            let function = body["function"]
                .as_str()
                .unwrap()
                .rsplit("::")
                .next()
                .unwrap();
            match function {
                "delegation_pool_exists" => json!([true]),
                "get_stake" => json!(["2000000000", "1000000000", "0"]),
                "get_pending_withdrawal" => json!([node.unlocked, "1000000000"]),
                "operator_commission_percentage" => json!(["500"]),
                "delegator_allowlisted" => json!([true]),
                "get_add_stake_fee" => json!(["1000000"]),
                "balance" => json!([node.balance.to_string()]),
                other => panic!("unexpected Aptos view {other}"),
            }
        }
        ("POST", "/transactions/simulate") => {
            let function = body["payload"]["function"].as_str().unwrap();
            let (event, field) = match function.rsplit("::").next().unwrap() {
                "add_stake" => ("AddStake", "amount_added"),
                "unlock" => ("UnlockStake", "amount_unlocked"),
                _ => ("WithdrawStake", "amount_withdrawn"),
            };
            let asked: u64 = body["payload"]["arguments"][1]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
            let moved = asked + u64::from(node.adjusted);
            let estimate = request
                .url
                .query()
                .is_some_and(|q| q.contains("estimate_max"));
            json!([{"success": node.simulation,
                "vm_status": if node.simulation { "Executed" } else { "ABORT" },
                "gas_used": "10000",
                "max_gas_amount": if estimate { json!("200000") } else { body["max_gas_amount"].clone() },
                "events": [{"type": format!("0x1::delegation_pool::{event}"),
                    "data": {field: moved.to_string()}}]}])
        }
        other => panic!("unexpected Aptos request {other:?}"),
    }
}

fn near(node: &Node, method: &str, params: &Value) -> Value {
    match method {
        "status" => json!({"chain_id": "mainnet"}),
        "block" if params["finality"] == "optimistic" => {
            json!({"header": {"hash": b58(0x66), "height": 10_001}})
        }
        "block" => json!({"header": {"hash": b58(0x33), "height": 10_000}}),
        "gas_price" => json!({"gas_price": node.gas_price.to_string()}),
        "EXPERIMENTAL_protocol_config" => {
            let mut config: Value = serde_json::from_str(include_str!(
                "../../../tests/fixtures/near-staking-fee-protocol86.json"
            ))
            .unwrap();
            config["transaction_validity_period"] = json!(86_400);
            config["runtime_config"]["storage_amount_per_byte"] = json!("10000000000000000000");
            if node.fee_schedule_doubled {
                let execution = &mut config["runtime_config"]["transaction_costs"]["action_creation_config"]
                    ["function_call_cost"]["execution"];
                *execution = json!(execution.as_u64().unwrap() * 2);
            }
            config
        }
        "query" => match params["request_type"].as_str().unwrap() {
            "view_access_key" => json!({"nonce": 7, "permission": if node.foreign {
                json!({"FunctionCall": {}})
            } else {
                json!("FullAccess")
            }}),
            "view_account" => json!({"amount": node.balance.to_string(), "storage_usage": 1000,
                "locked": "0"}),
            "call_function" => {
                let value = match params["method_name"].as_str().unwrap() {
                    "is_whitelisted" => json!(true),
                    "get_owner_id" => json!("operator.near"),
                    "get_account" => json!({"account_id": node.owner,
                        "staked_balance": (2 * 10u128.pow(24)).to_string(),
                        "unstaked_balance": 10u128.pow(24).to_string(),
                        "can_withdraw": node.unlocked}),
                    "get_reward_fee_fraction" => json!({"numerator": 5, "denominator": 100}),
                    "is_staking_paused" => json!(false),
                    other => panic!("unexpected NEAR view {other}"),
                };
                json!({"result": serde_json::to_vec(&value).unwrap(), "block_height": 100,
                    "logs": []})
            }
            other => panic!("unexpected NEAR query {other}"),
        },
        other => panic!("unexpected NEAR call {other}"),
    }
}

/// A wallet restored from the shared phrase on `chain`, and a node that is
/// that network's only endpoint for every API its staking reads.
struct Staking {
    _server: MockServer,
    node: Arc<Mutex<Node>>,
    service: crate::service::loopback_service::OpenService,
    chain: Chain,
    wallet: String,
}

impl Staking {
    async fn new(chain: Chain) -> Self {
        let service = crate::service::loopback_service::open().await;
        let wallet = service
            .import(crate::derivation::setup::tests::fixture(
                chain,
                WalletSetupMethod::ImportPhrase,
            ))
            .await;
        let owner = service.address(&wallet, chain).await;
        let node = Arc::new(Mutex::new(Node {
            chain,
            owner,
            balance: 100 * 10u128.pow(u32::from(chain.native_decimals())),
            fee: 5_000,
            gas_price: if chain == Chain::Aptos {
                100
            } else {
                100_000_000
            },
            foreign: false,
            simulation: true,
            unlocked: false,
            surplus: 0,
            simulation_unavailable: false,
            adjusted: false,
            fee_schedule_doubled: false,
        }));
        let server = MockServer::start().await;
        let answers = node.clone();
        Mock::given(any())
            .respond_with(move |request: &Request| {
                let node = answers.lock().unwrap();
                let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
                if node.chain == Chain::Near && request.url.path().starts_with("/v1/account/") {
                    // fastnear's index of the pools an account staked with.
                    return ResponseTemplate::new(200).set_body_json(json!({
                        "account_id": node.owner, "pools": [{"pool_id": target(Chain::Near)}]}));
                }
                if node.chain == Chain::Aptos {
                    return ResponseTemplate::new(200).set_body_json(aptos(&node, request, &body));
                }
                let method = body["method"].as_str().unwrap();
                let params = &body["params"];
                let result = match node.chain {
                    Chain::Solana => solana(&node, method, params),
                    Chain::Sui => sui(&node, method, params),
                    _ => near(&node, method, params),
                };
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc": "2.0", "id": body["id"], "result": result}))
            })
            .mount(&server)
            .await;
        use EndpointCapability::*;
        let apis: &[(crate::EndpointApi, &[EndpointCapability])] = match chain {
            Chain::Solana => &[(
                crate::EndpointApi::SolanaJsonRpc,
                &[Staking, Balance, Fee, Verification, Broadcast],
            )],
            Chain::Sui => &[(
                crate::EndpointApi::SuiJsonRpc,
                &[Staking, Balance, Fee, Verification, Broadcast],
            )],
            Chain::Aptos => &[
                (
                    crate::EndpointApi::AptosRest,
                    &[Staking, Balance, Fee, Verification, Broadcast],
                ),
                (crate::EndpointApi::AptosIndexer, &[History]),
            ],
            _ => &[
                (
                    crate::EndpointApi::NearJsonRpc,
                    &[Staking, Balance, Fee, Verification, Broadcast],
                ),
                (crate::EndpointApi::Fastnear, &[Staking]),
            ],
        };
        for (api, capabilities) in apis {
            service
                .use_endpoint(chain, *api, capabilities, &server.uri())
                .await;
        }
        Self {
            _server: server,
            node,
            service,
            chain,
            wallet,
        }
    }

    fn set(&self, change: impl FnOnce(&mut Node)) {
        change(&mut self.node.lock().unwrap());
    }

    fn owner(&self) -> String {
        self.node.lock().unwrap().owner.clone()
    }

    async fn positions(
        &self,
        targets: Vec<String>,
    ) -> Result<Vec<crate::staking::StakingPosition>, SpectraBridgeError> {
        self.service
            .fetch_staking_positions(self.wallet.clone(), self.chain, targets, None)
            .await
    }

    fn request(&self, action: StakingAction, amount: &str) -> StakingRequest {
        let stake = action == StakingAction::Stake;
        StakingRequest {
            wallet_id: self.wallet.clone(),
            chain_id: self.chain,
            action,
            validator_id: stake.then(|| target(self.chain)),
            position_id: (!stake).then(|| position(self.chain)),
            amount: Some(amount.into()),
            lockup_seconds: None,
        }
    }

    async fn build(
        &self,
        action: StakingAction,
        amount: &str,
    ) -> Result<crate::send::stages::SendArtifact, SpectraBridgeError> {
        self.service
            .build_staking(self.request(action, amount), None)
            .await
    }

    /// `build` refused with `words`, and nothing stored.
    async fn refused(&self, action: StakingAction, amount: &str, words: &str) {
        let error = self.build(action, amount).await.unwrap_err().to_string();
        assert!(error.contains(words), "{:?}: {error}", self.chain);
        assert!(self.service.list_sends().await.unwrap().is_empty());
    }

    async fn sign(
        &self,
        artifact: &crate::send::stages::SendArtifact,
    ) -> Result<crate::send::stages::SendArtifact, SpectraBridgeError> {
        self.service
            .sign_send(artifact.id.clone(), artifact.review_digest.clone(), None)
            .await
    }

    /// Signing `artifact` refused with `words`, and it is still unsigned.
    async fn sign_refused(&self, artifact: &crate::send::stages::SendArtifact, words: &str) {
        let error = self.sign(artifact).await.unwrap_err().to_string();
        assert!(error.contains(words), "{:?}: {error}", self.chain);
        let stored = self
            .service
            .inspect_send(artifact.id.clone())
            .await
            .unwrap();
        assert_eq!(stored.stage, SendStage::Prepared);
        assert!(stored.signed_payload.is_none());
    }
}

fn json_of(positions: &[crate::staking::StakingPosition]) -> Value {
    serde_json::to_value(positions).unwrap()
}

/// The positions a wallet owns are found without naming them: by stake
/// account on Solana, by object on Sui, through the Aptos indexer and
/// fastnear. Naming the pool finds the same one, owned by the wallet.
#[tokio::test]
async fn owned_positions_are_discovered_and_naming_their_pool_finds_the_same() {
    for chain in [Chain::Solana, Chain::Sui, Chain::Aptos, Chain::Near] {
        let staking = Staking::new(chain).await;
        let discovered = staking.positions(vec![]).await.unwrap();
        assert_eq!(discovered.len(), 1, "{chain:?}");
        assert_eq!(discovered[0].id, position(chain));
        assert_eq!(discovered[0].owner, staking.owner());
        let named = staking.positions(vec![position(chain)]).await.unwrap();
        assert_eq!(json_of(&named), json_of(&discovered), "{chain:?}");
        if chain == Chain::Sui {
            // Sui's reward estimate is what unstaking pays out beside the principal.
            assert_eq!(
                discovered[0].claimable_rewards_smallest_unit.as_deref(),
                Some("1234567")
            );
        }
    }
}

/// Lamports a Solana stake account holds beyond its delegation and rent
/// can leave while the stake stays active.
#[tokio::test]
async fn a_solana_stakes_surplus_is_withdrawable_while_it_stays_active() {
    let staking = Staking::new(Chain::Solana).await;
    staking.set(|node| node.surplus = 1_000_000_000);
    let [position] = <[_; 1]>::try_from(staking.positions(vec![]).await.unwrap()).unwrap();
    assert_eq!(
        position.status,
        crate::staking::StakingPositionStatus::Active
    );
    assert_eq!(position.staked_amount_smallest_unit, "2000000000");
    assert_eq!(position.withdrawable_amount_smallest_unit, "1000000000");
    assert_eq!(
        position.available_actions,
        [StakingAction::Unstake, StakingAction::Withdraw]
    );
}

/// A simulation the node could not run says nothing about whether a stake
/// is locked, so reading positions fails rather than reporting none
/// withdrawable.
#[tokio::test]
async fn a_solana_simulation_the_node_cannot_run_fails_the_positions_read() {
    let staking = Staking::new(Chain::Solana).await;
    staking.set(|node| node.simulation_unavailable = true);
    let error = staking.positions(vec![]).await.unwrap_err().to_string();
    assert!(
        error.contains("Unable to determine Solana staking execution"),
        "{error}"
    );
}

/// A stake the balance cannot cover with its fee is refused, and nothing
/// is stored.
#[tokio::test]
async fn a_stake_the_balance_cannot_cover_is_refused() {
    for (chain, amount, words) in [
        (
            Chain::Solana,
            "2",
            "Insufficient native balance for staking and network fee",
        ),
        (Chain::Sui, "2", "Insufficient 0x2::sui::SUI coin objects"),
        (
            Chain::Aptos,
            "20",
            "Insufficient native balance for staking and network fee",
        ),
        (
            Chain::Near,
            "2",
            "Insufficient native balance for staking and network fee",
        ),
    ] {
        let staking = Staking::new(chain).await;
        staking.set(|node| node.balance = 0);
        staking.refused(StakingAction::Stake, amount, words).await;
    }
}

/// Solana's live minimum delegation and Sui's protocol minimum.
#[tokio::test]
async fn a_stake_below_the_networks_minimum_is_refused() {
    for (chain, words) in [
        (Chain::Solana, "Stake is below the live minimum delegation"),
        (Chain::Sui, "Sui stake is below the protocol minimum"),
    ] {
        let staking = Staking::new(chain).await;
        staking.refused(StakingAction::Stake, "0.5", words).await;
    }
}

/// A stake account whose authorities, or a stake object whose owner, is
/// another account's is no position of this wallet's.
#[tokio::test]
async fn a_position_under_another_accounts_authority_is_refused() {
    for (chain, words) in [
        (Chain::Solana, "account authority mismatch"),
        (Chain::Sui, "owner differs from wallet"),
    ] {
        let staking = Staking::new(chain).await;
        staking.set(|node| node.foreign = true);
        let error = staking
            .positions(vec![position(chain)])
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(words), "{chain:?}: {error}");
    }
}

/// Stake still locked is refused before anything is stored; the same
/// withdrawal once unlocked is built.
#[tokio::test]
async fn locked_stake_cannot_be_withdrawn() {
    for (chain, amount, words) in [
        (
            Chain::Solana,
            "2.00228288",
            "Solana staking simulation refused this authority, amount or unlock state",
        ),
        (
            Chain::Aptos,
            "10",
            "Aptos stake is still locked or withdrawal exceeds owned stake",
        ),
        (
            Chain::Near,
            "1",
            "NEAR withdrawal is not available for this owned position",
        ),
    ] {
        let staking = Staking::new(chain).await;
        staking
            .refused(StakingAction::Withdraw, amount, words)
            .await;
        staking.set(|node| node.unlocked = true);
        staking
            .build(StakingAction::Withdraw, amount)
            .await
            .unwrap();
    }
}

/// Aptos rounds delegation amounts to shares; a simulation that would move
/// another amount than the one reviewed is refused.
#[tokio::test]
async fn an_aptos_delegation_that_would_move_another_amount_is_refused() {
    let staking = Staking::new(Chain::Aptos).await;
    staking.set(|node| node.adjusted = true);
    staking
        .refused(
            StakingAction::Stake,
            "20",
            "Aptos delegation would adjust the reviewed amount",
        )
        .await;
}

/// Signing reads the network again and refuses a reviewed stake whose fee
/// rose, whose gas coin changed hands, whose simulation now fails, or whose
/// key, gas price or fee schedule changed; once the network is as reviewed
/// it signs.
#[tokio::test]
async fn signing_refuses_a_stake_the_network_no_longer_allows_as_reviewed() {
    type Change = fn(&mut Node);
    let cases: [(Chain, &str, &[(Change, &str)]); 4] = [
        (
            Chain::Solana,
            "2",
            &[(
                |node| node.fee = 10_000,
                "Solana fee or rent exceeds reviewed amount",
            )],
        ),
        (
            Chain::Sui,
            "2",
            &[(
                |node| node.foreign = true,
                "Reviewed Sui gas object changed or is not owned by this wallet",
            )],
        ),
        (
            Chain::Aptos,
            "20",
            &[(
                |node| node.simulation = false,
                "Aptos staking simulation refused",
            )],
        ),
        (
            Chain::Near,
            "2",
            &[
                (
                    |node| node.foreign = true,
                    "NEAR requires the wallet's full-access key",
                ),
                (
                    |node| node.gas_price *= 2,
                    "NEAR fee exceeds reviewed budget",
                ),
                (
                    |node| node.fee_schedule_doubled = true,
                    "NEAR fee exceeds reviewed budget",
                ),
            ],
        ),
    ];
    for (chain, amount, changes) in cases {
        let staking = Staking::new(chain).await;
        let built = staking.build(StakingAction::Stake, amount).await.unwrap();
        for (change, words) in changes {
            let before = staking.node.lock().unwrap().clone();
            staking.set(change);
            staking.sign_refused(&built, words).await;
            staking.set(|node| *node = before);
        }
        let signed = staking.sign(&built).await.unwrap();
        assert_eq!(signed.stage, SendStage::Signed, "{chain:?}");
    }
}
