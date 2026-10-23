//! Owner observations shared by target wiring and the composed host harness.
use crate::handoff::Producer;
use crate::handoff::{OwnerChannel, OwnerSample, OwnerState, OwnerView, ProductObservation};
use core::fmt::{self, Write};
use ruzina_control_scheduler::SchedulerStats;
use ruzina_controller_session::{ControllerSession, SessionStats};
use ruzina_media_session::{MediaHandle, MediaLease, MediaSession};

// Software progress contracts, not measured hardware latency limits.
pub const CONTROL_MAX_AGE_MS: u64 = 2000;
pub const MEDIA_MAX_AGE_MS: u64 = 5000;
pub const OWNER_TICK_MS: u64 = 250;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControllerObservation {
    pub connected: bool,
    pub failed: bool,
    pub backpressured: bool,
    pub counters: SessionStats,
    pub scheduler: SchedulerStats,
}
impl ControllerObservation {
    pub fn capture(owner: &ControllerSession, backpressured: bool) -> Self {
        Self {
            connected: owner.connected(),
            failed: false,
            backpressured,
            counters: owner.stats(),
            scheduler: owner.scheduler_stats(),
        }
    }
}
impl OwnerSample for ControllerObservation {
    fn state(self) -> OwnerState {
        if self.failed {
            OwnerState::Failed
        } else if !self.connected {
            OwnerState::Disconnected
        } else if self.backpressured {
            OwnerState::Backpressured
        } else {
            OwnerState::Idle
        }
    }
    fn errors(self) -> u64 {
        u64::from(self.counters.semantic_adapter_errors)
            + u64::from(self.scheduler.fifo_full)
            + u64::from(self.scheduler.continuous_slot_full)
            + u64::from(self.scheduler.jog_saturated)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaPhase {
    Idle,
    Discovering,
    Mounted,
    Failed,
    Detached,
    Superseded,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaObservation {
    pub lease: Option<MediaLease>,
    pub handle: Option<MediaHandle>,
    pub mounted: bool,
    pub phase: MediaPhase,
    pub failures: u32,
}
impl MediaObservation {
    pub fn capture(owner: &MediaSession, phase: MediaPhase, failures: u32) -> Self {
        Self {
            lease: owner.lease(),
            handle: owner.handle(),
            mounted: owner.is_mounted(),
            phase,
            failures,
        }
    }
}
impl OwnerSample for MediaObservation {
    fn state(self) -> OwnerState {
        if matches!(self.phase, MediaPhase::Failed | MediaPhase::Superseded) {
            OwnerState::Failed
        } else if self.lease.is_none() {
            OwnerState::Disconnected
        } else if self.phase == MediaPhase::Discovering {
            OwnerState::Active
        } else {
            OwnerState::Idle
        }
    }
    fn errors(self) -> u64 {
        u64::from(self.failures)
    }
}

pub struct RuntimeOwners {
    pub product: OwnerChannel<ProductObservation>,
    pub controller: OwnerChannel<ControllerObservation>,
    pub media: MediaHandoff,
}
impl RuntimeOwners {
    pub const fn new() -> Self {
        Self {
            product: OwnerChannel::new(CONTROL_MAX_AGE_MS),
            controller: OwnerChannel::new(CONTROL_MAX_AGE_MS),
            media: MediaHandoff::new(),
        }
    }
    /// Each channel receives its own fixed budget, preventing cross-owner starvation.
    pub fn observe(&mut self, now_ms: u64) -> RuntimeView {
        self.product.drain(now_ms);
        self.controller.drain(now_ms);
        self.media.channel.drain(now_ms);
        self.view(now_ms)
    }
    /// Reading never drains, renews, or performs owner work.
    pub fn view(&self, now_ms: u64) -> RuntimeView {
        RuntimeView {
            analysis: crate::analysis::Observation::DISABLED,
            elapsed_ms: now_ms,
            product: self.product.view(now_ms),
            controller: self.controller.view(now_ms),
            media: self.media.channel.view(now_ms),
        }
    }
}
impl Default for RuntimeOwners {
    fn default() -> Self {
        Self::new()
    }
}

struct PendingMedia {
    key: (MediaLease, MediaHandle),
    detached: bool,
    producer: Producer,
}
/// Coordinator-to-worker token transfer shared with target wiring. Ready and
/// Detached for the same binding cannot claim each other's producer token.
pub struct MediaHandoff {
    pub channel: OwnerChannel<MediaObservation>,
    pending: Option<PendingMedia>,
    started: bool,
}
impl MediaHandoff {
    pub const fn new() -> Self {
        Self {
            channel: OwnerChannel::new(MEDIA_MAX_AGE_MS),
            pending: None,
            started: false,
        }
    }
    pub fn initial(&mut self) -> Option<Producer> {
        if self.started {
            None
        } else {
            self.started = true;
            self.channel.restart()
        }
    }
    pub fn invalidate(&mut self, lease: MediaLease, handle: MediaHandle, detached: bool) {
        self.pending = self.channel.restart().map(|producer| PendingMedia {
            key: (lease, handle),
            detached,
            producer,
        });
        self.started = true;
    }
    pub fn claim(
        &mut self,
        lease: MediaLease,
        handle: MediaHandle,
        detached: bool,
    ) -> Option<Producer> {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.key == (lease, handle) && p.detached == detached)
        {
            self.pending.take().map(|p| p.producer)
        } else {
            None
        }
    }
}
impl Default for MediaHandoff {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Clone, Copy, Debug)]
pub struct RuntimeView {
    pub analysis: crate::analysis::Observation,
    pub elapsed_ms: u64,
    pub product: OwnerView<ProductObservation>,
    pub controller: OwnerView<ControllerObservation>,
    pub media: OwnerView<MediaObservation>,
}
impl RuntimeView {
    /// Versioned owner report is separate from the immutable boot snapshot.
    /// Discard the whole buffer if formatting fails.
    pub fn write_to(self, output: &mut impl Write) -> fmt::Result {
        writeln!(output, "M1_OWNER_HEALTH_V1")?;
        writeln!(output, "observer_elapsed_ms={}", self.elapsed_ms)?;
        writeln!(output, "product={:?}", self.product)?;
        writeln!(output, "controller={:?}", self.controller)?;
        writeln!(output, "media={:?}", self.media)?;
        writeln!(output, "readiness=NOT_QUALIFIED")?;
        self.analysis.write_to(output)
    }
}
