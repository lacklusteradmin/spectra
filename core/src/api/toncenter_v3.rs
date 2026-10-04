//! The TON Center v3 adapter: the indexer that enumerates jetton wallets and
//! reads jetton masters, which v2 cannot.

use crate::api::error::ApiError;
use serde::Deserialize;

use crate::api::http::{HttpClient, RetryProfile, race};

pub struct ToncenterV3Client {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

#[derive(Debug, Deserialize)]
pub struct JettonTransferWallet {
    pub address: String,
    pub owner: String,
    pub jetton: String,
    pub balance: String,
}

/// Reviewed identities and base units, supplied by core's durable send.
/// The adapter does not infer the intended asset from a provider's action.
#[derive(Clone)]
pub(crate) struct TonTransferExpectation {
    pub owner: String,
    pub recipient: String,
    pub amount: u128,
    pub jetton: Option<TonJettonExpectation>,
}

#[derive(Clone)]
pub(crate) struct TonJettonExpectation {
    pub master: String,
    pub source_wallet: String,
    pub query_id: u64,
}

impl ToncenterV3Client {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    /// `sendBocReturnHash` identifies an external message, not a transaction.
    /// Follow that exact message into a committed trace. Jetton success needs
    /// the receiver jetton wallet's execution; a wallet seqno increment alone
    /// does not establish delivery.
    pub(crate) async fn fetch_transaction_status(
        &self,
        chain: crate::registry::Chain,
        hash: &str,
        expected: &TonTransferExpectation,
    ) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
        let message_hash = ton_hash(hash)?;
        let path = format!(
            "/traces?msg_hash={}&include_actions=true&limit=2",
            hex::encode(message_hash)
        );
        let response = self.get_for_network(chain, &path).await?;
        trace_status(&response, message_hash, expected)
    }

