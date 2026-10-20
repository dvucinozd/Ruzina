#![no_std]
#![forbid(unsafe_code)]

use core::num::{NonZeroU32, NonZeroU64};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MediaSourceId(NonZeroU32);

impl MediaSourceId {
    pub const fn new(value: u32) -> Option<Self> {
        match NonZeroU32::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MediaHandle(NonZeroU64);

impl MediaHandle {
    pub const fn new(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MediaGeneration(u32);

impl MediaGeneration {
    pub const fn initial() -> Self {
        Self(0)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    fn next(self) -> Self {
        let mut value = self.0.wrapping_add(1);
        if value == 0 {
            value = 1;
        }
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaLease {
    pub generation: MediaGeneration,
    pub source: MediaSourceId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectResult {
    Accepted(MediaLease),
    Duplicate(MediaLease),
    IgnoredSecondary(MediaLease),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisconnectResult {
    Accepted,
    IgnoredForeign,
    AlreadyInactive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaSession {
    connected: bool,
    mounted: bool,
    source: Option<MediaSourceId>,
    handle: Option<MediaHandle>,
    generation: MediaGeneration,
}

impl MediaSession {
    pub const fn new() -> Self {
        Self {
            connected: false,
            mounted: false,
            source: None,
            handle: None,
            generation: MediaGeneration::initial(),
        }
    }

    pub fn on_connect(&mut self, source: MediaSourceId) -> ConnectResult {
        if self.connected
            && let Some(current_source) = self.source
        {
            let current = MediaLease {
                generation: self.generation,
                source: current_source,
            };
            if current_source == source {
                return ConnectResult::Duplicate(current);
            }
            if self.handle.is_some() || self.mounted {
                return ConnectResult::IgnoredSecondary(current);
            }

            self.source = Some(source);
            self.generation = self.generation.next();
            return ConnectResult::Accepted(MediaLease {
                generation: self.generation,
                source,
            });
        }

        self.connected = true;
        self.mounted = false;
        self.source = Some(source);
        self.handle = None;
        self.generation = self.generation.next();
        ConnectResult::Accepted(MediaLease {
            generation: self.generation,
            source,
        })
    }

    pub fn bind_handle(&mut self, lease: MediaLease, handle: MediaHandle) -> bool {
        if !self.validate(lease) {
            return false;
        }
        if let Some(current) = self.handle
            && current != handle
        {
            return false;
        }
        self.handle = Some(handle);
        true
    }

    pub fn release_handle(&mut self, handle: MediaHandle) {
        if self.handle == Some(handle) {
            self.handle = None;
        }
    }

    pub fn on_disconnect(&mut self, handle: Option<MediaHandle>) -> DisconnectResult {
        if !self.connected {
            return DisconnectResult::AlreadyInactive;
        }
        if let Some(owner) = self.handle
            && handle != Some(owner)
        {
            return DisconnectResult::IgnoredForeign;
        }

        self.connected = false;
        self.mounted = false;
        self.source = None;
        self.handle = None;
        self.generation = self.generation.next();
        DisconnectResult::Accepted
    }

    pub fn commit_mounted(&mut self, lease: MediaLease) -> bool {
        if !self.validate(lease) || self.handle.is_none() {
            return false;
        }
        self.mounted = true;
        true
    }

    pub fn mark_unmounted(&mut self) {
        self.mounted = false;
    }

    pub const fn is_connected(&self) -> bool {
        self.connected
    }

    pub const fn is_mounted(&self) -> bool {
        self.mounted
    }

    pub const fn is_available(&self) -> bool {
        self.connected && self.mounted
    }

    pub const fn generation(&self) -> MediaGeneration {
        self.generation
    }

    pub const fn handle(&self) -> Option<MediaHandle> {
        self.handle
    }

    pub const fn lease(&self) -> Option<MediaLease> {
        match (self.connected, self.source) {
            (true, Some(source)) => Some(MediaLease {
                generation: self.generation,
                source,
            }),
            _ => None,
        }
    }

    pub const fn validate(&self, lease: MediaLease) -> bool {
        self.connected
            && self.generation.0 == lease.generation.0
            && match self.source {
                Some(source) => source.get() == lease.source.get(),
                None => false,
            }
    }
}

impl Default for MediaSession {
    fn default() -> Self {
        Self::new()
    }
}

pub use pajoniiir_media_block::{BlockDevice, BlockGeometry};

pub trait MediaReader {
    type Error;

    fn lease(&self) -> MediaLease;
    fn len(&self) -> u64;
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> Result<usize, Self::Error>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(value: u32) -> MediaSourceId {
        MediaSourceId::new(value).unwrap()
    }

    fn handle(value: u64) -> MediaHandle {
        MediaHandle::new(value).unwrap()
    }

    fn accepted(result: ConnectResult) -> MediaLease {
        match result {
            ConnectResult::Accepted(lease) => lease,
            _ => panic!("expected accepted media session"),
        }
    }

    #[test]
    fn zero_source_and_handle_tokens_are_unrepresentable() {
        assert_eq!(MediaSourceId::new(0), None);
        assert_eq!(MediaHandle::new(0), None);
    }

    #[test]
    fn first_connect_and_duplicate_match_released_session_behavior() {
        let mut session = MediaSession::new();
        let first = accepted(session.on_connect(source(3)));
        assert_eq!(first.generation.get(), 1);
        assert!(session.is_connected());
        assert!(!session.is_mounted());

        assert_eq!(
            session.on_connect(source(3)),
            ConnectResult::Duplicate(first)
        );
        assert_eq!(session.generation(), first.generation);
    }

    #[test]
    fn unbound_enumeration_bounce_supersedes_stale_source() {
        let mut session = MediaSession::new();
        let stale = accepted(session.on_connect(source(3)));
        let fresh = accepted(session.on_connect(source(4)));

        assert_ne!(stale.generation, fresh.generation);
        assert!(!session.validate(stale));
        assert!(session.validate(fresh));
        assert_eq!(fresh.source, source(4));
    }

    #[test]
    fn bound_or_mounted_session_ignores_secondary_source() {
        let mut session = MediaSession::new();
        let lease = accepted(session.on_connect(source(3)));
        assert!(session.bind_handle(lease, handle(10)));

        assert_eq!(
            session.on_connect(source(4)),
            ConnectResult::IgnoredSecondary(lease)
        );
        assert!(session.commit_mounted(lease));
        assert_eq!(
            session.on_connect(source(5)),
            ConnectResult::IgnoredSecondary(lease)
        );
        assert!(session.is_available());
    }

    #[test]
    fn handle_binding_is_generation_and_owner_safe() {
        let mut session = MediaSession::new();
        let old = accepted(session.on_connect(source(3)));
        let current = accepted(session.on_connect(source(4)));

        assert!(!session.bind_handle(old, handle(10)));
        assert!(session.bind_handle(current, handle(11)));
        assert!(session.bind_handle(current, handle(11)));
        assert!(!session.bind_handle(current, handle(12)));
        assert_eq!(session.handle(), Some(handle(11)));
    }

    #[test]
    fn foreign_disconnect_is_ignored_after_handle_binding() {
        let mut session = MediaSession::new();
        let lease = accepted(session.on_connect(source(3)));
        assert!(session.bind_handle(lease, handle(10)));

        assert_eq!(
            session.on_disconnect(Some(handle(11))),
            DisconnectResult::IgnoredForeign
        );
        assert!(session.validate(lease));
        assert_eq!(
            session.on_disconnect(None),
            DisconnectResult::IgnoredForeign
        );
        assert_eq!(
            session.on_disconnect(Some(handle(10))),
            DisconnectResult::Accepted
        );
        assert!(!session.is_connected());
    }

    #[test]
    fn unbound_disconnect_is_accepted_and_invalidates_lease() {
        let mut session = MediaSession::new();
        let lease = accepted(session.on_connect(source(3)));

        assert_eq!(session.on_disconnect(None), DisconnectResult::Accepted);
        assert!(!session.validate(lease));
        assert_eq!(
            session.on_disconnect(None),
            DisconnectResult::AlreadyInactive
        );
    }

    #[test]
    fn reconnect_same_source_never_revalidates_old_handles() {
        let mut session = MediaSession::new();
        let old = accepted(session.on_connect(source(3)));
        assert!(session.bind_handle(old, handle(10)));
        assert_eq!(
            session.on_disconnect(Some(handle(10))),
            DisconnectResult::Accepted
        );

        let fresh = accepted(session.on_connect(source(3)));
        assert_ne!(fresh.generation, old.generation);
        assert!(!session.validate(old));
        assert!(session.validate(fresh));
    }

    #[test]
    fn mount_commit_requires_current_bound_lease() {
        let mut session = MediaSession::new();
        let stale = accepted(session.on_connect(source(3)));
        let lease = accepted(session.on_connect(source(4)));

        assert!(!session.commit_mounted(lease));
        assert!(session.bind_handle(lease, handle(22)));
        assert!(!session.commit_mounted(stale));
        assert!(session.commit_mounted(lease));
        assert!(session.is_available());

        session.mark_unmounted();
        assert!(!session.is_available());
        assert!(session.is_connected());
    }

    #[test]
    fn releasing_foreign_handle_does_not_clear_owner() {
        let mut session = MediaSession::new();
        let lease = accepted(session.on_connect(source(3)));
        assert!(session.bind_handle(lease, handle(22)));

        session.release_handle(handle(23));
        assert_eq!(session.handle(), Some(handle(22)));
        session.release_handle(handle(22));
        assert_eq!(session.handle(), None);
    }

    #[test]
    fn generation_skips_zero_on_wrap() {
        let mut session = MediaSession::new();
        session.generation = MediaGeneration(u32::MAX);
        let lease = accepted(session.on_connect(source(9)));

        assert_eq!(lease.generation.get(), 1);
        assert!(session.validate(lease));
    }
}
