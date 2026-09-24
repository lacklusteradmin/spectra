//! Commands that talk to a chain: the supported list, balances and history.
//!
//! Endpoint selection is core's — `filtered_endpoint_records_for_chain` picks
//! them from the catalog by API and capability. The CLI supplies the filter mask for what it
//! is about to do and nothing else.

use clap::Args;
use colored::Colorize as _;
use spectra_core::registry::Chain;
use spectra_core::service::{ChainEndpoints, WalletService};
use std::sync::Arc;

use super::resolve_chain;
use crate::ctx::{wallet_address, Ctx};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out};

pub use spectra_core::{
    ENDPOINT_CAPABILITY_BALANCE, ENDPOINT_CAPABILITY_BROADCAST, ENDPOINT_CAPABILITY_FEE,
    ENDPOINT_CAPABILITY_HISTORY, ENDPOINT_CAPABILITY_UTXO,
};

#[derive(Args)]
pub struct ChainsArgs {
    /// Only chains whose name or symbol contains this text.
    #[arg(long)]
    filter: Option<String>,
    /// Include testnets.
    #[arg(long)]
    testnets: bool,
}

#[derive(Args)]
pub struct BalanceArgs {
    /// Wallet id, name or address.
    wallet: String,
}

#[derive(Args)]
pub struct HistoryArgs {
    /// Wallet id, name or address.
    wallet: String,
    /// Most entries to show.
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// Merge what is fetched into the stored history, as the app does.
    #[arg(long)]
    save: bool,
    /// Maximum Bitcoin history pages to merge in this session.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=1000), requires = "save")]
    pages: u32,
    /// Override the selected network's history endpoint (also useful for local fixtures).
    #[arg(long)]
    endpoint: Option<String>,
}

/// A service bound to one chain's endpoints for the capabilities a command needs.
pub fn service_for_chain(
    ctx: &Ctx,
    chain: Chain,
    filter_mask: u32,
) -> CliResult<Arc<WalletService>> {
    let service = ctx.service()?;
    let records =
        spectra_core::filtered_endpoint_records_for_chain(chain.str_id().into(), filter_mask)?;
    let api = chain.endpoint_api(spectra_core::registry::EndpointSlot::Primary);
    if !records.iter().any(|row| row.api == api) {
        return Err(CliError::failure(format!(
            "no compatible endpoints registered for {}",
            chain.chain_display_name()
        )));
    }
    Ok(service)
}

