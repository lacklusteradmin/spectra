use base64::Engine as _;
use colored::Colorize;
use dialoguer::{FuzzySelect, Input};
use std::collections::HashMap;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::LazyLock;

use spectra_core::store::secret_backends::FileSecretStore;
use spectra_core::store::secret_store::{SecretClass, SecretStore};
use spectra_core::store::state::{CoreAppState, WalletSummary};
use spectra_core::store::wallet_db;

// ─── Store ───────────────────────────────────────────────────────────────────
//
// The CLI holds no wallet model of its own. A wallet is a `WalletSummary` and
// the whole app state is a `CoreAppState`, persisted by `wallet_db` — the same
// types and the same SQLite store the iOS app uses. That is the CLI's job here:
// if a core API can only be driven from Swift, it isn't core yet.

fn data_dir() -> PathBuf {
    dirs::home_dir()
        .expect("cannot determine home directory")
        .join(".spectra")
}

fn db_path() -> String {
    data_dir()
        .join("spectra.sqlite")
        .to_string_lossy()
        .into_owned()
}

/// Process-wide secret store rooted at `~/.spectra/secrets`.
///
/// The Rust-side counterpart to the Keychain-backed `SecretStore` iOS
/// registers: same trait, same key layout, different backend.
static SECRETS: LazyLock<FileSecretStore> = LazyLock::new(|| {
    FileSecretStore::new(data_dir().join("secrets")).expect("failed to open ~/.spectra/secrets")
});

fn ensure_dirs() {
    std::fs::create_dir_all(data_dir()).expect("failed to create ~/.spectra");
    LazyLock::force(&SECRETS);
}

fn load_state() -> CoreAppState {
    wallet_db::app_state_load(&db_path()).unwrap_or_else(|e| {
        eprintln!("  {} failed to load wallets: {e}", accent("✗").bold());
        CoreAppState::default()
    })
}

fn save_state(state: &CoreAppState) {
    if let Err(e) = wallet_db::app_state_save(&db_path(), state) {
        eprintln!("  {} failed to save wallets: {e}", accent("✗").bold());
    }
}

/// Address the CLI shows and queries for a wallet. Wallets always carry one by
/// construction; the fallback keeps a malformed record printable rather than
/// panicking mid-listing.
fn wallet_address(wallet: &WalletSummary) -> &str {
    wallet.primary_address().unwrap_or("")
}

// ─── Wallet secrets ──────────────────────────────────────────────────────────
//
// Three blobs per wallet: the AES-GCM seed envelope, the salt its master key
// was derived from, and the password verifier. `SecretStore` values are opaque
// strings, so each is base64 on the way in and out.

/// Encrypted seed material for one wallet.
struct WalletSecrets {
    envelope: Vec<u8>,
    salt: Vec<u8>,
    verifier: Vec<u8>,
}

fn seed_key(wallet_id: &str) -> String {
    format!("{wallet_id}.seed")
}

fn salt_key(wallet_id: &str) -> String {
    format!("{wallet_id}.salt")
}

fn verifier_key(wallet_id: &str) -> String {
    format!("{wallet_id}.password")
}

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn save_wallet_secrets(wallet_id: &str, secrets: &WalletSecrets) -> Result<(), String> {
    let engine = b64();
    for (kind, key, value) in [
        (
            SecretClass::Seed,
            seed_key(wallet_id),
            &secrets.envelope,
        ),
        (SecretClass::Generic, salt_key(wallet_id), &secrets.salt),
        (
            SecretClass::Generic,
            verifier_key(wallet_id),
            &secrets.verifier,
        ),
    ] {
        SECRETS
            .save_secret(kind, key, engine.encode(value))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn load_wallet_secrets(wallet_id: &str) -> Result<WalletSecrets, String> {
    let engine = b64();
    let read = |kind, key: String| -> Result<Vec<u8>, String> {
        let raw = SECRETS.load_secret(kind, key).map_err(|e| e.to_string())?;
        engine
            .decode(raw.trim())
            .map_err(|e| format!("corrupt secret: {e}"))
    };
    Ok(WalletSecrets {
        envelope: read(SecretClass::Seed, seed_key(wallet_id))?,
        salt: read(SecretClass::Generic, salt_key(wallet_id))?,
        verifier: read(SecretClass::Generic, verifier_key(wallet_id))?,
    })
}

fn delete_wallet_secrets(wallet_id: &str) {
    let _ = SECRETS.delete_secret(SecretClass::Seed, seed_key(wallet_id));
    let _ = SECRETS.delete_secret(SecretClass::Generic, salt_key(wallet_id));
    let _ = SECRETS.delete_secret(SecretClass::Generic, verifier_key(wallet_id));
}

/// Encrypt `seed_phrase` under `password` and persist all three blobs.
fn seal_wallet_secrets(wallet_id: &str, seed_phrase: &str, password: &str) -> Result<(), String> {
    let mut salt = [0u8; 16];
    rand_fill(&mut salt);
    let master_key = derive_master_key(password, &salt);
    let envelope = spectra_core::store::seed_envelope::encrypt(seed_phrase.as_bytes(), &master_key)?;
    let verifier = spectra_core::store::password_verifier::create_verifier(password)?;
    save_wallet_secrets(
        wallet_id,
        &WalletSecrets {
            envelope,
            salt: salt.to_vec(),
            verifier,
        },
    )
}

/// Check `password` against a wallet's stored verifier, then decrypt its seed
/// phrase. `Err` carries a message already suitable for display.
fn unlock_seed_phrase(wallet_id: &str, password: &str) -> Result<String, String> {
    let secrets = load_wallet_secrets(wallet_id)?;
    if !spectra_core::store::password_verifier::verify(password, &secrets.verifier) {
        return Err("incorrect password".to_string());
    }
    let master_key = derive_master_key(password, &secrets.salt);
    spectra_core::store::seed_envelope::decrypt(&secrets.envelope, &master_key)
}

// ─── Chain catalog ───────────────────────────────────────────────────────────

struct ChainInfo {
    name: String,
    default_path: String,
}

struct ChainPreset {
    chain: String,
    derivation_paths: Vec<DerivationPathChoice>,
    networks: Vec<NetworkChoice>,
}

struct DerivationPathChoice {
    title: String,
    derivation_path: String,
    is_default: bool,
}

struct NetworkChoice {
    title: String,
    detail: String,
    network: String,
    is_default: bool,
}

fn supported_chains() -> Vec<ChainInfo> {
    load_chain_presets()
        .into_iter()
        .filter_map(|preset| {
            preset
                .derivation_paths
                .iter()
                .find(|p| p.is_default)
                .or_else(|| preset.derivation_paths.first())
                .map(|p| ChainInfo {
                    name: preset.chain.clone(),
                    default_path: p.derivation_path.clone(),
                })
        })
        .collect()
}

fn load_chain_presets() -> Vec<ChainPreset> {
    spectra_core::chains::list_all_chains()
        .into_iter()
        .filter_map(|chain| {
            let derivation_paths: Vec<DerivationPathChoice> = chain
                .derivation_path
                .into_iter()
                .filter_map(|entry| {
                    let derivation_path = entry.path.replace("{account}", "0");
                    derivation_path
                        .starts_with("m/")
                        .then_some(DerivationPathChoice {
                            title: entry.tag,
                            derivation_path,
                            is_default: entry.is_default,
                        })
                })
                .collect();

            if derivation_paths.is_empty() {
                return None;
            }

            Some(ChainPreset {
                chain: chain.name,
                derivation_paths,
                networks: vec![NetworkChoice {
                    title: chain.category.clone(),
                    detail: chain.id.clone(),
                    network: chain.id,
                    is_default: true,
                }],
            })
        })
        .collect()
}

fn derive_address_for_chain(
    chain_name: &str,
    seed_phrase: &str,
    derivation_path: &str,
) -> Result<String, spectra_core::SpectraBridgeError> {
    let result = spectra_core::derivation::dispatch::derive_for_chain_name(
        chain_name,
        seed_phrase,
        derivation_path,
        None,
        None,
        None,
        true,
        false,
        false,
    )?;
    result
        .address
        .ok_or_else(|| spectra_core::SpectraBridgeError::Failure {
            message: format!("No address could be derived for {chain_name}."),
        })
}

/// Brand-accurate truecolor for each chain. Returns the legacy `colored::Color`
/// (kept for the few code paths that need it) — for everything else use `chain_rgb`.
fn chain_color(chain: &str) -> colored::Color {
    let (r, g, b) = chain_rgb(chain);
    colored::Color::TrueColor { r, g, b }
}

fn chain_rgb(chain: &str) -> (u8, u8, u8) {
    match chain {
        // Bitcoin family — orange/amber
        "Bitcoin" => (247, 147, 26),
        "Bitcoin Cash" => (139, 197, 65),
        "Bitcoin SV" => (255, 153, 0),
        "Litecoin" => (191, 191, 191),
        "Dogecoin" => (194, 167, 89),
        // Ethereum family — distinct hues per L2
        "Ethereum" => (98, 126, 234),
        "Ethereum Classic" => (60, 132, 99),
        "Arbitrum" => (40, 160, 240),
        "Optimism" => (255, 4, 32),
        "Base" => (0, 82, 255),
        "Polygon" => (130, 71, 229),
        "Avalanche" => (232, 65, 66),
        "BNB Chain" => (240, 185, 11),
        "Hyperliquid" => (151, 252, 228),
        "Linea" => (97, 223, 255),
        "Scroll" => (255, 215, 173),
        "Blast" => (252, 252, 3),
        "Mantle" => (159, 242, 198),
        // Layer 1 / alt
        "Solana" => (153, 69, 255),
        "Tron" => (235, 0, 41),
        "XRP Ledger" => (35, 41, 47),
        "XRP" => (35, 41, 47),
        "Cardano" => (0, 51, 173),
        "Polkadot" => (230, 0, 122),
        "Sui" => (77, 162, 255),
        "Aptos" => (109, 232, 207),
        "TON" => (0, 152, 234),
        "Stellar" => (123, 95, 255),
        "NEAR" => (138, 220, 138),
        "Internet Computer" => (235, 0, 153),
        "ICP" => (235, 0, 153),
        "Monero" => (255, 102, 0),
        _ => (200, 200, 210),
    }
}

fn chain_paint(s: &str, chain: &str) -> colored::ColoredString {
    let (r, g, b) = chain_rgb(chain);
    s.truecolor(r, g, b)
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
        io::stdin()
            .lock()
            .read_line(&mut line)
            .expect("failed to read input");
        line.trim_end_matches('\n')
            .trim_end_matches('\r')
            .to_string()
    }
}

// ─── Design tokens ──────────────────────────────────────────────────────────
// Vibrant palette: purple accent, cyan for secondary info, lavender for hints.

const ACCENT: (u8, u8, u8) = (165, 130, 255); // purple — primary
const ACCENT_SOFT: (u8, u8, u8) = (130, 200, 255); // sky blue — secondary
const MUTED: (u8, u8, u8) = (90, 220, 200); // teal/mint — info
const FAINT: (u8, u8, u8) = (200, 150, 230); // lavender — hints

fn accent(s: &str) -> colored::ColoredString {
    s.truecolor(ACCENT.0, ACCENT.1, ACCENT.2)
}
fn accent_soft(s: &str) -> colored::ColoredString {
    s.truecolor(ACCENT_SOFT.0, ACCENT_SOFT.1, ACCENT_SOFT.2)
}
fn muted(s: &str) -> colored::ColoredString {
    s.truecolor(MUTED.0, MUTED.1, MUTED.2)
}
fn faint(s: &str) -> colored::ColoredString {
    s.truecolor(FAINT.0, FAINT.1, FAINT.2)
}

fn print_prompt() {
    print!(
        "{}{}{}{} ",
        "[".truecolor(140, 100, 200),
        "spectra".truecolor(220, 180, 255).bold(),
        "]".truecolor(140, 100, 200),
        ">".truecolor(255, 140, 100).bold(),
    );
    io::stdout().flush().ok();
}

// ─── Banner / Logo ──────────────────────────────────────────────────────────

fn print_banner() {
    println!();
    println!(
        "  {}  {}",
        accent("spectra").bold(),
        muted("type 'help' to begin"),
    );
    println!();
}

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
        out.push_str(&format!(
            "\x1b[38;2;{};{};{}m{}\x1b[0m",
            r as u8, g as u8, b as u8, ch
        ));
    }
    out
}

