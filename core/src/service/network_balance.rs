//! Network balance: service adapters and dispatch.
use super::*;

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
impl WalletService {
    pub(crate) async fn fetch_native_balance_summary_auto(
        &self,
        chain: crate::registry::Chain,
        address: String,
    ) -> Result<NativeBalanceSummary, SpectraBridgeError> {
        // The Bitcoin family, not the literal id: a wallet on Testnet4 arrives
        // as `bitcoin-testnet-4`, and comparing the string meant its xpub was
        // walked as a plain address instead.
        let is_bitcoin_family = chain.mainnet_counterpart() == crate::registry::Chain::Bitcoin;
        if is_bitcoin_family && is_extended_public_key(&address) {
            let bal = self.bitcoin_xpub_balance(chain, address, 20, 20).await?;
            return Ok(NativeBalanceSummary {
                smallest_unit: bal.confirmed_sats.to_string(),
                amount_display: crate::decimal::from_units(bal.confirmed_sats as u128, 8),
            });
        }
        fetch_native_balance_summary(&address, chain, self).await
    }
}
async fn fetch_native_balance_summary(
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
            let width = chain.substrate_balance_bytes().ok_or_else(|| {
                SpectraBridgeError::failure("no Substrate balance layout for this chain")
            })?;
            SubstrateClient::new(endpoints)
                .fetch_balance(&account, width)
                .await?
                .transferable()
                .to_string()
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
