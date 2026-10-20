//! Live-only, at-most-once admission. A crash is reported, never replayed.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use moli_core::{Occurrence, OccurrenceId};
use moli_runtime::StateFile;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{Automation, Step};

const MAX_SOURCES: usize = 128;
const MAX_PENDING: usize = 256;

#[derive(Clone, Debug)]
pub(crate) struct Target {
    pub automation: usize,
    pub trigger: String,
    fingerprint: String,
    floor: u64,
    activation: CancellationToken,
}

#[derive(Debug, Default)]
pub(crate) struct Index {
    pub live: Arc<Vec<Automation>>,
    points: HashMap<String, Vec<Target>>,
}

impl Index {
    pub fn refresh(
        &mut self,
        live: Arc<Vec<Automation>>,
        position: u64,
        mut activate: impl FnMut(&str) -> CancellationToken,
    ) {
        let mut points: HashMap<String, Vec<Target>> = HashMap::new();
        for (automation, a) in live.iter().enumerate() {
            let fingerprint = a.fingerprint();
            for node in a.graph.triggers() {
                if let Step::WhenOccurrence { point, .. } = &node.step {
                    let previous = self.points.get(point).and_then(|targets| {
                        targets.iter().find(|t| {
                            self.live[t.automation].id == a.id
                                && t.trigger == node.id
                                && t.fingerprint == fingerprint
                                && !t.activation.is_cancelled()
                        })
                    });
                    points.entry(point.clone()).or_default().push(Target {
                        automation,
                        trigger: node.id.clone(),
                        fingerprint: fingerprint.clone(),
                        floor: previous.map_or(position, |t| t.floor),
                        activation: previous
                            .map_or_else(|| activate(&a.id), |t| t.activation.clone()),
                    });
                }
            }
        }
        self.live = live;
        self.points = points;
    }

