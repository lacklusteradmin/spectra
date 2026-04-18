use colored::Colorize;
use dialoguer::{FuzzySelect, Input};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;

const VERSION: &str = env!("CARGO_PKG_VERSION");

// ─── Persisted wallet ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CliWallet {
    id: String,
    name: String,
    chain_name: String,
    address: String,
    #[serde(default)]
    derivation_path: Option<String>,
    #[serde(default)]
    watch_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CliWalletStore {
    version: u32,
    wallets: Vec<CliWallet>,
}

impl Default for CliWalletStore {
    fn default() -> Self {
        Self { version: 1, wallets: Vec::new() }
    }
}

// ─── Data directory ──────────────────────────────────────────────────────────

fn data_dir() -> PathBuf {
    dirs::home_dir()
        .expect("cannot determine home directory")
        .join(".spectra")
}

fn secrets_dir() -> PathBuf {
    data_dir().join("secrets")
}

fn wallets_path() -> PathBuf {
    data_dir().join("wallets.json")
}

fn ensure_dirs() {
    let sd = secrets_dir();
    if !sd.exists() {
        fs::create_dir_all(&sd).expect("failed to create ~/.spectra/secrets");
    }
}

fn load_store() -> CliWalletStore {
    let path = wallets_path();
    if !path.exists() {
        return CliWalletStore::default();
    }
    let data = fs::read_to_string(&path).expect("failed to read wallets.json");
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_store(store: &CliWalletStore) {
    let path = wallets_path();
    let json = serde_json::to_string_pretty(store).expect("failed to serialize wallet store");
    fs::write(&path, json).expect("failed to write wallets.json");
}

// ─── Chain catalog ───────────────────────────────────────────────────────────

struct ChainInfo {
    name: String,
    default_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChainPresetRaw {
    chain: String,
    networks: Vec<NetworkPresetRaw>,
    derivation_paths: Vec<PathPresetRaw>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetworkPresetRaw {
    network: String,
    title: String,
    detail: String,
    is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathPresetRaw {
    title: String,
    derivation_path: String,
    is_default: bool,
}

fn supported_chains() -> Vec<ChainInfo> {
    let bootstrap = spectra_core::catalog::core_bootstrap()
        .expect("failed to load chain catalog");
    bootstrap
        .chains
        .into_iter()
        .filter_map(|c| {
            c.default_derivation_path.map(|path| ChainInfo {
                name: c.chain_name,
                default_path: path,
            })
        })
        .collect()
}

fn load_chain_presets() -> Vec<ChainPresetRaw> {
    const PRESETS_JSON: &str = include_str!("../../swift/derivation/DerivationPresets.json");
    serde_json::from_str(PRESETS_JSON).expect("failed to parse chain presets")
}

fn chain_color(chain: &str) -> colored::Color {
    match chain {
        "Bitcoin" | "Bitcoin Cash" | "Bitcoin SV" => colored::Color::Yellow,
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism" => colored::Color::Blue,
        "Solana" => colored::Color::Magenta,
        "Dogecoin" => colored::Color::BrightYellow,
        "Litecoin" => colored::Color::BrightBlack,
        "BNB Chain" => colored::Color::Yellow,
        "Avalanche" => colored::Color::Red,
        "Tron" => colored::Color::Red,
        "XRP Ledger" => colored::Color::BrightBlue,
        "Cardano" => colored::Color::Blue,
        "Polkadot" => colored::Color::Magenta,
        "Sui" => colored::Color::Cyan,
        "Aptos" => colored::Color::Cyan,
        "TON" => colored::Color::Blue,
        "Stellar" => colored::Color::BrightBlue,
        "NEAR" => colored::Color::Green,
        "Internet Computer" => colored::Color::Magenta,
        "Hyperliquid" => colored::Color::Green,
        "Monero" => colored::Color::BrightRed,
        _ => colored::Color::White,
    }
}

// ─── Password → AES key derivation ──────────────────────────────────────────

fn derive_master_key(password: &str, salt: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.trim().as_bytes(), salt, 210_000, &mut key);
    key
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn rand_fill(buf: &mut [u8]) {
    getrandom::getrandom(buf).expect("failed to get random bytes");
}

fn read_secret(prompt: &str) -> String {
    if io::stdin().is_terminal() {
        rpassword::prompt_password(prompt).expect("failed to read input")
    } else {
        eprint!("{prompt}");
        io::stderr().flush().ok();
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line).expect("failed to read input");
        line.trim_end_matches('\n').trim_end_matches('\r').to_string()
    }
}

fn print_prompt() {
    print!("{} ", "spectra>".cyan().bold());
    io::stdout().flush().ok();
}

// ─── Banner ──────────────────────────────────────────────────────────────────

/// Apply a red→magenta→purple horizontal gradient to a line using truecolor ANSI.
fn gradient_line(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len().max(1) as f64;
    let mut out = String::new();
    for (i, ch) in chars.iter().enumerate() {
        if *ch == ' ' {
            out.push(' ');
            continue;
        }
        let t = i as f64 / (len - 1.0).max(1.0);
        // Red (255,40,40) → Orange (255,150,30) → Yellow (255,230,50) → Green (80,220,60) → Cyan (40,200,220) → Blue (50,80,255) → Purple (140,50,255)
        let (r, g, b) = if t < 0.167 {
            let s = t / 0.167;
            (255.0, 40.0 + 110.0 * s, 40.0 - 10.0 * s)
        } else if t < 0.333 {
            let s = (t - 0.167) / 0.167;
            (255.0, 150.0 + 80.0 * s, 30.0 + 20.0 * s)
        } else if t < 0.5 {
            let s = (t - 0.333) / 0.167;
            (255.0 - 175.0 * s, 230.0 - 10.0 * s, 50.0 + 10.0 * s)
        } else if t < 0.667 {
            let s = (t - 0.5) / 0.167;
            (80.0 - 40.0 * s, 220.0 - 20.0 * s, 60.0 + 160.0 * s)
        } else if t < 0.833 {
            let s = (t - 0.667) / 0.167;
            (40.0 + 10.0 * s, 200.0 - 120.0 * s, 220.0 + 35.0 * s)
        } else {
            let s = (t - 0.833) / 0.167;
            (50.0 + 90.0 * s, 80.0 - 30.0 * s, 255.0)
        };
        out.push_str(&format!("\x1b[38;2;{};{};{}m{}\x1b[0m", r as u8, g as u8, b as u8, ch));
    }
    out
}

fn print_banner() {
    let lines = [
        " █████  ██████  ███████  ██████ ████████ ██████   █████ ",
        "██      ██   ██ ██      ██         ██    ██   ██ ██   ██",
        " █████  ██████  █████   ██         ██    ██████  ███████",
        "     ██ ██      ██      ██         ██    ██   ██ ██   ██",
        " █████  ██      ███████  ██████    ██    ██   ██ ██   ██",
    ];
    println!();
    for line in &lines {
        println!("  {}", gradient_line(line));
    }
    println!();
    println!("  {}  {}", "Crypto Wallet".white().bold(), format!("v{VERSION}").dimmed());
    println!("  {}", "Type `help` for commands, `quit` to exit.".dimmed());
    println!();
}

// ─── Commands ────────────────────────────────────────────────────────────────

fn cmd_import(args: &[&str]) {
    ensure_dirs();

    let mut chain_arg: Option<String> = None;
    let mut name_arg: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--chain" if i + 1 < args.len() => { chain_arg = Some(args[i + 1].to_string()); i += 2; }
            "--name" if i + 1 < args.len() => { name_arg = Some(args[i + 1].to_string()); i += 2; }
            _ => { i += 1; }
        }
    }

    let chains = supported_chains();
    if chains.is_empty() {
        eprintln!("{} No supported chains found in catalog.", "error:".red().bold());
        return;
    }

    // 1. Select chain
    let selected = match &chain_arg {
        Some(name) => {
            match chains.iter().find(|c| c.name.eq_ignore_ascii_case(name)) {
                Some(c) => c,
                None => {
                    eprintln!("{} Unknown chain \"{}\".", "error:".red().bold(), name.yellow());
                    eprintln!("{}", "Supported chains:".dimmed());
                    for c in &chains {
                        eprintln!("  {}", c.name.color(chain_color(&c.name)));
                    }
                    return;
                }
            }
        }
        None => {
            let names: Vec<&str> = chains.iter().map(|c| c.name.as_str()).collect();
            let idx = match FuzzySelect::new()
                .with_prompt("Select a chain")
                .items(&names)
                .default(0)
                .interact_opt()
            {
                Ok(Some(i)) => i,
                _ => { println!("{}", "Cancelled.".dimmed()); return; }
            };
            &chains[idx]
        }
    };

    println!("  {} {}", "Chain:".dimmed(), selected.name.color(chain_color(&selected.name)).bold());

    // 2. Wallet name
    let wallet_name = match name_arg {
        Some(n) => n,
        None => {
            match Input::<String>::new()
                .with_prompt("Wallet name")
                .default(format!("My {} Wallet", selected.name))
                .interact_text()
            {
                Ok(n) => n,
                Err(_) => { println!("{}", "Cancelled.".dimmed()); return; }
            }
        }
    };

    // 3. Seed phrase
    let seed_phrase = read_secret(&format!("  {} ", "Enter seed phrase:".dimmed()));
    let seed_trimmed = seed_phrase.trim().to_string();

    if seed_trimmed.is_empty() {
        eprintln!("{} Seed phrase cannot be empty.", "error:".red().bold());
        return;
    }

    if bip39::Mnemonic::parse_in(bip39::Language::English, &seed_trimmed).is_err() {
        eprintln!("{} Invalid BIP39 mnemonic. Expected 12 or 24 words.", "error:".red().bold());
        return;
    }

    let word_count = seed_trimmed.split_whitespace().count();
    println!("  {} {}-word mnemonic", "validated".green(), word_count);

    // Derive address
    let mut chain_paths = HashMap::new();
    chain_paths.insert(selected.name.clone(), selected.default_path.clone());

    let addresses = match spectra_core::derivation_derive_all_addresses(
        seed_trimmed.clone(),
        chain_paths,
    ) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{} Derivation failed: {e}", "error:".red().bold());
            return;
        }
    };

    let address = match addresses.get(&selected.name) {
        Some(a) => a.clone(),
        None => {
            eprintln!("{} No address could be derived for {}.", "error:".red().bold(), selected.name);
            return;
        }
    };

    // 4. Password
    let password = loop {
        let pw = read_secret(&format!("  {} ", "Set password:".dimmed()));
        if pw.trim().is_empty() {
            eprintln!("{} Password cannot be empty.", "error:".red().bold());
            continue;
        }
        let confirm = read_secret(&format!("  {} ", "Confirm password:".dimmed()));
        if pw != confirm {
            eprintln!("{} Passwords do not match.", "error:".red().bold());
            continue;
        }
        break pw;
    };

    // 5. Encrypt
    let mut salt = [0u8; 16];
    rand_fill(&mut salt);
    let master_key = derive_master_key(&password, &salt);

    let encrypted_seed = match spectra_core::seed_envelope::encrypt(
        seed_trimmed.as_bytes(),
        &master_key,
    ) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("{} Encryption failed: {e}", "error:".red().bold());
            return;
        }
    };

    let password_verifier = match spectra_core::password_verifier::create_verifier(&password) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("{} Verifier failed: {e}", "error:".red().bold());
            return;
        }
    };

    // 6. Persist
    let wallet_id = uuid::Uuid::new_v4().to_string().to_uppercase();

    fs::write(secrets_dir().join(format!("{wallet_id}.seed")), &encrypted_seed)
        .expect("failed to write encrypted seed");
    fs::write(secrets_dir().join(format!("{wallet_id}.salt")), &salt)
        .expect("failed to write salt");
    fs::write(secrets_dir().join(format!("{wallet_id}.password")), &password_verifier)
        .expect("failed to write password verifier");

    let wallet = CliWallet {
        id: wallet_id,
        name: wallet_name.clone(),
        chain_name: selected.name.clone(),
        address: address.clone(),
        derivation_path: Some(selected.default_path.clone()),
        watch_only: false,
    };

    let mut store = load_store();
    store.wallets.push(wallet);
    save_store(&store);

    println!();
    println!("  {} Wallet imported.", "OK".green().bold());
    println!("  {} {}", "Name:".dimmed(), wallet_name.white().bold());
    println!("  {} {}", "Chain:".dimmed(), selected.name.color(chain_color(&selected.name)));
    println!("  {} {}", "Path:".dimmed(), selected.default_path.dimmed());
    println!("  {} {}", "Address:".dimmed(), address.bright_white());
}