fn print_logo() {
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
}

// ─── Section header (single style for every flow) ──────────────────────────

fn section_tinted(title: &str, subtitle: &str, rgb: (u8, u8, u8)) {
    println!();
    println!(
        "  {}  {}",
        "◆".truecolor(rgb.0, rgb.1, rgb.2).bold(),
        title.to_lowercase().truecolor(rgb.0, rgb.1, rgb.2).bold(),
    );
    if !subtitle.is_empty() {
        println!("     {}", subtitle.truecolor(rgb.0, rgb.1, rgb.2).dimmed());
    }
    println!();
}

// ─── Commands ────────────────────────────────────────────────────────────────

fn cmd_import(args: &[&str]) {
    ensure_dirs();
    section_tinted(
        "Simple Import",
        "restore wallet from seed phrase",
        (165, 130, 255),
    );

    let mut chain_arg: Option<String> = None;
    let mut name_arg: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--chain" if i + 1 < args.len() => {
                chain_arg = Some(args[i + 1].to_string());
                i += 2;
            }
            "--name" if i + 1 < args.len() => {
                name_arg = Some(args[i + 1].to_string());
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let chains = supported_chains();
    if chains.is_empty() {
        eprintln!(
            "{} No supported chains found in catalog.",
            accent("✗").bold()
        );
        return;
    }

    // 1. Select chain
    let selected = match &chain_arg {
        Some(name) => match chains.iter().find(|c| c.name.eq_ignore_ascii_case(name)) {
            Some(c) => c,
            None => {
                eprintln!(
                    "{} Unknown chain \"{}\".",
                    accent("✗").bold(),
                    name.yellow()
                );
                eprintln!("{}", "Supported chains:".dimmed());
                for c in &chains {
                    eprintln!("  {}", c.name.color(chain_color(&c.name)));
                }
                return;
            }
        },
        None => {
            let names: Vec<&str> = chains.iter().map(|c| c.name.as_str()).collect();
            let idx = match FuzzySelect::new()
                .with_prompt("Select a chain")
                .items(&names)
                .default(0)
                .interact_opt()
            {
                Ok(Some(i)) => i,
                _ => {
                    println!("{}", muted("cancelled"));
                    return;
                }
            };
            &chains[idx]
        }
    };

    println!(
        "  {} {}",
        muted("chain  "),
        selected.name.color(chain_color(&selected.name)).bold()
    );

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
                Err(_) => {
                    println!("{}", muted("cancelled"));
                    return;
                }
            }
        }
    };

    // 3. Seed phrase
    let seed_phrase = read_secret(&format!("  {} ", "Enter seed phrase:".dimmed()));
    let seed_trimmed = seed_phrase.trim().to_string();

    if seed_trimmed.is_empty() {
        eprintln!("{} Seed phrase cannot be empty.", accent("✗").bold());
        return;
    }

    if bip39::Mnemonic::parse_in(bip39::Language::English, &seed_trimmed).is_err() {
        eprintln!(
            "{} Invalid BIP39 mnemonic. Expected 12 or 24 words.",
            accent("✗").bold()
        );
        return;
    }

    let word_count = seed_trimmed.split_whitespace().count();
    println!("  {} {}-word mnemonic", "validated".green(), word_count);

    // Derive address
    let address =
        match derive_address_for_chain(&selected.name, &seed_trimmed, &selected.default_path) {
            Ok(address) => address,
            Err(e) => {
                eprintln!("{} Derivation failed: {e}", accent("✗").bold());
                return;
            }
        };

    // 4. Password
    let password = loop {
        let pw = read_secret(&format!("  {} ", "Set password:".dimmed()));
        if pw.trim().is_empty() {
            eprintln!("{} Password cannot be empty.", accent("✗").bold());
            continue;
        }
        let confirm = read_secret(&format!("  {} ", "Confirm password:".dimmed()));
        if pw != confirm {
            eprintln!("{} Passwords do not match.", accent("✗").bold());
            continue;
        }
        break pw;
    };

    // 5. Encrypt + persist secrets
    let wallet_id = uuid::Uuid::new_v4().to_string().to_uppercase();
    if let Err(e) = seal_wallet_secrets(&wallet_id, &seed_trimmed, &password) {
        eprintln!("{} Failed to store seed: {e}", accent("✗").bold());
        return;
    }

    // 6. Persist the wallet
    let mut state = load_state();
    state.wallets.push(WalletSummary::single_address(
        wallet_id,
        wallet_name.clone(),
        selected.name.clone(),
        address.clone(),
        Some(selected.default_path.clone()),
        false,
    ));
    save_state(&state);

    println!();
    println!(
        "  {} {}",
        accent("✓").bold(),
        "Wallet imported.".white().bold()
    );
    println!("  {} {}", muted("name   "), wallet_name.white().bold());
    println!(
        "  {} {}",
        muted("chain  "),
        selected.name.color(chain_color(&selected.name))
    );
    println!("  {} {}", muted("path   "), selected.default_path.dimmed());
    println!("  {} {}", muted("addr   "), address.bright_white());
}

