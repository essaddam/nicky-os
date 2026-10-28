//! Synthetic proposal checks; no production imports, I/O, permissions or device commands.
use std::mem::size_of;

// Exact C field layouts from tools/alias-banc/main.rs, not a production storage ABI.
#[repr(C)]
struct Numeric32 {
    min: u64,
    max: u64,
    step: u64,
    anchor: u64,
}
#[repr(C)]
struct Row64 {
    id: u64,
    parent: u64,
    payload: u64,
    observed_ms: u64,
    received_ms: u64,
    changed_ms: u64,
    schema: u32,
    aux: u32,
    revision: u32,
    flags: u16,
    row_kind: u8,
    value_kind: u8,
}
#[repr(C)]
struct Row96 {
    base: Row64,
    numeric: Numeric32,
}
#[repr(C)]
struct Alias32 {
    alias_id: u64,
    owner_id: u64,
    source_id: u64,
    slot: u32,
    generation: u32,
}
#[repr(C)]
struct StableMembership {
    from: u64,
    to: u64,
    relation: u16,
    flags: u16,
    revision: u32,
}
#[repr(C)]
struct CachedEdge {
    from: u32,
    to: u32,
}
#[repr(C)]
struct ReferencePair {
    device: u32,
    position: u16,
}

fn csr_bytes(nodes: u64, arcs: u64, offset_bytes: u64) -> Option<u64> {
    nodes
        .checked_add(1)?
        .checked_mul(offset_bytes)?
        .checked_add(arcs.checked_mul(4)?)
}

