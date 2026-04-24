//! Platform — app-wide static/bootstrap modules that don't fit under
//! a chain-function (derivation / fetch / send) or business area
//! (store / registry). Localization tables, formatting helpers,
//! resource loaders, the capabilities catalog, and shared UniFFI
//! types all live here.

pub mod catalog;
pub mod formatting;
pub mod localization;
pub mod resources;
pub mod types;
