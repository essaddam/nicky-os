use crate::{Error, ProfileRevoker, ProfileService, RemovalGuard, state};
use std::sync::{Arc, atomic::Ordering};

impl ProfileService {
    pub fn revoker(&self) -> ProfileRevoker {
        ProfileRevoker(Arc::downgrade(&self.0))
    }
    pub async fn begin_person_removal(&self, person: &str) -> Result<RemovalGuard, Error> {
        let service = self.clone();
        let person = person.to_owned();
        tokio::spawn(async move { service.person_removal_inner(&person).await })
            .await
            .map_err(|_| Error::Unavailable)?
    }
    async fn person_removal_inner(&self, person: &str) -> Result<RemovalGuard, Error> {
        let guard = Arc::clone(&self.0.write).lock_owned().await;
        self.0.ready()?;
        let mut next = self.0.read().clone();
        let mut changed = false;
        for profile in next.profiles.profiles.values_mut() {
            if profile.profile.person.as_deref() == Some(person) && !profile.revoked {
                profile.revoked = true;
                changed = true;
            }
        }
        if changed {
            self.0.fenced.store(true, Ordering::Release);
            if let Err(e) = self
                .0
                .storage
                .write_async(state::PROFILES, next.profiles.clone())
                .await
            {
                return Err(self.0.fail(e));
            }
            self.0.publish(next);
        }
        Ok(RemovalGuard { _guard: guard })
    }
}
