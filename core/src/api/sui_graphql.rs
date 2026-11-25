//! The Sui GraphQL adapter: balances, coin and staked objects, coin
//! metadata, the epoch's gas price and validators, address history, and the
//! simulation and execution of a transaction.
//!
//! Each request is one named operation (`operationName`), posted to every
//! endpoint at once; the first complete answer wins. An answer carrying
//! `errors` is a refusal, never partial data. GraphQL writes a `UInt53` as a
//! JSON number, a `BigInt` and a Move `u64` as a string of digits, and every
//! address and type in its 64-digit form; core writes types as the catalog
//! normalizes them (`0x2::sui::SUI`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::error::{ApiError, OrDecode};
use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::transaction_status::TransactionStatus;
use crate::registry::Chain;

pub const SUI_COIN_TYPE: &str = "0x2::sui::SUI";
const STAKED_SUI: &str = "0x3::staking_pool::StakedSui";
const SUI_SYSTEM_STATE: &str = "0x3::sui_system::SuiSystemState";

/// The most pages a listing reads: 5,000 rows at the services' 50 a page.
const MAX_PAGES: usize = 100;

const CHAIN: &str = "query Chain { chainIdentifier checkpoint { sequenceNumber } }";
const EPOCH: &str = "query Epoch { epoch { epochId referenceGasPrice } }";
const BALANCE: &str = "query Balance($owner: SuiAddress!, $coinType: String!) { \
    address(address: $owner) { balance(coinType: $coinType) { totalBalance coinBalance } } }";
const BALANCES: &str = "query Balances($owner: SuiAddress!, $after: String) { \
    address(address: $owner) { balances(first: 50, after: $after) { \
    pageInfo { hasNextPage endCursor } nodes { coinType { repr } totalBalance } } } }";
const OBJECTS: &str = "query Objects($owner: SuiAddress!, $type: String!, $after: String) { \
    address(address: $owner) { objects(first: 50, after: $after, filter: {type: $type}) { \
    pageInfo { hasNextPage endCursor } \
    nodes { address version digest owner { __typename ... on AddressOwner { address { address } } } \
    contents { type { repr } json } } } } }";
const OBJECT: &str = "query Object($id: SuiAddress!) { object(address: $id) { \
    address version digest \
    owner { __typename ... on AddressOwner { address { address } } \
    ... on Shared { initialSharedVersion } } \
    asMoveObject { contents { type { repr } json } } } }";
const COIN_METADATA: &str = "query CoinMetadata($coinType: String!) { \
    coinMetadata(coinType: $coinType) { decimals } }";
const VALIDATORS: &str = "query Validators($after: String) { epoch { validatorSet { \
    activeValidators(first: 50, after: $after) { \
    pageInfo { hasNextPage endCursor } nodes { contents { json } } } } } }";
const HISTORY: &str = "query History($owner: SuiAddress!, $before: String) { \
    address(address: $owner) { transactions(last: 25, before: $before, relation: AFFECTED) { \
    pageInfo { hasPreviousPage startCursor } \
    nodes { digest sender { address } gasInput { gasSponsor { address } } \
    effects { timestamp checkpoint { sequenceNumber } \
    gasEffects { gasSummary { computationCost storageCost storageRebate } } \
    balanceChanges(first: 50) { pageInfo { hasNextPage endCursor } \
    nodes { owner { address } coinType { repr } amount } } } } } } }";
const BALANCE_CHANGES: &str = "query BalanceChanges($digest: String!, $after: String) { \
    transactionEffects(digest: $digest) { balanceChanges(first: 50, after: $after) { \
    pageInfo { hasNextPage endCursor } nodes { owner { address } coinType { repr } amount } } } }";
const STATUS: &str = "query Status($digest: String!) { transactionEffects(digest: $digest) { \
    digest status checkpoint { sequenceNumber } } }";
const SIMULATE: &str = "query Simulate($transaction: JSON!) { \
    simulateTransaction(transaction: $transaction) { effects { status executionError { message } \
    gasEffects { gasSummary { computationCost storageCost storageRebate } } } } }";
const EXECUTE: &str = "mutation Execute($transaction: Base64!, $signatures: [Base64!]!) { \
    executeTransaction(transactionDataBcs: $transaction, signatures: $signatures) { \
    effects { digest status executionError { message } } } }";

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiBalance {
    /// MIST (1 SUI = 1_000_000_000 MIST), in coin objects and the address
    /// balance together.
    pub mist: u64,
    /// The part in coin objects, which is all a transaction core builds can
    /// spend.
    pub coin_mist: u64,
}

