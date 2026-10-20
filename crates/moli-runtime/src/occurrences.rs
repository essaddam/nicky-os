//! Live occurrence identities and bounded, durable button request receipts.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use moli_core::{MAX_OCCURRENCE_SEQUENCE, Occurrence, OccurrenceId, PointId, RequestId, Value};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

use crate::StateFile;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccurrenceError {
    RequestId,
    Value,
    Source,
    Conflict,
    Stale,
    Capacity,
    Sequence,
    CorruptState,
    Storage,
    Uncertain,
}

impl std::fmt::Display for OccurrenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use moli_i18n::tr;
        let message = match self {
            Self::RequestId => tr!("serveur.occurrence.requete_id"),
            Self::Value => tr!("serveur.occurrence.valeur"),
            Self::Source => tr!("serveur.occurrence.source_invalide"),
            Self::Conflict => tr!("serveur.occurrence.conflit"),
            Self::Stale => tr!("serveur.occurrence.ancienne"),
            Self::Capacity => tr!("serveur.occurrence.capacite"),
            Self::Sequence => tr!("serveur.occurrence.sequence_epuisee"),
            Self::CorruptState => tr!("serveur.occurrence.corrompue"),
            Self::Storage => tr!("serveur.occurrence.persistance"),
            Self::Uncertain => tr!("serveur.occurrence.ambigue"),
        };
        f.write_str(&message)
    }
}

impl std::error::Error for OccurrenceError {}

pub(crate) fn random_id() -> std::io::Result<Arc<str>> {
    let mut bytes = [0_u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| std::io::Error::other("cannot create occurrence identity"))?;
    let mut text = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    Ok(text.into())
}

#[derive(Debug, Default)]
pub(crate) struct Stream {
    id: Mutex<Option<Arc<str>>>,
    position: AtomicU64,
}

impl Stream {
    pub(crate) fn current(&self) -> Option<Arc<str>> {
        self.id
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn position(&self) -> u64 {
        self.position.load(Ordering::Acquire)
    }

    pub(crate) fn event(
        &self,
        point: PointId,
        value: Value,
        ts: u64,
        identity: Option<OccurrenceId>,
        correlation: Option<String>,
        durable: bool,
    ) -> std::io::Result<Occurrence> {
        let stream = {
            let mut id = self.id.lock().unwrap_or_else(PoisonError::into_inner);
            if id.is_none() {
                *id = Some(random_id()?);
            }
            id.as_ref()
                .cloned()
                .ok_or_else(|| std::io::Error::other("missing occurrence stream"))?
        };
        let mut previous = self.position.load(Ordering::Acquire);
        let position = loop {
            let next = previous
                .checked_add(1)
                .filter(|next| *next <= MAX_OCCURRENCE_SEQUENCE)
                .ok_or_else(|| std::io::Error::other("occurrence sequence exhausted"))?;
            match self.position.compare_exchange_weak(
                previous,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break next,
                Err(observed) => previous = observed,
            }
        };
        let id = identity.unwrap_or_else(|| OccurrenceId {
            source: stream.clone(),
            sequence: position,
        });
        if !id.is_valid() {
            return Err(std::io::Error::other("invalid occurrence identity"));
        }
        Ok(Occurrence {
            id,
            stream,
            position,
            point,
            value,
            ts,
            durable,
            correlation,
        })
    }
}

pub(crate) const REQUEST_CLIENTS: usize = 64;
pub(crate) const REQUEST_WINDOW: u64 = 32;
const RECENT_UNKEYED: usize = 256;
const STATE_VERSION: u32 = 1;
pub(crate) const STATE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Dispatch {
    /// Persisted before publication. Never republished on retry or restart.
    Uncertain,
    Published,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    pub point: PointId,
    pub id: OccurrenceId,
    pub correlation: String,
    pub status: Dispatch,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Client {
    highest: u64,
    requests: BTreeMap<u64, Receipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ButtonState {
    version: u32,
    delivery: String,
    source: Arc<str>,
    next_sequence: u64,
    clients: BTreeMap<String, Client>,
    unkeyed: VecDeque<Receipt>,
}

impl ButtonState {
    fn new() -> std::io::Result<Self> {
        Ok(Self {
            version: STATE_VERSION,
            delivery: "live_only_at_most_once".into(),
            source: random_id()?,
            next_sequence: 1,
            clients: BTreeMap::new(),
            unkeyed: VecDeque::new(),
        })
    }

    fn valid(&self) -> bool {
        self.version == STATE_VERSION
            && self.delivery == "live_only_at_most_once"
            && (1..=MAX_OCCURRENCE_SEQUENCE + 1).contains(&self.next_sequence)
            && self.clients.len() <= REQUEST_CLIENTS
            && self.unkeyed.len() <= RECENT_UNKEYED
            && OccurrenceId {
                source: self.source.clone(),
                sequence: 1,
            }
            .is_valid()
            && self.clients.iter().all(|(name, client)| {
                format!("{name}:1").parse::<RequestId>().is_ok()
                    && client.highest > 0
                    && !client.requests.is_empty()
                    && u64::try_from(client.requests.len()).is_ok_and(|len| len <= REQUEST_WINDOW)
                    && client.requests.iter().all(|(seq, receipt)| {
                        *seq > client.highest.saturating_sub(REQUEST_WINDOW)
                            && *seq <= client.highest
                            && self.valid_receipt(receipt)
                    })
            })
            && self
                .unkeyed
                .iter()
                .all(|receipt| self.valid_receipt(receipt))
    }

    fn valid_receipt(&self, receipt: &Receipt) -> bool {
        receipt.id.is_valid()
            && receipt.id.source == self.source
            && receipt.id.sequence < self.next_sequence
            && receipt.point.split().is_some()
            && receipt.point.as_str().len() <= 256
            && (receipt.correlation.parse::<RequestId>().is_ok()
                || (receipt.correlation.len() == 32
                    && receipt.correlation.bytes().all(|b| b.is_ascii_hexdigit())))
    }
}

#[derive(Debug)]
pub(crate) struct Buttons {
    pub file: StateFile,
    state: Option<ButtonState>,
    fenced: bool,
}

pub(crate) enum Reservation {
    New(Receipt),
    Duplicate(Receipt),
}

impl Buttons {
    pub(crate) fn new(path: Option<std::path::PathBuf>) -> Self {
        Self {
            file: StateFile::new(path, STATE_BYTES),
            state: None,
            fenced: false,
        }
    }

    async fn ready(&mut self) -> Result<(), OccurrenceError> {
        if self.fenced {
            return Err(OccurrenceError::Uncertain);
        }
        if self.state.is_none() {
            let state: ButtonState = self
                .file
                .load()
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "button business state could not be loaded");
                    OccurrenceError::Storage
                })?
                .map_or_else(ButtonState::new, Ok)
                .map_err(|error| {
                    tracing::warn!(%error, "button identity could not be initialized");
                    OccurrenceError::Storage
                })?;
            if !state.valid() {
                return Err(OccurrenceError::CorruptState);
            }
            self.state = Some(state);
        }
        Ok(())
    }

