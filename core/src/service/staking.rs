//! Staking queries use committed transport settings, refreshed for every call.
use super::*;
use crate::staking::{StakingPosition, StakingValidator, service::StakingService};

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn fetch_staking_validators(
        &self,
        chain_id: crate::registry::Chain,
    ) -> Result<Vec<StakingValidator>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let endpoints = this.staking_endpoints(chain_id).await?;
            StakingService::new(vec![endpoints])
                .fetch_validators(chain_id)
                .await
                .map_err(SpectraBridgeError::failure)
        })
        .await
    }
}
impl WalletService {
    /// Inspect the same effective configuration used for the next query.
    pub async fn staking_endpoints(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<ChainEndpoints, SpectraBridgeError> {
        if chain.is_testnet() || !chain.supports_staking() {
            return Err(SpectraBridgeError::InvalidInput {
                message: format!(
                    "Staking queries are unavailable for {}",
                    chain.chain_display_name()
                )
                .into(),
            });
        }
        if !chain.staking_uses_endpoint() {
            return Ok(ChainEndpoints {
                capabilities: vec![],
                chain_id: chain,
                endpoints: vec![],
            });
        }
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Staking])
            .await
            .as_ref()
            .clone();
        if endpoints.is_empty() {
            return Err(SpectraBridgeError::failure(
                "No staking endpoints configured",
            ));
        }
        Ok(ChainEndpoints {
            capabilities: vec![EndpointCapability::Staking],
            chain_id: chain,
            endpoints,
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
                .ok_or_else(|| SpectraBridgeError::failure("Wallet not found"))?;
            let chain = wallet.chain_id;
            let address = wallet
                .address_on(chain)
                .ok_or_else(|| SpectraBridgeError::failure("Wallet has no staking address"))?
                .to_string();
            (chain, address)
        };
        let endpoints = self.staking_endpoints(chain).await?;
        if !crate::send::flow::is_valid_send_address(chain, address.clone()) {
            return Err(SpectraBridgeError::failure(
                "Invalid staking wallet address",
            ));
        }
        StakingService::new(vec![endpoints])
            .fetch_positions(chain, address)
            .await
            .map_err(SpectraBridgeError::failure)
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
                    update: crate::store::state::AppSettingUpdate::AddCustomEndpoint {
                        capabilities: crate::endpoint_capability_options(
                            crate::registry::Chain::Solana,
                            crate::EndpointApi::SolanaJsonRpc,
                        ),
                        chain_id: crate::registry::Chain::Solana,
                        api: "solana-json-rpc".into(),
                        endpoint: url.into(),
                    },
                })
                .await
                .unwrap();
            assert_eq!(
                service
                    .staking_endpoints(crate::registry::Chain::Solana)
                    .await
                    .unwrap()
                    .endpoints[0],
                url
            );
        }
        for chain in [Chain::Bitcoin, Chain::SolanaDevnet] {
            assert!(service.fetch_staking_validators(chain).await.is_err());
        }
        let explicit = WalletService::new(vec![ChainEndpoints {
            capabilities: vec![EndpointCapability::Staking],
            chain_id: crate::registry::Chain::Solana,
            endpoints: vec!["http://127.0.0.1:13003".into()],
        }])
        .unwrap();
        explicit
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        assert_eq!(
            explicit
                .staking_endpoints(crate::registry::Chain::Solana)
                .await
                .unwrap()
                .endpoints,
            vec!["http://127.0.0.1:13003"]
        );
        let _ = std::fs::remove_file(path);
    }
}
