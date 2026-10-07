//! A Solana wallet's empty token accounts, and closing them to get their
//! rent back.
//!
//! Every SPL or Token-2022 token a Solana address has held sits in a token
//! account that keeps about 0.002 SOL of rent, and sending a token's whole
//! balance away leaves the account behind, empty. Closing one returns its
//! rent to the owner. Core lists the empty accounts with what each returns,
//! refuses those the network would not close, and closes up to 20 in one
//! transaction through the ordinary send stages under
//! [`WalletOperation::CloseTokenAccounts`].

use super::*;
use crate::send::solana::{MAX_CLOSED_ACCOUNTS, PreparedSolanaAccountClosure};
use crate::send::stages::{
    PreparedPayload, SendArtifact, SendArtifactReview, SendStage, StoredSend, WalletOperation,
};

/// One empty token account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EmptyTokenAccount {
    pub address: String,
    pub mint: String,
    /// The token's symbol where Spectra knows it, else the mint.
    pub symbol: String,
    /// Whether the account is a Token-2022 one.
    pub token_2022: bool,
    /// The rent closing it returns, as an exact decimal of SOL.
    pub rent: String,
    /// Why the network would not close it; `None` when it can be closed.
    pub blocked: Option<String>,
}

/// A wallet's empty token accounts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EmptyTokenAccounts {
    pub accounts: Vec<EmptyTokenAccount>,
    /// The rent the closable ones return together, as an exact decimal of
    /// SOL.
    pub reclaimable: String,
}

/// Why the network would refuse closing `account` for `owner`, or `None`.
fn closure_blocked(
    account: &crate::api::solana_json_rpc::SolanaTokenAccount,
    owner: &str,
) -> Option<&'static str> {
    if account.amount != 0 {
        Some("The account still holds tokens.")
    } else if account.owner != owner {
        Some("The wallet does not own this account.")
    } else if account
        .close_authority
        .as_deref()
        .is_some_and(|authority| authority != owner)
    {
        Some("Another address holds this account's close authority.")
    } else if account.frozen {
        Some("The token's issuer froze this account.")
    } else if account.withheld != 0 {
        Some("Transfer fees are withheld in this account until the issuer collects them.")
    } else {
        None
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The wallet's empty token accounts and the rent each returns, read
    /// from a verified node; the ones the network would not close say why.
    pub async fn wallet_empty_token_accounts(
        &self,
        wallet_id: String,
    ) -> Result<EmptyTokenAccounts, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (chain, owner) = this.solana_owner(&wallet_id).await?;
            let state = this.app_state().await;
            let decimals = u32::from(chain.native_decimals());
            let mut reclaimable = 0u128;
            let accounts = this
                .solana_client(chain)
                .await?
                .fetch_token_accounts(&owner)
                .await?
                .into_iter()
                .filter(|account| account.amount == 0)
                .map(|account| {
                    let blocked = closure_blocked(&account, &owner);
                    if blocked.is_none() {
                        reclaimable += u128::from(account.lamports);
                    }
                    EmptyTokenAccount {
                        symbol: super::send_records::send_asset_names(
                            &state,
                            chain,
                            Some(&account.mint),
                        )
                        .0,
                        token_2022: account.program == crate::send::solana::TOKEN_PROGRAMS[1],
                        rent: crate::decimal::from_units(u128::from(account.lamports), decimals),
                        blocked: blocked.map(str::to_string),
                        address: account.address,
                        mint: account.mint,
                    }
                })
                .collect();
            Ok(EmptyTokenAccounts {
                accounts,
                reclaimable: crate::decimal::from_units(reclaimable, decimals),
            })
        })
        .await
    }

    /// Build the transaction that closes empty token accounts into the
    /// wallet: `accounts`, or with none named every closable one, at most 20
    /// at once. Prepared and stored like any send, to be signed and
    /// broadcast through the same stages. An account the network would not
    /// close is refused.
    pub async fn build_token_account_closure(
        &self,
        wallet_id: String,
        accounts: Vec<String>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (chain, owner) = this.solana_owner(&wallet_id).await?;
            if this.stored_wallet(&wallet_id).await?.is_watch_only() {
                return Err(SpectraBridgeError::invalid(
                    "a watch-only wallet cannot send",
                ));
            }
            let client = this.solana_client(chain).await?;
            let held = client.fetch_token_accounts(&owner).await?;
            let chosen: Vec<_> = if accounts.is_empty() {
                held.iter()
                    .filter(|account| {
                        account.amount == 0 && closure_blocked(account, &owner).is_none()
                    })
                    .take(MAX_CLOSED_ACCOUNTS)
                    .collect()
            } else {
                if accounts.len() > MAX_CLOSED_ACCOUNTS {
                    return Err(SpectraBridgeError::invalid(
                        "One transaction closes at most 20 accounts.",
                    ));
                }
                let mut chosen = Vec::new();
                for address in &accounts {
                    let account = held
                        .iter()
                        .find(|account| account.address == *address)
                        .ok_or_else(|| {
                            SpectraBridgeError::refused(
                                "%@ is not a token account of this wallet.",
                                [address],
                            )
                        })?;
                    if let Some(reason) = closure_blocked(account, &owner) {
                        return Err(SpectraBridgeError::invalid(reason));
                    }
                    if !chosen.contains(&account) {
                        chosen.push(account);
                    }
                }
                chosen
            };
            if chosen.is_empty() {
                return Err(SpectraBridgeError::invalid(
                    "The wallet has no empty token accounts to close.",
                ));
            }
            let rent: u128 = chosen
                .iter()
                .map(|account| u128::from(account.lamports))
                .sum();
            let pairs: Vec<(String, String)> = chosen
                .iter()
                .map(|account| (account.address.clone(), account.program.clone()))
                .collect();
            let blockhash = client.fetch_recent_blockhash().await?;
            let trial =
                PreparedSolanaAccountClosure::prepare(&owner, pairs.clone(), &blockhash, 0)?;
            let fee = client
                .fetch_staking_message_fee(&trial.transaction.message)
                .await?;
            // The fee is paid before any rent comes back.
            if client.fetch_balance(&owner).await?.lamports < fee {
                return Err(SpectraBridgeError::invalid(
                    "Insufficient SOL for the network fee",
                ));
            }
            let prepared = PreparedSolanaAccountClosure::prepare(&owner, pairs, &blockhash, fee)?;
            let decimals = u32::from(chain.native_decimals());
            let network_fee = crate::decimal::from_units(u128::from(fee), decimals);
            let rent = crate::decimal::from_units(rent, decimals);
            let request = crate::send::SendExecutionRequest {
                chain_id: chain,
                wallet_id: wallet_id.clone(),
                password: None,
                to_address: owner.clone(),
                amount_str: rent.clone(),
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: Some(network_fee.clone()),
                evm_overrides: None,
                sign_only: false,
            };
            let signing_payload_hex = hex::encode(&prepared.transaction.message);
            let closed = prepared
                .accounts
                .iter()
                .map(|(account, _)| account.clone())
                .collect();
            let prepared = PreparedPayload::SolanaAccountClosure(prepared);
            let mut stored = StoredSend {
                view: SendArtifact {
                    id: crate::store::new_transaction_id(),
                    revision: 0,
                    stage: SendStage::Prepared,
                    wallet_id,
                    chain_id: chain,
                    sender: owner.clone(),
                    recipient: owner,
                    amount: rent.clone(),
                    asset: chain.coin_symbol().into(),
                    symbol: chain.coin_symbol().into(),
                    staking: None,
                    operation: Some(WalletOperation::CloseTokenAccounts {
                        accounts: closed,
                        rent,
                        network_fee,
                    }),
                    created_at: crate::store::now_unix().floor(),
                    review_digest: String::new(),
                    review: SendArtifactReview::default(),
                    prepared_details: serde_json::to_string_pretty(&prepared)?,
                    signing_payload_hex,
                    signed_payload: None,
                    transaction_hash: None,
                    attempts: Vec::new(),
                    selected_endpoints: Vec::new(),
                },
                request,
                prepared,
                submission: None,
                signed_digest: None,
                substrate_verified_through: None,
                icp_staking_receipts: vec![],
            };
            stored.view.review_digest = stored.digest()?;
            this.save_send_artifact(&stored, Vec::new()).await?;
            Ok(stored.view)
        })
        .await
    }
}

