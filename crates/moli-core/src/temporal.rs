//! Civil shapes only. Time-zone resolution and persistence belong to the runtime.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemporalFormat {
    Date,
    Time,
    Datetime,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalSpec {
    pub format: TemporalFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<Arc<str>>,
}
impl TemporalSpec {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        match self.format {
            TemporalFormat::Date | TemporalFormat::Time => self.timezone.is_none(),
            TemporalFormat::Datetime => self.timezone.as_ref().is_some_and(|tz| {
                !tz.is_empty()
                    && tz.len() <= 80
                    && tz.bytes().all(|c| {
                        c.is_ascii_alphanumeric() || matches!(c, b'/' | b'_' | b'-' | b'+')
                    })
            }),
        }
    }
    #[must_use]
    pub fn accepts_shape(&self, value: &str) -> bool {
        if !self.is_valid() || !value.is_ascii() {
            return false;
        }
        match self.format {
            TemporalFormat::Date => valid_date(value),
            TemporalFormat::Time => valid_time(value),
            TemporalFormat::Datetime => {
                if ![19, 20, 25].contains(&value.len())
                    || &value[10..11] != "T"
                    || !valid_date(&value[..10])
                    || !valid_time(&value[11..19])
                {
                    return false;
                }
                match &value[19..] {
                    "" | "Z" => true,
                    offset => {
                        offset.len() == 6
                            && matches!(&offset[..1], "+" | "-")
                            && &offset[3..4] == ":"
                            && number(&offset[1..3]).is_some_and(|n| n < 24)
                            && number(&offset[4..6]).is_some_and(|n| n < 60)
                            && offset != "-00:00"
                    }
                }
            }
        }
    }
}
fn number(value: &str) -> Option<u16> {
    value
        .bytes()
        .all(|c| c.is_ascii_digit())
        .then(|| value.parse().ok())
        .flatten()
}
fn valid_date(value: &str) -> bool {
    if value.len() != 10 || &value[4..5] != "-" || &value[7..8] != "-" {
        return false;
    }
    let (Some(year), Some(month), Some(day)) = (
        number(&value[..4]),
        number(&value[5..7]),
        number(&value[8..]),
    ) else {
        return false;
    };
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    year > 0 && day > 0 && day <= days
}
fn valid_time(value: &str) -> bool {
    value.len() == 8
        && &value[2..3] == ":"
        && &value[5..6] == ":"
        && number(&value[..2]).is_some_and(|n| n < 24)
        && number(&value[3..5]).is_some_and(|n| n < 60)
        && number(&value[6..]).is_some_and(|n| n < 60)
}
