//! Moli OS runtime.
//!
//! The [`Hub`] owns the live model of the home (devices, current values,
//! availability, labels), broadcasts every change as an [`Event`], routes
//! validated commands to the driver that owns the target device and keeps an
//! auditable journal of every write. Drivers run under a [`supervisor`] that
//! isolates their failures from the core.
//!
//! [`Event`]: moli_core::Event

mod addresses;
mod counters;
mod driver;
pub mod guard;
mod hub;
mod journal_log;
mod keypad;
mod labels;
pub mod media;
mod occurrences;
pub mod orders;
mod private_files;
mod secrets;
mod state_cache;
mod state_file;
mod stats;
pub mod supervisor;
pub mod voice;

pub use addresses::KnownAddress;
pub use counters::CounterError;
pub use driver::{
    BoxFuture, CommandMetadata, CommandRequest, Driver, DriverCtx, MediaPublisher, ScopedSecrets,
    SendPermit,
};
pub use hub::{
    ApprovalError, CORE_NAMESPACE, CommandError, DeviceView, DriverView, Hub, HubOptions,
    LabelError, LabelPatch, MAX_LABEL, Snapshot,
};
pub use keypad::{KeypadCode, KeypadInput, KeypadReceiver};
pub use occurrences::OccurrenceError;
pub use secrets::{MasterKey, forget_secret, set_secret};
pub use state_file::{StateFile, StateFileError};
pub use stats::Stats;
pub use supervisor::spawn_driver;
