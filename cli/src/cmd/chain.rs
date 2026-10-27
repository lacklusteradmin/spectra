//! Commands about networks: the supported list, their endpoints, explorers
//! and donation addresses, and a wallet's balance and history read from its
//! network.
//!
//! Endpoint selection is core's — `endpoints::records_for_chain` picks
//! them from the catalog by API and capability. The CLI names the capabilities
//! it is about to use and nothing else.

use std::sync::Arc;

use clap::Args;
use colored::Colorize as _;
use serde::Serialize;
use spectra_core::api::endpoint::EndpointApi;
use spectra_core::catalog::chains::{ChainTag, ChainWikiEntry};
use spectra_core::derivation::setup::WalletSetupMethod;
use spectra_core::registry::{Chain, TokenStandard};
use spectra_core::service::{CustomEndpoint, WalletService};
use spectra_core::store::state::{AppSettingUpdate, StateCommand, StateEvent};
use spectra_core::store::wallet_domain::TransactionKind;

use super::resolve_chain;
use crate::ctx::{Ctx, WalletArg, wallet_address};
use crate::error::{CliError, CliResult};
use crate::out::{self, Out, Report};

pub use spectra_core::api::endpoint::EndpointCapability;

#[derive(Args)]
pub struct ChainsArgs {
    /// Only chains whose name or symbol contains this text.
    #[arg(long)]
    filter: Option<String>,
    /// Include testnets.
    #[arg(long)]
    testnets: bool,
    /// Only chains with this picker tag (layer-1, evm, utxo, move, …).
    #[arg(long, value_parser = parse_tag)]
    tag: Option<ChainTag>,
    /// Show mainnet wiki descriptions with the same catalog tags.
    #[arg(long, conflicts_with = "testnets")]
    wiki: bool,
}

fn parse_tag(value: &str) -> Result<ChainTag, String> {
    ChainTag::parse(value).ok_or_else(|| {
        let known: Vec<&str> = ChainTag::ALL.iter().map(|tag| tag.as_str()).collect();
        format!("unknown tag; expected one of {}", known.join(", "))
    })
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
    /// Continue the saved provider cursor, including after a process restart.
    #[arg(long, requires = "save")]
    load_more: bool,
    /// Maximum history pages to merge in this session.
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
    any_of: &[EndpointCapability],
) -> CliResult<Arc<WalletService>> {
    let service = ctx.service()?;
    let records = spectra_core::catalog::endpoints::records_for_chain(chain, any_of);
    if !records
        .iter()
        .any(|row| chain.endpoint_apis().contains(&row.api))
    {
        return Err(CliError::failure(format!(
            "no compatible endpoints registered for {}",
            chain.chain_display_name()
        )));
    }
    Ok(service)
}

// ─── chains ─────────────────────────────────────────────────────────────────

/// A supported network, with the registry facts a picker and a script use.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChainRow {
    id: &'static str,
    name: &'static str,
    native_symbol: &'static str,
    native_deployment_id: String,
    mainnet: Chain,
    is_testnet: bool,
    faucet: Option<&'static str>,
    is_evm: bool,
    token_standards: Vec<TokenStandard>,
    /// The ways a wallet can be added on the network: the setup descriptor
    /// the app's network page and `wallet methods` read.
    setup_methods: Vec<WalletSetupMethod>,
    /// Whether the staking tab offers this chain.
    staking: bool,
    supports_separate_signing: bool,
    /// The picker's popular order and filters.
    popular_rank: u16,
    tags: Vec<&'static str>,
    /// Explorer history source for EVM chains only. Other families use
    /// different history routes.
    history_source: Option<&'static str>,
    #[serde(skip)]
    chain: Chain,
}

impl From<Chain> for ChainRow {
    fn from(chain: Chain) -> Self {
        let entry = chain.entry();
        Self {
            id: chain.str_id(),
            name: chain.chain_display_name(),
            native_symbol: chain.coin_symbol(),
            native_deployment_id: entry.native_deployment_id.clone(),
            mainnet: chain.mainnet_counterpart(),
            is_testnet: chain.is_testnet(),
            faucet: chain.faucet_url(),
            is_evm: chain.is_evm(),
            token_standards: chain.token_standards().to_vec(),
            setup_methods: spectra_core::derivation::setup::wallet_setup_descriptor(chain)
                .options
                .iter()
                .map(|option| option.method)
                .collect(),
            staking: chain.supports_staking(),
            supports_separate_signing: chain.supports_sign_only(),
            popular_rank: entry.popular_rank,
            tags: entry.tags.iter().map(|tag| tag.as_str()).collect(),
            history_source: chain.is_evm().then(|| match chain.evm_history_source() {
                spectra_core::registry::EvmHistorySource::Open(base) => base,
                spectra_core::registry::EvmHistorySource::Unavailable => "none",
            }),
            chain,
        }
    }
}

