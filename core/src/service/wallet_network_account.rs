//! What a wallet's account on its network holds and needs, where the network
//! keeps more than a balance: Tron's bandwidth and energy, the reserve XRP and
//! Stellar lock, a TON wallet's contract and state, what a Substrate balance
//! is made of, and the storage a NEAR account pays for. Read from one verified
//! node each; the page that shows it is the wallet's Account.

use super::*;

/// A TON account's lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum TonAccountState {
    /// The wallet contract runs; the account sends.
    Active,
    /// No contract yet: the first send deploys it with the message.
    Uninitialized,
    /// Frozen for unpaid storage until it is paid.
    Frozen,
}

/// A wallet's account on its network. Amounts are exact decimals of the
/// native coin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Enum)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum NetworkAccount {
    /// Tron's resources and what a transfer burns without them.
    Tron {
        /// Whether the account exists on the chain yet.
        activated: bool,
        bandwidth_available: u64,
        bandwidth_limit: u64,
        energy_available: u64,
        energy_limit: u64,
        /// TRX burned per bandwidth point a transaction lacks.
        bandwidth_price: String,
        /// TRX burned per energy unit a contract call lacks.
        energy_price: String,
        /// What a TRX transfer burns with no bandwidth left: its bytes at the
        /// bandwidth price.
        transfer_burn: String,
        /// What a TRC-20 transfer usually burns with no energy, and the most
        /// its signed fee limit lets it burn.
        token_transfer_burn: String,
        token_transfer_ceiling: String,
    },
    /// The reserve XRP and Stellar lock in an account.
    Reserve {
        exists: bool,
        balance: String,
        /// The reserve every account holds.
        base_reserve: String,
        /// What the account's owned objects (XRP) or subentries and
        /// sponsorships (Stellar) add.
        owned_objects: u64,
        object_reserve: String,
        /// Base plus objects: what the account cannot spend.
        total_reserve: String,
        spendable: String,
    },
    /// A TON wallet's contract and state.
    Ton {
        state: TonAccountState,
        /// The contract a node recognizes once deployed, such as `W5` or
        /// `v4R2`; `None` before.
        contract: Option<String>,
    },
    /// What a Substrate balance is made of.
    Substrate {
        free: String,
        reserved: String,
        frozen: String,
        /// The least an account may hold; below it the account is reaped.
        existential_deposit: String,
        /// What a transfer that keeps the account alive can move.
        transferable: String,
    },
    /// The storage a NEAR account's balance must cover.
    Near {
        storage_bytes: u64,
        /// NEAR per byte stored.
        storage_price: String,
        /// Stake locked in the account, which counts toward its storage.
        locked: String,
        /// What the liquid balance keeps for storage beyond that.
        storage_reserve: String,
    },
}

/// A wallet's network account and the coin its amounts are in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletNetworkAccount {
    pub chain: Chain,
    pub symbol: String,
    pub account: NetworkAccount,
    /// Whether the wallet can close this account into another to recover its
    /// reserve: an XRP or Stellar account that exists, with keys to sign.
    pub closable: bool,
}

