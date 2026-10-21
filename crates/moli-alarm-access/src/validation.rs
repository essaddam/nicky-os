use crate::{
    Error, Mode, Person, ProfileDraft,
    state::{self, AlarmFile, Credentials, ProfileFile},
};
use std::collections::BTreeSet;

pub(crate) fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
pub(crate) fn identifier(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub(crate) fn draft(profile: &ProfileDraft) -> Result<(), Error> {
    if !text(&profile.name, 80)
        || profile.person.as_ref().is_some_and(|p| !text(p, 64))
        || profile.modes.is_empty()
        || profile.from.zip(profile.until).is_some_and(|(a, b)| a >= b)
    {
        return Err(Error::InvalidProfile);
    }
    Ok(())
}
pub(crate) fn bound(profile: &mut ProfileDraft, person: &Person) -> Result<(), Error> {
    intersect(profile, person);
    if person.guest && profile.until.is_none() {
        return Err(Error::InvalidProfile);
    }
    draft(profile)
}

fn intersect(profile: &mut ProfileDraft, person: &Person) {
    if person.guest {
        profile.from = profile.from.into_iter().chain(person.from).max();
        profile.until = profile.until.into_iter().chain(person.until).min();
    }
}

/// A live stay can close a previously valid window. Represent it as [t, t)
/// for display, without validating a new draft or changing persisted rights.
pub(crate) fn display_window(profile: &mut ProfileDraft, person: &Person, now: u64) {
    intersect(profile, person);
    if person.guest
        && (profile.until.is_none() || profile.from.zip(profile.until).is_some_and(|(a, b)| a >= b))
    {
        let boundary = profile.from.unwrap_or(now);
        profile.from = Some(boundary);
        profile.until = Some(boundary);
    }
}
pub(crate) fn files(profiles: &ProfileFile, alarm: &AlarmFile) -> Result<(), Error> {
    if profiles.version != state::VERSION
        || alarm.version != state::VERSION
        || profiles.profiles.len() > state::MAX_PROFILES
        || alarm.code_locks.len() > state::MAX_SOURCES
    {
        return Err(Error::Corrupt);
    }
    let mut slots = BTreeSet::new();
    for (id, p) in &profiles.profiles {
        if !identifier(id)
            || !identifier(&p.slot)
            || !slots.insert(&p.slot)
            || draft(&p.profile).is_err()
        {
            return Err(Error::Corrupt);
        }
    }
    for (key, a) in &alarm.code_locks {
        if key.len() != 64
            || !key.bytes().all(|b| b.is_ascii_hexdigit())
            || a.failures.len() > 4
            || a.strikes > 8
            || a.failures.iter().any(|t| *t > a.last)
        {
            return Err(Error::Corrupt);
        }
    }
    Ok(())
}
pub(crate) fn credentials(profiles: &ProfileFile, codes: &Credentials) -> Result<(), Error> {
    // A failed pre-publication write can leave an orphan slot. It grants nothing.
    if codes.entries.len() > state::MAX_PROFILES * 2 {
        return Err(Error::Corrupt);
    }
    for (slot, c) in &codes.entries {
        if !identifier(slot)
            || !identifier(&c.profile)
            || crate::crypto::unhex(&c.mac).is_none_or(|mac| mac.len() != 32)
        {
            return Err(Error::Corrupt);
        }
    }
    for (id, p) in &profiles.profiles {
        if codes.entries.get(&p.slot).is_none_or(|c| &c.profile != id) {
            return Err(Error::Corrupt);
        }
    }
    Ok(())
}
pub(crate) fn mode(mode: moli_alarm::Mode) -> Mode {
    match mode {
        moli_alarm::Mode::Away => Mode::Away,
        moli_alarm::Mode::Home => Mode::Home,
        moli_alarm::Mode::Night => Mode::Night,
        moli_alarm::Mode::Vacation => Mode::Vacation,
    }
}
