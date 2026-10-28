use super::*;
use layouts::*;
use std::mem::{offset_of, size_of};

#[test]
fn exact_layouts_and_padding() {
    let sizes = [
        size_of::<Row64>(),
        size_of::<Numeric32>(),
        size_of::<Row96>(),
        size_of::<Hot32>(),
        size_of::<Cold32>(),
        size_of::<Alias32>(),
        size_of::<GlobalSlot16>(),
        size_of::<Handle8>(),
        size_of::<Meta32>(),
        size_of::<Schema64>(),
        size_of::<Transform64>(),
        size_of::<Edge24>(),
        size_of::<Target12>(),
        size_of::<PoolEntry16>(),
        size_of::<Slice8>(),
        size_of::<Component32>(),
        size_of::<Calc48>(),
        size_of::<Route32>(),
    ];
    assert_eq!(
        sizes,
        [
            64, 32, 96, 32, 32, 32, 16, 8, 32, 64, 64, 24, 12, 16, 8, 32, 48, 32
        ]
    );
    assert_eq!(
        [
            offset_of!(Row64, id),
            offset_of!(Row64, parent),
            offset_of!(Row64, payload),
            offset_of!(Row64, previous_payload),
            offset_of!(Row64, received_ms),
            offset_of!(Row64, changed_ms),
            offset_of!(Row64, schema),
            offset_of!(Row64, aux),
            offset_of!(Row64, revision),
            offset_of!(Row64, flags),
            offset_of!(Row64, row_kind),
            offset_of!(Row64, value_kind)
        ],
        [0, 8, 16, 24, 32, 40, 48, 52, 56, 60, 62, 63],
    );
    assert_eq!(
        [
            offset_of!(Hot32, payload),
            offset_of!(Hot32, previous_payload),
            offset_of!(Hot32, received_ms),
            offset_of!(Hot32, changed_ms)
        ],
        [0, 8, 16, 24]
    );
    assert_eq!(offset_of!(Row96, numeric), 64);
    assert_eq!(offset_of!(Row64, revision), 56);
    assert_eq!(offset_of!(Row64, value_kind), 63);
    assert_eq!(offset_of!(Schema64, native_min), 24);
    assert_eq!(offset_of!(Schema64, reserved), 56);
    assert_eq!(offset_of!(Transform64, input_kind), 56);
    assert_eq!(offset_of!(Transform64, reserved), 60);
    assert_eq!(offset_of!(Calc48, history_ref), 36);
    assert_eq!(offset_of!(Calc48, reserved), 44);
    assert_eq!(offset_of!(Route32, access), 31);
    assert_eq!(offset_of!(Alias32, source_slot), 24);
}
#[test]
fn alternative_arenas_preserve_every_field_and_explicit_numeric_refs() {
    use crate::alternatives::{Annex64, HotCold};
    let mut bank = example();
    let position = bank.slots[bank.handle(100).unwrap().slot as usize].physical_index as usize;
    bank.rows[position].numeric = Numeric32 {
        min: 0,
        max: 100,
        step: 1,
        anchor: 0,
    };
    let numeric: Vec<_> = bank
        .rows
        .iter()
        .map(|row| matches!(row.base.id, 100 | 101))
        .collect();
    let annex = Annex64::new(&bank.rows, &numeric).unwrap();
    assert_eq!(annex.numeric_refs.len(), bank.rows.len());
    assert_eq!(annex.numeric.len(), 2);
    for (index, row) in bank.rows.iter().enumerate() {
        let (base, dimensions) = annex.get(index).unwrap();
        assert_eq!(base, row.base);
        assert_eq!(dimensions, numeric[index].then_some(row.numeric));
    }
    let split = HotCold::new(annex);
    for (index, row) in bank.rows.iter().enumerate() {
        assert_eq!(split.base(index).unwrap(), row.base);
    }
    assert_eq!(split.numeric.len(), 2);
    assert!(Annex64::new(&bank.rows, &[]).is_err());
}
#[test]
fn alias_shares_source_and_calculation_keeps_own_state() {
    let mut bank = example();
    bank.observe(100, INTEGER, 23, 2000, 2050).unwrap();
    assert_eq!(bank.read_at(200, 1).unwrap().base.payload, 23);
    assert_eq!(bank.read_at(201, 1).unwrap().base.payload, 23);
    assert_eq!(bank.read_at(202, 1).unwrap().base.payload, 42);
    assert_eq!(bank.aliases[1].source_id, 100);
    assert!(bank.observe(200, INTEGER, 99, 1, 1).is_err());
    assert!(bank.alias(200, 60, 200).is_err());
    assert!(bank.alias(900, 50, 100).is_err());
}
#[test]
fn aliases_issue_intentions_without_observation() {
    let bank = example();
    let before = *bank.read_at(200, 1).unwrap();
    assert_eq!(
        bank.intent(200, 99, false).unwrap(),
        Intent {
            owner_id: 50,
            point_id: 100,
            payload: 99,
            active_get: false
        }
    );
    assert_eq!(*bank.read_at(200, 1).unwrap(), before);
    assert!(bank.intent(202, 99, false).is_err());
}
#[test]
fn rename_compaction_and_vector_reallocation_preserve_identity() {
    let mut bank = example();
    let handle = bank.handle(100).unwrap();
    bank.rename(100, 999).unwrap();
    for id in 1000..2000 {
        bank.insert(id, VALUE, 50, 0).unwrap();
    }
    bank.compact();
    assert_eq!(bank.handle(100).unwrap(), handle);
    assert_eq!(bank.read_at(200, 1).unwrap().base.id, 100);
    assert_eq!(
        bank.metadata[bank.slots[handle.slot as usize].meta_index as usize].label,
        999
    );
}
#[test]
fn removal_reuse_stale_generation_and_epoch() {
    let mut bank = example();
    let old = bank.handle(100).unwrap();
    bank.remove(100).unwrap();
    let new = bank.insert(500, VALUE, 50, 0).unwrap();
    assert_eq!(new.slot, old.slot);
    assert_eq!(new.generation, old.generation + 1);
    assert!(bank.read_at(200, 1).is_err());
    assert!(bank.csr().unwrap().targets(old).is_err());
    bank.next_epoch().unwrap();
    assert!(bank.read_at(500, 1).is_err());
    assert!(bank.read_at(500, 2).is_ok());
}
#[test]
fn generation_and_epoch_never_wrap() {
    let mut bank = example();
    let handle = bank.handle(100).unwrap();
    bank.slots[handle.slot as usize].generation = u32::MAX;
    bank.index.get_mut(&100).unwrap().generation = u32::MAX;
    assert!(bank.remove(100).is_err());
    assert!(bank.index.contains_key(&100));
    bank.epoch = u64::MAX;
    assert!(bank.next_epoch().is_err());
}
#[test]
fn source_id_checked_even_when_a_cache_has_valid_generation() {
    let mut bank = example();
    bank.aliases[0].source_id = 999;
    assert!(bank.read_at(200, 1).is_err());
}
#[test]
fn overlapping_zones_use_edges_and_csr_both_directions() {
    let mut bank = example();
    let row_count = bank.rows.len();
    bank.insert(11, HOUSE, 1, 0).unwrap();
    bank.insert(31, ROOM, 11, 0).unwrap();
    assert!(bank.membership(40, 31).is_err());
    assert!(bank.membership(70, 30).is_err()); // Geographic resource != functional zone.
    assert!(bank.membership(40, 30).is_err());
    assert!(bank.reparent(30, 11).is_err());
    assert_eq!(bank.house(30).unwrap(), 10);
    assert_eq!(bank.rows.len(), row_count + 2);
    let csr = bank.csr().unwrap();
    let zone = bank.handle(40).unwrap();
    assert!(
        csr.targets(zone)
            .unwrap()
            .iter()
            .any(|target| target.to == bank.handle(30).unwrap().slot && target.relation == MEMBER)
    );
    let slot = bank.handle(30).unwrap().slot as usize;
    let incoming =
        &csr.reverse[csr.reverse_offsets[slot] as usize..csr.reverse_offsets[slot + 1] as usize];
    assert_eq!(incoming.len(), 3); // Parent plus two zones.
    assert_eq!(
        incoming
            .iter()
            .filter(|index| bank.edges[**index as usize].relation == MEMBER)
            .count(),
        2
    );
    assert_eq!(csr.forward.len(), bank.edges.len());
    assert_eq!(csr.reverse.len(), bank.edges.len());
}
#[test]
fn topology_rejects_dangling_cycles_and_wrong_owners() {
    let mut bank = example();
    assert!(bank.insert(300, ROOM, 999, 0).is_err());
    assert!(bank.insert(300, PHYSICAL, 20, 0).is_err());
    assert!(bank.insert(300, VALUE, 30, 0).is_err());
    assert!(bank.reparent(10, 30).is_err());
    bank.rows
        .iter_mut()
        .find(|row| row.base.id == 30)
        .unwrap()
        .base
        .parent = 30;
    assert!(bank.insert(300, PHYSICAL, 30, 0).is_err());
}
#[test]
fn timestamps_null_zero_and_active_get_are_independent() {
    let mut bank = example();
    bank.insert(300, VALUE, 50, ACTIVE_GET).unwrap();
    bank.observe(300, NULL, 0, 10, 12).unwrap();
    bank.observe(300, INTEGER, 0, 20, 25).unwrap();
    bank.observe(300, INTEGER, 0, 30, 40).unwrap();
    let row = &bank.read_at(300, 1).unwrap().base;
    assert_eq!((row.received_ms, row.changed_ms), (40, 20));
    assert_eq!((row.value_kind, row.payload), (INTEGER, 0));
    assert!(bank.intent(300, 1, false).is_err());
    assert!(bank.intent(300, 0, true).unwrap().active_get);
}
fn composite_bytes(version: u32, records: &[(u32, u64)]) -> Vec<u8> {
    let mut bytes = version.to_le_bytes().to_vec();
    bytes.extend_from_slice(&(records.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&[0, 0]);
    for (key, value) in records {
        bytes.extend_from_slice(&key.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}
#[test]
fn composite_validates_before_atomic_versioned_replace() {
    let mut bank = example();
    let first = composite_bytes(2, &[(7, 0x0102030405060708), (8, 9)]);
    assert_eq!(
        parse_composite(&first).unwrap(),
        (2, vec![(7, 0x0102030405060708), (8, 9)])
    );
    bank.composite(100, 1, &first, 2000, 2030).unwrap();
    let before = *bank.read_at(100, 1).unwrap();
    let pool_len = bank.pool.bytes.len();
    for malformed in [
        first[..first.len() - 1].to_vec(),
        composite_bytes(3, &[(1, 2), (1, 3)]),
        composite_bytes(9, &[(1, 2)]),
    ] {
        assert!(bank.composite(100, 2, &malformed, 3000, 3030).is_err());
        assert_eq!(*bank.read_at(100, 1).unwrap(), before);
        assert_eq!(bank.pool.bytes.len(), pool_len);
    }
    let reference = Handle8 {
        slot: before.base.payload as u32,
        generation: (before.base.payload >> 32) as u32,
    };
    assert_eq!(bank.pool.get(reference, COMPOSITE as u16).unwrap(), first);
    assert_eq!(bank.read_at(200, 1).unwrap().base.revision, 2);
    let mut corrupt = first.clone();
    corrupt[6] = 1;
    assert!(parse_composite(&corrupt).is_err());
}
#[test]
fn pool_checks_bounds_kind_and_generation() {
    let mut pool = Pool::default();
    assert!(pool.put(&vec![0; Pool::MAX_ENTRY + 1], 1).is_err());
    let reference = pool.put(&[1, 2, 3], 1).unwrap();
    assert!(
        pool.get(
            Handle8 {
                slot: reference.slot,
                generation: 2
            },
            1
        )
        .is_err()
    );
    assert!(pool.get(reference, 2).is_err());
    pool.entries[0].len = u32::MAX;
    assert!(pool.get(reference, 1).is_err());
    for _ in 0..255 {
        pool.put(&vec![0; Pool::MAX_ENTRY], 1).unwrap();
    }
    assert!(pool.put(&vec![0; Pool::MAX_ENTRY], 1).is_err());
    let mut empty_pool = Pool::default();
    for _ in 0..Pool::MAX_ENTRIES {
        empty_pool.put(&[], 1).unwrap();
    }
    assert!(empty_pool.put(&[], 1).is_err());
}

#[test]
fn first_null_observation_is_present_even_with_zero_timestamps() {
    let mut bank = example();
    bank.insert(300, VALUE, 50, WRITE | ACTIVE_GET).unwrap();
    bank.observe(300, NULL, 0, 0, 0).unwrap();
    let row = bank.read_at(300, bank.epoch).unwrap().base;
    assert_ne!(row.flags & HAS_OBSERVATION, 0);
    assert_eq!((row.received_ms, row.changed_ms, row.revision), (0, 0, 1));
    assert_eq!(bank.previous(300).unwrap(), None);
    bank.observe(300, NULL, 0, 19, 23).unwrap();
    assert_eq!(bank.previous(300).unwrap(), None);
    assert_eq!(bank.read_at(300, bank.epoch).unwrap().base.changed_ms, 0);
    assert_eq!(row.flags & (WRITE | ACTIVE_GET), WRITE | ACTIVE_GET);
}

#[test]
fn first_integer_zero_has_no_previous_and_sets_changed_at() {
    let mut bank = example();
    bank.insert(300, VALUE, 50, ACTIVE_GET).unwrap();
    assert_eq!(bank.previous(300).unwrap(), None);
    bank.observe(300, INTEGER, 0, 17, 23).unwrap();
    let row = bank.read_at(300, bank.epoch).unwrap().base;
    assert_eq!((row.payload, row.received_ms, row.changed_ms), (0, 23, 17));
    assert_ne!(row.flags & HAS_OBSERVATION, 0);
    assert_eq!(row.flags & HAS_PREVIOUS, 0);
    assert_eq!(bank.previous(300).unwrap(), None);
    bank.observe(300, INTEGER, 0, 29, 31).unwrap();
    assert_eq!(bank.read_at(300, bank.epoch).unwrap().base.changed_ms, 17);
    assert_eq!(bank.previous(300).unwrap(), None);
}

#[test]
fn previous_tracks_distinct_values_and_is_shared_by_aliases() {
    let mut bank = example(); // Initial value: 21.
    let position = bank.slots[bank.handle(100).unwrap().slot as usize].physical_index as usize;
    bank.rows[position].base.flags |= RESERVED_FLAGS;
    assert_eq!(bank.previous(200).unwrap(), None);
    bank.observe(100, INTEGER, 22, 2000, 2030).unwrap();
    assert_eq!(bank.previous(100).unwrap(), Some((INTEGER, 21)));
    let changed = bank.read_at(100, bank.epoch).unwrap().base;
    bank.observe(100, INTEGER, 22, 3000, 3030).unwrap();
    let mut expected = changed;
    expected.received_ms = 3030;
    expected.revision += 1;
    assert_eq!(bank.read_at(100, bank.epoch).unwrap().base, expected);
    bank.observe(100, INTEGER, 23, 4000, 4030).unwrap();
    for id in [100, 200, 201] {
        assert_eq!(bank.previous(id).unwrap(), Some((INTEGER, 22)));
        let row = bank.read_at(id, bank.epoch).unwrap().base;
        assert_eq!(
            (row.payload, row.received_ms, row.changed_ms, row.revision),
            (23, 4030, 4000, 4)
        );
        assert_eq!(
            row.flags & (WRITE | ACTIVE_GET | RESERVED_FLAGS),
            WRITE | ACTIVE_GET | RESERVED_FLAGS
        );
        assert_eq!(
            row.flags & PREVIOUS_KIND_MASK,
            u16::from(INTEGER) << PREVIOUS_KIND_SHIFT
        );
    }
    assert!(bank.intent(201, 99, false).is_ok());
    assert!(bank.intent(201, 0, true).is_ok());
}

#[test]
fn type_changes_keep_null_and_composite_previous_types() {
    let mut bank = example();
    bank.observe(100, NULL, 0, 0, 0).unwrap();
    assert_eq!(bank.previous(100).unwrap(), Some((INTEGER, 21)));
    let bytes = composite_bytes(3, &[(8, 9)]);
    bank.composite(100, 2, &bytes, 100, 110).unwrap();
    assert_eq!(bank.previous(100).unwrap(), Some((NULL, 0)));
    let composite_handle = bank.read_at(100, bank.epoch).unwrap().base.payload;
    assert_ne!(
        bank.read_at(100, bank.epoch).unwrap().base.flags & HAS_PREVIOUS,
        0
    );
    assert_eq!(
        bank.read_at(100, bank.epoch).unwrap().base.flags & PREVIOUS_KIND_MASK,
        0
    );
    bank.observe(100, INTEGER, 0, 120, 130).unwrap();
    assert_eq!(
        bank.previous(200).unwrap(),
        Some((COMPOSITE, composite_handle))
    );
    assert_eq!(
        bank.pool
            .get(
                Handle8 {
                    slot: composite_handle as u32,
                    generation: (composite_handle >> 32) as u32
                },
                COMPOSITE as u16
            )
            .unwrap(),
        bytes
    );
    bank.observe(100, NULL, 0, 140, 150).unwrap();
    assert_eq!(bank.previous(100).unwrap(), Some((INTEGER, 0)));
    bank.observe(100, NULL, 0, 160, 170).unwrap();
    assert_eq!(bank.previous(100).unwrap(), Some((INTEGER, 0)));
    assert_eq!(bank.read_at(100, bank.epoch).unwrap().base.changed_ms, 140);
}

#[test]
fn identical_composite_fields_ignore_version_and_wire_order() {
    let mut bank = example();
    let first = composite_bytes(2, &[(7, 8), (9, 10)]);
    bank.composite(100, 1, &first, 2000, 2030).unwrap();
    let before = bank.read_at(100, bank.epoch).unwrap().base;
    let pool_entries = bank.pool.entries.clone();
    let pool_bytes = bank.pool.bytes.clone();
    let reordered = composite_bytes(3, &[(9, 10), (7, 8)]);
    bank.composite(100, 2, &reordered, 3000, 3030).unwrap();
    let mut expected = before;
    expected.received_ms = 3030;
    expected.revision = 3;
    assert_eq!(bank.read_at(100, bank.epoch).unwrap().base, expected);
    assert_eq!(bank.pool.entries, pool_entries);
    assert_eq!(bank.pool.bytes, pool_bytes);
    assert_eq!(bank.previous(100).unwrap(), Some((INTEGER, 21)));
}

#[test]
fn changed_composite_keeps_old_handle_readable() {
    let mut bank = example();
    let first = composite_bytes(2, &[(7, 8)]);
    bank.composite(100, 1, &first, 2000, 2030).unwrap();
    let old = bank.read_at(100, bank.epoch).unwrap().base.payload;
    let second = composite_bytes(3, &[(7, 9)]);
    bank.composite(100, 2, &second, 3000, 3030).unwrap();
    assert_eq!(bank.previous(201).unwrap(), Some((COMPOSITE, old)));
    assert_eq!(
        bank.pool
            .get(
                Handle8 {
                    slot: old as u32,
                    generation: (old >> 32) as u32
                },
                COMPOSITE as u16
            )
            .unwrap(),
        first
    );
    let current = bank.read_at(100, bank.epoch).unwrap().base;
    assert_ne!(current.payload, old);
    assert_eq!(
        (current.changed_ms, current.received_ms, current.revision),
        (3000, 3030, 3)
    );
    assert_eq!(bank.pool.entries.len(), 2);
}

#[test]
fn observation_errors_preserve_rows_pool_and_permissions() {
    let mut bank = example();
    let before = bank.rows.clone();
    let entries = bank.pool.entries.clone();
    let bytes = bank.pool.bytes.clone();
    for kind in [3, 15, 16, 255] {
        assert!(bank.observe(100, kind, 99, 4000, 4030).is_err());
    }
    assert!(bank.observe(100, COMPOSITE, 0, 4000, 4030).is_err());
    assert!(bank.observe(200, INTEGER, 99, 4000, 4030).is_err());
    assert!(bank.observe(999, INTEGER, 99, 4000, 4030).is_err());
    let valid = composite_bytes(2, &[(7, 8)]);
    assert!(bank.composite(200, 1, &valid, 4000, 4030).is_err());
    assert!(bank.composite(999, 1, &valid, 4000, 4030).is_err());
    assert!(bank.composite(100, 0, &valid, 4000, 4030).is_err());
    assert_eq!(bank.rows, before);
    assert_eq!(bank.pool.entries, entries);
    assert_eq!(bank.pool.bytes, bytes);
    assert!(bank.intent(200, 99, false).is_ok());
    assert!(bank.intent(200, 0, true).is_ok());
}

#[test]
fn revision_and_pool_exhaustion_fail_before_publication() {
    let mut bank = example();
    let position = bank.slots[bank.handle(100).unwrap().slot as usize].physical_index as usize;
    bank.rows[position].base.revision = u32::MAX;
    let before = bank.rows.clone();
    assert!(bank.observe(100, INTEGER, 22, 4000, 4030).is_err());
    assert!(
        bank.composite(
            100,
            u32::MAX,
            &composite_bytes(u32::MAX, &[(7, 8)]),
            4000,
            4030
        )
        .is_err()
    );
    assert_eq!(bank.rows, before);
    assert!(bank.pool.entries.is_empty());
    assert!(bank.pool.bytes.is_empty());
    bank.rows[position].base.revision = 1;
    bank.pool.bytes.resize(Pool::MAX_TOTAL, 0);
    let before = bank.rows.clone();
    let entries = bank.pool.entries.clone();
    let bytes = bank.pool.bytes.clone();
    assert!(
        bank.composite(100, 1, &composite_bytes(2, &[(7, 8)]), 4000, 4030)
            .is_err()
    );
    assert_eq!(bank.rows, before);
    assert_eq!(bank.pool.entries, entries);
    assert_eq!(bank.pool.bytes, bytes);
}

#[test]
fn demo_exposes_previous_and_exactly_two_timestamps() {
    let output = demo();
    assert!(output.contains("\"previous_payload\":null,\"previous_kind\":null"));
    assert!(output.contains("\"received_ms\":1030,\"changed_ms\":1000"));
    assert!(!output.contains("observed_ms"));
}

#[test]
fn insert_rejects_forged_observation_flags_without_mutation() {
    let mut bank = example();
    let rows = bank.rows.clone();
    let slots = bank.slots.clone();
    let edges = bank.edges.clone();
    for flags in [
        HAS_OBSERVATION,
        HAS_PREVIOUS,
        PREVIOUS_KIND_MASK,
        WRITE | HAS_PREVIOUS,
    ] {
        assert!(bank.insert(300, VALUE, 50, flags).is_err());
        assert_eq!(bank.rows, rows);
        assert_eq!(bank.slots, slots);
        assert_eq!(bank.edges, edges);
        assert!(!bank.index.contains_key(&300));
    }
    bank.insert(300, VALUE, 50, WRITE | ACTIVE_GET | RESERVED_FLAGS)
        .unwrap();
    assert_eq!(bank.previous(300).unwrap(), None);
}

#[test]
fn direct_composite_observation_uses_the_same_distinct_value_contract() {
    let mut bank = example();
    let first = composite_bytes(2, &[(7, 8)]);
    bank.composite(100, 1, &first, 2000, 2030).unwrap();
    let before = bank.read_at(100, bank.epoch).unwrap().base;
    let equivalent = bank
        .pool
        .put(&composite_bytes(999, &[(7, 8)]), COMPOSITE as u16)
        .unwrap();
    let payload = u64::from(equivalent.slot) | (u64::from(equivalent.generation) << 32);
    bank.observe(100, COMPOSITE, payload, 3000, 3030).unwrap();
    let mut expected = before;
    expected.received_ms = 3030;
    expected.revision = 3;
    assert_eq!(bank.read_at(100, bank.epoch).unwrap().base, expected);
}
