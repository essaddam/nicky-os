use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// A typed value. Serialized as plain JSON (`true`, `42`, `21.5`, `"auto"`, `null`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Schema-bound catalogue code; plain JSON numbers never infer this type.
    #[serde(skip_deserializing)]
    EnumCode(u8),
    Text(Arc<str>),
}

impl Value {
    /// Converts a JSON scalar. Objects and arrays are not values (drivers
    /// flatten them into dotted keys); they are kept as compact JSON text so
    /// nothing is silently lost.
    #[must_use]
    pub fn from_json(json: &serde_json::Value) -> Self {
        match json {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(*b),
            serde_json::Value::Number(n) => n
                .as_i64()
                .map_or_else(|| Self::Float(n.as_f64().unwrap_or(f64::NAN)), Self::Int),
            serde_json::Value::String(s) => Self::Text(s.as_str().into()),
            other => Self::Text(other.to_string().into()),
        }
    }

    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(b) => (*b).into(),
            Self::Int(i) => (*i).into(),
            Self::EnumCode(code) => (*code).into(),
            Self::Float(f) => {
                serde_json::Number::from_f64(*f).map_or(serde_json::Value::Null, Into::into)
            }
            Self::Text(s) => serde_json::Value::String(s.to_string()),
        }
    }

    /// Numeric view (ints widen to floats).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(i) => Some(*i as f64),
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Self::Float(f)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Self::Int(i)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::Text(s.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_round_trip() {
        for (j, v) in [
            (json!(null), Value::Null),
            (json!(true), Value::Bool(true)),
            (json!(42), Value::Int(42)),
            (json!(21.5), Value::Float(21.5)),
            (json!("auto"), Value::from("auto")),
        ] {
            assert_eq!(Value::from_json(&j), v);
            assert_eq!(v.to_json(), j);
            assert_eq!(serde_json::to_value(&v).unwrap(), j);
            assert_eq!(serde_json::from_value::<Value>(j).unwrap(), v);
        }
    }

    #[test]
    fn compound_json_is_kept_as_text() {
        assert_eq!(Value::from_json(&json!([1, 2])), Value::from("[1,2]"));
    }

    #[test]
    fn enum_codes_serialize_as_numbers_but_require_a_schema_to_decode() {
        for code in [0, 1, 127, 255] {
            let value = Value::EnumCode(code);
            assert_eq!(value.to_json(), json!(code));
            assert_eq!(serde_json::to_value(&value).unwrap(), json!(code));
            assert_eq!(Value::from_json(&json!(code)), Value::Int(i64::from(code)));
            assert_eq!(
                serde_json::from_value::<Value>(json!(code)).unwrap(),
                Value::Int(i64::from(code))
            );
            assert_eq!(value.as_f64(), None);
        }
        assert!(std::mem::size_of::<Value>() <= 24);
        assert!(std::mem::size_of::<crate::Sample>() <= 32);
    }
}
