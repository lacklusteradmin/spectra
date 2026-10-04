//! Provider pagination is part of the wire contract, before transfer filtering.
use crate::api::error::ApiError;

#[derive(Debug, Clone)]
pub struct HistoryPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

pub(crate) fn page_number(cursor: Option<&str>) -> Result<u32, ApiError> {
    cursor.map_or(Ok(1), |value| {
        value
            .parse::<u32>()
            .ok()
            .filter(|page| *page > 0 && *page < u32::MAX)
            .ok_or_else(|| ApiError::InvalidInput("invalid history page cursor".into()))
    })
}

pub(crate) fn query_value(value: &str) -> String {
    reqwest::Url::parse_with_params("https://cursor.invalid", &[("cursor", value)])
        .expect("static cursor URL")
        .query()
        .expect("one cursor parameter")
        .trim_start_matches("cursor=")
        .to_string()
}
