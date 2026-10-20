//! Strict, bounded component and date parsing. No network or clock access.

use jiff::{
    Timestamp,
    civil::{Date, DateTime},
    tz::TimeZone,
};

use crate::{CalendarError, Reservation};

const MAX_BODY: usize = 1024 * 1024;
const MAX_LINE: usize = 4096;
const MAX_EVENTS: usize = 512;

#[derive(Clone, Debug)]
pub(super) struct DateValue {
    pub(super) timestamp: Timestamp,
    pub(super) date_only: bool,
    pub(super) civil: DateTime,
    pub(super) zone: TimeZone,
}

impl DateValue {
    pub(super) fn on_date(&self, date: Date) -> Result<Self, CalendarError> {
        let civil = DateTime::from_parts(date, self.civil.time());
        Ok(Self {
            timestamp: self
                .zone
                .to_ambiguous_timestamp(civil)
                .unambiguous()
                .map_err(|_| CalendarError)?,
            civil,
            ..self.clone()
        })
    }
}

impl PartialEq for DateValue {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp
            && self.date_only == other.date_only
            && self.civil == other.civil
            && self.zone.iana_name() == other.zone.iana_name()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Event {
    pub(super) uid: Option<String>,
    pub(super) start: Option<DateValue>,
    pub(super) end: Option<DateValue>,
    pub(super) cancelled: Option<bool>,
    pub(super) rule: Option<String>,
    pub(super) rdates: Vec<DateValue>,
    pub(super) exdates: Vec<DateValue>,
    pub(super) sequence: Option<u32>,
    pub(super) recurrence_id: Option<DateValue>,
}

impl Event {
    fn property(&mut self, head: &str, value: &str, zone: &TimeZone) -> Result<(), CalendarError> {
        let name = head.split(';').next().ok_or(CalendarError)?;
        match name.to_ascii_uppercase().as_str() {
            "UID" => {
                if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
                    return Err(CalendarError);
                }
                once(&mut self.uid, value.to_owned())?;
            }
            "DTSTART" => once(&mut self.start, datetime(head, value, zone)?)?,
            "DTEND" => once(&mut self.end, datetime(head, value, zone)?)?,
            "STATUS" => {
                let status = value.to_ascii_uppercase();
                if !["CANCELLED", "CONFIRMED", "TENTATIVE"].contains(&status.as_str()) {
                    return Err(CalendarError);
                }
                once(&mut self.cancelled, status == "CANCELLED")?;
            }
            "RRULE" => once(&mut self.rule, value.to_ascii_uppercase())?,
            "RDATE" | "EXDATE" => {
                let dates = if name.eq_ignore_ascii_case("RDATE") {
                    &mut self.rdates
                } else {
                    &mut self.exdates
                };
                for value in value.split(',') {
                    if dates.len() == MAX_EVENTS {
                        return Err(CalendarError);
                    }
                    dates.push(datetime(head, value, zone)?);
                }
            }
            // Never silently treat an unimplemented temporal rule as a single stay.
            "RECURRENCE-ID" => once(&mut self.recurrence_id, datetime(head, value, zone)?)?,
            "SEQUENCE" => {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(CalendarError);
                }
                let sequence = value.parse::<u32>().map_err(|_| CalendarError)?;
                if sequence > i32::MAX as u32 {
                    return Err(CalendarError);
                }
                once(&mut self.sequence, sequence)?;
            }
            "DURATION" => {
                return Err(CalendarError);
            }
            _ => {}
        }
        Ok(())
    }
}

fn component(value: &str) -> Result<&'static str, CalendarError> {
    [
        "VCALENDAR",
        "VEVENT",
        "VTIMEZONE",
        "VALARM",
        "STANDARD",
        "DAYLIGHT",
    ]
    .into_iter()
    .find(|name| value.eq_ignore_ascii_case(name))
    .ok_or(CalendarError)
}

fn once<T>(slot: &mut Option<T>, value: T) -> Result<(), CalendarError> {
    if slot.is_some() {
        return Err(CalendarError);
    }
    *slot = Some(value);
    Ok(())
}

fn unfold(input: &str) -> Result<Vec<String>, CalendarError> {
    if input.len() > MAX_BODY {
        return Err(CalendarError);
    }
    let mut lines: Vec<String> = Vec::new();
    for raw in input.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.chars().any(|c| c.is_control() && c != '\t') || line.len() > MAX_LINE {
            return Err(CalendarError);
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            let previous = lines.last_mut().ok_or(CalendarError)?;
            if previous.len() + line.len() - 1 > MAX_LINE {
                return Err(CalendarError);
            }
            previous.push_str(&line[1..]);
        } else if !line.is_empty() {
            lines.push(line.to_owned());
        }
    }
    Ok(lines)
}

