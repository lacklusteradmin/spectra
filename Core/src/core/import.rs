use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletImportAddresses {
    pub bitcoin_address: Option<String>,
    pub bitcoin_xpub: Option<String>,
    pub bitcoin_cash_address: Option<String>,
    pub bitcoin_sv_address: Option<String>,
    pub litecoin_address: Option<String>,
    pub dogecoin_address: Option<String>,
    pub ethereum_address: Option<String>,
    pub ethereum_classic_address: Option<String>,
    pub tron_address: Option<String>,
    pub solana_address: Option<String>,
    pub xrp_address: Option<String>,
    pub stellar_address: Option<String>,
    pub monero_address: Option<String>,
    pub cardano_address: Option<String>,
    pub sui_address: Option<String>,
    pub aptos_address: Option<String>,
    pub ton_address: Option<String>,
    pub icp_address: Option<String>,
    pub near_address: Option<String>,
    pub polkadot_address: Option<String>,
}

impl WalletImportAddresses {
    fn empty() -> Self {
        Self {
            bitcoin_address: None,
            bitcoin_xpub: None,
            bitcoin_cash_address: None,
            bitcoin_sv_address: None,
            litecoin_address: None,
            dogecoin_address: None,
            ethereum_address: None,
            ethereum_classic_address: None,
            tron_address: None,
            solana_address: None,
            xrp_address: None,
            stellar_address: None,
            monero_address: None,
            cardano_address: None,
            sui_address: None,
            aptos_address: None,
            ton_address: None,
            icp_address: None,
            near_address: None,
            polkadot_address: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletImportWatchOnlyEntries {
    pub bitcoin_addresses: Vec<String>,
    pub bitcoin_xpub: Option<String>,
    pub bitcoin_cash_addresses: Vec<String>,
    pub bitcoin_sv_addresses: Vec<String>,
    pub litecoin_addresses: Vec<String>,
    pub dogecoin_addresses: Vec<String>,
    pub ethereum_addresses: Vec<String>,
    pub tron_addresses: Vec<String>,
    pub solana_addresses: Vec<String>,
    pub xrp_addresses: Vec<String>,
    pub stellar_addresses: Vec<String>,
    pub cardano_addresses: Vec<String>,
    pub sui_addresses: Vec<String>,
    pub aptos_addresses: Vec<String>,
    pub ton_addresses: Vec<String>,
    pub icp_addresses: Vec<String>,
    pub near_addresses: Vec<String>,
    pub polkadot_addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletImportRequest {
    pub wallet_name: String,
    pub default_wallet_name_start_index: u64,
    pub primary_selected_chain_name: String,
    pub selected_chain_names: Vec<String>,
    pub planned_wallet_ids: Vec<String>,
    pub is_watch_only_import: bool,
    pub is_private_key_import: bool,
    pub has_wallet_password: bool,
    pub resolved_addresses: WalletImportAddresses,
    pub watch_only_entries: WalletImportWatchOnlyEntries,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletSecretInstruction {
    pub wallet_id: String,
    pub secret_kind: String,
    pub should_store_seed_phrase: bool,
    pub should_store_private_key: bool,
    pub should_store_password_verifier: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PlannedWallet {
    pub wallet_id: String,
    pub name: String,
    pub chain_name: String,
    pub addresses: WalletImportAddresses,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletImportPlan {
    pub secret_kind: String,
    pub wallets: Vec<PlannedWallet>,
    pub secret_instructions: Vec<WalletSecretInstruction>,
}

pub fn plan_wallet_import(request: WalletImportRequest) -> Result<WalletImportPlan, String> {
    if request.is_watch_only_import {
        plan_watch_only_import(request)
    } else {
        plan_signing_import(request)
    }
}

fn plan_signing_import(request: WalletImportRequest) -> Result<WalletImportPlan, String> {
    if request.selected_chain_names.is_empty() {
        return Err("Select a chain first.".to_string());
    }
    if request.selected_chain_names.len() != request.planned_wallet_ids.len() {
        return Err("Wallet ID plan did not match selected chains.".to_string());
    }

    let selected_chain_count = request.selected_chain_names.len();
    let mut wallets = Vec::with_capacity(selected_chain_count);
    let mut secret_instructions = Vec::with_capacity(selected_chain_count);
    let secret_kind = if request.is_private_key_import {
        "privateKey"
    } else {
        "seedPhrase"
    };

    for (index, (chain_name, wallet_id)) in request
        .selected_chain_names
        .iter()
        .zip(request.planned_wallet_ids.iter())
        .enumerate()
    {
        wallets.push(PlannedWallet {
            wallet_id: wallet_id.clone(),
            name: wallet_display_name(
                &request.wallet_name,
                index + 1,
                request.default_wallet_name_start_index as usize + index,
                selected_chain_count,
            ),
            chain_name: chain_name.clone(),
            addresses: addresses_for_chain(chain_name, &request.resolved_addresses),
        });
        secret_instructions.push(WalletSecretInstruction {
            wallet_id: wallet_id.clone(),
            secret_kind: secret_kind.to_string(),
            should_store_seed_phrase: !request.is_private_key_import,
            should_store_private_key: request.is_private_key_import,
            should_store_password_verifier: !request.is_private_key_import
                && request.has_wallet_password,
        });
    }

    Ok(WalletImportPlan {
        secret_kind: secret_kind.to_string(),
        wallets,
        secret_instructions,
    })
}

fn plan_watch_only_import(request: WalletImportRequest) -> Result<WalletImportPlan, String> {
    let watch_entries = watch_only_addresses_for_chain(
        &request.primary_selected_chain_name,
        &request.watch_only_entries,
    )?;
    if watch_entries.is_empty() {
        return Err("Enter at least one valid address to import.".to_string());
    }
    if request.planned_wallet_ids.len() != watch_entries.len() {
        return Err("Watch-only wallet ID plan did not match expanded requests.".to_string());
    }

    let selected_chain_count = watch_entries.len();
    let wallets = watch_entries
        .into_iter()
        .zip(request.planned_wallet_ids.iter())
        .enumerate()
        .map(
            |(index, ((chain_name, addresses), wallet_id))| PlannedWallet {
                wallet_id: wallet_id.clone(),
                name: wallet_display_name(
                    &request.wallet_name,
                    index + 1,
                    (request.default_wallet_name_start_index as usize) + index,
                    selected_chain_count,
                ),
                chain_name,
                addresses,
            },
        )
        .collect::<Vec<_>>();
    let secret_instructions = request
        .planned_wallet_ids
        .into_iter()
        .map(|wallet_id| WalletSecretInstruction {
            wallet_id,
            secret_kind: "watchOnly".to_string(),
            should_store_seed_phrase: false,
            should_store_private_key: false,
            should_store_password_verifier: false,
        })
        .collect::<Vec<_>>();

    Ok(WalletImportPlan {
        secret_kind: "watchOnly".to_string(),
        wallets,
        secret_instructions,
    })
}

fn watch_only_addresses_for_chain(
    primary_chain_name: &str,
    entries: &WalletImportWatchOnlyEntries,
) -> Result<Vec<(String, WalletImportAddresses)>, String> {
    let wallets = match primary_chain_name {
        "Bitcoin" => {
            if let Some(xpub) = trim_optional(entries.bitcoin_xpub.as_deref()) {
                vec![(
                    "Bitcoin".to_string(),
                    WalletImportAddresses {
                        bitcoin_xpub: Some(xpub.to_string()),
                        ..WalletImportAddresses::empty()
                    },
                )]
            } else {
                entries
                    .bitcoin_addresses
                    .iter()
                    .map(|address| {
                        (
                            "Bitcoin".to_string(),
                            WalletImportAddresses {
                                bitcoin_address: Some(address.clone()),
                                ..WalletImportAddresses::empty()
                            },
                        )
                    })
                    .collect()
            }
        }
        "Bitcoin Cash" => entries
            .bitcoin_cash_addresses
            .iter()
            .map(|address| {
                (
                    "Bitcoin Cash".to_string(),
                    WalletImportAddresses {
                        bitcoin_cash_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Bitcoin SV" => entries
            .bitcoin_sv_addresses
            .iter()
            .map(|address| {
                (
                    "Bitcoin SV".to_string(),
                    WalletImportAddresses {
                        bitcoin_sv_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Litecoin" => entries
            .litecoin_addresses
            .iter()
            .map(|address| {
                (
                    "Litecoin".to_string(),
                    WalletImportAddresses {
                        litecoin_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Dogecoin" => entries
            .dogecoin_addresses
            .iter()
            .map(|address| {
                (
                    "Dogecoin".to_string(),
                    WalletImportAddresses {
                        dogecoin_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism" | "BNB Chain" | "Avalanche"
        | "Hyperliquid" => entries
            .ethereum_addresses
            .iter()
            .map(|address| {
                (
                    primary_chain_name.to_string(),
                    WalletImportAddresses {
                        ethereum_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Tron" => entries
            .tron_addresses
            .iter()
            .map(|address| {
                (
                    "Tron".to_string(),
                    WalletImportAddresses {
                        tron_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Solana" => entries
            .solana_addresses
            .iter()
            .map(|address| {
                (
                    "Solana".to_string(),
                    WalletImportAddresses {
                        solana_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "XRP Ledger" => entries
            .xrp_addresses
            .iter()
            .map(|address| {
                (
                    "XRP Ledger".to_string(),
                    WalletImportAddresses {
                        xrp_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Stellar" => entries
            .stellar_addresses
            .iter()
            .map(|address| {
                (
                    "Stellar".to_string(),
                    WalletImportAddresses {
                        stellar_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Cardano" => entries
            .cardano_addresses
            .iter()
            .map(|address| {
                (
                    "Cardano".to_string(),
                    WalletImportAddresses {
                        cardano_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Sui" => entries
            .sui_addresses
            .iter()
            .map(|address| {
                (
                    "Sui".to_string(),
                    WalletImportAddresses {
                        sui_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Aptos" => entries
            .aptos_addresses
            .iter()
            .map(|address| {
                (
                    "Aptos".to_string(),
                    WalletImportAddresses {
                        aptos_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "TON" => entries
            .ton_addresses
            .iter()
            .map(|address| {
                (
                    "TON".to_string(),
                    WalletImportAddresses {
                        ton_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Internet Computer" => entries
            .icp_addresses
            .iter()
            .map(|address| {
                (
                    "Internet Computer".to_string(),
                    WalletImportAddresses {
                        icp_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "NEAR" => entries
            .near_addresses
            .iter()
            .map(|address| {
                (
                    "NEAR".to_string(),
                    WalletImportAddresses {
                        near_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        "Polkadot" => entries
            .polkadot_addresses
            .iter()
            .map(|address| {
                (
                    "Polkadot".to_string(),
                    WalletImportAddresses {
                        polkadot_address: Some(address.clone()),
                        ..WalletImportAddresses::empty()
                    },
                )
            })
            .collect(),
        unsupported => {
            return Err(format!(
                "Watch-only planning is not available for chain: {unsupported}"
            ))
        }
    };
    Ok(wallets)
}

fn addresses_for_chain(
    chain_name: &str,
    addresses: &WalletImportAddresses,
) -> WalletImportAddresses {
    match chain_name {
        "Bitcoin" => WalletImportAddresses {
            bitcoin_address: addresses.bitcoin_address.clone(),
            bitcoin_xpub: addresses.bitcoin_xpub.clone(),
            ..WalletImportAddresses::empty()
        },
        "Bitcoin Cash" => WalletImportAddresses {
            bitcoin_cash_address: addresses.bitcoin_cash_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Bitcoin SV" => WalletImportAddresses {
            bitcoin_sv_address: addresses.bitcoin_sv_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Litecoin" => WalletImportAddresses {
            litecoin_address: addresses.litecoin_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Dogecoin" => WalletImportAddresses {
            dogecoin_address: addresses.dogecoin_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Ethereum" | "Arbitrum" | "Optimism" | "BNB Chain" | "Avalanche" | "Hyperliquid" => {
            WalletImportAddresses {
                ethereum_address: addresses.ethereum_address.clone(),
                ..WalletImportAddresses::empty()
            }
        }
        "Ethereum Classic" => WalletImportAddresses {
            ethereum_address: addresses.ethereum_classic_address.clone(),
            ethereum_classic_address: addresses.ethereum_classic_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Tron" => WalletImportAddresses {
            tron_address: addresses.tron_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Solana" => WalletImportAddresses {
            solana_address: addresses.solana_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "XRP Ledger" => WalletImportAddresses {
            xrp_address: addresses.xrp_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Stellar" => WalletImportAddresses {
            stellar_address: addresses.stellar_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Monero" => WalletImportAddresses {
            monero_address: addresses.monero_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Cardano" => WalletImportAddresses {
            cardano_address: addresses.cardano_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Sui" => WalletImportAddresses {
            sui_address: addresses.sui_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Aptos" => WalletImportAddresses {
            aptos_address: addresses.aptos_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "TON" => WalletImportAddresses {
            ton_address: addresses.ton_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Internet Computer" => WalletImportAddresses {
            icp_address: addresses.icp_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "NEAR" => WalletImportAddresses {
            near_address: addresses.near_address.clone(),
            ..WalletImportAddresses::empty()
        },
        "Polkadot" => WalletImportAddresses {
            polkadot_address: addresses.polkadot_address.clone(),
            ..WalletImportAddresses::empty()
        },
        _ => WalletImportAddresses::empty(),
    }
}

fn wallet_display_name(
    base_name: &str,
    batch_position: usize,
    default_wallet_index: usize,
    selected_chain_count: usize,
) -> String {
    let trimmed = base_name.trim();
    if trimmed.is_empty() {
        return format!("Wallet {}", default_wallet_index);
    }
    if selected_chain_count > 1 {
        format!("{trimmed} {batch_position}")
    } else {
        trimmed.to_string()
    }
}

fn trim_optional(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_multi_chain_seed_import() {
        let plan = plan_wallet_import(WalletImportRequest {
            wallet_name: "Main".to_string(),
            default_wallet_name_start_index: 4,
            primary_selected_chain_name: "Bitcoin".to_string(),
            selected_chain_names: vec!["Bitcoin".to_string(), "Ethereum".to_string()],
            planned_wallet_ids: vec!["1".to_string(), "2".to_string()],
            is_watch_only_import: false,
            is_private_key_import: false,
            has_wallet_password: true,
            resolved_addresses: WalletImportAddresses {
                bitcoin_address: Some("bc1qexample".to_string()),
                ethereum_address: Some("0x1234".to_string()),
                ethereum_classic_address: Some("0x5678".to_string()),
                ..WalletImportAddresses::empty()
            },
            watch_only_entries: WalletImportWatchOnlyEntries {
                bitcoin_addresses: Vec::new(),
                bitcoin_xpub: None,
                bitcoin_cash_addresses: Vec::new(),
                bitcoin_sv_addresses: Vec::new(),
                litecoin_addresses: Vec::new(),
                dogecoin_addresses: Vec::new(),
                ethereum_addresses: Vec::new(),
                tron_addresses: Vec::new(),
                solana_addresses: Vec::new(),
                xrp_addresses: Vec::new(),
                stellar_addresses: Vec::new(),
                cardano_addresses: Vec::new(),
                sui_addresses: Vec::new(),
                aptos_addresses: Vec::new(),
                ton_addresses: Vec::new(),
                icp_addresses: Vec::new(),
                near_addresses: Vec::new(),
                polkadot_addresses: Vec::new(),
            },
        })
        .expect("plan");

        assert_eq!(plan.wallets.len(), 2);
        assert_eq!(plan.wallets[0].name, "Main 1");
        assert_eq!(plan.secret_instructions[0].secret_kind, "seedPhrase");
    }

    #[test]
    fn plans_watch_only_bitcoin_xpub_import() {
        let plan = plan_wallet_import(WalletImportRequest {
            wallet_name: String::new(),
            default_wallet_name_start_index: 7,
            primary_selected_chain_name: "Bitcoin".to_string(),
            selected_chain_names: vec!["Bitcoin".to_string()],
            planned_wallet_ids: vec!["watch-1".to_string()],
            is_watch_only_import: true,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses::empty(),
            watch_only_entries: WalletImportWatchOnlyEntries {
                bitcoin_addresses: Vec::new(),
                bitcoin_xpub: Some("xpub123".to_string()),
                bitcoin_cash_addresses: Vec::new(),
                bitcoin_sv_addresses: Vec::new(),
                litecoin_addresses: Vec::new(),
                dogecoin_addresses: Vec::new(),
                ethereum_addresses: Vec::new(),
                tron_addresses: Vec::new(),
                solana_addresses: Vec::new(),
                xrp_addresses: Vec::new(),
                stellar_addresses: Vec::new(),
                cardano_addresses: Vec::new(),
                sui_addresses: Vec::new(),
                aptos_addresses: Vec::new(),
                ton_addresses: Vec::new(),
                icp_addresses: Vec::new(),
                near_addresses: Vec::new(),
                polkadot_addresses: Vec::new(),
            },
        })
        .expect("plan");

        assert_eq!(plan.wallets.len(), 1);
        assert_eq!(plan.wallets[0].name, "Wallet 7");
        assert_eq!(
            plan.wallets[0].addresses.bitcoin_xpub.as_deref(),
            Some("xpub123")
        );
        assert_eq!(plan.secret_kind, "watchOnly");
    }
}