// ─── Advanced import ────────────────────────────────────────────────────────

fn cmd_advimport() {
    ensure_dirs();

    let presets = load_chain_presets();
    if presets.is_empty() {
        eprintln!("{} No chain presets found.", "error:".red().bold());
        return;
    }

    // 1. Select chain
    let chain_names: Vec<&str> = presets.iter().map(|p| p.chain.as_str()).collect();
    let chain_idx = match FuzzySelect::new()
        .with_prompt("Select a chain")
        .items(&chain_names)
        .default(0)
        .interact_opt()
    {
        Ok(Some(i)) => i,
        _ => { println!("{}", "Cancelled.".dimmed()); return; }
    };
    let preset = &presets[chain_idx];
    println!("  {} {}", "Chain:".dimmed(), preset.chain.color(chain_color(&preset.chain)).bold());

    // 2. Select network (if multiple)
    let network = if preset.networks.len() > 1 {
        let labels: Vec<String> = preset.networks.iter()
            .map(|n| format!("{} — {}", n.title, n.detail))
            .collect();
        let default_idx = preset.networks.iter().position(|n| n.is_default).unwrap_or(0);
        let idx = match FuzzySelect::new()
            .with_prompt("Select network")
            .items(&labels)
            .default(default_idx)
            .interact_opt()
        {
            Ok(Some(i)) => i,
            _ => { println!("{}", "Cancelled.".dimmed()); return; }
        };
        let net = &preset.networks[idx];
        println!("  {} {}", "Network:".dimmed(), net.title.white().bold());
        Some(net.network.clone())
    } else {
        preset.networks.first().map(|n| n.network.clone())
    };

    // 3. Select derivation path (if multiple, plus custom option)
    let derivation_path = if preset.derivation_paths.len() > 1 {
        let mut labels: Vec<String> = preset.derivation_paths.iter()
            .map(|p| format!("{:<20} {}", p.title, p.derivation_path.dimmed()))
            .collect();
        labels.push("Custom path...".to_string());
        let default_idx = preset.derivation_paths.iter().position(|p| p.is_default).unwrap_or(0);
        let idx = match FuzzySelect::new()
            .with_prompt("Select derivation path")
            .items(&labels)
            .default(default_idx)
            .interact_opt()
        {
            Ok(Some(i)) => i,
            _ => { println!("{}", "Cancelled.".dimmed()); return; }
        };
        if idx == preset.derivation_paths.len() {
            // Custom path
            match Input::<String>::new()
                .with_prompt("Enter derivation path (e.g. m/44'/0'/0'/0/0)")
                .interact_text()
            {
                Ok(p) => {
                    println!("  {} {}", "Path:".dimmed(), p.white().bold());
                    p
                }
                Err(_) => { println!("{}", "Cancelled.".dimmed()); return; }
            }
        } else {
            let path = &preset.derivation_paths[idx];
            println!("  {} {} ({})", "Path:".dimmed(), path.derivation_path.white().bold(), path.title.dimmed());
            path.derivation_path.clone()
        }
    } else if let Some(path) = preset.derivation_paths.first() {
        // Only one path — offer to use it or enter custom
        let labels = vec![
            format!("{:<20} {}", path.title, path.derivation_path.dimmed()),
            "Custom path...".to_string(),
        ];
        let idx = match FuzzySelect::new()
            .with_prompt("Select derivation path")
            .items(&labels)
            .default(0)
            .interact_opt()
        {
            Ok(Some(i)) => i,
            _ => { println!("{}", "Cancelled.".dimmed()); return; }
        };
        if idx == 1 {
            match Input::<String>::new()
                .with_prompt("Enter derivation path (e.g. m/44'/0'/0'/0/0)")
                .interact_text()
            {
                Ok(p) => {
                    println!("  {} {}", "Path:".dimmed(), p.white().bold());
                    p
                }
                Err(_) => { println!("{}", "Cancelled.".dimmed()); return; }
            }
        } else {
            println!("  {} {} ({})", "Path:".dimmed(), path.derivation_path.white().bold(), path.title.dimmed());
            path.derivation_path.clone()
        }
    } else {
        eprintln!("{} No derivation paths available for {}.", "error:".red().bold(), preset.chain);
        return;
    };

    // 4. Wallet name
    let wallet_name = match Input::<String>::new()
        .with_prompt("Wallet name")
        .default(format!("My {} Wallet", preset.chain))
        .interact_text()
    {
        Ok(n) => n,
        Err(_) => { println!("{}", "Cancelled.".dimmed()); return; }
    };

    // 5. Seed phrase
    let seed_phrase = read_secret(&format!("  {} ", "Enter seed phrase:".dimmed()));
    let seed_trimmed = seed_phrase.trim().to_string();

    if seed_trimmed.is_empty() {
        eprintln!("{} Seed phrase cannot be empty.", "error:".red().bold());
        return;
    }

    if bip39::Mnemonic::parse_in(bip39::Language::English, &seed_trimmed).is_err() {
        eprintln!("{} Invalid BIP39 mnemonic. Expected 12 or 24 words.", "error:".red().bold());
        return;
    }

    let word_count = seed_trimmed.split_whitespace().count();
    println!("  {} {}-word mnemonic", "validated".green(), word_count);

    // Derive address
    let mut chain_paths = HashMap::new();
    chain_paths.insert(preset.chain.clone(), derivation_path.clone());

    let addresses = match spectra_core::derivation_derive_all_addresses(
        seed_trimmed.clone(),
        chain_paths,
    ) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{} Derivation failed: {e}", "error:".red().bold());
            return;
        }
    };

    let address = match addresses.get(&preset.chain) {
        Some(a) => a.clone(),
        None => {
            eprintln!("{} No address could be derived for {}.", "error:".red().bold(), preset.chain);
            return;
        }
    };

    // 6. Password
    let password = loop {
        let pw = read_secret(&format!("  {} ", "Set password:".dimmed()));
        if pw.trim().is_empty() {
            eprintln!("{} Password cannot be empty.", "error:".red().bold());
            continue;
        }
        let confirm = read_secret(&format!("  {} ", "Confirm password:".dimmed()));
        if pw != confirm {
            eprintln!("{} Passwords do not match.", "error:".red().bold());
            continue;
        }
        break pw;
    };

    // 7. Encrypt
    let mut salt = [0u8; 16];
    rand_fill(&mut salt);
    let master_key = derive_master_key(&password, &salt);

    let encrypted_seed = match spectra_core::seed_envelope::encrypt(
        seed_trimmed.as_bytes(),
        &master_key,
    ) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("{} Encryption failed: {e}", "error:".red().bold());
            return;
        }
    };

    let password_verifier = match spectra_core::password_verifier::create_verifier(&password) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("{} Verifier failed: {e}", "error:".red().bold());
            return;
        }
    };

    // 8. Persist
    let wallet_id = uuid::Uuid::new_v4().to_string().to_uppercase();

    fs::write(secrets_dir().join(format!("{wallet_id}.seed")), &encrypted_seed)
        .expect("failed to write encrypted seed");
    fs::write(secrets_dir().join(format!("{wallet_id}.salt")), &salt)
        .expect("failed to write salt");
    fs::write(secrets_dir().join(format!("{wallet_id}.password")), &password_verifier)
        .expect("failed to write password verifier");

    let wallet = CliWallet {
        id: wallet_id,
        name: wallet_name.clone(),
        chain_name: preset.chain.clone(),
        address: address.clone(),
        derivation_path: Some(derivation_path.clone()),
        watch_only: false,
    };

    let mut store = load_store();
    store.wallets.push(wallet);
    save_store(&store);

    println!();
    println!("  {} Wallet imported.", "OK".green().bold());
    println!("  {} {}", "Name:".dimmed(), wallet_name.white().bold());
    println!("  {} {}", "Chain:".dimmed(), preset.chain.color(chain_color(&preset.chain)));
    if let Some(ref net) = network {
        println!("  {} {}", "Network:".dimmed(), net.white());
    }
    println!("  {} {}", "Path:".dimmed(), derivation_path.dimmed());
    println!("  {} {}", "Address:".dimmed(), address.bright_white());
}

