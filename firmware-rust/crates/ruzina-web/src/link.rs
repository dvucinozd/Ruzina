//! Bounded C6 network link (fake transport contract) and Wi-Fi lifecycle.
//!
//! * One Wi-Fi transition at a time, owned by a probe, OTA or the user
//!   toggle (M2.5 `wifi_transition_lease`).
//! * Every link-up starts a new generation; requests still pending when the
//!   link goes down fail at once, and a response from an older generation or
//!   for an unknown request is rejected, never applied.
//! * The SDMMC host may be shared by the SD card and the C6 (one slot each).
//!   Wi-Fi off releases only the C6 slot; the host is initialized once and
//!   torn down only when no slot is attached, so toggling Wi-Fi never resets
//!   SD ownership or the media lease built on it.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionOwner {
    Probe,
    Ota,
    Control,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TransitionLease {
    owner: Option<TransitionOwner>,
}

impl TransitionLease {
    pub const fn new() -> Self {
        Self { owner: None }
    }

    pub fn acquire(&mut self, owner: TransitionOwner) -> bool {
        if self.owner.is_some() {
            return false;
        }
        self.owner = Some(owner);
        true
    }

    /// Only the holder can release.
    pub fn release(&mut self, owner: TransitionOwner) {
        if self.owner == Some(owner) {
            self.owner = None;
        }
    }

