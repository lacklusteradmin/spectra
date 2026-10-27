//! What a command prints. Every command ends by writing one [`Report`]: the
//! same value as one JSON document under `--json`, and as its own text
//! otherwise, so neither form can be missing or say something the other does
//! not.
//!
//! The colour table here maps a *semantic* name to a terminal colour. Which
//! colour a chain has is a chain fact and comes from `chains.toml`.

use std::sync::atomic::{AtomicBool, Ordering};

use colored::Colorize;
use serde::Serialize;
use spectra_core::catalog::chains::CatalogColor;
use spectra_core::registry::Chain;
use spectra_core::store::state::FiatCurrency;

use crate::error::{CliError, CliResult};

/// Set once a JSON document is written. `--json` prints exactly one document,
/// so `main` prints its error object only when the command printed nothing:
/// a command that reports a verdict and then fails has already said why.
static EMITTED: AtomicBool = AtomicBool::new(false);

/// What a command found or did, for a script and for a person.
///
/// Under `--json` the report is serialized beside `ok`; otherwise
/// [`Report::render`] prints it. A report is the command's whole answer:
/// progress goes to stderr through [`progress`].
pub trait Report: Serialize {
    /// The report as a person reads it in a terminal.
    fn render(&self);

    /// The report's verdict: `false` when it carries a failure it found —
    /// an unreachable endpoint, a failed self-test, a refresh error.
    fn ok(&self) -> bool {
        true
    }
}

/// A change with nothing to report beyond having been made.
#[derive(Serialize)]
pub struct Done {
    #[serde(skip)]
    what: String,
}

impl Done {
    pub fn new(what: impl Into<String>) -> Self {
        Self { what: what.into() }
    }
}

