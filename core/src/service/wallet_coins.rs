//! The addresses and coins behind an account-discovery UTXO wallet's balance.
//!
//! A Bitcoin-family wallet's balance is spread over the receive and change
//! addresses it has handed out; this is that spread, read from the network:
//! each owned address with its branch, whether it has been used and what it
//! holds, the next receive address, and every unspent output with its
//! confirmations. On Peercoin a minting reward still maturing is in the
//! total and in no send, and says so. Read-only but for one step a refresh
//! takes as well: a reserved receive address found used is moved past.

use super::*;

/// Which branch of the account an address is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AddressBranch {
    Receive,
    Change,
}

/// One address the wallet owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct OwnedAddressCoins {
    pub address: String,
    /// `None` for an address known only from a transaction.
    pub branch: Option<AddressBranch>,
    pub index: Option<u32>,
    /// Whether it has ever received.
    pub used: bool,
    /// What its unspent outputs hold, as an exact decimal.
    pub balance: String,
}

/// One unspent output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UnspentOutput {
    pub txid: String,
    pub vout: u32,
    pub address: String,
    pub amount: String,
    /// Blocks since it was mined, counting its own; 0 while unconfirmed.
    pub confirmations: u64,
    /// Whether a send can spend it: false for a Peercoin minting reward
    /// still maturing.
    pub spendable: bool,
}

/// The addresses and coins behind a wallet's balance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletCoins {
    pub chain: Chain,
    pub symbol: String,
    /// Receive addresses by index, then change, then any known only from a
    /// transaction.
    pub addresses: Vec<OwnedAddressCoins>,
    /// The receive address the wallet hands out next.
    pub next_receive_address: Option<String>,
    /// Largest first.
    pub outputs: Vec<UnspentOutput>,
    /// What the outputs no send can spend yet hold, as an exact decimal.
    pub maturing: String,
    pub tip_height: u64,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The addresses and coins behind the wallet's balance, read from the
    /// network. Refuses a wallet on a network with one address.
    pub async fn wallet_coins(&self, wallet_id: String) -> Result<WalletCoins, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let wallet = this.stored_wallet(&wallet_id).await?;
            let chain = wallet.chain_id;
            if !chain.supports_deep_utxo_discovery() {
                return Err(SpectraBridgeError::refused(
                    "%@ wallets hold one address; there is no account to list.",
                    [chain.chain_display_name()],
                ));
            }
            let mut places: HashMap<String, (AddressBranch, u32)> = this
                .keypool
                .read()
                .await
                .owned_on(chain)
                .iter()
                .filter(|row| row.wallet_id == wallet_id)
                .filter_map(|row| {
                    let branch = match row.branch.as_deref()? {
                        "external" => AddressBranch::Receive,
                        "change" => AddressBranch::Change,
                        _ => return None,
                    };
                    Some((
                        row.address.clone(),
                        (branch, u32::try_from(row.branch_index?).ok()?),
                    ))
                })
                .collect();
            // The wallet's own address is the one its path ends at.
            if let (Some(address), Some(place)) = (
                wallet.address_on(chain),
                wallet.derivation_path.as_deref().and_then(path_place),
            ) {
                places.entry(address.to_string()).or_insert(place);
            }
            let addresses = this.known_utxo_addresses(wallet_id.clone(), chain).await?;
            let client = this.utxo_client(chain, &[EndpointCapability::Utxo]).await;
            let tip_height = client.fetch_tip_height().await?;
            let decimals = u32::from(chain.native_decimals());
            let peercoin = chain.mainnet_counterpart() == Chain::Peercoin;
            let reads = addresses.iter().map(|address| {
                let client = &client;
                async move {
                    let outputs: Vec<(String, u32, u64, u64, bool)> = if peercoin {
                        client
                            .fetch_peercoin_outputs(address)
                            .await?
                            .into_iter()
                            .map(|output| {
                                let (txid, vout, value, _) = output.input;
                                (txid, vout, value, output.confirmations, output.mature)
                            })
                            .collect()
                    } else {
                        client
                            .fetch_utxos(address)
                            .await?
                            .into_iter()
                            .map(|utxo| {
                                let confirmations = utxo
                                    .status
                                    .block_height
                                    .filter(|_| utxo.status.confirmed)
                                    .filter(|height| *height <= tip_height)
                                    .map_or(0, |height| tip_height - height + 1);
                                (utxo.txid, utxo.vout, utxo.value, confirmations, true)
                            })
                            .collect()
                    };
                    let used = !outputs.is_empty() || client.has_activity(address).await?;
                    Ok::<_, crate::api::error::ApiError>((address.clone(), used, outputs))
                }
            });
            let read: Vec<_> = futures::future::try_join_all(reads).await?;
            let mut owned = Vec::new();
            let mut outputs = Vec::new();
            let mut maturing: u128 = 0;
            for (address, used, unspent) in read {
                let balance: u128 = unspent
                    .iter()
                    .map(|(_, _, value, _, _)| u128::from(*value))
                    .sum();
                let place = places.get(&address).copied();
                owned.push(OwnedAddressCoins {
                    address: address.clone(),
                    branch: place.map(|(branch, _)| branch),
                    index: place.map(|(_, index)| index),
                    used,
                    balance: crate::decimal::from_units(balance, decimals),
                });
                for (txid, vout, value, confirmations, spendable) in unspent {
                    if !spendable {
                        maturing += u128::from(value);
                    }
                    outputs.push((
                        value,
                        UnspentOutput {
                            txid,
                            vout,
                            address: address.clone(),
                            amount: crate::decimal::from_units(u128::from(value), decimals),
                            confirmations,
                            spendable,
                        },
                    ));
                }
            }
            // The receive address handed out next must be unused: past one
            // that has received since it was reserved, as a refresh would.
            let used: std::collections::HashSet<&str> = owned
                .iter()
                .filter(|entry| entry.used)
                .map(|entry| entry.address.as_str())
                .collect();
            let mut next_receive_address = this
                .receive_address(wallet_id.clone(), chain, false)
                .await?;
            for _ in 0..100 {
                if !next_receive_address
                    .as_deref()
                    .is_some_and(|address| used.contains(address))
                {
                    break;
                }
                let Some(reserved) = this
                    .keypool_state(wallet_id.clone(), chain)
                    .await?
                    .reserved_receive_index
                else {
                    break;
                };
                if this
                    .advance_receive_index_if_current(wallet_id.clone(), chain, reserved)
                    .await?
                    .is_none()
                {
                    break;
                }
                next_receive_address = this.receive_address(wallet_id.clone(), chain, true).await?;
            }
            owned.sort_by_key(|entry| {
                (
                    entry.branch.map_or(2, |branch| branch as u8),
                    entry.index.unwrap_or(u32::MAX),
                )
            });
            outputs.sort_by_key(|output| std::cmp::Reverse(output.0));
            Ok(WalletCoins {
                chain,
                symbol: chain.coin_symbol().to_string(),
                addresses: owned,
                next_receive_address,
                outputs: outputs.into_iter().map(|(_, output)| output).collect(),
                maturing: crate::decimal::from_units(maturing, decimals),
                tip_height,
            })
        })
        .await
    }
}

/// The branch and index a BIP-44-shaped path ends at: `…/0/7` is receive 7,
/// `…/1/3` change 3.
fn path_place(path: &str) -> Option<(AddressBranch, u32)> {
    let mut segments = path.rsplit('/');
    let index = segments.next()?.parse().ok()?;
    let branch = match segments.next()? {
        "0" => AddressBranch::Receive,
        "1" => AddressBranch::Change,
        _ => return None,
    };
    Some((branch, index))
}