// ─── New wallet ─────────────────────────────────────────────────────────────

fn cmd_newwallet() {
    ensure_dirs();

    let chains = supported_chains();
    if chains.is_empty() {
        eprintln!("{} No supported chains found in catalog.", "error:".red().bold());
        return;
    }

    // 1. Select chain
    let chain_names: Vec<&str> = chains.iter().map(|c| c.name.as_str()).collect();
    let chain_idx = match FuzzySelect::new()
        .with_prompt("Select a chain")
        .items(&chain_names)
        .default(0)
        .interact_opt()
    {
        Ok(Some(i)) => i,
        _ => { println!("{}", "Cancelled.".dimmed()); return; }
    };
    let selected = &chains[chain_idx];
    println!("  {} {}", "Chain:".dimmed(), selected.name.color(chain_color(&selected.name)).bold());

    // 2. Word count
    let word_options = ["12 words", "24 words"];
    let word_idx = match FuzzySelect::new()
        .with_prompt("Seed phrase length")
        .items(&word_options)
        .default(0)
        .interact_opt()
    {
        Ok(Some(i)) => i,
        _ => { println!("{}", "Cancelled.".dimmed()); return; }
    };
    let word_count: usize = if word_idx == 0 { 12 } else { 24 };

    // 3. Generate mnemonic
    let entropy_len = if word_count == 12 { 16 } else { 32 };
    let mut entropy = vec![0u8; entropy_len];
    rand_fill(&mut entropy);
    let mnemonic = match bip39::Mnemonic::from_entropy_in(bip39::Language::English, &entropy) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{} Failed to generate mnemonic: {e}", "error:".red().bold());
            return;
        }
    };
    let seed_phrase = mnemonic.to_string();

    println!();
    println!("  {}", "WRITE DOWN YOUR SEED PHRASE AND KEEP IT SAFE!".yellow().bold());
    println!("  {}", "Anyone with these words can access your funds.".yellow());
    println!();

    // Display words in a numbered grid
    let words: Vec<&str> = seed_phrase.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        let num = format!("{:>2}.", i + 1).dimmed();
        let w = format!("{:<12}", word).white().bold();
        if (i + 1) % 4 == 0 {
            println!("  {num} {w}");
        } else {
            print!("  {num} {w}");
        }
    }
    if words.len() % 4 != 0 {
        println!();
    }
    println!();

    // 4. Wallet name
    let wallet_name: String = match Input::<String>::new()
        .with_prompt("Wallet name")
        .default(format!("My {} Wallet", selected.name))
        .interact_text()
    {
        Ok(n) => n,
        Err(_) => { println!("{}", "Cancelled.".dimmed()); return; }
    };

    // 5. Derive address
    let mut chain_paths = HashMap::new();
    chain_paths.insert(selected.name.clone(), selected.default_path.clone());

    let addresses = match spectra_core::derivation_derive_all_addresses(
        seed_phrase.clone(),
        chain_paths,
    ) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{} Derivation failed: {e}", "error:".red().bold());
            return;
        }
    };

    let address = match addresses.get(&selected.name) {
        Some(a) => a.clone(),
        None => {
            eprintln!("{} No address could be derived for {}.", "error:".red().bold(), selected.name);
            return;
        }
    };

    // 6. Password
    let password = loop {
        let pw = read_secret(&format!("  {} ", "Set password:".dimmed()));
        if pw.trim().is_empty() {
            eprintln!("{} Password cannot be empty.", "error:".red().bold());
            continue;
        }
        let confirm = read_secret(&format!("  {} ", "Confirm password:".dimmed()));
        if pw != confirm {
            eprintln!("{} Passwords do not match.", "error:".red().bold());
            continue;
        }
        break pw;
    };

    // 7. Encrypt
    let mut salt = [0u8; 16];
    rand_fill(&mut salt);
    let master_key = derive_master_key(&password, &salt);

    let encrypted_seed = match spectra_core::seed_envelope::encrypt(
        seed_phrase.as_bytes(),
        &master_key,
    ) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("{} Encryption failed: {e}", "error:".red().bold());
            return;
        }
    };

    let password_verifier = match spectra_core::password_verifier::create_verifier(&password) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("{} Verifier failed: {e}", "error:".red().bold());
            return;
        }
    };

    // 8. Persist
    let wallet_id = uuid::Uuid::new_v4().to_string().to_uppercase();

    fs::write(secrets_dir().join(format!("{wallet_id}.seed")), &encrypted_seed)
        .expect("failed to write encrypted seed");
    fs::write(secrets_dir().join(format!("{wallet_id}.salt")), &salt)
        .expect("failed to write salt");
    fs::write(secrets_dir().join(format!("{wallet_id}.password")), &password_verifier)
        .expect("failed to write password verifier");

    let wallet = CliWallet {
        id: wallet_id,
        name: wallet_name.clone(),
        chain_name: selected.name.clone(),
        address: address.clone(),
        derivation_path: Some(selected.default_path.clone()),
        watch_only: false,
    };

    let mut store = load_store();
    store.wallets.push(wallet);
    save_store(&store);

    println!();
    println!("  {} Wallet created.", "OK".green().bold());
    println!("  {} {}", "Name:".dimmed(), wallet_name.white().bold());
    println!("  {} {}", "Chain:".dimmed(), selected.name.color(chain_color(&selected.name)));
    println!("  {} {}", "Path:".dimmed(), selected.default_path.dimmed());
    println!("  {} {}", "Address:".dimmed(), address.bright_white());
}

