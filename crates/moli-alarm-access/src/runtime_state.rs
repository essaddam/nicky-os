use crate::{Error, ProfileService, state};
use serde::Serialize;
use std::sync::atomic::Ordering;

impl ProfileService {
    /// Shared writer for lot 4/9: replacing runtime state never loses code lockouts.
    pub async fn write_runtime_state<T: Serialize + Send + 'static>(
        &self,
        runtime: T,
    ) -> Result<(), Error> {
        let value = serde_json::to_value(runtime).map_err(|_| Error::InvalidProfile)?;
        let service = self.clone();
        tokio::spawn(async move {
            let _guard = service.0.write.lock().await;
            service.0.ready()?;
            let mut next = service.0.read().clone();
            next.alarm.runtime = Some(value);
            service.0.fenced.store(true, Ordering::Release);
            if let Err(e) = service
                .0
                .storage
                .write_async(state::ALARM, next.alarm.clone())
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
    pub fn runtime_state<T: serde::de::DeserializeOwned>(&self) -> Result<Option<T>, Error> {
        self.0.ready()?;
        self.0
            .read()
            .alarm
            .runtime
            .clone()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| Error::Corrupt)
    }
}
