//! Bounded desired state, with no operation or executable expression.
use crate::PointId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneTarget {
    pub point: PointId,
    pub value: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSpec {
    pub targets: Vec<SceneTarget>,
    pub parallel: u8,
    pub timeout_s: u16,
    pub confirmation_s: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneInvalid;

impl SceneSpec {
    pub fn normalized(mut self) -> Result<Self, SceneInvalid> {
        if self.targets.is_empty()
            || self.targets.len() > 32
            || !(1..=4).contains(&self.parallel)
            || !(1..=120).contains(&self.timeout_s)
            || !(1..=30).contains(&self.confirmation_s)
            || u16::from(self.confirmation_s) > self.timeout_s
        {
            return Err(SceneInvalid);
        }
        let mut unique: Vec<SceneTarget> = Vec::with_capacity(self.targets.len());
        for target in self.targets {
            let id = target.point.as_str();
            if id.len() > 256
                || id.chars().any(char::is_control)
                || target.point.split().is_none()
                || target.value.is_null()
                || target.value.is_object()
                || target.value.is_array()
                || target.value.as_str().is_some_and(|v| v.len() > 2000)
                || crate::privacy::private_value(&crate::Value::from_json(&target.value))
            {
                return Err(SceneInvalid);
            }
            if let Some(previous) = unique.iter().find(|v| v.point == target.point) {
                if previous.value != target.value {
                    return Err(SceneInvalid);
                }
            } else {
                unique.push(target);
            }
        }
        self.targets = unique;
        if serde_json::to_vec(&self).map_or(true, |bytes| bytes.len() > 64 * 1024) {
            return Err(SceneInvalid);
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn spec() -> SceneSpec {
        SceneSpec {
            targets: vec![SceneTarget {
                point: "fixture:lamp/on".into(),
                value: json!(true),
            }],
            parallel: 4,
            timeout_s: 60,
            confirmation_s: 5,
        }
    }
    #[test]
    fn auto_s1_rejects_bounds_non_scalars_private_values_and_contradictions() {
        let mut scene = spec();
        assert!(scene.clone().normalized().is_ok());
        scene.targets.push(scene.targets[0].clone());
        assert_eq!(scene.clone().normalized().unwrap().targets.len(), 1);
        scene.targets[1].value = json!(false);
        assert!(scene.normalized().is_err());
        for value in [
            json!(null),
            json!({"toggle": true}),
            json!([1]),
            json!("x".repeat(2001)),
        ] {
            let mut scene = spec();
            scene.targets[0].value = value;
            assert!(scene.normalized().is_err());
        }
        for (parallel, timeout_s, confirmation_s) in [
            (0, 60, 5),
            (5, 60, 5),
            (1, 0, 5),
            (1, 121, 5),
            (1, 60, 0),
            (1, 60, 31),
            (1, 1, 2),
        ] {
            let mut scene = spec();
            scene.parallel = parallel;
            scene.timeout_s = timeout_s;
            scene.confirmation_s = confirmation_s;
            assert!(scene.normalized().is_err());
        }
    }
}