/// One transaction's effect on an address's holding of one coin type, fee
/// excluded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiHistoryEntry {
    pub digest: String,
    /// `None` until the transaction is in a checkpoint, which is what dates it.
    pub timestamp_ms: Option<u64>,
    pub is_incoming: bool,
    pub amount_mist: String,
    pub contract: Option<String>,
    pub amount_display: Option<String>,
    /// The sender when incoming; the largest other recipient when outgoing.
    pub from: String,
    pub to: String,
}

impl SuiHistoryEntry {
    pub(crate) fn history_entry(self) -> Option<crate::api::history_page::HistoryEntry> {
        Some(crate::api::history_page::HistoryEntry {
            txid: self.digest,
            amount: match self.amount_display {
                Some(amount) => crate::api::history_page::HistoryAmount::Decimal(amount),
                None => crate::api::history_page::HistoryAmount::unit_digits(&self.amount_mist)?,
            },
            contract: self.contract,
            incoming: self.is_incoming,
            outcome: crate::api::history_page::HistoryOutcome::Confirmed,
            block_height: None,
            timestamp: self.timestamp_ms.map(|t| t as f64 / 1e3),
            counterparty: if self.is_incoming { self.from } else { self.to },
        })
    }
}

/// One coin object, as gas or as the amount sent.
pub struct SuiCoin {
    pub object_id: String,
    pub version: u64,
    pub digest: [u8; 32],
    pub balance: u64,
}

/// A page of an owner's coins of one type; `next_cursor` is `None` on the
/// last page.
pub struct SuiCoinPage {
    pub coins: Vec<SuiCoin>,
    pub next_cursor: Option<String>,
}

/// An active validator, from its `0x3::validator::Validator` object.
#[derive(Debug, Clone)]
pub struct SuiValidator {
    pub sui_address: String,
    pub name: String,
    pub description: String,
    pub project_url: String,
    /// Basis points: 500 is 5%.
    pub commission_rate: u64,
    /// MIST.
    pub staking_pool_sui_balance: String,
    pub staking_pool_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SuiStakedObject {
    pub version: u64,
    pub digest: [u8; 32],
    pub principal: u64,
    pub pool_id: String,
    pub activation_epoch: u64,
}

/// A `StakedSui` object the owner holds.
#[derive(Debug, Clone)]
pub(crate) struct SuiStake {
    pub id: String,
    pub object: SuiStakedObject,
    /// The active validator whose pool holds it; `None` once the validator
    /// has left the active set.
    pub validator: Option<String>,
    /// Earning: its activation epoch has begun.
    pub active: bool,
}

// ── Client

pub struct SuiClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

impl SuiClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn query(&self, document: &'static str, variables: Value) -> Result<Value, ApiError> {
        self.post(document, variables, RetryProfile::ChainRead)
            .await
    }

    async fn post(
        &self,
        document: &'static str,
        variables: Value,
        profile: RetryProfile,
    ) -> Result<Value, ApiError> {
        let body = Arc::new(json!({
            "query": document,
            "operationName": operation_name(document),
            "variables": variables,
        }));
        race(&self.endpoints, |url| {
            let client = Arc::clone(&self.client);
            let body = Arc::clone(&body);
            async move { answer(client.post_json(&url, &*body, profile).await?) }
        })
        .await
    }

    /// The latest checkpoint of an endpoint that names `chain`'s genesis
    /// checkpoint as its chain identifier.
    pub(crate) async fn latest_checkpoint(&self, chain: Chain) -> Result<u64, ApiError> {
        let genesis = chain
            .sui_genesis_checkpoint()
            .ok_or_else(|| ApiError::invalid("Not a Sui network"))?;
        let data = self.query(CHAIN, json!({})).await?;
        if data["chainIdentifier"].as_str() != Some(genesis) {
            return Err(ApiError::invalid("Sui endpoint is on the wrong network"));
        }
        number(
            &data["checkpoint"]["sequenceNumber"],
            "Sui: missing latest checkpoint",
        )
    }

    pub async fn verify_network(&self, chain: Chain) -> Result<(), ApiError> {
        self.latest_checkpoint(chain).await.map(drop)
    }

    /// The current epoch and its reference gas price.
    async fn epoch(&self) -> Result<(u64, u64), ApiError> {
        let data = self.query(EPOCH, json!({})).await?;
        Ok((
            number(&data["epoch"]["epochId"], "Sui: missing epoch")?,
            digits(
                &data["epoch"]["referenceGasPrice"],
                "Sui: missing reference gas price",
            )?,
        ))
    }

    pub async fn fetch_reference_gas_price(&self) -> Result<u64, ApiError> {
        Ok(self.epoch().await?.1)
    }