// ─── Advanced import ────────────────────────────────────────────────────────

fn cmd_advimport() {
    ensure_dirs();
    section_tinted(
        "Advanced Import",
        "custom network and derivation path",
        (130, 200, 255),
    );

    let presets = load_chain_presets();
    if presets.is_empty() {
        eprintln!("{} No chain presets found.", accent("✗").bold());
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
        _ => {
            println!("{}", muted("cancelled"));
            return;
        }
    };
    let preset = &presets[chain_idx];
    println!(
        "  {} {}",
        muted("chain  "),
        preset.chain.color(chain_color(&preset.chain)).bold()
    );

    // 2. Select network (if multiple)
    let network = if preset.networks.len() > 1 {
        let labels: Vec<String> = preset
            .networks
            .iter()
            .map(|n| format!("{} — {}", n.title, n.detail))
            .collect();
        let default_idx = preset
            .networks
            .iter()
            .position(|n| n.is_default)
            .unwrap_or(0);
        let idx = match FuzzySelect::new()
            .with_prompt("Select network")
            .items(&labels)
            .default(default_idx)
            .interact_opt()
        {
            Ok(Some(i)) => i,
            _ => {
                println!("{}", muted("cancelled"));
                return;
            }
        };
        let net = &preset.networks[idx];
        println!("  {} {}", muted("net    "), net.title.white().bold());
        Some(net.network.clone())
    } else {
        preset.networks.first().map(|n| n.network.clone())
    };

    // 3. Select derivation path (if multiple, plus custom option)
    let derivation_path = if preset.derivation_paths.len() > 1 {
        let mut labels: Vec<String> = preset
            .derivation_paths
            .iter()
            .map(|p| format!("{:<20} {}", p.title, p.derivation_path.dimmed()))
            .collect();
        labels.push("Custom path...".to_string());
        let default_idx = preset
            .derivation_paths
            .iter()
            .position(|p| p.is_default)
            .unwrap_or(0);
        let idx = match FuzzySelect::new()
            .with_prompt("Select derivation path")
            .items(&labels)
            .default(default_idx)
            .interact_opt()
        {
            Ok(Some(i)) => i,
            _ => {
                println!("{}", muted("cancelled"));
                return;
            }
        };
        if idx == preset.derivation_paths.len() {
            // Custom path
            match Input::<String>::new()
                .with_prompt("Enter derivation path (e.g. m/44'/0'/0'/0/0)")
                .interact_text()
            {
                Ok(p) => {
                    println!("  {} {}", muted("path   "), p.white().bold());
                    p
                }
                Err(_) => {
                    println!("{}", muted("cancelled"));
                    return;
                }
            }
        } else {
            let path = &preset.derivation_paths[idx];
            println!(
                "  {} {} ({})",
                muted("path   "),
                path.derivation_path.white().bold(),
                path.title.dimmed()
            );
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
            _ => {
                println!("{}", muted("cancelled"));
                return;
            }
        };
        if idx == 1 {
            match Input::<String>::new()
                .with_prompt("Enter derivation path (e.g. m/44'/0'/0'/0/0)")
                .interact_text()
            {
                Ok(p) => {
                    println!("  {} {}", muted("path   "), p.white().bold());
                    p
                }
                Err(_) => {
                    println!("{}", muted("cancelled"));
                    return;
                }
            }
        } else {
            println!(
                "  {} {} ({})",
                muted("path   "),
                path.derivation_path.white().bold(),
                path.title.dimmed()
            );
            path.derivation_path.clone()
        }
    } else {
        eprintln!(
            "{} No derivation paths available for {}.",
            accent("✗").bold(),
            preset.chain
        );
        return;
    };

    // 4. Wallet name
    let wallet_name = match Input::<String>::new()
        .with_prompt("Wallet name")
        .default(format!("My {} Wallet", preset.chain))
        .interact_text()
    {
        Ok(n) => n,
        Err(_) => {
            println!("{}", muted("cancelled"));
            return;
        }
    };

    // 5. Seed phrase
    let seed_phrase = read_secret(&format!("  {} ", "Enter seed phrase:".dimmed()));
    let seed_trimmed = seed_phrase.trim().to_string();

    if seed_trimmed.is_empty() {
        eprintln!("{} Seed phrase cannot be empty.", accent("✗").bold());
        return;
    }

    if bip39::Mnemonic::parse_in(bip39::Language::English, &seed_trimmed).is_err() {
        eprintln!(
            "{} Invalid BIP39 mnemonic. Expected 12 or 24 words.",
            accent("✗").bold()
        );
        return;
    }

    let word_count = seed_trimmed.split_whitespace().count();
    println!("  {} {}-word mnemonic", "validated".green(), word_count);

    // Derive address
    let address = match derive_address_for_chain(&preset.chain, &seed_trimmed, &derivation_path) {
        Ok(address) => address,
        Err(e) => {
            eprintln!("{} Derivation failed: {e}", accent("✗").bold());
            return;
        }
    };

    // 6. Password
    let password = loop {
        let pw = read_secret(&format!("  {} ", "Set password:".dimmed()));
        if pw.trim().is_empty() {
            eprintln!("{} Password cannot be empty.", accent("✗").bold());
            continue;
        }
        let confirm = read_secret(&format!("  {} ", "Confirm password:".dimmed()));
        if pw != confirm {
            eprintln!("{} Passwords do not match.", accent("✗").bold());
            continue;
        }
        break pw;
    };

    // 7. Encrypt + persist secrets
    let wallet_id = uuid::Uuid::new_v4().to_string().to_uppercase();
    if let Err(e) = seal_wallet_secrets(&wallet_id, &seed_trimmed, &password) {
        eprintln!("{} Failed to store seed: {e}", accent("✗").bold());
        return;
    }

    // 8. Persist the wallet
    let mut state = load_state();
    state.wallets.push(WalletSummary::single_address(
        wallet_id,
        wallet_name.clone(),
        preset.chain.clone(),
        address.clone(),
        Some(derivation_path.clone()),
        false,
    ));
    save_state(&state);

    println!();
    println!(
        "  {} {}",
        accent("✓").bold(),
        "Wallet imported.".white().bold()
    );
    println!("  {} {}", muted("name   "), wallet_name.white().bold());
    println!(
        "  {} {}",
        muted("chain  "),
        preset.chain.color(chain_color(&preset.chain))
    );
    if let Some(ref net) = network {
        println!("  {} {}", muted("net    "), net.white());
    }
    println!("  {} {}", muted("path   "), derivation_path.dimmed());
    println!("  {} {}", muted("addr   "), address.bright_white());
}

