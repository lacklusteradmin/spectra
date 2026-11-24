//! The Sapling proving parameters, downloaded from where Zcash publishes
//! them. `send::zcash_shielded` pins what each file must hash to.

use crate::api::error::ApiError;
use crate::api::http::HttpClient;

const SAPLING_PARAMETERS_URL: &str = "https://download.z.cash/downloads";

/// The published parameter file `name`, refused past `size` bytes.
pub(crate) async fn fetch_sapling_parameter(name: &str, size: usize) -> Result<Vec<u8>, ApiError> {
    HttpClient::shared()
        .get_bytes(&format!("{SAPLING_PARAMETERS_URL}/{name}"), size)
        .await
}
