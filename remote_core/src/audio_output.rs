//! Pure, scope-aware ownership policy for native audio output routing.
//!
//! The native coordinator owns OS changes and should update this ledger only
//! after the corresponding native operation succeeds. Lease IDs are unique
//! owner tokens, not subscription or stream IDs.
use protocol::audio_output::{AudioOutputMode, AudioOutputScope, LocalPlayback};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioOutputLease {
    pub scope: AudioOutputScope,
    pub mode: AudioOutputMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidAudioOutputScope;

#[derive(Debug, Clone, Default)]
pub struct AudioOutputLeases {
    leases: BTreeMap<u64, AudioOutputLease>,
}

impl AudioOutputLeases {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new owner. Returns false without changing the ledger if its ID
    /// is already present. Use `update` to deliberately replace an owner's lease.
    pub fn insert(
        &mut self,
        id: u64,
        scope: AudioOutputScope,
        mode: AudioOutputMode,
    ) -> Result<bool, InvalidAudioOutputScope> {
        validate_scope(scope)?;
        if self.leases.contains_key(&id) {
            return Ok(false);
        }
        self.leases.insert(id, AudioOutputLease { scope, mode });
        Ok(true)
    }

    /// Replace an existing owner's scope and mode. Returns false when the ID
    /// is absent, leaving other owners untouched.
    pub fn update(
        &mut self,
        id: u64,
        scope: AudioOutputScope,
        mode: AudioOutputMode,
    ) -> Result<bool, InvalidAudioOutputScope> {
        validate_scope(scope)?;
        let Some(lease) = self.leases.get_mut(&id) else {
            return Ok(false);
        };
        *lease = AudioOutputLease { scope, mode };
        Ok(true)
    }

    /// Remove one owner. Repeated removal is harmless and returns false.
    pub fn remove(&mut self, id: u64) -> bool {
        self.leases.remove(&id).is_some()
    }

    /// Compute local playback allowed for this scope across all current owners.
    pub fn effective(
        &self,
        scope: AudioOutputScope,
    ) -> Result<LocalPlayback, InvalidAudioOutputScope> {
        validate_scope(scope)?;
        let system_private = self.leases.values().any(|lease| {
            lease.scope == AudioOutputScope::System && lease.mode == AudioOutputMode::Private
        });
        if system_private {
            return Ok(LocalPlayback::Disabled);
        }

        match scope {
            AudioOutputScope::System => {
                if self.leases.values().any(|lease| {
                    matches!(lease.scope, AudioOutputScope::Application(_))
                        && lease.mode == AudioOutputMode::Private
                }) {
                    Ok(LocalPlayback::Partial)
                } else {
                    Ok(LocalPlayback::Enabled)
                }
            }
            AudioOutputScope::Application(process_id) => {
                if self.leases.values().any(|lease| {
                    lease.scope == AudioOutputScope::Application(process_id)
                        && lease.mode == AudioOutputMode::Private
                }) {
                    Ok(LocalPlayback::Disabled)
                } else {
                    Ok(LocalPlayback::Enabled)
                }
            }
        }
    }

    pub fn get(&self, id: u64) -> Option<AudioOutputLease> {
        self.leases.get(&id).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (u64, AudioOutputLease)> + '_ {
        self.leases.iter().map(|(&id, &lease)| (id, lease))
    }

    pub fn len(&self) -> usize {
        self.leases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }
}

fn validate_scope(scope: AudioOutputScope) -> Result<(), InvalidAudioOutputScope> {
    if scope.is_valid() {
        Ok(())
    } else {
        Err(InvalidAudioOutputScope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYSTEM: AudioOutputScope = AudioOutputScope::System;
    const APP_A: AudioOutputScope = AudioOutputScope::Application(101);
    const APP_B: AudioOutputScope = AudioOutputScope::Application(202);

    fn effective(leases: &AudioOutputLeases, scope: AudioOutputScope) -> LocalPlayback {
        leases.effective(scope).unwrap()
    }

    #[test]
    fn mirror_and_private_owners_resolve_independently_and_private_wins_overlap() {
        let mut leases = AudioOutputLeases::new();
        assert!(leases.insert(1, APP_A, AudioOutputMode::Mirror).unwrap());
        assert!(leases.insert(2, APP_A, AudioOutputMode::Private).unwrap());
        assert!(leases.insert(3, APP_B, AudioOutputMode::Private).unwrap());

        assert_eq!(effective(&leases, APP_A), LocalPlayback::Disabled);
        assert_eq!(effective(&leases, APP_B), LocalPlayback::Disabled);
        assert_eq!(effective(&leases, SYSTEM), LocalPlayback::Partial);

        assert!(leases.insert(4, SYSTEM, AudioOutputMode::Private).unwrap());
        assert_eq!(effective(&leases, SYSTEM), LocalPlayback::Disabled);
        assert_eq!(effective(&leases, APP_B), LocalPlayback::Disabled);
    }

    #[test]
    fn removing_one_owner_never_releases_another_owners_private_claim() {
        let mut leases = AudioOutputLeases::new();
        leases.insert(1, SYSTEM, AudioOutputMode::Private).unwrap();
        leases.insert(2, SYSTEM, AudioOutputMode::Private).unwrap();
        leases.insert(3, APP_A, AudioOutputMode::Mirror).unwrap();

        assert!(leases.remove(1));
        assert_eq!(effective(&leases, APP_A), LocalPlayback::Disabled);
        assert!(leases.remove(2));
        assert_eq!(effective(&leases, APP_A), LocalPlayback::Enabled);
        assert_eq!(effective(&leases, APP_B), LocalPlayback::Enabled);
        assert!(!leases.remove(2));
    }

    #[test]
    fn updating_replaces_only_that_lease_and_duplicate_insert_is_noop() {
        let mut leases = AudioOutputLeases::new();
        leases.insert(1, APP_A, AudioOutputMode::Private).unwrap();
        leases.insert(2, APP_B, AudioOutputMode::Mirror).unwrap();

        assert!(!leases.insert(1, SYSTEM, AudioOutputMode::Mirror).unwrap());
        assert_eq!(leases.get(1).unwrap().scope, APP_A);
        assert!(leases.update(1, APP_B, AudioOutputMode::Mirror).unwrap());
        assert_eq!(effective(&leases, APP_A), LocalPlayback::Enabled);
        assert_eq!(effective(&leases, APP_B), LocalPlayback::Enabled);
        assert_eq!(effective(&leases, SYSTEM), LocalPlayback::Enabled);
        assert!(!leases.update(99, SYSTEM, AudioOutputMode::Private).unwrap());
        assert_eq!(leases.len(), 2);
    }

    #[test]
    fn application_private_is_partial_for_system_and_does_not_affect_other_apps() {
        let mut leases = AudioOutputLeases::new();
        leases.insert(1, APP_A, AudioOutputMode::Private).unwrap();

        assert_eq!(effective(&leases, SYSTEM), LocalPlayback::Partial);
        assert_eq!(effective(&leases, APP_A), LocalPlayback::Disabled);
        assert_eq!(effective(&leases, APP_B), LocalPlayback::Enabled);
        assert!(
            leases
                .iter()
                .any(|(id, lease)| id == 1 && lease.scope == APP_A)
        );
    }

    #[test]
    fn invalid_application_pid_is_rejected_without_mutating_ledger() {
        let mut leases = AudioOutputLeases::new();
        assert_eq!(
            leases.insert(
                1,
                AudioOutputScope::Application(0),
                AudioOutputMode::Private
            ),
            Err(InvalidAudioOutputScope)
        );
        assert_eq!(
            leases.effective(AudioOutputScope::Application(0)),
            Err(InvalidAudioOutputScope)
        );
        assert!(leases.is_empty());
    }
}
