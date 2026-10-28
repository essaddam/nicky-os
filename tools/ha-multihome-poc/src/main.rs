mod authority;
mod config;
mod engine;
mod link_fault;
#[path = "../../moteur-integre-banc/src/memory.rs"]
#[allow(dead_code)]
mod memory;
mod server;
mod site;
mod site_transport;
mod tcp_loss;
mod transport;
#[global_allocator]
#[cfg(not(feature = "plain-timing"))]
static ALLOCATOR: memory::Meter = memory::Meter;
#[global_allocator]
#[cfg(feature = "plain-timing")]
static ALLOCATOR: std::alloc::System = std::alloc::System;

use anyhow::{Result, ensure};
use engine::{Action, Bundle, Engine};
use serde_json::{Value as Json, json};
use tokio::io::AsyncReadExt;

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    // Replication/hash work in the caller cannot monopolize the lease timer or MQTT pumps.
    // A node stays on one runtime thread; only the coordinator has two worker threads.
    let mut runtime = if args == ["--worker"] {
        tokio::runtime::Builder::new_current_thread()
    } else {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.worker_threads(2);
        builder
    };
    runtime.enable_all().build()?.block_on(run(args))
}
async fn run(args: Vec<String>) -> Result<()> {
    if args == ["--worker"] {
        return transport::worker().await;
    }
    let mut bytes = Vec::new();
    tokio::io::stdin()
        .take(20 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(bytes.len() <= 20 * 1024 * 1024, "seed input too large");
    if args == ["--site-bench"] {
        let input = serde_json::from_slice(&bytes)?;
        drop(bytes);
        return site::run(input).await;
    }
    let bundle = if bytes.is_empty() {
        Bundle::fixture()
    } else {
        serde_json::from_slice(&bytes)?
    };
    if args == ["--tcp-loss"] {
        println!("TCP_LOSS_REPORT {}", tcp_loss::run(bundle).await?);
        return Ok(());
    }
    let mut engine = Engine::new(bundle)?;
    if args == ["--bench"] {
        let report = bench(&mut engine).await;
        engine.shutdown().await;
        println!("HA_REPORT {}", report?);
        return Ok(());
    }
    ensure!(
        args.len() == 3 && args[0] == "--serve" && args[1] == "--port",
        "use --bench or --serve --port <port>"
    );
    let port = args[2].parse()?;
    ensure!(
        (1024..65535).contains(&port) && ![1414, 1415, 1883, 1884].contains(&port),
        "reserved port"
    );
    server::serve(engine, port).await
}
fn action(op: &str, node: &str) -> Action {
    Action {
        op: op.into(),
        node: node.into(),
        house: String::new(),
    }
}
async fn bench(e: &mut Engine) -> Result<Json> {
    let mut cases = Vec::new();
    e.configure(config::preset("cross")?).await?;
    e.tick().await?;
    let initial = e.status();
    ensure!(
        initial["houses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|h| h["copies"].as_array().unwrap().len() == 2
                && h["copies"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|c| c["in_sync"] == true)),
        "cross replicas differ"
    );
    cases.push(json!({"case":"cross_initial","status":"PASS","snapshot":initial}));
    // Deliberately block the caller: lease renewal and MQTT pumps must remain independent.
    std::thread::sleep(std::time::Duration::from_millis(3500));
    e.refresh().await;
    let after_pause = e.status();
    ensure!(
        after_pause["houses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|h| h["lease_valid"] == true),
        "coordinator computation pause expired healthy leases"
    );
    cases.push(json!({"case":"coordinator_compute_pause","status":"PASS","pause_ms":3500,"snapshot":after_pause}));
    let before = e.status();
    e.action(action("isolate", "node-a")).await?;
    for _ in 0..24 {
        e.tick().await?;
    }
    let isolated = e.status();
    eprintln!(
        "HA_PARTITION_CHECK {}",
        json!({"error":isolated["error"],"houses":isolated["houses"],"nodes":isolated["nodes"]})
    );
    for (i, h) in isolated["houses"].as_array().unwrap().iter().enumerate() {
        ensure!(
            h["owner"] == before["houses"][i]["owner"]
                && h["lease_valid"] == true
                && h["sequence"].as_u64() > before["houses"][i]["sequence"].as_u64(),
            "WAN loss stopped a local house"
        );
        ensure!(
            h["copies"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["in_sync"] == false),
            "partition did not create lag"
        );
    }
    cases.push(
        json!({"case":"wan_partition_independent_houses","status":"PASS","snapshot":isolated}),
    );
    let denied = e.promote("house-b", "node-a", true).await.is_err();
    ensure!(denied, "partition incorrectly allowed stale promotion");
    e.action(action("heal", "node-a")).await?;
    e.tick().await?;
    let recovered = e.status();
    ensure!(
        recovered["houses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|h| h["copies"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["in_sync"] == true)),
        "WAN recovery differs"
    );
    cases.push(json!({"case":"wan_recovery_and_stale_candidate_refused","status":"PASS","snapshot":recovered}));
    e.action(action("stop", "node-a")).await?;
    let b = e.status()["houses"][1]["sequence"].as_u64();
    e.tick().await?;
    ensure!(
        e.status()["houses"][1]["sequence"].as_u64() > b,
        "house B stopped with node A"
    );
    e.promote("house-a", "node-b", true).await?;
    e.tick().await?;
    let failover = e.status();
    ensure!(
        failover["houses"][0]["owner"] == "node-b" && failover["houses"][0]["scope"] == "cloud",
        "remote promotion scope incorrect"
    );
    ensure!(
        e.test_effect("house-a", "node-a", 1, "old-token").is_err(),
        "old epoch accepted"
    );
    cases.push(json!({"case":"node_a_failure_cloud_only_takeover_and_fencing","status":"PASS","snapshot":failover}));
    e.action(action("restart", "node-a")).await?;
    e.promote("house-a", "node-a", true).await.err(); // A live former owner cannot preempt a still-valid lease.
    e.action(action("stop", "node-b")).await?;
    e.tick().await?;
    e.promote("house-a", "node-a", true).await?;
    e.promote("house-b", "node-a", true).await?;
    e.tick().await?;
    let reversed = e.status();
    ensure!(
        reversed["houses"][0]["scope"] == "local" && reversed["houses"][1]["scope"] == "cloud",
        "reverse ownership incorrect"
    );
    cases.push(
        json!({"case":"node_b_failure_house_a_continues","status":"PASS","snapshot":reversed}),
    );
    for preset in ["none", "local", "cloud", "local-cloud"] {
        e.configure(config::preset(preset)?).await?;
        e.tick().await?;
        let s = e.status();
        let expected = if preset == "none" {
            1
        } else if preset == "local-cloud" {
            3
        } else {
            2
        };
        ensure!(
            s["houses"]
                .as_array()
                .unwrap()
                .iter()
                .all(|h| h["copies"].as_array().unwrap().len() == expected
                    && h["copies"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|c| c["in_sync"] == true)),
            "preset copies differ"
        );
        if preset == "none" {
            let b = e.status()["houses"][1]["sequence"].as_u64();
            e.action(action("stop", "node-a")).await?;
            e.tick().await?;
            ensure!(
                e.status()["houses"][1]["sequence"].as_u64() > b,
                "no-HA node A failure stopped house B"
            );
            e.action(action("restart", "node-a")).await?;
            e.tick().await?;
            ensure!(
                e.status()["houses"][0]["lease_valid"] == true
                    && e.status()["houses"][0]["epoch"] == 2,
                "no-HA primary did not reincarnate"
            );
        }
        if preset == "local" {
            e.action(action("stop", "node-a")).await?;
            e.tick().await?;
            ensure!(
                e.status()["houses"][0]["owner"] == "backup-local-a",
                "automatic local standby did not take over"
            );
        }
        cases.push(json!({"case":preset,"status":"PASS","snapshot":e.status()}));
    }
    Ok(
        json!({"schema":1,"physical_commands":0,"cases":cases,"test_witness":"central in this process; not distributed consensus","transport":"real loopback MQTT and owned node processes","sources":e.status()["sources"],"process_metrics":"RSS/CPU per node; shared-page and sampler overhead caveats apply"}),
    )
}
