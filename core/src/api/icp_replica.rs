//! IC replica CBOR transport and NNS Candid DTOs. Execution is read from the
//! exact ingress path and verified against the registry's pinned mainnet key.
use super::error::ApiError;
use super::http::{HttpClient, RetryProfile, race};
use crate::registry::Chain;
use candid::{CandidType, Principal};
use ciborium::Value;
use ic_cbor::CertificateToCbor;
use ic_certificate_verification::VerifyCertificate;
use ic_certification::{Certificate, LookupResult};
use serde::Deserialize;
use std::sync::Arc;

pub(crate) struct IcpReplicaClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IngressStatus {
    Pending,
    Processing,
    Unknown,
    Replied(Vec<u8>),
    Rejected(String),
    Done,
}

#[derive(Debug, Clone)]
pub(crate) struct CertifiedIngressStatus {
    pub status: IngressStatus,
    pub certificate: Vec<u8>,
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ApiError> {
    if let Value::Tag(55799, value) = value {
        return field(value, name);
    }
    let Value::Map(map) = value else {
        return Err(ApiError::decode("IC response is not a CBOR map"));
    };
    map.iter()
        .find(|(key, _)| key == &Value::Text(name.into()))
        .map(|(_, v)| v)
        .ok_or_else(|| ApiError::decode(format!("Missing IC response {name}")))
}
fn blob(value: &Value) -> Result<&[u8], ApiError> {
    match value {
        Value::Bytes(bytes) => Ok(bytes),
        _ => Err(ApiError::decode("IC response expected a blob")),
    }
}

impl IcpReplicaClient {
    pub(crate) fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    /// Read-only availability diagnostic. The status endpoint is not a
    /// certified identity source; signing/receipts still use the pinned key.
    /// https://docs.internetcomputer.org/references/ic-interface-spec/https-interface/#status-endpoint
    pub(crate) async fn health(&self) -> Result<(), ApiError> {
        race(&self.endpoints, |base| async move {
            let url = format!("{}/api/v2/status", base.trim_end_matches('/'));
            if super::http::refuse_non_loopback(&url) {
                return Err(ApiError::Transport("non-loopback endpoint refused".into()));
            }
            let mut response = self
                .client
                .reqwest_client()
                .get(&url)
                .header("Accept", "application/cbor")
                .send()
                .await
                .map_err(|error| ApiError::Transport(error.to_string()))?;
            let status = response.status().as_u16();
            if status != 200 {
                return Err(ApiError::Status {
                    status,
                    body: "ICP replica status endpoint refused the diagnostic".into(),
                });
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| ApiError::Transport(error.to_string()))?
            {
                if bytes.len().saturating_add(chunk.len()) > 64 * 1024 {
                    return Err(ApiError::decode("ICP replica status exceeds size limit"));
                }
                bytes.extend_from_slice(&chunk);
            }
            validate_health(&bytes)
        })
        .await
    }
    async fn post(
        &self,
        canister: &str,
        route: &str,
        body: &[u8],
        profile: RetryProfile,
    ) -> Result<Vec<u8>, ApiError> {
        Principal::from_text(canister).map_err(ApiError::invalid)?;
        race(&self.endpoints, |base| {
            let body = body.to_vec();
            let client = self.client.clone();
            let url = format!(
                "{}/api/v2/canister/{canister}/{route}",
                base.trim_end_matches('/')
            );
            async move {
                let (status, reply) = client
                    .post_bytes(&url, "application/cbor", body, profile)
                    .await?;
                if !(200..300).contains(&status) {
                    return Err(ApiError::Status {
                        status,
                        body: String::from_utf8_lossy(&reply).into(),
                    });
                }
                Ok(reply)
            }
        })
        .await
    }
    /// Endpoint reads have the same trust model as other JSON-RPC reads;
    /// they do not substitute for certified execution receipts.
    pub(crate) async fn query(&self, body: &[u8]) -> Result<Vec<u8>, ApiError> {
        let canister = Chain::Icp.icp_governance_id().map_err(ApiError::invalid)?;
        let bytes = self
            .post(canister, "query", body, RetryProfile::ChainRead)
            .await?;
        let value: Value = ciborium::from_reader(bytes.as_slice()).map_err(ApiError::decode)?;
        match field(&value, "status")? {
            Value::Text(status) if status == "replied" => {
                Ok(blob(field(field(&value, "reply")?, "arg")?)?.to_vec())
            }
            Value::Text(status) if status == "rejected" => Err(ApiError::rejected(format!(
                "IC query rejected: {:?}",
                field(&value, "reject_message")?
            ))),
            _ => Err(ApiError::decode("Unrecognised IC query status")),
        }
    }
    pub(crate) async fn submit(&self, canister: &str, body: &[u8]) -> Result<(), ApiError> {
        self.post(canister, "call", body, RetryProfile::ChainWrite)
            .await?;
        Ok(())
    }
    pub(crate) async fn status(
        &self,
        canister: &str,
        request_id: &[u8; 32],
        read_state: &[u8],
    ) -> Result<CertifiedIngressStatus, ApiError> {
        let bytes = self
            .post(canister, "read_state", read_state, RetryProfile::ChainRead)
            .await?;
        let value: Value = ciborium::from_reader(bytes.as_slice()).map_err(ApiError::decode)?;
        let certificate = blob(field(&value, "certificate")?)?.to_vec();
        Ok(CertifiedIngressStatus {
            status: certified_status(&certificate, canister, request_id)?,
            certificate,
        })
    }
}

/// Official implementation says clients should check only equality with
/// Healthy, rather than interpreting the replica's initialization stages.
/// https://github.com/dfinity/ic/blob/master/rs/types/types/src/messages/http.rs
fn validate_health(bytes: &[u8]) -> Result<(), ApiError> {
    let mut cursor = std::io::Cursor::new(bytes);
    let value: Value = ciborium::from_reader(&mut cursor).map_err(ApiError::decode)?;
    if cursor.position() != bytes.len() as u64 {
        return Err(ApiError::decode(
            "ICP replica status has trailing CBOR data",
        ));
    }
    let value = match &value {
        Value::Tag(55799, value) => value.as_ref(),
        value => value,
    };
    let Value::Map(map) = value else {
        return Err(ApiError::decode("ICP replica status is not a CBOR map"));
    };
    let mut fields = std::collections::HashSet::new();
    for (key, _) in map {
        let Value::Text(key) = key else {
            return Err(ApiError::decode("ICP replica status field is not text"));
        };
        if !fields.insert(key) {
            return Err(ApiError::decode(
                "ICP replica status contains duplicate fields",
            ));
        }
    }
    match field(value, "replica_health_status")? {
        Value::Text(status) if status == "healthy" => Ok(()),
        Value::Text(status) => Err(ApiError::decode(format!(
            "ICP replica is not healthy: {status}"
        ))),
        _ => Err(ApiError::decode("ICP replica health status is not text")),
    }
}

fn certified_status(
    bytes: &[u8],
    canister: &str,
    id: &[u8; 32],
) -> Result<IngressStatus, ApiError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(ApiError::decode)?
        .as_nanos();
    verified_status(bytes, canister, id, now, 300_000_000_000)
}

