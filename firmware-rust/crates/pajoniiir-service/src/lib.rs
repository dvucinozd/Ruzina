//! Bounded service observations. Health describes the supplied runtime scope,
//! never physical/product acceptance. No allocator, HAL, clock or UI dependency.
#![no_std]
#![forbid(unsafe_code)]

use core::fmt::{self, Write};
use pajoniiir_board_profile::{BoardId, profile};
use pajoniiir_core::DeckId;

#[cfg(feature = "owner-adapters")]
pub mod adapters;
pub mod analysis;
pub mod handoff;
#[cfg(feature = "runtime-owners")]
pub mod owners;
pub mod publication;
use publication::{Freshness, Observation, Slot};

pub const EVENT_CAPACITY: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Environment {
    HostSimulation,
    DeviceBootSnapshot,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityCheck {
    NotMeasured,
    Partial,
    MatchesDocumented,
    Mismatch,
}
#[derive(Clone, Copy, Debug)]
pub struct Identity {
    pub environment: Environment,
    pub board: BoardId,
    pub image_id: Option<&'static str>,
    pub chip_revision: Option<u16>,
    pub flash_bytes: Option<u32>,
    pub psram_bytes: Option<u32>,
    /// HAL raw reset code on a device; absent in a host process.
    pub reset_code: Option<u32>,
}
impl Identity {
    pub const fn host(board: BoardId, image_id: Option<&'static str>) -> Self {
        Self {
            environment: Environment::HostSimulation,
            board,
            image_id,
            chip_revision: None,
            flash_bytes: None,
            psram_bytes: None,
            reset_code: None,
        }
    }
    pub fn image_label(&self) -> &'static str {
        match self.image_id {
            None | Some("") => "NOT_RECORDED",
            Some(id) if id.len() <= 64 && id.bytes().all(|b| b.is_ascii_graphic()) => id,
            Some(_) => "INVALID_ID",
        }
    }
    pub fn check(&self) -> IdentityCheck {
        let expected = profile(self.board);
        if self
            .chip_revision
            .is_some_and(|r| r < expected.min_chip_revision)
            || self.flash_bytes.is_some_and(|b| b != expected.flash_bytes)
            || self.psram_bytes.is_some_and(|b| b != expected.psram_bytes)
        {
            return IdentityCheck::Mismatch;
        }
        match (self.chip_revision, self.flash_bytes, self.psram_bytes) {
            (Some(_), Some(_), Some(_)) => IdentityCheck::MatchesDocumented,
            (None, None, None) => IdentityCheck::NotMeasured,
            _ => IdentityCheck::Partial,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaStatus {
    NotStarted,
    Connected,
    Absent,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisStatus {
    Unloaded,
    Ready,
    Missing,
    Rejected,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeStatus {
    pub media: MediaStatus,
    pub analysis: [AnalysisStatus; 2],
}
impl RuntimeStatus {
    pub const BOOT_ONLY: Self = Self {
        media: MediaStatus::NotStarted,
        analysis: [AnalysisStatus::Unloaded; 2],
    };
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Health {
    RuntimeStale,
    RuntimeAwaiting,
    BootOnly,
    DemoMediaReady,
    AnalysisFallback,
    MediaUnavailable,
    IdentityMismatch,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadBlock {
    Playing,
    /// A PLAY request awaits transport confirmation (load lock on).
    PlayPending,
    /// The jog holds the destination deck for scratch audio (load lock on).
    Scratching,
    /// Another accepted load still owns the deck's single-flight permit.
    LoadInFlight,
    /// PLAY or scratch claimed the deck after admission, before replacement.
    Superseded,
    NoMedia,
    InvalidSelection,
    TrackIdentity,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    Boot,
    MediaConnected,
    MediaRemoved,
    LoadAttempt(DeckId),
    Loaded {
        deck: DeckId,
        analysis: AnalysisStatus,
    },
    LoadBlocked {
        deck: DeckId,
        reason: LoadBlock,
    },
    PlaybackRejected(DeckId),
    EndOfTrack(DeckId),
}
impl EventKind {
    pub const fn warning(self) -> bool {
        matches!(
            self,
            Self::LoadBlocked { .. }
                | Self::PlaybackRejected(_)
                | Self::Loaded {
                    analysis: AnalysisStatus::Rejected,
                    ..
                }
        )
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Event {
    pub sequence: u64,
    pub elapsed_ms: u64,
    pub kind: EventKind,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Counters {
    pub media_connects: u32,
    pub media_removals: u32,
    pub load_attempts: u32,
    pub loads_completed: u32,
    pub loads_blocked: u32,
    pub analysis_ready: u32,
    pub analysis_missing: u32,
    pub analysis_rejected: u32,
    pub playback_rejected: u32,
    pub end_of_track: u32,
    /// Delivered semantic input events, including releases/no-op values.
    pub control_events: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct EventLog {
    events: [Option<Event>; EVENT_CAPACITY],
    head: usize,
    len: usize,
    sequence: u64,
    pub overwritten: u64,
}
impl EventLog {
    const fn new() -> Self {
        Self {
            events: [None; EVENT_CAPACITY],
            head: 0,
            len: 0,
            sequence: 0,
            overwritten: 0,
        }
    }
    fn push(&mut self, elapsed_ms: u64, kind: EventKind) {
        self.sequence = self.sequence.saturating_add(1);
        let event = Some(Event {
            sequence: self.sequence,
            elapsed_ms,
            kind,
        });
        if self.len == EVENT_CAPACITY {
            self.events[self.head] = event;
            self.head = (self.head + 1) % EVENT_CAPACITY;
            self.overwritten = self.overwritten.saturating_add(1);
        } else {
            self.events[(self.head + self.len) % EVENT_CAPACITY] = event;
            self.len += 1;
        }
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn get(&self, index: usize) -> Option<Event> {
        if index >= self.len {
            None
        } else {
            self.events[(self.head + index) % EVENT_CAPACITY]
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub identity: Identity,
    pub elapsed_ms: u64,
    pub counters: Counters,
    pub runtime: RuntimeStatus,
    /// None is an explicitly synchronous/boot snapshot, not a heartbeat.
    pub observation: Option<Observation>,
    pub events: EventLog,
}
impl Snapshot {
    pub fn health(&self) -> Health {
        if self.identity.check() == IdentityCheck::Mismatch {
            return Health::IdentityMismatch;
        }
        if self.identity.environment != Environment::DeviceBootSnapshot {
            match self.observation.map(|o| o.freshness) {
                Some(Freshness::Stale | Freshness::ClockReversed) => return Health::RuntimeStale,
                Some(Freshness::Unbound | Freshness::Awaiting) => return Health::RuntimeAwaiting,
                _ => {}
            }
        }
        if self.identity.environment == Environment::DeviceBootSnapshot
            || self.runtime.media == MediaStatus::NotStarted
        {
            return Health::BootOnly;
        }
        if self.runtime.media == MediaStatus::Absent {
            return Health::MediaUnavailable;
        }
        if self
            .runtime
            .analysis
            .iter()
            .any(|a| matches!(a, AnalysisStatus::Missing | AnalysisStatus::Rejected))
        {
            return Health::AnalysisFallback;
        }
        Health::DemoMediaReady
    }
    /// Caller must discard the entire buffer on error, never emit a partial record.
    pub fn write_to(&self, output: &mut impl Write) -> fmt::Result {
        let board = profile(self.identity.board);
        writeln!(output, "M1_SERVICE_DIAGNOSTIC_V2")?;
        writeln!(output, "environment={:?}", self.identity.environment)?;
        writeln!(output, "board_profile={}", board.model)?;
        writeln!(output, "image_id={}", self.identity.image_label())?;
        writeln!(
            output,
            "minimum_revision_documented={}",
            board.min_chip_revision
        )?;
        writeln!(output, "flash_bytes_documented={}", board.flash_bytes)?;
        writeln!(output, "psram_bytes_documented={}", board.psram_bytes)?;
        for (name, value) in [
            (
                "chip_revision_observed",
                self.identity.chip_revision.map(u32::from),
            ),
            ("flash_bytes_observed", self.identity.flash_bytes),
            ("psram_bytes_observed", self.identity.psram_bytes),
            ("reset_code_observed", self.identity.reset_code),
        ] {
            match value {
                Some(v) => writeln!(output, "{name}={v}")?,
                None => writeln!(output, "{name}=NOT_MEASURED")?,
            }
        }
        writeln!(output, "identity_check={:?}", self.identity.check())?;
        writeln!(output, "sample_elapsed_ms={}", self.elapsed_ms)?;
        writeln!(output, "runtime_publication={:?}", self.observation)?;
        writeln!(output, "health_scope={:?}", self.health())?;
        writeln!(output, "media={:?}", self.runtime.media)?;
        writeln!(output, "analysis={:?}", self.runtime.analysis)?;
        writeln!(output, "counters={:?}", self.counters)?;
        writeln!(output, "events_retained={}", self.events.len())?;
        writeln!(output, "events_overwritten={}", self.events.overwritten)?;
        for index in 0..self.events.len() {
            let event = self.events.get(index).unwrap();
            writeln!(
                output,
                "event={} t={} {:?}",
                event.sequence, event.elapsed_ms, event.kind
            )?;
        }
        writeln!(output, "readiness=NOT_QUALIFIED")
    }
}

/// Observations only. The product owner supplies current runtime state when
/// publishing; history cannot resurrect old faults or authorize operations.
pub struct Service {
    identity: Identity,
    elapsed_ms: u64,
    counters: Counters,
    events: EventLog,
}
impl Service {
    pub fn new(identity: Identity) -> Self {
        let mut result = Self {
            identity,
            elapsed_ms: 0,
            counters: Counters::default(),
            events: EventLog::new(),
        };
        result.record(EventKind::Boot);
        result
    }
    pub fn advance(&mut self, elapsed_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(elapsed_ms);
    }
    pub const fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }
    pub fn observed_snapshot(&self, slot: &Slot<RuntimeStatus>) -> Snapshot {
        let mut snapshot =
            self.snapshot(slot.latest().map_or(RuntimeStatus::BOOT_ONLY, |s| s.value));
        snapshot.observation = Some(slot.observation(self.elapsed_ms));
        snapshot
    }
    pub fn note_control(&mut self) {
        self.counters.control_events = self.counters.control_events.saturating_add(1);
    }
    pub fn record(&mut self, kind: EventKind) {
        let count = match kind {
            EventKind::Boot => None,
            EventKind::MediaConnected => Some(&mut self.counters.media_connects),
            EventKind::MediaRemoved => Some(&mut self.counters.media_removals),
            EventKind::LoadAttempt(_) => Some(&mut self.counters.load_attempts),
            EventKind::LoadBlocked { .. } => Some(&mut self.counters.loads_blocked),
            EventKind::PlaybackRejected(_) => Some(&mut self.counters.playback_rejected),
            EventKind::EndOfTrack(_) => Some(&mut self.counters.end_of_track),
            EventKind::Loaded { analysis, .. } => {
                self.counters.loads_completed = self.counters.loads_completed.saturating_add(1);
                match analysis {
                    AnalysisStatus::Ready => Some(&mut self.counters.analysis_ready),
                    AnalysisStatus::Missing => Some(&mut self.counters.analysis_missing),
                    AnalysisStatus::Rejected => Some(&mut self.counters.analysis_rejected),
                    AnalysisStatus::Unloaded => None,
                }
            }
        };
        if let Some(count) = count {
            *count = count.saturating_add(1);
        }
        self.events.push(self.elapsed_ms, kind);
    }
    pub const fn snapshot(&self, runtime: RuntimeStatus) -> Snapshot {
        Snapshot {
            identity: self.identity,
            elapsed_ms: self.elapsed_ms,
            counters: self.counters,
            runtime,
            observation: None,
            events: self.events,
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use pajoniiir_board_profile::diagnostics::ReportBuffer;
    const BOARD: BoardId = BoardId::GuitionJc1060P470cIwY;
    #[test]
    fn expected_values_do_not_become_observations_or_qualification() {
        let mut id = Identity::host(BOARD, None);
        assert_eq!(id.check(), IdentityCheck::NotMeasured);
        assert_eq!(id.image_label(), "NOT_RECORDED");
        id.chip_revision = Some(302);
        assert_eq!(id.check(), IdentityCheck::Partial);
        id.flash_bytes = Some(16 * 1024 * 1024);
        id.psram_bytes = Some(32 * 1024 * 1024);
        assert_eq!(id.check(), IdentityCheck::MatchesDocumented);
        id.environment = Environment::DeviceBootSnapshot;
        let snapshot = Service::new(id).snapshot(RuntimeStatus::BOOT_ONLY);
        assert_eq!(snapshot.health(), Health::BootOnly);
        let mut text = std::string::String::new();
        snapshot.write_to(&mut text).unwrap();
        assert!(text.ends_with("readiness=NOT_QUALIFIED\n"));
        id.chip_revision = Some(301);
        assert_eq!(
            Service::new(id).snapshot(RuntimeStatus::BOOT_ONLY).health(),
            Health::IdentityMismatch
        );
    }
    #[test]
    fn current_state_recovers_while_fault_history_remains() {
        let mut service = Service::new(Identity::host(BOARD, Some("test")));
        service.record(EventKind::Loaded {
            deck: DeckId::One,
            analysis: AnalysisStatus::Rejected,
        });
        let bad = RuntimeStatus {
            media: MediaStatus::Connected,
            analysis: [AnalysisStatus::Rejected, AnalysisStatus::Unloaded],
        };
        assert_eq!(service.snapshot(bad).health(), Health::AnalysisFallback);
        service.record(EventKind::MediaRemoved);
        service.record(EventKind::MediaConnected);
        let good = RuntimeStatus {
            media: MediaStatus::Connected,
            analysis: [AnalysisStatus::Unloaded; 2],
        };
        let snapshot = service.snapshot(good);
        assert_eq!(snapshot.health(), Health::DemoMediaReady);
        assert_eq!(snapshot.counters.analysis_rejected, 1);
        assert!(snapshot.events.get(1).unwrap().kind.warning());
    }
    #[test]
    fn bounded_log_keeps_chronological_tail_and_saturates_observations() {
        let mut service = Service::new(Identity::host(BOARD, None));
        for _ in 0..20 {
            service.advance(5);
            service.record(EventKind::LoadAttempt(DeckId::Two));
        }
        service.counters.control_events = u32::MAX;
        service.note_control();
        service.advance(u64::MAX);
        let s = service.snapshot(RuntimeStatus::BOOT_ONLY);
        assert_eq!(s.events.len(), 16);
        assert_eq!(s.events.overwritten, 5);
        assert_eq!(s.events.get(0).unwrap().sequence, 6);
        assert_eq!(s.events.get(15).unwrap().elapsed_ms, 100);
        assert!(s.events.get(16).is_none());
        assert_eq!(s.counters.control_events, u32::MAX);
        assert_eq!(s.elapsed_ms, u64::MAX);
        assert!(core::mem::size_of::<Snapshot>() <= 2048);
    }
    #[test]
    fn report_is_bounded_rejects_invalid_identity_and_preserves_unknown_fields() {
        let mut id = Identity::host(BOARD, Some("bad\nimage_id=fake"));
        assert_eq!(id.image_label(), "INVALID_ID");
        id.environment = Environment::DeviceBootSnapshot;
        id.chip_revision = Some(302);
        id.reset_code = Some(1);
        let mut service = Service::new(id);
        for _ in 0..20 {
            service.record(EventKind::LoadBlocked {
                deck: DeckId::Two,
                reason: LoadBlock::Playing,
            });
        }
        let s = service.snapshot(RuntimeStatus::BOOT_ONLY);
        let mut report = ReportBuffer::<4096>::default();
        s.write_to(&mut report).unwrap();
        let text = core::str::from_utf8(report.bytes()).unwrap();
        assert!(text.contains("image_id=INVALID_ID\n"));
        assert!(text.contains("flash_bytes_observed=NOT_MEASURED\n"));
        assert!(text.contains("reset_code_observed=1\n"));
        assert!(s.write_to(&mut ReportBuffer::<16>::default()).is_err());
    }
    #[test]
    fn combined_legacy_and_service_boot_report_fits_device_buffer() {
        let id = Identity {
            environment: Environment::DeviceBootSnapshot,
            chip_revision: Some(302),
            reset_code: Some(u32::MAX),
            ..Identity::host(
                BOARD,
                Some("0123456789012345678901234567890123456789012345678901234567890123"),
            )
        };
        let mut buffer = ReportBuffer::<2048>::default();
        let boot = pajoniiir_board_profile::diagnostics::BootReport {
            profile: profile(BOARD),
            chip_revision: 302,
            image_id: id.image_label(),
            observed_flash_bytes: None,
            observed_psram_bytes: None,
        };
        boot.write_to(&mut buffer).unwrap();
        writeln!(buffer, "reset_reason=Some(SysSuperWdt)").unwrap();
        Service::new(id)
            .snapshot(RuntimeStatus::BOOT_ONLY)
            .write_to(&mut buffer)
            .unwrap();
        let text = core::str::from_utf8(buffer.bytes()).unwrap();
        assert!(text.starts_with("M1_BOOT_DIAGNOSTIC_V1\n"));
        assert!(text.contains("M1_SERVICE_DIAGNOSTIC_V2\n"));
    }
}
