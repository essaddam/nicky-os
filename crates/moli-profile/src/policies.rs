//! Opt-in read pacing and fixed target identity. No discovery or value restoration.
use super::{Answer, Compiled, DeviceId, DriverCtx, Instant, Value, profile};
use std::{collections::HashMap, time::Duration};

#[derive(Clone, Debug)]
pub(super) struct Pacer {
    next: std::sync::Arc<std::sync::Mutex<Option<Instant>>>,
    gap: Duration,
}
impl Pacer {
    pub fn new(ms: u64) -> Self {
        Self {
            next: std::sync::Arc::default(),
            gap: Duration::from_millis(ms),
        }
    }
    pub fn finished(&self) {
        if self.gap.is_zero() {
            return;
        }
        let after = Instant::now() + self.gap;
        let mut next = self
            .next
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *next = Some(next.map_or(after, |next| next.max(after)));
    }
    pub async fn wait(&self) {
        if self.gap.is_zero() {
            return;
        }
        let at = {
            let mut next = self
                .next
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let now = Instant::now();
            // An initial delay also protects a newly constructed driver after
            // its predecessor stopped. Reservations survive cancelled waits.
            let at = next.map_or(now + self.gap, |next| next.max(now));
            *next = Some(at + self.gap);
            at
        };
        tokio::time::sleep_until(at).await;
    }
}
#[derive(Debug)]
pub(super) struct IdentityMismatch;
impl std::fmt::Display for IdentityMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&moli_i18n::tr!("pilotes.profile.identite_fixe"))
    }
}
impl std::error::Error for IdentityMismatch {}

/// Canonical, bounded identity. MACs accept compact or regular colon/hyphen form.
pub(super) fn canonical(profile: &Compiled, raw: &str) -> Result<String, IdentityMismatch> {
    if profile.meta.mac.is_some() && profile.meta.mac == profile.meta.identity {
        let compact = if raw.len() == 12 {
            raw.to_owned()
        } else if raw.len() == 17
            && matches!(raw.as_bytes()[2], b':' | b'-')
            && raw.bytes().enumerate().all(|(i, b)| {
                if i % 3 == 2 {
                    b == raw.as_bytes()[2]
                } else {
                    b.is_ascii_hexdigit()
                }
            })
        {
            raw.chars().filter(|c| !matches!(c, ':' | '-')).collect()
        } else {
            return Err(IdentityMismatch);
        };
        if compact.bytes().all(|b| b.is_ascii_hexdigit())
            && compact != "000000000000"
            && !compact.eq_ignore_ascii_case("ffffffffffff")
        {
            return Ok(compact.to_ascii_lowercase());
        }
        return Err(IdentityMismatch);
    }
    if raw.is_empty() || raw.len() > 128 || !raw.bytes().all(|b| b.is_ascii_graphic() && b != b'/')
    {
        return Err(IdentityMismatch);
    }
    Ok(raw.to_owned())
}
pub(super) fn admit(ctx: &DriverCtx, native: &str) -> Result<(), IdentityMismatch> {
    let target = ctx.device_id(native);
    if ctx.known_devices().iter().any(|id| *id != target) {
        return Err(IdentityMismatch);
    }
    Ok(())
}
pub(super) fn native(
    profile: &Compiled,
    answers: &HashMap<String, Answer>,
) -> Result<String, IdentityMismatch> {
    let (request, _) = profile
        .meta
        .identity
        .as_deref()
        .and_then(|r| r.split_once('.'))
        .ok_or(IdentityMismatch)?;
    identity(profile, answers.get(request).ok_or(IdentityMismatch)?)
}
fn identity(profile: &Compiled, answer: &Answer) -> Result<String, IdentityMismatch> {
    let (_, path) = profile
        .meta
        .identity
        .as_deref()
        .and_then(|r| r.split_once('.'))
        .ok_or(IdentityMismatch)?;
    let path = path.split('.').map(str::to_owned).collect::<Vec<_>>();
    let raw = answer.identity_text(&path).ok_or(IdentityMismatch)?;
    canonical(profile, &raw)
}
pub(super) fn verify_read(
    profile: &Compiled,
    ctx: &DriverCtx,
    device: &DeviceId,
    request: &str,
    answer: &Answer,
) -> Result<(), IdentityMismatch> {
    if !profile.meta.fixed_target {
        return Ok(());
    }
    let Some((needed, _)) = profile
        .meta
        .identity
        .as_deref()
        .and_then(|r| r.split_once('.'))
    else {
        return Err(IdentityMismatch);
    };
    if request == needed && ctx.device_id(&identity(profile, answer)?) != *device {
        return Err(IdentityMismatch);
    }
    Ok(())
}

pub(super) struct RunGuard<'a> {
    ctx: &'a mut DriverCtx,
    profile: &'a Compiled,
}
impl<'a> RunGuard<'a> {
    pub fn new(ctx: &'a mut DriverCtx, profile: &'a Compiled) -> Self {
        if profile.meta.fixed_target {
            unknown(ctx, profile);
        }
        Self { ctx, profile }
    }
    pub fn context(&mut self) -> &mut DriverCtx {
        self.ctx
    }
}
impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        if self.profile.meta.fixed_target {
            unknown(self.ctx, self.profile);
        }
    }
}
pub(super) fn unknown(ctx: &DriverCtx, profile: &Compiled) {
    for id in ctx.devices() {
        for point in &profile.points {
            ctx.set_state(&id, &point.spec.key, Value::Null);
        }
        ctx.set_availability(&id, false);
    }
}
pub(super) fn clear_cache(ctx: &DriverCtx, profile: &Compiled, device: &DeviceId) {
    for point in &profile.points {
        if point.missing == profile::Missing::Null {
            ctx.set_state(device, &point.spec.key, Value::Null);
        }
    }
}