    async fn persist(&mut self, state: ButtonState) -> Result<(), OccurrenceError> {
        match self.file.write(&state).await {
            Ok(()) => {
                self.state = Some(state);
                Ok(())
            }
            Err(error) => {
                if error.commit_unknown {
                    self.state = Some(state);
                    self.fenced = true;
                }
                tracing::warn!(%error, "button business state was not acknowledged");
                Err(if error.commit_unknown {
                    OccurrenceError::Uncertain
                } else {
                    OccurrenceError::Storage
                })
            }
        }
    }

    pub(crate) async fn reserve(
        &mut self,
        point: &PointId,
        request: Option<&RequestId>,
        correlation: &str,
    ) -> Result<Reservation, OccurrenceError> {
        self.ready().await?;
        let mut state = self.state.clone().ok_or(OccurrenceError::CorruptState)?;
        if let Some(request) = request {
            if let Some(client) = state.clients.get(request.client()) {
                if let Some(receipt) = client.requests.get(&request.sequence()) {
                    if &receipt.point != point {
                        return Err(OccurrenceError::Conflict);
                    }
                    return Ok(Reservation::Duplicate(receipt.clone()));
                }
                if request.sequence() <= client.highest.saturating_sub(REQUEST_WINDOW) {
                    return Err(OccurrenceError::Stale);
                }
            } else if state.clients.len() == REQUEST_CLIENTS {
                return Err(OccurrenceError::Capacity);
            }
        }
        if state.next_sequence > MAX_OCCURRENCE_SEQUENCE {
            return Err(OccurrenceError::Sequence);
        }
        let receipt = Receipt {
            point: point.clone(),
            id: OccurrenceId {
                source: state.source.clone(),
                sequence: state.next_sequence,
            },
            correlation: correlation.into(),
            status: Dispatch::Uncertain,
        };
        state.next_sequence += 1;
        if let Some(request) = request {
            let client = state.clients.entry(request.client().into()).or_default();
            client.highest = client.highest.max(request.sequence());
            client
                .requests
                .retain(|sequence, _| *sequence > client.highest.saturating_sub(REQUEST_WINDOW));
            client.requests.insert(request.sequence(), receipt.clone());
        } else {
            if state.unkeyed.len() == RECENT_UNKEYED {
                state.unkeyed.pop_front();
            }
            state.unkeyed.push_back(receipt.clone());
        }
        self.persist(state).await?;
        Ok(Reservation::New(receipt))
    }