    pub fn targets(&self, event: &Occurrence) -> Vec<(Automation, String, CancellationToken)> {
        self.points
            .get(event.point.as_str())
            .into_iter()
            .flatten()
            .filter_map(|target| {
                if event.position <= target.floor || target.activation.is_cancelled() {
                    return None;
                }
                let a = &self.live[target.automation];
                let Step::WhenOccurrence { value, .. } = &a.graph.node(&target.trigger)?.step
                else {
                    return None;
                };
                if value
                    .as_ref()
                    .is_some_and(|value| !crate::run::same(&event.value, value))
                {
                    return None;
                }
                Some((
                    a.clone(),
                    target.trigger.clone(),
                    target.activation.child_token(),
                ))
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Window {
    highest: u64,
    seen: u64,
    durable: bool,
    stream: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    event: Occurrence,
    remaining: BTreeSet<(String, String)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    windows: BTreeMap<String, Window>,
    pending: BTreeMap<String, Pending>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            windows: BTreeMap::new(),
            pending: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Consumer {
    file: StateFile,
    state: Option<State>,
    fenced: bool,
}

pub(crate) fn key(id: &OccurrenceId) -> String {
    format!("{}:{}", id.source, id.sequence)
}

impl Consumer {
    pub fn new(path: Option<std::path::PathBuf>) -> Self {
        Self {
            file: StateFile::new(path, 1024 * 1024),
            state: None,
            fenced: false,
        }
    }

    async fn save(&mut self, state: State) -> std::io::Result<()> {
        match self.file.write(&state).await {
            Ok(()) => {
                self.state = Some(state);
                Ok(())
            }
            Err(error) => {
                if error.commit_unknown {
                    self.fenced = true;
                }
                Err(error.error)
            }
        }
    }

    /// Old pending runs are ambiguous. They are surfaced before accepting more work.
    pub async fn recover(&mut self, stream: &str) -> std::io::Result<Vec<Occurrence>> {
        if self.fenced {
            return Err(std::io::Error::other("occurrence consumer fenced"));
        }
        if self.state.is_none() {
            let state: State = self.file.load().await?.unwrap_or_default();
            if state.version != 1
                || state.windows.len() > MAX_SOURCES
                || state.pending.len() > MAX_PENDING
                || state.windows.iter().any(|(source, w)| {
                    !(OccurrenceId {
                        source: source.as_str().into(),
                        sequence: w.highest,
                    })
                    .is_valid()
                        || w.seen & 1 == 0
                })
                || state.pending.iter().any(|(id, p)| {
                    *id != key(&p.event.id)
                        || !p.event.id.is_valid()
                        || p.remaining.is_empty()
                        || p.remaining.len() > 1024
                })
            {
                return Err(std::io::Error::other("invalid occurrence consumer state"));
            }
            self.state = Some(state);
        }
        let mut state = self
            .state
            .clone()
            .ok_or_else(|| std::io::Error::other("missing consumer state"))?;
        let interrupted: Vec<Occurrence> = state
            .pending
            .values()
            .filter(|p| p.event.stream.as_ref() != stream)
            .map(|p| p.event.clone())
            .collect();
        state
            .pending
            .retain(|_, p| p.event.stream.as_ref() == stream);
        state.windows.retain(|_, w| w.durable || w.stream == stream);
        if interrupted.is_empty() {
            self.state = Some(state);
        } else {
            self.save(state).await?;
        }
        Ok(interrupted)
    }

    pub async fn claim(
        &mut self,
        event: &Occurrence,
        targets: BTreeSet<(String, String)>,
    ) -> std::io::Result<bool> {
        if self.fenced {
            return Err(std::io::Error::other("occurrence consumer fenced"));
        }
        if targets.is_empty() || targets.len() > 1024 {
            return Err(std::io::Error::other("invalid occurrence targets"));
        }
        let mut state = self
            .state
            .clone()
            .ok_or_else(|| std::io::Error::other("consumer not recovered"))?;
        if state.pending.len() >= MAX_PENDING {
            return Err(std::io::Error::other("too many pending occurrences"));
        }
        let source = event.id.source.to_string();
        if !state.windows.contains_key(&source) && state.windows.len() >= MAX_SOURCES {
            return Err(std::io::Error::other("too many occurrence sources"));
        }
        let window = state.windows.entry(source).or_insert_with(|| Window {
            highest: event.id.sequence,
            seen: 0,
            durable: event.durable,
            stream: event.stream.to_string(),
        });
        if window.durable != event.durable {
            return Err(std::io::Error::other("occurrence source changed"));
        }
        let seq = event.id.sequence;
        if seq > window.highest {
            window.seen = window
                .seen
                .checked_shl(u32::try_from(seq - window.highest).unwrap_or(u32::MAX))
                .unwrap_or(0);
            window.highest = seq;
        }
        let distance = window.highest - seq;
        if distance >= 64 || window.seen & (1_u64 << distance) != 0 {
            return Ok(false);
        }
        window.seen |= 1_u64 << distance;
        state.pending.insert(
            key(&event.id),
            Pending {
                event: event.clone(),
                remaining: targets,
            },
        );
        self.save(state).await?;
        Ok(true)
    }

    pub async fn complete(
        &mut self,
        id: &OccurrenceId,
        automation: &str,
        trigger: &str,
    ) -> std::io::Result<()> {
        if self.fenced {
            return Err(std::io::Error::other("occurrence consumer fenced"));
        }
        let mut state = self
            .state
            .clone()
            .ok_or_else(|| std::io::Error::other("missing consumer state"))?;
        let id = key(id);
        if let Some(pending) = state.pending.get_mut(&id) {
            pending
                .remaining
                .remove(&(automation.to_owned(), trigger.to_owned()));
            if pending.remaining.is_empty() {
                state.pending.remove(&id);
            }
            self.save(state).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(sequence: u64, stream: &str) -> Occurrence {
        Occurrence {
            id: OccurrenceId {
                source: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                sequence,
            },
            stream: stream.into(),
            position: sequence,
            point: "simulated:button/press".into(),
            value: moli_core::Value::Null,
            ts: 42,
            durable: true,
            correlation: None,
        }
    }

    fn target() -> BTreeSet<(String, String)> {
        BTreeSet::from([("automation".into(), "trigger".into())])
    }

    #[test]
    fn auto_h4_delayed_events_cannot_cross_graph_validation_boundaries() {
        let mut automation = Automation {
            id: "automation".into(),
            name: "Simulated".into(),
            enabled: true,
            mode: crate::Mode::Queued,
            graph: serde_json::from_value(serde_json::json!({ "nodes": [
                { "id": "trigger", "type": "when_occurrence", "point": "simulated:button/press" }
            ], "edges": [] }))
            .unwrap(),
            author: crate::Author::Human,
            approved: None,
            approved_version: None,
            note: None,
            created: 0,
            updated: 0,
        };
        let mut index = Index::default();
        index.refresh(Arc::new(vec![automation.clone()]), 10, |_| {
            CancellationToken::new()
        });
        assert!(
            index
                .targets(&event(10, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"))
                .is_empty()
        );
        assert_eq!(
            index
                .targets(&event(11, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"))
                .len(),
            1
        );
        index.refresh(Arc::new(vec![automation.clone()]), 20, |_| {
            CancellationToken::new()
        });
        assert_eq!(
            index
                .targets(&event(11, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"))
                .len(),
            1,
            "an unchanged version keeps its admission boundary"
        );
        automation.graph.nodes.push(
            serde_json::from_value(
                serde_json::json!({ "id": "delay", "type": "wait", "seconds": 1 }),
            )
            .unwrap(),
        );
        index.refresh(Arc::new(vec![automation]), 20, |_| CancellationToken::new());
        assert!(
            index
                .targets(&event(11, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"))
                .is_empty(),
            "a delayed event cannot start a different version"
        );
        assert_eq!(
            index
                .targets(&event(21, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"))
                .len(),
            1
        );
        index.refresh(Arc::default(), 30, |_| CancellationToken::new());
        assert!(
            index
                .targets(&event(31, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"))
                .is_empty(),
            "no draft is indexed"
        );
    }

    #[test]
    fn auto_h4_same_fingerprint_never_revives_a_cancelled_activation() {
        let automation = Automation {
            id: "automation".into(),
            name: "Simulated".into(),
            enabled: true,
            mode: crate::Mode::Queued,
            graph: serde_json::from_value(serde_json::json!({ "nodes": [
                { "id": "trigger", "type": "when_occurrence", "point": "simulated:button/press" }
            ], "edges": [] }))
            .unwrap(),
            author: crate::Author::Human,
            approved: None,
            approved_version: None,
            note: None,
            created: 0,
            updated: 0,
        };
        let first = CancellationToken::new();
        let second = CancellationToken::new();
        let mut index = Index::default();
        index.refresh(Arc::new(vec![automation.clone()]), 10, |_| first.clone());
        let captured = index.targets(&event(11, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
        assert_eq!(captured.len(), 1);
        first.cancel();
        // Even if two concurrent definition writes coalesce before refresh,
        // the indexed parent remembers the off transition forever.
        index.refresh(Arc::new(vec![automation]), 20, |_| second.clone());
        assert!(captured[0].2.is_cancelled());
        assert!(
            index
                .targets(&event(11, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"))
                .is_empty()
        );
        let fresh = index.targets(&event(21, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
        assert_eq!(fresh.len(), 1);
        assert!(!fresh[0].2.is_cancelled());
    }

    #[tokio::test]
    async fn auto_h4_consumer_rejects_duplicate_old_and_restart_replay() {
        let root = std::env::temp_dir().join(format!(
            "moli-h4-consumer-{}-{}",
            std::process::id(),
            moli_core::now_ms()
        ));
        let path = root.join("occurrences.json");
        let mut consumer = Consumer::new(Some(path.clone()));
        consumer
            .recover("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .await
            .unwrap();
        let first = event(2, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        assert!(consumer.claim(&first, target()).await.unwrap());
        assert!(!consumer.claim(&first, target()).await.unwrap());
        assert!(
            consumer
                .claim(&event(1, &first.stream), target())
                .await
                .unwrap(),
            "out-of-order within the window"
        );
        consumer
            .complete(&first.id, "automation", "trigger")
            .await
            .unwrap();
        drop(consumer);
        let mut consumer = Consumer::new(Some(path));
        let interrupted = consumer
            .recover("cccccccccccccccccccccccccccccccc")
            .await
            .unwrap();
        assert_eq!(
            interrupted.len(),
            1,
            "unfinished admission is surfaced, never replayed"
        );
        assert_eq!(interrupted[0].id.sequence, 1);
        assert!(
            !consumer
                .claim(&event(2, "cccccccccccccccccccccccccccccccc"), target())
                .await
                .unwrap()
        );
        assert!(
            consumer
                .claim(&event(80, "cccccccccccccccccccccccccccccccc"), target())
                .await
                .unwrap()
        );
        assert!(
            !consumer
                .claim(&event(3, "cccccccccccccccccccccccccccccccc"), target())
                .await
                .unwrap(),
            "old IDs do not become fresh after eviction"
        );
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn auto_h4_consumer_corruption_blocks_actions_and_preserves_evidence() {
        let root = std::env::temp_dir().join(format!(
            "moli-h4-corrupt-consumer-{}-{}",
            std::process::id(),
            moli_core::now_ms()
        ));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let path = root.join("occurrences.json");
        tokio::fs::write(&path, b"broken").await.unwrap();
        let mut consumer = Consumer::new(Some(path.clone()));
        assert!(
            consumer
                .recover("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
                .await
                .is_err()
        );
        assert!(
            consumer
                .claim(&event(1, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"), target())
                .await
                .is_err()
        );
        assert_eq!(tokio::fs::read(path).await.unwrap(), b"broken");
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