#[derive(Serialize)]
struct ChainsReport {
    chains: Vec<ChainRow>,
}

impl Report for ChainsReport {
    fn render(&self) {
        println!();
        for row in &self.chains {
            println!(
                "  {}  {:<22} {:<8} {}",
                out::tint("●", row.chain).bold(),
                out::chain(row.chain),
                row.native_symbol,
                out::hint(row.id),
            );
        }
        println!();
        println!("  {}", out::count(self.chains.len(), "chain"));
    }
}

#[derive(Serialize)]
struct WikiReport {
    chains: Vec<ChainWikiEntry>,
}

impl Report for WikiReport {
    fn render(&self) {
        for row in &self.chains {
            let tags: Vec<&str> = row.tags.iter().map(|tag| tag.as_str()).collect();
            println!("  {}  {}", out::accent(&row.name).bold(), tags.join(" · "));
            println!("    {}", row.comment);
        }
    }
}

pub fn chains(out: Out, args: ChainsArgs) -> CliResult<()> {
    let needle = args.filter.as_deref().map(str::to_lowercase);
    // The picker's popular order; a testnet shares its mainnet's rank and
    // follows it in catalog order.
    let mut listed: Vec<Chain> = Chain::all()
        .filter(|chain| args.testnets || chain.mainnet_counterpart() == *chain)
        .filter(|chain| args.tag.is_none_or(|tag| chain.entry().tags.contains(&tag)))
        .filter(|chain| match &needle {
            None => true,
            Some(needle) => {
                chain.chain_display_name().to_lowercase().contains(needle)
                    || chain.coin_symbol().to_lowercase().contains(needle)
            }
        })
        .collect();
    listed.sort_by_key(|chain| chain.entry().popular_rank);

    if args.wiki {
        let chains = spectra_core::catalog::chains::list_chain_wiki()
            .into_iter()
            .filter(|row| listed.iter().any(|chain| chain.str_id() == row.id))
            .collect();
        return out.report(&WikiReport { chains });
    }
    out.report(&ChainsReport {
        chains: listed.into_iter().map(ChainRow::from).collect(),
    })
}

// ─── endpoints ──────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct EndpointsArgs {
    /// Chain to probe. Omit to probe every chain the catalog knows.
    #[arg(long)]
    chain: Option<String>,
    /// List registered endpoints and capabilities offline, without health probes.
    #[arg(long)]
    catalog: bool,
    /// Save a custom endpoint offline. Requires --chain, --api and --capabilities.
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
    /// Use only your endpoints on this network (`true`), so the catalog's
    /// are never contacted, or the catalog's too (`false`). Requires --chain.
    #[arg(long, requires = "chain", conflicts_with_all = ["catalog", "add"], action = clap::ArgAction::Set)]
    custom_only: Option<bool>,
    /// Remove one of your endpoints from this network. Removing its last one
    /// also stops using only yours there. Requires --chain.
    #[arg(long, requires = "chain", conflicts_with_all = ["catalog", "add", "custom_only"])]
    remove: Option<String>,
    /// Edit one of your endpoints in place: the saved URL to change, with
    /// --add naming what it becomes, and its --api and --capabilities.
    #[arg(long, requires = "add")]
    replace: Option<String>,
}

/// The endpoints of yours that are stored, after a change to them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CustomEndpointsReport {
    custom_endpoints: Vec<CustomEndpoint>,
    custom_endpoints_only: Vec<Chain>,
}

