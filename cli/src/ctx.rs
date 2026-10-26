//! Process context: paths, secret input, and the one place `WalletService` is
//! opened.
//!
//! The CLI holds no wallet model of its own — `WalletState` and
//! `ResidentState` are core's, using the same SQLite schema as the app.

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::sync::Arc;

use spectra_core::api::endpoint::EndpointCapability;
use spectra_core::registry::Chain;
use spectra_core::secrets::backends::FileSecretStore;
use spectra_core::service::{ChainEndpoints, WalletService};
use spectra_core::store::state::{ResidentState, StateCommand, StateTransition, WalletState};

use crate::error::{CliError, CliResult};

/// How a command was told to read a secret.
///
/// A seed phrase or password never arrives as a command-line argument: `ps`
/// shows another user's arguments and the shell writes them to history, so an
/// argument is the one channel that leaks by default.
#[derive(Debug, Clone, Default)]
pub struct SecretSource {
    /// Read from this file. `-` means standard input.
    pub file: Option<String>,
    /// Read from this environment variable when it is set. The name is
    /// usually a default, so an unset variable falls through to the prompt
    /// rather than failing an interactive run.
    pub env: Option<String>,
}

impl SecretSource {
    /// Errors rather than prompting when stdin is not a terminal: a CLI
    /// blocked on a hidden prompt in CI is indistinguishable from one that
    /// hung.
    pub fn resolve(&self, prompt: &str, file_flag: &str) -> CliResult<String> {
        if let Some(path) = &self.file {
            return read_secret_file(path);
        }
        if let Some(name) = &self.env {
            match std::env::var(name) {
                Ok(value) => return Ok(value),
                Err(std::env::VarError::NotUnicode(_)) => {
                    return Err(CliError::usage(format!(
                        "environment variable {name} is not valid UTF-8"
                    )));
                }
                Err(std::env::VarError::NotPresent) => {}
            }
        }
        if std::io::stdin().is_terminal() {
            return rpassword::prompt_password(format!("  {prompt}: "))
                .map_err(|e| CliError::failure(format!("could not read {prompt}: {e}")));
        }
        let variable = self.env.as_deref().unwrap_or("its environment variable");
        Err(CliError::usage(format!(
            "no {prompt} available — pass --{file_flag} <path> (or `-` for stdin), \
             or set {variable}"
        )))
    }
}

/// Where a wallet's password comes from: one definition, flattened into every
/// command that opens a sealed wallet, so each reads it the same way.
#[derive(clap::Args, Debug, Clone)]
pub struct PasswordArgs {
    /// Read the wallet password from this file; `-` means stdin.
    #[arg(long, value_name = "PATH")]
    password_file: Option<String>,
    /// Read the wallet password from this environment variable.
    #[arg(long, value_name = "VAR", default_value = "SPECTRA_PASSWORD")]
    password_env: String,
}

impl PasswordArgs {
    /// The file, else the variable when it is set, else the prompt.
    pub fn resolve(&self) -> CliResult<String> {
        SecretSource {
            file: self.password_file.clone(),
            env: Some(self.password_env.clone()),
        }
        .resolve("password", "password-file")
    }

    /// The password `wallet` signs with, or `None` when it is not sealed
    /// with one: a wallet without a password is never asked for one.
    pub fn for_wallet(&self, wallet: &WalletState) -> CliResult<Option<String>> {
        if wallet.signing.requires_password() {
            self.resolve().map(Some)
        } else {
            Ok(None)
        }
    }
}

fn read_secret_file(path: &str) -> CliResult<String> {
    let raw = if path == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|e| CliError::failure(format!("could not read stdin: {e}")))?;
        buffer
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| CliError::failure(format!("could not read {path}: {e}")))?
    };
    Ok(raw.trim().to_string())
}

pub struct Ctx {
    pub rt: tokio::runtime::Runtime,
    pub data_dir: PathBuf,
    pub secrets: Arc<FileSecretStore>,
}

