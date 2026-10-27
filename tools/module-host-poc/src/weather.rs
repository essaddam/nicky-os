//! Deterministic weather fixture and approximate local solar ephemeris, read-only.
//! No network, household coordinates, wall clock, or physical command executor.
use crate::adapter::{Adapter, Observation};
use anyhow::{Result, bail, ensure};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use moli_core::{
    Access, Device, DeviceId, InstanceId, Kind, PointId, PointSpec, Semantic, Unit, Value,
};
use serde_json::Value as Json;

#[path = "../../../crates/moli-automation/src/sun.rs"]
mod existing_sun;

const LATITUDE: f64 = 45.0;
const LONGITUDE: f64 = 0.0;
const CLOCK: &str = "2026-03-20T12:00:00Z";
const MAX_STEP: u64 = 366 * 24 * 60;
const EVENT_KEYS: [&str; 9] = [
    "astronomical_dawn",
    "nautical_dawn",
    "civil_dawn",
    "sunrise",
    "solar_noon",
    "sunset",
    "civil_dusk",
    "nautical_dusk",
    "astronomical_dusk",
];
const WMO: [&str; 29] = [
    "0", "1", "2", "3", "45", "48", "51", "53", "55", "56", "57", "61", "63", "65", "66", "67",
    "71", "73", "75", "77", "80", "81", "82", "85", "86", "95", "96", "97", "99",
];

pub fn build(instance: &InstanceId) -> Result<Box<dyn Adapter>> {
    let weather_id = DeviceId::new(instance, "synthetic-weather");
    let sun_id = DeviceId::new(instance, "synthetic-sun");
    let description = vec![
        device(
            instance,
            weather_id.clone(),
            "Fixture weather",
            weather_specs(),
        ),
        device(
            instance,
            sun_id.clone(),
            "Fixture solar ephemeris",
            sun_specs(),
        ),
    ];
    Ok(Box::new(Weather {
        description,
        weather_id,
        sun_id,
        epoch: CLOCK.parse()?,
    }))
}

struct Weather {
    description: Vec<Device>,
    weather_id: DeviceId,
    sun_id: DeviceId,
    epoch: Timestamp,
}

impl Adapter for Weather {
    fn take_description(&mut self) -> Vec<Device> {
        std::mem::take(&mut self.description)
    }

