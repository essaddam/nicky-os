//! Deliberately small RRULE subset. Expansion has a shared work budget.

use std::collections::BTreeMap;

use jiff::{SignedDuration, Span, Timestamp, civil::Date};

use crate::{CalendarError, calendar::DateValue};

#[derive(Debug)]
struct Rule {
    weekly: bool,
    interval: i64,
    count: Option<usize>,
    until: Option<Timestamp>,
    weekdays: Vec<i64>,
}

fn positive(value: &str, maximum: usize) -> Result<usize, CalendarError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CalendarError);
    }
    value
        .parse::<usize>()
        .ok()
        .filter(|number| (1..=maximum).contains(number))
        .ok_or(CalendarError)
}

impl Rule {
    fn parse(text: &str, start: &DateValue) -> Result<Self, CalendarError> {
        let normalized = text.to_ascii_uppercase();
        let mut parts = BTreeMap::new();
        for part in normalized.split(';') {
            let (key, value) = part.split_once('=').ok_or(CalendarError)?;
            if !matches!(key, "FREQ" | "INTERVAL" | "COUNT" | "UNTIL" | "BYDAY")
                || parts.insert(key, value).is_some()
            {
                return Err(CalendarError);
            }
        }
        let weekly = match parts.get("FREQ").copied() {
            Some("DAILY") => false,
            Some("WEEKLY") => true,
            _ => return Err(CalendarError),
        };
        let interval = parts
            .get("INTERVAL")
            .map_or(Ok(1), |value| positive(value, 3660))?;
        let count = parts
            .get("COUNT")
            .map(|value| positive(value, 4096))
            .transpose()?;
        let until = parts
            .get("UNTIL")
            .map(|value| {
                let head = if start.date_only {
                    "UNTIL;VALUE=DATE"
                } else {
                    "UNTIL"
                };
                let end = crate::calendar::datetime(head, value, &start.zone)?;
                if end.date_only != start.date_only || end.timestamp < start.timestamp {
                    return Err(CalendarError);
                }
                Ok(end.timestamp)
            })
            .transpose()?;
        if count.is_some() && until.is_some() {
            return Err(CalendarError);
        }
        let first_weekday = i64::from(start.civil.date().weekday().to_monday_zero_offset());
        let weekdays = match parts.get("BYDAY") {
            None => vec![first_weekday],
            Some(_) if !weekly => return Err(CalendarError),
            Some(value) => {
                let mut days = Vec::new();
                for day in value.split(',') {
                    let offset = match day {
                        "MO" => 0,
                        "TU" => 1,
                        "WE" => 2,
                        "TH" => 3,
                        "FR" => 4,
                        "SA" => 5,
                        "SU" => 6,
                        _ => return Err(CalendarError),
                    };
                    if days.contains(&offset) {
                        return Err(CalendarError);
                    }
                    days.push(offset);
                }
                if !days.contains(&first_weekday) {
                    return Err(CalendarError);
                }
                days
            }
        };
        Ok(Self {
            weekly,
            interval: i64::try_from(interval).map_err(|_| CalendarError)?,
            count,
            until,
            weekdays,
        })
    }

    fn matches(&self, offset: i64, weekday: i64) -> bool {
        if self.weekly {
            let week = (offset + weekday) / 7;
            week % self.interval == 0 && self.weekdays.contains(&((offset + weekday) % 7))
        } else {
            offset % self.interval == 0
        }
    }
}

fn day_distance(start: Date, end: Date) -> Result<i64, CalendarError> {
    let utc = jiff::tz::TimeZone::UTC;
    let midnight = |date: Date| {
        utc.to_ambiguous_timestamp(date.at(0, 0, 0, 0))
            .unambiguous()
            .map(Timestamp::as_second)
            .map_err(|_| CalendarError)
    };
    Ok((midnight(end)? - midnight(start)?) / 86_400)
}

pub(super) fn starts(
    start: &DateValue,
    text: &str,
    now: Timestamp,
    budget: &mut usize,
) -> Result<Vec<DateValue>, CalendarError> {
    let rule = Rule::parse(text, start)?;
    let lookback = now
        .checked_sub(SignedDuration::from_secs(366 * 86_400))
        .map_err(|_| CalendarError)?;
    let horizon = now
        .checked_add(SignedDuration::from_secs(366 * 86_400))
        .map_err(|_| CalendarError)?;
    let beginning = start.civil.date();
    let last_date = start.zone.to_datetime(horizon).date();
    let mut offset = if rule.count.is_none() {
        day_distance(beginning, start.zone.to_datetime(lookback).date())?.max(0)
    } else {
        0
    };
    let weekday = i64::from(beginning.weekday().to_monday_zero_offset());
    let mut count = 0;
    let mut result = Vec::new();
    loop {
        let date = beginning
            .checked_add(Span::new().days(offset))
            .map_err(|_| CalendarError)?;
        if date > last_date {
            break;
        }
        *budget = budget.checked_sub(1).ok_or(CalendarError)?;
        if rule.matches(offset, weekday) {
            let candidate = start.on_date(date)?;
            if rule.until.is_some_and(|until| candidate.timestamp > until) {
                break;
            }
            count += 1;
            if candidate.timestamp >= lookback && candidate.timestamp <= horizon {
                result.push(candidate);
            }
            if rule.count.is_some_and(|maximum| count >= maximum) {
                break;
            }
        }
        offset += 1;
    }
    Ok(result)
}
