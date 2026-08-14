//! Process context: where data lives, how secrets are read, and the one place
//! `WalletService` is opened.
//!
//! The CLI holds no wallet model of its own. A wallet is a `WalletSummary`, the
//! resident state is a `CoreAppState`, and both are core's — persisted by
//! `wallet_db` into the same SQLite store the iOS app uses. That is the point
//! of this crate existing: a core API that can only be driven from Swift is not
//! core yet.

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::sync::Arc;

use spectra_core::service::WalletService;
use spectra_core::store::secret_backends::FileSecretStore;
use spectra_core::store::state::{CoreAppState, StateCommand, StateTransition, WalletSummary};

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
    /// Read from this environment variable.
    pub env: Option<String>,
}

impl SecretSource {
    /// Resolve the secret, prompting on a terminal when no source was given.
    ///
    /// `prompt` is only ever shown interactively; a scripted run that reaches
    /// the prompt gets an error instead, because a CLI that blocks on a hidden
    /// prompt in CI is indistinguishable from one that hung.
    pub fn resolve(&self, prompt: &str) -> CliResult<String> {
        if let Some(path) = &self.file {
            return read_secret_file(path);
        }
        if let Some(name) = &self.env {
            return std::env::var(name).map_err(|_| {
                CliError::usage(format!("environment variable {name} is not set"))
            });
        }
        if std::io::stdin().is_terminal() {
            return rpassword::prompt_password(format!("  {prompt}: "))
                .map_err(|e| CliError::failure(format!("could not read {prompt}: {e}")));
        }
        Err(CliError::usage(format!(
            "no {prompt} available — pass --{}-file <path> (or `-` for stdin), \
             or set the matching environment variable",
            prompt.replace(' ', "-")
        )))
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

/// Everything a command needs that is not its own arguments.
pub struct Ctx {
    pub rt: tokio::runtime::Runtime,
    pub data_dir: PathBuf,
    pub secrets: Arc<FileSecretStore>,
}

impl Ctx {
    /// `--data-dir`, else `SPECTRA_DATA_DIR`, else `~/.spectra`.
    ///
    /// Overridable because the acceptance script has to run against a scratch
    /// directory. A test suite that can only run against the user's real
    /// wallets is one nobody runs.
    pub fn new(data_dir: Option<PathBuf>) -> CliResult<Self> {
        let data_dir = data_dir
            .or_else(|| std::env::var_os("SPECTRA_DATA_DIR").map(PathBuf::from))
            .or_else(|| dirs::home_dir().map(|home| home.join(".spectra")))
            .ok_or_else(|| CliError::failure("cannot determine a data directory"))?;

        // `wallet_db` opens a SQLite file and needs its parent to exist before
        // the first read, so a fresh install must not fail on `list` before
        // anything has ever been imported.
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

    /// A service with no chain endpoints, for state and off-chain work.
    ///
    /// Opened fresh per command on purpose: a CLI process is short-lived, and
    /// reopening means it never holds a snapshot old enough to overwrite a
    /// newer store with.
    pub fn service(&self) -> CliResult<Arc<WalletService>> {
        let service = WalletService::new_typed(Vec::new()).map_err(CliError::from)?;
        self.rt
            .block_on(service.open_state(self.db_path()))
            .map_err(CliError::from)?;
        Ok(service)
    }

    /// The resident domain state, as core holds it.
    pub fn state(&self) -> CliResult<CoreAppState> {
        let service = WalletService::new_typed(Vec::new()).map_err(CliError::from)?;
        self.rt
            .block_on(service.open_state(self.db_path()))
            .map_err(CliError::from)
    }

    /// Send one command to core and return what it decided.
    pub fn apply(&self, command: StateCommand) -> CliResult<StateTransition> {
        let service = self.service()?;
        self.rt
            .block_on(service.apply_state_command(command))
            .map_err(CliError::from)
    }

    /// Resolve a wallet by id, or by an unambiguous name or address prefix.
    ///
    /// Selection is by argument, never by an interactive picker: a picker
    /// needs a TTY, and a command that needs a TTY cannot be a test.
    pub fn find_wallet(&self, needle: &str) -> CliResult<WalletSummary> {
        let state = self.state()?;
        let matches: Vec<&WalletSummary> = state
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

fn wallet_matches(wallet: &WalletSummary, needle: &str) -> bool {
    wallet.id.eq_ignore_ascii_case(needle)
        || wallet.name.eq_ignore_ascii_case(needle)
        || wallet
            .primary_address()
            .is_some_and(|address| address.eq_ignore_ascii_case(needle))
}

/// The address the CLI shows for a wallet. Wallets carry one by construction;
/// the fallback keeps a malformed record printable rather than panicking
/// mid-listing.
pub fn wallet_address(wallet: &WalletSummary) -> &str {
    wallet.primary_address().unwrap_or("")
}
