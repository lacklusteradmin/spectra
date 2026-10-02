//! Recipient resolution and risk checks.
use super::*;
#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Does this destination look unused for the asset this holding sends?
    ///
    /// Named by wallet and holding, not by chain and token descriptor: which
    /// contract a symbol means is a catalog question, and the catalog is
    /// core's.
    ///
    /// The history signal is one question: has this address transacted on
    /// this chain. EVM adds the nonce because the balance probe returns it
    /// anyway and it needs no explorer key. A UTXO count is not that question
    /// — an address that received and later spent everything has history and
    /// no UTXOs.
    ///
    /// `destination_input` is what the user typed. Resolving it here rather
    /// than trusting a caller-supplied address keeps the probe asking about
    /// the address a send would actually reach; for one already resolved it is
    /// re-validation and nothing more.
    ///
    /// `holding_key` names what the probe asks about, and resolves to the
    /// chain's own asset or to one tracked token. A token the user does not
    /// track has no descriptor and is a refusal rather than a fallback: the
    /// probe would otherwise read the *chain's* balance and report it as the
    /// token's, or read nothing and show no verdict. Neither says "we could
    /// not check".
    pub async fn send_destination_risk(
        &self,
        wallet_id: String,
        holding_key: String,
        destination_input: String,
    ) -> Result<SendDestinationRisk, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let (chain, token) = this
                .destination_probe_target(&wallet_id, &holding_key)
                .await?;
            let chain_id = chain;
            let address = this
                .resolve_send_destination(chain_id, destination_input)
                .await?
                .address;

            let balance_read = async {
                let raw = match token {
                    Some(descriptor) => this
                        .fetch_token_balances(chain_id, address.clone(), vec![descriptor])
                        .await?
                        .first()
                        .ok_or_else(|| SpectraBridgeError::failure("token balance unavailable"))?
                        .balance_raw
                        .clone(),
                    None => {
                        this.fetch_native_balance_summary(chain_id, address.clone())
                            .await?
                            .smallest_unit
                    }
                };
                if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(SpectraBridgeError::failure("invalid destination balance"));
                }
                Ok::<_, SpectraBridgeError>(raw.bytes().all(|b| b == b'0'))
            };
            let history_read = async {
                // A positive nonce proves activity without an explorer lookup. Zero
                // alone cannot rule out incoming transfers; ask history in that case.
                if chain.is_evm() {
                    let client = EvmClient::new(
                        this.endpoints_for(chain, &[EndpointCapability::Verification])
                            .await,
                        chain.evm_chain_id()?,
                    );
                    if client.fetch_nonce(&address).await? > 0 {
                        return Ok(true);
                    }
                }
                Ok::<_, SpectraBridgeError>(
                    this.fetch_history_summary(chain_id, address.clone())
                        .await?
                        .entry_count
                        > 0,
                )
            };
            let (balance, history) = tokio::join!(balance_read, history_read);
            let balance_is_zero = balance?;
            // A balance is itself history: something was sent here. Only an
            // empty address needs the history read to tell used from unused,
            // so its failure is fatal only then. Otherwise a chain without an
            // explorer — every testnet — could never be checked at all, even
            // for an address its RPC shows holding funds.
            let has_history = match history {
                Ok(has_history) => has_history,
                Err(_) if !balance_is_zero => true,
                Err(error) => return Err(error),
            };

            Ok(SendDestinationRisk::from_probe(
                balance_is_zero,
                has_history,
            ))
        })
        .await
    }

    /// Always resolve afresh; no service-lifetime cache for payment destinations.
    pub async fn resolve_send_destination(
        &self,
        chain_id: crate::registry::Chain,
        input: String,
    ) -> Result<SendDestinationResolution, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            resolve_destination(chain_id, input, |name| this.resolve_ens_name(name)).await
        })
        .await
    }
}

impl WalletService {
    /// Bind the user's review to an address. A changed name requires a new review.
    ///
    /// Not exported: the build binds the review itself, and the CLI calls it
    /// as Rust.
    pub async fn verify_send_destination(
        &self,
        chain: crate::registry::Chain,
        input: String,
        expected_address: String,
    ) -> Result<SendDestinationResolution, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let resolved = this.resolve_send_destination(chain, input).await?;
            verify_reviewed_destination(chain, resolved, &expected_address)
        })
        .await
    }
}

impl WalletService {
    /// The network and asset a destination check for this holding asks about.
    pub(super) async fn destination_probe_target(
        &self,
        wallet_id: &str,
        holding_key: &str,
    ) -> Result<(Chain, Option<TokenDescriptor>), SpectraBridgeError> {
        let state = self.wallet_state.read().await;
        let holding = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .and_then(|wallet| {
                wallet
                    .holdings
                    .iter()
                    .find(|h| h.deployment_id() == holding_key)
            })
            .ok_or_else(|| SpectraBridgeError::InvalidInput {
                message: format!("no holding {holding_key} on wallet {wallet_id}"),
            })?;
        let (family, token) = destination_probe_asset(holding, &state.token_preferences)?;
        let chain = super::send_execution::send_chain_for(&state, wallet_id, family)?;
        Ok((chain, token))
    }
}

pub(super) fn destination_probe_asset(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> Result<(Chain, Option<TokenDescriptor>), SpectraBridgeError> {
    let asset = crate::send::SendAsset::of(holding, preferences).ok_or_else(|| {
        SpectraBridgeError::InvalidInput {
            message: format!("unknown chain: {}", holding.chain_id),
        }
    })?;
    let chain = asset.chain;
    let identity = match asset.kind {
        crate::send::SendAssetKind::Native => return Ok((chain, None)),
        crate::send::SendAssetKind::Token(identity) => identity,
        crate::send::SendAssetKind::UntrackedToken => {
            return Err(SpectraBridgeError::InvalidInput {
                message: format!(
                    "{} on {} is not a token this wallet tracks",
                    holding.symbol, holding.chain_id
                ),
            });
        }
    };
    // The catalog's precision, not a clamp of it: clamping would turn an
    // impossible 300 into a plausible 255.
    let decimals =
        u8::try_from(identity.decimals).map_err(|_| SpectraBridgeError::InvalidInput {
            message: format!(
                "{} on {} declares {} decimals",
                holding.symbol, holding.chain_id, identity.decimals
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
    let id = chain;
    let typed = input.trim().to_string();
    if crate::send::flow::is_valid_send_address(id, typed.clone()) {
        return Ok(SendDestinationResolution {
            address: crate::send::flow::normalized_send_address(id, typed),
            used_ens: false,
        });
    }
    if !chain.resolves_ens_names() || !crate::send::flow::is_ens_name_candidate(&typed) {
        return Err(SpectraBridgeError::InvalidInput {
            message: format!(
                "enter a valid {} destination address",
                chain.chain_display_name()
            ),
        });
    }
    let address = lookup(typed.clone())
        .await?
        .filter(|a| crate::send::flow::is_valid_send_address(id, a.clone()))
        .ok_or_else(|| SpectraBridgeError::InvalidInput {
            message: format!("unable to resolve ENS name '{typed}'"),
        })?;
    Ok(SendDestinationResolution {
        address: crate::send::flow::normalized_send_address(id, address),
        used_ens: true,
    })
}

pub(super) fn verify_reviewed_destination(
    chain: Chain,
    resolved: SendDestinationResolution,
    expected: &str,
) -> Result<SendDestinationResolution, SpectraBridgeError> {
    if !crate::send::flow::is_valid_send_address(chain, expected.into())
        || crate::send::flow::normalized_send_address(chain, expected.into()) != resolved.address
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
