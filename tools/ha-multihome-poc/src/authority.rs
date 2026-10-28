use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::config::{Topology, valid_id};

pub const LEASE_MS: u64 = 3_000;
const MAX_RECEIPTS_PER_HOUSE: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Lease {
    pub house: String,
    pub owner: String,
    pub epoch: u64,
    pub expires_ms: u64,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Local,
    Cloud,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    pub house: String,
    pub node: String,
    pub epoch: u64,
    pub id: String,
    pub scope: Scope,
}

/// Central test authority; this is neither a distributed witness nor an executor.
#[derive(Clone, Debug)]
pub struct Authority {
    topology: Topology,
    leases: BTreeMap<String, Lease>,
    receipts: BTreeMap<String, HashSet<String>>,
    last_now_ms: u64,
}

impl Authority {
    pub fn new(topology: Topology, now_ms: u64) -> Result<Self> {
        topology.validate()?;
        let expires_ms = deadline(now_ms)?;
        let leases = topology
            .houses
            .iter()
            .map(|house| {
                (
                    house.id.clone(),
                    Lease {
                        house: house.id.clone(),
                        owner: house.primary.clone(),
                        epoch: 1,
                        expires_ms,
                        sequence: 0,
                    },
                )
            })
            .collect();
        Ok(Self {
            topology,
            leases,
            receipts: BTreeMap::new(),
            last_now_ms: now_ms,
        })
    }

    pub fn lease(&self, house: &str) -> Option<&Lease> {
        self.leases.get(house)
    }

    pub fn leases(&self) -> Vec<Lease> {
        self.leases.values().cloned().collect()
    }

    pub fn renew(&mut self, house: &str, node: &str, epoch: u64, now_ms: u64) -> Result<Lease> {
        self.observe_time(now_ms)?;
        self.active_lease(house, node, epoch, now_ms)?;
        let expires_ms = deadline(now_ms)?;
        let lease = self.leases.get_mut(house).context("unknown house")?;
        lease.expires_ms = expires_ms;
        Ok(lease.clone())
    }

    pub fn committed(
        &mut self,
        house: &str,
        node: &str,
        epoch: u64,
        now_ms: u64,
        sequence: u64,
    ) -> Result<()> {
        self.observe_time(now_ms)?;
        let current = self.active_lease(house, node, epoch, now_ms)?;
        let expected = current
            .sequence
            .checked_add(1)
            .context("sequence exhausted")?;
        ensure!(
            sequence == expected,
            "commit must advance exactly one sequence"
        );
        self.leases
            .get_mut(house)
            .context("unknown house")?
            .sequence = sequence;
        Ok(())
    }

    pub fn promote(
        &mut self,
        house: &str,
        node: &str,
        now_ms: u64,
        manual: bool,
        previous_fenced: bool,
        replica_sequence: u64,
    ) -> Result<Lease> {
        self.observe_time(now_ms)?;
        ensure!(
            self.topology.enabled
                || (manual
                    && previous_fenced
                    && self.lease(house).is_some_and(|lease| lease.owner == node)),
            "replication is disabled"
        );
        let config = self.topology.house(house).context("unknown house")?;
        ensure!(
            self.topology
                .holders(house)
                .iter()
                .any(|holder| holder == node),
            "candidate is not a house holder"
        );
        let candidate = self.topology.node(node).context("unknown candidate")?;
        ensure!(candidate.enabled, "candidate is disabled");
        let current = self.lease(house).context("unknown house")?;
        ensure!(
            current.owner != node || (manual && previous_fenced),
            "owner reincarnation requires manual promotion and confirmed fencing"
        );
        ensure!(
            replica_sequence == current.sequence,
            "candidate replica is not current"
        );
        ensure!(
            previous_fenced || now_ms >= current.expires_ms,
            "previous lease is still active"
        );
        ensure!(
            self.topology.witness || previous_fenced,
            "without a witness, the previous process must be fenced"
        );
        if !manual {
            ensure!(
                self.topology.witness,
                "automatic promotion requires a witness"
            );
            ensure!(
                self.topology.local(node, house),
                "remote promotion must be manual"
            );
            ensure!(
                config
                    .standbys
                    .iter()
                    .any(|standby| standby.node == node && standby.automatic),
                "candidate is not an automatic standby"
            );
        }
        let lease = Lease {
            house: house.into(),
            owner: node.into(),
            epoch: current.epoch.checked_add(1).context("epoch exhausted")?,
            expires_ms: deadline(now_ms)?,
            sequence: current.sequence,
        };
        self.leases.insert(house.into(), lease.clone());
        Ok(lease)
    }

