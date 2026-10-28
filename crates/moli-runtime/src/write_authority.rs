//! Server-owned write authority for devices whose physical writer is being
//! transferred between integrations.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

use moli_core::{DeviceId, InstanceId};

/// The current authority generation for a managed device.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WriteAuthority {
    pub generation: u64,
    pub allowed: bool,
}

/// Refusal to install an invalid or out-of-order authority generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum WriteAuthorityError {
    #[error("write authority generation must be nonzero")]
    ZeroGeneration,
    #[error("write authority generation is older than the installed generation")]
    StaleGeneration,
    #[error("write authority cannot change within the same generation")]
    GenerationConflict,
}

#[derive(Debug, Default)]
pub(crate) struct WriteAuthorities(
    RwLock<BTreeMap<DeviceId, WriteAuthority>>,
    RwLock<BTreeSet<InstanceId>>,
);

impl WriteAuthorities {
    pub(crate) fn get(&self, device: &DeviceId) -> Option<WriteAuthority> {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(device)
            .copied()
            .or_else(|| self.default_for(device))
    }

    fn default_for(&self, device: &DeviceId) -> Option<WriteAuthority> {
        let instance = device
            .as_str()
            .split_once(':')
            .map_or(device.as_str(), |(instance, _)| instance);
        self.1
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|managed| managed.as_str() == instance)
            .then_some(WriteAuthority {
                generation: 1,
                allowed: false,
            })
    }

    pub(crate) fn manage_instance(&self, instance: &InstanceId) {
        self.1
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(instance.clone());
    }

    pub(crate) fn set(
        &self,
        device: &DeviceId,
        next: WriteAuthority,
    ) -> Result<(), WriteAuthorityError> {
        if next.generation == 0 {
            return Err(WriteAuthorityError::ZeroGeneration);
        }
        let mut authorities = self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(current) = authorities
            .get(device)
            .copied()
            .or_else(|| self.default_for(device))
        {
            if next.generation < current.generation {
                return Err(WriteAuthorityError::StaleGeneration);
            }
            if next.generation == current.generation {
                return if next.allowed == current.allowed {
                    Ok(())
                } else {
                    Err(WriteAuthorityError::GenerationConflict)
                };
            }
        }
        authorities.insert(device.clone(), next);
        Ok(())
    }

    /// Closes the current generation without advancing it. This emergency
    /// fence is deliberately one-way: only a later persisted generation can
    /// reopen the device.
    pub(crate) fn fence(&self, device: &DeviceId) -> WriteAuthority {
        let mut authorities = self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let authority = authorities.entry(device.clone()).or_insert(WriteAuthority {
            generation: 1,
            allowed: false,
        });
        authority.allowed = false;
        *authority
    }

    pub(crate) fn snapshot_permit(
        authorities: std::sync::Arc<Self>,
        point: moli_core::PointId,
        device: Option<DeviceId>,
        expected: Option<WriteAuthority>,
        previous: Option<crate::CommandPermit>,
    ) -> crate::CommandPermit {
        crate::CommandPermit::new(move |candidate| {
            if candidate != &point {
                return false;
            }
            let current = device.as_ref().and_then(|device| authorities.get(device));
            let authority_allows = current == expected && expected.is_none_or(|a| a.allowed);
            authority_allows
                && previous
                    .as_ref()
                    .is_none_or(|permit| permit.allows(candidate))
        })
    }

    pub(crate) fn generation_permit(
        authorities: std::sync::Arc<Self>,
        device: DeviceId,
        expected_generation: u64,
    ) -> crate::CommandPermit {
        crate::CommandPermit::new(move |point| {
            point.split().is_some_and(|(target, _)| target == device)
                && authorities.get(&device).is_some_and(|authority| {
                    authority.allowed && authority.generation == expected_generation
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> DeviceId {
        DeviceId::new(&moli_core::InstanceId::from("authority-test"), "cover")
    }

    #[test]
    fn generations_are_monotonic_and_same_generation_is_only_idempotent() {
        let authorities = WriteAuthorities::default();
        let device = device();
        assert_eq!(
            authorities.set(
                &device,
                WriteAuthority {
                    generation: 0,
                    allowed: true,
                }
            ),
            Err(WriteAuthorityError::ZeroGeneration)
        );
        let first = WriteAuthority {
            generation: 1,
            allowed: true,
        };
        assert_eq!(authorities.set(&device, first), Ok(()));
        assert_eq!(authorities.set(&device, first), Ok(()));
        assert_eq!(
            authorities.set(
                &device,
                WriteAuthority {
                    generation: 1,
                    allowed: false,
                }
            ),
            Err(WriteAuthorityError::GenerationConflict)
        );
        assert_eq!(
            authorities.set(
                &device,
                WriteAuthority {
                    generation: 0,
                    allowed: false,
                }
            ),
            Err(WriteAuthorityError::ZeroGeneration)
        );
        assert_eq!(
            authorities.set(
                &device,
                WriteAuthority {
                    generation: 2,
                    allowed: false,
                }
            ),
            Ok(())
        );
        assert_eq!(
            authorities.set(&device, first),
            Err(WriteAuthorityError::StaleGeneration)
        );
        assert_eq!(
            authorities.get(&device),
            Some(WriteAuthority {
                generation: 2,
                allowed: false,
            })
        );
    }

    #[test]
    fn authority_is_serializable() {
        let authority = WriteAuthority {
            generation: 7,
            allowed: true,
        };
        let encoded = serde_json::to_string(&authority).unwrap();
        assert_eq!(
            serde_json::from_str::<WriteAuthority>(&encoded).unwrap(),
            authority
        );
    }

    #[test]
    fn managed_instance_is_closed_before_its_devices_are_discovered() {
        let authorities = WriteAuthorities::default();
        let device = device();
        authorities.manage_instance(&device.instance());
        assert_eq!(
            authorities.get(&device),
            Some(WriteAuthority {
                generation: 1,
                allowed: false
            })
        );
        assert_eq!(
            authorities.set(
                &device,
                WriteAuthority {
                    generation: 1,
                    allowed: true
                }
            ),
            Err(WriteAuthorityError::GenerationConflict)
        );
        authorities
            .set(
                &device,
                WriteAuthority {
                    generation: 2,
                    allowed: true,
                },
            )
            .unwrap();
        authorities.manage_instance(&device.instance());
        assert_eq!(
            authorities.get(&device),
            Some(WriteAuthority {
                generation: 2,
                allowed: true
            })
        );
        assert_eq!(
            authorities.get(&DeviceId::new(&InstanceId::from("other"), "device")),
            None
        );
    }

    #[test]
    fn emergency_fence_never_advances_or_reopens_a_generation() {
        let authorities = WriteAuthorities::default();
        let device = device();
        authorities
            .set(
                &device,
                WriteAuthority {
                    generation: 37,
                    allowed: true,
                },
            )
            .unwrap();
        assert_eq!(
            authorities.fence(&device),
            WriteAuthority {
                generation: 37,
                allowed: false,
            }
        );
        assert_eq!(
            authorities.set(
                &device,
                WriteAuthority {
                    generation: 37,
                    allowed: true,
                }
            ),
            Err(WriteAuthorityError::GenerationConflict)
        );
        assert_eq!(
            authorities.set(
                &device,
                WriteAuthority {
                    generation: 38,
                    allowed: true,
                }
            ),
            Ok(())
        );
        assert_eq!(
            authorities.get(&device),
            Some(WriteAuthority {
                generation: 38,
                allowed: true,
            })
        );
    }

    #[test]
    fn emergency_fence_installs_a_closed_first_generation_for_unmanaged_devices() {
        let authorities = WriteAuthorities::default();
        let device = device();
        assert_eq!(authorities.get(&device), None);
        assert_eq!(
            authorities.fence(&device),
            WriteAuthority {
                generation: 1,
                allowed: false,
            }
        );
        assert_eq!(
            authorities.fence(&device),
            WriteAuthority {
                generation: 1,
                allowed: false,
            }
        );
    }
}