impl Report for Done {
    fn render(&self) {
        println!("  {} {}", ok_mark(), self.what);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Out {
    json: bool,
}

impl Out {
    pub fn new(json: bool) -> Self {
        if json {
            // JSON is for machines; ANSI escapes in it are noise at best and
            // a parse failure at worst.
            colored::control::set_override(false);
        }
        Self { json }
    }

    /// Write the command's report.
    pub fn report(self, report: &impl Report) -> CliResult<()> {
        if !self.json {
            report.render();
            return Ok(());
        }
        #[derive(Serialize)]
        struct Document<'a, R> {
            ok: bool,
            #[serde(flatten)]
            report: &'a R,
        }
        let document = serde_json::to_string(&Document {
            ok: report.ok(),
            report,
        })
        .map_err(|error| CliError::failure(format!("cannot write the report: {error}")))?;
        println!("{document}");
        EMITTED.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Say why the command failed: on stderr, and as the JSON document when
    /// the command reported nothing before failing.
    pub fn fail(self, error: &CliError) {
        if !self.json {
            eprintln!("  {} {}", fail_mark(), error);
        } else if !EMITTED.load(Ordering::Relaxed) {
            println!(
                "{}",
                serde_json::json!({ "ok": false, "error": error.message })
            );
        }
    }
}

/// A line about work under way, on stderr so it never mixes with a report.
pub fn progress(text: &str) {
    eprintln!("  {} {text}", hint("→"));
}

// ─── Palette ────────────────────────────────────────────────────────────────

const ACCENT: (u8, u8, u8) = (165, 130, 255);
const INFO: (u8, u8, u8) = (130, 200, 255);
const HINT: (u8, u8, u8) = (150, 150, 170);
const UP: (u8, u8, u8) = (120, 230, 160);
const DOWN: (u8, u8, u8) = (255, 110, 130);

pub fn accent(s: &str) -> colored::ColoredString {
    s.truecolor(ACCENT.0, ACCENT.1, ACCENT.2)
}

pub fn info(s: &str) -> colored::ColoredString {
    s.truecolor(INFO.0, INFO.1, INFO.2)
}

pub fn hint(s: &str) -> colored::ColoredString {
    s.truecolor(HINT.0, HINT.1, HINT.2)
}

/// Money arriving, or a check that passed.
pub fn up(s: &str) -> colored::ColoredString {
    s.truecolor(UP.0, UP.1, UP.2)
}

/// Money leaving, or a check that failed.
pub fn down(s: &str) -> colored::ColoredString {
    s.truecolor(DOWN.0, DOWN.1, DOWN.2)
}

pub fn ok_mark() -> colored::ColoredString {
    up("✓").bold()
}

pub fn fail_mark() -> colored::ColoredString {
    down("✗").bold()
}

/// A labelled line, the label padded to eight columns.
pub fn field(label: &str, value: &str) {
    wide_field(8, label, value);
}

/// A labelled line, the label padded to `width` columns.
pub fn wide_field(width: usize, label: &str, value: &str) {
    println!("  {}  {}", hint(&format!("{label:<width$}")), value);
}

/// `count` of `noun`, with an `s` unless there is one.
pub fn count(count: usize, noun: &str) -> String {
    format!(
        "{} {}",
        accent(&count.to_string()).bold(),
        hint(&format!("{noun}{}", if count == 1 { "" } else { "s" }))
    )
}

/// A value in the display currency, to the places the currency shows;
/// a dash when there is no price.
pub fn fiat(value: Option<f64>, currency: FiatCurrency) -> String {
    let places = spectra_core::amount::presentation::fiat_amount_rules(currency).decimals as usize;
    value.map_or_else(|| "—".into(), |value| format!("{value:.places$}"))
}

/// A serde unit variant's name, as `--json` spells it.
pub fn serde_name(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

// ─── Chain tint ─────────────────────────────────────────────────────────────

/// Built from `chains.toml`, so a new chain is tinted without a change here.
/// Exhaustive over the catalog's palette, so a colour added there is a compile
/// error here rather than a grey chain.
fn rgb_for_color(color: CatalogColor) -> (u8, u8, u8) {
    match color {
        CatalogColor::Orange => (247, 147, 26),
        CatalogColor::Yellow => (240, 185, 11),
        CatalogColor::Green => (120, 220, 130),
        CatalogColor::Mint => (140, 240, 200),
        CatalogColor::Teal => (90, 210, 200),
        CatalogColor::Cyan => (100, 210, 245),
        CatalogColor::Blue => (90, 140, 245),
        CatalogColor::Indigo => (120, 120, 240),
        CatalogColor::Purple => (165, 130, 255),
        CatalogColor::Pink => (240, 110, 190),
        CatalogColor::Red => (240, 85, 95),
        CatalogColor::Gray => (170, 170, 185),
    }
}

pub fn tint(s: &str, chain: Chain) -> colored::ColoredString {
    let (r, g, b) = rgb_for_color(chain.entry().color);
    s.truecolor(r, g, b)
}

/// A chain's display name in its colour.
pub fn chain(chain: Chain) -> colored::ColoredString {
    tint(chain.chain_display_name(), chain)
}

/// Filled dot for a spending wallet, hollow for watch-only.
pub fn wallet_dot(chain: Chain, is_watch_only: bool) -> colored::ColoredString {
    if is_watch_only {
        tint("○", chain)
    } else {
        tint("●", chain).bold()
    }
}

pub fn relative_time(ts: i64) -> String {
    if ts <= 0 {
        return "—".to_string();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = now - ts;
    match diff {
        d if d < 0 => "future".to_string(),
        d if d < 60 => format!("{d}s ago"),
        d if d < 3_600 => format!("{}m ago", d / 60),
        d if d < 86_400 => format!("{}h ago", d / 3_600),
        d if d < 86_400 * 30 => format!("{}d ago", d / 86_400),
        d if d < 86_400 * 365 => format!("{}mo ago", d / (86_400 * 30)),
        d => format!("{}y ago", d / (86_400 * 365)),
    }
}

pub fn short_hash(hash: &str) -> String {
    if hash.chars().count() > 20 {
        let chars: Vec<char> = hash.chars().collect();
        let head: String = chars[..10].iter().collect();
        let tail: String = chars[chars.len() - 6..].iter().collect();
        format!("{head}…{tail}")
    } else {
        hash.to_string()
    }
}
