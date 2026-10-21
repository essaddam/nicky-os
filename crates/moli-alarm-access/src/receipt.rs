use crate::{ProfileService, RequestSource, service::Inner};
use moli_alarm::{Authorization, Permission, Profiles, RequestId, Source, TimestampMs};
use serde::{Serialize, Serializer};
use std::{
    fmt,
    sync::{Arc, Weak},
    time::Instant,
};

pub struct Receipt {
    pub(crate) service: Weak<Inner>,
    pub(crate) request: RequestId,
    pub(crate) source: RequestSource,
    pub(crate) checked: u64,
    pub(crate) started: Instant,
    pub(crate) authorization: Authorization,
    pub(crate) slot: Option<String>,
    pub(crate) key_tag: Option<Vec<u8>>,
}
impl Receipt {
    pub(crate) fn new(
        service: &ProfileService,
        request: RequestId,
        source: RequestSource,
        checked: u64,
        authorization: Authorization,
        slot: Option<String>,
        key_tag: Option<Vec<u8>>,
    ) -> Self {
        Self {
            service: Arc::downgrade(&service.0),
            request,
            source,
            checked,
            started: Instant::now(),
            authorization,
            slot,
            key_tag,
        }
    }
    pub fn public_status(&self) -> &'static str {
        match self.authorization {
            Authorization::Missing => "missing_code",
            Authorization::Invalid(_) => "invalid_code",
            Authorization::Allowed(_) => "allowed",
        }
    }
}
impl fmt::Debug for Receipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receipt")
            .field("status", &self.public_status())
            .finish()
    }
}
impl Serialize for Receipt {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Receipt", 1)?;
        state.serialize_field("status", self.public_status())?;
        state.end()
    }
}
impl Profiles for Receipt {
    fn authorize(
        &self,
        request: RequestId,
        source: &Source,
        permission: Permission,
        mode: Option<moli_alarm::Mode>,
        now: TimestampMs,
    ) -> Authorization {
        super::authorization::receipt(self, request, source, permission, mode, now)
    }
}
