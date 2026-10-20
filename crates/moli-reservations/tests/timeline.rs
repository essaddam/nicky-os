use jiff::Timestamp;
use moli_reservations::{Boundary, Reservation, Timeline};

fn ts(value: &str) -> Timestamp {
    value.parse().unwrap()
}

fn booking(id: &str, start: &str, end: &str) -> Reservation {
    Reservation {
        id: id.into(),
        start: ts(start),
        end: ts(end),
    }
}

fn stay() -> Reservation {
    booking(
        "fixture-stay",
        "2026-10-20T12:00:00Z",
        "2026-10-20T12:01:00Z",
    )
}

#[test]
fn restart_inside_or_after_a_stay_does_not_replay_an_arrival() {
    for now in ["2026-10-20T12:00:30Z", "2026-10-20T12:02:00Z"] {
        let mut timeline = Timeline::default();
        assert!(
            timeline
                .update(Some(&[stay()]), ts(now))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn known_future_boundaries_are_distinct_live_events_and_never_repeated() {
    let mut arrival = Timeline::default();
    arrival
        .update(Some(&[stay()]), ts("2026-10-20T11:59:59Z"))
        .unwrap();
    let events = arrival
        .update(Some(&[stay()]), ts("2026-10-20T12:00:00Z"))
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].boundary, Boundary::Arrival);
    assert_eq!(events[0].reservation, "fixture-stay");
    assert!(
        arrival
            .update(Some(&[stay()]), ts("2026-10-20T12:00:00Z"))
            .unwrap()
            .is_empty()
    );

    let mut departure = Timeline::default();
    departure
        .update(Some(&[stay()]), ts("2026-10-20T12:00:59Z"))
        .unwrap();
    let events = departure
        .update(Some(&[stay()]), ts("2026-10-20T12:01:00Z"))
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].boundary, Boundary::Departure);
    assert_eq!(events[0].at, stay().end);
}

#[test]
fn an_outage_covering_the_whole_window_does_not_emit_past_boundaries() {
    let mut timeline = Timeline::default();
    timeline
        .update(Some(&[stay()]), ts("2026-10-20T11:59:59Z"))
        .unwrap();
    assert!(
        timeline
            .update(Some(&[stay()]), ts("2026-10-20T12:02:00Z"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn reconnecting_to_an_active_stay_does_not_emit_a_late_arrival() {
    let mut timeline = Timeline::default();
    timeline
        .update(Some(&[stay()]), ts("2026-10-20T11:59:58Z"))
        .unwrap();
    timeline.update(None, ts("2026-10-20T11:59:59Z")).unwrap();
    assert!(
        timeline
            .update(Some(&[stay()]), ts("2026-10-20T12:00:00Z"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_backward_clock_does_not_replay_an_already_emitted_boundary() {
    let mut timeline = Timeline::default();
    timeline
        .update(Some(&[stay()]), ts("2026-10-20T11:59:59Z"))
        .unwrap();
    assert_eq!(
        timeline
            .update(Some(&[stay()]), ts("2026-10-20T12:00:00Z"))
            .unwrap()
            .len(),
        1
    );
    assert!(
        timeline
            .update(Some(&[stay()]), ts("2026-10-20T11:59:59Z"))
            .unwrap()
            .is_empty()
    );
    assert!(
        timeline
            .update(Some(&[stay()]), ts("2026-10-20T12:00:00Z"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_reschedule_into_the_past_is_a_snapshot_change_not_an_arrival() {
    let mut timeline = Timeline::default();
    timeline
        .update(Some(&[stay()]), ts("2026-10-20T11:59:59Z"))
        .unwrap();
    let changed = booking(
        "fixture-stay",
        "2026-10-20T11:59:50Z",
        "2026-10-20T12:01:00Z",
    );
    assert!(
        timeline
            .update(Some(&[changed]), ts("2026-10-20T12:00:00Z"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn departures_precede_arrivals_at_an_identical_boundary() {
    let stays = [
        booking("previous", "2026-10-20T11:00:00Z", "2026-10-20T12:00:00Z"),
        stay(),
    ];
    let mut timeline = Timeline::default();
    timeline
        .update(Some(&stays), ts("2026-10-20T11:59:59Z"))
        .unwrap();
    let events = timeline
        .update(Some(&stays), ts("2026-10-20T12:00:00Z"))
        .unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.boundary)
            .collect::<Vec<_>>(),
        [Boundary::Departure, Boundary::Arrival]
    );
}

#[test]
fn malformed_windows_do_not_become_a_live_baseline() {
    let mut timeline = Timeline::default();
    let invalid = booking(
        "fixture-stay",
        "2026-10-20T12:00:00Z",
        "2026-10-20T11:59:00Z",
    );
    assert!(
        timeline
            .update(Some(&[invalid]), ts("2026-10-20T11:59:59Z"))
            .is_err()
    );
    assert!(
        timeline
            .update(Some(&[stay()]), ts("2026-10-20T12:00:00Z"))
            .unwrap()
            .is_empty()
    );
}