    /// Identity and the requested data must come from the same candidate.
    /// Verifying one racer before reading another does not pin a network.
    async fn get_for_network<T: serde::de::DeserializeOwned>(
        &self,
        chain: crate::registry::Chain,
        path: &str,
    ) -> Result<T, ApiError> {
        let path = path.to_string();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let base = base.trim_end_matches('/').to_string();
            let path = path.clone();
            async move {
                let network: serde_json::Value = client
                    .get_json(&format!("{base}/masterchainInfo"), RetryProfile::ChainRead)
                    .await?;
                verify_first_block(chain, &network)?;
                client
                    .get_json(&format!("{base}{path}"), RetryProfile::ChainRead)
                    .await
            }
        })
        .await
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ApiError> {
        if self.endpoints.is_empty() {
            return Err(ApiError::NoEndpoint);
        }
        let path = path.to_string();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

    /// Every jetton `address` holds: one row per jetton wallet with a
    /// balance, keyed by its master's raw address (`0:HEX`), with the master's
    /// decimals when the indexer has its metadata.
    ///
    /// The list is paged. A holder with more pages than this reads is refused
    /// rather than truncated, since a caller reads a jetton missing from the
    /// list as a zero balance.
    pub async fn fetch_jetton_balances(
        &self,
        chain: crate::registry::Chain,
        address: &str,
    ) -> Result<Vec<crate::api::HeldToken>, ApiError> {
        const PAGE_SIZE: usize = 1000;
        const MAX_PAGES: usize = 10;
        let mut held = Vec::new();
        for page in 0..MAX_PAGES {
            let offset = page * PAGE_SIZE;
            let response: serde_json::Value = self
                .get_for_network(
                    chain,
                    &format!(
                        "/jetton/wallets?owner_address={address}&limit={PAGE_SIZE}&offset={offset}"
                    ),
                )
                .await?;
            let (rows, count) = parse_jetton_wallets(&response)?;
            held.extend(rows);
            if count < PAGE_SIZE {
                return Ok(held);
            }
        }
        Err(ApiError::Rejected(format!(
            "holds more than {} jetton wallets; the list cannot be read whole",
            PAGE_SIZE * MAX_PAGES
        )))
    }

    /// Indexer association is checked again by the service against the
    /// requested owner/master before it becomes a reviewed send destination.
    pub async fn fetch_transfer_wallet(
        &self,
        chain: crate::registry::Chain,
        owner: &str,
        master: &str,
    ) -> Result<JettonTransferWallet, ApiError> {
        #[derive(Deserialize)]
        struct Envelope {
            jetton_wallets: Vec<JettonTransferWallet>,
        }
        let response: Envelope = self
            .get_for_network(
                chain,
                &format!("/jetton/wallets?owner_address={owner}&jetton_address={master}&limit=2"),
            )
            .await?;
        if response.jetton_wallets.len() != 1 {
            return Err(ApiError::Rejected(
                "Jetton sender wallet is missing or ambiguous".into(),
            ));
        }
        response
            .jetton_wallets
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::Decode("Missing jetton sender wallet".into()))
    }

    /// Indexed TEP-74 transfers between owners. The cursor advances over the
    /// provider's page before failed transfers are filtered.
    pub async fn fetch_jetton_history_page(
        &self,
        chain: crate::registry::Chain,
        owner: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<serde_json::Value>, ApiError> {
        use crate::api::error::OrDecode;
        let owner = raw_address(owner)?;
        let offset = cursor.map_or(Ok(0u64), |value| {
            value
                .parse::<u64>()
                .map_err(|_| ApiError::invalid("invalid jetton history offset"))
        })?;
        let response: serde_json::Value = self
            .get(&format!(
                "/jetton/transfers?owner_address={owner}&limit=50&offset={offset}&sort=desc"
            ))
            .await?;
        let rows = response["jetton_transfers"]
            .as_array()
            .or_decode("Jetton history: missing transfer list")?;
        if rows.len() > 50 {
            return Err(ApiError::decode(
                "Jetton history exceeds requested page size",
            ));
        }
        let mut items = Vec::new();
        for row in rows {
            if row["transaction_aborted"]
                .as_bool()
                .or_decode("Jetton history: missing execution status")?
            {
                continue;
            }
            let source = raw_address(
                row["source"]
                    .as_str()
                    .or_decode("Jetton transfer: missing source")?,
            )?;
            let destination = raw_address(
                row["destination"]
                    .as_str()
                    .or_decode("Jetton transfer: missing destination")?,
            )?;
            if source != owner && destination != owner {
                return Err(ApiError::decode("Jetton history owner mismatch"));
            }
            if source == destination {
                continue;
            }
            let amount = row["amount"]
                .as_str()
                .and_then(|value| value.parse::<u128>().ok())
                .or_decode("Jetton transfer: invalid amount")?;
            if amount == 0 {
                continue;
            }
            let master = row["jetton_master"]
                .as_str()
                .or_decode("Jetton transfer: missing master")?;
            let contract = raw_address(master)?;
            let decimals = match jetton_precision(&response, master) {
                Some(value) => value?,
                None => self
                    .fetch_jetton_decimals(chain, &contract)
                    .await
                    .or_decode("Jetton transfer: precision unavailable")?,
            };
            let txid = row["transaction_hash"]
                .as_str()
                .filter(|value| !value.is_empty())
                .or_decode("Jetton transfer: missing transaction hash")?;
            let timestamp =
                crate::api::time::confirmed_history_time(row["transaction_now"].as_u64(), txid)?;
            let lt = row["transaction_lt"]
                .as_str()
                .filter(|value| value.parse::<u64>().is_ok())
                .or_decode("Jetton transfer: missing logical time")?;
            let event_id = format!("ton:jetton:{txid}:{lt}");
            items.push(serde_json::json!({"event_id":event_id,"txid":txid,"timestamp":timestamp,"amount_display":crate::decimal::from_units(amount,u32::from(decimals)),"contract":contract,"from":source,"to":destination,"is_incoming":destination==owner}));
        }
        Ok(crate::api::HistoryPage {
            items,
            next_cursor: (rows.len() == 50)
                .then(|| offset.checked_add(50).map(|value| value.to_string()))
                .flatten(),
        })
    }

    /// A jetton master's own decimals, from its content. `None` when the
    /// master will not answer.
    pub async fn fetch_jetton_decimals(
        &self,
        chain: crate::registry::Chain,
        master_address: &str,
    ) -> Option<u8> {
        #[derive(Deserialize)]
        struct MasterEnvelope {
            jetton_masters: Option<Vec<Master>>,
        }
        #[derive(Deserialize)]
        struct Master {
            jetton_content: Option<Content>,
        }
        #[derive(Deserialize)]
        struct Content {
            decimals: Option<serde_json::Value>,
        }
        let path = format!("/jetton/masters?address={master_address}&limit=1");
        let content = self
            .get_for_network::<MasterEnvelope>(chain, &path)
            .await
            .ok()?
            .jetton_masters?
            .into_iter()
            .next()?
            .jetton_content?;
        // TON metadata carries decimals as a string as often as a number, and
        // both mean the same count.
        let raw = content.decimals?;
        raw.as_u64()
            .or_else(|| raw.as_str().and_then(|s| s.parse().ok()))
            .and_then(|d| crate::api::checked_token_decimals(u128::from(d)).ok())
    }
}

