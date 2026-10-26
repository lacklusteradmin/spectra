//! The catalogs embedded at build time from `core/data/`: chains, tokens,
//! endpoints, explorers, donations, the wiki and Monero restore heights.
//! Each is validated on first use and panics when broken, since a broken
//! embedded file is a build defect.

pub mod chains;
pub mod donations;
pub mod endpoints;
pub mod explorers;
pub mod restore_heights;
pub mod tokens;
pub mod wiki;