    /// What `owner` holds of `coin_type`: in coin objects and its address
    /// balance together, and in coin objects alone. Zero for a type it never
    /// held.
    async fn balance(&self, owner: &str, coin_type: &str) -> Result<(u128, u128), ApiError> {
        let data = self
            .query(BALANCE, json!({"owner": owner, "coinType": coin_type}))
            .await?;
        let address = data
            .get("address")
            .filter(|address| address.is_object())
            .or_decode("Sui balance: missing address")?;
        match address.get("balance").filter(|balance| !balance.is_null()) {
            None => Ok((0, 0)),
            Some(balance) => Ok((
                digits(&balance["totalBalance"], "Sui balance: missing total")?,
                digits(&balance["coinBalance"], "Sui balance: missing coin total")?,
            )),
        }
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<SuiBalance, ApiError> {
        let (total, coins) = self.balance(address, SUI_COIN_TYPE).await?;
        Ok(SuiBalance {
            mist: u64::try_from(total).map_err(ApiError::decode)?,
            coin_mist: u64::try_from(coins).map_err(ApiError::decode)?,
        })
    }

    /// The raw balance of one coin type (e.g. `0x5d4b…::coin::COIN`), in
    /// coin objects and the address balance together.
    pub async fn fetch_coin_balance(
        &self,
        address: &str,
        coin_type: &str,
    ) -> Result<u128, ApiError> {
        Ok(self.balance(address, coin_type).await?.0)
    }

    /// Every coin type but SUI the address holds a nonzero balance of.
    ///
    /// The listing carries no decimals; a caller reads those for the coin
    /// types it needs.
    pub async fn fetch_all_coin_balances(
        &self,
        address: &str,
    ) -> Result<Vec<crate::api::HeldToken>, ApiError> {
        let mut held = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..MAX_PAGES {
            let data = self
                .query(BALANCES, json!({"owner": address, "after": cursor}))
                .await?;
            let connection = &data["address"]["balances"];
            for node in nodes(connection, "Sui balances: missing list")? {
                let contract = short_type(
                    node["coinType"]["repr"]
                        .as_str()
                        .or_decode("Sui balances: missing coin type")?,
                );
                let balance_raw = digits(&node["totalBalance"], "Sui balances: missing total")?;
                if contract != SUI_COIN_TYPE && balance_raw > 0 {
                    held.push(crate::api::HeldToken {
                        contract,
                        balance_raw,
                        decimals: None,
                    });
                }
            }
            match next_page(connection, &mut seen)? {
                Some(next) => cursor = Some(next),
                None => return Ok(held),
            }
        }
        Err(too_many("coin types"))
    }

    /// A coin type's own decimals, from its metadata.
    async fn coin_decimals(&self, coin_type: &str) -> Result<u8, ApiError> {
        let data = self
            .query(COIN_METADATA, json!({"coinType": coin_type}))
            .await?;
        crate::api::checked_token_decimals(u128::from(number(
            &data["coinMetadata"]["decimals"],
            "Sui coin metadata: missing decimals",
        )?))
    }

    /// A coin type's own decimals; `None` when its metadata is absent or
    /// exceeds core's supported precision.
    pub async fn fetch_coin_decimals(&self, coin_type: &str) -> Option<u8> {
        self.coin_decimals(coin_type).await.ok()
    }

    /// One page of `owner`'s objects of exactly `type_`, a filter GraphQL
    /// also matches a type's instances with (`0x2::coin::Coin` matches every
    /// `Coin<T>`).
    async fn objects_page(
        &self,
        owner: &str,
        type_: &str,
        cursor: Option<&str>,
    ) -> Result<Value, ApiError> {
        let data = self
            .query(
                OBJECTS,
                json!({"owner": owner, "type": type_, "after": cursor}),
            )
            .await?;
        Ok(data["address"]["objects"].clone())
    }

    /// Up to 50 of `owner`'s `coin_type` coins, from `cursor` on.
    pub async fn fetch_coins_page(
        &self,
        owner: &str,
        coin_type: &str,
        cursor: Option<&str>,
    ) -> Result<SuiCoinPage, ApiError> {
        let wanted = short_type(coin_type);
        let connection = self
            .objects_page(owner, &format!("0x2::coin::Coin<{coin_type}>"), cursor)
            .await?;
        let coins = nodes(&connection, "Sui coins: missing list")?
            .iter()
            .map(|node| {
                let object = MoveObject::listed(node, owner)?;
                if coin_of(&object.type_).as_deref() != Some(wanted.as_str()) {
                    return Err(ApiError::decode("Sui coins: an object of another type"));
                }
                Ok(SuiCoin {
                    balance: digits(&object.json["balance"], "Sui coin: missing balance")?,
                    object_id: object.id,
                    version: object.version,
                    digest: object.digest,
                })
            })
            .collect::<Result<_, ApiError>>()?;
        let info = &connection["pageInfo"];
        let next_cursor = if info["hasNextPage"]
            .as_bool()
            .or_decode("Sui coins: missing page info")?
        {
            Some(
                info["endCursor"]
                    .as_str()
                    .or_decode("Sui coins: missing cursor")?
                    .to_string(),
            )
        } else {
            None
        };
        Ok(SuiCoinPage { coins, next_cursor })
    }

    /// Each coin type the address holds in coin objects, SUI included: its
    /// type, how many objects hold it and their total.
    pub(crate) async fn fetch_coin_object_counts(
        &self,
        address: &str,
    ) -> Result<Vec<(String, u64, u128)>, ApiError> {
        let mut types: BTreeMap<String, (u64, u128)> = BTreeMap::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..MAX_PAGES {
            let connection = self
                .objects_page(address, "0x2::coin::Coin", cursor.as_deref())
                .await?;
            for node in nodes(&connection, "Sui coins: missing list")? {
                let object = MoveObject::listed(node, address)?;
                let kind = coin_of(&object.type_).or_decode("Sui coins: not a coin object")?;
                let balance: u128 = digits(&object.json["balance"], "Sui coin: missing balance")?;
                let (count, total) = types.entry(kind).or_default();
                *count += 1;
                *total = total
                    .checked_add(balance)
                    .or_decode("Sui coin balance overflow")?;
            }
            match next_page(&connection, &mut seen)? {
                Some(next) => cursor = Some(next),
                None => {
                    return Ok(types
                        .into_iter()
                        .map(|(kind, (count, total))| (kind, count, total))
                        .collect());
                }
            }
        }
        Err(too_many("coin objects"))
    }

    /// What a transaction would cost, simulated: computation, storage and the
    /// storage rebate, in MIST. Refuses one that would not succeed.
    pub(crate) async fn simulate(&self, bytes: &[u8]) -> Result<(u64, u64, u64), ApiError> {
        let data = self
            .query(
                SIMULATE,
                json!({"transaction": {"bcs": {"value": STANDARD.encode(bytes)}}}),
            )
            .await?;
        let effects = &data["simulateTransaction"]["effects"];
        match effects["status"].as_str() {
            Some("SUCCESS") => {}
            Some("FAILURE") => {
                return Err(ApiError::invalid(format!(
                    "Sui simulation refused: {}",
                    effects["executionError"]["message"]
                        .as_str()
                        .unwrap_or("execution failed")
                )));
            }
            _ => return Err(ApiError::decode("Sui simulation: missing status")),
        }
        let gas = &effects["gasEffects"]["gasSummary"];
        let cost = |field: &str| number(&gas[field], "Sui simulation: missing gas used");
        Ok((
            cost("computationCost")?,
            cost("storageCost")?,
            cost("storageRebate")?,
        ))
    }

    /// Execute a signed transaction; its digest once it is final and
    /// succeeded.
    pub async fn execute_signed_tx(
        &self,
        tx_bytes_b64: &str,
        sig_b64: &str,
    ) -> Result<String, ApiError> {
        let data = self
            .post(
                EXECUTE,
                json!({"transaction": tx_bytes_b64, "signatures": [sig_b64]}),
                RetryProfile::ChainWrite,
            )
            .await?;
        let effects = &data["executeTransaction"]["effects"];
        if effects["status"].as_str() != Some("SUCCESS") {
            return Err(ApiError::Rejected(format!(
                "Sui execution did not succeed: {}",
                effects["executionError"]["message"]
                    .as_str()
                    .unwrap_or("no effects")
            )));
        }
        Ok(effects["digest"]
            .as_str()
            .filter(|digest| !digest.is_empty())
            .or_decode("missing Sui transaction digest")?
            .to_string())
    }

    pub(crate) async fn fetch_transaction_status(
        &self,
        digest: &str,
    ) -> Result<TransactionStatus, ApiError> {
        crate::api::transaction_status::validate_base58_hash(digest, 32)?;
        let data = self.query(STATUS, json!({"digest": digest})).await?;
        transaction_status(&data["transactionEffects"], digest)
    }

    /// A page of the address's transfers, newest first, read from the
    /// balance changes of every transaction it took part in.
    pub async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<SuiHistoryEntry>, ApiError> {
        let data = self
            .query(HISTORY, json!({"owner": address, "before": cursor}))
            .await?;
        let connection = &data["address"]["transactions"];
        let mut transactions = Vec::new();
        for node in nodes(connection, "Sui history: missing transactions")? {
            let mut node = node.clone();
            self.complete_balance_changes(&mut node).await?;
            transactions.push(node);
        }
        let info = &connection["pageInfo"];
        let next_cursor = if info["hasPreviousPage"]
            .as_bool()
            .or_decode("Sui history: missing page info")?
        {
            Some(
                info["startCursor"]
                    .as_str()
                    .or_decode("Sui history: missing cursor")?
                    .to_string(),
            )
        } else {
            None
        };
        let mut items = history_entries(&transactions, address)?;
        if items
            .iter()
            .filter_map(|entry| entry.contract.as_ref())
            .collect::<BTreeSet<_>>()
            .len()
            > 100
        {
            return Err(ApiError::Rejected(
                "Sui history page exceeds 100 token types".into(),
            ));
        }
        let mut decimals = HashMap::new();
        for entry in &mut items {
            if let Some(contract) = &entry.contract {
                let precision = match decimals.get(contract) {
                    Some(precision) => *precision,
                    None => {
                        let precision = self.coin_decimals(contract).await?;
                        decimals.insert(contract.clone(), precision);
                        precision
                    }
                };
                entry.amount_display = Some(
                    crate::amount::decimal::from_unit_digits(
                        &entry.amount_mist,
                        u32::from(precision),
                    )
                    .or_decode("Sui token history: invalid amount")?,
                );
            }
        }
        Ok(crate::api::HistoryPage { items, next_cursor })
    }

    /// A transaction's balance changes beyond the first page, read into its
    /// node so the node holds all of them.
    async fn complete_balance_changes(&self, transaction: &mut Value) -> Result<(), ApiError> {
        let digest = transaction["digest"]
            .as_str()
            .or_decode("Sui history: missing digest")?
            .to_string();
        let changes = &transaction["effects"]["balanceChanges"];
        let mut all = nodes(changes, "Sui history: missing balance changes")?.to_vec();
        let mut seen = HashSet::new();
        let mut cursor = next_page(changes, &mut seen)?;
        let mut pages = 0;
        while let Some(after) = cursor {
            pages += 1;
            if pages == MAX_PAGES {
                return Err(too_many("balance changes"));
            }
            let data = self
                .query(BALANCE_CHANGES, json!({"digest": digest, "after": after}))
                .await?;
            let page = &data["transactionEffects"]["balanceChanges"];
            all.extend_from_slice(nodes(page, "Sui history: missing balance changes")?);
            cursor = next_page(page, &mut seen)?;
        }
        transaction["effects"]["balanceChanges"]["nodes"] = Value::Array(all);
        Ok(())
    }

    /// The active validators, from every page of the current epoch's set.
    pub async fn fetch_staking_validators(&self) -> Result<Vec<SuiValidator>, ApiError> {
        let mut validators = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..MAX_PAGES {
            let data = self.query(VALIDATORS, json!({"after": cursor})).await?;
            let connection = &data["epoch"]["validatorSet"]["activeValidators"];
            for node in nodes(connection, "Sui validators: missing list")? {
                validators.push(validator(&node["contents"]["json"])?);
            }
            match next_page(connection, &mut seen)? {
                Some(next) => cursor = Some(next),
                None => return Ok(validators),
            }
        }
        Err(too_many("validators"))
    }

    /// An object's current version, digest, owner, type and contents.
    async fn object(&self, id: &str) -> Result<MoveObject, ApiError> {
        let data = self.query(OBJECT, json!({"id": id})).await?;
        let object = data
            .get("object")
            .filter(|object| object.is_object())
            .ok_or_else(|| ApiError::Rejected(format!("Sui object {id} does not exist")))?;
        MoveObject::read(object, &object["asMoveObject"]["contents"])
    }

    /// The system state object is the one shared at the version the
    /// staking calls name.
    pub(crate) async fn validate_staking_system(&self, chain: Chain) -> Result<(), ApiError> {
        let (_, id, version) = chain.sui_staking_system()?;
        let object = self.object(id).await?;
        if !matches!(object.owner, Owner::Shared(shared) if shared == version)
            || object.type_ != SUI_SYSTEM_STATE
        {
            return Err(ApiError::invalid("Unexpected Sui staking system identity"));
        }
        Ok(())
    }

    pub(crate) async fn fetch_staked_object(
        &self,
        id: &str,
        owner: &str,
    ) -> Result<SuiStakedObject, ApiError> {
        let object = self.object(id).await?;
        if !object.held_by(owner) || object.type_ != STAKED_SUI || object.id != long_address(id) {
            return Err(ApiError::invalid(
                "Sui stake object identity or owner differs from wallet",
            ));
        }
        staked_object(&object)
    }

    /// Every `StakedSui` object `owner` holds, with the active validator
    /// whose pool holds it.
    pub(crate) async fn fetch_stakes(&self, owner: &str) -> Result<Vec<SuiStake>, ApiError> {
        let ((epoch, _), validators) =
            futures::try_join!(self.epoch(), self.fetch_staking_validators())?;
        let pools: HashMap<String, String> = validators
            .into_iter()
            .map(|validator| {
                (
                    long_address(&validator.staking_pool_id),
                    validator.sui_address,
                )
            })
            .collect();
        let mut stakes = Vec::new();
        let mut ids = HashSet::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..MAX_PAGES {
            let connection = self
                .objects_page(owner, STAKED_SUI, cursor.as_deref())
                .await?;
            for node in nodes(&connection, "Sui stakes: missing list")? {
                let object = MoveObject::listed(node, owner)?;
                if object.type_ != STAKED_SUI || !ids.insert(object.id.clone()) {
                    return Err(ApiError::decode(
                        "Sui stake: malformed or duplicate position",
                    ));
                }
                let staked = staked_object(&object)?;
                stakes.push(SuiStake {
                    validator: pools.get(&long_address(&staked.pool_id)).cloned(),
                    active: staked.activation_epoch <= epoch,
                    id: object.id,
                    object: staked,
                });
            }
            match next_page(&connection, &mut seen)? {
                Some(next) => cursor = Some(next),
                None => return Ok(stakes),
            }
        }
        Err(too_many("stakes"))
    }

    pub(crate) async fn validate_owned_sui_coin(
        &self,
        id: &str,
        owner: &str,
        version: u64,
        digest: &[u8; 32],
        balance: u64,
    ) -> Result<(), ApiError> {
        let object = self.object(id).await?;
        if object.id != long_address(id)
            || !object.held_by(owner)
            || coin_of(&object.type_).as_deref() != Some(SUI_COIN_TYPE)
            || object.version != version
            || object.digest != *digest
            || digits::<u64>(&object.json["balance"], "").ok() != Some(balance)
        {
            return Err(ApiError::invalid(
                "Reviewed Sui gas object changed or is not owned by this wallet",
            ));
        }
        Ok(())
    }
}

// ── Reading answers

/// The name of a document's one operation: the word after `query` or
/// `mutation`.
fn operation_name(document: &str) -> &str {
    document
        .split_whitespace()
        .nth(1)
        .and_then(|word| word.split(['(', '{']).next())
        .unwrap_or_default()
}

/// The `data` of an answer; one with `errors` refused the request.
fn answer(response: Value) -> Result<Value, ApiError> {
    if let Some(errors) = response
        .get("errors")
        .filter(|errors| errors.as_array().is_none_or(|errors| !errors.is_empty()))
    {
        let messages: Vec<&str> = errors
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|error| error["message"].as_str())
            .collect();
        return Err(ApiError::Rejected(format!(
            "Sui GraphQL: {}",
            if messages.is_empty() {
                errors.to_string()
            } else {
                messages.join("; ")
            }
        )));
    }
    response
        .get("data")
        .filter(|data| data.is_object())
        .cloned()
        .or_decode("Sui GraphQL: missing data")
}

