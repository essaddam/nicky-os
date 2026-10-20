//! Select revisions before deriving windows; apply exceptions to their original ids.

use std::collections::{BTreeMap, btree_map::Entry};

use jiff::{SignedDuration, Timestamp};

use crate::{CalendarError, Reservation, calendar::Event};

fn latest(
    events: Vec<Event>,
) -> Result<BTreeMap<(String, Option<Timestamp>), Event>, CalendarError> {
    let mut selected: BTreeMap<(String, Option<Timestamp>), Event> = BTreeMap::new();
    for mut event in events {
        event.sequence = Some(event.sequence.unwrap_or(0));
        event.cancelled = Some(event.cancelled.unwrap_or(false));
        let uid = event.uid.clone().ok_or(CalendarError)?;
        let instance = event.recurrence_id.as_ref().map(|date| date.timestamp);
        match selected.entry((uid, instance)) {
            Entry::Vacant(slot) => {
                slot.insert(event);
            }
            Entry::Occupied(mut slot) => {
                let previous = slot.get();
                if event.sequence > previous.sequence {
                    slot.insert(event);
                } else if event.sequence == previous.sequence && event != *previous {
                    if event.cancelled == Some(true) {
                        slot.insert(event);
                    } else if previous.cancelled != Some(true) {
                        return Err(CalendarError);
                    }
                }
            }
        }
    }
    Ok(selected)
}

fn apply(
    event: Event,
    uid: &str,
    recurring: bool,
    reservations: &mut Vec<Reservation>,
    now: Timestamp,
    budget: &mut usize,
) -> Result<(), CalendarError> {
    let original = event.recurrence_id.as_ref().ok_or(CalendarError)?.timestamp;
    if event.rule.is_some() || !event.rdates.is_empty() || !event.exdates.is_empty() {
        return Err(CalendarError);
    }
    let key = if recurring {
        format!("{uid}\0{original}")
    } else {
        uid.to_owned()
    };
    let id = crate::public_id(&key);
    let Some(index) = reservations
        .iter()
        .position(|reservation| reservation.id == id)
    else {
        let low = now
            .checked_sub(SignedDuration::from_secs(366 * 86_400))
            .map_err(|_| CalendarError)?;
        let high = now
            .checked_add(SignedDuration::from_secs(366 * 86_400))
            .map_err(|_| CalendarError)?;
        if original < low || original > high {
            return Ok(());
        }
        return Err(CalendarError);
    };
    reservations.remove(index);
    if event.cancelled == Some(true) {
        return Ok(());
    }
    let changed = crate::series::expand(event, now, budget)?;
    if changed.len() > 1 {
        return Err(CalendarError);
    }
    for mut replacement in changed {
        replacement.id.clone_from(&id);
        reservations.push(replacement);
    }
    Ok(())
}

pub(super) fn expand(
    events: Vec<Event>,
    now: Timestamp,
) -> Result<Vec<Reservation>, CalendarError> {
    let selected = latest(events)?;
    let mut masters = BTreeMap::new();
    let mut exceptions: BTreeMap<String, Vec<Event>> = BTreeMap::new();
    for ((uid, instance), event) in selected {
        if instance.is_some() {
            exceptions.entry(uid).or_default().push(event);
        } else {
            masters.insert(uid, event);
        }
    }
    let mut result = Vec::new();
    let mut budget = 3660;
    for (uid, master) in masters {
        let changes = exceptions.remove(&uid).unwrap_or_default();
        if master.cancelled == Some(true) {
            continue;
        }
        let recurring = master.rule.is_some() || !master.rdates.is_empty();
        let date_only = master.start.as_ref().ok_or(CalendarError)?.date_only;
        let mut reservations = crate::series::expand(master, now, &mut budget)?;
        for change in changes {
            if change
                .recurrence_id
                .as_ref()
                .ok_or(CalendarError)?
                .date_only
                != date_only
            {
                return Err(CalendarError);
            }
            apply(change, &uid, recurring, &mut reservations, now, &mut budget)?;
        }
        result.extend(reservations);
        if result.len() > 4096 {
            return Err(CalendarError);
        }
    }
    if exceptions
        .into_values()
        .flatten()
        .any(|event| event.cancelled != Some(true))
    {
        return Err(CalendarError);
    }
    Ok(result)
}
