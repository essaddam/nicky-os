use std::collections::HashSet;

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Topology {
    pub enabled: bool,
    pub witness: bool,
    pub nodes: Vec<Node>,
    pub houses: Vec<House>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub site: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct House {
    pub id: String,
    pub primary: String,
    pub standbys: Vec<Standby>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Standby {
    pub node: String,
    pub automatic: bool,
}

impl Topology {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.nodes.is_empty() && self.nodes.len() <= 8,
            "expected 1..=8 nodes"
        );
        ensure!(
            !self.houses.is_empty() && self.houses.len() <= 2,
            "expected 1..=2 houses"
        );
        let mut nodes = HashSet::new();
        for node in &self.nodes {
            ensure!(valid_id(&node.id, 48), "invalid node ID");
            ensure!(nodes.insert(&node.id), "duplicate node ID");
            ensure!(
                matches!(node.site.as_str(), "house-a" | "house-b" | "cloud"),
                "invalid node site"
            );
        }
        let mut houses = HashSet::new();
        for house in &self.houses {
            ensure!(
                matches!(house.id.as_str(), "house-a" | "house-b"),
                "invalid house ID"
            );
            ensure!(houses.insert(&house.id), "duplicate house ID");
            ensure!(house.standbys.len() < 4, "at most four holders per house");
            let primary = self.enabled_node(&house.primary)?;
            ensure!(
                primary.site == house.id,
                "initial primary must be on its house site"
            );
            let mut holders = HashSet::from([&house.primary]);
            for standby in &house.standbys {
                ensure!(holders.insert(&standby.node), "duplicate house holder");
                let node = self.enabled_node(&standby.node)?;
                ensure!(
                    !standby.automatic || (self.witness && node.site == house.id),
                    "automatic standby requires a witness and the house site"
                );
            }
        }
        Ok(())
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn house(&self, id: &str) -> Option<&House> {
        self.houses.iter().find(|house| house.id == id)
    }

    pub fn holders(&self, house: &str) -> Vec<String> {
        let Some(house) = self.house(house) else {
            return Vec::new();
        };
        let mut holders = vec![house.primary.clone()];
        if self.enabled {
            holders.extend(house.standbys.iter().map(|standby| standby.node.clone()));
        }
        holders
    }

    pub fn local(&self, node: &str, house: &str) -> bool {
        self.house(house).is_some() && self.node(node).is_some_and(|node| node.site == house)
    }

    fn enabled_node(&self, id: &str) -> Result<&Node> {
        ensure!(valid_id(id, 48), "invalid holder ID");
        let Some(node) = self.node(id) else {
            bail!("unknown holder node");
        };
        ensure!(node.enabled, "holder node must be enabled");
        Ok(node)
    }
}

pub(crate) fn valid_id(id: &str, max_len: usize) -> bool {
    !id.is_empty()
        && id.len() <= max_len
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub fn preset(name: &str) -> Result<Topology> {
    ensure!(
        matches!(name, "none" | "cross" | "local" | "cloud" | "local-cloud"),
        "unknown topology preset"
    );
    let mut topology = Topology {
        enabled: name != "none",
        witness: name != "none",
        nodes: vec![node("node-a", "house-a"), node("node-b", "house-b")],
        houses: vec![
            House {
                id: "house-a".into(),
                primary: "node-a".into(),
                standbys: Vec::new(),
            },
            House {
                id: "house-b".into(),
                primary: "node-b".into(),
                standbys: Vec::new(),
            },
        ],
    };
    match name {
        "cross" => {
            topology.houses[0].standbys.push(standby("node-b", false));
            topology.houses[1].standbys.push(standby("node-a", false));
        }
        "local" | "cloud" | "local-cloud" => {
            for (index, suffix) in ["a", "b"].into_iter().enumerate() {
                if matches!(name, "local" | "local-cloud") {
                    let id = format!("backup-local-{suffix}");
                    topology.nodes.push(node(&id, &format!("house-{suffix}")));
                    topology.houses[index].standbys.push(standby(&id, true));
                }
                if matches!(name, "cloud" | "local-cloud") {
                    let id = format!("cloud-{suffix}");
                    topology.nodes.push(node(&id, "cloud"));
                    topology.houses[index].standbys.push(standby(&id, false));
                }
            }
        }
        _ => {}
    }
    topology.validate()?;
    Ok(topology)
}

fn node(id: &str, site: &str) -> Node {
    Node {
        id: id.into(),
        site: site.into(),
        enabled: true,
    }
}

fn standby(node: &str, automatic: bool) -> Standby {
    Standby {
        node: node.into(),
        automatic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_roles_are_per_house() {
        let topology = preset("cross").unwrap();
        assert_eq!(topology.holders("house-a"), ["node-a", "node-b"]);
        assert_eq!(topology.holders("house-b"), ["node-b", "node-a"]);
        assert!(topology.local("node-a", "house-a"));
        assert!(!topology.local("node-a", "house-b"));
        assert!(!topology.houses[0].standbys[0].automatic);
        assert!(!topology.houses[1].standbys[0].automatic);
    }

    #[test]
    fn no_ha_keeps_primaries_without_replication() {
        let mut topology = preset("none").unwrap();
        assert!(!topology.enabled);
        assert!(!topology.witness);
        topology.houses[0].standbys.push(standby("node-b", false));
        topology.validate().unwrap();
        assert_eq!(topology.holders("house-a"), ["node-a"]);
        assert_eq!(topology.holders("house-b"), ["node-b"]);
        assert!(topology.holders("unknown").is_empty());
    }

    #[test]
    fn every_preset_validates_with_bounded_holders() {
        for name in ["none", "cross", "local", "cloud", "local-cloud"] {
            let topology = preset(name).unwrap();
            topology.validate().unwrap();
            assert_eq!(topology.houses.len(), 2);
            assert!(topology.nodes.len() <= 8);
            assert!(
                topology
                    .houses
                    .iter()
                    .all(|house| topology.holders(&house.id).len() <= 4)
            );
        }
        assert_eq!(preset("local-cloud").unwrap().holders("house-a").len(), 3);
        assert!(preset("unknown").is_err());
    }

    #[test]
    fn duplicate_nodes_houses_and_assignments_are_rejected() {
        let topology = preset("cross").unwrap();
        let mut duplicate = topology.clone();
        duplicate.nodes.push(duplicate.nodes[0].clone());
        assert!(duplicate.validate().is_err());
        let mut duplicate = topology.clone();
        duplicate.houses[1] = duplicate.houses[0].clone();
        assert!(duplicate.validate().is_err());
        let mut duplicate = topology.clone();
        duplicate.houses[0].standbys.push(standby("node-b", false));
        assert!(duplicate.validate().is_err());
        let mut duplicate = topology;
        duplicate.houses[0].standbys.push(standby("node-a", false));
        assert!(duplicate.validate().is_err());
    }

    #[test]
    fn unknown_disabled_and_remote_initial_primaries_are_rejected() {
        let topology = preset("cross").unwrap();
        let mut invalid = topology.clone();
        invalid.houses[0].standbys[0].node = "missing".into();
        assert!(invalid.validate().is_err());
        let mut invalid = topology.clone();
        invalid.nodes[1].enabled = false;
        assert!(invalid.validate().is_err());
        let mut invalid = topology;
        invalid.houses[0].primary = "node-b".into();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn automatic_promotion_requires_locality_and_witness() {
        let mut topology = preset("local").unwrap();
        topology.witness = false;
        assert!(topology.validate().is_err());
        let mut topology = preset("cross").unwrap();
        topology.houses[0].standbys[0].automatic = true;
        assert!(topology.validate().is_err());
    }

    #[test]
    fn identities_sites_and_bounds_are_checked() {
        for id in ["", "../node", "node/name", "nœud", &"x".repeat(49)] {
            let mut topology = preset("none").unwrap();
            topology.nodes[0].id = id.into();
            topology.houses[0].primary = id.into();
            assert!(topology.validate().is_err());
        }
        let mut topology = preset("none").unwrap();
        topology.nodes[0].site = "elsewhere".into();
        assert!(topology.validate().is_err());
        let mut topology = preset("none").unwrap();
        topology.houses[0].id = "another-house".into();
        assert!(topology.validate().is_err());
        let mut topology = preset("local-cloud").unwrap();
        topology.houses[0]
            .standbys
            .extend([standby("node-b", false), standby("cloud-b", false)]);
        assert!(topology.validate().is_err());
    }

    #[test]
    fn unknown_json_fields_are_rejected_at_every_level() {
        let topology = preset("cross").unwrap();
        for path in ["topology", "node", "house", "standby"] {
            let mut value = serde_json::to_value(&topology).unwrap();
            let target = match path {
                "node" => &mut value["nodes"][0],
                "house" => &mut value["houses"][0],
                "standby" => &mut value["houses"][0]["standbys"][0],
                _ => &mut value,
            };
            target["unexpected"] = true.into();
            assert!(serde_json::from_value::<Topology>(value).is_err());
        }
    }
}