// ─── New wallet ─────────────────────────────────────────────────────────────

fn cmd_newwallet() {
    ensure_dirs();
    section_tinted(
        "New Wallet",
        "generate fresh keys and seed phrase",
        (255, 200, 100),
    );

    let chains = supported_chains();
    if chains.is_empty() {
        eprintln!(
            "{} No supported chains found in catalog.",
            accent("✗").bold()
        );
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
        _ => {
            println!("{}", muted("cancelled"));
            return;
        }
    };
    let selected = &chains[chain_idx];
    println!(
        "  {} {}",
        muted("chain  "),
        selected.name.color(chain_color(&selected.name)).bold()
    );

    // 2. Word count
    let word_options = ["12 words", "24 words"];
    let word_idx = match FuzzySelect::new()
        .with_prompt("Seed phrase length")
        .items(&word_options)
        .default(0)
        .interact_opt()
    {
        Ok(Some(i)) => i,
        _ => {
            println!("{}", muted("cancelled"));
            return;
        }
    };
    let word_count: usize = if word_idx == 0 { 12 } else { 24 };

    // 3. Generate mnemonic
    let entropy_len = if word_count == 12 { 16 } else { 32 };
    let mut entropy = vec![0u8; entropy_len];
    rand_fill(&mut entropy);
    let mnemonic = match bip39::Mnemonic::from_entropy_in(bip39::Language::English, &entropy) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{} Failed to generate mnemonic: {e}", accent("✗").bold());
            return;
        }
    };
    let seed_phrase = mnemonic.to_string();

    println!();
    println!(
        "  {}  {}",
        accent("!").bold(),
        "save these words securely".white().bold()
    );
    println!(
        "     {}",
        muted("anyone with this phrase can spend your funds")
    );
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
    if !words.len().is_multiple_of(4) {
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
        Err(_) => {
            println!("{}", muted("cancelled"));
            return;
        }
    };

    // 5. Derive address
    let address =
        match derive_address_for_chain(&selected.name, &seed_phrase, &selected.default_path) {
            Ok(address) => address,
            Err(e) => {
                eprintln!("{} Derivation failed: {e}", accent("✗").bold());
                return;
            }
        };

    // 6. Password
    let password = loop {
        let pw = read_secret(&format!("  {} ", "Set password:".dimmed()));
        if pw.trim().is_empty() {
            eprintln!("{} Password cannot be empty.", accent("✗").bold());
            continue;
        }
        let confirm = read_secret(&format!("  {} ", "Confirm password:".dimmed()));
        if pw != confirm {
            eprintln!("{} Passwords do not match.", accent("✗").bold());
            continue;
        }
        break pw;
    };

    // 7. Encrypt + persist secrets
    let wallet_id = uuid::Uuid::new_v4().to_string().to_uppercase();
    if let Err(e) = seal_wallet_secrets(&wallet_id, &seed_phrase, &password) {
        eprintln!("{} Failed to store seed: {e}", accent("✗").bold());
        return;
    }

    // 8. Persist the wallet
    let mut state = load_state();
    state.wallets.push(WalletSummary::single_address(
        wallet_id,
        wallet_name.clone(),
        selected.name.clone(),
        address.clone(),
        Some(selected.default_path.clone()),
        false,
    ));
    save_state(&state);

    println!();
    println!(
        "  {} {}",
        accent("✓").bold(),
        "Wallet created.".white().bold()
    );
    println!("  {} {}", muted("name   "), wallet_name.white().bold());
    println!(
        "  {} {}",
        muted("chain  "),
        selected.name.color(chain_color(&selected.name))
    );
    println!("  {} {}", muted("path   "), selected.default_path.dimmed());
    println!("  {} {}", muted("addr   "), address.bright_white());
}

// ─── Watch-only import ──────────────────────────────────────────────────────

fn cmd_wimport() {
    ensure_dirs();
    section_tinted(
        "Watch Import",
        "read-only tracking by address",
        (90, 220, 200),
    );

    let chains = supported_chains();
    if chains.is_empty() {
        eprintln!(
            "{} No supported chains found in catalog.",
            accent("✗").bold()
        );
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
        _ => {
            println!("{}", muted("cancelled"));
            return;
        }
    };
    let selected = &chains[chain_idx];
    println!(
        "  {} {}",
        muted("chain  "),
        selected.name.color(chain_color(&selected.name)).bold()
    );

    // 2. Address
    let address: String = match Input::<String>::new()
        .with_prompt("Watch address")
        .interact_text()
    {
        Ok(a) => a.trim().to_string(),
        Err(_) => {
            println!("{}", muted("cancelled"));
            return;
        }
    };

    if address.is_empty() {
        eprintln!("{} Address cannot be empty.", accent("✗").bold());
        return;
    }

    // 3. Wallet name
    let wallet_name: String = match Input::new()
        .with_prompt("Wallet name")
        .default(format!("{} (watch)", selected.name))
        .interact_text()
    {
        Ok(n) => n,
        Err(_) => {
            println!("{}", muted("cancelled"));
            return;
        }
    };

    // 4. Persist (no secrets needed for watch-only)
    let wallet_id = uuid::Uuid::new_v4().to_string().to_uppercase();
    let mut state = load_state();
    state.wallets.push(WalletSummary::single_address(
        wallet_id,
        wallet_name.clone(),
        selected.name.clone(),
        address.clone(),
        None,
        true,
    ));
    save_state(&state);

    println!();
    println!(
        "  {} {}",
        accent("✓").bold(),
        "Watch-only wallet added.".white().bold()
    );
    println!("  {} {}", muted("name   "), wallet_name.white().bold());
    println!(
        "  {} {}",
        muted("chain  "),
        selected.name.color(chain_color(&selected.name))
    );
    println!("  {} {}", muted("addr   "), address.bright_white());
}

fn cmd_list() {
    let state = load_state();

    if state.wallets.is_empty() {
        println!();
        println!("  {}", muted("no wallets yet"));
        println!(
            "  {} {} {}",
            faint("run"),
            accent_soft("import"),
            faint("to add one"),
        );
        return;
    }

    println!();
    for w in &state.wallets {
        let dot = if w.is_watch_only {
            chain_paint("○", &w.chain_name)
        } else {
            chain_paint("●", &w.chain_name).bold()
        };
        let tag = if w.is_watch_only {
            faint(" watch").to_string()
        } else {
            String::new()
        };
        println!(
            "  {}  {}  {}{}",
            dot,
            w.name.white().bold(),
            chain_paint(&w.chain_name, &w.chain_name).bold(),
            tag,
        );
        println!("     {}", accent_soft(wallet_address(w)));
    }
    println!();
    println!(
        "  {} {}",
        accent(&format!("{}", state.wallets.len())).bold(),
        faint(if state.wallets.len() == 1 {
            "wallet"
        } else {
            "wallets"
        }),
    );
}

