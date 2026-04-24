use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

pub const LOCALIZATION_TABLES: [&str; 7] = [
    "ChainWikiEntries",
    "CommonContent",
    "DiagnosticsContent",
    "DonationsContent",
    "EndpointsContent",
    "ImportFlowContent",
    "SettingsContent",
];

#[derive(Debug, Clone)]
pub struct LocalizationCatalog {
    /// locale → table → parsed JSON value. Nested map allows O(1) lookup
    /// with `&str` keys, avoiding String allocation on every call.
    documents: HashMap<String, HashMap<String, Value>>,
    supported_locales: Vec<String>,
}

#[derive(Clone, Copy)]
struct EmbeddedDocument {
    locale: &'static str,
    table: &'static str,
    json: &'static str,
}

static LOCALIZATION_CATALOG: OnceLock<Result<LocalizationCatalog, String>> = OnceLock::new();

const EMBEDDED_DOCUMENTS: &[EmbeddedDocument] = &[
    EmbeddedDocument {
        locale: "Base",
        table: "ChainWikiEntries",
        json: include_str!("../../../resources/strings/base/ChainWikiEntries.json"),
    },
    EmbeddedDocument {
        locale: "Base",
        table: "CommonContent",
        json: include_str!("../../../resources/strings/base/CommonContent.json"),
    },
    EmbeddedDocument {
        locale: "Base",
        table: "DiagnosticsContent",
        json: include_str!("../../../resources/strings/base/DiagnosticsContent.json"),
    },
    EmbeddedDocument {
        locale: "Base",
        table: "DonationsContent",
        json: include_str!("../../../resources/strings/base/DonationsContent.json"),
    },
    EmbeddedDocument {
        locale: "Base",
        table: "EndpointsContent",
        json: include_str!("../../../resources/strings/base/EndpointsContent.json"),
    },
    EmbeddedDocument {
        locale: "Base",
        table: "ImportFlowContent",
        json: include_str!("../../../resources/strings/base/ImportFlowContent.json"),
    },
    EmbeddedDocument {
        locale: "Base",
        table: "SettingsContent",
        json: include_str!("../../../resources/strings/base/SettingsContent.json"),
    },
    EmbeddedDocument {
        locale: "en",
        table: "ChainWikiEntries",
        json: include_str!("../../../resources/strings/en/ChainWikiEntries.en.json"),
    },
    EmbeddedDocument {
        locale: "en",
        table: "CommonContent",
        json: include_str!("../../../resources/strings/en/CommonContent.en.json"),
    },
    EmbeddedDocument {
        locale: "en",
        table: "DiagnosticsContent",
        json: include_str!("../../../resources/strings/en/DiagnosticsContent.en.json"),
    },
    EmbeddedDocument {
        locale: "en",
        table: "DonationsContent",
        json: include_str!("../../../resources/strings/en/DonationsContent.en.json"),
    },
    EmbeddedDocument {
        locale: "en",
        table: "EndpointsContent",
        json: include_str!("../../../resources/strings/en/EndpointsContent.en.json"),
    },
    EmbeddedDocument {
        locale: "en",
        table: "ImportFlowContent",
        json: include_str!("../../../resources/strings/en/ImportFlowContent.en.json"),
    },
    EmbeddedDocument {
        locale: "en",
        table: "SettingsContent",
        json: include_str!("../../../resources/strings/en/SettingsContent.en.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hans",
        table: "ChainWikiEntries",
        json: include_str!("../../../resources/strings/zh-Hans/ChainWikiEntries.zh-Hans.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hans",
        table: "CommonContent",
        json: include_str!("../../../resources/strings/zh-Hans/CommonContent.zh-Hans.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hans",
        table: "DiagnosticsContent",
        json: include_str!("../../../resources/strings/zh-Hans/DiagnosticsContent.zh-Hans.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hans",
        table: "DonationsContent",
        json: include_str!("../../../resources/strings/zh-Hans/DonationsContent.zh-Hans.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hans",
        table: "EndpointsContent",
        json: include_str!("../../../resources/strings/zh-Hans/EndpointsContent.zh-Hans.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hans",
        table: "ImportFlowContent",
        json: include_str!("../../../resources/strings/zh-Hans/ImportFlowContent.zh-Hans.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hans",
        table: "SettingsContent",
        json: include_str!("../../../resources/strings/zh-Hans/SettingsContent.zh-Hans.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hant",
        table: "ChainWikiEntries",
        json: include_str!("../../../resources/strings/zh-Hant/ChainWikiEntries.zh-Hant.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hant",
        table: "CommonContent",
        json: include_str!("../../../resources/strings/zh-Hant/CommonContent.zh-Hant.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hant",
        table: "DiagnosticsContent",
        json: include_str!("../../../resources/strings/zh-Hant/DiagnosticsContent.zh-Hant.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hant",
        table: "DonationsContent",
        json: include_str!("../../../resources/strings/zh-Hant/DonationsContent.zh-Hant.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hant",
        table: "EndpointsContent",
        json: include_str!("../../../resources/strings/zh-Hant/EndpointsContent.zh-Hant.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hant",
        table: "ImportFlowContent",
        json: include_str!("../../../resources/strings/zh-Hant/ImportFlowContent.zh-Hant.json"),
    },
    EmbeddedDocument {
        locale: "zh-Hant",
        table: "SettingsContent",
        json: include_str!("../../../resources/strings/zh-Hant/SettingsContent.zh-Hant.json"),
    },
];

pub fn localization_catalog() -> Result<&'static LocalizationCatalog, String> {
    match LOCALIZATION_CATALOG.get_or_init(load_localization_catalog) {
        Ok(catalog) => Ok(catalog),
        Err(message) => Err(message.clone()),
    }
}

impl LocalizationCatalog {
    pub fn supported_locales(&self) -> Vec<String> {
        self.supported_locales.clone()
    }

    pub fn document_for(&self, preferred_locales: &[String], table: &str) -> Option<&Value> {
        let candidates = locale_candidates(preferred_locales);
        for locale in &candidates {
            if let Some(table_map) = self.documents.get(locale.as_str()) {
                if let Some(value) = table_map.get(table) {
                    return Some(value);
                }
            }
        }
        None
    }
}

fn load_localization_catalog() -> Result<LocalizationCatalog, String> {
    let mut documents: HashMap<String, HashMap<String, Value>> = HashMap::new();
    let mut supported_locales = BTreeSet::new();

    for embedded in EMBEDDED_DOCUMENTS {
        let value = serde_json::from_str::<Value>(embedded.json).map_err(display_error)?;
        documents
            .entry(embedded.locale.to_string())
            .or_default()
            .insert(embedded.table.to_string(), value);
        supported_locales.insert(embedded.locale.to_string());
    }

    Ok(LocalizationCatalog {
        documents,
        supported_locales: supported_locales.into_iter().collect(),
    })
}

fn locale_candidates(preferred_locales: &[String]) -> Vec<String> {
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();

    for locale in preferred_locales {
        for fallback in locale_fallbacks(locale) {
            if seen.insert(fallback.clone()) {
                ordered.push(fallback);
            }
        }
    }

    for fallback in ["en", "Base"] {
        let fallback = fallback.to_string();
        if seen.insert(fallback.clone()) {
            ordered.push(fallback);
        }
    }

    ordered
}

fn locale_fallbacks(locale: &str) -> Vec<String> {
    let normalized = locale.replace('_', "-");
    let parts = normalized.split('-').collect::<Vec<_>>();
    if parts.is_empty() {
        return Vec::new();
    }

    let mut fallbacks = Vec::new();
    for index in (1..=parts.len()).rev() {
        let candidate = parts[..index].join("-");
        fallbacks.push(candidate.clone());
        let lowered = candidate.to_lowercase();
        if lowered.starts_with("zh-hans") {
            fallbacks.push("zh-Hans".to_string());
        }
        if lowered.starts_with("zh-hant") {
            fallbacks.push("zh-Hant".to_string());
        }
    }

    fallbacks
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_from_regional_locale() {
        let catalog = localization_catalog().expect("catalog");
        let document = catalog
            .document_for(&["zh-Hans-CN".to_string()], "CommonContent")
            .expect("document");
        assert!(document
            .get("walletImportErrorTitle")
            .and_then(|value| value.as_str())
            .is_some());
    }

    #[test]
    fn falls_back_to_base_when_locale_missing() {
        let catalog = localization_catalog().expect("catalog");
        let document = catalog
            .document_for(&["fr-FR".to_string()], "DiagnosticsContent")
            .expect("document");
        assert!(document.get("navigationTitle").is_some());
    }
}
