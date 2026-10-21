//! Pure alarm decisions. The caller supplies time and executes typed effects.
//! No credential, I/O, clock or asynchronous runtime belongs here.

mod authorization;
pub mod configuration;
mod context;
pub mod coverage;
mod engine;
mod model;
mod restore;

pub use authorization::{Authorization, AuthorizationFailure, Permission, ProfileId, Profiles};
pub use context::{
    ActionPolicy, AfterTrigger, Cause, Notice, RestoreRequest, ReturnFrame, SavedState,
};
pub use engine::step;
pub use model::{
    ArmRequest, Config, DisarmRequest, Effect, Event, Mode, ModeConfig, Phase, RequestId, Source,
    State, TimerToken, TimestampMs, Transition, Verdict,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod exit_tests;

#[cfg(test)]
mod coverage_tests;

#[cfg(test)]
mod contract_tests;