    fn observe(&mut self, step: u64) -> Result<Vec<Observation>> {
        ensure!(step <= MAX_STEP, "fixture clock exceeds one synthetic year");
        let at = self
            .epoch
            .checked_add(SignedDuration::from_secs(i64::try_from(step)? * 60))?;
        let local = at.to_zoned(TimeZone::UTC);
        let events = day_events(local.date(), LATITUDE, LONGITUDE)?;
        let future = next_events(at, LATITUDE, LONGITUDE)?;
        let (elevation, azimuth) = position(at, LATITUDE, LONGITUDE);
        let daylight = events.events[3]
            .zip(events.events[5])
            .map_or(elevation > -0.833, |(rise, set)| {
                at.as_second() >= rise && at.as_second() < set
            });
        let phase = f64::from(u16::try_from(step % 120)?);
        let wet = step % 60 >= 40;
        let weather = [
            ("observed_at", Value::Int(at.as_second())),
            ("temperature", Value::Float(16.0 + phase / 20.0)),
            ("humidity", Value::Int(45 + i64::try_from(step % 30)?)),
            ("apparent_temperature", Value::Float(14.0 + phase / 20.0)),
            ("rain", Value::Float(if wet { 1.5 } else { 0.0 })),
            (
                "precipitation_probability",
                Value::Int(if wet { 80 } else { 10 }),
            ),
            ("wind_speed", Value::Float(2.0 + phase / 60.0)),
            ("wind_gusts", Value::Float(4.0 + phase / 30.0)),
            (
                "wind_direction",
                Value::Float((180.0 + phase).rem_euclid(360.0)),
            ),
            ("pressure", Value::Float(1012.0 + phase / 120.0)),
            ("cloud_cover", Value::Int(if wet { 90 } else { 20 })),
            ("uv_index", Value::Float(if daylight { 3.0 } else { 0.0 })),
            (
                "weather_code",
                Value::Text(if wet { "61" } else { "2" }.into()),
            ),
            ("daylight", Value::Bool(daylight)),
        ];
        let mut observations: Vec<_> = weather
            .into_iter()
            .map(|(key, value)| Observation {
                point: PointId::new(&self.weather_id, key),
                value,
            })
            .collect();
        let mut solar = vec![
            ("observed_at".to_owned(), Value::Int(at.as_second())),
            ("day_length".to_owned(), Value::Int(events.day_length)),
            ("elevation".to_owned(), Value::Float(elevation)),
            (
                "azimuth".to_owned(),
                azimuth.map_or(Value::Null, Value::Float),
            ),
        ];
        for (index, key) in EVENT_KEYS.iter().enumerate() {
            solar.push(((*key).to_owned(), timestamp_value(events.events[index])));
            solar.push((format!("next_{key}"), timestamp_value(future[index])));
        }
        let next = future
            .iter()
            .enumerate()
            .filter_map(|(index, time)| time.map(|t| (index, t)))
            .min_by_key(|(_, time)| *time);
        solar.push((
            "next_event_at".to_owned(),
            next.map_or(Value::Null, |(_, t)| Value::Int(t)),
        ));
        solar.push((
            "next_event".to_owned(),
            next.map_or(Value::Null, |(index, _)| {
                Value::Text(EVENT_KEYS[index].into())
            }),
        ));
        observations.extend(solar.into_iter().map(|(key, value)| Observation {
            point: PointId::new(&self.sun_id, &key),
            value,
        }));
        Ok(observations)
    }

    fn simulate_command(&self, _point: &PointId, _value: &Value) -> Result<Json> {
        bail!("synthetic weather and astronomy are read-only")
    }
}

fn timestamp_value(time: Option<i64>) -> Value {
    time.map_or(Value::Null, Value::Int)
}

fn device(instance: &InstanceId, id: DeviceId, name: &str, points: Vec<PointSpec>) -> Device {
    Device {
        id, instance: instance.clone(), native_name: name.into(), manufacturer: None,
        model: Some("synthetic-local-fixture-v1".into()),
        description: Some("Synthetic data; timezone=UTC; latitude=45N; longitude=0E; clock=2026-03-20T12:00:00Z+step*60s; approximate solar model; no network".into()),
        native_room: None, members: Vec::new(), points,
    }
}

fn spec(key: &str, kind: Kind, unit: Option<Unit>) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: format!("Fixture {key}").into(),
        kind,
        access: Access {
            read: true,
            write: false,
        },
        semantic: Semantic::infer(key, unit.as_ref()),
        unit,
    }
}

fn number(key: &str, unit: Option<Unit>, min: f64, max: f64) -> PointSpec {
    spec(
        key,
        Kind::Numeric {
            min: Some(min),
            max: Some(max),
            step: None,
        },
        unit,
    )
}

fn instant(key: &str) -> PointSpec {
    number(key, Some(Unit::Second), 0.0, 4_102_444_800.0)
}

fn weather_specs() -> Vec<PointSpec> {
    vec![
        instant("observed_at"),
        number("temperature", Some(Unit::Celsius), -100.0, 70.0),
        number("humidity", Some(Unit::Percent), 0.0, 100.0),
        number("apparent_temperature", Some(Unit::Celsius), -120.0, 90.0),
        number("rain", Some(Unit::Other("mm".into())), 0.0, 1000.0),
        number("precipitation_probability", Some(Unit::Percent), 0.0, 100.0),
        number("wind_speed", Some(Unit::Other("m/s".into())), 0.0, 150.0),
        number("wind_gusts", Some(Unit::Other("m/s".into())), 0.0, 150.0),
        number("wind_direction", Some(Unit::Other("°".into())), 0.0, 360.0),
        number("pressure", Some(Unit::HectoPascal), 500.0, 1200.0),
        number("cloud_cover", Some(Unit::Percent), 0.0, 100.0),
        number("uv_index", None, 0.0, 30.0),
        spec(
            "weather_code",
            Kind::Enum {
                values: WMO.iter().map(|value| (*value).into()).collect(),
            },
            None,
        ),
        spec("daylight", Kind::Binary, None),
    ]
}

