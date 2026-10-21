use crate::{Mode, RequestId, Source, TimestampMs};

/// Public identity only, never a code or a digest of a code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    Arm,
    ChangeMode,
    Disarm,
    Exclude,
    SkipDelay,
}

/// Audit detail only. Every refusal has the same public invalid-code verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationFailure {
    Unknown,
    Revoked,
    OutsideValidity,
    PermissionDenied,
    ModeDenied,
    SourceLocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Authorization {
    Missing,
    Invalid(AuthorizationFailure),
    Allowed(ProfileId),
}

/// Boundary for lot 3/9. The implementer privately correlates a server-created
/// request ID with its input, and checks current rights and validity at `now`.
/// A client-supplied ID is never itself evidence of authorization.
/// Neither codes nor hashes cross this interface or enter alarm state/effects.
pub trait Profiles {
    fn authorize(
        &self,
        request: RequestId,
        source: &Source,
        permission: Permission,
        mode: Option<Mode>,
        now: TimestampMs,
    ) -> Authorization;
}
