//! Alarm-specific owner capability. HTTP routes remain in lot 7/9.
use crate::Household;
use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use moli_alarm_access::{Audit, Error, OwnerPermit, ProfileService};
use moli_runtime::Hub;
use std::sync::Arc;

#[derive(Debug)]
pub struct AlarmOwner(OwnerPermit);
impl AlarmOwner {
    pub fn permit(&self) -> &OwnerPermit {
        &self.0
    }
}
impl FromRequestParts<Hub> for AlarmOwner {
    type Rejection = Response;
    fn from_request_parts(
        parts: &mut Parts,
        _state: &Hub,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Self::extract(parts).ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                axum::Json(
                    serde_json::json!({"error":moli_i18n::tr!("alarme.codes.proprietaire_humain")}),
                ),
            )
                .into_response()
        }))
    }
}
impl AlarmOwner {
    fn extract(parts: &Parts) -> Option<Self> {
        let caller = parts.extensions.get::<crate::caller::Caller>()?;
        let persons = parts.extensions.get::<Arc<crate::people::People>>()?;
        let humans = parts
            .extensions
            .get::<Arc<crate::session::Sessions>>()?
            .clone();
        let id = caller
            .person
            .as_ref()
            .filter(|p| p.is_owner())
            .map(|p| p.id.clone())?;
        let key = caller.key.clone();
        let mut headers = axum::http::HeaderMap::new();
        if let Some(cookie) = parts.headers.get(axum::http::header::COOKIE) {
            headers.insert(axum::http::header::COOKIE, cookie.clone());
        }
        let people = Arc::downgrade(persons);
        let permit = OwnerPermit::new(move || {
            humans.is_human(&headers, &key)
                && people
                    .upgrade()
                    .and_then(|p| p.get(&id))
                    .is_some_and(|p| p.is_owner())
        });
        if !permit.check_for_surface() {
            return None;
        }
        Some(Self(permit))
    }
}
impl moli_alarm_access::PersonDirectory for Household {
    fn person(&self, id: &str, now: u64) -> Option<moli_alarm_access::Person> {
        self.0.alarm_person(id, now)
    }
}
impl Household {
    /// Open only when alarm is configured. No HTTP management surface is added.
    pub async fn alarm_profiles(&self, audit: Arc<dyn Audit>) -> Result<ProfileService, Error> {
        let mut registration = self.0.alarm_profiles.lock().await;
        if let Some(service) = registration
            .as_ref()
            .and_then(moli_alarm_access::ProfileRevoker::upgrade)
        {
            return Ok(service);
        }
        let (hub, path) = self.0.alarm_setup().ok_or(Error::Unavailable)?;
        let service = ProfileService::open(hub, Arc::new(self.clone()), &path, audit).await?;
        *registration = Some(service.revoker());
        Ok(service)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        caller::{Caller, Via},
        people::{Draft, Role},
        session::Sessions,
    };
    use axum::http::{HeaderMap, header::COOKIE};
    use moli_alarm::{Permission, RequestId};
    use moli_alarm_access::{Code, ProfileDraft};
    use moli_runtime::{HubOptions, MasterKey};
    use serde_json::json;
    #[derive(Debug)]
    struct AuditSink;
    impl Audit for AuditSink {
        fn record(&self, _: moli_alarm_access::AuditEvent) {}
    }
    fn setup() -> (tempfile::TempDir, Hub, Household, Arc<Sessions>) {
        let dir = tempfile::tempdir().unwrap();
        let hub = Hub::new(HubOptions {
            secrets: Some((dir.path().join("vault.enc"), MasterKey::generate().unwrap())),
            ..Default::default()
        })
        .unwrap();
        let house = Household::open(&hub, Some(dir.path()), &[]);
        let humans = Arc::new(Sessions::new(
            hub.clone(),
            Some(crate::UiPin::new("482915")),
            &[],
        ));
        (dir, hub, house, humans)
    }
    async fn actor(house: &Household, role: Role) -> Caller {
        let person = house
            .0
            .create(
                serde_json::from_value::<Draft>(
                    json!({"name":format!("Synthetic {role:?}"),"role":role}),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        Caller {
            via: Via::Loopback,
            key: format!("synthetic-{role:?}"),
            person: house.0.get(&person.id),
        }
    }
    async fn owner(
        hub: &Hub,
        house: &Household,
        humans: &Arc<Sessions>,
        caller: Caller,
        login: bool,
    ) -> Result<AlarmOwner, StatusCode> {
        let mut headers = HeaderMap::new();
        if login {
            let token = humans.login("482915", &caller).await.unwrap();
            headers.insert(
                COOKIE,
                Sessions::cookie(&token)
                    .split(';')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap(),
            );
        }
        let (mut parts, ()) = axum::http::Request::builder()
            .body(())
            .unwrap()
            .into_parts();
        parts.headers = headers;
        parts.extensions.insert(caller);
        parts.extensions.insert(house.0.clone());
        parts.extensions.insert(humans.clone());
        AlarmOwner::from_request_parts(&mut parts, hub)
            .await
            .map_err(|response| response.status())
    }
    fn draft(person: Option<String>) -> ProfileDraft {
        ProfileDraft {
            person,
            ..serde_json::from_value(
                json!({"name":"Synthetic alarm profile","can_arm":true,"can_disarm":true}),
            )
            .unwrap()
        }
    }

    #[tokio::test]
    async fn alarm_management_requires_both_owner_identity_and_a_live_pin_session() {
        let (_dir, hub, house, humans) = setup();
        let a = actor(&house, Role::Owner).await;
        assert!(
            owner(&hub, &house, &humans, a.clone(), false)
                .await
                .is_err()
        );
        let mut anonymous = a.clone();
        anonymous.person = None;
        assert!(owner(&hub, &house, &humans, anonymous, true).await.is_err());
        let grant = owner(&hub, &house, &humans, a.clone(), true).await.unwrap();
        let profiles = house.alarm_profiles(Arc::new(AuditSink)).await.unwrap();
        profiles
            .create(grant.permit(), draft(None), Code::provided("70710937"))
            .await
            .unwrap();
        for role in [Role::Member, Role::Guest] {
            let caller = actor(&house, role).await;
            assert!(owner(&hub, &house, &humans, caller, true).await.is_err());
        }
        house
            .0
            .create(
                serde_json::from_value::<Draft>(json!({"name":"Synthetic keeper","role":"owner"}))
                    .unwrap(),
            )
            .await
            .unwrap();
        house
            .0
            .update(
                &a.person.as_ref().unwrap().id,
                serde_json::from_value(json!({"name":"Synthetic demoted","role":"member"}))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            profiles
                .create(grant.permit(), draft(None), Code::provided("70710938"))
                .await
                .err(),
            Some(Error::Forbidden)
        );
    }

    #[tokio::test]
    async fn removing_and_recreating_a_person_never_reactivates_their_alarm_profile() {
        let (_dir, hub, house, humans) = setup();
        let a = actor(&house, Role::Owner).await;
        let grant = owner(&hub, &house, &humans, a, true).await.unwrap();
        let visitor=house.0.create(serde_json::from_value::<Draft>(json!({"name":"Synthetic visitor","role":"guest","expires":moli_core::now_ms()+120_000})).unwrap()).await.unwrap();
        let profiles = house.alarm_profiles(Arc::new(AuditSink)).await.unwrap();
        profiles
            .create(
                grant.permit(),
                draft(Some(visitor.id.clone())),
                Code::provided("70710937"),
            )
            .await
            .unwrap();
        house.0.remove(&visitor.id).await.unwrap();
        let recreated = house
            .0
            .create(
                serde_json::from_value::<Draft>(
                    json!({"name":"Synthetic visitor","role":"member"}),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(recreated.id, visitor.id);
        assert_eq!(
            profiles
                .verify(
                    RequestId(1),
                    moli_alarm_access::RequestSource::dashboard("example", true).unwrap(),
                    Code::provided("70710937"),
                    Permission::Disarm,
                    None
                )
                .await
                .unwrap()
                .public_status(),
            "invalid_code"
        );
        assert!(profiles.profiles(grant.permit()).unwrap()[0].revoked);
    }

    #[tokio::test]
    async fn person_removal_revokes_persisted_profiles_even_when_the_alarm_service_is_dormant() {
        let (_dir, hub, house, humans) = setup();
        let a = actor(&house, Role::Owner).await;
        let grant = owner(&hub, &house, &humans, a, true).await.unwrap();
        let person = house
            .0
            .create(
                serde_json::from_value::<Draft>(
                    json!({"name":"Synthetic visitor","role":"member"}),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let profiles = house.alarm_profiles(Arc::new(AuditSink)).await.unwrap();
        profiles
            .create(
                grant.permit(),
                draft(Some(person.id.clone())),
                Code::provided("70710937"),
            )
            .await
            .unwrap();
        drop(profiles);
        house.0.remove(&person.id).await.unwrap();
        house
            .0
            .create(
                serde_json::from_value::<Draft>(
                    json!({"name":"Synthetic visitor","role":"member"}),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let profiles = house.alarm_profiles(Arc::new(AuditSink)).await.unwrap();
        assert!(profiles.profiles(grant.permit()).unwrap()[0].revoked);
    }
    #[tokio::test]
    async fn a_failed_last_owner_removal_keeps_their_alarm_profile_usable() {
        let (_dir, hub, house, humans) = setup();
        let a = actor(&house, Role::Owner).await;
        let id = a.person.as_ref().unwrap().id.clone();
        let grant = owner(&hub, &house, &humans, a, true).await.unwrap();
        let profiles = house.alarm_profiles(Arc::new(AuditSink)).await.unwrap();
        profiles
            .create(
                grant.permit(),
                draft(Some(id.clone())),
                Code::provided("70710937"),
            )
            .await
            .unwrap();
        assert!(matches!(
            house.0.remove(&id).await,
            Err(crate::people::Refused::LastOwner)
        ));
        assert!(!profiles.profiles(grant.permit()).unwrap()[0].revoked);
        assert_eq!(
            profiles
                .verify(
                    RequestId(1),
                    moli_alarm_access::RequestSource::dashboard("example", true).unwrap(),
                    Code::provided("70710937"),
                    Permission::Disarm,
                    None
                )
                .await
                .unwrap()
                .public_status(),
            "allowed"
        );
    }

    #[tokio::test]
    async fn changing_the_pin_invalidates_an_already_extracted_alarm_management_permit() {
        let (_dir, hub, house, humans) = setup();
        let a = actor(&house, Role::Owner).await;
        let grant = owner(&hub, &house, &humans, a, true).await.unwrap();
        let profiles = house.alarm_profiles(Arc::new(AuditSink)).await.unwrap();
        humans.set_pin("821745", "synthetic owner").await.unwrap();
        assert_eq!(
            profiles
                .create(grant.permit(), draft(None), Code::provided("70710937"))
                .await
                .err(),
            Some(Error::Forbidden)
        );
        assert!(hub.secret(&"alarm".into(), "code_key").is_none());
    }
}