fn cmd_delete() {
    let mut state = load_state();

    if state.wallets.is_empty() {
        println!("  {} Nothing to delete.", "No wallets.".dimmed());
        return;
    }

    let labels: Vec<String> = state
        .wallets
        .iter()
        .map(|w| {
            let tag = if w.is_watch_only { " (watch)" } else { "" };
            format!("{} — {} — {}{}", w.name, w.chain_name, wallet_address(w), tag)
        })
        .collect();

    let idx = match FuzzySelect::new()
        .with_prompt("Select wallet to delete")
        .items(&labels)
        .interact_opt()
    {
        Ok(Some(i)) => i,
        _ => {
            println!("{}", muted("cancelled"));
            return;
        }
    };

    let wallet = &state.wallets[idx];
    let confirm_label = format!(
        "Delete \"{}\" ({})? Type '{}' to confirm",
        wallet.name,
        wallet.chain_name,
        "yes".red().bold()
    );
    let answer: String = match Input::<String>::new()
        .with_prompt(&confirm_label)
        .default("no".into())
        .interact_text()
    {
        Ok(a) => a,
        Err(_) => {
            println!("{}", muted("cancelled"));
            return;
        }
    };

    if answer.trim().to_lowercase() != "yes" {
        println!("{}", muted("  cancelled"));
        return;
    }

    let wallet_id = wallet.id.clone();
    let wallet_name = wallet.name.clone();
    let is_watch = wallet.is_watch_only;

    state.wallets.remove(idx);
    save_state(&state);
    // Keypool, owned addresses and history rows for this wallet go too —
    // `app_state_save` only rewrites the wallet list.
    if let Err(e) = wallet_db::delete_wallet_data(&db_path(), &wallet_id) {
        eprintln!("  {} failed to clear wallet data: {e}", accent("✗").bold());
    }

    if !is_watch {
        delete_wallet_secrets(&wallet_id);
    }

    println!(
        "  {} Wallet \"{}\" deleted.",
        accent("✓").bold(),
        wallet_name
    );
}

// ─── Chain-name → Chain enum mapping ────────────────────────────────────────

fn chain_id_for_name(name: &str) -> Option<String> {
    use spectra_core::registry::Chain;
    let n = name.trim();
    Chain::all()
        .find(|c| {
            c.chain_display_name().eq_ignore_ascii_case(n) || c.coin_name().eq_ignore_ascii_case(n)
        })
        .map(|c| c.str_id().to_string())
}

fn chain_native_symbol(chain_id: &str) -> &'static str {
    use spectra_core::registry::Chain;
    Chain::from_str_id(chain_id)
        .map(|c| c.coin_symbol())
        .unwrap_or("?")
}

// ─── Wallet picker (shared by balance/history/staking) ──────────────────────

fn pick_wallet<'a>(state: &'a CoreAppState, prompt: &str) -> Option<&'a WalletSummary> {
    if state.wallets.is_empty() {
        println!(
            "  {} Use {} to add one.",
            "No wallets.".dimmed(),
            "import".cyan()
        );
        return None;
    }
    let labels: Vec<String> = state
        .wallets
        .iter()
        .map(|w| {
            let tag = if w.is_watch_only { " (watch)" } else { "" };
            format!("{} — {} — {}{}", w.name, w.chain_name, wallet_address(w), tag)
        })
        .collect();
    let idx = match FuzzySelect::new()
        .with_prompt(prompt)
        .items(&labels)
        .interact_opt()
    {
        Ok(Some(i)) => i,
        _ => {
            println!("{}", muted("cancelled"));
            return None;
        }
    };
    Some(&state.wallets[idx])
}

// ─── Endpoint resolution + service construction ─────────────────────────────

const ENDPOINT_ROLE_BALANCE: u32 = 1 << 1;
const ENDPOINT_ROLE_HISTORY: u32 = 1 << 2;
const ENDPOINT_ROLE_RPC: u32 = 1 << 7;

fn endpoints_for_chain(chain_name: &str, role_mask: u32) -> Vec<String> {
    let recs =
        spectra_core::app_core_endpoint_records_for_chain(chain_name.to_string(), role_mask, false)
            .unwrap_or_default();
    recs.into_iter().map(|r| r.endpoint).collect()
}

fn build_service_for_chain(
    chain_id: String,
    chain_name: &str,
    role_mask: u32,
) -> Result<std::sync::Arc<spectra_core::service::WalletService>, String> {
    let endpoints = endpoints_for_chain(chain_name, role_mask);
    if endpoints.is_empty() {
        return Err(format!("No endpoints registered for {chain_name}."));
    }
    let chain_endpoints = vec![spectra_core::service::ChainEndpoints {
        chain_id,
        endpoints,
        api_key: None,
    }];
    spectra_core::service::WalletService::new_typed(chain_endpoints)
        .map_err(|e| format!("Failed to construct wallet service: {e}"))
}

// ─── balance command ────────────────────────────────────────────────────────

fn cmd_balance(rt: &tokio::runtime::Runtime) {
    let state = load_state();
    let wallet = match pick_wallet(&state, "Wallet for balance") {
        Some(w) => w.clone(),
        None => return,
    };
    let chain_id = match chain_id_for_name(&wallet.chain_name) {
        Some(id) => id,
        None => {
            eprintln!(
                "  {} Chain {} is not registered for balance lookups.",
                accent("?").bold(),
                wallet.chain_name
            );
            return;
        }
    };

    println!(
        "  {} {}",
        muted("→"),
        faint(&format!("fetching {}", wallet_address(&wallet)))
    );

    let service = match build_service_for_chain(
        chain_id.clone(),
        &wallet.chain_name,
        ENDPOINT_ROLE_BALANCE | ENDPOINT_ROLE_RPC,
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  {} {e}", accent("✗").bold());
            return;
        }
    };

    let result = rt.block_on(async {
        service
            .fetch_native_balance_summary(chain_id.clone(), wallet_address(&wallet).to_string())
            .await
    });

    match result {
        Ok(summary) => {
            let symbol = chain_native_symbol(&chain_id);
            println!();
            println!(
                "  {}  {} {}",
                chain_paint("●", &wallet.chain_name).bold(),
                summary.amount_display.white().bold(),
                chain_paint(symbol, &wallet.chain_name).bold(),
            );
            println!("     {} {}", muted("raw"), faint(&summary.smallest_unit),);
            if summary.utxo_count > 0 {
                println!(
                    "     {} {}",
                    muted("utxo"),
                    accent_soft(&summary.utxo_count.to_string()),
                );
            }
        }
        Err(e) => {
            eprintln!("  {} {e}", accent("✗").bold());
        }
    }
}

// ─── history command ────────────────────────────────────────────────────────

fn cmd_history(rt: &tokio::runtime::Runtime) {
    let state = load_state();
    let wallet = match pick_wallet(&state, "Wallet for history") {
        Some(w) => w.clone(),
        None => return,
    };
    let chain_id = match chain_id_for_name(&wallet.chain_name) {
        Some(id) => id,
        None => {
            eprintln!(
                "  {} Chain {} is not registered for history lookups.",
                accent("?").bold(),
                wallet.chain_name
            );
            return;
        }
    };

    println!(
        "  {} {}",
        muted("→"),
        faint(&format!("fetching {}", wallet_address(&wallet)))
    );

    let service = match build_service_for_chain(
        chain_id.clone(),
        &wallet.chain_name,
        ENDPOINT_ROLE_HISTORY | ENDPOINT_ROLE_BALANCE | ENDPOINT_ROLE_RPC,
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  {} {e}", accent("✗").bold());
            return;
        }
    };

    let result = rt.block_on(async {
        service
            .fetch_normalized_history(chain_id, wallet_address(&wallet).to_string())
            .await
    });

    let entries = match result {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  {} {e}", accent("✗").bold());
            return;
        }
    };

    println!();
    if entries.is_empty() {
        println!("  {}", muted("no transactions yet"));
        return;
    }

    println!(
        "  {}  {} {}",
        chain_paint("●", &wallet.chain_name).bold(),
        entries.len().to_string().white().bold(),
        chain_paint(
            if entries.len() == 1 {
                "transaction"
            } else {
                "transactions"
            },
            &wallet.chain_name
        ),
    );
    println!();

    for e in entries.iter().take(20) {
        let arrow = match e.kind.as_str() {
            "send" | "Send" => "↑".truecolor(255, 110, 130).bold(),
            "receive" | "Receive" => "↓".truecolor(120, 230, 160).bold(),
            _ => "·".truecolor(180, 180, 200),
        };
        let amount = match e.kind.as_str() {
            "send" | "Send" => format!("{:>10.4}", e.amount)
                .truecolor(255, 110, 130)
                .bold(),
            "receive" | "Receive" => format!("{:>10.4}", e.amount)
                .truecolor(120, 230, 160)
                .bold(),
            _ => format!("{:>10.4}", e.amount).white(),
        };
        let when = if e.timestamp > 0.0 {
            format_unix(e.timestamp as i64)
        } else {
            "—".to_string()
        };
        println!(
            "  {}  {} {}  {}  {}",
            arrow,
            amount,
            chain_paint(&e.symbol, &wallet.chain_name),
            accent_soft(&e.counterparty),
            faint(&when),
        );
        let hash_short = if e.tx_hash.len() > 20 {
            format!("{}…{}", &e.tx_hash[..10], &e.tx_hash[e.tx_hash.len() - 6..])
        } else {
            e.tx_hash.clone()
        };
        println!("     {}", faint(&hash_short));
    }
    if entries.len() > 20 {
        println!();
        println!("  {}", faint(&format!("+{} more", entries.len() - 20)));
    }
}

