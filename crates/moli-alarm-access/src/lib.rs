//! Private credential boundary for the pure alarm engine. No network surface.

mod authorization;
mod crypto;
mod lockouts;
mod management;
mod model;
mod person_removal;
mod receipt;
mod runtime_state;
mod service;
mod state;
mod storage;
mod validation;
mod verification;

pub use model::{Code, Mode, Person, PersonDirectory, ProfileDraft, ProfileView};
pub use service::{
    Audit, AuditEvent, Clock, Error, OwnerPermit, ProfileRevoker, ProfileService, RemovalGuard,
    RequestSource, SECRET_NAMESPACE,
};

pub use receipt::Receipt;
