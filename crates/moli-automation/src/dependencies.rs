//! Stable references in executable logic, covered by the graph fingerprint.
use crate::{Automation, Automations, Graph, Rule, Step};
use moli_core::{DependencyRef, PointId};
use moli_runtime::{DependencyConsumer, Hub};
use std::collections::{BTreeMap, BTreeSet};

fn point<'a>(rule: &'a Rule, points: &mut Vec<&'a str>) {
    if let Rule::State { point, .. } = rule {
        points.push(point);
    }
}

fn points(graph: &Graph) -> Vec<&str> {
    let mut points = Vec::new();
    for node in &graph.nodes {
        match &node.step {
            Step::WhenOccurrence { point, .. }
            | Step::WhenState { point, .. }
            | Step::WhenThreshold { point, .. }
            | Step::Set { point, .. }
            | Step::Toggle { point } => points.push(point.as_str()),
            Step::If { rules, .. } => {
                for rule in rules {
                    point(rule, &mut points);
                }
            }
            Step::WaitFor { rule, .. } => point(rule, &mut points),
            _ => {}
        }
    }
    points
}

fn template_points(graph: &Graph) -> Vec<&str> {
    let mut points = Vec::new();
    for node in &graph.nodes {
        let templates: Vec<&str> = match &node.step {
            Step::Notify { title, message, .. } => title
                .as_deref()
                .into_iter()
                .chain([message.as_str()])
                .collect(),
            Step::Write { prompt } => vec![prompt],
            _ => Vec::new(),
        };
        for mut rest in templates {
            while let Some(start) = rest.find("{{") {
                let Some(end) = rest[start + 2..].find("}}") else {
                    break;
                };
                let key = rest[start + 2..start + 2 + end].trim();
                if key.contains('/') {
                    points.push(key);
                }
                rest = &rest[start + 2 + end + 2..];
            }
        }
    }
    points
}

pub(crate) fn references(graph: &Graph, hub: &Hub) -> BTreeSet<DependencyRef> {
    let mut references: BTreeSet<_> = points(graph)
        .into_iter()
        .chain(template_points(graph))
        .filter_map(|p| {
            let (device, key) = PointId::from(p).split().map(|(d, k)| (d, k.to_owned()))?;
            let (found, _) = hub.describe(&device)?;
            found.point(&key)?.kind.dependency(&device)
        })
        .collect();
    for node in &graph.nodes {
        if let Step::Scene { scene } = &node.step {
            references.insert(DependencyRef {
                kind: moli_core::DependencyKind::Scene,
                id: scene.as_str().into(),
            });
        }
    }
    references
}

impl Automations {
    pub fn dependencies_ready(&self, graph: &Graph) -> bool {
        references(graph, &self.0.hub)
            .iter()
            .all(|r| self.0.hub.dependency_ready(r))
            && points(graph).into_iter().all(|p| {
                let Some((device, key)) = PointId::from(p).split().map(|(d, k)| (d, k.to_owned()))
                else {
                    return false;
                };
                let Some((found, _)) = self.0.hub.describe(&device) else {
                    return false;
                };
                found.point(&key).is_some_and(|p| {
                    p.kind
                        .dependency(&device)
                        .is_none_or(|r| self.0.hub.dependency_ready(&r))
                })
            })
    }
    pub(crate) fn runnable_version(&self, current: &Automation, original: &Automation) -> bool {
        current.is_live()
            && self.dependencies_ready(&current.graph)
            && current.fingerprint() == original.fingerprint()
    }

    pub(crate) fn index_dependencies(&self, list: &[Automation]) {
        let mut index: BTreeMap<DependencyRef, Vec<DependencyConsumer>> = BTreeMap::new();
        for a in list {
            let references = references(&a.graph, &self.0.hub);
            if references.is_empty() {
                continue;
            }
            let consumer = DependencyConsumer {
                id: a.id.clone(),
                name: a.name.clone(),
                fingerprint: a.fingerprint(),
            };
            for reference in references {
                index.entry(reference).or_default().push(consumer.clone());
            }
        }
        self.0.hub.set_dependency_consumers(index);
    }
}

/// A fresh consumer snapshot is computed at preview/publication while the shared
/// definition gate is held. A late bus notification cannot hide a consumer.
#[derive(Debug)]
pub(crate) struct ConsumerSource(pub std::sync::Weak<crate::Inner>);
impl moli_runtime::DependencyConsumerSource for ConsumerSource {
    fn consumers(
        &self,
        reference: &DependencyRef,
    ) -> Result<Vec<DependencyConsumer>, moli_runtime::DependencyError> {
        self.consumers_for(std::slice::from_ref(reference))
    }
    fn consumers_for(
        &self,
        references: &[DependencyRef],
    ) -> Result<Vec<DependencyConsumer>, moli_runtime::DependencyError> {
        let Some(inner) = self.0.upgrade() else {
            return Err(moli_runtime::DependencyError::Consumers);
        };
        if inner
            .dependency_generation
            .load(std::sync::atomic::Ordering::Acquire)
            != inner
                .dependency_persisted
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(moli_runtime::DependencyError::Consumers);
        }
        let wanted: BTreeSet<_> = references.iter().cloned().collect();
        let list = inner
            .list
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(list
            .iter()
            .filter(|a| {
                crate::dependencies::references(&a.graph, &inner.hub)
                    .iter()
                    .any(|r| wanted.contains(r))
            })
            .map(|a| DependencyConsumer {
                id: a.id.clone(),
                name: a.name.clone(),
                fingerprint: a.fingerprint(),
            })
            .collect())
    }
}
