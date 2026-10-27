//! Business adapters publish typed observations; this POC has no physical executor.
use anyhow::Result;
use moli_core::{Device, PointId, Value};
use serde_json::Value as Json;

pub struct Observation {
    pub point: PointId,
    pub value: Value,
}

pub trait Adapter: Send {
    /// Bounded cold extension data, e.g. a paged library description, never PCM.
    fn cold_metadata(&self) -> Json {
        Json::Null
    }
    /// Small diagnostic snapshot, separate from the cold catalogue and PCM.
    fn runtime_metrics(&self) -> Json {
        Json::Null
    }
    /// Moves the cold schema out once, avoiding a full clone on each update.
    fn take_description(&mut self) -> Vec<Device>;
    fn observe(&mut self, step: u64) -> Result<Vec<Observation>>;
    /// Returns a native request body without transmitting it or confirming state.
    fn simulate_command(&self, point: &PointId, value: &Value) -> Result<Json>;
    /// Optional display label for a native enum selection.
    fn enum_label(&self, _point: &PointId, native: &str) -> String {
        native.to_owned()
    }
}