    /// Accept a simulated effect once per stable (house, ID), across promotions.
    pub fn accept_effect(&mut self, effect: &Effect, now_ms: u64) -> Result<bool> {
        self.observe_time(now_ms)?;
        ensure!(valid_id(&effect.id, 64), "invalid effect ID");
        self.active_lease(&effect.house, &effect.node, effect.epoch, now_ms)?;
        ensure!(
            effect.scope != Scope::Local || self.topology.local(&effect.node, &effect.house),
            "remote owner cannot produce local effects"
        );
        let receipts = self.receipts.entry(effect.house.clone()).or_default();
        if receipts.contains(&effect.id) {
            return Ok(false);
        }
        ensure!(
            receipts.len() < MAX_RECEIPTS_PER_HOUSE,
            "house receipt capacity exhausted"
        );
        receipts.insert(effect.id.clone());
        Ok(true)
    }

    fn active_lease(&self, house: &str, node: &str, epoch: u64, now_ms: u64) -> Result<&Lease> {
        let lease = self.lease(house).context("unknown house")?;
        ensure!(lease.owner == node, "node is not the current owner");
        ensure!(lease.epoch == epoch, "stale or unknown house epoch");
        ensure!(now_ms < lease.expires_ms, "house lease expired");
        Ok(lease)
    }

    // Failed calls also observe time: an expired lease cannot be revived by a rewind.
    fn observe_time(&mut self, now_ms: u64) -> Result<()> {
        ensure!(
            now_ms >= self.last_now_ms,
            "monotonic test clock moved backwards"
        );
        self.last_now_ms = now_ms;
        Ok(())
    }
}