fn raw_address(value: &str) -> Result<String, ApiError> {
    let (workchain, account) = value
        .split_once(':')
        .ok_or_else(|| ApiError::decode("TON indexer did not return a raw address"))?;
    let workchain: i8 = workchain.parse().map_err(ApiError::decode)?;
    if !matches!(workchain, -1 | 0)
        || account.len() != 64
        || !account.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ApiError::decode("Invalid TON indexer account address"));
    }
    Ok(format!("{workchain}:{}", account.to_ascii_lowercase()))
}

fn ton_hash(value: &str) -> Result<[u8; 32], ApiError> {
    use base64::{Engine, engine::general_purpose};
    let bytes = if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        hex::decode(value).map_err(ApiError::decode)?
    } else {
        general_purpose::STANDARD
            .decode(value)
            .or_else(|_| general_purpose::URL_SAFE.decode(value))
            .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(value))
            .map_err(ApiError::decode)?
    };
    bytes
        .try_into()
        .map_err(|_| ApiError::decode("TON hash must contain 32 bytes"))
}

fn verify_first_block(
    chain: crate::registry::Chain,
    response: &serde_json::Value,
) -> Result<(), ApiError> {
    use crate::api::error::OrDecode;
    let (global_id, root, file) = chain
        .ton_first_block()
        .or_decode("Unsupported TON network")?;
    let first = &response["first"];
    if first["workchain"].as_i64() != Some(-1)
        || first["shard"].as_str() != Some("8000000000000000")
        || first["seqno"].as_u64() != Some(1)
        || first["global_id"].as_i64() != Some(global_id)
        || first["root_hash"].as_str() != Some(root)
        || first["file_hash"].as_str() != Some(file)
    {
        return Err(ApiError::invalid(
            "TON indexer has the wrong first masterchain block",
        ));
    }
    Ok(())
}

fn committed(transaction: &serde_json::Value) -> Result<bool, ApiError> {
    use crate::api::error::OrDecode;
    if transaction["emulated"]
        .as_bool()
        .or_decode("TON transaction: missing emulation status")?
    {
        return Ok(false);
    }
    match &transaction["finality"] {
        serde_json::Value::String(value) if value == "pending" => Ok(false),
        serde_json::Value::String(value) if value == "confirmed" || value == "finalized" => {
            Ok(true)
        }
        serde_json::Value::Number(value) => match value.as_u64() {
            Some(0) => Ok(false),
            Some(1 | 2) => Ok(true),
            _ => Err(ApiError::decode("TON transaction: invalid finality")),
        },
        _ => Err(ApiError::decode("TON transaction: missing finality")),
    }
}

fn vm_succeeded(transaction: &serde_json::Value, requires_message: bool) -> Result<bool, ApiError> {
    use crate::api::error::OrDecode;
    let description = &transaction["description"];
    if description["aborted"]
        .as_bool()
        .or_decode("TON transaction: missing aborted status")?
    {
        return Ok(false);
    }
    let compute = &description["compute_ph"];
    if compute["skipped"]
        .as_bool()
        .or_decode("TON transaction: missing compute phase")?
    {
        return Ok(false);
    }
    if !compute["success"]
        .as_bool()
        .or_decode("TON transaction: missing VM result")?
        || !matches!(
            compute["exit_code"]
                .as_i64()
                .or_decode("TON transaction: missing VM exit code")?,
            0 | 1
        )
    {
        return Ok(false);
    }
    let action = &description["action"];
    if action.is_null() {
        return Ok(!requires_message);
    }
    Ok(action["success"]
        .as_bool()
        .or_decode("TON transaction: missing action result")?
        && action["valid"]
            .as_bool()
            .or_decode("TON transaction: missing action validity")?
        && !action["no_funds"]
            .as_bool()
            .or_decode("TON transaction: missing action budget result")?
        && action["result_code"]
            .as_i64()
            .or_decode("TON transaction: missing action exit code")?
            == 0
        && (!requires_message
            || action["skipped_actions"]
                .as_u64()
                .or_decode("TON transaction: missing skipped-action count")?
                == 0))
}

fn message_opcode(message: &serde_json::Value, opcode: u32) -> bool {
    message["opcode"].as_u64() == Some(u64::from(opcode))
        || message["opcode"]
            .as_str()
            .and_then(|value| u32::from_str_radix(value.strip_prefix("0x")?, 16).ok())
            == Some(opcode)
}