impl Report for CustomEndpointsReport {
    fn render(&self) {
        println!();
        if self.custom_endpoints.is_empty() {
            println!("  {}", out::hint("no endpoints of yours"));
        }
        for endpoint in &self.custom_endpoints {
            let only = if self.custom_endpoints_only.contains(&endpoint.chain_id) {
                "  (only yours)"
            } else {
                ""
            };
            println!(
                "  {}  {}  {}{only}",
                out::chain(endpoint.chain_id),
                endpoint.endpoint,
                out::hint(endpoint.api.as_str())
            );
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogRow {
    chain: Chain,
    endpoint: String,
    api: EndpointApi,
    capabilities: Vec<EndpointCapability>,
    is_built_in: bool,
    supported_capabilities: Vec<EndpointCapability>,
}

#[derive(Serialize)]
struct SettingsGroupRow {
    chain: Chain,
    title: String,
    endpoints: Vec<String>,
}

#[derive(Serialize)]
struct ConfiguredRow {
    chain: Chain,
    endpoints: Vec<String>,
}

/// The endpoint catalog, offline: what is registered, how the settings page
/// groups it, and what each network is configured to use.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogReport {
    settings_groups: Vec<SettingsGroupRow>,
    configured: Vec<ConfiguredRow>,
    total: usize,
    endpoints: Vec<CatalogRow>,
}

impl Report for CatalogReport {
    fn render(&self) {
        for row in &self.endpoints {
            println!(
                "  {}  {}  {}",
                out::chain(row.chain),
                row.endpoint,
                out::hint(if row.is_built_in {
                    "built-in"
                } else {
                    "custom"
                })
            );
            let capabilities: Vec<&str> = row.capabilities.iter().map(|c| c.as_str()).collect();
            println!(
                "     {} · {}",
                row.api.as_str(),
                out::hint(&capabilities.join(" · "))
            );
        }
        println!();
        println!("  {}", out::count(self.total, "endpoint"));
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeRow {
    chain: Chain,
    endpoint: String,
    api: EndpointApi,
    capabilities: Vec<EndpointCapability>,
    checked: bool,
    reachable: bool,
    detail: String,
}

/// Which endpoints answered core's health probe.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthReport {
    networks_without_apis: Vec<&'static str>,
    unchecked_apis: usize,
    total: usize,
    unreachable: usize,
    endpoints: Vec<ProbeRow>,
}

impl Report for HealthReport {
    fn render(&self) {
        println!();
        for row in &self.endpoints {
            let mark = if !row.checked {
                out::hint("?")
            } else if row.reachable {
                out::ok_mark()
            } else {
                out::fail_mark()
            };
            println!(
                "  {mark}  {:<18} {}",
                row.chain.chain_display_name(),
                row.endpoint
            );
            if row.checked && !row.reachable {
                println!("       {}", out::hint(&row.detail));
            }
        }
        for chain in &self.networks_without_apis {
            println!(
                "  {}  {chain}: no configured API endpoints",
                out::fail_mark()
            );
        }
        println!();
        println!(
            "  {} reachable, {} unreachable, {} with no probe",
            self.total - self.unreachable - self.unchecked_apis,
            self.unreachable,
            self.unchecked_apis
        );
    }

    fn ok(&self) -> bool {
        self.unreachable == 0
            && self.unchecked_apis == 0
            && self.networks_without_apis.is_empty()
            && self.total > 0
    }
}

/// Check read methods for every API, including testnets and history indexers.
/// Networks with no API at all are reported separately from unchecked ones.
pub fn endpoints(ctx: &Ctx, out: Out, args: EndpointsArgs) -> CliResult<()> {
    let chains: Vec<Chain> = match &args.chain {
        Some(name) => vec![resolve_chain(name)?],
        None => Chain::all().collect(),
    };
    let update = if let Some(value) = args.custom_only {
        Some(AppSettingUpdate::CustomEndpointsOnly {
            chain_id: chains[0],
            value,
        })
    } else if let Some(endpoint) = args.remove {
        Some(AppSettingUpdate::RemoveCustomEndpoint {
            chain_id: chains[0],
            endpoint,
        })
    } else if let Some(url) = args.add {
        let capabilities = args
            .capabilities
            .iter()
            .map(|name| {
                name.parse()
                    .map_err(|e: spectra_core::error::SpectraBridgeError| {
                        CliError::usage(e.to_string())
                    })
            })
            .collect::<CliResult<_>>()?;
        let api = args.api.unwrap_or_default();
        Some(match args.replace {
            Some(saved) => AppSettingUpdate::ReplaceCustomEndpoint {
                chain_id: chains[0],
                endpoint: saved,
                api,
                new_endpoint: url,
                capabilities,
            },
            None => AppSettingUpdate::AddCustomEndpoint {
                chain_id: chains[0],
                api,
                endpoint: url,
                capabilities,
            },
        })
    } else {
        None
    };
    if let Some(update) = update {
        let transition = ctx.apply(StateCommand::SetAppSetting { update })?;
        if transition.events.contains(&StateEvent::AppSettingRejected) {
            return Err(CliError::rejected(
                "Invalid or duplicate endpoint, or unsupported/empty capabilities for this adapter",
            ));
        }
        let settings = transition.state.settings;
        return out.report(&CustomEndpointsReport {
            custom_endpoints: settings.custom_endpoints,
            custom_endpoints_only: settings.custom_endpoints_only,
        });
    }
    if args.catalog {
        return catalog(ctx, out, &chains, args.chain.is_some(), args.source);
    }

    let service = ctx.service()?;
    // Probe each network once however many times the request names it.
    let mut probed = std::collections::HashSet::new();
    let mut endpoints = Vec::new();
    let mut networks_without_apis = Vec::new();
    for network in chains {
        if !probed.insert(network) {
            continue;
        }
        let probes = ctx.rt.block_on(service.probe_chain_endpoints(network))?;
        if probes.is_empty() {
            networks_without_apis.push(network.str_id());
        }
        endpoints.extend(probes.into_iter().map(|probe| ProbeRow {
            chain: probe.chain_id,
            endpoint: probe.endpoint,
            api: probe.api,
            capabilities: probe.capabilities,
            checked: probe.checked,
            reachable: probe.reachable,
            detail: probe.detail,
        }));
    }
    out.report(&HealthReport {
        networks_without_apis,
        unchecked_apis: endpoints.iter().filter(|r| !r.checked).count(),
        total: endpoints.len(),
        unreachable: endpoints
            .iter()
            .filter(|r| r.checked && !r.reachable)
            .count(),
        endpoints,
    })
}

fn catalog(
    ctx: &Ctx,
    out: Out,
    chains: &[Chain],
    named: bool,
    source: Option<String>,
) -> CliResult<()> {
    let service = ctx.offline_service()?;
    let endpoints: Vec<CatalogRow> = ctx
        .rt
        .block_on(service.endpoint_directory())?
        .into_iter()
        .filter(|entry| {
            (!named || chains.contains(&entry.record.chain_id))
                && source
                    .as_deref()
                    .is_none_or(|source| entry.is_built_in == (source == "built-in"))
        })
        .map(|entry| CatalogRow {
            chain: entry.record.chain_id,
            supported_capabilities: spectra_core::api::endpoint::endpoint_capability_options(
                entry.record.chain_id,
                entry.record.api,
            ),
            endpoint: entry.record.endpoint,
            api: entry.record.api,
            capabilities: entry.record.capabilities,
            is_built_in: entry.is_built_in,
        })
        .collect();
    out.report(&CatalogReport {
        settings_groups: spectra_core::catalog::endpoints::endpoint_settings()
            .into_iter()
            .filter(|row| chains.contains(&row.chain_id))
            .flat_map(|row| row.groups)
            .filter(|group| chains.contains(&group.chain_id))
            .map(|group| SettingsGroupRow {
                chain: group.chain_id,
                title: group.title,
                endpoints: group.endpoints,
            })
            .collect(),
        configured: ctx
            .rt
            .block_on(service.configured_endpoints())
            .into_iter()
            .filter(|row| chains.contains(&row.chain_id))
            .map(|row| ConfiguredRow {
                chain: row.chain_id,
                endpoints: row.endpoints,
            })
            .collect(),
        total: endpoints.len(),
        endpoints,
    })
}

// ─── explorers and donations ────────────────────────────────────────────────

#[derive(Args)]
pub struct ExplorersArgs {
    /// Only this chain's explorer.
    #[arg(long)]
    chain: Option<String>,
    /// Print the explorer page for this transaction hash.
    #[arg(long, requires = "chain", conflicts_with = "address")]
    tx: Option<String>,
    /// Print the explorer page for this address.
    #[arg(long, requires = "chain")]
    address: Option<String>,
}

#[derive(Serialize)]
struct LinkReport {
    chain: Chain,
    url: String,
}

impl Report for LinkReport {
    fn render(&self) {
        println!("{}", self.url);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExplorerRow {
    chain: Chain,
    name: String,
    tx_url: String,
    address_url: Option<String>,
}

#[derive(Serialize)]
struct ExplorersReport {
    explorers: Vec<ExplorerRow>,
}

impl Report for ExplorersReport {
    fn render(&self) {
        println!();
        for row in &self.explorers {
            println!(
                "  {:<22} {:<24} {}",
                row.chain.chain_display_name(),
                row.name,
                out::hint(&row.tx_url)
            );
        }
    }
}

/// The pages a transaction's detail screen and a wallet's page link to.
/// Nothing is requested.
pub fn explorers(out: Out, args: ExplorersArgs) -> CliResult<()> {
    let chain = args.chain.as_deref().map(resolve_chain).transpose()?;
    if let Some(chain) = chain
        && (args.tx.is_some() || args.address.is_some())
    {
        let Some(explorer) = chain.explorer() else {
            return Err(CliError::failure(format!(
                "{} has no explorer",
                chain.chain_display_name()
            )));
        };
        let link = match (args.tx, args.address) {
            (Some(hash), _) => {
                spectra_core::catalog::explorers::transaction_explorer_link(chain, hash)
                    .ok_or_else(|| CliError::usage("--tx needs a transaction hash"))?
            }
            (_, Some(address)) => {
                if explorer.address_url.is_none() {
                    return Err(CliError::failure(format!(
                        "{}'s explorer has no address page",
                        chain.chain_display_name()
                    )));
                }
                spectra_core::catalog::explorers::address_explorer_link(chain, address)
                    .ok_or_else(|| CliError::usage("--address needs an address"))?
            }
            (None, None) => unreachable!("one of --tx and --address is given"),
        };
        return out.report(&LinkReport {
            chain,
            url: link.url,
        });
    }
    out.report(&ExplorersReport {
        explorers: spectra_core::catalog::explorers::explorers()
            .into_iter()
            .filter(|e| chain.is_none_or(|chain| chain == e.chain_id))
            .map(|e| ExplorerRow {
                chain: e.chain_id,
                name: e.name,
                tx_url: e.tx_url,
                address_url: e.address_url,
            })
            .collect(),
    })
}

#[derive(Serialize)]
struct DonationRow {
    chain: Chain,
    address: String,
}

#[derive(Serialize)]
struct DonationsReport {
    donations: Vec<DonationRow>,
}

impl Report for DonationsReport {
    fn render(&self) {
        println!();
        for row in &self.donations {
            println!("  {:<12} {}", row.chain.chain_display_name(), row.address);
        }
    }
}

/// The Donate screen's addresses, as core validated them.
pub fn donations(out: Out) -> CliResult<()> {
    out.report(&DonationsReport {
        donations: spectra_core::catalog::donations::donation_destinations()
            .into_iter()
            .map(|d| DonationRow {
                chain: d.chain_id,
                address: d.address,
            })
            .collect(),
    })
}

// ─── balance and history ────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BalanceReport {
    wallet: String,
    chain: Chain,
    native_symbol: &'static str,
    amount: String,
    smallest_unit: String,
    #[serde(skip)]
    is_watch_only: bool,
}

impl Report for BalanceReport {
    fn render(&self) {
        println!();
        println!(
            "  {}  {} {}",
            out::wallet_dot(self.chain, self.is_watch_only),
            self.amount.bold(),
            out::tint(self.native_symbol, self.chain).bold(),
        );
        out::field("raw", &out::hint(&self.smallest_unit).to_string());
    }
}

pub fn balance(ctx: &Ctx, out: Out, args: WalletArg) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let chain = wallet.chain_id;
    let service = service_for_chain(ctx, chain, &[EndpointCapability::Balance])?;

    let summary = if chain.uses_account_utxo() {
        let updated = ctx
            .rt
            .block_on(service.refresh_wallet_balances(wallet.id.clone()))?;
        let native = updated
            .holdings
            .iter()
            .find(|holding| holding.chain_id == chain && holding.is_native())
            .ok_or_else(|| CliError::failure("wallet has no native balance"))?;
        spectra_core::service::NativeBalanceSummary {
            amount_display: native.amount.clone(),
            smallest_unit: spectra_core::amount::decimal::to_units(
                &native.amount,
                u32::from(chain.native_decimals()),
            )
            .ok_or_else(|| CliError::failure("invalid native balance"))?
            .to_string(),
        }
    } else {
        ctx.rt.block_on(
            service.fetch_native_balance_summary(chain, wallet_address(&wallet).to_string()),
        )?
    };
    out.report(&BalanceReport {
        wallet: wallet.id.clone(),
        chain,
        native_symbol: chain.coin_symbol(),
        amount: summary.amount_display,
        smallest_unit: summary.smallest_unit,
        is_watch_only: wallet.is_watch_only(),
    })
}

/// What one save of a wallet's history merged into the store.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedHistoryReport {
    chain: Chain,
    wallets_refreshed: u32,
    wallets_failed: u32,
    added: u32,
    updated: u32,
    pages: u32,
    exhausted: bool,
}

impl Report for SavedHistoryReport {
    fn render(&self) {
        println!();
        println!(
            "  {} {} added, {} updated",
            out::ok_mark(),
            self.added.to_string().bold(),
            self.updated.to_string().bold()
        );
        if self.wallets_failed > 0 {
            println!("  {}", out::hint("a provider did not answer"));
        }
    }
}

/// Fetch this wallet's history and merge it into the store, the way the app
/// does — one core operation that plans, fetches, builds the records and
/// merges. The listing below is a read; this is the write.
fn save_history(
    ctx: &Ctx,
    out: Out,
    service: &Arc<WalletService>,
    chain: Chain,
    wallet_id: &str,
    args: &HistoryArgs,
) -> CliResult<()> {
    let fetch = |load_more| -> CliResult<spectra_core::service::HistoryRefreshOutcome> {
        let mut results = ctx.rt.block_on(service.refresh_history(
            spectra_core::service::HistoryRefreshScope::Wallets {
                wallet_ids: vec![wallet_id.into()],
            },
            load_more,
            Some(args.limit.min(100) as u32),
            0.0,
        ))?;
        let row = results
            .pop()
            .ok_or_else(|| CliError::rejected("No history wallet found"))?;
        row.outcome
            .ok_or_else(|| CliError::failure(row.error.unwrap_or_default()))
    };
    let mut outcome = fetch(args.load_more)?;
    let mut pages = 1;
    while pages < args.pages && !outcome.exhausted && outcome.wallets_failed == 0 {
        let next = fetch(true)?;
        outcome.added += next.added;
        outcome.updated += next.updated;
        outcome.wallets_failed += next.wallets_failed;
        outcome.exhausted = next.exhausted;
        pages += 1;
    }
    out.report(&SavedHistoryReport {
        chain,
        wallets_refreshed: outcome.wallets_refreshed,
        wallets_failed: outcome.wallets_failed,
        added: outcome.added,
        updated: outcome.updated,
        pages,
        exhausted: outcome.exhausted,
    })
}

#[derive(Serialize)]
struct HistoryRow {
    hash: String,
    kind: TransactionKind,
    amount: String,
    symbol: String,
    counterparty: String,
    timestamp: f64,
}

/// A wallet's history as its network reports it, newest first.
#[derive(Serialize)]
struct HistoryReport {
    wallet: String,
    count: usize,
    transactions: Vec<HistoryRow>,
    #[serde(skip)]
    chain: Chain,
}

impl Report for HistoryReport {
    fn render(&self) {
        println!();
        if self.transactions.is_empty() {
            println!("  {}", out::hint("no transactions"));
            return;
        }
        for row in &self.transactions {
            let mark = if row.kind == TransactionKind::Receive {
                out::up("↓")
            } else {
                out::down("↑")
            };
            println!(
                "  {}  {:>14} {}  {}  {}",
                mark.bold(),
                row.amount,
                out::tint(&row.symbol, self.chain),
                out::info(&row.counterparty),
                out::hint(&out::relative_time(row.timestamp as i64)),
            );
            println!("     {}", out::hint(&out::short_hash(&row.hash)));
        }
        if self.count > self.transactions.len() {
            println!();
            println!(
                "  {}",
                out::hint(&format!("+{} more", self.count - self.transactions.len()))
            );
        }
    }
}

pub fn history(ctx: &Ctx, out: Out, args: HistoryArgs) -> CliResult<()> {
    let wallet = ctx.find_wallet(&args.wallet)?;
    let network = wallet.chain_id;
    let service = if let Some(endpoint) = &args.endpoint {
        ctx.service_with_endpoints(network, vec![endpoint.clone()])?
    } else {
        service_for_chain(
            ctx,
            network,
            &[EndpointCapability::History, EndpointCapability::Balance],
        )?
    };
    if args.save {
        return save_history(
            ctx,
            out,
            &service,
            network.mainnet_counterpart(),
            &wallet.id,
            &args,
        );
    }
    let address = wallet
        .active_address()
        .ok_or_else(|| CliError::rejected("wallet has no address on selected network"))?
        .to_string();
    let entries = ctx
        .rt
        .block_on(service.fetch_normalized_history(network, address))?;
    out.report(&HistoryReport {
        wallet: wallet.id.clone(),
        count: entries.len(),
        transactions: entries
            .into_iter()
            .take(args.limit)
            .map(|entry| HistoryRow {
                hash: entry.tx_hash,
                kind: entry.kind,
                amount: entry.amount,
                symbol: entry.symbol,
                counterparty: entry.counterparty,
                timestamp: entry.timestamp,
            })
            .collect(),
        chain: network,
    })
}