// ─── Watch-only import ──────────────────────────────────────────────────────

fn cmd_wimport() {
    ensure_dirs();

    let chains = supported_chains();
    if chains.is_empty() {
        eprintln!("{} No supported chains found in catalog.", "error:".red().bold());
        return;
    }

    // 1. Select chain
    let chain_names: Vec<&str> = chains.iter().map(|c| c.name.as_str()).collect();
    let chain_idx = match FuzzySelect::new()
        .with_prompt("Select a chain")
        .items(&chain_names)
        .default(0)
        .interact_opt()
    {
        Ok(Some(i)) => i,
        _ => { println!("{}", "Cancelled.".dimmed()); return; }
    };
    let selected = &chains[chain_idx];
    println!("  {} {}", "Chain:".dimmed(), selected.name.color(chain_color(&selected.name)).bold());

    // 2. Address
    let address: String = match Input::<String>::new()
        .with_prompt("Watch address")
        .interact_text()
    {
        Ok(a) => a.trim().to_string(),
        Err(_) => { println!("{}", "Cancelled.".dimmed()); return; }
    };

    if address.is_empty() {
        eprintln!("{} Address cannot be empty.", "error:".red().bold());
        return;
    }

    // 3. Wallet name
    let wallet_name: String = match Input::new()
        .with_prompt("Wallet name")
        .default(format!("{} (watch)", selected.name))
        .interact_text()
    {
        Ok(n) => n,
        Err(_) => { println!("{}", "Cancelled.".dimmed()); return; }
    };

    // 4. Persist (no secrets needed for watch-only)
    let wallet_id = uuid::Uuid::new_v4().to_string().to_uppercase();

    let wallet = CliWallet {
        id: wallet_id,
        name: wallet_name.clone(),
        chain_name: selected.name.clone(),
        address: address.clone(),
        derivation_path: None,
        watch_only: true,
    };

    let mut store = load_store();
    store.wallets.push(wallet);
    save_store(&store);

    println!();
    println!("  {} Watch-only wallet added.", "OK".green().bold());
    println!("  {} {}", "Name:".dimmed(), wallet_name.white().bold());
    println!("  {} {}", "Chain:".dimmed(), selected.name.color(chain_color(&selected.name)));
    println!("  {} {}", "Address:".dimmed(), address.bright_white());
}