fn nodes<'a>(connection: &'a Value, what: &str) -> Result<&'a [Value], ApiError> {
    connection["nodes"]
        .as_array()
        .map(Vec::as_slice)
        .or_decode(what)
}

/// Where a connection's next page starts; `None` after its last. A cursor
/// seen before would page forever, and is refused.
fn next_page(connection: &Value, seen: &mut HashSet<String>) -> Result<Option<String>, ApiError> {
    let info = &connection["pageInfo"];
    if !info["hasNextPage"]
        .as_bool()
        .or_decode("Sui GraphQL: missing page info")?
    {
        return Ok(None);
    }
    let cursor = info["endCursor"]
        .as_str()
        .or_decode("Sui GraphQL: missing page cursor")?
        .to_string();
    if !seen.insert(cursor.clone()) {
        return Err(ApiError::decode("Sui GraphQL: repeated page cursor"));
    }
    Ok(Some(cursor))
}

fn too_many(what: &str) -> ApiError {
    ApiError::Rejected(format!("Sui: more than {MAX_PAGES} pages of {what}"))
}

/// A `UInt53` or an `Int`, which GraphQL writes as a JSON number.
fn number(value: &Value, what: &str) -> Result<u64, ApiError> {
    value.as_u64().or_decode(what)
}

/// A `BigInt` or a Move `u64`, which GraphQL writes as a string of digits.
fn digits<T: std::str::FromStr>(value: &Value, what: &str) -> Result<T, ApiError> {
    value
        .as_str()
        .filter(|text| {
            let text = text.strip_prefix('-').unwrap_or(text);
            !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
        })
        .and_then(|text| text.parse().ok())
        .or_decode(what)
}

