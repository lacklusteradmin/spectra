use clap::{Parser, Subcommand};
use std::collections::BTreeMap;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "spectra", about = "Primitive CLI over the Spectra Rust core", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Derive addresses for a set of chains from a seed phrase.
    /// Uses canonical BIP44-style paths for each chain.
    Addresses {
        /// BIP39 seed phrase (quote it so the shell keeps it as a single arg).
        seed: String,
        /// Optional chain filter (comma-separated). Defaults to: Bitcoin,Ethereum,Solana.
        #[arg(long, value_delimiter = ',')]
        chains: Option<Vec<String>>,
    },
}

fn default_chain_paths() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Bitcoin", "m/84'/0'/0'/0/0"),
        ("Ethereum", "m/44'/60'/0'/0/0"),
        ("Solana", "m/44'/501'/0'/0'"),
        ("Litecoin", "m/84'/2'/0'/0/0"),
        ("Dogecoin", "m/44'/3'/0'/0/0"),
        ("Tron", "m/44'/195'/0'/0/0"),
        ("XRP Ledger", "m/44'/144'/0'/0/0"),
        ("Cardano", "m/1852'/1815'/0'/0/0"),
        ("Sui", "m/44'/784'/0'/0'/0'"),
        ("Aptos", "m/44'/637'/0'/0'/0'"),
        ("TON", "m/44'/607'/0'"),
        ("NEAR", "m/44'/397'/0'"),
        ("Polkadot", "m/44'/354'/0'/0'/0'"),
    ]
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Addresses { seed, chains } => run_addresses(&seed, chains.as_deref()),
    }
}

fn run_addresses(seed: &str, filter: Option<&[String]>) -> ExitCode {
    let defaults = default_chain_paths();
    let filter_set: Option<std::collections::HashSet<String>> =
        filter.map(|values| values.iter().map(|value| value.to_string()).collect());

    let chain_paths: BTreeMap<String, String> = defaults
        .into_iter()
        .filter(|(chain, _)| match &filter_set {
            Some(set) => set.contains(*chain),
            None => matches!(*chain, "Bitcoin" | "Ethereum" | "Solana"),
        })
        .map(|(chain, path)| (chain.to_string(), path.to_string()))
        .collect();

    if chain_paths.is_empty() {
        eprintln!("No matching chains. Known: {:?}", default_chain_paths()
            .into_iter().map(|(chain, _)| chain).collect::<Vec<_>>());
        return ExitCode::from(2);
    }

    let chain_paths_json = match serde_json::to_string(&chain_paths) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("Failed to encode chain map: {error}");
            return ExitCode::FAILURE;
        }
    };

    let result_json = match spectra_core::derivation_derive_all_addresses_json(
        seed.to_string(),
        chain_paths_json,
    ) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("Derivation failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    let addresses: BTreeMap<String, Option<String>> = match serde_json::from_str(&result_json) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("Could not parse derivation result: {error}");
            return ExitCode::FAILURE;
        }
    };

    let width = addresses.keys().map(|key| key.len()).max().unwrap_or(0);
    for (chain, address) in addresses {
        match address {
            Some(value) => println!("{chain:<width$}  {value}"),
            None => println!("{chain:<width$}  <unavailable>"),
        }
    }

    ExitCode::SUCCESS
}