/// A saved execution proof is historical: verify its root signature, delegation,
/// canister range and exact request path at the certificate's own signed time.
/// Its age does not turn a previously proved execution back into uncertainty.
pub(crate) fn historical_certified_status(
    bytes: &[u8],
    canister: &str,
    id: &[u8; 32],
) -> Result<IngressStatus, ApiError> {
    let certificate = Certificate::from_cbor(bytes).map_err(ApiError::decode)?;
    let LookupResult::Found(time) = certificate.tree.lookup_path([b"time".as_slice()]) else {
        return Err(ApiError::decode(
            "Historical IC certificate has no witnessed time",
        ));
    };
    let mut timestamp = 0u128;
    let mut finished = false;
    for (i, byte) in time.iter().copied().enumerate() {
        if i >= 19 || (i == 18 && byte > 3) {
            return Err(ApiError::decode("IC certificate time overflows"));
        }
        timestamp |= u128::from(byte & 127) << (i * 7);
        if byte & 128 == 0 {
            if i + 1 != time.len() {
                return Err(ApiError::decode("IC certificate time has trailing bytes"));
            }
            finished = true;
            break;
        }
    }
    if !finished {
        return Err(ApiError::decode("IC certificate time is truncated"));
    }
    verified_status(bytes, canister, id, timestamp, 0)
}
fn verified_status(
    bytes: &[u8],
    canister: &str,
    id: &[u8; 32],
    now: u128,
    offset: u128,
) -> Result<IngressStatus, ApiError> {
    let certificate = Certificate::from_cbor(bytes).map_err(ApiError::decode)?;
    let canister = Principal::from_text(canister).map_err(ApiError::invalid)?;
    let ledger = Principal::from_slice(
        &hex::decode(Chain::Icp.icp_ledger_id().map_err(ApiError::invalid)?)
            .map_err(ApiError::invalid)?,
    );
    let governance =
        Principal::from_text(Chain::Icp.icp_governance_id().map_err(ApiError::invalid)?)
            .map_err(ApiError::invalid)?;
    if canister != ledger && canister != governance {
        return Err(ApiError::invalid(
            "ICP staking proof targets an unsupported canister",
        ));
    }
    certificate
        .verify(
            canister.as_slice(),
            &Chain::Icp.icp_root_key().map_err(ApiError::invalid)?,
            &now,
            &offset,
        )
        .map_err(ApiError::decode)?;
    let lookup = |key: &[u8]| {
        certificate
            .tree
            .lookup_path([b"request_status".as_slice(), id.as_slice(), key])
    };
    match lookup(b"status") {
        LookupResult::Found(b"replied") => match lookup(b"reply") {
            LookupResult::Found(reply) => Ok(IngressStatus::Replied(reply.to_vec())),
            _ => Err(ApiError::decode("Certified IC reply is absent or pruned")),
        },
        LookupResult::Found(b"rejected") => match lookup(b"reject_message") {
            LookupResult::Found(message) => Ok(IngressStatus::Rejected(
                String::from_utf8_lossy(message).into(),
            )),
            _ => Err(ApiError::decode(
                "Certified IC rejection is absent or pruned",
            )),
        },
        LookupResult::Found(b"done") => Ok(IngressStatus::Done),
        LookupResult::Found(b"received" | b"processing") => Ok(IngressStatus::Processing),
        LookupResult::Absent => Ok(IngressStatus::Pending),
        LookupResult::Unknown => Ok(IngressStatus::Unknown),
        _ => Err(ApiError::decode("Unrecognised certified IC ingress status")),
    }
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub(crate) struct NeuronId {
    pub id: u64,
}
#[derive(CandidType, Deserialize, Debug, Clone)]
pub(crate) enum DissolveState {
    DissolveDelaySeconds(u64),
    WhenDissolvedTimestampSeconds(u64),
}
#[derive(CandidType, Deserialize, Debug, Clone)]
pub(crate) struct Neuron {
    pub id: Option<NeuronId>,
    pub controller: Option<Principal>,
    pub account: Vec<u8>,
    pub cached_neuron_stake_e8s: u64,
    pub neuron_fees_e8s: u64,
    pub maturity_e8s_equivalent: u64,
    pub staked_maturity_e8s_equivalent: Option<u64>,
    pub dissolve_state: Option<DissolveState>,
    pub spawn_at_timestamp_seconds: Option<u64>,
    pub maturity_disbursements_in_progress: Option<Vec<MaturityDisbursement>>,
}
#[derive(CandidType, Deserialize, Debug, Clone)]
pub(crate) struct MaturityDisbursement {
    pub amount_e8s: Option<u64>,
    pub timestamp_of_disbursement_seconds: Option<u64>,
    pub finalize_disbursement_timestamp_seconds: Option<u64>,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct ListNeurons {
    pub neuron_ids: Vec<u64>,
    pub include_neurons_readable_by_caller: bool,
    pub include_empty_neurons_readable_by_caller: Option<bool>,
    pub include_public_neurons_in_full_neurons: Option<bool>,
    pub page_number: Option<u64>,
    pub page_size: Option<u64>,
    pub neuron_subaccounts: Option<Vec<NeuronSubaccount>>,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct NeuronSubaccount {
    pub subaccount: Vec<u8>,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct ListNeuronsResponse {
    pub full_neurons: Vec<Neuron>,
    pub total_pages_available: Option<u64>,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct NetworkEconomics {
    pub neuron_minimum_stake_e8s: u64,
    pub transaction_fee_e8s: u64,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct KnownNeuronData {
    pub name: String,
    pub description: Option<String>,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct KnownNeuron {
    pub id: Option<NeuronId>,
    pub known_neuron_data: Option<KnownNeuronData>,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct ListKnownNeuronsResponse {
    pub known_neurons: Vec<KnownNeuron>,
}

#[derive(CandidType, Deserialize)]
pub(crate) struct Empty {}
#[derive(CandidType, Deserialize)]
pub(crate) struct ClaimNeuron {
    pub memo: u64,
    pub controller: Option<Principal>,
}
#[derive(CandidType, Deserialize)]
pub(crate) enum NeuronSelector {
    Subaccount(Vec<u8>),
    NeuronId(NeuronId),
}
#[derive(CandidType, Deserialize)]
pub(crate) struct ManageNeuron {
    pub id: Option<NeuronId>,
    pub neuron_id_or_subaccount: Option<NeuronSelector>,
    pub command: Option<ManageCommand>,
}
#[derive(CandidType, Deserialize)]
pub(crate) enum ManageCommand {
    Configure(Configure),
    Disburse(Disburse),
    Follow(Follow),
    DisburseMaturity(DisburseMaturity),
}
#[derive(CandidType, Deserialize)]
pub(crate) struct Configure {
    pub operation: Option<ConfigureOperation>,
}
#[derive(CandidType, Deserialize)]
pub(crate) enum ConfigureOperation {
    StartDissolving(Empty),
    IncreaseDissolveDelay(IncreaseDissolveDelay),
}
#[derive(CandidType, Deserialize, Debug, Clone)]
pub(crate) struct IncreaseDissolveDelay {
    pub additional_dissolve_delay_seconds: u32,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct AccountIdentifier {
    pub hash: Vec<u8>,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct Disburse {
    pub to_account: Option<AccountIdentifier>,
    pub amount: Option<Amount>,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct Amount {
    pub e8s: u64,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct Follow {
    pub topic: i32,
    pub followees: Vec<NeuronId>,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct IcrcAccount {
    pub owner: Option<Principal>,
    pub subaccount: Option<Vec<u8>>,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct DisburseMaturity {
    pub percentage_to_disburse: u32,
    pub to_account: Option<IcrcAccount>,
    pub to_account_identifier: Option<AccountIdentifier>,
}

#[derive(CandidType, Deserialize, Debug)]
pub(crate) struct GovernanceError {
    pub error_type: i32,
    pub error_message: String,
}
#[derive(CandidType, Deserialize)]
pub(crate) enum ClaimResult {
    NeuronId(NeuronId),
    Error(GovernanceError),
}
#[derive(CandidType, Deserialize)]
pub(crate) struct ClaimResponse {
    pub result: Option<ClaimResult>,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct EmptyResponse {}
#[derive(CandidType, Deserialize)]
pub(crate) struct DisburseResponse {
    pub transfer_block_height: u64,
}
#[derive(CandidType, Deserialize)]
pub(crate) struct DisburseMaturityResponse {
    pub amount_disbursed_e8s: Option<u64>,
}
#[derive(CandidType, Deserialize)]
pub(crate) enum ManageReply {
    Error(GovernanceError),
    Configure(EmptyResponse),
    Follow(EmptyResponse),
    Disburse(DisburseResponse),
    DisburseMaturity(DisburseMaturityResponse),
}
#[derive(CandidType, Deserialize)]
pub(crate) struct ManageResponse {
    pub command: Option<ManageReply>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_root_verifies_saved_absent_ingress_and_refuses_tampering() {
        let bytes = hex::decode(
            include_str!("../../tests/fixtures/icp-absent-ingress-certificate.hex").trim(),
        )
        .unwrap();
        let request: [u8; 32] =
            hex::decode("231dcf5739b91235eef6193dc67c008ce1b8d2a7d2221552510832a82483c2dd")
                .unwrap()
                .try_into()
                .unwrap();
        assert_eq!(
            historical_certified_status(&bytes, Chain::Icp.icp_governance_id().unwrap(), &request)
                .unwrap(),
            IngressStatus::Pending,
        );
        assert!(historical_certified_status(&bytes, "2vxsx-fae", &request).is_err());
        let mut certificate: Value = ciborium::from_reader(bytes.as_slice()).unwrap();
        let inner = match &mut certificate {
            Value::Tag(55799, inner) => inner.as_mut(),
            value => value,
        };
        let Value::Map(map) = inner else {
            panic!("certificate map")
        };
        let (_, Value::Bytes(signature)) = map
            .iter_mut()
            .find(|(key, _)| key == &Value::Text("signature".into()))
            .unwrap()
        else {
            panic!("certificate signature")
        };
        signature[0] ^= 1;
        let mut altered = Vec::new();
        ciborium::into_writer(&certificate, &mut altered).unwrap();
        assert!(
            historical_certified_status(
                &altered,
                Chain::Icp.icp_governance_id().unwrap(),
                &request
            )
            .is_err()
        );
    }
    #[test]
    fn uncertified_reply_cannot_claim_success() {
        let bogus = Value::Map(vec![
            (
                Value::Text("tree".into()),
                Value::Array(vec![Value::Integer(0.into())]),
            ),
            (Value::Text("signature".into()), Value::Bytes(vec![0; 48])),
        ]);
        let mut bytes = Vec::new();
        ciborium::into_writer(&bogus, &mut bytes).unwrap();
        assert!(
            certified_status(&bytes, Chain::Icp.icp_governance_id().unwrap(), &[1; 32]).is_err()
        );
    }
}