impl Ctx {
    /// `--data-dir`, else `SPECTRA_DATA_DIR`, else `~/.spectra`. Overridable
    /// so the acceptance script never touches the user's real wallets.
    pub fn new(data_dir: Option<PathBuf>) -> CliResult<Self> {
        let data_dir = data_dir
            .or_else(|| std::env::var_os("SPECTRA_DATA_DIR").map(PathBuf::from))
            .or_else(|| dirs::home_dir().map(|home| home.join(".spectra")))
            .ok_or_else(|| CliError::failure("cannot determine a data directory"))?;

        // SQLite needs the parent directory before the first read, so a fresh
        // install must not fail on `list`.
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| CliError::failure(format!("cannot create {}: {e}", data_dir.display())))?;
        let secrets = FileSecretStore::new(data_dir.join("secrets"))
            .map_err(|e| CliError::failure(format!("cannot open secret store: {e}")))?;

        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| CliError::failure(format!("cannot start async runtime: {e}")))?;

        Ok(Self {
            rt,
            data_dir,
            secrets: Arc::new(secrets),
        })
    }

    pub fn db_path(&self) -> String {
        self.data_dir
            .join("spectra.sqlite")
            .to_string_lossy()
            .into_owned()
    }

    /// The service every network command uses: its secret store, the opened
    /// store and a ready network transport, Tor included when it is on.
    ///
    /// Opened fresh per command on purpose: a short-lived process never holds
    /// a snapshot old enough to overwrite a newer store with.
    pub fn service(&self) -> CliResult<Arc<WalletService>> {
        let service = self.offline_service()?;
        self.rt.block_on(service.configure_network_runtime(
            self.data_dir.join("cache").to_string_lossy().into_owned(),
        ))?;
        self.rt.block_on(service.await_network_ready())?;
        Ok(service)
    }

    /// [`Ctx::service`], reading `chain` only through `endpoints` when the
    /// command named any.
    pub fn service_with_endpoints(
        &self,
        chain: Chain,
        endpoints: Vec<String>,
    ) -> CliResult<Arc<WalletService>> {
        let service = self.service()?;
        if !endpoints.is_empty() {
            self.rt
                .block_on(service.update_endpoints(vec![ChainEndpoints {
                    capabilities: EndpointCapability::ALL.to_vec(),
                    chain_id: chain,
                    endpoints,
                }]))?;
        }
        Ok(service)
    }

    /// The opened store without the network: editing stored settings must
    /// work offline, including turning Tor off after a failed bootstrap.
    pub fn offline_service(&self) -> CliResult<Arc<WalletService>> {
        let service = self.unopened_service()?;
        self.rt.block_on(service.open_state(self.db_path()))?;
        Ok(service)
    }

    /// The service with its secret store but no database: for discarding a
    /// store `open_state` cannot read.
    pub fn unopened_service(&self) -> CliResult<Arc<WalletService>> {
        let service = WalletService::new_catalog()?;
        service.set_secret_store(self.secrets.clone());
        Ok(service)
    }

    pub fn state(&self) -> CliResult<ResidentState> {
        let service = self.unopened_service()?;
        Ok(self.rt.block_on(service.open_state(self.db_path()))?)
    }

    pub fn apply(&self, command: StateCommand) -> CliResult<StateTransition> {
        let service = self.offline_service()?;
        Ok(self.rt.block_on(service.apply_state_command(command))?)
    }

    /// By argument, never an interactive picker: a command that needs a TTY
    /// cannot be a test.
    pub fn find_wallet(&self, needle: &str) -> CliResult<WalletState> {
        let state = self.state()?;
        let matches: Vec<&WalletState> = state
            .wallets
            .iter()
            .filter(|w| wallet_matches(w, needle))
            .collect();
        match matches.as_slice() {
            [] => Err(CliError::failure(format!("no wallet matching {needle:?}"))),
            [one] => Ok((*one).clone()),
            many => {
                let names: Vec<&str> = many.iter().map(|w| w.name.as_str()).collect();
                Err(CliError::usage(format!(
                    "{needle:?} matches {} wallets ({}) — use the wallet id",
                    many.len(),
                    names.join(", ")
                )))
            }
        }
    }
}

fn wallet_matches(wallet: &WalletState, needle: &str) -> bool {
    wallet.id.eq_ignore_ascii_case(needle)
        || wallet.name.eq_ignore_ascii_case(needle)
        || wallet
            .primary_address()
            .is_some_and(|address| address.eq_ignore_ascii_case(needle))
}

/// Wallets carry an address by construction; the fallback keeps a malformed
/// record printable rather than panicking mid-listing.
pub fn wallet_address(wallet: &WalletState) -> &str {
    wallet.active_address().unwrap_or("")
}