fn format_unix(ts: i64) -> String {
    if ts <= 0 {
        return "—".to_string();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = now - ts;
    if diff < 0 {
        return "future".to_string();
    }
    if diff < 60 {
        return format!("{diff}s ago");
    }
    if diff < 3600 {
        return format!("{}m ago", diff / 60);
    }
    if diff < 86400 {
        return format!("{}h ago", diff / 3600);
    }
    if diff < 86400 * 30 {
        return format!("{}d ago", diff / 86400);
    }
    if diff < 86400 * 365 {
        return format!("{}mo ago", diff / (86400 * 30));
    }
    format!("{}y ago", diff / (86400 * 365))
}

// ─── staking command ────────────────────────────────────────────────────────

fn cmd_staking(_rt: &tokio::runtime::Runtime) {
    let state = load_state();
    let wallet = match pick_wallet(&state, "Wallet for staking") {
        Some(w) => w.clone(),
        None => return,
    };

    let stakable = matches!(
        wallet.chain_name.as_str(),
        "Solana" | "Sui" | "Aptos" | "Polkadot" | "Internet Computer" | "ICP" | "NEAR" | "Cardano"
    );

    println!();
    if !stakable {
        println!(
            "  {}  {}",
            muted("·"),
            faint(&format!("staking unavailable on {}", wallet.chain_name))
        );
        println!(
            "     {}",
            faint("supports: Solana, Sui, Aptos, Polkadot, ICP, NEAR, Cardano")
        );
        return;
    }

    println!(
        "  {}  {}",
        accent("!").bold(),
        format!("staking on {} not yet wired up", wallet.chain_name).white(),
    );
    println!();
    println!(
        "     {}",
        faint("planned: positions, rewards, validators with APY")
    );
}

// ─── Wallet detail / show ────────────────────────────────────────────────────

fn cmd_show() {
    let state = load_state();
    let wallet = match pick_wallet(&state, "Wallet to inspect") {
        Some(w) => w.clone(),
        None => return,
    };
    let (r, g, b) = chain_rgb(&wallet.chain_name);
    section_tinted("Wallet Detail", &wallet.chain_name, (r, g, b));
    println!("  {} {}", muted("name   "), wallet.name.white().bold());
    println!(
        "  {} {}",
        muted("chain  "),
        chain_paint(&wallet.chain_name, &wallet.chain_name).bold()
    );
    println!(
        "  {} {}",
        muted("type   "),
        if wallet.is_watch_only {
            faint("watch-only")
        } else {
            accent_soft("hd · seed phrase")
        }
    );
    if let Some(ref path) = wallet.derivation_path {
        println!("  {} {}", muted("path   "), accent_soft(path));
    }
    println!("  {} {}", muted("addr   "), wallet_address(&wallet).white().bold());
    println!("  {} {}", muted("id     "), faint(&wallet.id));
}

// ─── Rename ─────────────────────────────────────────────────────────────────

fn cmd_rename() {
    let mut state = load_state();
    let labels: Vec<String> = state
        .wallets
        .iter()
        .map(|w| format!("{} — {}", w.name, w.chain_name))
        .collect();
    if labels.is_empty() {
        println!("  {}", muted("no wallets"));
        return;
    }
    let selection = FuzzySelect::new()
        .with_prompt("Wallet to rename")
        .items(&labels)
        .interact_opt();
    let idx = match selection {
        Ok(Some(i)) => i,
        _ => {
            println!("{}", muted("cancelled"));
            return;
        }
    };

    let current = state.wallets[idx].name.clone();
    let new_name: String = match Input::<String>::new()
        .with_prompt("New name")
        .with_initial_text(&current)
        .interact_text()
    {
        Ok(n) => n.trim().to_string(),
        Err(_) => {
            println!("{}", muted("cancelled"));
            return;
        }
    };

    if new_name.is_empty() {
        eprintln!("  {} name cannot be empty", accent("✗").bold());
        return;
    }

    state.wallets[idx].name = new_name.clone();
    save_state(&state);

    println!();
    println!(
        "  {} {} {} {}",
        accent("✓").bold(),
        faint(&current),
        muted("→"),
        new_name.white().bold(),
    );
}

// ─── Receive (show address) ─────────────────────────────────────────────────

fn cmd_receive() {
    let state = load_state();
    let wallet = match pick_wallet(&state, "Wallet to receive on") {
        Some(w) => w.clone(),
        None => return,
    };
    let (r, g, b) = chain_rgb(&wallet.chain_name);
    section_tinted(
        "Receive",
        &format!("{} address", wallet.chain_name),
        (r, g, b),
    );
    println!("  {}", wallet_address(&wallet).white().bold());
    println!();
    println!(
        "  {} {}",
        muted("symbol "),
        chain_paint(
            chain_native_symbol(
                chain_id_for_name(&wallet.chain_name)
                    .as_deref()
                    .unwrap_or("")
            ),
            &wallet.chain_name
        )
        .bold()
    );
    if let Some(ref p) = wallet.derivation_path {
        println!("  {} {}", muted("path   "), faint(p));
    }
    println!();
    println!(
        "  {} {}",
        accent_soft("→"),
        faint("share this address with the sender"),
    );
}

// ─── Export (reveal seed phrase) ────────────────────────────────────────────

fn cmd_export() {
    let state = load_state();
    let wallet = match pick_wallet(&state, "Wallet to export") {
        Some(w) => w.clone(),
        None => return,
    };

    if wallet.is_watch_only {
        println!();
        println!(
            "  {} {}",
            accent("!").bold(),
            "watch-only wallets have no seed phrase".white(),
        );
        return;
    }

    section_tinted(
        "Export Seed",
        "decrypt and display recovery phrase",
        (255, 130, 100),
    );

    println!(
        "  {}",
        "this will display your seed phrase in plain text.".truecolor(255, 180, 120)
    );
    println!(
        "  {}",
        faint("anyone watching your screen can take your funds.")
    );
    println!();

    let confirm: String = match Input::<String>::new()
        .with_prompt("Type 'yes' to continue")
        .default("no".into())
        .interact_text()
    {
        Ok(c) => c,
        Err(_) => {
            println!("{}", muted("cancelled"));
            return;
        }
    };
    if confirm.trim().to_lowercase() != "yes" {
        println!("{}", muted("  cancelled"));
        return;
    }

    let password = read_secret(&format!("  {} ", muted("password:")));

    let seed_phrase = match unlock_seed_phrase(&wallet.id, &password) {
        Ok(phrase) => phrase,
        Err(e) => {
            eprintln!("  {} {e}", accent("✗").bold());
            return;
        }
    };

    println!();
    let words: Vec<&str> = seed_phrase.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        let num = format!("{:>2}.", i + 1);
        let w = format!("{:<12}", word);
        if (i + 1) % 4 == 0 {
            println!("  {} {}", faint(&num), w.white().bold());
        } else {
            print!("  {} {}", faint(&num), w.white().bold());
        }
    }
    if !words.len().is_multiple_of(4) {
        println!();
    }
    println!();
    println!(
        "  {} {}",
        accent("!").bold(),
        "store this securely. clear your terminal when done.".truecolor(255, 180, 120),
    );
}

