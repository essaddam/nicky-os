use moli_core::{MAX_WEEKLY_SLOTS, WeeklySchedule, WeeklySlot};

fn slot(day: u8, start: u16, end: u16) -> WeeklySlot {
    WeeklySlot { day, start, end }
}

#[test]
fn auto_h5_sunday_midnight_is_split_and_half_open() {
    let plan = WeeklySchedule {
        timezone: "UTC".into(),
        slots: vec![slot(7, 1380, 60)],
    }
    .normalize()
    .unwrap();
    assert_eq!(plan.slots, vec![slot(1, 0, 60), slot(7, 1380, 1440)]);
    assert!(plan.contains(7, 1439));
    assert!(plan.contains(1, 0));
    assert!(!plan.contains(1, 60));
    assert!(!plan.contains(7, 1379));
}

#[test]
fn auto_h5_overlaps_and_out_of_bounds_are_rejected() {
    for slots in [
        vec![slot(0, 1, 2)],
        vec![slot(8, 1, 2)],
        vec![slot(1, 1440, 1)],
        vec![slot(1, 1, 1441)],
        vec![slot(1, 1, 1)],
        vec![slot(1, 1, 10), slot(1, 9, 20)],
        vec![slot(7, 1380, 60), slot(1, 0, 120)],
    ] {
        assert!(
            WeeklySchedule {
                timezone: "UTC".into(),
                slots
            }
            .normalize()
            .is_err()
        );
    }
    assert!(
        WeeklySchedule {
            timezone: "UTC".into(),
            slots: vec![slot(1, 0, 60), slot(1, 60, 1440)]
        }
        .normalize()
        .is_ok()
    );
}

#[test]
fn auto_h5_slot_limit_applies_after_midnight_split() {
    let slots = (0..MAX_WEEKLY_SLOTS)
        .map(|i| {
            slot(
                1,
                u16::try_from(i * 2).unwrap(),
                u16::try_from(i * 2 + 1).unwrap(),
            )
        })
        .collect();
    let mut plan = WeeklySchedule {
        timezone: "UTC".into(),
        slots,
    };
    assert!(plan.clone().normalize().is_ok());
    plan.slots.push(slot(7, 1380, 60));
    assert!(plan.normalize().is_err());
    let slots = (0..MAX_WEEKLY_SLOTS - 1)
        .map(|i| {
            slot(
                2,
                u16::try_from(i * 2).unwrap(),
                u16::try_from(i * 2 + 1).unwrap(),
            )
        })
        .chain([slot(7, 1380, 60)])
        .collect();
    assert!(
        WeeklySchedule {
            timezone: "UTC".into(),
            slots
        }
        .normalize()
        .is_err()
    );
}

#[test]
fn auto_h5_schedule_state_is_neither_a_cached_value_nor_an_operation() {
    use moli_core::{Kind, Value};
    assert!(!Kind::Schedule.uses_state_cache());
    assert!(!Kind::Schedule.is_operation());
    assert_eq!(
        moli_core::validate(&Kind::Schedule, &Value::Bool(false)),
        Ok(Value::Bool(false))
    );
    assert!(moli_core::validate(&Kind::Schedule, &Value::Text("false".into())).is_err());
}