fn main() {
    let nodes = 1_000_000_u64;
    let arcs = 4_000_000_u64;
    let offsets = u64::try_from(size_of::<usize>()).expect("usize size fits u64");
    let rows = nodes * u64::try_from(size_of::<Row96>()).expect("row size fits u64");
    let aliases = nodes * u64::try_from(size_of::<Alias32>()).expect("alias size fits u64");
    let memberships =
        arcs * u64::try_from(size_of::<StableMembership>()).expect("membership size fits u64");
    let edges = arcs * u64::try_from(size_of::<CachedEdge>()).expect("edge size fits u64");
    println!(
        "{{\"sizes_bytes\":{{\"row\":{},\"alias\":{},\"stable_membership\":{},\"cached_edge\":{},\"reference_pair\":{},\"raw_u32\":{},\"pointer\":{},\"usize\":{}}},\"csr_example\":{{\"nodes\":{nodes},\"arcs\":{arcs},\"usize_offsets_bytes\":{},\"u32_offsets_bytes\":{},\"u32_offsets_require_bounded_arc_count\":true}},\"raw_budget_example\":{{\"rows\":{nodes},\"aliases\":{nodes},\"memberships\":{arcs},\"cached_edges\":{arcs},\"row_bytes\":{rows},\"alias_bytes\":{aliases},\"membership_bytes\":{memberships},\"cached_edge_bytes\":{edges},\"total_bytes\":{},\"excludes_indexes_allocators_strings_runtime\":true}}}}",
        size_of::<Row96>(),
        size_of::<Alias32>(),
        size_of::<StableMembership>(),
        size_of::<CachedEdge>(),
        size_of::<ReferencePair>(),
        size_of::<u32>(),
        size_of::<*const Row96>(),
        size_of::<usize>(),
        csr_bytes(nodes, arcs, offsets).expect("small example fits u64"),
        csr_bytes(nodes, arcs, 4).expect("small example fits u64"),
        rows + aliases + memberships + edges,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Kind {
        Family,
        House,
        Floor,
        Room,
        Zone,
        GeographicZone,
        Device,
    }
    #[derive(Clone, Copy, Debug)]
    struct Node {
        id: u64,
        kind: Kind,
        parent: Option<u64>,
    }
    #[derive(Debug, PartialEq, Eq)]
    enum Invalid {
        Duplicate,
        Dangling,
        Cycle,
        ParentKind,
        MembershipKind,
        DifferentHouse,
    }
    struct Organization {
        nodes: BTreeMap<u64, Node>,
        memberships: Vec<(u64, u64)>,
    }
    impl Organization {
        fn validate(nodes: &[Node], memberships: &[(u64, u64)]) -> Result<Self, Invalid> {
            let mut model = Self {
                nodes: BTreeMap::new(),
                memberships: memberships.to_vec(),
            };
            for node in nodes {
                if model.nodes.insert(node.id, *node).is_some() {
                    return Err(Invalid::Duplicate);
                }
            }
            // Check parent references and cycles independently of allowed kind combinations.
            for node in nodes {
                let mut seen = BTreeSet::new();
                let mut next = Some(node.id);
                while let Some(id) = next {
                    if !seen.insert(id) {
                        return Err(Invalid::Cycle);
                    }
                    next = model.nodes.get(&id).ok_or(Invalid::Dangling)?.parent;
                }
            }
            for node in nodes {
                let parent = node.parent.map(|id| model.nodes[&id].kind);
                let allowed = matches!(
                    (node.kind, parent),
                    (Kind::Family, None)
                        | (Kind::House, Some(Kind::Family))
                        | (
                            Kind::Floor | Kind::Zone | Kind::GeographicZone,
                            Some(Kind::House)
                        )
                        | (Kind::Room, Some(Kind::Floor | Kind::House))
                        | (Kind::Device, Some(Kind::Room | Kind::House))
                );
                if !allowed {
                    return Err(Invalid::ParentKind);
                }
            }
            for &(zone, member) in memberships {
                let zone_node = model.nodes.get(&zone).ok_or(Invalid::Dangling)?;
                let member_node = model.nodes.get(&member).ok_or(Invalid::Dangling)?;
                if zone_node.kind != Kind::Zone
                    || !matches!(member_node.kind, Kind::Room | Kind::Device)
                {
                    return Err(Invalid::MembershipKind);
                }
                if model.house(zone) != model.house(member) {
                    return Err(Invalid::DifferentHouse);
                }
            }
            Ok(model)
        }
        fn house(&self, id: u64) -> Option<u64> {
            let mut node = self.nodes.get(&id)?;
            loop {
                if node.kind == Kind::House {
                    return Some(node.id);
                }
                node = self.nodes.get(&node.parent?)?;
            }
        }
    }
    fn node(id: u64, kind: Kind, parent: Option<u64>) -> Node {
        Node { id, kind, parent }
    }
    fn fixture() -> Vec<Node> {
        vec![
            node(1, Kind::Family, None),
            node(2, Kind::House, Some(1)),
            node(3, Kind::Floor, Some(2)),
            node(4, Kind::Room, Some(3)),
            node(5, Kind::Zone, Some(2)),
            node(6, Kind::Zone, Some(2)),
            node(7, Kind::Device, Some(4)),
            node(8, Kind::GeographicZone, Some(2)),
            node(9, Kind::House, Some(1)),
            node(10, Kind::Room, Some(9)),
            node(11, Kind::Room, Some(2)),
            node(12, Kind::Device, Some(2)),
        ]
    }
    fn rejected(nodes: &[Node], members: &[(u64, u64)], expected: Invalid) {
        assert_eq!(Organization::validate(nodes, members).err(), Some(expected));
    }
    #[test]
    fn measured_layouts_include_padding() {
        assert_eq!(
            (
                size_of::<Row96>(),
                size_of::<Alias32>(),
                size_of::<StableMembership>()
            ),
            (96, 32, 24)
        );
        assert_eq!(
            (
                size_of::<CachedEdge>(),
                size_of::<ReferencePair>(),
                size_of::<u32>()
            ),
            (8, 8, 4)
        );
        assert_eq!(std::mem::offset_of!(ReferencePair, position), 4);
    }
    #[test]
    fn csr_formula_counts_terminal_offset_and_checks_overflow() {
        assert_eq!(csr_bytes(1_000_000, 4_000_000, 8), Some(24_000_008));
        assert_eq!(csr_bytes(1_000_000, 4_000_000, 4), Some(20_000_004));
        assert_eq!(csr_bytes(u64::MAX, 1, 8), None);
    }
    #[test]
    fn hierarchy_keeps_floor_and_allows_rooms_without_floor() {
        let model = Organization::validate(&fixture(), &[]).unwrap();
        assert_eq!(model.nodes[&4].parent, Some(3));
        assert_eq!(model.house(4), Some(2));
        assert_eq!(model.house(11), Some(2));
        assert_eq!(model.house(12), Some(2));
        assert_eq!(model.house(1), None);
    }
    #[test]
    fn overlapping_zones_do_not_change_containment() {
        let model = Organization::validate(&fixture(), &[(5, 4), (6, 4), (5, 7), (6, 7)]).unwrap();
        assert_eq!(model.memberships.len(), 4);
        assert_eq!(model.nodes[&7].parent, Some(4));
        assert_eq!(model.house(7), Some(2));
    }
    #[test]
    fn duplicate_identity_is_rejected() {
        let mut nodes = fixture();
        nodes.push(nodes[0]);
        rejected(&nodes, &[], Invalid::Duplicate);
    }
    #[test]
    fn dangling_parent_and_membership_are_rejected() {
        let mut nodes = fixture();
        nodes[3].parent = Some(99);
        rejected(&nodes, &[], Invalid::Dangling);
        rejected(&fixture(), &[(5, 99)], Invalid::Dangling);
    }
    #[test]
    fn containment_cycle_is_rejected_without_repair() {
        let mut nodes = fixture();
        nodes[0].parent = Some(4);
        rejected(&nodes, &[], Invalid::Cycle);
    }
    #[test]
    fn wrong_parent_kind_is_rejected() {
        let mut nodes = fixture();
        nodes[3].parent = Some(5);
        rejected(&nodes, &[], Invalid::ParentKind);
    }
    #[test]
    fn zone_cannot_cross_houses() {
        rejected(&fixture(), &[(5, 10)], Invalid::DifferentHouse);
    }
    #[test]
    fn geographic_zone_does_not_implicitly_accept_logical_members() {
        rejected(&fixture(), &[(8, 4)], Invalid::MembershipKind);
        rejected(&fixture(), &[(5, 3)], Invalid::MembershipKind);
    }
    #[test]
    fn ambiguous_containment_is_rejected_instead_of_selecting_parent() {
        let mut nodes = fixture();
        nodes.push(node(7, Kind::Device, Some(11)));
        rejected(&nodes, &[], Invalid::Duplicate);
    }
    #[test]
    fn stable_alias_source_survives_device_placement_change() {
        let alias = Alias32 {
            alias_id: 100,
            owner_id: 101,
            source_id: 102,
            slot: 0,
            generation: 1,
        };
        let mut nodes = fixture();
        nodes[6].parent = Some(11);
        let model = Organization::validate(&nodes, &[(5, 7), (6, 7)]).unwrap();
        assert_eq!(model.house(7), Some(2));
        assert_eq!(alias.source_id, 102);
        assert!(!model.nodes.contains_key(&alias.source_id));
    }
    fn rises(states: &[bool]) -> usize {
        states.windows(2).filter(|pair| !pair[0] && pair[1]).count()
    }
    #[test]
    fn coalescing_state_loses_an_alert_occurrence() {
        let occurrences = [false, true, false];
        let coalesced = [occurrences[0], *occurrences.last().unwrap()];
        assert_eq!(rises(&occurrences), 1);
        assert_eq!(rises(&coalesced), 0);
        assert_eq!(occurrences.len(), 3); // Occurrences need their own retained ordering.
    }
    // Reference for small ordinary finite values only, not a production numeric algorithm.
    fn mean_at(inputs: &[(Option<f64>, u64)], now: u64) -> Option<f64> {
        let usable: Vec<f64> = inputs
            .iter()
            .filter(|(_, expires)| now < *expires)
            .filter_map(|(value, _)| value.filter(|number| number.is_finite()))
            .collect();
        if usable.is_empty() {
            None
        } else {
            Some(usable.iter().sum::<f64>() / usable.len() as f64)
        }
    }
    #[test]
    fn expiry_changes_mean_without_new_value() {
        let observations = [(Some(10.0), 10), (Some(30.0), 20)];
        assert_eq!(mean_at(&observations, 9), Some(20.0));
        assert_eq!(mean_at(&observations, 10), Some(30.0));
        assert_eq!(mean_at(&observations, 20), None);
    }
    #[test]
    fn null_nan_infinity_and_empty_mean_are_not_zero() {
        assert_eq!(
            mean_at(
                &[(None, 10), (Some(f64::NAN), 10), (Some(f64::INFINITY), 10)],
                0
            ),
            None
        );
        assert_eq!(mean_at(&[], 0), None);
        assert_eq!(mean_at(&[(Some(0.0), 10), (None, 10)], 0), Some(0.0));
    }
    #[test]
    fn naive_floating_accumulator_loses_exact_integer_total() {
        let integers = [10_000_000_000_000_000_i64, 1, -10_000_000_000_000_000];
        let exact: i128 = integers.iter().map(|value| i128::from(*value)).sum();
        let naive = integers
            .iter()
            .fold(0.0_f64, |total, value| total + *value as f64);
        assert_eq!(exact, 1);
        assert_eq!(naive, 0.0);
        assert_ne!(naive, exact as f64);
    }
    fn same_explicit_unit(units: &[Option<&str>]) -> bool {
        units
            .first()
            .is_some_and(|first| first.is_some() && units.iter().all(|unit| unit == first))
    }
    #[test]
    fn numeric_representation_does_not_prove_unit_compatibility() {
        // Conservative illustrative proposal rule; production also supports explicit ratios.
        assert!(!same_explicit_unit(&[Some("W"), Some("°C")]));
        assert!(!same_explicit_unit(&[Some("W"), None]));
        assert!(same_explicit_unit(&[Some("W"), Some("W")]));
    }
    #[test]
    fn same_value_refresh_extends_freshness_without_transition() {
        let old = (Some(10.0), 10);
        let refreshed = (old.0, 20);
        assert_eq!(usize::from(old.0 != refreshed.0), 0);
        assert_eq!(mean_at(&[old], 10), None);
        assert_eq!(mean_at(&[refreshed], 10), Some(10.0));
    }
}