fn cmd_list() {
    let store = load_store();

    if store.wallets.is_empty() {
        println!("  {} Use {} to add one.", "No wallets.".dimmed(), "import".cyan());
        return;
    }

    let chain_width = store.wallets.iter().map(|w| w.chain_name.len()).max().unwrap_or(0).max(5);
    let name_width = store.wallets.iter().map(|w| w.name.len()).max().unwrap_or(0).max(4);

    println!(
        "  {:<name_width$}  {:<chain_width$}  {}",
        "NAME".dimmed().bold(),
        "CHAIN".dimmed().bold(),
        "ADDRESS".dimmed().bold(),
    );
    println!(
        "  {:<name_width$}  {:<chain_width$}  {}",
        "─".repeat(name_width).dimmed(),
        "─".repeat(chain_width).dimmed(),
        "─".repeat(42).dimmed(),
    );

    for w in &store.wallets {
        let cc = chain_color(&w.chain_name);
        let tag = if w.watch_only { " (watch)".dimmed().to_string() } else { String::new() };
        println!(
            "  {:<name_width$}  {:<chain_width$}  {}{}",
            w.name.white().bold(),
            // Pad manually because .color() adds escape codes that break padding
            format!("{:<chain_width$}", w.chain_name).color(cc),
            w.address.bright_white(),
            tag,
        );
    }

    println!();
    println!(
        "  {} wallet{}",
        store.wallets.len().to_string().cyan().bold(),
        if store.wallets.len() == 1 { "" } else { "s" },
    );
}