/// Whether core reads a network account for `chain`.
pub(crate) fn has_network_account(chain: Chain) -> bool {
    matches!(
        chain.mainnet_counterpart(),
        Chain::Tron
            | Chain::Xrp
            | Chain::Stellar
            | Chain::Ton
            | Chain::Polkadot
            | Chain::Bittensor
            | Chain::Near
    )
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The wallet's account on its network, read from a verified node.
    /// Refuses a network whose account is only a balance.
    pub async fn wallet_network_account(
        &self,
        wallet_id: String,
    ) -> Result<WalletNetworkAccount, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let wallet = this.stored_wallet(&wallet_id).await?;
            let chain = wallet.chain_id;
            if !has_network_account(chain) {
                return Err(SpectraBridgeError::refused(
                    "A %@ account holds only its balance.",
                    [chain.chain_display_name()],
                ));
            }
            let address = wallet
                .address_on(chain)
                .ok_or_else(|| {
                    SpectraBridgeError::failure("Wallet has no address on this network")
                })?
                .to_string();
            let decimals = u32::from(chain.native_decimals());
            let coin = |units: u128| crate::decimal::from_units(units, decimals);
            let endpoints = this
                .endpoints_for(chain, &[EndpointCapability::Verification])
                .await;
            let account = match chain.mainnet_counterpart() {
                Chain::Tron => {
                    let resources = TronHttpClient::new(endpoints)
                        .fetch_account_resources(chain, &address)
                        .await?;
                    let bandwidth = |transfer| -> Result<u64, SpectraBridgeError> {
                        // Any block measures the same: a block id leads
                        // with its number, as Tron requires.
                        let mut id = [0; 32];
                        id[..8].copy_from_slice(&1u64.to_be_bytes());
                        let reference = crate::api::tron_http::BlockReference {
                            number: 1,
                            id,
                            timestamp_ms: 1,
                        };
                        Ok(
                            crate::send::tron::prepare_transfer(&address, transfer, reference)?
                                .bandwidth_bytes()?,
                        )
                    };
                    let transfer_bytes = bandwidth(crate::send::tron::Transfer::Native {
                        to: &address,
                        amount: 1,
                    })?;
                    let free = resources
                        .free_bandwidth_limit
                        .saturating_sub(resources.free_bandwidth_used);
                    let staked = resources
                        .staked_bandwidth_limit
                        .saturating_sub(resources.staked_bandwidth_used);
                    NetworkAccount::Tron {
                        activated: resources.activated,
                        bandwidth_available: free + staked,
                        bandwidth_limit: resources.free_bandwidth_limit
                            + resources.staked_bandwidth_limit,
                        energy_available: resources
                            .energy_limit
                            .saturating_sub(resources.energy_used),
                        energy_limit: resources.energy_limit,
                        bandwidth_price: coin(u128::from(resources.bandwidth_price_sun)),
                        energy_price: coin(u128::from(resources.energy_price_sun)),
                        transfer_burn: coin(
                            u128::from(transfer_bytes) * u128::from(resources.bandwidth_price_sun),
                        ),
                        token_transfer_burn: coin(u128::from(
                            crate::send::tron::TRC20_TYPICAL_FEE_SUN,
                        )),
                        token_transfer_ceiling: coin(u128::from(
                            crate::send::tron::TRC20_FEE_LIMIT_SUN,
                        )),
                    }
                }
                Chain::Xrp => {
                    let state = XrplClient::new(endpoints)
                        .fetch_reserve_state(chain, &address)
                        .await?;
                    reserve_account(
                        state.exists,
                        state.balance_drops,
                        u128::from(state.reserve_base),
                        state.owner_count,
                        state.reserve_increment,
                        decimals,
                    )
                }
                Chain::Stellar => {
                    let state = HorizonClient::new(endpoints)
                        .fetch_reserve_state(chain, &address)
                        .await?;
                    // CAP-33: an account holds two base reserves, one more per
                    // subentry and per sponsoring, one fewer per sponsored.
                    reserve_account(
                        state.exists,
                        state.balance_stroops,
                        2 * u128::from(state.base_reserve),
                        (state.subentries + state.sponsoring).saturating_sub(state.sponsored),
                        state.base_reserve,
                        decimals,
                    )
                }
                Chain::Ton => {
                    let (state, wallet_type) = ToncenterV2Client::new(endpoints)
                        .fetch_wallet_state(chain, &address)
                        .await?;
                    NetworkAccount::Ton {
                        state: ton_account_state(&state).ok_or_else(|| {
                            SpectraBridgeError::failure(format!(
                                "Unknown TON account state: {state}"
                            ))
                        })?,
                        contract: wallet_type.map(ton_contract_name),
                    }
                }
                Chain::Polkadot | Chain::Bittensor => {
                    let account = crate::derivation::primitives::decode_ss58(&address, None)?.1;
                    let client = SubstrateClient::new(endpoints);
                    let context = client.polkadot_context(chain).await?;
                    let balance = client
                        .fetch_balance_at(chain, &account, &context.block_hash)
                        .await?;
                    let deposit = context.runtime.existential_deposit;
                    NetworkAccount::Substrate {
                        free: coin(balance.free),
                        reserved: coin(balance.reserved),
                        frozen: coin(balance.frozen),
                        existential_deposit: coin(deposit),
                        transferable: coin(balance.keep_alive_spendable(deposit)),
                    }
                }
                Chain::Near => {
                    let state = NearClient::new(endpoints)
                        .fetch_storage_state(&address)
                        .await?;
                    NetworkAccount::Near {
                        storage_bytes: state.storage_usage,
                        storage_price: coin(state.cost_per_byte),
                        locked: coin(state.locked),
                        storage_reserve: coin(state.storage_reserve()?),
                    }
                }
                _ => unreachable!("has_network_account names these networks"),
            };
            Ok(WalletNetworkAccount {
                chain,
                symbol: chain.coin_symbol().to_string(),
                closable: super::wallet_closing::closes_accounts(chain)
                    && !wallet.is_watch_only()
                    && matches!(account, NetworkAccount::Reserve { exists: true, .. }),
                account,
            })
        })
        .await
    }
}

/// An XRP or Stellar account's reserve: the base every account holds, and
/// what each object the account owns adds. Amounts in the coin's smallest
/// unit.
fn reserve_account(
    exists: bool,
    balance: u64,
    base_reserve: u128,
    owned_objects: u64,
    per_object: u64,
    decimals: u32,
) -> NetworkAccount {
    let coin = |units: u128| crate::decimal::from_units(units, decimals);
    let object_reserve = u128::from(owned_objects) * u128::from(per_object);
    let total = base_reserve + object_reserve;
    NetworkAccount::Reserve {
        exists,
        balance: coin(u128::from(balance)),
        base_reserve: coin(base_reserve),
        owned_objects,
        object_reserve: coin(object_reserve),
        total_reserve: coin(total),
        spendable: coin(u128::from(balance).saturating_sub(total)),
    }
}

/// A toncenter `getWalletInformation` account state.
fn ton_account_state(state: &str) -> Option<TonAccountState> {
    match state {
        "active" => Some(TonAccountState::Active),
        "uninitialized" | "uninit" | "nonexist" => Some(TonAccountState::Uninitialized),
        "frozen" => Some(TonAccountState::Frozen),
        _ => None,
    }
}

/// A toncenter wallet type in the names wallets show.
fn ton_contract_name(kind: String) -> String {
    match kind.as_str() {
        "wallet v5 r1" => "W5".to_string(),
        "wallet v4 r2" => "v4R2".to_string(),
        _ => kind,
    }
}

#[cfg(test)]
#[path = "tests/wallet_network_account.rs"]
mod tests;
