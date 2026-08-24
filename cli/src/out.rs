//! Terminal rendering and its `--json` counterpart. Every command writes both
//! at the same call site, so one cannot quietly fall behind the other.
//!
//! The colour table here maps a *semantic* name to a terminal colour. Which
//! colour a chain has is a chain fact and comes from `chains.toml`.

use std::collections::HashMap;
use std::sync::LazyLock;

use colored::Colorize;

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

    pub fn text(self, body: impl FnOnce()) {
        if !self.json {
            body();
        }
    }

    pub fn emit(self, value: serde_json::Value) {
        if self.json {
            println!("{}", value);
        }
    }
}

// ─── Palette ────────────────────────────────────────────────────────────────

const ACCENT: (u8, u8, u8) = (165, 130, 255);
const INFO: (u8, u8, u8) = (130, 200, 255);
const HINT: (u8, u8, u8) = (150, 150, 170);

pub fn accent(s: &str) -> colored::ColoredString {
    s.truecolor(ACCENT.0, ACCENT.1, ACCENT.2)
}

pub fn info(s: &str) -> colored::ColoredString {
    s.truecolor(INFO.0, INFO.1, INFO.2)
}

pub fn hint(s: &str) -> colored::ColoredString {
    s.truecolor(HINT.0, HINT.1, HINT.2)
}

pub fn ok_mark() -> colored::ColoredString {
    "✓".truecolor(120, 230, 160).bold()
}

pub fn fail_mark() -> colored::ColoredString {
    "✗".truecolor(255, 110, 130).bold()
}

pub fn field(label: &str, value: &str) {
    println!("  {}  {}", hint(&format!("{label:<8}")), value);
}

// ─── Chain tint ─────────────────────────────────────────────────────────────

/// Built from `chains.toml`, so a new chain is tinted without a change here.
/// The previous CLI hardcoded 30 of the 78 and rendered the rest grey.
static CHAIN_COLOR: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    spectra_core::chains::list_all_chains()
        .into_iter()
        .map(|chain| (chain.name, chain.color))
        .collect()
});

fn rgb_for_color_name(name: &str) -> (u8, u8, u8) {
    match name {
        "orange" => (247, 147, 26),
        "yellow" => (240, 185, 11),
        "green" => (120, 220, 130),
        "mint" => (140, 240, 200),
        "teal" => (90, 210, 200),
        "cyan" => (100, 210, 245),
        "blue" => (90, 140, 245),
        "indigo" => (120, 120, 240),
        "purple" => (165, 130, 255),
        "pink" => (240, 110, 190),
        "red" => (240, 85, 95),
        "gray" | "grey" => (170, 170, 185),
        _ => (200, 200, 210),
    }
}

fn chain_rgb(chain_name: &str) -> (u8, u8, u8) {
    CHAIN_COLOR
        .get(chain_name)
        .map(|name| rgb_for_color_name(name))
        .unwrap_or((200, 200, 210))
}

pub fn tint(s: &str, chain_name: &str) -> colored::ColoredString {
    let (r, g, b) = chain_rgb(chain_name);
    s.truecolor(r, g, b)
}

/// Filled dot for a spending wallet, hollow for watch-only.
pub fn wallet_dot(chain_name: &str, is_watch_only: bool) -> colored::ColoredString {
    if is_watch_only {
        tint("○", chain_name)
    } else {
        tint("●", chain_name).bold()
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
    if hash.len() > 20 {
        format!("{}…{}", &hash[..10], &hash[hash.len() - 6..])
    } else {
        hash.to_string()
    }
}