/// An address as GraphQL writes it: `0x` and 64 lowercase hex digits.
fn long_address(address: &str) -> String {
    let address = address.trim().to_ascii_lowercase();
    format!("0x{:0>64}", address.strip_prefix("0x").unwrap_or(&address))
}

/// A coin type as the catalog writes it, its package without leading zeros.
fn short_type(repr: &str) -> String {
    crate::catalog::tokens::normalize_sui_token_identifier(repr.to_string())
}

/// The coin type `T` of a `0x2::coin::Coin<T>` object type.
fn coin_of(object_type: &str) -> Option<String> {
    object_type
        .strip_prefix("0x2::coin::Coin<")?
        .strip_suffix('>')
        .map(short_type)
}

enum Owner {
    Address(String),
    /// Its initial shared version.
    Shared(u64),
    Other,
}

impl Owner {
    fn read(owner: &Value) -> Result<Self, ApiError> {
        Ok(match owner["__typename"].as_str() {
            Some("AddressOwner") => Self::Address(long_address(
                owner["address"]["address"]
                    .as_str()
                    .or_decode("Sui object: missing owner")?,
            )),
            Some("Shared") => Self::Shared(number(
                &owner["initialSharedVersion"],
                "Sui object: missing initial shared version",
            )?),
            _ => Self::Other,
        })
    }
}

