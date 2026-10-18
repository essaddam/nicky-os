//! Scan JSON one member at a time; never retain an array or object tree.
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::fmt;

pub(super) fn inspect(text: &str, depth: usize) -> bool {
    let mut parser = serde_json::Deserializer::from_str(text);
    Probe {
        depth,
        point: false,
    }
    .deserialize(&mut parser)
    .and_then(|found| parser.end().map(|()| found))
    // An error cannot prove absence of a code, including before its field.
    .unwrap_or(true)
}

struct Probe {
    depth: usize,
    point: bool,
}

impl<'de> DeserializeSeed<'de> for Probe {
    type Value = bool;

    fn deserialize<D: de::Deserializer<'de>>(self, parser: D) -> Result<bool, D::Error> {
        if self.depth >= 16 {
            parser.deserialize_ignored_any(IgnoredAny)?;
            Ok(true)
        } else {
            parser.deserialize_any(self)
        }
    }
}

impl<'de> Visitor<'de> for Probe {
    type Value = bool;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<bool, M::Error> {
        let mut found = false;
        while let Some(key) = map.next_key::<String>()? {
            let private = super::code_property(&key);
            let point = key == "point";
            drop(key);
            let child = map.next_value_seed(Self {
                depth: self.depth + 1,
                point,
            })?;
            found |= private || child;
        }
        Ok(found)
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut sequence: S) -> Result<bool, S::Error> {
        let mut found = false;
        while let Some(child) = sequence.next_element_seed(Self {
            depth: self.depth + 1,
            point: false,
        })? {
            found |= child;
        }
        Ok(found)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<bool, E> {
        Ok((self.point && super::code_point(value)) || super::inspect_text(value, self.depth + 1))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<bool, E> {
        Ok(false)
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<bool, E> {
        Ok(false)
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<bool, E> {
        Ok(false)
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<bool, E> {
        Ok(false)
    }
    fn visit_unit<E: de::Error>(self) -> Result<bool, E> {
        Ok(false)
    }
}
