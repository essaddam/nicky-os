//! Startup-only process layout; identities are independent of grouping.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Merged,
    Grouped,
    Isolated,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Tahoma,
    Ariston,
    Weather,
    AppleTv,
    Music,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub id: String,
    pub kind: Kind,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "fixture_source")]
    pub source_ref: String,
}
fn yes() -> bool {
    true
}
fn fixture_source() -> String {
    "fixture".into()
}
fn seconds() -> u64 {
    5
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: String,
    pub modules: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub mode: Mode,
    pub modules: Vec<Module>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default = "seconds")]
    pub seconds: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Plan {
    pub id: String,
    pub modules: Vec<Module>,
}
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Layout {
    pub fn plan(&self) -> Result<Vec<Plan>> {
        ensure!(
            self.seconds == 0 || (2..=30).contains(&self.seconds),
            "duration must be 0 (continuous) or 2..30 seconds"
        );
        ensure!(
            self.modules.len() <= 32 && self.groups.len() <= 8,
            "module/group bound exceeded"
        );
        let mut known = BTreeMap::new();
        let mut owners = BTreeSet::new();
        for module in &self.modules {
            ensure!(
                safe_id(&module.id) && safe_id(&module.source_ref),
                "invalid module/source identity"
            );
            ensure!(
                known.insert(module.id.clone(), module.clone()).is_none(),
                "duplicate module identity"
            );
            if module.enabled {
                ensure!(
                    owners.insert((module.kind, module.source_ref.clone())),
                    "duplicate source owner"
                );
            }
        }
        let active: Vec<_> = self.modules.iter().filter(|m| m.enabled).cloned().collect();
        match self.mode {
            Mode::Merged => Ok(if active.is_empty() {
                Vec::new()
            } else {
                vec![Plan {
                    id: "merged".into(),
                    modules: active,
                }]
            }),
            Mode::Isolated => Ok(active
                .into_iter()
                .map(|module| Plan {
                    id: module.id.clone(),
                    modules: vec![module],
                })
                .collect()),
            Mode::Grouped => {
                let mut ids = BTreeSet::new();
                let mut assigned = BTreeSet::new();
                let mut plans = Vec::new();
                for group in &self.groups {
                    ensure!(
                        safe_id(&group.id) && ids.insert(group.id.clone()),
                        "invalid or duplicate group identity"
                    );
                    let mut modules = Vec::new();
                    for name in &group.modules {
                        let module = known.get(name).context("unknown module in group")?;
                        ensure!(
                            assigned.insert(name.clone()),
                            "module assigned to multiple groups"
                        );
                        if module.enabled {
                            modules.push(module.clone());
                        }
                    }
                    if !modules.is_empty() {
                        plans.push(Plan {
                            id: group.id.clone(),
                            modules,
                        });
                    }
                }
                ensure!(
                    active.iter().all(|m| assigned.contains(&m.id)),
                    "active module has no process group"
                );
                Ok(plans)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout(mode: Mode) -> Layout {
        Layout {
            mode,
            seconds: 5,
            modules: vec![
                Module {
                    id: "tahoma".into(),
                    kind: Kind::Tahoma,
                    enabled: true,
                    source_ref: "fixture".into(),
                },
                Module {
                    id: "ariston".into(),
                    kind: Kind::Ariston,
                    enabled: true,
                    source_ref: "fixture".into(),
                },
            ],
            groups: vec![],
        }
    }
    #[test]
    fn regrouping_preserves_ids_and_disabled_modules_never_get_a_group() {
        let mut l = layout(Mode::Merged);
        let merged = l.plan().unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].modules.len(), 2);
        l.mode = Mode::Isolated;
        assert_eq!(l.plan().unwrap().len(), 2);
        l.modules[1].enabled = false;
        let p = l.plan().unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].modules[0].id, "tahoma");
        l.modules[0].enabled = false;
        assert!(l.plan().unwrap().is_empty());
    }
    #[test]
    fn startup_rejects_duplicate_owners_unknown_members_and_missing_assignments() {
        let mut l = layout(Mode::Grouped);
        l.groups = vec![Group {
            id: "one".into(),
            modules: vec!["tahoma".into(), "ariston".into()],
        }];
        assert!(l.plan().is_ok());
        l.groups[0].modules.pop();
        assert!(l.plan().is_err());
        l.groups[0].modules.push("missing".into());
        assert!(l.plan().is_err());
        l = layout(Mode::Isolated);
        l.modules[1].kind = Kind::Tahoma;
        assert!(l.plan().is_err());
        l.modules[1].source_ref = "other-fixture".into();
        assert!(l.plan().is_ok());
        l.modules[1].id = "tahoma".into();
        assert!(l.plan().is_err());
    }
}