fn cmd_delete() {
    let mut store = load_store();

    if store.wallets.is_empty() {
        println!("  {} Nothing to delete.", "No wallets.".dimmed());
        return;
    }

    let labels: Vec<String> = store.wallets.iter()
        .map(|w| {
            let tag = if w.watch_only { " (watch)" } else { "" };
            format!("{} — {} — {}{}", w.name, w.chain_name, w.address, tag)
        })
        .collect();

    let idx = match FuzzySelect::new()
        .with_prompt("Select wallet to delete")
        .items(&labels)
        .interact_opt()
    {
        Ok(Some(i)) => i,
        _ => { println!("{}", "Cancelled.".dimmed()); return; }
    };

    let wallet = &store.wallets[idx];
    let confirm_label = format!(
        "Delete \"{}\" ({})? Type '{}' to confirm",
        wallet.name, wallet.chain_name, "yes".red().bold()
    );
    let answer: String = match Input::<String>::new()
        .with_prompt(&confirm_label)
        .default("no".into())
        .interact_text()
    {
        Ok(a) => a,
        Err(_) => { println!("{}", "Cancelled.".dimmed()); return; }
    };

    if answer.trim().to_lowercase() != "yes" {
        println!("{}", "  Cancelled.".dimmed());
        return;
    }

    let wallet_id = wallet.id.clone();
    let wallet_name = wallet.name.clone();
    let is_watch = wallet.watch_only;

    store.wallets.remove(idx);
    save_store(&store);

    // Clean up secret files (if not watch-only)
    if !is_watch {
        let _ = fs::remove_file(secrets_dir().join(format!("{wallet_id}.seed")));
        let _ = fs::remove_file(secrets_dir().join(format!("{wallet_id}.salt")));
        let _ = fs::remove_file(secrets_dir().join(format!("{wallet_id}.password")));
    }

    println!("  {} Wallet \"{}\" deleted.", "OK".green().bold(), wallet_name);
}