impl WalletService {
    /// A Solana wallet's network and address, or a refusal.
    async fn solana_owner(&self, wallet_id: &str) -> Result<(Chain, String), SpectraBridgeError> {
        let wallet = self.stored_wallet(wallet_id).await?;
        let chain = wallet.chain_id;
        if chain.mainnet_counterpart() != Chain::Solana {
            return Err(SpectraBridgeError::invalid(
                "Only a Solana wallet holds token accounts.",
            ));
        }
        let owner = wallet
            .address_on(chain)
            .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address on this network"))?
            .to_string();
        Ok((chain, owner))
    }

    async fn solana_client(&self, chain: Chain) -> Result<SolanaClient, SpectraBridgeError> {
        let client = SolanaClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        );
        client.verify_network(chain).await?;
        Ok(client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::solana_json_rpc::SolanaTokenAccount;

    const OWNER: &str = "AKnL4NNf3DGWZJS6cPknBuEGnVsV4A4m5tgebLHaRSZ9";

    fn empty() -> SolanaTokenAccount {
        SolanaTokenAccount {
            address: "29d2S7vB453rNYFdR5Ycwt7y9haRT5fwVwL9zTmBhfV2".into(),
            program: crate::send::solana::TOKEN_PROGRAMS[1].into(),
            mint: "3JF3sEqM796hk5WFqA6EtmEwJQ9quALszsfJyvXNQKy3".into(),
            owner: OWNER.into(),
            amount: 0,
            lamports: 2_039_280,
            close_authority: None,
            frozen: false,
            withheld: 0,
        }
    }

    /// Each reason the token programs refuse `CloseAccount`, and the owner
    /// closing its own empty account.
    #[test]
    fn only_what_the_network_closes_is_closable() {
        assert_eq!(closure_blocked(&empty(), OWNER), None);
        assert_eq!(
            closure_blocked(
                &SolanaTokenAccount {
                    close_authority: Some(OWNER.into()),
                    ..empty()
                },
                OWNER
            ),
            None
        );
        for (account, words) in [
            (
                SolanaTokenAccount {
                    amount: 1,
                    ..empty()
                },
                "holds tokens",
            ),
            (
                SolanaTokenAccount {
                    owner: "someone".into(),
                    ..empty()
                },
                "does not own",
            ),
            (
                SolanaTokenAccount {
                    close_authority: Some("someone".into()),
                    ..empty()
                },
                "close authority",
            ),
            (
                SolanaTokenAccount {
                    frozen: true,
                    ..empty()
                },
                "froze",
            ),
            (
                SolanaTokenAccount {
                    withheld: 5,
                    ..empty()
                },
                "withheld",
            ),
        ] {
            assert!(
                closure_blocked(&account, OWNER).is_some_and(|reason| reason.contains(words)),
                "{words}"
            );
        }
    }
}