    pub(crate) async fn published(&mut self, id: &OccurrenceId) -> Result<(), OccurrenceError> {
        let mut state = self.state.clone().ok_or(OccurrenceError::CorruptState)?;
        let mut found = false;
        for receipt in state
            .clients
            .values_mut()
            .flat_map(|c| c.requests.values_mut())
            .chain(state.unkeyed.iter_mut())
        {
            if &receipt.id == id {
                receipt.status = Dispatch::Published;
                found = true;
            }
        }
        if !found {
            return Err(OccurrenceError::CorruptState);
        }
        self.persist(state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(seq: u64) -> RequestId {
        format!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:{seq}")
            .parse()
            .unwrap()
    }

    #[tokio::test]
    async fn auto_h4_request_receipts_survive_restart_and_uncertain_publication() {
        let root = std::env::temp_dir().join(format!("moli-h4-receipts-{}", random_id().unwrap()));
        let path = root.join("receipts.json");
        let point = PointId::from("simulated:button/press");
        let id = request(1);
        let mut buttons = Buttons::new(Some(path.clone()));
        let Reservation::New(first) = buttons
            .reserve(&point, Some(&id), &id.to_string())
            .await
            .unwrap()
        else {
            panic!("first request is new")
        };
        drop(buttons);
        let mut buttons = Buttons::new(Some(path.clone()));
        let Reservation::Duplicate(replay) = buttons
            .reserve(&point, Some(&id), &id.to_string())
            .await
            .unwrap()
        else {
            panic!("retry must not republish")
        };
        assert_eq!(replay.id, first.id);
        assert_eq!(replay.status, Dispatch::Uncertain);
        buttons.published(&first.id).await.unwrap();
        drop(buttons);
        let mut buttons = Buttons::new(Some(path));
        let Reservation::Duplicate(replay) = buttons
            .reserve(&point, Some(&id), &id.to_string())
            .await
            .unwrap()
        else {
            panic!("published retry is duplicate")
        };
        assert_eq!(replay.status, Dispatch::Published);
        let conflict = buttons
            .reserve(
                &PointId::from("simulated:other/press"),
                Some(&id),
                &id.to_string(),
            )
            .await;
        assert!(matches!(conflict, Err(OccurrenceError::Conflict)));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn auto_h4_request_window_and_capacity_fail_closed() {
        let point = PointId::from("simulated:button/press");
        let mut buttons = Buttons::new(None);
        for seq in [1, 40, 39] {
            let id = request(seq);
            assert!(matches!(
                buttons.reserve(&point, Some(&id), &id.to_string()).await,
                Ok(Reservation::New(_))
            ));
        }
        assert!(matches!(
            buttons
                .reserve(&point, Some(&request(1)), &request(1).to_string())
                .await,
            Err(OccurrenceError::Stale)
        ));
        for client in 1..REQUEST_CLIENTS {
            let id: RequestId = format!("{client:032x}:1").parse().unwrap();
            buttons
                .reserve(&point, Some(&id), &id.to_string())
                .await
                .unwrap();
        }
        let extra: RequestId = "ffffffffffffffffffffffffffffffff:1".parse().unwrap();
        assert!(matches!(
            buttons
                .reserve(&point, Some(&extra), &extra.to_string())
                .await,
            Err(OccurrenceError::Capacity)
        ));
        buttons.state.as_mut().unwrap().next_sequence = MAX_OCCURRENCE_SEQUENCE + 1;
        assert!(matches!(
            buttons
                .reserve(&point, None, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .await,
            Err(OccurrenceError::Sequence)
        ));
    }

    #[tokio::test]
    async fn auto_h4_button_write_cut_fences_unknown_commit_without_reallocation() {
        use crate::state_file::WritePhase;
        let root = std::env::temp_dir().join(format!("moli-h4-cut-{}", random_id().unwrap()));
        let path = root.join("receipts.json");
        let point = PointId::from("simulated:button/press");
        let mut buttons = Buttons::new(Some(path.clone()));
        buttons
            .reserve(&point, Some(&request(1)), &request(1).to_string())
            .await
            .unwrap();
        buttons.file.interrupt_at(Some(WritePhase::BeforeRename));
        assert!(matches!(
            buttons
                .reserve(&point, Some(&request(2)), &request(2).to_string())
                .await,
            Err(OccurrenceError::Storage)
        ));
        buttons.file.interrupt_at(None);
        let Reservation::New(second) = buttons
            .reserve(&point, Some(&request(2)), &request(2).to_string())
            .await
            .unwrap()
        else {
            panic!("pre-rename failure reserved nothing")
        };
        assert_eq!(second.id.sequence, 2);
        buttons.file.interrupt_at(Some(WritePhase::AfterRename));
        assert!(matches!(
            buttons
                .reserve(&point, Some(&request(3)), &request(3).to_string())
                .await,
            Err(OccurrenceError::Uncertain)
        ));
        buttons.file.interrupt_at(None);
        assert!(
            matches!(
                buttons
                    .reserve(&point, Some(&request(3)), &request(3).to_string())
                    .await,
                Err(OccurrenceError::Uncertain)
            ),
            "unknown durability fences the writer"
        );
        drop(buttons);
        let mut buttons = Buttons::new(Some(path));
        let Reservation::Duplicate(third) = buttons
            .reserve(&point, Some(&request(3)), &request(3).to_string())
            .await
            .unwrap()
        else {
            panic!("restart cannot allocate again")
        };
        assert_eq!(third.id.sequence, 3);
        assert_eq!(third.status, Dispatch::Uncertain);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