fn find_message<'a>(
    transaction: &'a serde_json::Value,
    destination: &str,
    opcode: Option<u32>,
) -> Result<Option<&'a serde_json::Value>, ApiError> {
    use crate::api::error::OrDecode;
    let messages = transaction["out_msgs"]
        .as_array()
        .or_decode("TON transaction: missing outbound messages")?;
    for message in messages {
        let target = message
            .get("destination")
            .or_decode("TON message: missing destination")?;
        if !target.is_null() {
            raw_address(
                target
                    .as_str()
                    .or_decode("TON message: invalid destination")?,
            )?;
        }
        if opcode.is_some() && message.get("opcode").is_none() {
            return Err(ApiError::decode("TON message: missing opcode"));
        }
    }
    let mut matches = messages.iter().filter(|message| {
        message["destination"]
            .as_str()
            .and_then(|value| raw_address(value).ok())
            .as_deref()
            == Some(destination)
            && opcode.is_none_or(|opcode| message_opcode(message, opcode))
    });
    let message = matches.next();
    if matches.next().is_some() {
        return Err(ApiError::decode(
            "TON trace: reviewed outbound message is ambiguous",
        ));
    }
    Ok(message)
}

fn message_receiver<'a>(
    transactions: &'a serde_json::Map<String, serde_json::Value>,
    message: &serde_json::Value,
) -> Result<Option<&'a serde_json::Value>, ApiError> {
    use crate::api::error::OrDecode;
    let hash = ton_hash(
        message["hash"]
            .as_str()
            .or_decode("TON message: missing hash")?,
    )?;
    let mut matches = transactions.values().filter(|transaction| {
        transaction["in_msg"]["hash"]
            .as_str()
            .and_then(|value| ton_hash(value).ok())
            == Some(hash)
    });
    let Some(transaction) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err(ApiError::decode(
            "TON message has multiple receiving transactions",
        ));
    }
    if raw_address(
        transaction["account"]
            .as_str()
            .or_decode("TON receiver: missing account")?,
    )? != raw_address(
        message["destination"]
            .as_str()
            .or_decode("TON message: missing destination")?,
    )? || transaction["in_msg"]["bounced"].as_bool() != Some(false)
        || transaction["in_msg"]["source"]
            .as_str()
            .map(raw_address)
            .transpose()?
            != message["source"].as_str().map(raw_address).transpose()?
    {
        return Err(ApiError::decode(
            "TON trace: receiving message identity mismatch",
        ));
    }
    Ok(Some(transaction))
}