// ─── Send ───────────────────────────────────────────────────────────────────

fn cmd_send(rt: &tokio::runtime::Runtime) {
    let state = load_state();
    let wallet = match pick_wallet(&state, "Wallet to send from") {
        Some(w) => w.clone(),
        None => return,
    };

    if wallet.is_watch_only {
        println!();
        println!(
            "  {} {}",
            accent("!").bold(),
            "watch-only wallets cannot send".white()
        );
        return;
    }

    let chain_id = match chain_id_for_name(&wallet.chain_name) {
        Some(id) => id,
        None => {
            eprintln!(
                "  {} chain {} not registered",
                accent("?").bold(),
                wallet.chain_name
            );
            return;
        }
    };
    let derivation_path = match wallet.derivation_path.clone() {
        Some(p) => p,
        None => {
            eprintln!(
                "  {} wallet has no derivation path stored",
                accent("✗").bold()
            );
            return;
        }
    };

    let (r, g, b) = chain_rgb(&wallet.chain_name);
    section_tinted(
        "Send",
        &format!(
            "transfer {} from {}",
            chain_native_symbol(&chain_id),
            wallet.name
        ),
        (r, g, b),
    );

    let to_address: String = match Input::<String>::new()
        .with_prompt("To address")
        .interact_text()
    {
        Ok(a) => a.trim().to_string(),
        Err(_) => {
            println!("{}", muted("cancelled"));
            return;
        }
    };
    if to_address.is_empty() {
        eprintln!("  {} address cannot be empty", accent("✗").bold());
        return;
    }

    let amount_str: String = match Input::<String>::new()
        .with_prompt(format!("Amount ({})", chain_native_symbol(&chain_id)))
        .interact_text()
    {
        Ok(a) => a.trim().to_string(),
        Err(_) => {
            println!("{}", muted("cancelled"));
            return;
        }
    };
    let amount: f64 = match amount_str.parse() {
        Ok(v) if v > 0.0 => v,
        _ => {
            eprintln!("  {} invalid amount", accent("✗").bold());
            return;
        }
    };

    println!();
    println!(
        "  {} {} {} {}",
        muted("→ sending"),
        format!("{} {}", amount, chain_native_symbol(&chain_id))
            .white()
            .bold(),
        muted("to"),
        accent_soft(&to_address),
    );
    let confirm: String = match Input::<String>::new()
        .with_prompt("Type 'yes' to confirm")
        .default("no".into())
        .interact_text()
    {
        Ok(c) => c,
        Err(_) => {
            println!("{}", muted("cancelled"));
            return;
        }
    };
    if confirm.trim().to_lowercase() != "yes" {
        println!("{}", muted("  cancelled"));
        return;
    }

    // Decrypt seed
    let password = read_secret(&format!("  {} ", muted("password:")));
    let seed_phrase = match unlock_seed_phrase(&wallet.id, &password) {
        Ok(phrase) => phrase,
        Err(e) => {
            eprintln!("  {} {e}", accent("✗").bold());
            return;
        }
    };

    // Build service with broadcast endpoints
    let service = match build_service_for_chain(
        chain_id.clone(),
        &wallet.chain_name,
        ENDPOINT_ROLE_BALANCE | ENDPOINT_ROLE_RPC | (1 << 5) /* BROADCAST */ | (1 << 4) /* FEE */ | (1 << 3), /* UTXO */
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  {} {e}", accent("✗").bold());
            return;
        }
    };

    let request = spectra_core::send::SendExecutionRequest {
        chain_id,
        chain_name: wallet.chain_name.clone(),
        derivation_path,
        seed_phrase: Some(seed_phrase),
        private_key_hex: None,
        from_address: wallet_address(&wallet).to_string(),
        to_address: to_address.clone(),
        amount,
        amount_str: Some(amount_str.clone()),
        contract_address: None,
        token_decimals: None,
        fee_rate_svb: None,
        fee_sat: None,
        gas_budget: None,
        fee_amount: None,
        evm_overrides: None,
        monero_priority: None,
        derivation_overrides: None,
    };

    println!();
    println!("  {} {}", muted("→"), faint("signing and broadcasting…"));
    let result = rt.block_on(async { service.execute_send(request).await });

    match result {
        Ok(res) => {
            println!();
            println!(
                "  {} {}",
                accent("✓").bold(),
                "transaction broadcast".white().bold()
            );
            if !res.transaction_hash.is_empty() {
                println!(
                    "  {} {}",
                    muted("tx     "),
                    accent_soft(&res.transaction_hash)
                );
            }
        }
        Err(e) => {
            eprintln!(
                "  {} {}",
                accent("✗").bold(),
                format!("broadcast failed: {e}").white()
            );
        }
    }
}

// ─── Price ──────────────────────────────────────────────────────────────────

fn cmd_price(rt: &tokio::runtime::Runtime) {
    let state = load_state();
    if state.wallets.is_empty() {
        // Allow asking for any chain even with no wallets
        let chains = supported_chains();
        let names: Vec<&str> = chains.iter().map(|c| c.name.as_str()).collect();
        let idx = match FuzzySelect::new()
            .with_prompt("Which coin?")
            .items(&names)
            .default(0)
            .interact_opt()
        {
            Ok(Some(i)) => i,
            _ => {
                println!("{}", muted("cancelled"));
                return;
            }
        };
        return print_price_for_chain(rt, &chains[idx].name);
    }

    let unique_chains: std::collections::BTreeSet<String> =
        state.wallets.iter().map(|w| w.chain_name.clone()).collect();
    let chain_names: Vec<String> = unique_chains.into_iter().collect();
    let labels: Vec<&str> = chain_names.iter().map(|s| s.as_str()).collect();
    let idx = match FuzzySelect::new()
        .with_prompt("Which coin?")
        .items(&labels)
        .default(0)
        .interact_opt()
    {
        Ok(Some(i)) => i,
        _ => {
            println!("{}", muted("cancelled"));
            return;
        }
    };
    print_price_for_chain(rt, &chain_names[idx]);
}

fn print_price_for_chain(rt: &tokio::runtime::Runtime, chain_name: &str) {
    use spectra_core::registry::Chain;
    let chain_id = match chain_id_for_name(chain_name) {
        Some(id) => id,
        None => {
            eprintln!("  {} unknown chain", accent("✗").bold());
            return;
        }
    };
    let chain = Chain::from_str_id(&chain_id).unwrap();
    let symbol = chain.coin_symbol();
    let coin_gecko_id = chain.coin_gecko_id();

    println!(
        "  {} {}",
        muted("→"),
        faint(&format!("fetching {} price…", symbol))
    );

    let request = vec![spectra_core::price::PriceRequestCoin {
        holding_key: symbol.to_string(),
        symbol: symbol.to_string(),
        coin_gecko_id: coin_gecko_id.to_string(),
    }];

    // We construct a service even though pricing doesn't need chain endpoints
    let service = spectra_core::service::WalletService::new_typed(vec![]).unwrap();
    let result = rt.block_on(async {
        service
            .fetch_prices_typed("CoinGecko".to_string(), request)
            .await
    });

    match result {
        Ok(map) => {
            let price = map.get(symbol).copied().unwrap_or(0.0);
            println!();
            println!(
                "  {}  {} {}  {}",
                chain_paint("●", chain_name).bold(),
                format!("${:.2}", price).white().bold(),
                muted("USD"),
                chain_paint(symbol, chain_name).bold(),
            );
            println!("     {} {}", muted("via"), faint("CoinGecko"),);
        }
        Err(e) => {
            eprintln!("  {} {e}", accent("✗").bold());
        }
    }
}

// ─── Portfolio (sum balance × price across wallets) ─────────────────────────