/// An object's identity, owner and contents; `type_` as [`short_type`]
/// writes it.
struct MoveObject {
    id: String,
    version: u64,
    digest: [u8; 32],
    owner: Owner,
    type_: String,
    json: Value,
}

impl MoveObject {
    /// An object a listing of `owner`'s objects names, which has to be
    /// `owner`'s.
    fn listed(node: &Value, owner: &str) -> Result<Self, ApiError> {
        let object = Self::read(node, &node["contents"])?;
        if !object.held_by(owner) {
            return Err(ApiError::invalid("Sui object owner differs from wallet"));
        }
        Ok(object)
    }

    fn held_by(&self, owner: &str) -> bool {
        matches!(&self.owner, Owner::Address(address) if *address == long_address(owner))
    }

    fn read(object: &Value, contents: &Value) -> Result<Self, ApiError> {
        let digest = bs58::decode(
            object["digest"]
                .as_str()
                .or_decode("Sui object: missing digest")?,
        )
        .into_vec()
        .map_err(|_| ApiError::decode("Sui object: invalid digest"))?
        .try_into()
        .map_err(|_| ApiError::decode("Sui object: digest must be 32 bytes"))?;
        Ok(Self {
            id: long_address(
                object["address"]
                    .as_str()
                    .or_decode("Sui object: missing id")?,
            ),
            version: number(&object["version"], "Sui object: missing version")?,
            digest,
            owner: Owner::read(&object["owner"])?,
            type_: short_type(
                contents["type"]["repr"]
                    .as_str()
                    .or_decode("Sui object: missing type")?,
            ),
            json: contents["json"].clone(),
        })
    }
}