    pub const fn owner(&self) -> Option<TransitionOwner> {
        self.owner
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdmmcSlot {
    SdCard,
    C6,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostAction {
    /// First slot attached: bring the host up.
    InitHost,
    None,
    /// Last slot detached: the host may go down.
    DeinitHost,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SharedSdmmcHost {
    sd: bool,
    c6: bool,
}

impl SharedSdmmcHost {
    pub const fn new() -> Self {
        Self {
            sd: false,
            c6: false,
        }
    }

    pub fn attach(&mut self, slot: SdmmcSlot) -> HostAction {
        let was_idle = !self.sd && !self.c6;
        match slot {
            SdmmcSlot::SdCard => self.sd = true,
            SdmmcSlot::C6 => self.c6 = true,
        }
        if was_idle {
            HostAction::InitHost
        } else {
            HostAction::None
        }
    }

    pub fn detach(&mut self, slot: SdmmcSlot) -> HostAction {
        let attached = match slot {
            SdmmcSlot::SdCard => core::mem::replace(&mut self.sd, false),
            SdmmcSlot::C6 => core::mem::replace(&mut self.c6, false),
        };
        if attached && !self.sd && !self.c6 {
            HostAction::DeinitHost
        } else {
            HostAction::None
        }
    }

    pub const fn attached(&self, slot: SdmmcSlot) -> bool {
        match slot {
            SdmmcSlot::SdCard => self.sd,
            SdmmcSlot::C6 => self.c6,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RadioState {
    Off,
    Up { generation: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestId {
    generation: u32,
    id: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkError {
    Down,
    QueueFull,
    /// A transition is held by another owner.
    Busy,
    /// Response for another generation or an unknown/finished request.
    Stale,
}

/// A bounded request queue over the C6 transport, generation-fenced.
pub struct C6Link<K: Copy, const N: usize> {
    state: RadioState,
    generation: u32,
    next_id: u32,
    pending: [Option<(u32, K)>; N],
}

impl<K: Copy, const N: usize> Default for C6Link<K, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Copy, const N: usize> C6Link<K, N> {
    pub const fn new() -> Self {
        Self {
            state: RadioState::Off,
            generation: 0,
            next_id: 0,
            pending: [None; N],
        }
    }

    pub const fn state(&self) -> RadioState {
        self.state
    }

    /// Wi-Fi on under a held transition lease: the C6 slot joins the shared
    /// host and a new link generation starts.
    pub fn up(
        &mut self,
        lease: &TransitionLease,
        owner: TransitionOwner,
        host: &mut SharedSdmmcHost,
    ) -> Result<HostAction, LinkError> {
        if lease.owner() != Some(owner) {
            return Err(LinkError::Busy);
        }
        if let RadioState::Up { .. } = self.state {
            return Ok(HostAction::None);
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        self.state = RadioState::Up {
            generation: self.generation,
        };
        Ok(host.attach(SdmmcSlot::C6))
    }

    /// Wi-Fi off: every pending request fails now; only the C6 slot leaves
    /// the shared host. Returns the failed requests' kinds via `failed`.
    pub fn down(
        &mut self,
        lease: &TransitionLease,
        owner: TransitionOwner,
        host: &mut SharedSdmmcHost,
        mut failed: impl FnMut(K),
    ) -> Result<HostAction, LinkError> {
        if lease.owner() != Some(owner) {
            return Err(LinkError::Busy);
        }
        if self.state == RadioState::Off {
            return Ok(HostAction::None);
        }
        for slot in &mut self.pending {
            if let Some((_, kind)) = slot.take() {
                failed(kind);
            }
        }
        self.state = RadioState::Off;
        Ok(host.detach(SdmmcSlot::C6))
    }

    pub fn submit(&mut self, kind: K) -> Result<RequestId, LinkError> {
        let RadioState::Up { generation } = self.state else {
            return Err(LinkError::Down);
        };
        let slot = self
            .pending
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(LinkError::QueueFull)?;
        self.next_id = self.next_id.wrapping_add(1);
        *slot = Some((self.next_id, kind));
        Ok(RequestId {
            generation,
            id: self.next_id,
        })
    }

    /// A response arrived for `request`; only the current generation's
    /// pending request is completed.
    pub fn complete(&mut self, request: RequestId) -> Result<K, LinkError> {
        if self.state
            != (RadioState::Up {
                generation: request.generation,
            })
        {
            return Err(LinkError::Stale);
        }
        let slot = self
            .pending
            .iter_mut()
            .find(|slot| slot.is_some_and(|(id, _)| id == request.id))
            .ok_or(LinkError::Stale)?;
        Ok(slot.take().expect("matched a pending request").1)
    }

    pub fn pending(&self) -> usize {
        self.pending.iter().flatten().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruzina_media_session::{ConnectResult, MediaSession, MediaSourceId};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Kind {
        Status,
        Ota,
    }

    #[test]
    fn one_transition_owner_at_a_time() {
        let mut lease = TransitionLease::new();
        assert!(lease.acquire(TransitionOwner::Control));
        assert!(!lease.acquire(TransitionOwner::Ota));
        lease.release(TransitionOwner::Ota);
        assert_eq!(lease.owner(), Some(TransitionOwner::Control));
        lease.release(TransitionOwner::Control);
        assert!(lease.acquire(TransitionOwner::Ota));

        let mut link = C6Link::<Kind, 2>::new();
        let mut host = SharedSdmmcHost::new();
        assert_eq!(
            link.up(&lease, TransitionOwner::Control, &mut host),
            Err(LinkError::Busy)
        );
    }

    #[test]
    fn responses_from_an_older_link_generation_are_rejected() {
        let mut lease = TransitionLease::new();
        let mut link = C6Link::<Kind, 2>::new();
        let mut host = SharedSdmmcHost::new();
        assert_eq!(link.submit(Kind::Status), Err(LinkError::Down));
        lease.acquire(TransitionOwner::Control);
        link.up(&lease, TransitionOwner::Control, &mut host)
            .unwrap();
        let status = link.submit(Kind::Status).unwrap();
        let ota = link.submit(Kind::Ota).unwrap();
        assert_eq!(link.submit(Kind::Status), Err(LinkError::QueueFull));
        assert_eq!(link.complete(status), Ok(Kind::Status));
        assert_eq!(
            link.complete(status),
            Err(LinkError::Stale),
            "completed once"
        );

        // Wi-Fi off fails the OTA request now; a late response is ignored.
        let mut failed = [None; 2];
        let mut count = 0;
        link.down(&lease, TransitionOwner::Control, &mut host, |kind| {
            failed[count] = Some(kind);
            count += 1;
        })
        .unwrap();
        assert_eq!(failed, [Some(Kind::Ota), None]);
        assert_eq!(link.pending(), 0);
        link.up(&lease, TransitionOwner::Control, &mut host)
            .unwrap();
        assert_eq!(link.complete(ota), Err(LinkError::Stale));
        let fresh = link.submit(Kind::Status).unwrap();
        assert_eq!(link.complete(fresh), Ok(Kind::Status));
    }

    #[test]
    fn wifi_toggles_never_reset_the_shared_sd_host_or_its_media_lease() {
        let mut media = MediaSession::new();
        let mut host = SharedSdmmcHost::new();
        assert_eq!(host.attach(SdmmcSlot::SdCard), HostAction::InitHost);
        let ConnectResult::Accepted(lease) = media.on_connect(MediaSourceId::new(1).unwrap())
        else {
            panic!()
        };

        let mut transition = TransitionLease::new();
        transition.acquire(TransitionOwner::Control);
        let mut link = C6Link::<Kind, 2>::new();
        for _ in 0..3 {
            assert_eq!(
                link.up(&transition, TransitionOwner::Control, &mut host),
                Ok(HostAction::None)
            );
            assert_eq!(
                link.down(&transition, TransitionOwner::Control, &mut host, |_| {}),
                Ok(HostAction::None)
            );
            assert!(host.attached(SdmmcSlot::SdCard));
            assert!(
                media.validate(lease),
                "SD media lease survived Wi-Fi off/on"
            );
        }

        // Without the SD card the C6 alone owns the host lifecycle.
        assert_eq!(host.detach(SdmmcSlot::SdCard), HostAction::DeinitHost);
        assert_eq!(
            link.up(&transition, TransitionOwner::Control, &mut host),
            Ok(HostAction::InitHost)
        );
        assert_eq!(host.detach(SdmmcSlot::SdCard), HostAction::None);
        assert_eq!(
            link.down(&transition, TransitionOwner::Control, &mut host, |_| {}),
            Ok(HostAction::DeinitHost)
        );
    }
}
