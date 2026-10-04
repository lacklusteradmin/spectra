//! Local ICP ingress construction. Each governance step has its own request ID;
//! funding a neuron never stands in for executing its management commands.
use super::error::SendError;
use super::icp_stages::{bytes, encode, envelope, map, request_hash, text};
use super::keys::Ed25519Seed;
use crate::api::icp_replica::{ClaimResponse, ClaimResult, ManageReply, ManageResponse};
use crate::derivation::icp::{account_from_principal, principal};
use ciborium::Value as Cbor;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum IcpStakingCallKind {
    Fund,
    Claim,
    Configure,
    Follow,
    Disburse,
    DisburseMaturity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct IcpStakingCall {
    pub canister: String,
    pub method: String,
    pub argument_hex: String,
    pub nonce_hex: Option<String>,
    pub kind: IcpStakingCallKind,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparedIcpStaking {
    pub sender: String,
    pub controller_hex: String,
    pub ingress_expiry_ns: u64,
    pub amount: u64,
    pub fee: u64,
    pub neuron_nonce: Option<u64>,
    pub subaccount_hex: String,
    pub calls: Vec<IcpStakingCall>,
    /// Exact original funding transfer hash, independent of ingress expiry.
    pub funding_ledger_hash: Option<String>,
    pub funding: Option<super::icp_stages::PreparedIcpTransaction>,
    /// Completed steps retain their original ingress IDs and signed requests.
    pub completed_calls: Vec<SignedIcpStakingCall>,
    /// Older signed management attempts remain available after a repair.
    pub prior_calls: Vec<SignedIcpStakingCall>,
    /// Broadcast attempts from earlier reviewed revisions remain durable while
    /// the current revision starts with no selected nodes or submission state.
    pub prior_attempts: Vec<super::stages::BroadcastAttempt>,
    pub funding_confirmed_by_ledger: bool,
    pub recovered_configured_neuron: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SignedIcpStakingCall {
    pub canister: String,
    pub request_id: String,
    pub update_hex: String,
    pub read_state_hex: String,
    pub kind: IcpStakingCallKind,
}

impl SignedIcpStakingCall {
    /// Bind a certificate's request path to the actual retained IC content,
    /// network canister, method and wallet controller before trusting its reply.
    pub(crate) fn validated_argument(&self, controller: &[u8]) -> Result<Vec<u8>, SendError> {
        fn field<'a>(value: &'a Cbor, key: &str) -> Result<&'a Cbor, SendError> {
            let value = match value {
                Cbor::Tag(55799, v) => v.as_ref(),
                value => value,
            };
            let Cbor::Map(fields) = value else {
                return Err(SendError::invalid("Invalid saved ICP envelope"));
            };
            let mut matching = fields
                .iter()
                .filter(|(name, _)| name == &Cbor::Text(key.into()));
            let value = matching
                .next()
                .map(|(_, value)| value)
                .ok_or_else(|| SendError::invalid("Missing saved ICP content field"))?;
            if matching.next().is_some() {
                return Err(SendError::invalid("Duplicate saved ICP content field"));
            }
            Ok(value)
        }
        let bytes = hex::decode(&self.update_hex)?;
        let envelope: Cbor = ciborium::from_reader(bytes.as_slice()).map_err(SendError::invalid)?;
        let content = field(&envelope, "content")?;
        if hex::encode(request_hash(content)?) != self.request_id
            || field(content, "request_type")? != &text("call")
            || field(content, "sender")? != &super::icp_stages::bytes(controller)
        {
            return Err(SendError::invalid(
                "Saved ICP ingress identity differs from its content",
            ));
        }
        let (canister, method) = if self.kind == IcpStakingCallKind::Fund {
            (
                candid::Principal::from_slice(&hex::decode(
                    crate::registry::Chain::Icp.icp_ledger_id()?,
                )?)
                .to_text(),
                "send_pb",
            )
        } else {
            (
                crate::registry::Chain::Icp.icp_governance_id()?.into(),
                if self.kind == IcpStakingCallKind::Claim {
                    "claim_or_refresh_neuron_from_account"
                } else {
                    "manage_neuron"
                },
            )
        };
        let principal = candid::Principal::from_text(&canister).map_err(SendError::invalid)?;
        if self.canister != canister
            || field(content, "canister_id")? != &super::icp_stages::bytes(principal.as_slice())
            || field(content, "method_name")? != &text(method)
        {
            return Err(SendError::invalid(
                "Saved ICP ingress targets another canister or method",
            ));
        }
        let Cbor::Bytes(argument) = field(content, "arg")? else {
            return Err(SendError::invalid(
                "Saved ICP ingress argument is not a blob",
            ));
        };
        Ok(argument.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct IcpStakingReceipt {
    pub canister: String,
    pub request_id: String,
    pub kind: IcpStakingCallKind,
    pub certificate_hex: String,
    pub reply_hex: String,
}
impl IcpStakingReceipt {
    pub(crate) fn validate(&self) -> Result<(), SendError> {
        let id: [u8; 32] = hex::decode(&self.request_id)?
            .try_into()
            .map_err(|_| SendError::invalid("Invalid saved ICP request ID"))?;
        let crate::api::icp_replica::IngressStatus::Replied(reply) =
            crate::api::icp_replica::historical_certified_status(
                &hex::decode(&self.certificate_hex)?,
                &self.canister,
                &id,
            )
            .map_err(SendError::invalid)?
        else {
            return Err(SendError::invalid(
                "Saved ICP proof does not certify a reply",
            ));
        };
        if hex::encode(&reply) != self.reply_hex {
            return Err(SendError::invalid(
                "Saved ICP reply differs from its certificate",
            ));
        }
        validate_reply(&self.kind, &reply)
    }
    pub(crate) fn matches(&self, call: &SignedIcpStakingCall) -> bool {
        self.request_id == call.request_id
            && self.canister == call.canister
            && self.kind == call.kind
    }
}

pub(crate) fn neuron_subaccount(controller: &[u8], nonce: u64) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"\x0cneuron-stake");
    hash.update(controller);
    hash.update(nonce.to_be_bytes());
    hash.finalize().into()
}
pub(crate) fn funding_call(
    mut transfer: super::icp_stages::PreparedIcpTransaction,
    nonce: u64,
) -> Result<IcpStakingCall, SendError> {
    transfer.memo = nonce;
    Ok(IcpStakingCall {
        canister: candid::Principal::from_slice(&hex::decode(&transfer.ledger_canister)?).to_text(),
        method: "send_pb".into(),
        argument_hex: hex::encode(transfer.argument()?),
        nonce_hex: None,
        kind: IcpStakingCallKind::Fund,
    })
}
pub(crate) fn expiry() -> Result<u64, SendError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(SendError::invalid)?
        .as_nanos();
    u64::try_from(now)
        .ok()
        .and_then(|n| n.checked_add(240_000_000_000))
        .ok_or_else(|| SendError::invalid("ICP ingress timestamp overflow"))
}
fn self_describing(value: Cbor) -> Result<Vec<u8>, SendError> {
    encode(&Cbor::Tag(55799, Box::new(value)))
}
pub(crate) fn signed_query(
    method: &str,
    argument: &[u8],
    key: &Ed25519Seed,
) -> Result<Vec<u8>, SendError> {
    let canister = candid::Principal::from_text(crate::registry::Chain::Icp.icp_governance_id()?)
        .map_err(SendError::invalid)?;
    self_describing(envelope(
        map(vec![
            ("request_type", text("query")),
            ("canister_id", bytes(canister.as_slice())),
            ("method_name", text(method)),
            ("arg", bytes(argument)),
            ("sender", bytes(&principal(&key.public_key()))),
            ("ingress_expiry", Cbor::Integer(expiry()?.into())),
        ]),
        key,
    )?)
}
pub(crate) fn read_state(
    id: &[u8; 32],
    expiry_ns: u64,
    key: &Ed25519Seed,
) -> Result<Vec<u8>, SendError> {
    self_describing(envelope(
        map(vec![
            ("request_type", text("read_state")),
            ("sender", bytes(&principal(&key.public_key()))),
            ("ingress_expiry", Cbor::Integer(expiry_ns.into())),
            (
                "paths",
                Cbor::Array(vec![Cbor::Array(vec![bytes(b"request_status"), bytes(id)])]),
            ),
        ]),
        key,
    )?)
}
impl PreparedIcpStaking {
    pub(crate) fn validate_original_funding(&self, controller: &[u8]) -> Result<(), SendError> {
        let funding = self
            .funding
            .as_ref()
            .ok_or_else(|| SendError::invalid("Original typed funding transfer is absent"))?;
        let nonce = self
            .neuron_nonce
            .ok_or_else(|| SendError::invalid("Original neuron nonce is absent"))?;
        let subaccount = neuron_subaccount(controller, nonce);
        let governance =
            candid::Principal::from_text(crate::registry::Chain::Icp.icp_governance_id()?)
                .map_err(SendError::invalid)?;
        if self.controller_hex != hex::encode(controller)
            || self.sender != hex::encode(account_from_principal(controller))
            || self.subaccount_hex != hex::encode(subaccount)
            || funding.sender != self.sender
            || funding.recipient
                != hex::encode(crate::derivation::icp::account_with_subaccount(
                    governance.as_slice(),
                    &subaccount,
                ))
            || funding.memo != nonce
            || funding.amount != self.amount
            || funding.ledger_canister != crate::registry::Chain::Icp.icp_ledger_id()?
            || funding.argument_hex != hex::encode(funding.argument()?)
            || self.funding_ledger_hash.as_deref() != Some(funding.transaction_hash()?.as_str())
        {
            return Err(SendError::invalid(
                "Original funding hash or transfer differs from the derived neuron",
            ));
        }
        Ok(())
    }

    pub(crate) fn retain_attempts_for_repair(
        &mut self,
        attempts: &mut Vec<super::stages::BroadcastAttempt>,
        selected_endpoints: &mut Vec<String>,
    ) {
        self.prior_attempts.append(attempts);
        selected_endpoints.clear();
    }

    pub(crate) fn sign(&self, key: &Ed25519Seed) -> Result<Vec<SignedIcpStakingCall>, SendError> {
        let controller = principal(&key.public_key());
        if hex::encode(&controller) != self.controller_hex
            || hex::encode(account_from_principal(&controller)) != self.sender
        {
            return Err(SendError::invalid(
                "ICP staking controller differs from signing identity",
            ));
        }
        if let Some(nonce) = self.neuron_nonce {
            let derived = neuron_subaccount(&controller, nonce);
            if self.subaccount_hex != hex::encode(derived) {
                return Err(SendError::invalid(
                    "Neuron subaccount differs from its controller and nonce",
                ));
            }
        }
        let has_completed_fund = self
            .completed_calls
            .iter()
            .any(|call| call.kind == IcpStakingCallKind::Fund);
        if has_completed_fund {
            self.validate_original_funding(&controller)?;
            for call in self
                .completed_calls
                .iter()
                .filter(|call| call.kind == IcpStakingCallKind::Fund)
            {
                if call.validated_argument(&controller)?
                    != self
                        .funding
                        .as_ref()
                        .ok_or_else(|| SendError::invalid("Missing completed funding transfer"))?
                        .argument()?
                {
                    return Err(SendError::invalid(
                        "Completed funding ingress differs from the original transfer",
                    ));
                }
            }
        }
        for (index, call) in self.calls.iter().enumerate() {
            let governance = crate::registry::Chain::Icp.icp_governance_id()?;
            if call.kind == IcpStakingCallKind::Fund {
                if index != 0 || has_completed_fund {
                    return Err(SendError::invalid(
                        "A repaired neuron cannot be funded again",
                    ));
                }
                let funding = self
                    .funding
                    .as_ref()
                    .ok_or_else(|| SendError::invalid("Funding transfer is missing"))?;
                self.validate_original_funding(&controller)?;
                let nonce = self
                    .neuron_nonce
                    .ok_or_else(|| SendError::invalid("Funding neuron nonce is missing"))?;
                let gov = candid::Principal::from_text(governance).map_err(SendError::invalid)?;
                let recipient = hex::encode(crate::derivation::icp::account_with_subaccount(
                    gov.as_slice(),
                    &neuron_subaccount(&controller, nonce),
                ));
                let ledger = candid::Principal::from_slice(&hex::decode(
                    crate::registry::Chain::Icp.icp_ledger_id()?,
                )?)
                .to_text();
                if call.canister != ledger
                    || call.method != "send_pb"
                    || funding.sender != self.sender
                    || funding.recipient != recipient
                    || funding.amount != self.amount
                    || funding.fee != self.fee
                    || funding.memo != nonce
                    || call.argument_hex != hex::encode(funding.argument()?)
                    || self.funding_ledger_hash.as_deref()
                        != Some(funding.transaction_hash()?.as_str())
                {
                    return Err(SendError::invalid(
                        "ICP funding transfer differs from the derived reviewed neuron",
                    ));
                }
            } else if call.canister != governance
                || call.method
                    != if call.kind == IcpStakingCallKind::Claim {
                        "claim_or_refresh_neuron_from_account"
                    } else {
                        "manage_neuron"
                    }
            {
                return Err(SendError::invalid(
                    "ICP governance step targets an unexpected canister or method",
                ));
            }
        }
        if self.calls.is_empty() || self.calls.len() > 6 {
            return Err(SendError::invalid("Invalid ICP staking steps"));
        }
        self.calls
            .iter()
            .map(|call| {
                let canister =
                    candid::Principal::from_text(&call.canister).map_err(SendError::invalid)?;
                let mut fields = vec![
                    ("request_type", text("call")),
                    ("canister_id", bytes(canister.as_slice())),
                    ("method_name", text(&call.method)),
                    ("arg", bytes(&hex::decode(&call.argument_hex)?)),
                    ("sender", bytes(&controller)),
                    (
                        "ingress_expiry",
                        Cbor::Integer(self.ingress_expiry_ns.into()),
                    ),
                ];
                if let Some(nonce) = &call.nonce_hex {
                    fields.push(("nonce", bytes(&hex::decode(nonce)?)));
                }
                let content = map(fields);
                let id = request_hash(&content)?;
                Ok(SignedIcpStakingCall {
                    canister: call.canister.clone(),
                    request_id: hex::encode(id),
                    update_hex: hex::encode(self_describing(envelope(content, key)?)?),
                    read_state_hex: hex::encode(read_state(&id, self.ingress_expiry_ns, key)?),
                    kind: call.kind.clone(),
                })
            })
            .collect()
    }
}

/// Validate the canister's *result*, as a replied ingress may contain a
/// governance error. Unknown or pruned replies do not become confirmation.
pub(crate) fn validate_reply(kind: &IcpStakingCallKind, bytes: &[u8]) -> Result<(), SendError> {
    let refuse = SendError::invalid;
    if *kind == IcpStakingCallKind::Fund {
        // ICP send_pb returns BlockIndex { block_height: uint64 }; a block-zero
        // message is also legal protobuf. Refuse arbitrary/error-shaped bytes.
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.first() != Some(&8) || bytes.len() > 11 {
            return Err(refuse("Invalid certified ICP ledger reply"));
        }
        let mut finished = false;
        for (i, b) in bytes[1..].iter().enumerate() {
            if i == 9 && *b > 1 {
                return Err(refuse("ICP block height overflows"));
            }
            if b & 128 == 0 {
                if i + 2 != bytes.len() {
                    return Err(refuse("Unexpected ICP ledger reply fields"));
                }
                finished = true;
                break;
            }
        }
        return if finished {
            Ok(())
        } else {
            Err(refuse("Truncated ICP ledger reply"))
        };
    }
    if *kind == IcpStakingCallKind::Claim {
        return match candid::decode_one::<ClaimResponse>(bytes)
            .map_err(SendError::invalid)?
            .result
        {
            Some(ClaimResult::NeuronId(id)) if id.id > 0 => Ok(()),
            Some(ClaimResult::Error(e)) => Err(SendError::invalid(format!(
                "Neuron claim refused ({}): {}",
                e.error_type, e.error_message
            ))),
            _ => Err(refuse("Missing certified neuron claim result")),
        };
    }
    let command = candid::decode_one::<ManageResponse>(bytes)
        .map_err(SendError::invalid)?
        .command;
    match (kind, command) {
        (_, Some(ManageReply::Error(e))) => Err(SendError::invalid(format!(
            "Neuron operation refused ({}): {}",
            e.error_type, e.error_message
        ))),
        (IcpStakingCallKind::Configure, Some(ManageReply::Configure(_)))
        | (IcpStakingCallKind::Follow, Some(ManageReply::Follow(_)))
        | (IcpStakingCallKind::Disburse, Some(ManageReply::Disburse(_))) => Ok(()),
        (IcpStakingCallKind::DisburseMaturity, Some(ManageReply::DisburseMaturity(reply)))
            if reply.amount_disbursed_e8s.is_some_and(|amount| amount > 0) =>
        {
            Ok(())
        }
        _ => Err(refuse(
            "Certified governance result differs from reviewed operation",
        )),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn new_neuron_fixture() -> (Ed25519Seed, PreparedIcpStaking) {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/icp-staking-vectors.json"
        ))
        .unwrap();
        let key = Ed25519Seed::from_hex(&"01".repeat(32)).unwrap();
        let controller = principal(&key.public_key());
        let subaccount = neuron_subaccount(&controller, 42);
        let governance =
            candid::Principal::from_text(crate::registry::Chain::Icp.icp_governance_id().unwrap())
                .unwrap();
        let mut funding = super::super::icp_stages::PreparedIcpTransaction {
            sender: fixture["owner"].as_str().unwrap().into(),
            recipient: fixture["neuron_account"].as_str().unwrap().into(),
            amount: 100_000_000,
            fee: 10_000,
            memo: 42,
            created_at_time_ns: 1_800_000_000_000_000_000,
            ingress_expiry_ns: 1_800_000_240_000_000_000,
            ledger_canister: crate::registry::Chain::Icp.icp_ledger_id().unwrap().into(),
            argument_hex: String::new(),
        };
        funding.argument_hex = hex::encode(funding.argument().unwrap());
        let mut calls = vec![funding_call(funding.clone(), 42).unwrap()];
        for vector in fixture["calls"].as_array().unwrap().iter().take(2) {
            calls.push(IcpStakingCall {
                canister: governance.to_text(),
                method: vector["method"].as_str().unwrap().into(),
                argument_hex: vector["argument_hex"].as_str().unwrap().into(),
                nonce_hex: vector["nonce_hex"].as_str().map(str::to_string),
                kind: serde_json::from_value(vector["kind"].clone()).unwrap(),
            });
        }
        (
            key,
            PreparedIcpStaking {
                sender: funding.sender.clone(),
                controller_hex: hex::encode(controller),
                ingress_expiry_ns: funding.ingress_expiry_ns,
                amount: funding.amount,
                fee: funding.fee,
                neuron_nonce: Some(42),
                subaccount_hex: hex::encode(subaccount),
                calls,
                funding_ledger_hash: Some(funding.transaction_hash().unwrap()),
                funding: Some(funding),
                completed_calls: vec![],
                prior_calls: vec![],
                prior_attempts: vec![],
                funding_confirmed_by_ledger: false,
                recovered_configured_neuron: None,
            },
        )
    }

    #[test]
    fn funding_proof_requires_the_exact_original_controller_subaccount_and_ledger_hash() {
        let (key, original) = new_neuron_fixture();
        let controller = principal(&key.public_key());
        original.validate_original_funding(&controller).unwrap();
        type Mutation = fn(&mut PreparedIcpStaking);
        let mutations: [Mutation; 9] = [
            |p| p.neuron_nonce = Some(43),
            |p| p.subaccount_hex = "00".repeat(32),
            |p| p.funding_ledger_hash = Some("00".repeat(32)),
            |p| p.funding.as_mut().unwrap().amount += 1,
            |p| p.funding.as_mut().unwrap().memo += 1,
            |p| p.funding.as_mut().unwrap().recipient = p.sender.clone(),
            |p| p.funding.as_mut().unwrap().sender = "00".repeat(32),
            |p| p.funding.as_mut().unwrap().ledger_canister = "00".into(),
            |p| p.funding.as_mut().unwrap().argument_hex.push_str("00"),
        ];
        for (index, mutate) in mutations.into_iter().enumerate() {
            let mut altered = original.clone();
            mutate(&mut altered);
            assert!(
                altered.validate_original_funding(&controller).is_err(),
                "mutation {index}"
            );
        }
        assert!(
            original
                .validate_original_funding(&principal(
                    &Ed25519Seed::from_hex(&"02".repeat(32))
                        .unwrap()
                        .public_key()
                ))
                .is_err()
        );
    }

    #[test]
    fn repaired_revision_can_sign_management_again_and_never_signs_another_fund() {
        let (key, mut prepared) = new_neuron_fixture();
        let first = prepared.sign(&key).unwrap();
        let first_fund = first[0].clone();
        let original_hash = prepared.funding_ledger_hash.clone();
        prepared.completed_calls.push(first_fund.clone());
        prepared.prior_calls = first.clone();
        prepared.calls.remove(0);
        prepared.ingress_expiry_ns += 1;
        prepared.fee = 0;
        prepared.funding_confirmed_by_ledger = true;
        let mut attempts = vec![super::super::stages::BroadcastAttempt {
            endpoint: "https://icp-api.io".into(),
            attempted_at: 1.0,
            outcome: super::super::stages::SubmissionOutcome::Uncertain,
            transaction_hash: first.last().map(|call| call.request_id.clone()),
            detail: "Interrupted after funding".into(),
        }];
        let mut selected = vec!["https://icp-api.io".into()];
        prepared.retain_attempts_for_repair(&mut attempts, &mut selected);
        assert!(attempts.is_empty());
        assert!(selected.is_empty());
        let reopened: PreparedIcpStaking =
            serde_json::from_str(&serde_json::to_string(&prepared).unwrap()).unwrap();
        assert_eq!(reopened.prior_attempts.len(), 1);
        assert_eq!(reopened.prior_calls[0].request_id, first_fund.request_id);
        assert_eq!(reopened.funding_ledger_hash, original_hash);
        let renewed = reopened.sign(&key).unwrap();
        assert!(
            renewed
                .iter()
                .all(|call| call.kind != IcpStakingCallKind::Fund)
        );
        assert_ne!(renewed[0].request_id, first[1].request_id);
        assert_eq!(
            renewed[0]
                .validated_argument(&principal(&key.public_key()))
                .unwrap(),
            first[1]
                .validated_argument(&principal(&key.public_key()))
                .unwrap()
        );
        prepared.calls.insert(
            0,
            funding_call(prepared.funding.clone().unwrap(), 42).unwrap(),
        );
        assert!(
            prepared
                .sign(&key)
                .unwrap_err()
                .to_string()
                .contains("cannot be funded again")
        );
    }

    #[test]
    fn saved_ingress_is_bound_to_its_request_content_network_and_controller() {
        let (key, prepared) = new_neuron_fixture();
        let controller = principal(&key.public_key());
        let original = prepared.sign(&key).unwrap().remove(0);
        original.validated_argument(&controller).unwrap();
        let mut altered = original.clone();
        altered.request_id = "00".repeat(32);
        assert!(altered.validated_argument(&controller).is_err());
        let mut altered = original.clone();
        altered.canister = crate::registry::Chain::Icp
            .icp_governance_id()
            .unwrap()
            .into();
        assert!(altered.validated_argument(&controller).is_err());
        let mut altered = original;
        altered.kind = IcpStakingCallKind::Configure;
        assert!(altered.validated_argument(&controller).is_err());
    }
    #[test]
    fn all_governance_request_ids_and_signatures_match_independent_dfinity_sdk_vectors() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/icp-staking-vectors.json"
        ))
        .unwrap();
        let key = Ed25519Seed::from_hex(&"01".repeat(32)).unwrap();
        let controller = principal(&key.public_key());
        assert_eq!(hex::encode(key.public_key()), fixture["public_key"]);
        assert_eq!(
            candid::Principal::from_slice(&controller).to_text(),
            fixture["controller"]
        );
        assert_eq!(
            hex::encode(neuron_subaccount(&controller, 42)),
            fixture["subaccount"]
        );
        let sub = neuron_subaccount(&controller, 42);
        let gov =
            candid::Principal::from_text(crate::registry::Chain::Icp.icp_governance_id().unwrap())
                .unwrap();
        assert_eq!(
            hex::encode(crate::derivation::icp::account_with_subaccount(
                gov.as_slice(),
                &sub
            )),
            fixture["neuron_account"]
        );
        fn signature(hex_bytes: &str) -> String {
            let bytes = hex::decode(hex_bytes).unwrap();
            let value: Cbor = ciborium::from_reader(bytes.as_slice()).unwrap();
            let value = match value {
                Cbor::Tag(_, value) => *value,
                value => value,
            };
            let Cbor::Map(fields) = value else {
                panic!("invalid envelope")
            };
            let (_, Cbor::Bytes(sig)) = fields
                .iter()
                .find(|(k, _)| k == &Cbor::Text("sender_sig".into()))
                .unwrap()
            else {
                panic!("signature absent")
            };
            hex::encode(sig)
        }
        for vector in fixture["calls"].as_array().unwrap() {
            let call = IcpStakingCall {
                canister: gov.to_text(),
                method: vector["method"].as_str().unwrap().into(),
                argument_hex: vector["argument_hex"].as_str().unwrap().into(),
                nonce_hex: vector["nonce_hex"].as_str().map(str::to_string),
                kind: serde_json::from_value(vector["kind"].clone()).unwrap(),
            };
            let prepared = PreparedIcpStaking {
                sender: fixture["owner"].as_str().unwrap().into(),
                controller_hex: hex::encode(&controller),
                ingress_expiry_ns: fixture["ingress_expiry"].as_str().unwrap().parse().unwrap(),
                amount: 0,
                fee: 0,
                neuron_nonce: Some(42),
                subaccount_hex: hex::encode(sub),
                calls: vec![call],
                funding_ledger_hash: None,
                funding: None,
                completed_calls: vec![],
                prior_calls: vec![],
                prior_attempts: vec![],
                funding_confirmed_by_ledger: false,
                recovered_configured_neuron: None,
            };
            let signed = prepared.sign(&key).unwrap().remove(0);
            assert_eq!(signed.request_id, vector["request_id"]);
            assert_eq!(signature(&signed.update_hex), vector["signature"]);
            assert_eq!(
                signature(&signed.read_state_hex),
                vector["read_state_signature"]
            );
        }
        for (name, kind) in [
            ("claim", IcpStakingCallKind::Claim),
            ("configure", IcpStakingCallKind::Configure),
            ("follow", IcpStakingCallKind::Follow),
            ("disburse", IcpStakingCallKind::Disburse),
            ("maturity", IcpStakingCallKind::DisburseMaturity),
        ] {
            validate_reply(
                &kind,
                &hex::decode(fixture["replies"][name].as_str().unwrap()).unwrap(),
            )
            .unwrap();
            assert!(
                validate_reply(
                    &kind,
                    &hex::decode(fixture["replies"]["error"].as_str().unwrap()).unwrap()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn controller_and_request_ids_bind_all_staking_steps() {
        let key = Ed25519Seed::from_hex(&"01".repeat(32)).unwrap();
        let owner = principal(&key.public_key());
        let mut p = PreparedIcpStaking {
            sender: hex::encode(account_from_principal(&owner)),
            controller_hex: hex::encode(owner),
            ingress_expiry_ns: 123,
            amount: 1,
            fee: 0,
            neuron_nonce: Some(42),
            subaccount_hex: hex::encode(neuron_subaccount(&principal(&key.public_key()), 42)),
            funding_ledger_hash: None,
            funding: None,
            completed_calls: vec![],
            prior_calls: vec![],
            prior_attempts: vec![],
            funding_confirmed_by_ledger: false,
            recovered_configured_neuron: None,
            calls: vec![IcpStakingCall {
                canister: crate::registry::Chain::Icp
                    .icp_governance_id()
                    .unwrap()
                    .into(),
                method: "manage_neuron".into(),
                argument_hex: "4449444c0000".into(),
                nonce_hex: None,
                kind: IcpStakingCallKind::Configure,
            }],
        };
        let first = p.sign(&key).unwrap();
        p.calls[0].argument_hex.push_str("00");
        assert_ne!(first[0].request_id, p.sign(&key).unwrap()[0].request_id);
        assert!(
            p.sign(&Ed25519Seed::from_hex(&"02".repeat(32)).unwrap())
                .is_err()
        );
        assert!(validate_reply(&IcpStakingCallKind::Configure, &[]).is_err());
    }
}
