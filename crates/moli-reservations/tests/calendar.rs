use jiff::Timestamp;
use moli_reservations::parse;
use std::fmt::Write as _;

fn ts(value: &str) -> Timestamp {
    value.parse().unwrap()
}

fn calendar(properties: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:fixture-booking-1@example.com\r\n{properties}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

fn events(records: &[&str]) -> String {
    let mut input = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n");
    for record in records {
        write!(
            input,
            "BEGIN:VEVENT\r\nUID:fixture-booking-1@example.com\r\n{record}\r\nEND:VEVENT\r\n"
        )
        .unwrap();
    }
    input.push_str("END:VCALENDAR\r\n");
    input
}

#[test]
fn utc_windows_include_the_start_and_exclude_the_end() {
    let input = calendar("DTSTART:20261020T120000Z\r\nDTEND:20261022T120000Z");
    let reservations = parse(&input, "UTC", ts("2026-10-19T00:00:00Z")).unwrap();
    assert_eq!(reservations.len(), 1);
    let reservation = &reservations[0];
    assert_eq!(reservation.start, ts("2026-10-20T12:00:00Z"));
    assert_eq!(reservation.end, ts("2026-10-22T12:00:00Z"));
    assert!(!reservation.allows(ts("2026-10-20T11:59:59.999Z")));
    assert!(reservation.allows(reservation.start));
    assert!(reservation.allows(ts("2026-10-22T11:59:59.999Z")));
    assert!(!reservation.allows(reservation.end));
}

#[test]
fn timezone_is_taken_from_the_property_before_the_configured_fallback() {
    let input = calendar(
        "DTSTART;TZID=Europe/Paris:20261020T160000\r\nDTEND;TZID=Europe/Paris:20261022T100000",
    );
    let reservations = parse(&input, "UTC", ts("2026-10-19T00:00:00Z")).unwrap();
    assert_eq!(reservations[0].start, ts("2026-10-20T14:00:00Z"));
    assert_eq!(reservations[0].end, ts("2026-10-22T08:00:00Z"));
}

#[test]
fn all_day_dates_follow_local_days_across_daylight_saving() {
    let input = calendar("DTSTART;VALUE=DATE:20260328\r\nDTEND;VALUE=DATE:20260330");
    let reservations = parse(&input, "Europe/Paris", ts("2026-03-27T00:00:00Z")).unwrap();
    assert_eq!(reservations[0].start, ts("2026-03-27T23:00:00Z"));
    assert_eq!(reservations[0].end, ts("2026-03-29T22:00:00Z"));
    assert_eq!(
        reservations[0].end.as_second() - reservations[0].start.as_second(),
        47 * 3600
    );
}

#[test]
fn ambiguous_and_nonexistent_local_access_bounds_are_refused() {
    for date in ["20260329", "20261025"] {
        let input = calendar(&format!(
            "DTSTART;TZID=Europe/Paris:{date}T023000\r\nDTEND;TZID=Europe/Paris:{date}T040000"
        ));
        assert!(parse(&input, "UTC", ts("2026-01-01T00:00:00Z")).is_err());
    }
}

#[test]
fn public_identity_is_stable_and_does_not_publish_the_supplier_uid() {
    let input = calendar("DTSTART:20261020T120000Z\r\nDTEND:20261020T130000Z");
    let reservations = parse(&input, "UTC", ts("2026-10-19T00:00:00Z")).unwrap();
    let id = &reservations[0].id;
    assert_eq!(id.len(), 32);
    assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(!id.contains("fixture-booking"));
    let renamed = input.replace("UID:", "SUMMARY:A different title\r\nUID:");
    assert_eq!(
        parse(&renamed, "UTC", ts("2026-10-19T00:00:00Z")).unwrap()[0].id,
        *id
    );
}

#[test]
fn dates_with_signs_and_mixed_date_types_are_refused() {
    for properties in [
        "DTSTART:2026+120T120000Z\r\nDTEND:20261022T120000Z",
        "DTSTART:+0261020T120000Z\r\nDTEND:20261022T120000Z",
        "DTSTART;VALUE=DATE:20261020\r\nDTEND:20261022T120000Z",
        "DTSTART:20261022T120000Z\r\nDTEND:20261020T120000Z",
    ] {
        assert!(parse(&calendar(properties), "UTC", ts("2026-10-19T00:00:00Z")).is_err());
    }
}

#[test]
fn daily_recurrences_count_excluded_instances_before_removing_exdate() {
    let input = calendar(
        "DTSTART:20261020T120000Z\r\nDTEND:20261020T130000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nEXDATE:20261021T120000Z",
    );
    let reservations = parse(&input, "UTC", ts("2026-10-19T00:00:00Z")).unwrap();
    assert_eq!(
        reservations.iter().map(|r| r.start).collect::<Vec<_>>(),
        [ts("2026-10-20T12:00:00Z"), ts("2026-10-22T12:00:00Z"),]
    );
}

#[test]
fn weekly_rules_apply_interval_byday_and_inclusive_until() {
    let input = calendar(
        "DTSTART:20261019T120000Z\r\nDTEND:20261019T130000Z\r\nRRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,FR;UNTIL=20261102T120000Z",
    );
    let reservations = parse(&input, "UTC", ts("2026-10-18T00:00:00Z")).unwrap();
    assert_eq!(
        reservations.iter().map(|r| r.start).collect::<Vec<_>>(),
        [
            ts("2026-10-19T12:00:00Z"),
            ts("2026-10-23T12:00:00Z"),
            ts("2026-11-02T12:00:00Z"),
        ]
    );
}

#[test]
fn rdate_and_exdate_form_a_set_including_the_initial_instance() {
    let input = calendar(
        "DTSTART:20261020T120000Z\r\nDTEND:20261020T130000Z\r\nRRULE:FREQ=DAILY;COUNT=2\r\nRDATE:20261021T120000Z,20261023T120000Z\r\nEXDATE:20261020T120000Z",
    );
    let reservations = parse(&input, "UTC", ts("2026-10-19T00:00:00Z")).unwrap();
    assert_eq!(
        reservations.iter().map(|r| r.start).collect::<Vec<_>>(),
        [ts("2026-10-21T12:00:00Z"), ts("2026-10-23T12:00:00Z"),]
    );
}

#[test]
fn unsupported_rules_and_hostile_sizes_fail_the_entire_calendar() {
    for rule in [
        "FREQ=SECONDLY",
        "FREQ=DAILY;BYSETPOS=1",
        "FREQ=DAILY;COUNT=0",
        "FREQ=DAILY;INTERVAL=0",
    ] {
        let input = calendar(&format!(
            "DTSTART:20261020T120000Z\r\nDTEND:20261020T130000Z\r\nRRULE:{rule}"
        ));
        assert!(parse(&input, "UTC", ts("2026-10-19T00:00:00Z")).is_err());
    }
    let input = calendar(&format!(
        "SUMMARY:{}\r\nDTSTART:20261020T120000Z\r\nDTEND:20261020T130000Z",
        "x".repeat(4097)
    ));
    assert!(parse(&input, "UTC", ts("2026-10-19T00:00:00Z")).is_err());
}

#[test]
fn the_latest_sequence_cancels_a_stay_regardless_of_record_order() {
    let active = "SEQUENCE:1\r\nDTSTART:20261020T120000Z\r\nDTEND:20261022T120000Z";
    let cancelled = "SEQUENCE:2\r\nSTATUS:CANCELLED";
    for records in [[active, cancelled], [cancelled, active]] {
        let parsed = parse(&events(&records), "UTC", ts("2026-10-19T00:00:00Z")).unwrap();
        assert!(parsed.is_empty());
    }
}

#[test]
fn duplicate_identical_stays_are_one_but_conflicting_equal_versions_are_refused() {
    let first = "DTSTART:20261020T120000Z\r\nDTEND:20261022T120000Z";
    let other = "DTSTART:20261021T120000Z\r\nDTEND:20261023T120000Z";
    let parsed = parse(&events(&[first, first]), "UTC", ts("2026-10-19T00:00:00Z")).unwrap();
    assert_eq!(parsed.len(), 1);
    assert!(parse(&events(&[first, other]), "UTC", ts("2026-10-19T00:00:00Z")).is_err());
}

#[test]
fn a_cancelled_exception_removes_only_its_original_instance() {
    let input = events(&[
        "DTSTART:20261020T120000Z\r\nDTEND:20261020T130000Z\r\nRRULE:FREQ=DAILY;COUNT=3",
        "RECURRENCE-ID:20261021T120000Z\r\nSTATUS:CANCELLED",
    ]);
    let parsed = parse(&input, "UTC", ts("2026-10-19T00:00:00Z")).unwrap();
    assert_eq!(
        parsed.iter().map(|r| r.start).collect::<Vec<_>>(),
        [ts("2026-10-20T12:00:00Z"), ts("2026-10-22T12:00:00Z"),]
    );
}

#[test]
fn a_moved_exception_keeps_the_identity_of_its_original_instance() {
    let base = "DTSTART:20261020T120000Z\r\nDTEND:20261020T130000Z\r\nRRULE:FREQ=DAILY;COUNT=3";
    let original = parse(&events(&[base]), "UTC", ts("2026-10-19T00:00:00Z")).unwrap();
    let moved = parse(
        &events(&[
            base,
            "RECURRENCE-ID:20261021T120000Z\r\nDTSTART:20261021T150000Z\r\nDTEND:20261021T160000Z",
        ]),
        "UTC",
        ts("2026-10-19T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(moved.len(), 3);
    assert_eq!(moved[1].start, ts("2026-10-21T15:00:00Z"));
    assert_eq!(moved[1].id, original[1].id);
}

#[test]
fn all_day_recurrences_keep_calendar_days_instead_of_the_first_utc_duration() {
    let input = calendar(
        "DTSTART;VALUE=DATE:20260328\r\nDTEND;VALUE=DATE:20260330\r\nRRULE:FREQ=DAILY;COUNT=3",
    );
    let parsed = parse(&input, "Europe/Paris", ts("2026-03-27T00:00:00Z")).unwrap();
    assert_eq!(
        parsed.iter().map(|r| r.end).collect::<Vec<_>>(),
        [
            ts("2026-03-29T22:00:00Z"),
            ts("2026-03-30T22:00:00Z"),
            ts("2026-03-31T22:00:00Z"),
        ]
    );
}

#[test]
fn component_and_parameter_names_are_case_insensitive() {
    let input = calendar("dtstart;tzid=Europe/Paris:20261020T160000\r\ndtend;tzid=Europe/Paris:20261020T170000\r\nrdate;tzid=Europe/Paris:20261021T160000")
        .replace("BEGIN:VCALENDAR", "begin:vcalendar")
        .replace("END:VCALENDAR", "end:vcalendar");
    let parsed = parse(&input, "UTC", ts("2026-10-19T00:00:00Z")).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[1].start, ts("2026-10-21T14:00:00Z"));
}

#[test]
fn malformed_timezone_quotes_are_not_repaired() {
    for name in ["\"\"Europe/Paris\"\"", "\"Europe/Paris", "Europe/Paris\""] {
        let input = calendar(&format!(
            "DTSTART;TZID={name}:20261020T160000\r\nDTEND:20261020T170000Z"
        ));
        assert!(parse(&input, "UTC", ts("2026-10-19T00:00:00Z")).is_err());
    }
}
