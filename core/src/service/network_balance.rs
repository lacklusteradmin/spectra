//! Network balance: service adapters and dispatch.
use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Unified per-chain native balance summary, replacing chain-specific JSON
    /// decoding on the Swift side. Smallest unit is returned as a decimal
    /// string (sats / wei / lamports / yocto-NEAR / ...) so callers can `UInt64`
    /// or `BigInt` parse as appropriate. `amount_display` is the human-readable
    /// native amount as decimal string. `utxo_count` is 0 for non-UTXO chains.
    pub async fn fetch_native_balance_summary(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<NativeBalanceSummary, SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        fetch_native_balance_summary(&address, chain, self).await
    }
}
impl WalletService {
    pub(crate) async fn fetch_native_balance_summary_auto(
        &self,
        chain_id: &str,
        address: String,
    ) -> Result<NativeBalanceSummary, SpectraBridgeError> {
        // The Bitcoin family, not the literal id: a wallet on Testnet4 arrives
        // as `bitcoin-testnet-4`, and comparing the string meant its xpub was
        // walked as a plain address instead.
        let is_bitcoin_family = crate::registry::Chain::from_str_id(chain_id)
            .is_some_and(|chain| chain.mainnet_counterpart() == crate::registry::Chain::Bitcoin);
        if is_bitcoin_family && is_extended_public_key(&address) {
            let bal = self.bitcoin_xpub_balance(chain_id, address, 20, 20).await?;
            return Ok(NativeBalanceSummary {
                smallest_unit: bal.confirmed_sats.to_string(),
                amount_display: format_smallest_unit_decimal(bal.confirmed_sats as u128, 8),
                utxo_count: bal.utxo_count as u32,
            });
        }
        let chain = chain_for_id(chain_id)?;
        fetch_native_balance_summary(&address, chain, self).await
    }
}
async fn fetch_native_balance_summary(
    address: &str,
    chain: Chain,
    service: &WalletService,
) -> Result<NativeBalanceSummary, SpectraBridgeError> {
    let (api, endpoints) = service.fetch_endpoints(chain).await?;
    use crate::EndpointApi as Api;
    let mut utxo_count = 0;
    let units = match api {
        Api::Esplora => {
            let balance = BitcoinClient::new(HttpClient::shared(), endpoints)
                .fetch_balance(address)
                .await?;
            utxo_count = balance.utxo_count as u32;
            balance.confirmed_sats.to_string()
        }
        Api::Blockbook => BlockbookClient::new(endpoints, chain)
            .fetch_balance(address)
            .await?
            .balance_sat
            .to_string(),
        Api::Whatsonchain => BitcoinSvClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .balance_sat
            .to_string(),
        Api::Blockcypher => DogecoinClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .balance_koin
            .to_string(),
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
        Api::TronHttp => TronClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .sun
            .to_string(),
        Api::Horizon => StellarClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .stroops
            .to_string(),
        Api::XrplJsonRpc => XrpClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .drops
            .to_string(),
        Api::Koios => CardanoClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .lovelace
            .to_string(),
        Api::SubstrateJsonRpc => {
            if chain.mainnet_counterpart() == Chain::Bittensor {
                BittensorClient::new(endpoints)
                    .fetch_balance(address)
                    .await?
                    .rao
                    .to_string()
            } else {
                PolkadotClient::new(endpoints)
                    .fetch_balance(address)
                    .await?
                    .planck
                    .to_string()
            }
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
        Api::ToncenterV2 => TonClient::new(endpoints)
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
        Api::MoneroWalletRpc => MoneroClient::new(endpoints)
            .fetch_balance(0)
            .await?
            .piconeros
            .to_string(),
        Api::Insight => DecredClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .balance_atoms
            .to_string(),
        Api::KaspaRest => KaspaClient::new(endpoints)
            .fetch_balance(address)
            .await?
            .balance_sompi
            .to_string(),
        api => return Err(format!("{} has no native balance adapter", api.as_str()).into()),
    };
    let amount = units
        .parse::<u128>()
        .map_err(|_| "native balance exceeds core precision")?;
    Ok(NativeBalanceSummary {
        amount_display: format_smallest_unit_decimal(amount, u32::from(chain.native_decimals())),
        smallest_unit: units,
        utxo_count,
    })
}
