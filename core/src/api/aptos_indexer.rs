//! Aptos address history and fungible holdings from the public Indexer API.
//! Account versions include incoming and orderless transactions; account REST
//! sequence-number history cannot serve this contract.
use crate::api::{
    HeldToken, HistoryPage,
    aptos_rest::AptosClient,
    error::{ApiError, OrDecode},
    http::{HttpClient, RetryProfile, race},
};
use futures::{StreamExt, stream};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

pub struct AptosIndexerClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
    chain_id: u8,
}
impl AptosIndexerClient {
    pub fn new(endpoints: Arc<Vec<String>>, chain_id: u8) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
            chain_id,
        }
    }
    async fn query(&self, query: &str, variables: Value) -> Result<Value, ApiError> {
        let body = json!({"query":query,"variables":variables});
        race(&self.endpoints, |base| {
            let body = &body;
            async move {
                let response: Value = self
                    .client
                    .post_json(&base, body, RetryProfile::ChainRead)
                    .await?;
                if response
                    .get("errors")
                    .is_some_and(|errors| errors.as_array().is_none_or(|errors| !errors.is_empty()))
                {
                    return Err(ApiError::Rejected(format!(
                        "Aptos indexer: {}",
                        response["errors"]
                    )));
                }
                let data = response.get("data").or_decode("indexer: missing data")?;
                let id = data["ledger_infos"]
                    .as_array()
                    .and_then(|rows| rows.first())
                    .and_then(|row| row["chain_id"].as_u64())
                    .or_decode("indexer: missing chain id")?;
                if id != u64::from(self.chain_id) {
                    return Err(ApiError::Rejected(format!(
                        "Aptos indexer chain id {id}, expected {}",
                        self.chain_id
                    )));
                }
                Ok(data.clone())
            }
        })
        .await
    }
    pub(crate) async fn verify_network(&self) -> Result<(), ApiError> {
        self.query("query { ledger_infos { chain_id } }", json!({}))
            .await
            .map(|_| ())
    }

    /// Every pool with nonzero delegator shares, including pools outside the
    /// current validator set. Amounts remain REST Move-view authority.
    pub(crate) async fn fetch_delegation_pool_ids(
        &self,
        owner: &str,
    ) -> Result<Vec<String>, ApiError> {
        let owner = padded_address(owner)?;
        let mut after = String::new();
        let mut pools = Vec::new();
        for _ in 0..100 {
            let data=self.query("query Stakes($owner:String!,$after:String!){ledger_infos{chain_id} current_delegator_balances(where:{delegator_address:{_eq:$owner},shares:{_gt:0},pool_address:{_gt:$after}},distinct_on:[pool_address],order_by:{pool_address:asc},limit:1000){pool_address}}",json!({"owner":owner,"after":after})).await?;
            let rows = data["current_delegator_balances"]
                .as_array()
                .or_decode("Aptos staking: missing indexed pool rows")?;
            for row in rows {
                let pool = padded_address(
                    row["pool_address"]
                        .as_str()
                        .or_decode("Aptos staking: missing pool address")?,
                )?;
                if pool <= after {
                    return Err(ApiError::decode(
                        "Aptos staking: pool pagination did not advance",
                    ));
                }
                after = pool.clone();
                pools.push(pool);
            }
            if rows.len() < 1000 {
                return Ok(pools);
            }
        }
        Err(ApiError::decode(
            "Aptos staking discovery exceeds pagination limit",
        ))
    }
    pub async fn fetch_holdings(&self, address: &str) -> Result<Vec<HeldToken>, ApiError> {
        let address = padded_address(address)?;
        let mut after = String::new();
        let mut holdings = BTreeMap::<String, (u128, u8)>::new();
        for _ in 0..10 {
            let data=self.query("query Holdings($owner:String!,$after:String!){ledger_infos{chain_id} current_fungible_asset_balances(where:{owner_address:{_eq:$owner},storage_id:{_gt:$after}},order_by:{storage_id:asc},limit:1000){storage_id asset_type amount metadata{decimals}}}",json!({"owner":address,"after":after})).await?;
            let rows = data["current_fungible_asset_balances"]
                .as_array()
                .or_decode("indexer: missing holdings")?;
            for row in rows {
                let asset = asset_identifier(
                    row["asset_type"]
                        .as_str()
                        .or_decode("holding: missing asset type")?,
                )?;
                if native_asset(&asset) {
                    continue;
                }
                let amount = units(&row["amount"])?;
                let decimals = precision(row)?;
                let holding = holdings.entry(asset).or_insert((0, decimals));
                if holding.1 != decimals {
                    return Err(ApiError::Decode(
                        "inconsistent Aptos token precision".into(),
                    ));
                }
                holding.0 = holding
                    .0
                    .checked_add(amount)
                    .or_decode("Aptos holdings overflow")?;
            }
            if rows.len() < 1000 {
                return Ok(holdings
                    .into_iter()
                    .filter(|(_, (amount, _))| *amount > 0)
                    .map(|(contract, (balance_raw, decimals))| HeldToken {
                        contract,
                        balance_raw,
                        decimals: Some(decimals),
                    })
                    .collect());
            }
            let next = rows
                .last()
                .and_then(|row| row["storage_id"].as_str())
                .or_decode("holding: missing storage cursor")?;
            if next <= after.as_str() {
                return Err(ApiError::Decode("indexer repeated storage cursor".into()));
            }
            after = next.into();
        }
        Err(ApiError::Rejected(
            "Aptos discovery exceeds 10000 storage objects; use a narrower indexed source".into(),
        ))
    }
    pub async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
        node: &AptosClient,
    ) -> Result<HistoryPage<Value>, ApiError> {
        let address = padded_address(address)?;
        let before = cursor
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| ApiError::invalid("invalid Aptos history cursor"))
            })
            .transpose()?;
        let predicate = if before.is_some() {
            ",transaction_version:{_lt:$before}"
        } else {
            ""
        };
        let declaration = if before.is_some() {
            ",$before:bigint!"
        } else {
            ""
        };
        let query = format!(
            "query History($owner:String!{declaration}){{ledger_infos{{chain_id}} account_transactions(where:{{account_address:{{_eq:$owner}}{predicate}}},order_by:{{transaction_version:desc}},limit:50){{transaction_version fungible_asset_activities(where:{{owner_address:{{_eq:$owner}}}},limit:1001){{amount asset_type type is_gas_fee is_transaction_success metadata{{decimals}}}}}}}}"
        );
        let data = self
            .query(
                &query,
                json!({"owner":address,"before":before.map(|v|v.to_string())}),
            )
            .await?;
        let rows = data["account_transactions"]
            .as_array()
            .or_decode("indexer: missing account versions")?;
        let mut versions = Vec::new();
        let mut previous = before.unwrap_or(u64::MAX);
        for row in rows {
            let version = units(&row["transaction_version"])
                .and_then(|v| u64::try_from(v).map_err(ApiError::decode))?;
            if version >= previous {
                return Err(ApiError::Decode(
                    "indexer account versions are not descending".into(),
                ));
            }
            previous = version;
            let activities = row["fungible_asset_activities"]
                .as_array()
                .or_decode("indexer: missing activities")?;
            if activities.len() > 1000 {
                return Err(ApiError::Rejected(
                    "Aptos transaction exceeds 1000 asset activities".into(),
                ));
            }
            versions.push((version, activities.clone()));
        }
        let next_cursor = (rows.len() == 50).then(|| previous.to_string());
        if !versions.is_empty() {
            let (id, _) = node.fetch_ledger_info().await?;
            if id != u64::from(self.chain_id) {
                return Err(ApiError::Rejected(
                    "Aptos fullnode and indexer networks differ".into(),
                ));
            }
        }
        let batches: Vec<_> = stream::iter(versions)
            .map(|(version, activities)| {
                let address = &address;
                async move {
                    let transaction = node.fetch_transaction_version(version).await?;
                    decode_transaction(version, activities, &transaction, address)
                }
            })
            .buffer_unordered(4)
            .collect()
            .await;
        let mut items = Vec::new();
        for batch in batches {
            items.extend(batch?);
        }
        Ok(HistoryPage { items, next_cursor })
    }
}
fn padded_address(address: &str) -> Result<String, ApiError> {
    let raw = address
        .strip_prefix("0x")
        .or_decode("Aptos address must start with 0x")?;
    if raw.is_empty() || raw.len() > 64 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::invalid("invalid Aptos address"));
    }
    Ok(format!("0x{raw:0>64}").to_ascii_lowercase())
}
fn asset_identifier(asset: &str) -> Result<String, ApiError> {
    if let Some((address, rest)) = asset.split_once("::") {
        let address = padded_address(address)?;
        Ok(format!(
            "0x{}::{rest}",
            address[2..].trim_start_matches('0')
        ))
    } else {
        padded_address(asset)
    }
}
fn native_asset(asset: &str) -> bool {
    asset == "0x1::aptos_coin::AptosCoin" || asset == format!("0x{:0>64}", "a")
}
fn units(value: &Value) -> Result<u128, ApiError> {
    value
        .as_u64()
        .map(u128::from)
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .or_decode("indexer: malformed integer amount")
}
fn precision(row: &Value) -> Result<u8, ApiError> {
    crate::api::checked_token_decimals(units(&row["metadata"]["decimals"])?)
}
fn decode_transaction(
    version: u64,
    activities: Vec<Value>,
    tx: &Value,
    address: &str,
) -> Result<Vec<Value>, ApiError> {
    if units(&tx["version"])? != u128::from(version) {
        return Err(ApiError::Decode(
            "Aptos transaction version mismatch".into(),
        ));
    }
    let hash = tx["hash"]
        .as_str()
        .or_decode("Aptos transaction: missing hash")?;
    if hash.len() != 66
        || !hash.starts_with("0x")
        || !hash[2..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(ApiError::Decode("invalid Aptos transaction hash".into()));
    }
    let timestamp = u64::try_from(units(&tx["timestamp"])?).map_err(ApiError::decode)?;
    if timestamp == 0 {
        return Err(ApiError::Decode(
            "Aptos transaction: missing timestamp".into(),
        ));
    }
    if tx["success"].as_bool() != Some(true) {
        return Ok(Vec::new());
    }
    let sender = padded_address(
        tx["sender"]
            .as_str()
            .or_decode("Aptos transaction: missing sender")?,
    )?;
    let mut totals = BTreeMap::<String, (u128, u128, u8)>::new();
    for activity in activities {
        if activity["is_gas_fee"]
            .as_bool()
            .or_decode("Aptos activity: missing gas flag")?
        {
            continue;
        }
        if activity["is_transaction_success"].as_bool() != Some(true) {
            return Err(ApiError::Decode("Aptos indexer success mismatch".into()));
        }
        let kind = activity["type"]
            .as_str()
            .or_decode("Aptos activity: missing type")?;
        let incoming = if kind.ends_with("::DepositEvent") || kind.ends_with("::Deposit") {
            true
        } else if kind.ends_with("::WithdrawEvent") || kind.ends_with("::Withdraw") {
            false
        } else {
            continue;
        };
        let asset = asset_identifier(
            activity["asset_type"]
                .as_str()
                .or_decode("Aptos activity: missing asset")?,
        )?;
        let decimals = if native_asset(&asset) {
            8
        } else {
            precision(&activity)?
        };
        let amount = units(&activity["amount"])?;
        let total = totals.entry(asset).or_insert((0, 0, decimals));
        if total.2 != decimals {
            return Err(ApiError::Decode(
                "inconsistent Aptos activity precision".into(),
            ));
        }
        let leg = if incoming { &mut total.0 } else { &mut total.1 };
        *leg = leg
            .checked_add(amount)
            .or_decode("Aptos history amount overflow")?;
    }
    Ok(totals.into_iter().filter(|(_, (received,spent,_))|received!=spent).map(|(asset,(received,spent,decimals))|{
        let incoming=received>spent;let amount=received.abs_diff(spent);
        json!({"txid":hash,"version":version,"timestamp_us":timestamp,"from":if incoming&&sender!=address{sender.as_str()}else{""},"to":"","amount_octas":amount.to_string(),"amount_display":crate::decimal::from_units(amount,u32::from(decimals)),"contract":if native_asset(&asset){None}else{Some(asset)},"is_incoming":incoming})
    }).collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incoming_and_orderless_assets_use_activities_and_exact_precision() {
        let address = padded_address("0x12").unwrap();
        let tx = json!({"version":"7","hash":format!("0x{}","ab".repeat(32)),"timestamp":"1700000000000000","success":true,"sender":"0x34","replay_protection_nonce":"99"});
        let activities = vec![
            json!({"amount":"1200000","asset_type":"0xbeef","type":"0x1::fungible_asset::Deposit","is_gas_fee":false,"is_transaction_success":true,"metadata":{"decimals":6}}),
            json!({"amount":"500","asset_type":"0x1::aptos_coin::AptosCoin","type":"0x1::coin::WithdrawEvent","is_gas_fee":true,"is_transaction_success":true}),
        ];
        let entries = decode_transaction(7, activities, &tx, &address).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["amount_display"], "1.2");
        assert_eq!(entries[0]["is_incoming"], true);
        assert_eq!(entries[0]["contract"], padded_address("0xbeef").unwrap());
    }
    /// Holdings of a thousand storage objects to a page, the first all APT
    /// (no token), the second the token at `decimals`.
    async fn holdings(decimals: u64) -> Result<Vec<HeldToken>, ApiError> {
        use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(move |request: &Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let rows: Vec<Value> = match body["variables"]["after"].as_str().unwrap() {
                    "" => (0..1000)
                        .map(|n| {
                            json!({"storage_id": format!("{n:04}"),
                            "asset_type": "0x1::aptos_coin::AptosCoin", "amount": "1"})
                        })
                        .collect(),
                    "0999" => vec![json!({"storage_id": "1000",
                        "asset_type": format!("0x{}", "44".repeat(32)), "amount": "2500000",
                        "metadata": {"decimals": decimals}})],
                    other => panic!("unexpected cursor {other}"),
                };
                ResponseTemplate::new(200).set_body_json(json!({"data": {
                    "ledger_infos": [{"chain_id": 1}], "current_fungible_asset_balances": rows}}))
            })
            .mount(&server)
            .await;
        AptosIndexerClient::new(Arc::new(vec![server.uri()]), 1)
            .fetch_holdings(&format!("0x{}", "11".repeat(32)))
            .await
    }

    /// A full page is followed from its last storage object to the next;
    /// APT is the native coin, not a token held.
    #[tokio::test]
    async fn holdings_are_read_past_every_full_page() {
        let held = holdings(6).await.unwrap();
        assert_eq!(
            held.iter()
                .map(|token| (token.contract.clone(), token.balance_raw, token.decimals))
                .collect::<Vec<_>>(),
            [(format!("0x{}", "44".repeat(32)), 2_500_000, Some(6))]
        );
    }

    #[tokio::test]
    async fn a_holding_past_38_decimals_refuses_the_listing() {
        let error = holdings(39).await.unwrap_err().to_string();
        assert!(error.contains("precision limit (38)"), "{error}");
    }

    #[test]
    fn unknown_precision_and_network_identity_are_refused() {
        assert!(precision(&json!({"metadata":{"decimals":39}})).is_err());
        assert!(padded_address("0xnot_hex").is_err());
    }
}