fn cmd_portfolio(rt: &tokio::runtime::Runtime) {
    use spectra_core::registry::Chain;
    let state = load_state();
    if state.wallets.is_empty() {
        println!("  {}", muted("no wallets"));
        return;
    }

    section_tinted(
        "Portfolio",
        "balances × prices, summed in USD",
        (140, 230, 180),
    );

    // 1. Fetch prices for unique chains (one CoinGecko call).
    let unique_chains: std::collections::BTreeSet<String> =
        state.wallets.iter().map(|w| w.chain_name.clone()).collect();
    let mut requests = Vec::new();
    let mut symbol_for_chain: HashMap<String, String> = HashMap::new();
    for c in &unique_chains {
        if let Some(id) = chain_id_for_name(c) {
            let chain = Chain::from_str_id(&id).unwrap();
            requests.push(spectra_core::price::PriceRequestCoin {
                holding_key: c.clone(),
                symbol: chain.coin_symbol().to_string(),
                coin_gecko_id: chain.coin_gecko_id().to_string(),
            });
            symbol_for_chain.insert(c.clone(), chain.coin_symbol().to_string());
        }
    }
    let pricing = spectra_core::service::WalletService::new_typed(vec![]).unwrap();
    let prices = rt
        .block_on(async {
            pricing
                .fetch_prices_typed("CoinGecko".to_string(), requests)
                .await
        })
        .unwrap_or_default();

    // 2. Fetch balance for each wallet sequentially.
    let mut total_usd = 0.0;
    for w in &state.wallets {
        let chain_id = match chain_id_for_name(&w.chain_name) {
            Some(i) => i,
            None => continue,
        };
        let svc = match build_service_for_chain(
            chain_id.clone(),
            &w.chain_name,
            ENDPOINT_ROLE_BALANCE | ENDPOINT_ROLE_RPC,
        ) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let bal_res = rt.block_on(async {
            svc.fetch_native_balance_summary(chain_id, wallet_address(w).to_string())
                .await
        });
        let amount: f64 = bal_res
            .ok()
            .and_then(|s| s.amount_display.parse().ok())
            .unwrap_or(0.0);
        let price = prices.get(&w.chain_name).copied().unwrap_or(0.0);
        let usd = amount * price;
        total_usd += usd;
        let symbol = symbol_for_chain
            .get(&w.chain_name)
            .cloned()
            .unwrap_or_else(|| "?".into());
        let dot = if w.is_watch_only {
            chain_paint("○", &w.chain_name)
        } else {
            chain_paint("●", &w.chain_name).bold()
        };
        println!(
            "  {}  {:<14}  {:>14}  {}  {}",
            dot,
            w.name.white(),
            format!("{:.4} {}", amount, symbol).truecolor(220, 220, 230),
            faint(&format!("@ ${:.2}", price)),
            format!("${:.2}", usd).white().bold(),
        );
    }
    println!();
    println!(
        "  {}  {} {}",
        accent("Σ").bold(),
        format!("${:.2}", total_usd).white().bold(),
        muted("USD"),
    );
}

fn cmd_about() {
    let chains = supported_chains();
    print_logo();
    println!();
    println!("  {}", muted("multi-chain self-custody wallet"));
    println!();

    let label_w = 10;
    println!(
        "  {}  {}",
        format!("{:<label_w$}", "encryption").white().bold(),
        faint("AES-256-GCM · PBKDF2-HMAC-SHA256"),
    );
    println!(
        "  {}  {}",
        format!("{:<label_w$}", "storage").white().bold(),
        faint(&data_dir().display().to_string()),
    );
    println!(
        "  {}  {} {}",
        format!("{:<label_w$}", "chains").white().bold(),
        accent_soft(&chains.len().to_string()),
        faint("supported"),
    );
    println!();
    println!("  {}", muted("supported chains"));

    let names: Vec<String> = chains.iter().map(|c| c.name.clone()).collect();
    let col_width = names.iter().map(|n| n.len()).max().unwrap_or(0) + 3;
    let term_width = 72;
    let cols = (term_width / col_width).max(1);

    for row in names.chunks(cols) {
        print!("  ");
        for name in row {
            let s = name.color(chain_color(name));
            let padding = col_width.saturating_sub(name.len());
            print!("{}{}", s, " ".repeat(padding));
        }
        println!();
    }
    println!();
    println!("  {}", faint("github.com/sheny6n/SpectraWallet"));
}

fn cmd_import_menu() {
    let options = [
        "new wallet      generate a new seed phrase",
        "simple import   restore from seed phrase",
        "advanced        custom network & derivation",
        "watch only      track an address read-only",
    ];
    let idx = match FuzzySelect::new()
        .with_prompt("how")
        .items(&options)
        .default(0)
        .interact_opt()
    {
        Ok(Some(i)) => i,
        _ => {
            println!("{}", muted("cancelled"));
            return;
        }
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
    type Group<'a> = (&'a str, (u8, u8, u8), &'a [(&'a str, &'a str)]);
    let groups: &[Group] = &[
        (
            "wallet",
            (165, 130, 255),
            &[
                ("import", "import a wallet (choose type)"),
                ("nw, newwallet", "generate a new wallet"),
                ("si, simport", "import from seed phrase"),
                ("ai, advimport", "import with custom path"),
                ("wi, wimport", "watch-only by address"),
                ("ls, list", "list wallets"),
                ("show, info", "show full wallet detail"),
                ("mv, rename", "rename a wallet"),
                ("rm, delete", "delete a wallet"),
                ("export", "reveal seed phrase (password)"),
            ],
        ),
        (
            "activity",
            (90, 220, 200),
            &[
                ("bal, balance", "on-chain balance"),
                ("hist, history", "transaction history"),
                ("recv, receive", "show address to receive funds"),
                ("send, tx", "send a transaction"),
                ("stake, staking", "staking info"),
            ],
        ),
        (
            "market",
            (130, 200, 255),
            &[
                ("price", "current USD price (CoinGecko)"),
                ("p, portfolio", "total balance × price across wallets"),
            ],
        ),
        (
            "system",
            (255, 200, 100),
            &[
                ("about", "about spectra"),
                ("help, ?", "show this help"),
                ("q, quit", "exit"),
            ],
        ),
    ];

    for (group, rgb, items) in groups {
        let (r, g, b) = *rgb;
        println!();
        println!(
            "  {} {}",
            "▎".truecolor(r, g, b).bold(),
            group.truecolor(r, g, b).bold(),
        );
        for (cmd, desc) in *items {
            println!(
                "    {}  {}",
                format!("{:<18}", cmd).truecolor(r, g, b),
                faint(desc),
            );
        }
    }
    println!();
}

// ─── Interactive shell ───────────────────────────────────────────────────────

fn run_shell(rt: &tokio::runtime::Runtime) {
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
            "show" | "info" => cmd_show(),
            "rename" | "mv" => cmd_rename(),
            "receive" | "recv" => cmd_receive(),
            "export" => cmd_export(),
            "send" | "tx" => cmd_send(rt),
            "balance" | "bal" => cmd_balance(rt),
            "history" | "hist" => cmd_history(rt),
            "staking" | "stake" => cmd_staking(rt),
            "price" => cmd_price(rt),
            "portfolio" | "p" => cmd_portfolio(rt),
            "about" => cmd_about(),
            "help" | "?" => cmd_help(),
            "quit" | "exit" | "q" => {
                println!("  {}", muted("bye"));
                break;
            }
            other => {
                eprintln!(
                    "  {}  {} {}",
                    accent("?").bold(),
                    faint(&format!("unknown: {other}")),
                    faint("· try 'help'"),
                );
            }
        }

        println!();
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    // Up front, not lazily per command: `wallet_db` opens a SQLite file and
    // needs its parent directory to exist before the first read, so a fresh
    // install must not have `list` fail before `import` has ever run.
    ensure_dirs();
    let rt = tokio::runtime::Runtime::new().expect("failed to start tokio runtime");
    run_shell(&rt);
}
