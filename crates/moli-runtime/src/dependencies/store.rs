use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use jiff::Timestamp;
use moli_core::{DependencyKind, DependencyRef, PublicationVersion, VersionedDefinition};
use serde::{Deserialize, Serialize};

use super::{Calendar, DependencyDefinition, DependencyError as Error, DependencyView};
use crate::StateFile;

const MAX_DEFINITIONS: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    view: DependencyView,
    projected: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema: u8,
    records: Vec<Record>,
}

#[derive(Debug)]
pub(crate) struct Dependencies {
    pub(crate) file: StateFile,
    state: Option<State>,
    registered: BTreeSet<DependencyRef>,
    calendars: BTreeMap<DependencyRef, Calendar>,
    next: BTreeMap<DependencyRef, Timestamp>,
    fenced: Option<Error>,
}

fn valid_reference(reference: &DependencyRef) -> bool {
    let id = reference.id.as_str();
    id.len() <= 240
        && id
            .split_once(':')
            .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains(':'))
        && !id.contains('/')
        && !id.chars().any(char::is_control)
}

impl Dependencies {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            file: StateFile::new(path, 4 * 1024 * 1024),
            state: None,
            registered: BTreeSet::new(),
            calendars: BTreeMap::new(),
            next: BTreeMap::new(),
            fenced: None,
        }
    }
    async fn ready(&mut self) -> Result<(), Error> {
        if let Some(error) = self.fenced {
            return Err(error);
        }
        if self.state.is_none() {
            let state = match self.file.load::<State>().await {
                Ok(state) => state.unwrap_or(State {
                    schema: 1,
                    records: Vec::new(),
                }),
                Err(error) => {
                    tracing::warn!(%error, "dependency state could not be loaded");
                    self.fenced = Some(Error::Corrupt);
                    return Err(Error::Corrupt);
                }
            };
            let mut seen = BTreeSet::new();
            if state.schema != 1
                || state.records.len() > MAX_DEFINITIONS
                || !state.records.iter().all(|record| {
                    let reference = &record.view.reference;
                    let versions = &record.view.versions;
                    valid_reference(reference)
                        && reference.kind == versions.draft().definition.kind()
                        && seen.insert(reference.clone())
                        && versions.is_valid()
                        && [Some(versions.draft()), versions.current()]
                            .into_iter()
                            .flatten()
                            .all(|v| {
                                v.definition.kind() == reference.kind
                                    && v.definition
                                        .clone()
                                        .normalized()
                                        .is_ok_and(|d| d == v.definition)
                            })
                        && (versions.current().is_some() && reference.kind == DependencyKind::Scene)
                            == record.view.scope.is_some()
                        && record.view.scope.as_ref().is_none_or(|scope| {
                            match versions.current().map(|v| &v.definition) {
                                Some(DependencyDefinition::Scene(spec)) => scope.valid_for(spec),
                                _ => false,
                            }
                        })
                        && (versions.current().is_some()
                            && reference.kind == DependencyKind::Schedule)
                            == record.projected.is_some()
                })
            {
                self.fenced = Some(Error::Corrupt);
                return Err(Error::Corrupt);
            }
            if moli_core::calculation_order(&state_calculations(&state)).is_err() {
                self.fenced = Some(Error::Corrupt);
                return Err(Error::Corrupt);
            }
            self.state = Some(state);
        }
        Ok(())
    }
    async fn persist(&mut self, state: State) -> Result<(), Error> {
        match self.file.write(&state).await {
            Ok(()) => {
                self.state = Some(state);
                Ok(())
            }
            Err(error) => {
                tracing::warn!(%error, commit_unknown=error.commit_unknown, "dependency state not acknowledged");
                if error.commit_unknown {
                    self.fenced = Some(Error::Uncertain);
                    Err(Error::Uncertain)
                } else {
                    Err(Error::Storage)
                }
            }
        }
    }
    fn record(&self, reference: &DependencyRef) -> Result<&Record, Error> {
        if let Some(error) = self.fenced {
            return Err(error);
        }
        if !self.registered.contains(reference) {
            return Err(Error::Unknown);
        }
        self.state
            .as_ref()
            .and_then(|s| s.records.iter().find(|r| &r.view.reference == reference))
            .ok_or(Error::Unknown)
    }
    pub(crate) fn view(&self, reference: &DependencyRef) -> Result<DependencyView, Error> {
        Ok(self.record(reference)?.view.clone())
    }
    pub(crate) fn list(&self) -> Result<Vec<DependencyView>, Error> {
        if let Some(error) = self.fenced {
            return Err(error);
        }
        self.registered.iter().map(|r| self.view(r)).collect()
    }
    fn refresh_calendar(&mut self, reference: &DependencyRef, now: Timestamp) -> Result<(), Error> {
        self.calendars.remove(reference);
        self.next.remove(reference);
        if let Some(current) = self.record(reference)?.view.versions.current()
            && let Some(calendar) = current.definition.calendar()?
        {
            if let Some(next) = calendar.next(now) {
                self.next.insert(reference.clone(), next);
            }
            self.calendars.insert(reference.clone(), calendar);
        }
        Ok(())
    }
    pub(crate) async fn initialize(
        &mut self,
        reference: DependencyRef,
        definition: DependencyDefinition,
        now: Timestamp,
    ) -> Result<DependencyView, Error> {
        if !valid_reference(&reference) || definition.kind() != reference.kind {
            return Err(Error::Definition);
        }
        let definition = definition.normalized()?;
        self.ready().await?;
        if !self
            .state
            .as_ref()
            .unwrap()
            .records
            .iter()
            .any(|r| r.view.reference == reference)
        {
            let mut state = self.state.clone().unwrap();
            if state.records.len() >= MAX_DEFINITIONS {
                return Err(Error::Capacity);
            }
            state.records.push(Record {
                view: DependencyView {
                    reference: reference.clone(),
                    versions: VersionedDefinition::new(definition),
                    scope: None,
                },
                projected: None,
            });
            self.persist(state).await?;
        }
        self.registered.insert(reference.clone());
        self.refresh_calendar(&reference, now)?;
        // Startup reads the current civil state; missed transitions are not occurrences.
        let projected = self.calendars.get(&reference).map(|c| c.active(now));
        let mut state = self.state.clone().unwrap();
        let record = state
            .records
            .iter_mut()
            .find(|r| r.view.reference == reference)
            .unwrap();
        if record.projected != projected {
            record.projected = projected;
            self.persist(state).await?;
        }
        self.view(&reference)
    }
    pub(crate) async fn stage(
        &mut self,
        reference: &DependencyRef,
        expected: u64,
        definition: DependencyDefinition,
    ) -> Result<DependencyView, Error> {
        if definition.kind() != reference.kind {
            return Err(Error::Definition);
        }
        let definition = definition.normalized()?;
        if reference.kind != definition.kind() {
            return Err(Error::Definition);
        }
        self.record(reference)?;
        let mut state = self.state.clone().unwrap();
        let record = state
            .records
            .iter_mut()
            .find(|r| &r.view.reference == reference)
            .unwrap();
        record
            .view
            .versions
            .stage(expected, definition)
            .map_err(|_| Error::Conflict)?;
        self.persist(state).await?;
        self.view(reference)
    }
    pub(crate) async fn publish(
        &mut self,
        reference: &DependencyRef,
        expected: PublicationVersion,
        now: Timestamp,
        scope: Option<crate::SceneScope>,
    ) -> Result<DependencyView, Error> {
        self.record(reference)?;
        let mut state = self.state.clone().unwrap();
        let record = state
            .records
            .iter_mut()
            .find(|r| &r.view.reference == reference)
            .unwrap();
        let changed = record
            .view
            .versions
            .publish(expected)
            .map_err(|_| Error::Conflict)?;
        if !changed && record.view.scope != scope {
            return Err(Error::Conflict);
        }
        if changed {
            match (&record.view.versions.current().unwrap().definition, &scope) {
                (DependencyDefinition::Scene(spec), Some(scope)) if scope.valid_for(spec) => {}
                (
                    DependencyDefinition::Schedule(_)
                    | DependencyDefinition::Aggregate(_)
                    | DependencyDefinition::Duration(_),
                    None,
                ) => {}
                _ => return Err(Error::Definition),
            }
            record.view.scope = scope;
            record.projected = record
                .view
                .versions
                .current()
                .unwrap()
                .definition
                .calendar()?
                .map(|calendar| calendar.active(now));
            moli_core::calculation_order(&state_calculations(&state))
                .map_err(super::graph_error)?;
            self.persist(state).await?;
            self.refresh_calendar(reference, now)?;
        }
        self.view(reference)
    }
    /// A scheduled transition cannot expose an old boolean as the current civil
    /// truth after its persistence failed. Restart reloads the durable head.
    pub(crate) fn suspend_projection(&mut self) {
        self.fenced = Some(Error::Projection);
    }
    pub(crate) fn calculations(
        &self,
    ) -> Result<BTreeMap<moli_core::DeviceId, moli_core::CalculationSpec>, Error> {
        if let Some(error) = self.fenced {
            return Err(error);
        }
        Ok(self
            .state
            .as_ref()
            .map(state_calculations)
            .unwrap_or_default())
    }
    pub(crate) fn durations(
        &self,
    ) -> Result<BTreeMap<moli_core::DeviceId, (u64, moli_core::DurationSpec)>, Error> {
        if let Some(error) = self.fenced {
            return Err(error);
        }
        Ok(self
            .state
            .as_ref()
            .into_iter()
            .flat_map(|s| &s.records)
            .filter_map(|r| {
                if !self.registered.contains(&r.view.reference) {
                    return None;
                }
                let current = r.view.versions.current()?;
                if let DependencyDefinition::Duration(spec) = &current.definition {
                    Some((r.view.reference.id.clone(), (current.version, spec.clone())))
                } else {
                    None
                }
            })
            .collect())
    }
    pub(crate) fn aggregates(
        &self,
        registered_only: bool,
    ) -> Result<BTreeMap<moli_core::DeviceId, moli_core::AggregateSpec>, Error> {
        if let Some(error) = self.fenced {
            return Err(error);
        }
        let mut result = BTreeMap::new();
        for record in self.state.as_ref().into_iter().flat_map(|s| &s.records) {
            if registered_only && !self.registered.contains(&record.view.reference) {
                continue;
            }
            if let Some(current) = record.view.versions.current()
                && let DependencyDefinition::Aggregate(spec) = &current.definition
            {
                result.insert(record.view.reference.id.clone(), spec.clone());
            }
        }
        Ok(result)
    }
    pub(crate) fn projection(&self, reference: &DependencyRef) -> Result<Option<bool>, Error> {
        Ok(self.record(reference)?.projected)
    }
    pub(crate) fn next(&self) -> Option<Timestamp> {
        if self.fenced.is_some() {
            None
        } else {
            self.next.values().copied().min()
        }
    }
    pub(crate) fn due(&self, now: Timestamp, clock_jump: bool) -> Vec<DependencyRef> {
        self.calendars
            .keys()
            .filter(|r| clock_jump || self.next.get(*r).is_some_and(|next| *next <= now))
            .cloned()
            .collect()
    }
    pub(crate) async fn tick(
        &mut self,
        now: Timestamp,
        clock_jump: bool,
    ) -> Result<Vec<(DependencyRef, bool)>, Error> {
        if let Some(error) = self.fenced {
            return Err(error);
        }
        let due = self.due(now, clock_jump);
        if due.is_empty() {
            return Ok(Vec::new());
        }
        let mut state = self.state.clone().unwrap();
        let mut changes = Vec::new();
        for reference in &due {
            let active = self.calendars[reference].active(now);
            let record = state
                .records
                .iter_mut()
                .find(|r| &r.view.reference == reference)
                .unwrap();
            if record.projected != Some(active) {
                record.projected = Some(active);
                changes.push((reference.clone(), active));
            }
        }
        if !changes.is_empty() {
            self.persist(state).await?;
        }
        for reference in &due {
            self.next.remove(reference);
            if let Some(next) = self.calendars[reference].next(now) {
                self.next.insert(reference.clone(), next);
            }
        }
        Ok(changes)
    }
}

fn state_calculations(state: &State) -> BTreeMap<moli_core::DeviceId, moli_core::CalculationSpec> {
    state
        .records
        .iter()
        .filter_map(|record| {
            let current = record.view.versions.current()?;
            match &current.definition {
                DependencyDefinition::Aggregate(spec) => Some((
                    record.view.reference.id.clone(),
                    moli_core::CalculationSpec::Aggregate(spec.clone()),
                )),
                DependencyDefinition::Duration(spec) => Some((
                    record.view.reference.id.clone(),
                    moli_core::CalculationSpec::Duration(spec.clone()),
                )),
                DependencyDefinition::Schedule(_) | DependencyDefinition::Scene(_) => None,
            }
        })
        .collect()
}