fn deadline(now_ms: u64) -> Result<u64> {
    now_ms
        .checked_add(LEASE_MS)
        .context("lease deadline overflow")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::preset;

    fn effect(house: &str, node: &str, epoch: u64, id: &str, scope: Scope) -> Effect {
        Effect {
            house: house.into(),
            node: node.into(),
            epoch,
            id: id.into(),
            scope,
        }
    }

    #[test]
    fn one_machine_can_be_primary_and_passive_for_independent_houses() {
        let mut authority = Authority::new(preset("cross").unwrap(), 10).unwrap();
        authority.committed("house-a", "node-a", 1, 20, 1).unwrap();
        let house_b = authority.lease("house-b").unwrap().clone();
        let promoted = authority
            .promote("house-a", "node-b", 30, true, true, 1)
            .unwrap();
        assert_eq!((promoted.epoch, promoted.sequence), (2, 1));
        assert_eq!(authority.lease("house-b"), Some(&house_b));
        assert_eq!(authority.lease("house-a").unwrap().owner, "node-b");
        assert_eq!(authority.lease("house-b").unwrap().owner, "node-b");
        authority.committed("house-a", "node-b", 2, 31, 2).unwrap();
        authority.committed("house-b", "node-b", 1, 31, 1).unwrap();
    }

    #[test]
    fn partition_cannot_promote_before_the_previous_lease_expires() {
        let mut authority = Authority::new(preset("cross").unwrap(), 0).unwrap();
        assert!(
            authority
                .promote("house-a", "node-b", LEASE_MS - 1, true, false, 0)
                .is_err()
        );
        assert_eq!(authority.lease("house-a").unwrap().epoch, 1);
        assert!(
            authority
                .promote("house-a", "node-b", LEASE_MS, true, false, 0)
                .is_ok()
        );
    }

    #[test]
    fn old_owner_and_old_epoch_are_rejected_even_for_duplicate_ids() {
        let mut authority = Authority::new(preset("cross").unwrap(), 0).unwrap();
        let old = effect("house-a", "node-a", 1, "stable", Scope::Cloud);
        assert!(authority.accept_effect(&old, 1).unwrap());
        authority
            .promote("house-a", "node-b", 2, true, true, 0)
            .unwrap();
        assert!(authority.accept_effect(&old, 2).is_err());
        assert!(authority.renew("house-a", "node-a", 1, 2).is_err());
        assert!(authority.committed("house-a", "node-a", 1, 2, 1).is_err());
        assert!(
            authority
                .accept_effect(&effect("house-a", "node-b", 1, "other", Scope::Cloud), 2)
                .is_err()
        );
        assert!(authority.renew("house-a", "node-b", 1, 2).is_err());
        assert!(authority.committed("house-a", "node-b", 1, 2, 1).is_err());
    }

    #[test]
    fn ttl_boundary_rejects_effects_commits_and_renewals() {
        let mut authority = Authority::new(preset("cross").unwrap(), 0).unwrap();
        assert!(
            authority
                .accept_effect(
                    &effect("house-a", "node-a", 1, "last", Scope::Local),
                    LEASE_MS - 1
                )
                .unwrap()
        );
        assert!(
            authority
                .accept_effect(
                    &effect("house-a", "node-a", 1, "expired", Scope::Local),
                    LEASE_MS
                )
                .is_err()
        );
        assert!(
            authority
                .committed("house-a", "node-a", 1, LEASE_MS, 1)
                .is_err()
        );
        assert!(authority.renew("house-a", "node-a", 1, LEASE_MS).is_err());
    }

    #[test]
    fn renewal_extends_only_a_live_matching_lease() {
        let mut authority = Authority::new(preset("cross").unwrap(), 0).unwrap();
        let renewed_at = LEASE_MS - 100;
        let lease = authority.renew("house-a", "node-a", 1, renewed_at).unwrap();
        assert_eq!(lease.expires_ms, renewed_at + LEASE_MS);
        assert_eq!(authority.lease("house-b").unwrap().expires_ms, LEASE_MS);
        assert!(
            authority
                .promote("house-a", "node-b", LEASE_MS, true, false, 0)
                .is_err()
        );
        assert!(
            authority
                .promote("house-a", "node-b", renewed_at + LEASE_MS, true, false, 0)
                .is_ok()
        );
    }

    #[test]
    fn without_witness_expiration_alone_never_grants_authority() {
        let mut topology = preset("cross").unwrap();
        topology.witness = false;
        let mut authority = Authority::new(topology, 0).unwrap();
        assert!(
            authority
                .promote("house-a", "node-b", LEASE_MS, true, false, 0)
                .is_err()
        );
        assert!(
            authority
                .promote("house-a", "node-b", LEASE_MS + 1, false, true, 0)
                .is_err()
        );
        assert!(
            authority
                .promote("house-a", "node-b", LEASE_MS + 2, true, true, 0)
                .is_ok()
        );
    }

    #[test]
    fn remote_promotion_is_manual_and_grants_cloud_effects_only() {
        for name in ["cross", "cloud"] {
            let mut authority = Authority::new(preset(name).unwrap(), 0).unwrap();
            let candidate = if name == "cross" { "node-b" } else { "cloud-a" };
            assert!(
                authority
                    .promote("house-a", candidate, LEASE_MS, false, false, 0)
                    .is_err()
            );
            let lease = authority
                .promote("house-a", candidate, LEASE_MS, true, false, 0)
                .unwrap();
            assert!(
                authority
                    .accept_effect(
                        &effect("house-a", candidate, lease.epoch, "local", Scope::Local),
                        LEASE_MS + 1
                    )
                    .is_err()
            );
            assert!(
                authority
                    .accept_effect(
                        &effect("house-a", candidate, lease.epoch, "cloud", Scope::Cloud),
                        LEASE_MS + 1
                    )
                    .unwrap()
            );
        }
    }

    #[test]
    fn configured_local_standby_can_promote_automatically() {
        let mut authority = Authority::new(preset("local").unwrap(), 0).unwrap();
        let lease = authority
            .promote("house-a", "backup-local-a", LEASE_MS, false, false, 0)
            .unwrap();
        assert_eq!(lease.epoch, 2);
        assert!(
            authority
                .accept_effect(
                    &effect("house-a", "backup-local-a", 2, "local", Scope::Local),
                    LEASE_MS + 1
                )
                .unwrap()
        );
        assert!(
            authority
                .promote("house-a", "node-a", LEASE_MS + 2, false, true, 0)
                .is_err()
        );
        assert!(
            authority
                .promote("house-a", "node-a", LEASE_MS + 2, true, true, 0)
                .is_ok()
        );
    }

    #[test]
    fn lagging_or_ahead_replicas_cannot_promote() {
        let mut authority = Authority::new(preset("cross").unwrap(), 0).unwrap();
        authority.committed("house-a", "node-a", 1, 1, 1).unwrap();
        assert!(
            authority
                .promote("house-a", "node-b", LEASE_MS, true, false, 0)
                .is_err()
        );
        assert!(
            authority
                .promote("house-a", "node-b", LEASE_MS, true, false, 2)
                .is_err()
        );
        assert_eq!(authority.lease("house-a").unwrap().epoch, 1);
        assert!(
            authority
                .promote("house-a", "node-b", LEASE_MS, true, false, 1)
                .is_ok()
        );
    }

    #[test]
    fn receipt_ids_survive_promotion_and_remain_separate_between_houses() {
        let mut authority = Authority::new(preset("cross").unwrap(), 0).unwrap();
        assert!(
            authority
                .accept_effect(&effect("house-a", "node-a", 1, "stable", Scope::Cloud), 1)
                .unwrap()
        );
        assert!(
            !authority
                .accept_effect(&effect("house-a", "node-a", 1, "stable", Scope::Cloud), 1)
                .unwrap()
        );
        assert!(
            authority
                .accept_effect(&effect("house-b", "node-b", 1, "stable", Scope::Cloud), 1)
                .unwrap()
        );
        authority
            .promote("house-a", "node-b", 2, true, true, 0)
            .unwrap();
        assert!(
            !authority
                .accept_effect(&effect("house-a", "node-b", 2, "stable", Scope::Cloud), 2)
                .unwrap()
        );
    }

    #[test]
    fn expired_owner_can_return_only_with_manual_fencing_and_a_complete_replica() {
        let mut authority = Authority::new(preset("cross").unwrap(), 0).unwrap();
        authority.committed("house-a", "node-a", 1, 1, 1).unwrap();
        let old_effect = effect("house-a", "node-a", 1, "stable", Scope::Local);
        assert!(authority.accept_effect(&old_effect, 1).unwrap());
        assert!(
            authority
                .promote("house-a", "node-a", 2, true, false, 1)
                .is_err()
        );
        assert!(
            authority
                .promote("house-a", "node-a", LEASE_MS, true, false, 1)
                .is_err()
        );
        assert!(
            authority
                .promote("house-a", "node-a", LEASE_MS, false, true, 1)
                .is_err()
        );
        assert!(
            authority
                .promote("house-a", "node-a", LEASE_MS, true, true, 0)
                .is_err()
        );
        assert_eq!(authority.lease("house-a").unwrap().epoch, 1);

        let mut preview = authority.clone();
        let lease = preview
            .promote("house-a", "node-a", LEASE_MS, true, true, 1)
            .unwrap();
        assert_eq!(authority.lease("house-a").unwrap().epoch, 1);
        assert_eq!(lease.owner, "node-a");
        assert_eq!((lease.epoch, lease.sequence), (2, 1));
        assert_eq!(lease.expires_ms, 2 * LEASE_MS);
        assert!(preview.accept_effect(&old_effect, LEASE_MS + 1).is_err());
        assert!(
            !preview
                .accept_effect(
                    &effect("house-a", "node-a", 2, "stable", Scope::Local),
                    LEASE_MS + 1
                )
                .unwrap()
        );
        preview
            .committed("house-a", "node-a", 2, LEASE_MS + 1, 2)
            .unwrap();
    }

    #[test]
    fn receipt_capacity_refuses_new_ids_without_allowing_old_ids_to_replay() {
        let mut authority = Authority::new(preset("cross").unwrap(), 0).unwrap();
        for index in 0..MAX_RECEIPTS_PER_HOUSE {
            assert!(
                authority
                    .accept_effect(
                        &effect("house-a", "node-a", 1, &format!("id-{index}"), Scope::Local),
                        1
                    )
                    .unwrap()
            );
        }
        assert!(
            authority
                .accept_effect(&effect("house-a", "node-a", 1, "overflow", Scope::Local), 1)
                .is_err()
        );
        assert!(
            !authority
                .accept_effect(&effect("house-a", "node-a", 1, "id-0", Scope::Local), 1)
                .unwrap()
        );
        authority
            .promote("house-a", "node-b", 2, true, true, 0)
            .unwrap();
        assert!(
            !authority
                .accept_effect(&effect("house-a", "node-b", 2, "id-0", Scope::Cloud), 2)
                .unwrap()
        );
        assert!(
            authority
                .accept_effect(
                    &effect("house-a", "node-b", 2, "new-epoch-id", Scope::Cloud),
                    2
                )
                .is_err()
        );
        assert!(
            authority
                .accept_effect(&effect("house-b", "node-b", 1, "overflow", Scope::Local), 2)
                .unwrap()
        );
    }

    #[test]
    fn monotonic_clock_cannot_revive_expired_authority() {
        let mut authority = Authority::new(preset("cross").unwrap(), 10).unwrap();
        assert!(authority.renew("house-a", "node-a", 1, 9).is_err());
        assert!(
            authority
                .renew("house-a", "node-a", 1, 10 + LEASE_MS)
                .is_err()
        );
        assert!(
            authority
                .renew("house-a", "node-a", 1, 10 + LEASE_MS - 1)
                .is_err()
        );
        assert!(
            authority
                .promote("house-a", "node-b", 10 + LEASE_MS - 1, true, true, 0)
                .is_err()
        );
        assert_eq!(authority.lease("house-a").unwrap().epoch, 1);
    }

    #[test]
    fn invalid_effect_ids_and_unknown_houses_are_rejected() {
        let mut authority = Authority::new(preset("cross").unwrap(), 0).unwrap();
        for id in ["", "../id", "white space", "é", &"x".repeat(65)] {
            assert!(
                authority
                    .accept_effect(&effect("house-a", "node-a", 1, id, Scope::Local), 1)
                    .is_err()
            );
        }
        assert!(
            authority
                .accept_effect(&effect("unknown", "node-a", 1, "id", Scope::Cloud), 1)
                .is_err()
        );
        assert!(
            authority
                .promote("house-a", "unknown", 1, true, true, 0)
                .is_err()
        );
    }

    #[test]
    fn no_ha_still_accepts_primary_but_cannot_promote() {
        let mut authority = Authority::new(preset("none").unwrap(), 0).unwrap();
        assert_eq!(authority.leases().len(), 2);
        assert!(
            authority
                .accept_effect(&effect("house-a", "node-a", 1, "id", Scope::Local), 1)
                .unwrap()
        );
        assert!(
            authority
                .promote("house-a", "node-b", 1, true, true, 0)
                .is_err()
        );
    }

    #[test]
    fn commits_must_be_contiguous_and_continue_after_promotion() {
        let mut authority = Authority::new(preset("cross").unwrap(), 0).unwrap();
        for sequence in [0, 2, u64::MAX] {
            assert!(
                authority
                    .committed("house-a", "node-a", 1, 1, sequence)
                    .is_err()
            );
        }
        authority.committed("house-a", "node-a", 1, 1, 1).unwrap();
        authority
            .promote("house-a", "node-b", 2, true, true, 1)
            .unwrap();
        assert!(authority.committed("house-a", "node-b", 2, 2, 1).is_err());
        authority.committed("house-a", "node-b", 2, 2, 2).unwrap();
    }

    #[test]
    fn deadline_and_epoch_overflows_do_not_wrap() {
        assert!(Authority::new(preset("cross").unwrap(), u64::MAX).is_err());
        let mut authority = Authority::new(preset("cross").unwrap(), 0).unwrap();
        authority.leases.get_mut("house-a").unwrap().epoch = u64::MAX;
        assert!(
            authority
                .promote("house-a", "node-b", LEASE_MS, true, false, 0)
                .is_err()
        );
        assert_eq!(authority.lease("house-a").unwrap().epoch, u64::MAX);
        authority.leases.get_mut("house-b").unwrap().expires_ms = 2 * LEASE_MS;
        authority.leases.get_mut("house-b").unwrap().sequence = u64::MAX;
        assert!(
            authority
                .committed("house-b", "node-b", 1, LEASE_MS, 0)
                .is_err()
        );
        assert_eq!(authority.lease("house-b").unwrap().sequence, u64::MAX);

        let current = authority.leases.get_mut("house-b").unwrap();
        current.expires_ms = u64::MAX;
        let before = current.clone();
        assert!(
            authority
                .renew("house-b", "node-b", 1, u64::MAX - 1)
                .is_err()
        );
        assert_eq!(authority.lease("house-b"), Some(&before));
    }
}
