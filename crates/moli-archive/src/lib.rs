//! Standalone durable archive, separated from the household command path.
pub mod protocol;
pub mod receipt;

mod manifest;
mod parquet;
mod store;
pub use manifest::Snapshot;
pub use store::{Archive, PublicationFailure};
pub mod http;
mod merge;
mod parts;
mod summaries;
mod verified;
