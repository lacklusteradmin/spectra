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
    let endpoints = service.endpoints_for(chain.str_id()).await;
    let dispatch = chain.mainnet_counterpart();
    match dispatch {
        Chain::Bitcoin => {
            let bal = BitcoinClient::new(HttpClient::shared(), endpoints)
                .fetch_balance(address)
                .await?;
            Ok(NativeBalanceSummary {
                smallest_unit: bal.confirmed_sats.to_string(),
                amount_display: format_smallest_unit_decimal(bal.confirmed_sats as u128, 8),
                utxo_count: bal.utxo_count as u32,
            })
        }
        Chain::BitcoinCash => {
            let bal = BitcoinCashClient::new(endpoints)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::BitcoinSV => {
            let bal = BitcoinSvClient::new(endpoints)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Litecoin => {
            let bal = LitecoinClient::new(endpoints)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Dogecoin => {
            let bal = DogecoinClient::new(endpoints)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(
                bal.balance_koin.to_string(),
                bal.balance_display,
            ))
        }
        c if c.is_evm() => {
            let bal = EvmClient::new(endpoints, chain.evm_chain_id())
                .fetch_balance(address)
                .await?;
            Ok(summary_native(bal.balance_wei, bal.balance_display))
        }
        Chain::Solana => {
            let bal = SolanaClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.lamports.to_string(), bal.sol_display))
        }
        Chain::Tron => {
            let bal = TronClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.sun.to_string(), bal.trx_display))
        }
        Chain::Stellar => {
            let bal = StellarClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.stroops.to_string(), bal.xlm_display))
        }
        Chain::Xrp => {
            let bal = XrpClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.drops.to_string(), bal.xrp_display))
        }
        Chain::Cardano => {
            let api_key = service
                .api_key_for(chain.str_id())
                .await
                .unwrap_or_default();
            let bal = CardanoClient::new(endpoints, api_key)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(bal.lovelace.to_string(), bal.ada_display))
        }
        Chain::Polkadot => {
            let subscan = service
                .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                .await;
            let api_key = service.api_key_for(chain.str_id()).await;
            let bal = PolkadotClient::new(endpoints, subscan, api_key)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(bal.planck.to_string(), bal.dot_display))
        }
        Chain::Sui => {
            let bal = SuiClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.mist.to_string(), bal.sui_display))
        }
        Chain::Aptos => {
            let bal = AptosClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.octas.to_string(), bal.apt_display))
        }
        Chain::Ton => {
            let api_key = service.api_key_for(chain.str_id()).await;
            let bal = TonClient::new(endpoints, api_key)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(bal.nanotons.to_string(), bal.ton_display))
        }
        Chain::Near => {
            let bal = NearClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.yocto_near, bal.near_display))
        }
        Chain::Icp => {
            let bal = IcpClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(bal.e8s.to_string(), bal.icp_display))
        }
        Chain::Monero => {
            let bal = MoneroClient::new(endpoints).fetch_balance(0).await?;
            Ok(summary_native(bal.piconeros.to_string(), bal.xmr_display))
        }
        Chain::Zcash => {
            let bal = ZcashClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::BitcoinGold => {
            let bal = BitcoinGoldClient::new(endpoints)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Decred => {
            let bal = DecredClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(
                bal.balance_atoms.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Kaspa => {
            let bal = KaspaClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(
                bal.balance_sompi.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Dash => {
            let bal = DashClient::new(endpoints).fetch_balance(address).await?;
            Ok(summary_native(
                bal.balance_sat.to_string(),
                bal.balance_display,
            ))
        }
        Chain::Bittensor => {
            let taostats = service
                .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                .await;
            let api_key = service.api_key_for(chain.str_id()).await;
            let bal = BittensorClient::new(endpoints, taostats, api_key)
                .fetch_balance(address)
                .await?;
            Ok(summary_native(bal.rao.to_string(), bal.tao_display))
        }
        c => Err(SpectraBridgeError::from(format!(
            "unsupported chain: {c:?}"
        ))),
    }
}

fn summary_native(smallest_unit: String, amount_display: String) -> NativeBalanceSummary {
    NativeBalanceSummary {
        smallest_unit,
        amount_display,
        utxo_count: 0,
    }
}