pub fn chains(out: Out, args: ChainsArgs) -> CliResult<()> {
    let needle = args.filter.as_deref().map(str::to_lowercase);
    let listed: Vec<Chain> = Chain::all()
        .filter(|chain| args.testnets || chain.mainnet_counterpart() == *chain)
        .filter(|chain| match &needle {
            None => true,
            Some(needle) => {
                chain.chain_display_name().to_lowercase().contains(needle)
                    || chain.coin_symbol().to_lowercase().contains(needle)
            }
        })
        .collect();

    out.text(|| {
        println!();
        for chain in &listed {
            println!(
                "  {}  {:<22} {:<8} {}",
                out::tint("●", chain.str_id()).bold(),
                out::tint(chain.chain_display_name(), chain.str_id()),
                chain.coin_symbol(),
                out::hint(chain.str_id()),
            );
        }
        println!();
        println!(
            "  {} {}",
            out::accent(&listed.len().to_string()).bold(),
            out::hint("chains")
        );
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chains": listed
            .iter()
            .map(|chain| serde_json::json!({
                "id": chain.str_id(),
                "name": chain.chain_display_name(),
                "nativeSymbol": chain.coin_symbol(),
                "nativeDeploymentId": chain.entry().native_deployment_id,
                "family": chain.entry().family,
                "isTestnet": chain.is_testnet(),
                "isEvm": chain.is_evm(),
                // The import picker's list, as a column rather than a second
                // array: a chain is offered for private-key import exactly
                // when a key derives an address on it.
                "privateKeyImport": chain.derives_from_private_key(),
                // Likewise the watch-addresses picker: the app rendered a
                // hand-written eighteen-section list against this flag and
                // disagreed with it in both directions.
                "watchOnlyImport": chain.supports_watch_only_import(),
                // And the staking tab's, which was a seven-case Swift enum and
                // two match arms in `StakingService` before it was a column.
                "staking": chain.supports_staking(),
                "supportsSeparateSigning": chain.supports_sign_only(),
                "sendUnavailableReason": chain.transparent_send_unavailable_reason(),
                // The setup picker's short list, which was eight ids typed
                // into the Swift view — a rank, so the order comes with it.
                "popularRank": chain.entry().popular_rank,
                // What the send screen's network card says core will do with
                // a send here — twelve per-chain sentences in the iOS view
                // before it was a column, one of which described Monero.
                "sendBroadcastMode": chain.has_send_preview().then(|| match chain.send_broadcast_mode() {
                    spectra_core::registry::SendBroadcastMode::SignsAndBroadcasts => "signsAndBroadcasts",
                    spectra_core::registry::SendBroadcastMode::PreparesWithBackend => "preparesWithBackend",
                }),
                // Explorer history source for EVM chains only. Other families use
                // different history routes.
                "historySource": chain.is_evm().then(|| match chain.evm_history_source() {
                    spectra_core::registry::EvmHistorySource::Open(base) => base,
                    spectra_core::registry::EvmHistorySource::Unavailable => "none",
                }),
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}

#[derive(Args)]
pub struct EndpointsArgs {
    /// Chain to probe. Omit to probe every chain the catalog knows.
    #[arg(long)]
    chain: Option<String>,
    /// List registered endpoints and capabilities offline, without health probes.
    #[arg(long)]
    catalog: bool,
    /// Save a custom endpoint offline. Requires --chain and --api.
    #[arg(long, requires_all = ["chain", "api", "capabilities"], conflicts_with = "catalog")]
    add: Option<String>,
    /// API contract, using the api value from endpoints.toml.
    #[arg(long, requires = "add")]
    api: Option<String>,
    /// Capabilities enabled on this endpoint (comma separated).
    #[arg(long, requires = "add", value_delimiter = ',')]
    capabilities: Vec<String>,
    /// Restrict offline listing to built-in or custom endpoints.
    #[arg(long, requires = "catalog", value_parser = ["built-in", "custom"])]
    source: Option<String>,
}

/// Check read methods for every API, including testnets and history indexers.
/// Missing providers and unchecked APIs are reported separately from web links.
pub fn endpoints(ctx: &Ctx, out: Out, args: EndpointsArgs) -> CliResult<()> {
    let chains: Vec<Chain> = match &args.chain {
        Some(name) => vec![super::resolve_chain(name)?],
        None => Chain::all().collect(),
    };
    if let Some(url) = args.add {
        let transition = ctx.apply(spectra_core::store::state::StateCommand::SetAppSetting {
            update: spectra_core::store::state::AppSettingUpdate::AddCustomEndpoint {
                chain_id: chains[0].str_id().into(),
                api: args.api.unwrap(),
                endpoint: url,
                capabilities: args.capabilities,
            },
        })?;
        if transition
            .events
            .contains(&spectra_core::store::state::StateEvent::AppSettingRejected)
        {
            return Err(CliError::rejected(
                "Invalid or duplicate endpoint, or unsupported/empty capabilities for this adapter",
            ));
        }
        out.emit(serde_json::json!({"ok":true,"customEndpoints":transition.state.settings.custom_endpoints}));
        return Ok(());
    }
    if args.catalog {
        let service = WalletService::new_catalog()?;
        ctx.rt.block_on(service.open_state(ctx.db_path()))?;
        let entries = ctx.rt.block_on(service.endpoint_directory())?;
        let records: Vec<_> = entries
            .into_iter()
            .filter(|entry| {
                (args.chain.is_none()
                    || chains
                        .iter()
                        .any(|chain| chain.str_id() == entry.record.chain_id))
                    && args
                        .source
                        .as_deref()
                        .is_none_or(|source| entry.is_built_in == (source == "built-in"))
            })
            .collect();
        out.text(|| {
            for record in &records {
                println!(
                    "{}  {}  {}",
                    record.record.chain_id,
                    record.record.endpoint,
                    if record.is_built_in {
                        "built-in"
                    } else {
                        "custom"
                    }
                );
                println!(
                    "  {} · {}",
                    record
                        .record
                        .api
                        .map(|api| api.as_str())
                        .unwrap_or("web link"),
                    record.record.capabilities.join(" · ")
                );
            }
        });
        out.emit(serde_json::json!({
            "catalog": true,
            "settingsGroups": spectra_core::chain_endpoints()?.into_iter()
                .filter(|row| chains.iter().any(|chain| chain.str_id() == row.chain_id))
                .flat_map(|row| row.grouped_settings)
                .filter(|group| chains.iter().any(|chain| chain.str_id() == group.chain_id))
                .map(|group| serde_json::json!({
                    "chainId": group.chain_id, "title": group.title, "endpoints": group.endpoints,
                })).collect::<Vec<_>>(),
            "configured": ctx.rt.block_on(service.configured_endpoints()).into_iter()
                .filter(|row| chains.iter().any(|chain| row.chain_id == chain.str_id()
                    || row.chain_id.starts_with(&format!("{}:", chain.str_id()))))
                .map(|row| serde_json::json!({"chainId": row.chain_id, "endpoints": row.endpoints}))
                .collect::<Vec<_>>(),
            "total": records.len(),
            "endpoints": records.iter().map(|r| serde_json::json!({
                "chainId": r.record.chain_id, "endpoint": r.record.endpoint,
                "api": r.record.api, "capabilities": r.record.capabilities, "isBuiltIn": r.is_built_in,
                "supportedCapabilities": r.record.api.map(|api| spectra_core::endpoint_capability_options(r.record.chain_id.clone(), api)).unwrap_or_default(),
            })).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    let service = ctx.service()?;

    let mut rows = Vec::new();
    let mut networks_without_apis = Vec::new();
    for chain in chains {
        let probes = ctx
            .rt
            .block_on(service.probe_chain_endpoints(chain.str_id().to_string()))?;
        if !probes.iter().any(|probe| probe.api.is_some()) {
            networks_without_apis.push(chain.str_id());
        }
        rows.extend(probes);
    }

    let unreachable = rows.iter().filter(|r| r.checked && !r.reachable).count();
    let unchecked = rows.iter().filter(|r| !r.checked).count();
    let unchecked_apis = rows
        .iter()
        .filter(|r| r.api.is_some() && !r.checked)
        .count();

    out.text(|| {
        println!();
        for r in &rows {
            let mark = if !r.checked {
                out::hint("?").to_string()
            } else if r.reachable {
                "✓".green().to_string()
            } else {
                "✗".red().to_string()
            };
            println!(
                "  {mark}  {:<18} {}",
                super::chain_name(&r.chain_id),
                r.endpoint
            );
            if r.checked && !r.reachable {
                println!("       {}", out::hint(&r.detail));
            }
        }
        for chain in &networks_without_apis {
            println!("  {chain}: no configured API endpoints");
        }
        println!();
        println!(
            "  {} reachable, {} unreachable, {} with no probe",
            rows.len() - unreachable - unchecked,
            unreachable,
            unchecked
        );
    });
    out.emit(serde_json::json!({
        "ok": unreachable == 0 && unchecked_apis == 0 && rows.len() > unchecked,
        "networksWithoutApis": networks_without_apis,
        "uncheckedApis": unchecked_apis,
        "total": rows.len(),
        "unreachable": unreachable,
        "unchecked": unchecked,
        "endpoints": rows.iter().map(|r| serde_json::json!({
            "chainId": r.chain_id, "chain": r.chain_id, "endpoint": r.endpoint,
            "api": r.api, "capabilities": r.capabilities,
            "checked": r.checked, "reachable": r.reachable, "detail": r.detail,
        })).collect::<Vec<_>>(),
    }));
    Ok(())
}

pub fn balance(ctx: &Ctx, out: Out, args: BalanceArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = resolve_chain(&wallet.chain_id)?.mainnet_counterpart();
    let service = service_for_chain(ctx, chain, ENDPOINT_CAPABILITY_BALANCE)?;

    let summary = ctx
        .rt
        .block_on(service.fetch_native_balance_summary(
            chain.str_id().to_string(),
            wallet_address(&wallet).to_string(),
        ))
        .map_err(CliError::from)?;

    out.text(|| {
        println!();
        println!(
            "  {}  {} {}",
            out::wallet_dot(&wallet.chain_id, wallet.is_watch_only()),
            summary.amount_display.bold(),
            out::tint(chain.coin_symbol(), &wallet.chain_id).bold(),
        );
        out::field("raw", &out::hint(&summary.smallest_unit).to_string());
        if summary.utxo_count > 0 {
            out::field("utxos", &summary.utxo_count.to_string());
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "wallet": wallet.id,
        "chain": chain.str_id(),
        "nativeSymbol": chain.coin_symbol(),
                "nativeDeploymentId": chain.entry().native_deployment_id,
                "family": chain.entry().family,
                "isTestnet": chain.is_testnet(),
        "amount": summary.amount_display,
        "smallestUnit": summary.smallest_unit,
        "utxoCount": summary.utxo_count,
    }));
    Ok(())
}

/// Fetch this wallet's history and merge it into the store, the way the app
/// does — one core operation that plans, fetches, builds the records and
/// merges. The listing above is a read; this is the write.
fn save_history(
    ctx: &Ctx,
    out: Out,
    service: &Arc<WalletService>,
    chain: Chain,
    wallet_id: &str,
    pages: u32,
    limit: usize,
) -> CliResult<()> {
    ctx.rt
        .block_on(service.open_state(ctx.db_path()))
        .map_err(CliError::from)?;
    service.set_secret_store(ctx.secrets.clone());
    let fetch = |load_more| -> Result<
        spectra_core::service::HistoryRefreshOutcome,
        spectra_core::SpectraBridgeError,
    > {
        let mut results = ctx.rt.block_on(service.refresh_history(
            spectra_core::service::HistoryRefreshScope::Wallets {
                wallet_ids: vec![wallet_id.into()],
            },
            load_more,
            Some(limit.min(100) as u32),
            0.0,
        ))?;
        let row = results
            .pop()
            .ok_or_else(|| spectra_core::SpectraBridgeError::InvalidInput {
                message: "No history wallet found".into(),
            })?;
        row.outcome
            .ok_or_else(|| spectra_core::SpectraBridgeError::from(row.error.unwrap_or_default()))
    };
    let mut outcome = fetch(false).map_err(CliError::from)?;
    let mut fetched_pages = 1;
    while fetched_pages < pages && !outcome.exhausted && outcome.wallets_failed == 0 {
        let next = fetch(true).map_err(CliError::from)?;
        outcome.added += next.added;
        outcome.updated += next.updated;
        outcome.wallets_failed += next.wallets_failed;
        outcome.exhausted = next.exhausted;
        fetched_pages += 1;
    }

    out.text(|| {
        println!();
        println!(
            "  {} {} added, {} updated",
            out::ok_mark(),
            outcome.added.to_string().bold(),
            outcome.updated.to_string().bold()
        );
        if outcome.wallets_failed > 0 {
            println!("  {}", out::hint("a provider did not answer"));
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "chain": chain.str_id(),
        "walletsRefreshed": outcome.wallets_refreshed,
        "walletsFailed": outcome.wallets_failed,
        "added": outcome.added,
        "updated": outcome.updated,
        "pages": fetched_pages,
        "exhausted": outcome.exhausted,
    }));
    Ok(())
}

pub fn history(ctx: &Ctx, out: Out, args: HistoryArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = resolve_chain(&wallet.chain_id)?.mainnet_counterpart();
    let network = wallet.chain().unwrap_or(chain);
    let service = if let Some(endpoint) = args.endpoint {
        WalletService::new(vec![ChainEndpoints {
            capabilities: vec![
                "balance",
                "history",
                "utxo",
                "fee",
                "broadcast",
                "verification",
                "token-balance",
                "token-discovery",
                "token-history",
                "staking",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            chain_id: network.str_id().into(),
            endpoints: vec![endpoint],
        }])
        .map_err(CliError::from)?
    } else {
        service_for_chain(
            ctx,
            network,
            ENDPOINT_CAPABILITY_HISTORY | ENDPOINT_CAPABILITY_BALANCE,
        )?
    };
    ctx.prepare_transport(&service)?;
    if args.save {
        return save_history(
            ctx, out, &service, chain, &wallet.id, args.pages, args.limit,
        );
    }

    let entries = ctx
        .rt
        .block_on(
            service.fetch_normalized_history(
                network.str_id().to_string(),
                wallet
                    .active_address()
                    .ok_or_else(|| CliError::rejected("wallet has no address on selected network"))?
                    .to_string(),
            ),
        )
        .map_err(CliError::from)?;

    out.text(|| {
        println!();
        if entries.is_empty() {
            println!("  {}", out::hint("no transactions"));
            return;
        }
        for entry in entries.iter().take(args.limit) {
            let incoming = entry.kind.eq_ignore_ascii_case("receive");
            let (mark, amount) = if incoming {
                (
                    "↓",
                    format!("{:>12.4}", entry.amount).truecolor(120, 230, 160),
                )
            } else {
                (
                    "↑",
                    format!("{:>12.4}", entry.amount).truecolor(255, 110, 130),
                )
            };
            println!(
                "  {}  {} {}  {}  {}",
                if incoming {
                    mark.truecolor(120, 230, 160).bold()
                } else {
                    mark.truecolor(255, 110, 130).bold()
                },
                amount.bold(),
                out::tint(&entry.symbol, &wallet.chain_id),
                out::info(&entry.counterparty),
                out::hint(&out::relative_time(entry.timestamp as i64)),
            );
            println!("     {}", out::hint(&out::short_hash(&entry.tx_hash)));
        }
        if entries.len() > args.limit {
            println!();
            println!(
                "  {}",
                out::hint(&format!("+{} more", entries.len() - args.limit))
            );
        }
    });
    out.emit(serde_json::json!({
        "ok": true,
        "wallet": wallet.id,
        "count": entries.len(),
        "transactions": entries
            .iter()
            .take(args.limit)
            .map(|entry| serde_json::json!({
                "hash": entry.tx_hash,
                "kind": entry.kind,
                "amount": entry.amount,
                "symbol": entry.symbol,
                "counterparty": entry.counterparty,
                "timestamp": entry.timestamp,
            }))
            .collect::<Vec<_>>(),
    }));
    Ok(())
}