fn sun_specs() -> Vec<PointSpec> {
    let mut points = vec![
        instant("observed_at"),
        number("day_length", Some(Unit::Second), 0.0, 86400.0),
        number("elevation", Some(Unit::Other("°".into())), -90.0, 90.0),
        number("azimuth", Some(Unit::Other("°".into())), 0.0, 360.0),
        instant("next_event_at"),
        spec(
            "next_event",
            Kind::Enum {
                values: EVENT_KEYS.iter().map(|key| (*key).into()).collect(),
            },
            None,
        ),
    ];
    for key in EVENT_KEYS {
        points.push(instant(key));
        points.push(instant(&format!("next_{key}")));
    }
    points
}

struct DayEvents {
    events: [Option<i64>; 9],
    day_length: i64,
}

/// Same daily fractional-year approximation as the existing production sun helper.
/// Using one shared twilight formula avoids separate algorithms per depression angle.
fn solar_terms(date: Date) -> (f64, f64) {
    let gamma = std::f64::consts::TAU / 365.0 * (f64::from(date.day_of_year()) - 1.0);
    let equation = 229.18
        * (0.000_075 + 0.001_868 * gamma.cos()
            - 0.032_077 * gamma.sin()
            - 0.014_615 * (2.0 * gamma).cos()
            - 0.040_849 * (2.0 * gamma).sin());
    let declination = 0.006_918 - 0.399_912 * gamma.cos() + 0.070_257 * gamma.sin()
        - 0.006_758 * (2.0 * gamma).cos()
        + 0.000_907 * (2.0 * gamma).sin()
        - 0.002_697 * (3.0 * gamma).cos()
        + 0.001_48 * (3.0 * gamma).sin();
    (equation, declination)
}

fn hour_angle(latitude: f64, declination: f64, depression: f64) -> Option<f64> {
    let latitude = latitude.to_radians();
    let denominator = latitude.cos() * declination.cos();
    if denominator.abs() < 1e-12 {
        return None;
    }
    let cosine =
        ((90.0 + depression).to_radians().cos() - latitude.sin() * declination.sin()) / denominator;
    (-1.0..=1.0)
        .contains(&cosine)
        .then(|| cosine.acos().to_degrees())
}

#[allow(clippy::cast_possible_truncation)]
fn seconds(minutes: f64) -> i64 {
    (minutes * 60.0).round() as i64
}

fn day_events(date: Date, latitude: f64, longitude: f64) -> Result<DayEvents> {
    ensure!(
        latitude.is_finite() && (-90.0..=90.0).contains(&latitude),
        "invalid fixture latitude"
    );
    ensure!(
        longitude.is_finite() && (-180.0..=180.0).contains(&longitude),
        "invalid fixture longitude"
    );
    let midnight = date.to_zoned(TimeZone::UTC)?.timestamp().as_second();
    let (equation, declination) = solar_terms(date);
    let noon = 720.0 - 4.0 * longitude - equation;
    let crossing = |depression| {
        hour_angle(latitude, declination, depression).map(|angle| {
            (
                midnight + seconds(noon - 4.0 * angle),
                midnight + seconds(noon + 4.0 * angle),
            )
        })
    };
    let astro = crossing(18.0);
    let nautical = crossing(12.0);
    let civil = crossing(6.0);
    let sun = existing_sun::times(date, latitude, longitude)
        .map(|(rise, set)| (rise.as_second(), set.as_second()));
    let day_length = sun.map_or_else(
        || {
            // No crossing: upper culmination distinguishes polar night and polar day.
            let maximum = 90.0 - (latitude - declination.to_degrees()).abs();
            if maximum > -0.833 { 86400 } else { 0 }
        },
        |(rise, set)| set - rise,
    );
    Ok(DayEvents {
        events: [
            astro.map(|pair| pair.0),
            nautical.map(|pair| pair.0),
            civil.map(|pair| pair.0),
            sun.map(|pair| pair.0),
            Some(midnight + seconds(noon)),
            sun.map(|pair| pair.1),
            civil.map(|pair| pair.1),
            nautical.map(|pair| pair.1),
            astro.map(|pair| pair.1),
        ],
        day_length,
    })
}