fn cmd_about() {
    let chains = supported_chains();
    println!();
    println!("  {} {}", "Spectra Crypto Wallet".white().bold(), format!("v{VERSION}").dimmed());
    println!("  {}", "Multi-chain self-custody wallet".dimmed());
    println!();
    println!("  {} AES-256-GCM + PBKDF2-HMAC-SHA256", "Encryption:".dimmed());
    println!("  {}  {}", "Storage:".dimmed(), data_dir().display().to_string().dimmed());
    println!("  {}   {}", "Chains:".dimmed(), chains.len().to_string().cyan());
    println!();
    println!("  {}", "Supported chains:".dimmed());

    // Print chains in columns
    let names: Vec<String> = chains.iter().map(|c| c.name.clone()).collect();
    let col_width = names.iter().map(|n| n.len()).max().unwrap_or(0) + 2;
    let term_width = 72;
    let cols = (term_width / col_width).max(1);

    for row in names.chunks(cols) {
        print!("  ");
        for name in row {
            let colored_name = name.color(chain_color(name));
            // Manually pad after the colored string
            let padding = col_width.saturating_sub(name.len());
            print!("{}{}", colored_name, " ".repeat(padding));
        }
        println!();
    }
    println!();
    println!("  {}", "github.com/sheny6n/SpectraWallet".dimmed());
}

