//! Network balance: service adapters and dispatch.
use super::*;

impl WalletService {
    /// Recovery and receive reservations add addresses to the same wallet.
    /// Its native balance includes every owned account UTXO address, read
    /// from the chain's own indexer.
    pub(super) async fn account_utxo_wallet_balance(
        &self,
        wallet_id: &str,
        chain: Chain,
    ) -> Result<NativeBalanceSummary, SpectraBridgeError> {
        use futures::stream::{self, StreamExt, TryStreamExt};
        if !chain.uses_account_utxo() {
            return Err(SpectraBridgeError::invalid(
                "Unsupported account UTXO network",
            ));
        }
        let addresses = self.known_utxo_addresses(wallet_id.into(), chain).await?;
        let reader = AccountBalanceReader::new(self, chain).await;
        let total = stream::iter(addresses)
            .map(|address| {
                let reader = &reader;
                async move { reader.balance(&address).await }
            })
            .buffered(4)
            .try_fold(0u64, |sum, balance| async move {
                sum.checked_add(balance).ok_or_else(|| {
                    crate::api::error::ApiError::InvalidInput("UTXO wallet balance overflow".into())
                })
            })
            .await?;
        Ok(NativeBalanceSummary {
            smallest_unit: total.to_string(),
            amount_display: crate::decimal::from_units(
                u128::from(total),
                u32::from(chain.native_decimals()),
            ),
        })
    }
}

/// One address's confirmed native balance, from the indexer its account
/// UTXO chain reads.
enum AccountBalanceReader {
    Utxo(crate::api::utxo::UtxoClient),
    Insight(InsightClient),
    Kaspa(KaspaClient),
}

impl AccountBalanceReader {
    async fn new(service: &WalletService, chain: Chain) -> Self {
        let endpoints = || service.endpoints_for(chain, &[EndpointCapability::Balance]);
        match chain.mainnet_counterpart() {
            Chain::Decred => Self::Insight(InsightClient::new(endpoints().await)),
            Chain::Kaspa => Self::Kaspa(KaspaClient::new(endpoints().await)),
            _ => Self::Utxo(
                service
                    .utxo_client(chain, &[EndpointCapability::Balance])
                    .await,
            ),
        }
    }

    async fn balance(&self, address: &str) -> Result<u64, crate::api::error::ApiError> {
        Ok(match self {
            Self::Utxo(client) => client.fetch_balance(address).await?.confirmed_sats,
            Self::Insight(client) => client.fetch_balance(address).await?.balance_atoms,
            Self::Kaspa(client) => client.fetch_balance(address).await?.balance_sompi,
        })
    }
}

#[cfg(test)]
#[path = "tests/network_balance.rs"]
mod tests;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Unified per-chain native balance summary, replacing chain-specific JSON
    /// decoding on the Swift side. Smallest unit is returned as a decimal
    /// string (sats / wei / lamports / yocto-NEAR / ...) so callers can `UInt64`
    /// or `BigInt` parse as appropriate. `amount_display` is the human-readable
    /// native amount as decimal string.
    pub async fn fetch_native_balance_summary(
        &self,
        chain: crate::registry::Chain,
        address: String,
    ) -> Result<NativeBalanceSummary, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            fetch_native_balance_summary(&address, chain, this).await
        })
        .await
    }
}
pub(super) async fn fetch_native_balance_summary(
    address: &str,
    chain: Chain,
    service: &WalletService,
) -> Result<NativeBalanceSummary, SpectraBridgeError> {
    if chain.mainnet_counterpart() == Chain::Monero {
        let state = service.app_state().await;
        let owner = state
            .wallets
            .iter()
            .find(|w| w.chain_id == chain && w.address_on(chain) == Some(address))
            .ok_or_else(|| {
                SpectraBridgeError::failure("Monero balance requires an owned local wallet")
            })?;
        let status = service
            .monero_sync_status(owner.id.clone())
            .await?
            .ok_or_else(|| SpectraBridgeError::failure("Monero local wallet unavailable"))?;
        if !status.complete {
            return Err(SpectraBridgeError::failure(
                "Sync the local Monero wallet before refreshing its balance",
            ));
        }
        return Ok(NativeBalanceSummary {
            smallest_unit: status.unlocked_piconeros.to_string(),
            amount_display: crate::decimal::from_units(status.unlocked_piconeros as u128, 12),
        });
    }
    let units = if chain.uses_utxo_client() {
        service
            .utxo_client(chain, &[EndpointCapability::Balance])
            .await
            .fetch_balance(address)
            .await?
            .confirmed_sats
            .to_string()
    } else {
        single_api_balance(address, chain, service).await?
    };
    let amount = units
        .parse::<u128>()
        .map_err(|_| SpectraBridgeError::failure("native balance exceeds core precision"))?;
    Ok(NativeBalanceSummary {
        amount_display: crate::decimal::from_units(amount, u32::from(chain.native_decimals())),
        smallest_unit: units,
    })
}

async fn single_api_balance(
    address: &str,
    chain: Chain,
    service: &WalletService,
) -> Result<String, SpectraBridgeError> {
    let (api, endpoints) = service
        .fetch_endpoints(chain, &[EndpointCapability::Balance])
        .await?;
    use crate::EndpointApi as Api;
    Ok(match api {
        Api::EvmJsonRpc => {
            EvmClient::new(endpoints, chain.evm_chain_id()?)
                .fetch_balance(address)
                .await?
                .balance_wei
        }
        Api::SolanaJsonRpc => SolanaClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .lamports
            .to_string(),
        Api::TronHttp => TronHttpClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .sun
            .to_string(),
        Api::Horizon => HorizonClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .stroops
            .to_string(),
        Api::XrplJsonRpc => XrplClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .drops
            .to_string(),
        Api::Koios => KoiosClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .lovelace
            .to_string(),
        Api::SubstrateJsonRpc => {
            let account = if chain.mainnet_counterpart() == Chain::Bittensor {
                crate::derivation::bittensor::decode_bittensor_ss58(address)?
            } else {
                crate::derivation::polkadot::decode_ss58(address)?
            };
            let client = SubstrateClient::new(endpoints);
            let balance = client.fetch_balance(chain, &account).await?;
            balance.transferable().to_string()
        }
        Api::SuiJsonRpc => SuiClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .mist
            .to_string(),
        Api::AptosRest => AptosClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .octas
            .to_string(),
        Api::ToncenterV2 => ToncenterV2Client::new(endpoints)
            .fetch_balance(address)
            .await?
            .nanotons
            .to_string(),
        Api::NearJsonRpc => {
            NearClient::new(endpoints)
                .fetch_balance(address)
                .await?
                .yocto_near
        }
        Api::IcpRosetta => IcpClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .e8s
            .to_string(),
        Api::Insight => InsightClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .balance_atoms
            .to_string(),
        Api::KaspaRest => KaspaClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .balance_sompi
            .to_string(),
        api => {
            return Err(SpectraBridgeError::failure(format!(
                "{} has no native balance adapter",
                api.as_str()
            )));
        }
    })
}