/// Each next event is strictly after `at`. A bounded horizon preserves Null if absent.
fn next_events(at: Timestamp, latitude: f64, longitude: f64) -> Result<[Option<i64>; 9]> {
    let date = at.to_zoned(TimeZone::UTC).date();
    let midnight = date.to_zoned(TimeZone::UTC)?.timestamp();
    let mut next = [None; 9];
    for day in 0..=370 {
        let day_at = midnight.checked_add(SignedDuration::from_secs(i64::from(day) * 86400))?;
        let candidate = day_events(day_at.to_zoned(TimeZone::UTC).date(), latitude, longitude)?;
        for (index, time) in candidate.events.into_iter().enumerate() {
            if next[index].is_none() && time.is_some_and(|time| time > at.as_second()) {
                next[index] = time;
            }
        }
        if next.iter().all(Option::is_some) {
            break;
        }
    }
    Ok(next)
}

/// Geometric elevation and north-clockwise azimuth, no atmospheric correction.
fn position(at: Timestamp, latitude: f64, longitude: f64) -> (f64, Option<f64>) {
    let time = at.to_zoned(TimeZone::UTC);
    let (equation, declination) = solar_terms(time.date());
    let minutes =
        f64::from(time.hour()) * 60.0 + f64::from(time.minute()) + f64::from(time.second()) / 60.0;
    let hour = ((minutes + equation + 4.0 * longitude) / 4.0 - 180.0).to_radians();
    let latitude = latitude.to_radians();
    let elevation = (latitude.sin() * declination.sin()
        + latitude.cos() * declination.cos() * hour.cos())
    .clamp(-1.0, 1.0)
    .asin();
    let east = -declination.cos() * hour.sin();
    let north =
        latitude.cos() * declination.sin() - latitude.sin() * declination.cos() * hour.cos();
    let azimuth =
        (east.hypot(north) >= 1e-12).then(|| east.atan2(north).to_degrees().rem_euclid(360.0));
    (elevation.to_degrees(), azimuth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equinox_orders_twilight_and_day_length_in_utc_seconds() {
        let date = Date::new(2026, 3, 20).unwrap();
        let day = day_events(date, LATITUDE, LONGITUDE).unwrap();
        let events: Vec<_> = day.events.into_iter().map(Option::unwrap).collect();
        assert!(events.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(day.day_length, events[5] - events[3]);
        assert!((11 * 3600..13 * 3600).contains(&day.day_length));
        let noon = Timestamp::from_second(events[4])
            .unwrap()
            .to_zoned(TimeZone::UTC);
        assert_eq!(noon.date(), date);
        assert_eq!(noon.hour(), 12);
        let (rise, set) = existing_sun::times(date, LATITUDE, LONGITUDE).unwrap();
        assert_eq!((events[3], events[5]), (rise.as_second(), set.as_second()));
    }

    #[test]
    fn polar_crossings_are_absent_without_fabricated_timestamps() {
        let winter = day_events(Date::new(2026, 12, 21).unwrap(), 80.0, 0.0).unwrap();
        let summer = day_events(Date::new(2026, 6, 21).unwrap(), 80.0, 0.0).unwrap();
        assert_eq!(
            (winter.events[3], winter.events[5], winter.day_length),
            (None, None, 0)
        );
        assert_eq!(
            (summer.events[3], summer.events[5], summer.day_length),
            (None, None, 86400)
        );
        assert!(
            summer
                .events
                .iter()
                .enumerate()
                .all(|(index, time)| index == 4 || time.is_none())
        );
        assert_eq!(timestamp_value(None), Value::Null);
        assert!(day_events(Date::new(2026, 6, 21).unwrap(), f64::NAN, 0.0).is_err());
    }

    #[test]
    fn angles_next_events_and_clock_have_bounded_conventions() {
        let at: Timestamp = CLOCK.parse().unwrap();
        let (elevation, azimuth) = position(at, LATITUDE, LONGITUDE);
        assert!((40.0..50.0).contains(&elevation));
        assert!((170.0..190.0).contains(&azimuth.unwrap()));
        for hour in 0..24 {
            let time = at
                .checked_add(SignedDuration::from_secs(hour * 3600))
                .unwrap();
            let (elevation, azimuth) = position(time, LATITUDE, LONGITUDE);
            assert!((-90.0..=90.0).contains(&elevation));
            assert!((0.0..360.0).contains(&azimuth.unwrap()));
        }
        let next = next_events(at, LATITUDE, LONGITUDE).unwrap();
        assert!(next.iter().all(|time| time.unwrap() > at.as_second()));
        assert!(next[3].unwrap() > next[5].unwrap()); // noon: next sunset before next sunrise
    }

    #[test]
    fn schemas_values_and_read_only_boundary_are_consistent() {
        let mut first = build(&InstanceId::from("fixture-weather")).unwrap();
        let mut second = build(&InstanceId::from("fixture-weather")).unwrap();
        let devices = first.take_description();
        assert_eq!(devices.len(), 2);
        assert!(first.take_description().is_empty());
        let observed = first.observe(0).unwrap();
        let repeated = second.observe(0).unwrap();
        assert_eq!(observed.len(), repeated.len());
        for (left, right) in observed.iter().zip(&repeated) {
            assert_eq!(left.point, right.point);
            assert_eq!(left.value, right.value);
        }
        for device in &devices {
            for point in &device.points {
                assert!(point.access.read && !point.access.write);
                let id = PointId::new(&device.id, &point.key);
                let value = &observed.iter().find(|item| item.point == id).unwrap().value;
                if point.key.as_ref() == "observed_at"
                    || point.key.ends_with("dawn")
                    || point.key.ends_with("dusk")
                    || point.key.contains("sunrise")
                    || point.key.contains("sunset")
                    || point.key.contains("noon")
                    || point.key.as_ref() == "next_event_at"
                    || point.key.as_ref() == "day_length"
                {
                    assert!(matches!(value, Value::Int(_) | Value::Null));
                    assert_eq!(point.unit, Some(Unit::Second));
                }
                if let Kind::Enum { values } = &point.kind {
                    let Value::Text(native) = value else {
                        panic!("fixture enum must be native text");
                    };
                    assert!(values.contains(native));
                }
                assert!(first.simulate_command(&id, &Value::Bool(true)).is_err());
            }
        }
        assert!(first.observe(MAX_STEP + 1).is_err());
        let next = first.observe(1).unwrap();
        for device in &devices {
            let clock = PointId::new(&device.id, "observed_at");
            assert_eq!(
                next.iter().find(|item| item.point == clock).unwrap().value,
                Value::Int(CLOCK.parse::<Timestamp>().unwrap().as_second() + 60)
            );
        }
        let weather = &devices[0];
        assert_eq!(
            weather.point("temperature").unwrap().unit,
            Some(Unit::Celsius)
        );
        assert_eq!(weather.point("humidity").unwrap().unit, Some(Unit::Percent));
        assert_eq!(
            weather.point("pressure").unwrap().unit,
            Some(Unit::HectoPascal)
        );
        assert_eq!(
            weather.point("wind_speed").unwrap().unit,
            Some(Unit::Other("m/s".into()))
        );
        assert_eq!(
            weather.point("wind_direction").unwrap().unit,
            Some(Unit::Other("°".into()))
        );
        assert_eq!(
            weather.point("rain").unwrap().unit,
            Some(Unit::Other("mm".into()))
        );
    }
}