fn cmd_import_menu() {
    let options = [
        "New Wallet        — generate a new seed phrase",
        "Simple Import     — import wallet from seed phrase",
        "Advanced Import   — choose network & derivation path",
        "Watch Import      — watch-only wallet by address",
    ];
    let idx = match FuzzySelect::new()
        .with_prompt("Import type")
        .items(&options)
        .default(0)
        .interact_opt()
    {
        Ok(Some(i)) => i,
        _ => { println!("{}", "Cancelled.".dimmed()); return; }
    };
    match idx {
        0 => cmd_newwallet(),
        1 => cmd_import(&[]),
        2 => cmd_advimport(),
        3 => cmd_wimport(),
        _ => unreachable!(),
    }
}

fn cmd_help() {
    println!();
    println!("  {}", "Commands:".white().bold());
    println!();
    println!("  {}  {}",
        format!("{:<42}", "import").cyan(),
        "Import a wallet (choose type)".dimmed(),
    );
    println!("  {}  {}",
        format!("{:<42}", "nw / newwallet").cyan(),
        "Generate new wallet with seed phrase".dimmed(),
    );
    println!("  {}  {}",
        format!("{:<42}", "si / simport [--chain <name>] [--name <n>]").cyan(),
        "Simple import from seed phrase".dimmed(),
    );
    println!("  {}  {}",
        format!("{:<42}", "ai / advimport").cyan(),
        "Advanced import (network, derivation path)".dimmed(),
    );
    println!("  {}  {}",
        format!("{:<42}", "wi / wimport").cyan(),
        "Watch-only import by address".dimmed(),
    );
    println!("  {}  {}",
        format!("{:<42}", "list").cyan(),
        "List all wallets".dimmed(),
    );
    println!("  {}  {}",
        format!("{:<42}", "delete").cyan(),
        "Delete a wallet".dimmed(),
    );
    println!("  {}  {}",
        format!("{:<42}", "about").cyan(),
        "About Spectra".dimmed(),
    );
    println!("  {}  {}",
        format!("{:<42}", "help").cyan(),
        "Show this help".dimmed(),
    );
    println!("  {}  {}",
        format!("{:<42}", "quit").cyan(),
        "Exit Spectra".dimmed(),
    );
    println!();
}

// ─── Interactive shell ───────────────────────────────────────────────────────

fn run_shell() {
    print_banner();

    let stdin = io::stdin();
    loop {
        print_prompt();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        let cmd = parts[0].to_lowercase();
        let args = &parts[1..];

        match cmd.as_str() {
            "import" => cmd_import_menu(),
            "nw" | "newwallet" => cmd_newwallet(),
            "si" | "simport" => cmd_import(args),
            "ai" | "advimport" => cmd_advimport(),
            "wi" | "wimport" => cmd_wimport(),
            "list" | "ls" => cmd_list(),
            "delete" | "rm" => cmd_delete(),
            "about" => cmd_about(),
            "help" | "?" => cmd_help(),
            "quit" | "exit" | "q" => {
                println!("  {}", "Goodbye.".dimmed());
                break;
            }
            other => {
                eprintln!("  {} Unknown command: {}. Type {} for help.",
                    "?".yellow().bold(),
                    other.white(),
                    "help".cyan(),
                );
            }
        }

        println!();
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    run_shell();
}