fn trace_status(
    response: &serde_json::Value,
    message_hash: [u8; 32],
    expected: &TonTransferExpectation,
) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
    use crate::api::{error::OrDecode, transaction_status::TransactionStatus};
    let traces = response["traces"]
        .as_array()
        .or_decode("TON status: missing traces")?;
    if traces.is_empty() {
        return Ok(TransactionStatus::Pending);
    }
    if traces.len() != 1 {
        return Err(ApiError::decode(
            "TON external message has ambiguous traces",
        ));
    }
    let trace = &traces[0];
    let transactions = trace["transactions"]
        .as_object()
        .or_decode("TON trace: missing transactions")?;
    if transactions.len() > 1000 {
        return Err(ApiError::decode("TON trace exceeds transaction limit"));
    }
    let root_hash = trace["trace"]["tx_hash"]
        .as_str()
        .or_decode("TON trace: missing root transaction")?;
    let root = transactions
        .get(root_hash)
        .or_decode("TON trace: root transaction is missing")?;
    if ton_hash(
        trace["external_hash"]
            .as_str()
            .or_decode("TON trace: missing external message hash")?,
    )? != message_hash
        || ton_hash(
            root["in_msg"]["hash"]
                .as_str()
                .or_decode("TON transaction: missing inbound hash")?,
        )? != message_hash
        || raw_address(
            root["account"]
                .as_str()
                .or_decode("TON transaction: missing wallet")?,
        )? != raw_address(&expected.owner)?
        || raw_address(
            root["in_msg"]["destination"]
                .as_str()
                .or_decode("TON message: missing wallet destination")?,
        )? != raw_address(&expected.owner)?
        || !root["in_msg"]["source"].is_null()
    {
        return Err(ApiError::decode(
            "TON external message does not match the reviewed sender",
        ));
    }
    if !committed(root)? {
        return Ok(TransactionStatus::Pending);
    }
    let block = root["mc_block_seqno"]
        .as_u64()
        .filter(|value| *value > 0)
        .or_decode("TON transaction: missing masterchain confirmation")?;
    let outcome = |succeeded| TransactionStatus::Confirmed {
        succeeded,
        block: Some(block),
    };
    if trace["is_incomplete"]
        .as_bool()
        .or_decode("TON trace: missing completeness")?
        || trace["trace_info"]["trace_state"].as_str() != Some("complete")
        || trace["trace_info"]["pending_messages"].as_u64() != Some(0)
    {
        return Ok(TransactionStatus::Pending);
    }
    if !vm_succeeded(root, true)? {
        return Ok(outcome(false));
    }
    let owner = raw_address(&expected.owner)?;
    let recipient = raw_address(&expected.recipient)?;
    let Some(jetton) = &expected.jetton else {
        let Some(message) = find_message(root, &recipient, None)? else {
            return Ok(outcome(false));
        };
        if message["value"]
            .as_str()
            .and_then(|value| value.parse::<u128>().ok())
            != Some(expected.amount)
        {
            return Err(ApiError::decode(
                "TON message amount differs from the reviewed send",
            ));
        }
        let Some(receiver) = message_receiver(transactions, message)? else {
            return Ok(TransactionStatus::Pending);
        };
        if !committed(receiver)? {
            return Ok(TransactionStatus::Pending);
        }
        // A non-bounce message credits an uninitialized wallet even when VM
        // computation is skipped. A bounced transfer never proves delivery.
        let credited = receiver["description"]["credit_ph"]["credit"]
            .as_str()
            .and_then(|value| value.parse::<u128>().ok())
            .is_some_and(|value| value > 0);
        let no_bounce = message["bounce"].as_bool() == Some(false)
            && receiver["description"]["bounce"].is_null();
        return Ok(outcome(
            vm_succeeded(receiver, false)? || (no_bounce && credited),
        ));
    };
    let source_wallet = raw_address(&jetton.source_wallet)?;
    let Some(message) = find_message(root, &source_wallet, Some(0x0f8a7ea5))? else {
        return Ok(outcome(false));
    };
    let Some(source) = message_receiver(transactions, message)? else {
        return Ok(TransactionStatus::Pending);
    };
    if !committed(source)? {
        return Ok(TransactionStatus::Pending);
    }
    if !vm_succeeded(source, true)? {
        return Ok(outcome(false));
    }
    let outbound = source["out_msgs"]
        .as_array()
        .or_decode("TON jetton wallet: missing outbound messages")?;
    for message in outbound {
        message
            .get("opcode")
            .or_decode("TON message: missing opcode")?;
        let destination = message
            .get("destination")
            .or_decode("TON message: missing destination")?;
        if !destination.is_null() {
            raw_address(
                destination
                    .as_str()
                    .or_decode("TON message: invalid destination")?,
            )?;
        }
    }
    if !outbound
        .iter()
        .any(|message| message_opcode(message, 0x178d4519))
    {
        return Ok(outcome(false));
    }
    let actions = trace["actions"]
        .as_array()
        .or_decode("TON trace: missing actions")?;
    let master = raw_address(&jetton.master)?;
    let mut actions = actions.iter().filter(|action| {
        let details = &action["details"];
        action["type"].as_str() == Some("jetton_transfer")
            && details["asset"]
                .as_str()
                .and_then(|value| raw_address(value).ok())
                .as_deref()
                == Some(&master)
            && details["sender"]
                .as_str()
                .and_then(|value| raw_address(value).ok())
                .as_deref()
                == Some(owner.as_str())
            && details["receiver"]
                .as_str()
                .and_then(|value| raw_address(value).ok())
                .as_deref()
                == Some(&recipient)
            && details["sender_jetton_wallet"]
                .as_str()
                .and_then(|value| raw_address(value).ok())
                .as_deref()
                == Some(&source_wallet)
            && details["amount"]
                .as_str()
                .and_then(|value| value.parse::<u128>().ok())
                == Some(expected.amount)
            && details["query_id"]
                .as_str()
                .and_then(|value| value.parse::<u64>().ok())
                == Some(jetton.query_id)
    });
    let Some(action) = actions.next() else {
        return Ok(TransactionStatus::Pending);
    };
    if actions.next().is_some() {
        return Err(ApiError::decode(
            "TON trace: ambiguous reviewed jetton action",
        ));
    }
    if !action["success"]
        .as_bool()
        .or_decode("TON jetton action: missing execution status")?
    {
        return Ok(outcome(false));
    }
    let destination_wallet = raw_address(
        action["details"]["receiver_jetton_wallet"]
            .as_str()
            .or_decode("TON jetton action: missing receiver wallet")?,
    )?;
    let Some(message) = find_message(source, &destination_wallet, Some(0x178d4519))? else {
        return Ok(outcome(false));
    };
    let Some(destination) = message_receiver(transactions, message)? else {
        return Ok(TransactionStatus::Pending);
    };
    if !committed(destination)? {
        return Ok(TransactionStatus::Pending);
    }
    let action_transactions = action["transactions"]
        .as_array()
        .or_decode("TON jetton action: missing transaction proof")?;
    for transaction in [root, source, destination] {
        let hash = transaction["hash"]
            .as_str()
            .or_decode("TON transaction: missing hash")?;
        if !action_transactions
            .iter()
            .any(|entry| entry.as_str() == Some(hash))
        {
            return Err(ApiError::decode(
                "TON jetton action is not linked to its wallet transactions",
            ));
        }
    }
    // Notification/excess-message failures after this deposit cannot undo it.
    Ok(outcome(vm_succeeded(destination, false)?))
}

