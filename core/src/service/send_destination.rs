//! Recipient resolution and risk checks.
use super::*;
#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Does this destination look unused for the asset this holding sends?
    ///
    /// Named by wallet and holding, not by chain and token descriptor. Which
    /// contract a symbol means is a catalog question and the catalog is core's,
    /// so the caller that used to answer it was reading core's token
    /// preferences to hand them straight back: the composer decided native vs
    /// token, built the descriptor, clamped its decimals into a `u8`, and
    /// silently showed no verdict at all when it could not identify the token.
    ///
    /// Swift also ran the read itself as four chain arms — Bitcoin, "every EVM
    /// chain", Tron, and everything else — that fetched different things and
    /// worded the answer three different ways. Two of the three wordings were
    /// built by string interpolation and never reached the locale files, so a
    /// Tron or EVM token send showed an English warning in a Chinese app.
    ///
    /// The history signal is one question now: has this address transacted on
    /// this chain. EVM adds the nonce because the balance probe returns it
    /// anyway and it needs no explorer key. Bitcoin used to ask
    /// `utxo_count > 0`, which is not that question — an address that received
    /// and later spent everything has history and no UTXOs, and got a warning
    /// stating it had "no transaction history", which was false.
    ///
    /// `destination_input` is what the user typed. Resolving it here rather
    /// than trusting a caller-supplied address keeps the probe asking about
    /// the address a send would actually reach; for one already resolved it is
    /// re-validation and nothing more.
    pub async fn send_destination_risk(
        &self,
        wallet_id: String,
        holding_key: String,
        destination_input: String,
    ) -> Result<SendDestinationRisk, SpectraBridgeError> {
        let (chain, token) = {
            let state = self.wallet_state.read().await;
            let holding = state
                .wallets
                .iter()
                .find(|w| w.id == wallet_id)
                .and_then(|wallet| {
                    wallet
                        .holdings
                        .iter()
                        .find(|h| format!("{}|{}", h.chain_name, h.symbol) == holding_key)
                })
                .ok_or_else(|| SpectraBridgeError::InvalidInput {
                    message: format!("no holding {holding_key} on wallet {wallet_id}"),
                })?;
            destination_probe_asset(holding, &state.token_preferences)?
        };
        let chain_id = chain.str_id().to_string();
        let address = self
            .resolve_send_destination(chain_id.clone(), destination_input)
            .await?
            .address;

        let balance_read = async {
            let display = match token {
                Some(descriptor) => self
                    .fetch_token_balances(chain_id.clone(), address.clone(), vec![descriptor])
                    .await?
                    .first()
                    .ok_or_else(|| SpectraBridgeError::from("token balance unavailable"))?
                    .balance_display
                    .clone(),
                None => {
                    self.fetch_native_balance_summary(chain_id.clone(), address.clone())
                        .await?
                        .amount_display
                }
            };
            let balance = display
                .parse::<f64>()
                .map_err(|_| SpectraBridgeError::from("invalid destination balance"))?;
            if !balance.is_finite() || balance < 0.0 {
                return Err(SpectraBridgeError::from("invalid destination balance"));
            }
            Ok::<_, SpectraBridgeError>(balance)
        };
        let history_read = async {
            // A positive nonce proves activity without an explorer lookup. Zero
            // alone cannot rule out incoming transfers; ask history in that case.
            if chain.is_evm() {
                let client = EvmClient::new(
                    self.endpoints_for(chain.str_id()).await,
                    chain.evm_chain_id(),
                );
                if client.fetch_nonce(&address).await? > 0 {
                    return Ok(true);
                }
            }
            Ok::<_, SpectraBridgeError>(
                self.fetch_history_summary(chain_id.clone(), address.clone())
                    .await?
                    .entry_count
                    > 0,
            )
        };
        let (balance, has_history) = tokio::try_join!(balance_read, history_read)?;

        Ok(SendDestinationRisk {
            balance_is_zero: balance <= 0.0,
            has_history,
        })
    }

    /// Always resolve afresh; no service-lifetime cache for payment destinations.
    pub async fn resolve_send_destination(
        &self,
        chain_id: String,
        input: String,
    ) -> Result<SendDestinationResolution, SpectraBridgeError> {
        resolve_destination(chain_for_id(&chain_id)?, input, |name| {
            self.resolve_ens_name_typed(name)
        })
        .await
    }

    /// Bind the user's review to an address. A changed name requires a new review.
    pub async fn verify_send_destination(
        &self,
        chain_id: String,
        input: String,
        expected_address: String,
    ) -> Result<SendDestinationResolution, SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        let resolved = self.resolve_send_destination(chain_id, input).await?;
        verify_reviewed_destination(chain, resolved, &expected_address)
    }
}

