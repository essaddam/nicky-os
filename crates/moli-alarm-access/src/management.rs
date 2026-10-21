use crate::{
    Code, Error, OwnerPermit, ProfileDraft, ProfileService, ProfileView, crypto,
    state::{self, Fingerprint, Record},
};
use crate::{SECRET_NAMESPACE, service::KEY};
use std::sync::atomic::Ordering;

impl ProfileService {
    pub async fn create(
        &self,
        owner: &OwnerPermit,
        draft: ProfileDraft,
        code: Code<'_>,
    ) -> Result<ProfileView, Error> {
        owner.check()?;
        let input = super::model::Input::capture(code);
        let owner = owner.clone();
        let service = self.clone();
        tokio::spawn(async move { service.modify(owner, None, draft, Some(input)).await })
            .await
            .map_err(|_| Error::Unavailable)?
    }
    pub async fn update(
        &self,
        owner: &OwnerPermit,
        id: &str,
        draft: ProfileDraft,
        code: Option<Code<'_>>,
    ) -> Result<ProfileView, Error> {
        owner.check()?;
        let input = code.map(super::model::Input::capture);
        let owner = owner.clone();
        let service = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move { service.modify(owner, Some(id), draft, input).await })
            .await
            .map_err(|_| Error::Unavailable)?
    }
    async fn modify(
        &self,
        owner: OwnerPermit,
        id: Option<String>,
        mut draft: ProfileDraft,
        input: Option<super::model::Input>,
    ) -> Result<ProfileView, Error> {
        let _guard = self.0.write.lock().await;
        owner.check()?;
        self.0.ready()?;
        super::validation::draft(&draft)?;
        let now = self.0.clock.now_ms();
        if let Some(person) = &draft.person {
            let person = self
                .0
                .directory
                .person(person, now)
                .ok_or(Error::InvalidProfile)?;
            super::validation::bound(&mut draft, &person)?;
        }
        let mut next = self.0.read().clone();
        if id.is_none() && next.profiles.profiles.len() >= state::MAX_PROFILES {
            return Err(Error::Capacity);
        }
        let id = match id {
            Some(id) => {
                let old = next.profiles.profiles.get(&id).ok_or(Error::NotFound)?;
                if old.revoked {
                    return Err(Error::InvalidProfile);
                }
                id
            }
            None => crypto::random_id()?,
        };
        let mut credentials = self.0.credentials()?;
        let slot = if let Some(input) = input {
            self.prepare_code(
                &owner,
                &id,
                &input,
                next.profiles.profiles.is_empty(),
                &mut credentials,
            )
            .await?
        } else {
            next.profiles.profiles[&id].slot.clone()
        };
        next.profiles.profiles.insert(
            id.clone(),
            Record {
                profile: draft,
                revoked: false,
                slot,
            },
        );
        owner.check()?;
        self.0.fenced.store(true, Ordering::Release);
        if let Err(e) = self
            .0
            .storage
            .write_async(state::ALARM, next.alarm.clone())
            .await
        {
            return Err(self.0.fail(e));
        }
        let raw =
            serde_json::to_string(&credentials).map_err(|_| self.0.fail(Error::Unavailable))?;
        if self
            .0
            .hub
            .store_secret(&SECRET_NAMESPACE.into(), state::CODES, &raw)
            .await
            .is_err()
        {
            return Err(self.0.fail(Error::Unavailable));
        }
        owner.check().map_err(|e| self.0.fail(e))?;
        if let Err(e) = self
            .0
            .storage
            .write_async(state::PROFILES, next.profiles.clone())
            .await
        {
            return Err(self.0.fail(e));
        }
        let view = self.view(&id, &next.profiles.profiles[&id], now)?;
        self.0.publish(next);
        Ok(view)
    }
    pub async fn revoke(&self, owner: &OwnerPermit, id: &str) -> Result<(), Error> {
        owner.check()?;
        let service = self.clone();
        let owner = owner.clone();
        let id = id.to_owned();
        tokio::spawn(async move {
            let _guard = service.0.write.lock().await;
            owner.check()?;
            service.0.ready()?;
            let mut next = service.0.read().clone();
            next.profiles
                .profiles
                .get_mut(&id)
                .ok_or(Error::NotFound)?
                .revoked = true;
            service.0.fenced.store(true, Ordering::Release);
            if let Err(e) = service
                .0
                .storage
                .write_async(state::PROFILES, next.profiles.clone())
                .await
            {
                return Err(service.0.fail(e));
            }
            service.0.publish(next);
            Ok(())
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }
}

impl ProfileService {
    async fn prepare_code(
        &self,
        owner: &OwnerPermit,
        id: &str,
        input: &super::model::Input,
        profiles_empty: bool,
        credentials: &mut crate::state::Credentials,
    ) -> Result<String, Error> {
        let code = input.borrow();
        if !code.valid() {
            return Err(Error::InvalidProfile);
        }
        let key = match self.0.key() {
            Ok(key) => key,
            Err(Error::MissingKey)
                if profiles_empty
                    && credentials.entries.is_empty()
                    && self.0.hub.secret(&SECRET_NAMESPACE.into(), KEY).is_none() =>
            {
                owner.check()?;
                let key = crypto::random_key()?;
                self.0.fenced.store(true, Ordering::Release);
                if self
                    .0
                    .hub
                    .store_secret(&SECRET_NAMESPACE.into(), KEY, &crypto::hex(&key))
                    .await
                    .is_err()
                {
                    return Err(self.0.fail(Error::Unavailable));
                }
                key
            }
            Err(e) => return Err(e),
        };
        let duplicate = credentials.entries.values().any(|tag| {
            tag.profile != id
                && crypto::unhex(&tag.mac).is_some_and(|tag| crypto::matches(&key, &code, &tag))
        });
        if duplicate {
            return Err(Error::DuplicateCode);
        }
        let slot = crypto::random_id()?;
        let tag = crypto::sign(&key, &code).ok_or(Error::InvalidProfile)?;
        credentials.entries.retain(|_, entry| entry.profile != id);
        credentials.entries.insert(
            slot.clone(),
            Fingerprint {
                profile: id.to_owned(),
                mac: crypto::hex(&tag),
            },
        );
        Ok(slot)
    }
}