fn jetton_precision(response: &serde_json::Value, master: &str) -> Option<Result<u8, ApiError>> {
    let info = response["metadata"][master]["token_info"]
        .as_array()?
        .iter()
        .find(|info| info["type"].as_str() == Some("jetton_masters"))?;
    let decimals = info["extra"]["decimals"]
        .as_u64()
        .or_else(|| info["extra"]["decimals"].as_str()?.parse().ok());
    Some(
        decimals
            .ok_or_else(|| ApiError::decode("Jetton metadata: invalid precision"))
            .and_then(|value| crate::api::checked_token_decimals(u128::from(value))),
    )
}

/// The holdings in one `/jetton/wallets` page, and how many wallets the page
/// listed. `jetton` is the master's raw address; its decimals ride in the
/// response's `metadata` when the indexer has read the master.
fn parse_jetton_wallets(
    response: &serde_json::Value,
) -> Result<(Vec<crate::api::HeldToken>, usize), ApiError> {
    use crate::api::error::OrDecode;
    let wallets = response
        .get("jetton_wallets")
        .and_then(serde_json::Value::as_array)
        .or_decode("jetton wallets: missing list")?;
    let decimals = |master: &str| {
        response
            .pointer(&format!("/metadata/{master}/token_info"))?
            .as_array()?
            .iter()
            .find(|info| info.get("type").and_then(|t| t.as_str()) == Some("jetton_masters"))?
            .pointer("/extra/decimals")
            .and_then(|raw| {
                // TON metadata carries decimals as a string as often as a
                // number, and both mean the same count.
                raw.as_u64().or_else(|| raw.as_str()?.parse().ok())
            })
            .and_then(|d| crate::api::checked_token_decimals(u128::from(d)).ok())
    };
    let held = wallets
        .iter()
        .filter_map(|wallet| {
            let master = wallet.get("jetton")?.as_str()?;
            let balance_raw: u128 = wallet.get("balance")?.as_str()?.parse().ok()?;
            (balance_raw > 0).then(|| crate::api::HeldToken {
                contract: master.to_string(),
                balance_raw,
                decimals: decimals(master),
            })
        })
        .collect();
    Ok((held, wallets.len()))
}

#[cfg(test)]
mod jetton_wallets {
    use super::parse_jetton_wallets;
    use serde_json::json;

    /// The shape `toncenter.com/api/v3/jetton/wallets` returns: `jetton` is a
    /// raw address string, and decimals live under `metadata`.
    #[test]
    fn a_page_names_each_master_and_its_decimals() {
        let usdt = "0:B113A994B5024A16719F69139328EB759596C38A25F59028B146FECDC3621DFE";
        let other = "0:52E0FE119C45BE79C25E2E7EDA3F7C6E90167036D5B390A2290C986C774A2EE1";
        let (held, count) = parse_jetton_wallets(&json!({
            "jetton_wallets": [
                {"address": "0:2626", "balance": "879187990649145", "jetton": usdt},
                {"address": "0:07FE", "balance": "500000000000", "jetton": other},
                {"address": "0:0000", "balance": "0", "jetton": other}
            ],
            "metadata": {
                usdt: {"token_info": [{"type": "jetton_masters", "extra": {"decimals": "6"}}]}
            }
        }))
        .expect("a page");
        assert_eq!(count, 3, "the page size counts every wallet listed");
        assert_eq!(held.len(), 2, "an empty jetton wallet is not a holding");
        assert_eq!(held[0].contract, usdt);
        assert_eq!(held[0].decimals, Some(6));
        assert_eq!(held[1].decimals, None, "no metadata, no decimals");
    }
}

#[cfg(test)]
mod transaction_status_tests {
    use super::*;
    use crate::api::transaction_status::TransactionStatus;
    use serde_json::{Value, json};

    fn fixture() -> (Value, [u8; 32], TonTransferExpectation) {
        // Official TON Center v3 response, fetched 2026-10-04. Its receiver
        // notification is aborted/no_gas despite a successful jetton deposit.
        let response: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/ton-status-v3.json")).unwrap();
        let action = &response["traces"][0]["actions"][0]["details"];
        let expected = TonTransferExpectation {
            owner: raw_address(action["sender"].as_str().unwrap()).unwrap(),
            recipient: raw_address(action["receiver"].as_str().unwrap()).unwrap(),
            amount: action["amount"].as_str().unwrap().parse().unwrap(),
            jetton: Some(TonJettonExpectation {
                master: raw_address(action["asset"].as_str().unwrap()).unwrap(),
                source_wallet: raw_address(action["sender_jetton_wallet"].as_str().unwrap())
                    .unwrap(),
                query_id: action["query_id"].as_str().unwrap().parse().unwrap(),
            }),
        };
        let hash = ton_hash(response["traces"][0]["external_hash"].as_str().unwrap()).unwrap();
        (response, hash, expected)
    }

