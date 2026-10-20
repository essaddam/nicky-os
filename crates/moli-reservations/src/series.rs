//! Sets of starts, exclusions and durations, independent of their presentation.

use std::collections::BTreeMap;

use jiff::{SignedDuration, Span, Timestamp, tz::TimeZone};

use crate::{
    CalendarError, Reservation,
    calendar::{DateValue, Event},
};

fn end_of(
    start: &DateValue,
    end: &DateValue,
    candidate: &DateValue,
) -> Result<Timestamp, CalendarError> {
    if start.date_only {
        let naive = |civil| {
            TimeZone::UTC
                .to_ambiguous_timestamp(civil)
                .unambiguous()
                .map_err(|_| CalendarError)
        };
        let seconds = naive(end.civil)?.as_second() - naive(start.civil)?.as_second();
        let civil = candidate
            .civil
            .checked_add(Span::new().seconds(seconds))
            .map_err(|_| CalendarError)?;
        end.zone
            .to_ambiguous_timestamp(civil)
            .unambiguous()
            .map_err(|_| CalendarError)
    } else {
        let seconds = end.timestamp.as_second() - start.timestamp.as_second();
        candidate
            .timestamp
            .checked_add(SignedDuration::from_secs(seconds))
            .map_err(|_| CalendarError)
    }
}

pub(super) fn expand(
    event: Event,
    now: Timestamp,
    budget: &mut usize,
) -> Result<Vec<Reservation>, CalendarError> {
    let uid = event.uid.ok_or(CalendarError)?;
    if event.cancelled == Some(true) {
        return Ok(Vec::new());
    }
    let start = event.start.ok_or(CalendarError)?;
    let end = event.end.ok_or(CalendarError)?;
    let seconds = end.timestamp.as_second() - start.timestamp.as_second();
    if start.date_only != end.date_only || seconds <= 0 || seconds > 366 * 86_400 {
        return Err(CalendarError);
    }
    if event
        .rdates
        .iter()
        .chain(&event.exdates)
        .any(|date| date.date_only != start.date_only)
    {
        return Err(CalendarError);
    }
    let recurring = event.rule.is_some() || !event.rdates.is_empty();
    let mut starts = BTreeMap::from([(start.timestamp, start.clone())]);
    if let Some(rule) = event.rule {
        for candidate in crate::recurrence::starts(&start, &rule, now, budget)? {
            starts.insert(candidate.timestamp, candidate);
        }
    }
    for candidate in event.rdates {
        starts.insert(candidate.timestamp, candidate);
    }
    for excluded in event.exdates {
        starts.remove(&excluded.timestamp);
    }
    let lookback = now
        .checked_sub(SignedDuration::from_secs(366 * 86_400))
        .map_err(|_| CalendarError)?;
    let horizon = now
        .checked_add(SignedDuration::from_secs(366 * 86_400))
        .map_err(|_| CalendarError)?;
    let mut result = Vec::new();
    for candidate in starts.into_values() {
        let end = end_of(&start, &end, &candidate)?;
        if end < lookback || candidate.timestamp > horizon {
            continue;
        }
        let key = if recurring {
            format!("{uid}\0{}", candidate.timestamp)
        } else {
            uid.clone()
        };
        result.push(Reservation {
            id: crate::public_id(&key),
            start: candidate.timestamp,
            end,
        });
    }
    Ok(result)
}
