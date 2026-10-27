//! Small Rust web/MQTT gateway for the isolated POC, separate from the Hub.
#[path = "../gateway_runtime/mod.rs"]
mod gateway_runtime;
#[path = "../../../moteur-integre-banc/src/memory.rs"]
#[allow(dead_code)] // Shared meter also exposes benchmark-only counters.
mod memory;

#[global_allocator]
static ALLOCATOR: memory::Meter = memory::Meter;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    gateway_runtime::run().await
}