    #[test]
    fn external_message_confirms_deposit_even_when_notification_fails() {
        let (response, hash, expected) = fixture();
        assert_eq!(
            trace_status(&response, hash, &expected).unwrap(),
            TransactionStatus::Confirmed {
                succeeded: true,
                block: Some(96918998)
            }
        );
        assert_ne!(
            ton_hash(response["traces"][0]["trace"]["tx_hash"].as_str().unwrap()).unwrap(),
            hash
        );
    }

    #[test]
    fn absence_incomplete_emulated_and_unclassified_actions_remain_pending() {
        let (response, hash, expected) = fixture();
        assert_eq!(
            trace_status(&json!({"traces":[]}), hash, &expected).unwrap(),
            TransactionStatus::Pending
        );
        let mut incomplete = response.clone();
        incomplete["traces"][0]["is_incomplete"] = json!(true);
        assert_eq!(
            trace_status(&incomplete, hash, &expected).unwrap(),
            TransactionStatus::Pending
        );
        let root = response["traces"][0]["trace"]["tx_hash"].as_str().unwrap();
        let mut emulated = response.clone();
        emulated["traces"][0]["transactions"][root]["emulated"] = json!(true);
        assert_eq!(
            trace_status(&emulated, hash, &expected).unwrap(),
            TransactionStatus::Pending
        );
        let mut no_action = response.clone();
        no_action["traces"][0]["actions"] = json!([]);
        assert_eq!(
            trace_status(&no_action, hash, &expected).unwrap(),
            TransactionStatus::Pending
        );
    }

