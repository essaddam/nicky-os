use crate::{Receipt, service::Inner, state::Record};
use moli_alarm::{
    Authorization, AuthorizationFailure as Failure, Permission, ProfileId, RequestId, Source,
    TimestampMs,
};
use std::sync::atomic::Ordering;

pub(crate) fn policy(
    inner: &Inner,
    id: &str,
    profile: &Record,
    permission: Permission,
    mode: Option<moli_alarm::Mode>,
    current: impl Fn() -> u64,
) -> Authorization {
    let now = current();
    let invalid = |reason| Authorization::Invalid(reason);
    if profile.revoked {
        return invalid(Failure::Revoked);
    }
    let mut from = profile.profile.from;
    let mut until = profile.profile.until;
    if let Some(person) = &profile.profile.person {
        let Some(person) = inner.directory.person(person, now) else {
            return invalid(Failure::Revoked);
        };
        if person.guest {
            from = from.into_iter().chain(person.from).max();
            until = until.into_iter().chain(person.until).min();
            if until.is_none() || !person.reservation_active {
                return invalid(Failure::OutsideValidity);
            }
        }
    }
    // The directory callback may block or cross the validity boundary.
    let now = current().max(inner.clock.now_ms());
    if from.is_some_and(|start| now < start) || until.is_some_and(|end| now >= end) {
        return invalid(Failure::OutsideValidity);
    }
    let permitted = match permission {
        Permission::Disarm => profile.profile.can_disarm,
        Permission::Arm | Permission::ChangeMode | Permission::Exclude | Permission::SkipDelay => {
            profile.profile.can_arm
        }
    };
    if !permitted {
        return invalid(Failure::PermissionDenied);
    }
    if mode.is_some_and(|mode| {
        !profile
            .profile
            .modes
            .contains(&super::validation::mode(mode))
    }) {
        return invalid(Failure::ModeDenied);
    }
    Authorization::Allowed(ProfileId(id.into()))
}

pub(crate) fn receipt(
    receipt: &Receipt,
    request: RequestId,
    source: &Source,
    permission: Permission,
    mode: Option<moli_alarm::Mode>,
    now: TimestampMs,
) -> Authorization {
    let invalid = || Authorization::Invalid(Failure::Unknown);
    if request != receipt.request || source != &receipt.source.source {
        return invalid();
    }
    let Some(inner) = receipt.service.upgrade() else {
        return invalid();
    };
    if inner.ready().is_err() || inner.fenced.load(Ordering::Acquire) {
        return invalid();
    }
    let now =
        now.0
            .max(inner.clock.now_ms())
            .max(receipt.checked.saturating_add(
                u64::try_from(receipt.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            ));
    if let Authorization::Allowed(id) = &receipt.authorization {
        let Some(slot) = receipt.slot.as_ref() else {
            return invalid();
        };
        let Ok(key) = inner.key() else {
            return invalid();
        };
        let Some(key_tag) = receipt.key_tag.as_ref() else {
            return invalid();
        };
        if ring::hmac::verify(
            &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key),
            b"alarm receipt identity",
            key_tag,
        )
        .is_err()
        {
            return invalid();
        }
        let Ok(codes) = inner.credentials() else {
            return invalid();
        };
        if codes.entries.get(slot).is_none_or(|c| c.profile != id.0) {
            return invalid();
        }
        let data = inner.read();
        if data
            .alarm
            .code_locks
            .get(&receipt.source.digest())
            .is_some_and(|a| a.locked(now))
        {
            return Authorization::Invalid(Failure::SourceLocked);
        }
        let Some(profile) = data
            .profiles
            .profiles
            .get(&id.0)
            .filter(|p| &p.slot == slot)
        else {
            return invalid();
        };
        let decision = policy(&inner, &id.0, profile, permission, mode, || {
            now.max(inner.clock.now_ms())
                .max(receipt.checked.saturating_add(
                    u64::try_from(receipt.started.elapsed().as_millis()).unwrap_or(u64::MAX),
                ))
        });
        if inner.ready().is_err() || inner.fenced.load(Ordering::Acquire) {
            invalid()
        } else {
            decision
        }
    } else {
        receipt.authorization.clone()
    }
}
