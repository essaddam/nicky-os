//! JSON only: duplicate keys, excessive nesting and resource use fail closed.
use crate::Failure;
use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;
pub(crate) const MAX_BODY: usize = 1024 * 1024;
struct Seed<'a> {
    depth: usize,
    nodes: &'a mut usize,
}
fn text<E: Error>(value: &str) -> Result<(), E> {
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(E::custom("invalid JSON field"));
    }
    Ok(())
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        *self.nodes += 1;
        if self.depth > 16 || *self.nodes > 10000 {
            return Err(D::Error::custom("JSON limit"));
        }
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_bool<E: Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E: Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_f64<E: Error>(self, v: f64) -> Result<Value, E> {
        Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite number"))
    }
    fn visit_unit<E: Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_str<E: Error>(self, v: &str) -> Result<Value, E> {
        text::<E>(v)?;
        Ok(Value::String(v.to_owned()))
    }
    fn visit_string<E: Error>(self, v: String) -> Result<Value, E> {
        text::<E>(&v)?;
        Ok(Value::String(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(v) = seq.next_element_seed(Seed {
            depth: self.depth + 1,
            nodes: self.nodes,
        })? {
            values.push(v);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(k) = map.next_key::<String>()? {
            text::<A::Error>(&k)?;
            if values.contains_key(&k) {
                return Err(A::Error::custom("duplicate JSON key"));
            }
            let v = map.next_value_seed(Seed {
                depth: self.depth + 1,
                nodes: self.nodes,
            })?;
            values.insert(k, v);
        }
        Ok(Value::Object(values))
    }
}
pub(crate) fn parse(body: &[u8]) -> Result<Value, Failure> {
    if body.len() > MAX_BODY {
        return Err(Failure::Invalid);
    }
    let mut d = serde_json::Deserializer::from_slice(body);
    let mut nodes = 0;
    let v = Seed {
        depth: 1,
        nodes: &mut nodes,
    }
    .deserialize(&mut d)
    .map_err(|_| Failure::Invalid)?;
    d.end().map_err(|_| Failure::Invalid)?;
    if !v.is_object() {
        return Err(Failure::Invalid);
    }
    Ok(v)
}