    #[test]
    fn wallet_vm_action_and_receiver_failures_have_committed_failure_proof() {
        let (response, hash, expected) = fixture();
        let root = response["traces"][0]["trace"]["tx_hash"].as_str().unwrap();
        for failure in ["aborted", "compute", "action", "skipped_send"] {
            let mut failed = response.clone();
            let description = &mut failed["traces"][0]["transactions"][root]["description"];
            match failure {
                "aborted" => description["aborted"] = json!(true),
                "compute" => description["compute_ph"]["success"] = json!(false),
                "action" => description["action"]["result_code"] = json!(37),
                _ => description["action"]["skipped_actions"] = json!(1),
            }
            assert_eq!(
                trace_status(&failed, hash, &expected).unwrap(),
                TransactionStatus::Confirmed {
                    succeeded: false,
                    block: Some(96918998)
                },
                "{failure}"
            );
        }
        let receiver = response["traces"][0]["actions"][0]["details"]["receiver_jetton_wallet"]
            .as_str()
            .unwrap();
        let mut failed = response.clone();
        let transaction = failed["traces"][0]["transactions"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .find(|tx| tx["account"].as_str() == Some(receiver))
            .unwrap();
        transaction["description"]["aborted"] = json!(true);
        assert_eq!(
            trace_status(&failed, hash, &expected).unwrap(),
            TransactionStatus::Confirmed {
                succeeded: false,
                block: Some(96918998)
            }
        );
    }

    #[test]
    fn hash_owner_metadata_and_message_edges_cannot_be_guessed() {
        let (response, hash, mut expected) = fixture();
        assert!(trace_status(&response, [0; 32], &expected).is_err());
        expected.owner = format!("0:{}", "01".repeat(32));
        assert!(trace_status(&response, hash, &expected).is_err());
        let (response, hash, expected) = fixture();
        let mut missing = response.clone();
        let root = response["traces"][0]["trace"]["tx_hash"].as_str().unwrap();
        missing["traces"][0]["transactions"][root]
            .as_object_mut()
            .unwrap()
            .remove("finality");
        assert!(trace_status(&missing, hash, &expected).is_err());
        let mut unlinked = response.clone();
        unlinked["traces"][0]["actions"][0]["transactions"] = json!([]);
        assert!(trace_status(&unlinked, hash, &expected).is_err());
    }

    #[test]
    fn complete_zero_delivery_is_failed_while_missing_schema_is_an_error() {
        let (response, hash, expected) = fixture();
        let root = response["traces"][0]["trace"]["tx_hash"].as_str().unwrap();
        let mut empty = response.clone();
        empty["traces"][0]["transactions"][root]["out_msgs"] = json!([]);
        assert!(matches!(
            trace_status(&empty, hash, &expected).unwrap(),
            TransactionStatus::Confirmed {
                succeeded: false,
                ..
            }
        ));
        empty["traces"][0]["transactions"][root]
            .as_object_mut()
            .unwrap()
            .remove("out_msgs");
        assert!(trace_status(&empty, hash, &expected).is_err());
        let mut source_empty = response.clone();
        let source = &expected.jetton.as_ref().unwrap().source_wallet;
        let transaction = source_empty["traces"][0]["transactions"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .find(|tx| raw_address(tx["account"].as_str().unwrap()).unwrap() == *source)
            .unwrap();
        transaction["out_msgs"] = json!([]);
        source_empty["traces"][0]["actions"] = json!([]);
        assert!(matches!(
            trace_status(&source_empty, hash, &expected).unwrap(),
            TransactionStatus::Confirmed {
                succeeded: false,
                ..
            }
        ));
        let mut incomplete = source_empty;
        incomplete["traces"][0]["is_incomplete"] = json!(true);
        assert_eq!(
            trace_status(&incomplete, hash, &expected).unwrap(),
            TransactionStatus::Pending
        );
    }

    #[test]
    fn native_delivery_requires_matching_value_and_receiver_execution_or_nonbounce_credit() {
        let (mut response, hash, mut expected) = fixture();
        let root = response["traces"][0]["trace"]["tx_hash"]
            .as_str()
            .unwrap()
            .to_string();
        let message = &response["traces"][0]["transactions"][&root]["out_msgs"][0];
        expected.recipient = raw_address(message["destination"].as_str().unwrap()).unwrap();
        expected.amount = message["value"].as_str().unwrap().parse().unwrap();
        expected.jetton = None;
        assert!(matches!(
            trace_status(&response, hash, &expected).unwrap(),
            TransactionStatus::Confirmed {
                succeeded: true,
                ..
            }
        ));
        expected.amount += 1;
        assert!(trace_status(&response, hash, &expected).is_err());
        expected.amount -= 1;
        response["traces"][0]["transactions"][&root]["out_msgs"][0]["bounce"] = json!(false);
        let transaction = response["traces"][0]["transactions"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .find(|tx| raw_address(tx["account"].as_str().unwrap()).unwrap() == expected.recipient)
            .unwrap();
        transaction["description"] = json!({"aborted":true,"compute_ph":{"skipped":true,"reason":"no_state"},"credit_ph":{"credit":expected.amount.to_string()}});
        assert!(matches!(
            trace_status(&response, hash, &expected).unwrap(),
            TransactionStatus::Confirmed {
                succeeded: true,
                ..
            }
        ));
    }

    #[test]
    fn first_block_pins_mainnet_and_testnet_indexers() {
        for chain in [
            crate::registry::Chain::Ton,
            crate::registry::Chain::TonTestnet,
        ] {
            let (id, root, file) = chain.ton_first_block().unwrap();
            let mut response = json!({"first":{"workchain":-1,"shard":"8000000000000000","seqno":1,"global_id":id,"root_hash":root,"file_hash":file}});
            assert!(verify_first_block(chain, &response).is_ok());
            response["first"]["global_id"] = json!(0);
            assert!(verify_first_block(chain, &response).is_err());
        }
    }

    #[tokio::test]
    async fn wrong_network_refuses_all_funds_reads_before_requesting_jettons() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};
        let server = MockServer::start().await;
        let (id, root, file) = crate::registry::Chain::TonTestnet
            .ton_first_block()
            .unwrap();
        Mock::given(any()).respond_with(ResponseTemplate::new(200).set_body_json(json!({"first":{"workchain":-1,"shard":"8000000000000000","seqno":1,"global_id":id,"root_hash":root,"file_hash":file}}))).mount(&server).await;
        let client = ToncenterV3Client::new(std::sync::Arc::new(vec![server.uri()]));
        let owner = format!("0:{}", "11".repeat(32));
        let master = format!("0:{}", "22".repeat(32));
        assert!(
            client
                .fetch_transfer_wallet(crate::registry::Chain::Ton, &owner, &master)
                .await
                .unwrap_err()
                .to_string()
                .contains("wrong first masterchain block")
        );
        assert!(
            client
                .fetch_jetton_balances(crate::registry::Chain::Ton, &owner)
                .await
                .is_err()
        );
        assert_eq!(
            client
                .fetch_jetton_decimals(crate::registry::Chain::Ton, &master)
                .await,
            None
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(
            requests
                .iter()
                .all(|request| request.url.path() == "/masterchainInfo")
        );
    }
}