fn staked_object(object: &MoveObject) -> Result<SuiStakedObject, ApiError> {
    let fields = &object.json;
    Ok(SuiStakedObject {
        version: object.version,
        digest: object.digest,
        principal: digits(&fields["principal"], "Sui stake: missing principal")?,
        pool_id: fields["pool_id"]
            .as_str()
            .or_decode("Sui stake: missing pool")?
            .to_string(),
        activation_epoch: digits(
            &fields["stake_activation_epoch"],
            "Sui stake: missing activation epoch",
        )?,
    })
}

fn validator(fields: &Value) -> Result<SuiValidator, ApiError> {
    let metadata = &fields["metadata"];
    let text = |value: &Value| value.as_str().unwrap_or_default().to_string();
    Ok(SuiValidator {
        sui_address: long_address(
            metadata["sui_address"]
                .as_str()
                .or_decode("Sui validator: missing address")?,
        ),
        name: text(&metadata["name"]),
        description: text(&metadata["description"]),
        project_url: text(&metadata["project_url"]),
        commission_rate: digits(
            &fields["commission_rate"],
            "Sui validator: missing commission",
        )?,
        staking_pool_sui_balance: digits::<u128>(
            &fields["staking_pool"]["sui_balance"],
            "Sui validator: missing pool balance",
        )?
        .to_string(),
        staking_pool_id: fields["staking_pool"]["id"]
            .as_str()
            .or_decode("Sui validator: missing pool")?
            .to_string(),
    })
}

