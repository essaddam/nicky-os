//! Isolated V6 structural prototype. Run --layouts or --demo; rustc --test for proof.
mod alternatives;
mod layouts;
mod model;
#[cfg(test)]
mod tests;
use model::*;

fn example() -> Bank {
    let mut bank = Bank::new();
    for (id, kind, parent) in [
        (1, FAMILY, 0),
        (10, HOUSE, 1),
        (20, FLOOR, 10),
        (30, ROOM, 20),
        (40, ZONE, 10),
        (41, ZONE, 10),
        (50, PHYSICAL, 30),
        (60, VIRTUAL, 30),
        (100, VALUE, 50),
        (101, CALC, 60),
        (70, GEOZONE, 10),
    ] {
        bank.insert(
            id,
            kind,
            parent,
            if id == 100 { WRITE | ACTIVE_GET } else { 0 },
        )
        .expect("synthetic topology");
    }
    bank.alias(200, 60, 100).expect("alias");
    bank.alias(201, 60, 200).expect("flattened alias");
    bank.alias(202, 60, 101).expect("calculation alias");
    bank.membership(40, 30).expect("zone one");
    bank.membership(41, 30).expect("zone two");
    bank.observe(100, INTEGER, 21, 1000, 1030)
        .expect("source observation");
    bank.observe(101, INTEGER, 42, 1000, 1031)
        .expect("own calculation state");
    bank.rename(100, 7).expect("rename independent of identity");
    bank.compact();
    bank
}
fn demo() -> String {
    let bank = example();
    let csr = bank.csr().expect("CSR");
    let mut ids: Vec<_> = bank.index.keys().copied().collect();
    ids.sort_unstable();
    let nodes:Vec<_>=ids.iter().map(|id| {
        let handle=bank.handle(*id).expect("index");
        let slot=bank.slots[handle.slot as usize];
        let row=bank.read_at(*id,bank.epoch).expect("resolve");
        let (previous_kind, previous_payload) = bank.previous(*id).expect("previous").map_or_else(
            || ("null".to_string(), "null".to_string()),
            |(kind, payload)| (kind.to_string(), payload.to_string()),
        );
        format!("{{\"id\":{},\"slot\":{},\"generation\":{},\"arena\":{},\"physical_index\":{},\"resolved_id\":{},\"payload\":{},\"value_kind\":{},\"previous_payload\":{},\"previous_kind\":{},\"received_ms\":{},\"changed_ms\":{}}}",
            id,handle.slot,handle.generation,slot.arena,slot.physical_index,row.base.id,row.base.payload,row.base.value_kind,previous_payload,previous_kind,row.base.received_ms,row.base.changed_ms)
    }).collect();
    let edges: Vec<_> = bank
        .edges
        .iter()
        .map(|edge| {
            format!(
                "{{\"from\":{},\"to\":{},\"relation\":{}}}",
                edge.from, edge.to, edge.relation
            )
        })
        .collect();
    format!(
        "{{\"scope\":\"synthetic single-writer structural prototype, no runtime benchmark\",\"epoch\":{},\"nodes\":[{}],\"edges\":[{}],\"memory\":{}}}",
        bank.epoch,
        nodes.join(","),
        edges.join(","),
        bank.memory(&csr)
    )
}
fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--layouts") => println!("{}", layouts::json()),
        Some("--demo") => println!("{}", demo()),
        _ => {
            eprintln!("usage: structure-v6 --layouts | --demo");
            std::process::exit(2);
        }
    }
}
