//! Staking queries use committed transport settings, refreshed for every call.
use super::*;
use crate::staking::{service::StakingService, StakingPosition, StakingValidator};

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn fetch_staking_validators(
        &self,
        chain_id: String,
    ) -> Result<Vec<StakingValidator>, SpectraBridgeError> {
        let endpoints = self.staking_endpoints(chain_id.clone()).await?;
        StakingService::new(vec![endpoints])
            .fetch_validators(chain_id)
            .await
            .map_err(|e| SpectraBridgeError::from(e.to_string()))
    }
}
impl WalletService {
    /// Inspect the same effective configuration used for the next query.
    pub async fn staking_endpoints(
        &self,
        chain_id: String,
    ) -> Result<ChainEndpoints, SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        if chain.is_testnet() || !chain.supports_staking() {
            return Err(SpectraBridgeError::InvalidInput {
                message: format!(
                    "{} does not have protocol-native staking",
                    chain.chain_display_name()
                ),
            });
        }
        let endpoints = self.endpoints_for(chain.str_id()).await.as_ref().clone();
        if endpoints.is_empty() {
            return Err("No staking endpoints configured".into());
        }
        Ok(ChainEndpoints {
            chain_id,
            endpoints,
            api_key: self.api_key_for(chain.str_id()).await,
        })
    }
    pub async fn fetch_staking_positions(
        &self,
        wallet_id: String,
    ) -> Result<Vec<StakingPosition>, SpectraBridgeError> {
        let (chain, address) = {
            let state = self.wallet_state.read().await;
            let wallet = state
                .wallets
                .iter()
                .find(|w| w.id == wallet_id)
                .ok_or_else(|| SpectraBridgeError::from("Wallet not found"))?;
            let chain = wallet
                .chain()
                .ok_or_else(|| SpectraBridgeError::from("Wallet has no network"))?;
            let address = wallet
                .address_on(chain)
                .ok_or_else(|| SpectraBridgeError::from("Wallet has no staking address"))?
                .to_string();
            (chain, address)
        };
        let endpoints = self.staking_endpoints(chain.str_id().into()).await?;
        if !crate::send::flow::is_valid_send_address(
            chain.chain_display_name().into(),
            address.clone(),
        ) {
            return Err("Invalid staking wallet address".into());
        }
        StakingService::new(vec![endpoints])
            .fetch_positions(chain.str_id().into(), address)
            .await
            .map_err(|e| SpectraBridgeError::from(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn staking_reads_latest_owned_settings_and_preserves_explicit_overrides() {
        let path =
            std::env::temp_dir().join(format!("staking-{}.db", crate::store::new_event_id()));
        let service = WalletService::new_catalog().unwrap();
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        for url in ["http://127.0.0.1:13001", "http://127.0.0.1:13002"] {
            service
                .apply_state_command(StateCommand::SetAppSetting {
                    update: crate::store::state::AppSettingUpdate::RpcEndpoint {
                        chain: "Solana".into(),
                        value: url.into(),
                    },
                })
                .await
                .unwrap();
            assert_eq!(
                service
                    .staking_endpoints("solana".into())
                    .await
                    .unwrap()
                    .endpoints[0],
                url
            );
        }
        for chain in ["bitcoin", "solana-devnet"] {
            assert!(service
                .fetch_staking_validators(chain.into())
                .await
                .is_err());
        }
        let explicit = WalletService::new(vec![ChainEndpoints {
            chain_id: "solana".into(),
            endpoints: vec!["http://127.0.0.1:13003".into()],
            api_key: None,
        }])
        .unwrap();
        explicit
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        assert_eq!(
            explicit
                .staking_endpoints("solana".into())
                .await
                .unwrap()
                .endpoints,
            vec!["http://127.0.0.1:13003"]
        );
        let _ = std::fs::remove_file(path);
    }
}