pub(super) fn destination_probe_asset(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> Result<(Chain, Option<TokenDescriptor>), SpectraBridgeError> {
    let chain = Chain::from_display_name(&holding.chain_name).ok_or_else(|| {
        SpectraBridgeError::InvalidInput {
            message: format!("unknown chain: {}", holding.chain_name),
        }
    })?;
    if holding.symbol == chain.coin_symbol() {
        return Ok((chain, None));
    }
    let identity =
        super::maintenance::send_token_identity(holding, preferences).ok_or_else(|| {
            SpectraBridgeError::InvalidInput {
                message: format!(
                    "{} on {} is not a token this wallet tracks",
                    holding.symbol, holding.chain_name
                ),
            }
        })?;
    // The catalog's precision, not a clamp of it. The caller that used to build
    // this descriptor wrote `UInt8(clamping:)`, which turns an impossible 300
    // into a plausible 255 and reads a balance off by 45 decimal places.
    let decimals =
        u8::try_from(identity.decimals).map_err(|_| SpectraBridgeError::InvalidInput {
            message: format!(
                "{} on {} declares {} decimals",
                holding.symbol, holding.chain_name, identity.decimals
            ),
        })?;
    Ok((
        chain,
        Some(TokenDescriptor {
            contract: identity.contract,
            symbol: holding.symbol.clone(),
            decimals,
            name: None,
        }),
    ))
}

pub(super) async fn resolve_destination<F, Fut>(
    chain: Chain,
    input: String,
    lookup: F,
) -> Result<SendDestinationResolution, SpectraBridgeError>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<Option<String>, SpectraBridgeError>>,
{
    let name = chain.chain_display_name();
    let typed = input.trim().to_string();
    if crate::send::flow::is_valid_send_address(name.into(), typed.clone()) {
        return Ok(SendDestinationResolution {
            address: crate::send::flow::normalized_send_address(name.into(), typed),
            used_ens: false,
        });
    }
    if !chain.resolves_ens_names() || !crate::send::flow::is_ens_name_candidate(&typed) {
        return Err(SpectraBridgeError::InvalidInput {
            message: format!("enter a valid {name} destination address"),
        });
    }
    let address = lookup(typed.clone())
        .await?
        .filter(|a| crate::send::flow::is_valid_send_address(name.into(), a.clone()))
        .ok_or_else(|| SpectraBridgeError::InvalidInput {
            message: format!("unable to resolve ENS name '{typed}'"),
        })?;
    Ok(SendDestinationResolution {
        address: crate::send::flow::normalized_send_address(name.into(), address),
        used_ens: true,
    })
}

pub(super) fn verify_reviewed_destination(
    chain: Chain,
    resolved: SendDestinationResolution,
    expected: &str,
) -> Result<SendDestinationResolution, SpectraBridgeError> {
    if !crate::send::flow::is_valid_send_address(chain.chain_display_name().into(), expected.into())
        || crate::send::flow::normalized_send_address(
            chain.chain_display_name().into(),
            expected.into(),
        ) != resolved.address
    {
        return Err(SpectraBridgeError::InvalidInput {
            message: format!(
                "Destination changed to {}. Review the recipient again before sending.",
                resolved.address
            ),
        });
    }
    Ok(resolved)
}