/// A digest's status from its effects: pending until a checkpoint holds it,
/// then settled by its execution result. No effects is not executed yet.
fn transaction_status(effects: &Value, digest: &str) -> Result<TransactionStatus, ApiError> {
    if effects.is_null() {
        return Ok(TransactionStatus::Pending);
    }
    if effects["digest"].as_str() != Some(digest) {
        return Err(ApiError::decode("Sui status: transaction digest mismatch"));
    }
    let Some(checkpoint) = effects.get("checkpoint").filter(|value| !value.is_null()) else {
        return Ok(TransactionStatus::Pending);
    };
    let block = number(
        &checkpoint["sequenceNumber"],
        "Sui status: invalid checkpoint",
    )?;
    let succeeded = match effects["status"].as_str() {
        Some("SUCCESS") => true,
        Some("FAILURE") => false,
        _ => return Err(ApiError::decode("Sui status: missing execution result")),
    };
    Ok(TransactionStatus::Confirmed {
        succeeded,
        block: Some(block),
    })
}

/// Each transaction's transfer of each coin type for `address`, newest
/// first.
///
/// A balance change for the gas owner includes the gas, which is a fee and not
/// a transfer, so it is added back: a transaction that only paid gas moved
/// nothing and yields no entry. Gas can be negative when a storage rebate
/// exceeds the cost, and the same arithmetic holds.
fn history_entries(
    transactions: &[Value],
    address: &str,
) -> Result<Vec<SuiHistoryEntry>, ApiError> {
    let address = long_address(address);
    let mut entries = Vec::new();
    for transaction in transactions {
        let digest = transaction["digest"]
            .as_str()
            .filter(|digest| !digest.is_empty())
            .or_decode("Sui history: missing digest")?;
        let effects = &transaction["effects"];
        let mut changes = Vec::new();
        for change in nodes(
            &effects["balanceChanges"],
            "Sui history: missing balance changes",
        )? {
            // A change without an owner is no address's.
            let Some(owner) = change["owner"]["address"].as_str() else {
                continue;
            };
            changes.push((
                long_address(owner),
                short_type(
                    change["coinType"]["repr"]
                        .as_str()
                        .or_decode("Sui history: missing coin type")?,
                ),
                digits::<i128>(&change["amount"], "Sui history: malformed amount")?,
            ));
        }
        let gas_owner = transaction["gasInput"]["gasSponsor"]["address"]
            .as_str()
            .map(long_address);
        let gas = if gas_owner.as_deref() == Some(address.as_str()) {
            let summary = &effects["gasEffects"]["gasSummary"];
            let cost = |field: &str| {
                number(&summary[field], "Sui history: missing gas used").map(i128::from)
            };
            cost("computationCost")? + cost("storageCost")? - cost("storageRebate")?
        } else {
            0
        };
        let sender = transaction["sender"]["address"]
            .as_str()
            .map(long_address)
            .unwrap_or_default();
        let timestamp_ms = crate::api::time::history_time(
            !effects["checkpoint"].is_null(),
            effects["timestamp"]
                .as_str()
                .and_then(crate::api::time::parse_iso8601_timestamp)
                .map(|seconds| seconds as u64 * 1000),
            digest,
        )?;
        let held: BTreeSet<&str> = changes
            .iter()
            .filter(|(owner, ..)| *owner == address)
            .map(|(_, kind, _)| kind.as_str())
            .collect();
        for kind in held {
            let of_kind = || changes.iter().filter(move |(_, other, _)| other == kind);
            let net = of_kind()
                .filter(|(owner, ..)| *owner == address)
                .try_fold(0i128, |total, (_, _, amount)| total.checked_add(*amount))
                .or_decode("Sui history amount overflow")?;
            let transfer = net
                .checked_add(if kind == SUI_COIN_TYPE { gas } else { 0 })
                .or_decode("Sui history net amount overflow")?;
            if transfer == 0 {
                continue;
            }
            let amount_mist = transfer.unsigned_abs().to_string();
            let is_incoming = transfer > 0;
            let (from, to) = if is_incoming {
                (sender.clone(), address.clone())
            } else {
                let recipient = of_kind()
                    .filter(|(owner, _, amount)| *owner != address && *amount > 0)
                    .max_by_key(|(_, _, amount)| *amount)
                    .map(|(owner, ..)| owner.clone())
                    .unwrap_or_default();
                (address.clone(), recipient)
            };
            entries.push(SuiHistoryEntry {
                digest: digest.to_string(),
                timestamp_ms,
                is_incoming,
                amount_display: (kind == SUI_COIN_TYPE)
                    .then(|| {
                        crate::amount::decimal::from_unit_digits(
                            &amount_mist,
                            u32::from(Chain::Sui.native_decimals()),
                        )
                    })
                    .flatten(),
                amount_mist,
                contract: (kind != SUI_COIN_TYPE).then(|| kind.to_string()),
                from,
                to,
            });
        }
    }
    // Undated transactions are the newest.
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms.unwrap_or(u64::MAX)));
    Ok(entries)
}

#[cfg(test)]
#[path = "tests/sui_graphql.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/sui_graphql_node.rs"]
pub(crate) mod node;
