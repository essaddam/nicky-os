//! Explicit reservations only; no priority or automatic owner selection.
use crate::discovery::Candidate;
use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
#[derive(Debug, Default)]
struct Claims {
    external_ids: BTreeSet<String>,
    external_hosts: BTreeSet<String>,
    held: BTreeMap<String, Ipv4Addr>,
}
#[derive(Clone, Debug, Default)]
pub struct Ownership(Arc<Mutex<Claims>>);
pub(crate) struct Lease {
    pool: Ownership,
    id: String,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.pool
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .held
            .remove(&self.id);
    }
}
fn hostname(value: &str) -> String {
    value.trim_end_matches('.').to_ascii_lowercase()
}
impl Ownership {
    /// Reserve a device already assigned to a TV or Music Assistant.
    /// A conflicting live Cast reservation is never displaced automatically.
    pub fn reserve_external(&self, identity: &str, host: &str) -> bool {
        let mut pool = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pool.held.contains_key(identity)
            || host
                .parse::<Ipv4Addr>()
                .is_ok_and(|ip| pool.held.values().any(|old| *old == ip))
        {
            return false;
        }
        if !identity.is_empty() {
            pool.external_ids.insert(identity.to_ascii_lowercase());
        }
        if !host.is_empty() {
            pool.external_hosts.insert(hostname(host));
        }
        true
    }
    pub(crate) async fn aliases_allowed(&self, candidate: &Candidate) -> bool {
        let hosts = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .external_hosts
            .clone();
        let check = async {
            for host in hosts {
                if host == hostname(&candidate.hostname) || host == candidate.host.to_string() {
                    return false;
                }
                if host.parse::<Ipv4Addr>().is_ok() {
                    continue;
                }
                let Ok(addresses) = tokio::net::lookup_host((host.as_str(), 8009)).await else {
                    return false;
                };
                if addresses
                    .into_iter()
                    .any(|address| address.ip() == candidate.host)
                {
                    return false;
                }
            }
            true
        };
        tokio::time::timeout(std::time::Duration::from_secs(3), check)
            .await
            .unwrap_or(false)
    }
    pub(crate) fn claim(&self, candidate: &Candidate) -> Option<Lease> {
        let mut pool = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = if candidate.id == "receiver" {
            format!("manual/{}", candidate.host)
        } else {
            candidate.id.clone()
        };
        if pool.external_ids.contains(&candidate.id)
            || pool.external_hosts.contains(&hostname(&candidate.hostname))
            || pool.external_hosts.contains(&candidate.host.to_string())
            || pool.held.contains_key(&key)
            || pool.held.values().any(|old| *old == candidate.host)
        {
            return None;
        }
        pool.held.insert(key.clone(), candidate.host);
        Some(Lease {
            pool: self.clone(),
            id: key,
        })
    }
}