pub(super) fn parse(
    input: &str,
    timezone: &str,
    now: Timestamp,
) -> Result<Vec<Reservation>, CalendarError> {
    if timezone.len() > 128 {
        return Err(CalendarError);
    }
    let zone = TimeZone::get(timezone).map_err(|_| CalendarError)?;
    let lines = unfold(input)?;
    let mut stack: Vec<&str> = Vec::new();
    let mut events = Vec::new();
    let mut current = Event::default();
    let mut ended = false;
    let mut version = None;
    let mut method = None;
    for line in &lines {
        let (head, value) = line.split_once(':').ok_or(CalendarError)?;
        if ended {
            return Err(CalendarError);
        }
        if head.eq_ignore_ascii_case("BEGIN") {
            let value = component(value)?;
            let allowed = match stack.as_slice() {
                [] => value == "VCALENDAR",
                ["VCALENDAR"] => matches!(value, "VEVENT" | "VTIMEZONE"),
                ["VCALENDAR", "VEVENT"] => value == "VALARM",
                ["VCALENDAR", "VTIMEZONE"] => matches!(value, "STANDARD" | "DAYLIGHT"),
                _ => false,
            };
            if !allowed {
                return Err(CalendarError);
            }
            stack.push(value);
        } else if head.eq_ignore_ascii_case("END") {
            let value = component(value)?;
            if stack.last().copied() != Some(value) {
                return Err(CalendarError);
            }
            if stack.as_slice() == ["VCALENDAR", "VEVENT"] {
                if events.len() == MAX_EVENTS {
                    return Err(CalendarError);
                }
                events.push(std::mem::take(&mut current));
            }
            stack.pop();
            ended = stack.is_empty();
        } else {
            match stack.as_slice() {
                ["VCALENDAR"] if head.eq_ignore_ascii_case("VERSION") => once(&mut version, value)?,
                ["VCALENDAR"] if head.eq_ignore_ascii_case("METHOD") => once(&mut method, value)?,
                ["VCALENDAR", "VEVENT"] => current.property(head, value, &zone)?,
                [] => return Err(CalendarError),
                _ => {}
            }
        }
    }
    if !ended || !stack.is_empty() || version != Some("2.0") {
        return Err(CalendarError);
    }
    if method.is_some_and(|value| value.eq_ignore_ascii_case("CANCEL")) {
        return Ok(Vec::new());
    }
    if method.is_some_and(|value| {
        !value.eq_ignore_ascii_case("PUBLISH") && !value.eq_ignore_ascii_case("REQUEST")
    }) {
        return Err(CalendarError);
    }
    let mut reservations = crate::revision::expand(events, now)?;
    reservations.sort_by_key(|reservation| (reservation.start, reservation.end));
    Ok(reservations)
}

pub(super) fn datetime(
    head: &str,
    value: &str,
    fallback: &TimeZone,
) -> Result<DateValue, CalendarError> {
    let mut timezone = None;
    let mut value_type = None;
    for parameter in head.split(';').skip(1) {
        let (key, value) = parameter.split_once('=').ok_or(CalendarError)?;
        match key.to_ascii_uppercase().as_str() {
            "TZID" => {
                let name = if let Some(quoted) = value.strip_prefix('"') {
                    quoted.strip_suffix('"').ok_or(CalendarError)?
                } else {
                    value
                };
                if name.contains('"') {
                    return Err(CalendarError);
                }
                once(&mut timezone, name)?;
            }
            "VALUE" => {
                let kind = if value.eq_ignore_ascii_case("DATE") {
                    "DATE"
                } else if value.eq_ignore_ascii_case("DATE-TIME") {
                    "DATE-TIME"
                } else {
                    return Err(CalendarError);
                };
                once(&mut value_type, kind)?;
            }
            _ => return Err(CalendarError),
        }
    }
    let date_only = value_type == Some("DATE");
    if value_type.is_some_and(|kind| !matches!(kind, "DATE" | "DATE-TIME")) {
        return Err(CalendarError);
    }
    let utc = value.ends_with('Z');
    let raw = value.strip_suffix('Z').unwrap_or(value);
    if !raw.is_ascii()
        || (date_only && (raw.len() != 8 || timezone.is_some() || utc))
        || (!date_only && (raw.len() != 15 || raw.as_bytes()[8] != b'T'))
        || (utc && timezone.is_some())
    {
        return Err(CalendarError);
    }
    if !raw.bytes().enumerate().all(|(index, byte)| {
        if index == 8 && !date_only {
            byte == b'T'
        } else {
            byte.is_ascii_digit()
        }
    }) {
        return Err(CalendarError);
    }
    let year = raw[0..4].parse().map_err(|_| CalendarError)?;
    if year < 1 {
        return Err(CalendarError);
    }
    let month = raw[4..6].parse().map_err(|_| CalendarError)?;
    let day = raw[6..8].parse().map_err(|_| CalendarError)?;
    let (hour, minute, second) = if date_only {
        (0, 0, 0)
    } else {
        (
            raw[9..11].parse().map_err(|_| CalendarError)?,
            raw[11..13].parse().map_err(|_| CalendarError)?,
            raw[13..15].parse().map_err(|_| CalendarError)?,
        )
    };
    let civil =
        DateTime::new(year, month, day, hour, minute, second, 0).map_err(|_| CalendarError)?;
    let zone = if utc {
        TimeZone::UTC
    } else if let Some(name) = timezone {
        if name.len() > 128 {
            return Err(CalendarError);
        }
        TimeZone::get(name).map_err(|_| CalendarError)?
    } else {
        fallback.clone()
    };
    Ok(DateValue {
        timestamp: zone
            .to_ambiguous_timestamp(civil)
            .unambiguous()
            .map_err(|_| CalendarError)?,
        date_only,
        civil,
        zone,
    })
}
