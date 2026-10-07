//! SUI transfer: resolve gas objects, build a local PTB, sign, execute.

use super::bcs;
use crate::api::sui_json_rpc::SuiClient;
use crate::send::error::SendError;
use crate::send::keys::Ed25519Seed;
use base64::{Engine, engine::general_purpose::STANDARD};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct GasCoin {
    pub id: [u8; 32],
    pub version: u64,
    pub digest: [u8; 32],
    pub balance: u64,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedSuiTransfer {
    sender: [u8; 32],
    pub(crate) bytes: Vec<u8>,
    pub(crate) objects: Vec<GasCoin>,
    pub(crate) gas_budget: u64,
}

pub(crate) fn prepare_transfer(
    from: &str,
    to: &str,
    amount: u64,
    gas_budget: u64,
    gas_price: u64,
    coins: &[GasCoin],
) -> Result<PreparedSuiTransfer, SendError> {
    let sender = bcs::address(from)?;
    let to = bcs::address(to)?;
    if amount == 0 || gas_budget == 0 || gas_price == 0 || coins.is_empty() || coins.len() > 256 {
        return Err(SendError::Invalid(
            "invalid Sui amount or gas payment".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let total = coins.iter().try_fold(0u64, |sum, c| {
        if !seen.insert(c.id) {
            return Err(SendError::invalid("duplicate Sui gas object"));
        }
        sum.checked_add(c.balance)
            .ok_or_else(|| SendError::Invalid("Sui gas balance overflow".into()))
    })?;
    if total
        < amount
            .checked_add(gas_budget)
            .ok_or_else(|| SendError::Invalid("Sui amount plus gas overflow".into()))?
    {
        return Err(SendError::InsufficientFunds(
            "insufficient SUI for amount plus gas budget".into(),
        ));
    }
    let mut bytes = vec![0, 0]; // TransactionData::V1, TransactionKind::ProgrammableTransaction
    bcs::uleb(2, &mut bytes); // inputs: Pure(amount), Pure(recipient)
    bytes.push(0);
    bcs::bytes(&amount.to_le_bytes(), &mut bytes);
    bytes.push(0);
    bcs::bytes(&to, &mut bytes);
    bcs::uleb(2, &mut bytes); // commands
    bytes.extend_from_slice(&[2, 0, 1, 1, 0, 0]); // SplitCoins(GasCoin, [Input(0)])
    bytes.extend_from_slice(&[1, 1, 3, 0, 0, 0, 0, 1, 1, 0]); // TransferObjects([NestedResult(0,0)], Input(1))
    bytes.extend_from_slice(&sender);
    bcs::uleb(coins.len(), &mut bytes);
    for coin in coins {
        bytes.extend_from_slice(&coin.id);
        bytes.extend_from_slice(&coin.version.to_le_bytes());
        bcs::bytes(&coin.digest, &mut bytes);
    }
    bytes.extend_from_slice(&sender); // gas owner
    bytes.extend_from_slice(&gas_price.to_le_bytes());
    bytes.extend_from_slice(&gas_budget.to_le_bytes());
    bytes.push(0); // TransactionExpiration::None
    Ok(PreparedSuiTransfer {
        sender,
        bytes,
        objects: coins.to_vec(),
        gas_budget,
    })
}
impl PreparedSuiTransfer {
    pub(crate) fn transaction_digest(&self) -> String {
        let hash = blake2b_simd::Params::new()
            .hash_length(32)
            .to_state()
            .update(b"TransactionData::")
            .update(&self.bytes)
            .finalize();
        bs58::encode(hash.as_bytes()).into_string()
    }

    pub(crate) fn sign(self, key: &Ed25519Seed) -> Result<(String, String), SendError> {
        let public = key.public_key();
        let expected = blake2b_simd::Params::new()
            .hash_length(32)
            .to_state()
            .update(&[0])
            .update(&public)
            .finalize();
        if self.sender != expected.as_bytes() {
            return Err(SendError::Invalid(
                "Sui sender does not match signing seed".into(),
            ));
        }
        let digest = blake2b_simd::Params::new()
            .hash_length(32)
            .to_state()
            .update(&[0, 0, 0])
            .update(&self.bytes)
            .finalize();
        let mut signature = vec![0];
        signature.extend_from_slice(&key.sign(digest.as_bytes()));
        signature.extend_from_slice(&public);
        Ok((STANDARD.encode(self.bytes), STANDARD.encode(signature)))
    }
}

pub(crate) async fn prepare_native_transfer(
    client: &SuiClient,
    from: &str,
    to: &str,
    mist: u64,
    gas_budget: u64,
) -> Result<PreparedSuiTransfer, SendError> {
    bcs::address(from)?;
    bcs::address(to)?;
    if mist == 0 || gas_budget == 0 {
        return Err(SendError::Invalid(
            "Sui amount and gas budget must be positive".into(),
        ));
    }
    let required = mist
        .checked_add(gas_budget)
        .ok_or_else(|| SendError::Invalid("Sui amount plus gas overflow".into()))?;
    let gas_price = client.fetch_reference_gas_price().await?;
    let mut coins = Vec::new();
    let mut cursor: Option<String> = None;
    let mut total = 0u64;
    let mut cursors = std::collections::HashSet::new();
    loop {
        let page = client.fetch_sui_coins_page(from, cursor.as_deref()).await?;
        for coin in page.coins {
            total = total
                .checked_add(coin.balance)
                .ok_or_else(|| SendError::Invalid("Sui coin balance overflow".into()))?;
            coins.push(GasCoin {
                id: bcs::address(&coin.object_id)?,
                version: coin.version,
                digest: coin.digest,
                balance: coin.balance,
            });
            if total >= required || coins.len() == 256 {
                break;
            }
        }
        if total >= required || coins.len() == 256 {
            break;
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        if !cursors.insert(next.clone()) {
            return Err(SendError::Invalid("repeated Sui coin cursor".into()));
        }
        cursor = Some(next);
    }
    prepare_transfer(from, to, mist, gas_budget, gas_price, &coins)
}

pub(crate) async fn prepare_token_transfer(
    client: &SuiClient,
    from: &str,
    to: &str,
    amount: u64,
    gas_budget: u64,
    coin_type: &str,
) -> Result<PreparedSuiTransfer, SendError> {
    let gas_price = client.fetch_reference_gas_price().await?;
    let gas = select_coins(client, from, "0x2::sui::SUI", gas_budget).await?;
    let tokens = select_coins(client, from, coin_type, amount).await?;
    prepare_coin_transfer(from, to, amount, gas_budget, gas_price, &gas, &tokens)
}

/// The first `limit` of `owner`'s objects of `coin_type` the node lists,
/// largest first.
pub(crate) async fn owned_coins(
    client: &SuiClient,
    owner: &str,
    coin_type: &str,
    limit: usize,
) -> Result<Vec<GasCoin>, SendError> {
    let mut coins = Vec::new();
    let mut cursor = None;
    let mut cursors = std::collections::HashSet::new();
    loop {
        let page = client
            .fetch_coins_page(owner, coin_type, cursor.as_deref())
            .await?;
        for coin in page.coins {
            coins.push(GasCoin {
                id: bcs::address(&coin.object_id)?,
                version: coin.version,
                digest: coin.digest,
                balance: coin.balance,
            });
        }
        if coins.len() >= limit {
            break;
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        if !cursors.insert(next.clone()) {
            return Err(SendError::invalid("repeated Sui coin cursor"));
        }
        cursor = Some(next);
    }
    coins.truncate(limit);
    coins.sort_by_key(|coin| std::cmp::Reverse(coin.balance));
    Ok(coins)
}

pub(crate) async fn select_coins(
    client: &SuiClient,
    owner: &str,
    coin_type: &str,
    required: u64,
) -> Result<Vec<GasCoin>, SendError> {
    let mut coins = Vec::new();
    let mut cursor = None;
    let mut total = 0u64;
    let mut cursors = std::collections::HashSet::new();
    loop {
        let page = client
            .fetch_coins_page(owner, coin_type, cursor.as_deref())
            .await?;
        for coin in page.coins {
            total = total
                .checked_add(coin.balance)
                .ok_or_else(|| SendError::invalid("Sui coin balance overflow"))?;
            coins.push(GasCoin {
                id: bcs::address(&coin.object_id)?,
                version: coin.version,
                digest: coin.digest,
                balance: coin.balance,
            });
            if total >= required || coins.len() == 256 {
                break;
            }
        }
        if total >= required {
            return Ok(coins);
        }
        if coins.len() == 256 {
            break;
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        if !cursors.insert(next.clone()) {
            return Err(SendError::invalid("repeated Sui coin cursor"));
        }
        cursor = Some(next);
    }
    Err(SendError::InsufficientFunds(
        format!("Insufficient {coin_type} coin objects").into(),
    ))
}

/// Merge selected token objects, split the exact amount, and transfer it;
/// native gas objects stay separate from the transferred Coin<T> inputs.
pub(crate) fn prepare_coin_transfer(
    from: &str,
    to: &str,
    amount: u64,
    gas_budget: u64,
    gas_price: u64,
    gas: &[GasCoin],
    tokens: &[GasCoin],
) -> Result<PreparedSuiTransfer, SendError> {
    let sender = bcs::address(from)?;
    let recipient = bcs::address(to)?;
    if amount == 0
        || gas_budget == 0
        || gas_price == 0
        || gas.is_empty()
        || tokens.is_empty()
        || gas.len() + tokens.len() > 256
    {
        return Err(SendError::invalid(
            "Invalid Sui token transfer or gas objects",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut total = |coins: &[GasCoin]| {
        coins.iter().try_fold(0u64, |sum, coin| {
            if !seen.insert(coin.id) {
                return Err(SendError::invalid("duplicate Sui object"));
            }
            sum.checked_add(coin.balance)
                .ok_or_else(|| SendError::invalid("Sui balance overflow"))
        })
    };
    if total(gas)? < gas_budget || total(tokens)? < amount {
        return Err(SendError::InsufficientFunds(
            "Insufficient Sui token or gas balance".into(),
        ));
    }
    let mut bytes = vec![0, 0];
    bcs::uleb(2 + tokens.len(), &mut bytes);
    bytes.push(0);
    bcs::bytes(&amount.to_le_bytes(), &mut bytes);
    bytes.push(0);
    bcs::bytes(&recipient, &mut bytes);
    for coin in tokens {
        bytes.extend([1, 0]); // CallArg::Object(ObjectArg::ImmOrOwnedObject)
        object_ref(coin, &mut bytes);
    }
    let merged = tokens.len() > 1;
    bcs::uleb(if merged { 3 } else { 2 }, &mut bytes);
    if merged {
        bytes.push(3);
        input(2, &mut bytes); // MergeCoins
        bcs::uleb(tokens.len() - 1, &mut bytes);
        for i in 1..tokens.len() {
            input((i + 2) as u16, &mut bytes);
        }
    }
    bytes.push(2);
    input(2, &mut bytes); // SplitCoins
    bcs::uleb(1, &mut bytes);
    input(0, &mut bytes);
    bytes.extend([1, 1, 3]); // TransferObjects([NestedResult(split,0)], Input(1))
    bytes.extend_from_slice(&u16::from(merged).to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    input(1, &mut bytes);
    bytes.extend_from_slice(&sender);
    bcs::uleb(gas.len(), &mut bytes);
    for coin in gas {
        object_ref(coin, &mut bytes);
    }
    bytes.extend_from_slice(&sender);
    bytes.extend_from_slice(&gas_price.to_le_bytes());
    bytes.extend_from_slice(&gas_budget.to_le_bytes());
    bytes.push(0);
    Ok(PreparedSuiTransfer {
        sender,
        bytes,
        objects: gas.iter().chain(tokens).cloned().collect(),
        gas_budget,
    })
}

/// A coin type's objects merged into one, and what the transaction was built
/// from, so it can be rebuilt and compared before signing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedSuiMerge {
    pub coin_type: String,
    pub gas: Vec<GasCoin>,
    /// Empty when the merged type is SUI, whose coins all pay gas.
    pub merged: Vec<GasCoin>,
    pub gas_price: u64,
    pub transaction: PreparedSuiTransfer,
}

impl PreparedSuiMerge {
    pub(crate) fn prepare(
        from: &str,
        coin_type: &str,
        gas_budget: u64,
        gas_price: u64,
        gas: Vec<GasCoin>,
        merged: Vec<GasCoin>,
    ) -> Result<Self, SendError> {
        let transaction = prepare_coin_merge(from, gas_budget, gas_price, &gas, &merged)?;
        Ok(Self {
            coin_type: coin_type.into(),
            gas,
            merged,
            gas_price,
            transaction,
        })
    }

    /// How many objects the transaction merges into one.
    pub(crate) fn object_count(&self) -> usize {
        if self.merged.is_empty() {
            self.gas.len()
        } else {
            self.merged.len()
        }
    }

    /// Whether the bytes are exactly this merge, from `from`.
    pub(crate) fn is_exact(&self, from: &str) -> bool {
        prepare_coin_merge(
            from,
            self.transaction.gas_budget,
            self.gas_price,
            &self.gas,
            &self.merged,
        )
        .is_ok_and(|rebuilt| rebuilt.bytes == self.transaction.bytes)
    }
}

/// The most SUI coins one transaction merges: Sui pays gas with at most 256
/// objects, and a SUI merge pays with all of them.
pub(crate) const MAX_SUI_MERGE: usize = 256;
/// The most objects of another type one transaction merges, kept well under
/// a command's argument limit.
pub(crate) const MAX_TOKEN_MERGE: usize = 500;

/// Merge a coin type's objects into one. With `merged` empty, SUI itself:
/// gas is paid with every coin in `gas`, which Sui smashes into the first,
/// and the transaction keeps that gas coin (`TransferObjects([GasCoin],
/// sender)`). Otherwise `merged` are another type's objects, merged into the
/// first (`MergeCoins`), and `gas` stays separate.
pub(crate) fn prepare_coin_merge(
    from: &str,
    gas_budget: u64,
    gas_price: u64,
    gas: &[GasCoin],
    merged: &[GasCoin],
) -> Result<PreparedSuiTransfer, SendError> {
    let sender = bcs::address(from)?;
    let native = merged.is_empty();
    let merging = if native { gas } else { merged };
    if gas_budget == 0
        || gas_price == 0
        || gas.is_empty()
        || merging.len() < 2
        || gas.len() > MAX_SUI_MERGE
        || merged.len() > MAX_TOKEN_MERGE
    {
        return Err(SendError::invalid("Invalid Sui coin merge or gas objects"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut gas_total = 0u64;
    for coin in gas.iter().chain(merged) {
        if !seen.insert(coin.id) {
            return Err(SendError::invalid("duplicate Sui object"));
        }
    }
    for coin in gas {
        gas_total = gas_total
            .checked_add(coin.balance)
            .ok_or_else(|| SendError::invalid("Sui balance overflow"))?;
    }
    if gas_total < gas_budget {
        return Err(SendError::InsufficientFunds(
            "Insufficient SUI for the gas budget".into(),
        ));
    }
    let mut bytes = vec![0, 0];
    if native {
        bcs::uleb(1, &mut bytes); // inputs: Pure(sender)
        bytes.push(0);
        bcs::bytes(&sender, &mut bytes);
        bcs::uleb(1, &mut bytes);
        bytes.extend([1, 1, 0]); // TransferObjects([GasCoin], Input(0))
        input(0, &mut bytes);
    } else {
        bcs::uleb(merged.len(), &mut bytes);
        for coin in merged {
            bytes.extend([1, 0]); // CallArg::Object(ObjectArg::ImmOrOwnedObject)
            object_ref(coin, &mut bytes);
        }
        bcs::uleb(1, &mut bytes);
        bytes.push(3);
        input(0, &mut bytes); // MergeCoins(Input(0), [Input(1)..])
        bcs::uleb(merged.len() - 1, &mut bytes);
        for index in 1..merged.len() {
            input(index as u16, &mut bytes);
        }
    }
    bytes.extend_from_slice(&sender);
    bcs::uleb(gas.len(), &mut bytes);
    for coin in gas {
        object_ref(coin, &mut bytes);
    }
    bytes.extend_from_slice(&sender);
    bytes.extend_from_slice(&gas_price.to_le_bytes());
    bytes.extend_from_slice(&gas_budget.to_le_bytes());
    bytes.push(0);
    Ok(PreparedSuiTransfer {
        sender,
        bytes,
        objects: gas.iter().chain(merged).cloned().collect(),
        gas_budget,
    })
}

fn input(index: u16, bytes: &mut Vec<u8>) {
    bytes.push(1);
    bytes.extend_from_slice(&index.to_le_bytes());
}

pub(crate) async fn prepare_staking(
    client: &SuiClient,
    owner: &str,
    validator: Option<&str>,
    amount: u64,
    staked: Option<(&str, &crate::api::sui_json_rpc::SuiStakedObject)>,
    gas_budget: u64,
) -> Result<PreparedSuiTransfer, SendError> {
    let required = amount
        .checked_add(gas_budget)
        .ok_or_else(|| SendError::invalid("Sui stake plus gas overflow"))?;
    let gas = select_coins(client, owner, "0x2::sui::SUI", required).await?;
    prepare_staking_data(
        owner,
        validator,
        amount,
        staked,
        gas_budget,
        client.fetch_reference_gas_price().await?,
        &gas,
    )
}

pub(crate) fn prepare_staking_data(
    owner: &str,
    validator: Option<&str>,
    amount: u64,
    staked: Option<(&str, &crate::api::sui_json_rpc::SuiStakedObject)>,
    gas_budget: u64,
    gas_price: u64,
    gas: &[GasCoin],
) -> Result<PreparedSuiTransfer, SendError> {
    let sender = bcs::address(owner)?;
    let (package, system, version) = crate::registry::Chain::Sui
        .sui_staking_system()
        .map_err(SendError::invalid)?;
    if gas.is_empty()
        || gas.len() > 255
        || gas_budget == 0
        || gas_price == 0
        || validator.is_some() == staked.is_some()
        || (validator.is_some() && amount == 0)
    {
        return Err(SendError::invalid(
            "Invalid Sui staking operation or gas objects",
        ));
    }
    let mut ids = std::collections::HashSet::new();
    let total = gas.iter().try_fold(0u64, |sum, coin| {
        if !ids.insert(coin.id) {
            return Err(SendError::invalid("Duplicate Sui gas object"));
        }
        sum.checked_add(coin.balance)
            .ok_or_else(|| SendError::invalid("Sui gas balance overflow"))
    })?;
    if total
        < amount
            .checked_add(gas_budget)
            .ok_or_else(|| SendError::invalid("Sui stake plus gas overflow"))?
    {
        return Err(SendError::InsufficientFunds(
            "Insufficient SUI for stake and gas".into(),
        ));
    }
    let mut bytes = vec![0, 0];
    bcs::uleb(if validator.is_some() { 3 } else { 2 }, &mut bytes);
    bytes.extend([1, 1]); // CallArg::Object(SharedObject)
    bytes.extend(bcs::address(system)?);
    bytes.extend(version.to_le_bytes());
    bytes.push(1);
    let mut objects = gas.to_vec();
    let (function, arguments) = if let Some(validator) = validator {
        bytes.push(0);
        bcs::bytes(&amount.to_le_bytes(), &mut bytes);
        bytes.push(0);
        bcs::bytes(&bcs::address(validator)?, &mut bytes);
        bytes.push(2); // two commands
        bytes.extend([2, 0, 1]);
        input(1, &mut bytes); // SplitCoins(GasCoin,[Input(1)])
        (
            "request_add_stake",
            vec![vec![1, 0, 0], vec![3, 0, 0, 0, 0], vec![1, 2, 0]],
        )
    } else {
        let (id, object) = staked.ok_or_else(|| SendError::invalid("Missing StakedSui object"))?;
        let coin = GasCoin {
            id: bcs::address(id)?,
            version: object.version,
            digest: object.digest,
            balance: object.principal,
        };
        if !ids.insert(coin.id) {
            return Err(SendError::invalid("Stake object is also a gas object"));
        }
        bytes.extend([1, 0]);
        object_ref(&coin, &mut bytes);
        objects.push(coin);
        bytes.push(1); // one command
        ("request_withdraw_stake", vec![vec![1, 0, 0], vec![1, 1, 0]])
    };
    bytes.push(0); // Command::MoveCall
    bytes.extend(bcs::address(package)?);
    bcs::bytes(b"sui_system", &mut bytes);
    bcs::bytes(function.as_bytes(), &mut bytes);
    bytes.push(0);
    bcs::uleb(arguments.len(), &mut bytes);
    for argument in arguments {
        bytes.extend(argument);
    }
    bytes.extend(sender);
    bcs::uleb(gas.len(), &mut bytes);
    for coin in gas {
        object_ref(coin, &mut bytes);
    }
    bytes.extend(sender);
    bytes.extend(gas_price.to_le_bytes());
    bytes.extend(gas_budget.to_le_bytes());
    bytes.push(0);
    Ok(PreparedSuiTransfer {
        sender,
        bytes,
        objects,
        gas_budget,
    })
}
fn object_ref(coin: &GasCoin, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&coin.id);
    bytes.extend_from_slice(&coin.version.to_le_bytes());
    bcs::bytes(&coin.digest, bytes);
}

#[cfg(test)]
mod token_tests {
    use super::*;
    /// Both merges exactly as the Sui SDK builds and signs them.
    #[test]
    fn coin_merges_match_the_official_sdk() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/sui-merge.json")).unwrap();
        let key = Ed25519Seed::from_hex(fixture["seed"].as_str().unwrap()).unwrap();
        let sender = fixture["sender"].as_str().unwrap();
        let coin = |id, version| GasCoin {
            id: [id; 32],
            version,
            digest: [0; 32],
            balance: 4_000_000,
        };
        for vector in fixture["vectors"].as_array().unwrap() {
            let prepared = match vector["name"].as_str().unwrap() {
                "token" => prepare_coin_merge(
                    sender,
                    5_000_000,
                    1000,
                    &[GasCoin {
                        balance: 9_000_000,
                        ..coin(0x33, 7)
                    }],
                    &[coin(0x44, 8), coin(0x55, 9), coin(0x66, 10)],
                ),
                _ => prepare_coin_merge(
                    sender,
                    5_000_000,
                    1000,
                    &[coin(0x33, 7), coin(0x44, 8), coin(0x55, 9)],
                    &[],
                ),
            }
            .unwrap();
            assert_eq!(
                hex::encode(&prepared.bytes),
                vector["raw"].as_str().unwrap()
            );
            assert_eq!(
                prepared.transaction_digest(),
                vector["transaction_digest"].as_str().unwrap()
            );
            assert_eq!(
                prepared.sign(&key).unwrap().1,
                vector["signature"].as_str().unwrap()
            );
        }
        // One object is not a merge; gas must cover the budget; an object
        // cannot be both gas and merged.
        assert!(prepare_coin_merge(sender, 5_000_000, 1000, &[coin(0x33, 7)], &[]).is_err());
        assert!(
            prepare_coin_merge(
                sender,
                5_000_000,
                1000,
                &[coin(0x33, 7)],
                &[coin(0x44, 8), coin(0x55, 9)]
            )
            .is_err()
        );
        assert!(
            prepare_coin_merge(
                sender,
                1_000_000,
                1000,
                &[coin(0x33, 7)],
                &[coin(0x33, 7), coin(0x55, 9)]
            )
            .is_err()
        );
    }

    #[test]
    fn coin_transfers_match_official_sdk_for_single_and_merged_inputs() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/token-send-vectors.json"))
                .unwrap();
        let key = Ed25519Seed::from_hex(&hex::encode([1u8; 32])).unwrap();
        let coin = |id, version, balance| GasCoin {
            id: [id; 32],
            version,
            digest: [0; 32],
            balance,
        };
        let gas = [coin(0x33, 7, 10000000)];
        for vector in fixtures["sui"].as_array().unwrap() {
            let tokens = if vector["count"] == 1 {
                vec![coin(0x44, 8, 123456789)]
            } else {
                vec![coin(0x44, 8, 60000000), coin(0x55, 9, 70000000)]
            };
            let prepared = prepare_coin_transfer(
                vector["sender"].as_str().unwrap(),
                &format!("0x{}", "22".repeat(32)),
                123456789,
                10000000,
                1000,
                &gas,
                &tokens,
            )
            .unwrap();
            assert_eq!(
                hex::encode(&prepared.bytes),
                vector["raw"].as_str().unwrap()
            );
            assert_eq!(prepared.objects.len(), 1 + tokens.len());
            assert_eq!(
                prepared.transaction_digest(),
                vector["transaction_digest"].as_str().unwrap()
            );
            assert_eq!(
                prepared.sign(&key).unwrap().1,
                vector["signature"].as_str().unwrap()
            );
        }
        let sender = fixtures["sui"][0]["sender"].as_str().unwrap();
        assert!(prepare_coin_transfer(sender, "0x22", 1, 10000000, 1000, &gas, &gas).is_err());
        assert!(
            prepare_coin_transfer(
                sender,
                "0x22",
                200000000,
                10000000,
                1000,
                &gas,
                &[coin(0x44, 8, 1)]
            )
            .is_err()
        );
    }
}
