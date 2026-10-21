//! Moli OS domain model.
//!
//! Pure types, no IO, no async runtime. Everything the rest of the system
//! exchanges — identities, typed values, point specifications, events and
//! journal entries — lives here so drivers, the runtime and the surfaces
//! agree on one vocabulary.

mod access_window;
pub mod color;
mod counter;
mod dependency;
mod event;
mod id;
mod model;
mod occurrence;
pub mod privacy;
mod spec;
mod temporal;
mod timer;
mod value;

pub use access_window::AccessWindow;
pub use counter::{CounterOperation, CounterSpec, MAX_COUNTER_VALUE};
pub use dependency::{
    DefinitionVersion, DependencyKind, DependencyRef, MAX_DEFINITION_VERSION, PublicationVersion,
    VersionError, VersionedDefinition,
};
pub use event::{
    Action, ApprovalRequest, AutomationChange, Decision, DriverStatus, Event, JournalEntry, Notice,
    Origin, Outcome,
};
pub use id::{DeviceId, InstanceId, PointId};
pub use model::{Device, Label, Sample};
pub use occurrence::{
    InvalidRequestId, MAX_OCCURRENCE_SEQUENCE, Occurrence, OccurrenceId, OccurrenceStatus,
    RequestId,
};
pub use spec::{Access, Kind, MAX_TEXT, PointSpec, Semantic, Unit, ValueError, validate};
pub use temporal::{TemporalFormat, TemporalSpec};
pub use timer::{
    DEFAULT_TIMER_SECONDS, MAX_TIMER_SECONDS, TimerField, TimerOperation, TimerRecipient,
    TimerStatus,
};
pub use value::Value;

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