#[cfg(test)]
mod paging_tests {
    use super::*;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

    /// A full page of versions none of which moved an asset for the address
    /// decodes to nothing and is still not the end: its cursor is the last
    /// version read. The next page asks for versions before it, and a short
    /// page ends the history. A cursor that is not a version is refused
    /// before anything is asked.
    #[tokio::test]
    async fn an_empty_decoded_page_is_not_the_end_and_pages_on_before_its_last_version() {
        let owner = padded_address("0x12").unwrap();
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(|request: &Request| {
                let body = if request.url.path() == "/graphql" {
                    let query: Value = request.body_json().unwrap();
                    let versions: Vec<u64> = if query["variables"]["before"].is_null() {
                        (51..=100).rev().collect()
                    } else {
                        vec![50]
                    };
                    let deposit = json!({"amount": "1250000", "asset_type": "0xbeef",
                        "type": "0x1::fungible_asset::Deposit", "is_gas_fee": false,
                        "is_transaction_success": true, "metadata": {"decimals": 6}});
                    let gas = json!({"amount": "100", "asset_type": "0x1::aptos_coin::AptosCoin",
                        "type": "0x1::coin::WithdrawEvent", "is_gas_fee": true,
                        "is_transaction_success": true});
                    let rows: Vec<_> = versions
                        .into_iter()
                        .map(|version| json!({"transaction_version": version.to_string(),
                            "fungible_asset_activities": if version == 50 { json!([deposit, gas]) } else { json!([]) }}))
                        .collect();
                    json!({"data": {"ledger_infos": [{"chain_id": 1}], "account_transactions": rows}})
                } else if let Some(version) = request.url.path().strip_prefix("/v1/transactions/by_version/") {
                    let version: u64 = version.parse().unwrap();
                    json!({"version": version.to_string(), "hash": format!("0x{version:064x}"),
                        "timestamp": "1700000000000000", "success": true, "sender": "0x99",
                        "replay_protection_nonce": "123"})
                } else {
                    json!({"chain_id": 1, "ledger_version": "101"})
                };
                ResponseTemplate::new(200).set_body_json(body)
            })
            .mount(&server)
            .await;
        let indexer =
            AptosIndexerClient::new(Arc::new(vec![format!("{}/graphql", server.uri())]), 1);
        let node = AptosClient::new(Arc::new(vec![format!("{}/v1", server.uri())]));

        let first = indexer
            .fetch_history_page(&owner, None, &node)
            .await
            .unwrap();
        assert!(first.items.is_empty());
        assert_eq!(first.next_cursor.as_deref(), Some("51"));
        let second = indexer
            .fetch_history_page(&owner, first.next_cursor.as_deref(), &node)
            .await
            .unwrap();
        assert_eq!(second.next_cursor, None);
        let rows: Vec<_> = second
            .items
            .iter()
            .map(|row| (row["amount_display"].clone(), row["is_incoming"].clone()))
            .collect();
        assert_eq!(rows, [(json!("1.25"), json!(true))]);
        let asked: Vec<Value> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path() == "/graphql")
            .map(|request| request.body_json::<Value>().unwrap())
            .collect();
        assert_eq!(
            asked
                .iter()
                .map(|query| query["variables"]["before"].clone())
                .collect::<Vec<_>>(),
            [Value::Null, json!("51")]
        );
        assert!(
            asked[1]["query"]
                .as_str()
                .unwrap()
                .contains("transaction_version:{_lt:$before}")
        );

        let asked_before = server.received_requests().await.unwrap().len();
        assert_eq!(
            indexer
                .fetch_history_page(&owner, Some("not-a-version"), &node)
                .await
                .unwrap_err(),
            ApiError::invalid("invalid Aptos history cursor")
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            asked_before
        );
    }
}
