//! Real loopback transport faults; the local control paths never traverse the replica relay.
use crate::{
    authority::{Authority, Effect, Scope},
    config,
    engine::Bundle,
    link_fault::Fault,
    transport::Cluster,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const HOMES: [(&str, &str, &str); 2] = [
    ("house-a", "node-a", "node-b"),
    ("house-b", "node-b", "node-a"),
];
fn now(origin: Instant) -> u64 {
    origin.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}
fn hash(value: &Json) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn copy<'a>(node: &'a Json, house: &str) -> Result<&'a Json> {
    node["houses"]
        .as_array()
        .and_then(|v| v.iter().find(|v| v["house"] == house))
        .context("missing house copy")
}
async fn reports(cluster: &mut Cluster) -> Result<Json> {
    Ok(json!({"node-a":cluster.status("node-a").await?,"node-b":cluster.status("node-b").await?}))
}
async fn wait_connected(cluster: &mut Cluster, previous: Option<&Json>) -> Result<Json> {
    let start = Instant::now();
    loop {
        let s = reports(cluster).await?;
        let fresh = previous.is_none_or(|old| {
            ["node-a", "node-b"].iter().all(|node| {
                s[*node]["replica_generation"].as_u64().unwrap_or(0)
                    > old[*node]["replica_generation"]
                        .as_u64()
                        .unwrap_or(u64::MAX)
            })
        });
        if fresh
            && s["node-a"]["replica_connected"] == true
            && s["node-b"]["replica_connected"] == true
        {
            return Ok(s);
        }
        ensure!(
            start.elapsed() < Duration::from_secs(8),
            "replica reconnection deadline"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
fn with_authority<T>(
    authority: &Arc<Mutex<Authority>>,
    origin: Instant,
    f: impl FnOnce(&mut Authority, u64) -> Result<T>,
) -> Result<T> {
    let mut a = authority
        .lock()
        .map_err(|_| anyhow::anyhow!("authority poisoned"))?;
    f(&mut a, now(origin))
}

pub async fn run(bundle: Bundle) -> Result<Json> {
    bundle.validate()?;
    let legacy = legacy_smoke(&bundle).await?;
    let mut cluster = Cluster::new_network_test().await?;
    let result = exercise(&mut cluster, bundle).await;
    cluster.shutdown().await;
    result.map(|mut report| {
        report["legacy_runtime_smoke"] = legacy;
        report
    })
}

async fn legacy_smoke(bundle: &Bundle) -> Result<Json> {
    let mut cluster = Cluster::new().await?;
    let result = async {
        cluster.spawn_node("legacy-node").await?;
        for seed in &bundle.houses {
            cluster
                .seed("legacy-node", &seed.id, 1, 0, &seed.document)
                .await?;
            cluster
                .set_role(
                    "legacy-node",
                    &seed.id,
                    1,
                    if seed.id == "house-a" {
                        "active"
                    } else {
                        "passive"
                    },
                )
                .await?;
        }
        let value = json!({"legacy":true});
        let ack = cluster
            .update("legacy-node", "house-a", 1, 1, "network_probe", &value)
            .await?;
        let mut expected = bundle.houses[0].document.clone();
        expected["network_probe"] = value;
        ensure!(
            ack["sha256"] == hash(&expected)?,
            "legacy default runtime changed"
        );
        Ok::<_, anyhow::Error>(
            json!({"status":"PASS","houses":2,"default_direct_transport_preserved":true}),
        )
    }
    .await;
    cluster.shutdown().await;
    result
}
async fn exercise(cluster: &mut Cluster, bundle: Bundle) -> Result<Json> {
    let mut documents: BTreeMap<_, _> = bundle
        .houses
        .iter()
        .map(|h| (h.id.clone(), h.document.clone()))
        .collect();
    cluster.spawn_site_node("node-a", "house-a").await?;
    cluster.spawn_site_node("node-b", "house-b").await?;
    wait_connected(cluster, None).await?;
    for (house, owner, standby) in HOMES {
        for node in [owner, standby] {
            cluster.seed(node, house, 1, 0, &documents[house]).await?;
            cluster
                .set_role(
                    node,
                    house,
                    1,
                    if node == owner { "active" } else { "passive" },
                )
                .await?;
        }
    }
    let initial = reports(cluster).await?;
    for (house, owner, standby) in HOMES {
        let expected = hash(&documents[house])?;
        ensure!(
            copy(&initial[owner], house)?["sha256"] == expected
                && copy(&initial[standby], house)?["sha256"] == expected,
            "initial copies differ"
        );
    }
    // A peer cannot secretly send a remote-house mutation over the unaffected local path.
    let bypass_refused = cluster
        .direct_update_for_test(
            "node-a",
            "house-b",
            1,
            1,
            "network_probe",
            &json!({"forbidden":true}),
        )
        .await
        .is_err();
    ensure!(
        bypass_refused,
        "foreign update bypassed the replication link"
    );
    let origin = Instant::now();
    let authority = Arc::new(Mutex::new(Authority::new(config::preset("cross")?, 0)?));
    let gate = authority.clone();
    let beats = cluster.heartbeats();
    let renewal = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let fresh = beats
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let Ok(mut a) = gate.lock() else { continue };
            let time = now(origin);
            for lease in a.leases() {
                if fresh
                    .get(&lease.owner)
                    .is_some_and(|t| t.elapsed() < Duration::from_millis(1500))
                {
                    let _ = a.renew(&lease.house, &lease.owner, lease.epoch, time);
                }
            }
        }
    });
    let result = phases(
        cluster,
        &mut documents,
        &authority,
        origin,
        initial,
        bypass_refused,
        bundle,
    )
    .await;
    renewal.abort();
    let _ = renewal.await;
    result
}
async fn phases(
    cluster: &mut Cluster,
    documents: &mut BTreeMap<String, Json>,
    authority: &Arc<Mutex<Authority>>,
    origin: Instant,
    initial: Json,
    bypass_refused: bool,
    bundle: Bundle,
) -> Result<Json> {
    let mut cases = Vec::new();
    let mut sequence = 0u64;
    let mut effects: BTreeMap<String, u64> = BTreeMap::new();
    for (name, fault) in [
        ("tcp_closed", Fault::Cut),
        ("tcp_blackhole", Fault::Blackhole),
    ] {
        let before = reports(cluster).await?;
        let link_before = cluster.link_stats();
        let fault_started = Instant::now();
        cluster.set_link_fault(fault)?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let settled = cluster.link_stats();
        let blocked_a = cluster
            .probe_update(
                "node-b",
                "house-a",
                1,
                sequence + 1,
                "network_probe",
                &json!({"must_not_arrive":true}),
            )
            .await
            .is_err();
        let blocked_b = cluster
            .probe_update(
                "node-a",
                "house-b",
                1,
                sequence + 1,
                "network_probe",
                &json!({"must_not_arrive":true}),
            )
            .await
            .is_err();
        ensure!(
            blocked_a && blocked_b,
            "replication did not stop in both directions"
        );
        let started = Instant::now();
        let old_sequence = sequence;
        while started.elapsed() < Duration::from_secs(5) {
            sequence += 1;
            for (house, owner, _) in HOMES {
                let value = json!({"phase":name,"step":sequence,"quality":"simulation"});
                let ack = cluster
                    .update(owner, house, 1, sequence, "network_probe", &value)
                    .await?;
                documents.get_mut(house).unwrap()["network_probe"] = value;
                ensure!(
                    ack["sequence"] == sequence && ack["sha256"] == hash(&documents[house])?,
                    "local primary failed to apply"
                );
                with_authority(authority, origin, |a, time| {
                    a.committed(house, owner, 1, time, sequence)?;
                    let accepted = a.accept_effect(
                        &Effect {
                            house: house.into(),
                            node: owner.into(),
                            epoch: 1,
                            id: format!("{name}-{sequence}"),
                            scope: Scope::Local,
                        },
                        time,
                    )?;
                    ensure!(accepted, "local workflow failed admission");
                    Ok(())
                })?;
                *effects.entry(house.into()).or_default() += 1;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let during = reports(cluster).await?;
        let link_during = cluster.link_stats();
        ensure!(
            link_during["forwarded_bytes"] == settled["forwarded_bytes"],
            "relay forwarded data while cut"
        );
        if fault == Fault::Cut {
            ensure!(
                link_during["active_connections"] == 0,
                "cut left TCP streams active"
            );
        } else {
            ensure!(
                link_during["active_connections"].as_u64().unwrap_or(0) >= 2,
                "blackhole did not retain TCP connections"
            );
        }
        let mut checks = Vec::new();
        for (house, owner, standby) in HOMES {
            let active = copy(&during[owner], house)?;
            let passive = copy(&during[standby], house)?;
            let previous = copy(&before[standby], house)?;
            ensure!(
                active["sequence"] == sequence && sequence > old_sequence,
                "local house did not progress"
            );
            ensure!(
                passive["sequence"] == previous["sequence"]
                    && passive["sha256"] == previous["sha256"],
                "remote copy changed during cut"
            );
            let (lease_ok, promotion_refused) = with_authority(authority, origin, |a, time| {
                let lease = a.lease(house).context("house lease")?;
                let ok = lease.owner == owner && lease.epoch == 1 && lease.expires_ms > time;
                let denied = a
                    .promote(
                        house,
                        standby,
                        time,
                        true,
                        false,
                        passive["sequence"].as_u64().context("passive sequence")?,
                    )
                    .is_err();
                Ok((ok, denied))
            })?;
            ensure!(
                lease_ok && promotion_refused,
                "link loss incorrectly changed authority"
            );
            checks.push(json!({"house":house,"owner":owner,"local_sequence":sequence,"passive_sequence":passive["sequence"],"local_lease_valid":lease_ok,"promotion_refused":promotion_refused,"local_workflow_effects_total":effects[house]}));
        }
        let progress_interval_ms = started.elapsed().as_millis();
        let fault_ms = fault_started.elapsed().as_millis();
        let restored = Instant::now();
        cluster.set_link_fault(Fault::Up)?;
        let recovered_connection = wait_connected(cluster, Some(&before)).await?;
        let reconnect_ms = restored.elapsed().as_millis();
        // The blackhole recovery closes the old sockets: timed-out messages must not reappear.
        for (house, _, standby) in HOMES {
            ensure!(
                copy(&recovered_connection[standby], house)?["sequence"]
                    == copy(&before[standby], house)?["sequence"],
                "queued timed-out update was replayed"
            );
        }
        let recovery = Instant::now();
        for (house, owner, standby) in HOMES {
            cluster
                .seed(standby, house, 1, sequence, &documents[house])
                .await?;
            cluster.set_role(standby, house, 1, "passive").await?;
            let current = cluster.status(owner).await?;
            ensure!(
                copy(&current, house)?["sequence"] == sequence,
                "healthy primary changed during recovery"
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let after = reports(cluster).await?;
        ensure!(
            ["node-a", "node-b"]
                .iter()
                .all(|node| after[*node]["applied_probe_updates"] == 0),
            "a timed-out probe was applied, even transiently"
        );
        for (house, owner, standby) in HOMES {
            let a = copy(&after[owner], house)?;
            let b = copy(&after[standby], house)?;
            ensure!(
                a["sha256"] == b["sha256"]
                    && a["sha256"] == hash(&documents[house])?
                    && b["sequence"] == sequence,
                "post-recovery copies differ"
            );
        }
        cases.push(json!({"case":name,"status":"PASS","fault_ms":fault_ms,"progress_interval_ms":progress_interval_ms,"local_steps_during_fault":sequence-old_sequence,"checks":checks,"link_before":link_before,"link_fault_settled":settled,"link_during":link_during,"link_after":cluster.link_stats(),"during":during,"after":after,"reconnect_ms":reconnect_ms,"snapshot_transfer_ms":recovery.elapsed().as_millis(),"recovery_ms":restored.elapsed().as_millis(),"timed_out_updates_replayed":false,"physical_commands":0}));
    }
    Ok(
        json!({"schema":1,"scope":"real TCP relay faults between local replica processes; local control and central test witness remain reachable","engine_isolation_flags_used":false,"physical_commands":0,"foreign_update_via_local_channel_refused":bypass_refused,"initial":initial,"cases":cases,"sources":bundle.houses.iter().map(|h|&h.summary).collect::<Vec<_>>(),"not_proven":["physical WAN or VPN outage between deployed HA hosts","loss of the central coordinator or witness","production device execution"]}),
    )
}
