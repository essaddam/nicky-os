use crate::{AuditEvent, Code, Error, ProfileService, Receipt, RequestSource, crypto, state};
use moli_alarm::{Authorization, AuthorizationFailure, Permission, RequestId, Source};
use std::sync::atomic::Ordering;

type Checked = (Authorization, Option<String>, Option<Vec<u8>>);

impl ProfileService {
    pub async fn verify(
        &self,
        request: RequestId,
        source: RequestSource,
        code: Code<'_>,
        permission: Permission,
        mode: Option<moli_alarm::Mode>,
    ) -> Result<Receipt, Error> {
        let input = super::model::Input::capture(code);
        let service = self.clone();
        tokio::spawn(async move {
            service
                .verify_inner(request, source, input, permission, mode)
                .await
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }
    async fn verify_inner(
        &self,
        request: RequestId,
        source: RequestSource,
        input: super::model::Input,
        permission: Permission,
        mode: Option<moli_alarm::Mode>,
    ) -> Result<Receipt, Error> {
        let _guard = self.0.write.lock().await;
        self.0.ready()?;
        let now = self.0.clock.now_ms();
        let source_key = source.digest();
        if matches!(input, super::model::Input::Missing) {
            return Ok(Receipt::new(
                self,
                request,
                source,
                now,
                Authorization::Missing,
                None,
                None,
            ));
        }
        let mut next = self.0.read().clone();
        let locked = next
            .alarm
            .code_locks
            .get(&source_key)
            .is_some_and(|a| a.locked(now));
        let (authorization, slot, key_tag) = if locked {
            (
                Authorization::Invalid(AuthorizationFailure::SourceLocked),
                None,
                None,
            )
        } else if matches!(source.source, Source::Agent | Source::Automation) {
            (
                Authorization::Invalid(AuthorizationFailure::PermissionDenied),
                None,
                None,
            )
        } else {
            self.match_code(&next, &input, permission, mode, now)?
        };
        let mut lock_started = None;
        if matches!(authorization, Authorization::Invalid(_)) && !locked {
            if !next.alarm.code_locks.contains_key(&source_key)
                && next.alarm.code_locks.len() >= state::MAX_SOURCES
            {
                return Err(Error::Capacity);
            }
            lock_started = next
                .alarm
                .code_locks
                .entry(source_key)
                .or_default()
                .refuse(now.max(self.0.clock.now_ms()));
            self.0.fenced.store(true, Ordering::Release);
            if let Err(e) = self
                .0
                .storage
                .write_async(state::ALARM, next.alarm.clone())
                .await
            {
                return Err(self.0.fail(e));
            }
            self.0.publish(next);
        }
        let (profile, reason) = match &authorization {
            Authorization::Allowed(id) => (Some(id.0.clone()), None),
            Authorization::Invalid(reason) => (None, Some(*reason)),
            Authorization::Missing => (None, None),
        };
        if lock_started.is_some() {
            self.0.hub.notice(moli_core::Notice {
                title: Some(moli_i18n::tr!("alarme.codes.verrouille_titre")),
                message: moli_i18n::tr!("alarme.codes.verrouille", source = source.key),
                from: Some("alarm".into()),
                ts: self.0.clock.now_ms(),
            });
        }
        self.0.audit.record(AuditEvent {
            source: source.key.clone(),
            profile,
            reason,
            lock_started,
        });
        Ok(Receipt::new(
            self,
            request,
            source,
            now,
            authorization,
            slot,
            key_tag,
        ))
    }
    fn match_code(
        &self,
        data: &crate::service::Data,
        input: &super::model::Input,
        permission: Permission,
        mode: Option<moli_alarm::Mode>,
        now: u64,
    ) -> Result<Checked, Error> {
        let key = self.0.key()?;
        let credentials = self.0.credentials()?;
        let code = input.borrow();
        let mut matched = None;
        for (slot, tag) in &credentials.entries {
            if crypto::unhex(&tag.mac).is_some_and(|tag| crypto::matches(&key, &code, &tag)) {
                matched = Some((tag.profile.clone(), slot.clone()));
            }
        }
        let key_tag = Some(
            ring::hmac::sign(
                &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key),
                b"alarm receipt identity",
            )
            .as_ref()
            .to_vec(),
        );
        Ok(match matched {
            Some((id, slot)) => {
                let auth = data
                    .profiles
                    .profiles
                    .get(&id)
                    .filter(|p| p.slot == slot)
                    .map_or(Authorization::Invalid(AuthorizationFailure::Unknown), |p| {
                        super::authorization::policy(&self.0, &id, p, permission, mode, || {
                            now.max(self.0.clock.now_ms())
                        })
                    });
                (auth, Some(slot), key_tag)
            }
            None => (
                Authorization::Invalid(AuthorizationFailure::Unknown),
                None,
                None,
            ),
        })
    }
}
