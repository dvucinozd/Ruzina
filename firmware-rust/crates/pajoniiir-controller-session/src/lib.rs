#![no_std]
#![forbid(unsafe_code)]

use pajoniiir_control_scheduler::{
    ConnectionDelivery, ConnectionReconciler, ConnectionState, ContinuousMode, EventScheduler,
    HeldStateReconciler, SchedulerStats,
};
use pajoniiir_controller_core::{ControlEvent, ControlValue, SemanticControl};
use pajoniiir_controller_profile::{
    MAX_INPUTS, Profile, ProfileRuntime, SemanticAdapterError, adapt_profile_event,
};
use pajoniiir_controller_usb::UsbMidiEvent;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngestOutcome {
    NoMapping,
    Scheduled,
    Dropped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbMidiIngestOutcome {
    IgnoredNonChannelVoice,
    UnsupportedCable(u8),
    Session(IngestOutcome),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UsbMidiBatchReport {
    pub packets: u32,
    pub decoded: u32,
    pub scheduled: u32,
    pub dropped: u32,
    pub no_mapping: u32,
    pub ignored_non_channel_voice: u32,
    pub unsupported_cable: u32,
    pub semantic_errors: u32,
}

impl UsbMidiBatchReport {
    pub fn accumulate(&mut self, other: Self) {
        self.packets = self.packets.saturating_add(other.packets);
        self.decoded = self.decoded.saturating_add(other.decoded);
        self.scheduled = self.scheduled.saturating_add(other.scheduled);
        self.dropped = self.dropped.saturating_add(other.dropped);
        self.no_mapping = self.no_mapping.saturating_add(other.no_mapping);
        self.ignored_non_channel_voice = self
            .ignored_non_channel_voice
            .saturating_add(other.ignored_non_channel_voice);
        self.unsupported_cable = self.unsupported_cable.saturating_add(other.unsupported_cable);
        self.semantic_errors = self.semantic_errors.saturating_add(other.semantic_errors);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbMidiBatchError {
    MisalignedTransferLength(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionDelivery {
    Connection(ConnectionState),
    Control(ControlEvent),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionStats {
    pub midi_messages: u32,
    pub mapped_messages: u32,
    pub semantic_adapter_errors: u32,
    pub reconnect_snapshots: u32,
    pub snapshot_events: u32,
    pub held_reconciliations: u32,
    pub downstream_retries: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InFlight {
    Connection(ConnectionDelivery),
    Held { key: usize, value: ControlValue },
    Retry,
    Snapshot,
    Buffered(ControlEvent),
}

pub struct ControllerSession {
    profile_runtime: ProfileRuntime,
    scheduler: EventScheduler,
    held: HeldStateReconciler,
    connection: ConnectionReconciler,
    snapshot: [Option<ControlEvent>; MAX_INPUTS],
    snapshot_count: usize,
    snapshot_cursor: usize,
    retry: Option<ControlEvent>,
    in_flight: Option<InFlight>,
    sequence: u8,
    stats: SessionStats,
}

impl Default for ControllerSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ControllerSession {
    pub const fn new() -> Self {
        Self {
            profile_runtime: ProfileRuntime::new(),
            scheduler: EventScheduler::new(),
            held: HeldStateReconciler::new(),
            connection: ConnectionReconciler::new(),
            snapshot: [None; MAX_INPUTS],
            snapshot_count: 0,
            snapshot_cursor: 0,
            retry: None,
            in_flight: None,
            sequence: 0,
            stats: SessionStats {
                midi_messages: 0,
                mapped_messages: 0,
                semantic_adapter_errors: 0,
                reconnect_snapshots: 0,
                snapshot_events: 0,
                held_reconciliations: 0,
                downstream_retries: 0,
            },
        }
    }

    pub fn reset_profile_state(&mut self) {
        self.profile_runtime.reset();
        self.snapshot_count = 0;
        self.snapshot_cursor = 0;
    }

    pub fn set_connected(&mut self, connected: bool) {
        let sequence = self.take_sequence();
        self.connection
            .set_connected(connected, &mut self.held, sequence);
        if !connected {
            self.snapshot_count = 0;
            self.snapshot_cursor = 0;
        }
    }

    pub const fn connected(&self) -> bool {
        self.connection.connected()
    }

    pub const fn scheduler_stats(&self) -> SchedulerStats {
        self.scheduler.stats()
    }

    pub const fn stats(&self) -> SessionStats {
        self.stats
    }

    pub fn process_usb_midi_event(
        &mut self,
        profile: &Profile<'_>,
        event: UsbMidiEvent,
    ) -> Result<UsbMidiIngestOutcome, SemanticAdapterError> {
        if event.cable != 0 {
            return Ok(UsbMidiIngestOutcome::UnsupportedCable(event.cable));
        }

        let Some(message) = event.channel_voice() else {
            return Ok(UsbMidiIngestOutcome::IgnoredNonChannelVoice);
        };

        self.process_midi(
            profile,
            message.status,
            message.data1,
            message.data2.unwrap_or(0),
        )
        .map(UsbMidiIngestOutcome::Session)
    }

    pub fn process_usb_midi_transfer(
        &mut self,
        profile: &Profile<'_>,
        bytes: &[u8],
    ) -> Result<UsbMidiBatchReport, UsbMidiBatchError> {
        if !bytes.len().is_multiple_of(4) {
            return Err(UsbMidiBatchError::MisalignedTransferLength(bytes.len()));
        }

        let mut report = UsbMidiBatchReport::default();
        for packet in bytes.chunks_exact(4) {
            report.packets = report.packets.saturating_add(1);
            let Some(event) =
                UsbMidiEvent::decode([packet[0], packet[1], packet[2], packet[3]])
            else {
                continue;
            };
            report.decoded = report.decoded.saturating_add(1);

            match self.process_usb_midi_event(profile, event) {
                Ok(UsbMidiIngestOutcome::IgnoredNonChannelVoice) => {
                    report.ignored_non_channel_voice =
                        report.ignored_non_channel_voice.saturating_add(1);
                }
                Ok(UsbMidiIngestOutcome::UnsupportedCable(_)) => {
                    report.unsupported_cable = report.unsupported_cable.saturating_add(1);
                }
                Ok(UsbMidiIngestOutcome::Session(IngestOutcome::NoMapping)) => {
                    report.no_mapping = report.no_mapping.saturating_add(1);
                }
                Ok(UsbMidiIngestOutcome::Session(IngestOutcome::Scheduled)) => {
                    report.scheduled = report.scheduled.saturating_add(1);
                }
                Ok(UsbMidiIngestOutcome::Session(IngestOutcome::Dropped)) => {
                    report.dropped = report.dropped.saturating_add(1);
                }
                Err(_) => {
                    report.semantic_errors = report.semantic_errors.saturating_add(1);
                }
            }
        }

        Ok(report)
    }

    pub fn process_midi(
        &mut self,
        profile: &Profile<'_>,
        status: u8,
        data1: u8,
        data2: u8,
    ) -> Result<IngestOutcome, SemanticAdapterError> {
        self.stats.midi_messages = self.stats.midi_messages.saturating_add(1);
        let Some(profile_event) = self.profile_runtime.process(profile, status, data1, data2)
        else {
            return Ok(IngestOutcome::NoMapping);
        };
        self.stats.mapped_messages = self.stats.mapped_messages.saturating_add(1);

        let event = match adapt_profile_event(profile_event) {
            Ok(event) => event,
            Err(error) => {
                self.stats.semantic_adapter_errors =
                    self.stats.semantic_adapter_errors.saturating_add(1);
                return Err(error);
            }
        };
        Ok(self.schedule_event(event))
    }

    pub fn schedule_event(&mut self, event: ControlEvent) -> IngestOutcome {
        let sequence = self.take_sequence();
        if self.held.observe(event, sequence).is_some() {
            return IngestOutcome::Scheduled;
        }

        let scheduled = match continuous_mode(event) {
            Some(mode) => self.scheduler.publish_continuous(event, mode),
            None => self.scheduler.enqueue_discrete(event),
        };

        if scheduled {
            IngestOutcome::Scheduled
        } else {
            IngestOutcome::Dropped
        }
    }

    pub fn next_delivery(&mut self, profile: Option<&Profile<'_>>) -> Option<SessionDelivery> {
        if self.in_flight.is_some() {
            return None;
        }

        if let Some(delivery) = self.connection.pending_delivery() {
            self.in_flight = Some(InFlight::Connection(delivery));
            return Some(SessionDelivery::Connection(delivery.state));
        }

        self.prepare_snapshot_if_requested(profile);

        let mut cursor = 0;
        if let Some(dirty) = self.held.next_dirty(&mut cursor) {
            self.in_flight = Some(InFlight::Held {
                key: dirty.key,
                value: dirty.event.value,
            });
            return Some(SessionDelivery::Control(dirty.event));
        }

        if let Some(event) = self.retry {
            self.in_flight = Some(InFlight::Retry);
            return Some(SessionDelivery::Control(event));
        }

        if self.snapshot_cursor < self.snapshot_count {
            let event = self.snapshot[self.snapshot_cursor]?;
            self.in_flight = Some(InFlight::Snapshot);
            return Some(SessionDelivery::Control(event));
        }

        if let Some(event) = self.scheduler.dequeue_discrete() {
            self.in_flight = Some(InFlight::Buffered(event));
            return Some(SessionDelivery::Control(event));
        }

        if let Some(event) = self.scheduler.take_continuous() {
            self.in_flight = Some(InFlight::Buffered(event));
            return Some(SessionDelivery::Control(event));
        }

        None
    }

    pub fn complete_delivery(&mut self, succeeded: bool) {
        let Some(in_flight) = self.in_flight.take() else {
            return;
        };

        match in_flight {
            InFlight::Connection(delivery) => {
                if succeeded {
                    self.connection.mark_delivered(delivery);
                }
            }
            InFlight::Held { key, value } => {
                if succeeded {
                    let _ = self.held.mark_scheduled(key, value);
                    self.stats.held_reconciliations =
                        self.stats.held_reconciliations.saturating_add(1);
                }
            }
            InFlight::Retry => {
                if succeeded {
                    self.retry = None;
                }
            }
            InFlight::Snapshot => {
                if succeeded {
                    self.snapshot_cursor += 1;
                    if self.snapshot_cursor == self.snapshot_count {
                        self.clear_snapshot();
                    }
                }
            }
            InFlight::Buffered(event) => {
                if !succeeded {
                    self.retry = Some(event);
                    self.stats.downstream_retries = self.stats.downstream_retries.saturating_add(1);
                }
            }
        }
    }

    fn prepare_snapshot_if_requested(&mut self, profile: Option<&Profile<'_>>) {
        if self.snapshot_cursor < self.snapshot_count {
            return;
        }
        let Some(profile) = profile else {
            return;
        };
        if !self.connection.take_snapshot_request() {
            return;
        }

        self.clear_snapshot();

        let runtime = &self.profile_runtime;
        let snapshot = &mut self.snapshot;
        let mut count = 0usize;
        let mut adapter_errors = 0u32;

        runtime.emit_snapshot(profile, |profile_event| {
            match adapt_profile_event(profile_event) {
                Ok(event) if count < snapshot.len() => {
                    snapshot[count] = Some(event);
                    count += 1;
                    true
                }
                Ok(_) => false,
                Err(_) => {
                    adapter_errors = adapter_errors.saturating_add(1);
                    true
                }
            }
        });

        self.snapshot_count = count;
        self.stats.semantic_adapter_errors = self
            .stats
            .semantic_adapter_errors
            .saturating_add(adapter_errors);
        self.stats.reconnect_snapshots = self.stats.reconnect_snapshots.saturating_add(1);
        self.stats.snapshot_events = self.stats.snapshot_events.saturating_add(count as u32);
    }

    fn clear_snapshot(&mut self) {
        for slot in &mut self.snapshot[..self.snapshot_count] {
            *slot = None;
        }
        self.snapshot_count = 0;
        self.snapshot_cursor = 0;
    }

    fn take_sequence(&mut self) -> u8 {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        sequence
    }
}

fn continuous_mode(event: ControlEvent) -> Option<ContinuousMode> {
    match event.control {
        SemanticControl::JogScratch | SemanticControl::JogBend | SemanticControl::JogSearch => {
            Some(ContinuousMode::AccumulateDelta)
        }
        SemanticControl::LoopSize
        | SemanticControl::Tempo
        | SemanticControl::ChannelVolume
        | SemanticControl::Crossfader
        | SemanticControl::Trim
        | SemanticControl::EqHigh
        | SemanticControl::EqMid
        | SemanticControl::EqLow
        | SemanticControl::Filter
        | SemanticControl::MasterVolume
        | SemanticControl::HeadphoneMix
        | SemanticControl::HeadphoneLevel
        | SemanticControl::BeatFxDepth => Some(ContinuousMode::LatestValue),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pajoniiir_controller_core::{DeckId, SemanticControl};
    use pajoniiir_controller_profile::{
        HEADER_SIZE, INPUT_ENTRY_SIZE, INPUT_FLAG_REPLAY, S3CP_MAGIC, S3CP_VERSION, crc32,
    };

    fn pressed(deck: DeckId, control: SemanticControl, down: bool) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control,
            value: ControlValue::Pressed(down),
        }
    }

    fn relative(deck: DeckId, control: SemanticControl, value: i16) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control,
            value: ControlValue::Relative(value),
        }
    }

    fn absolute(control: SemanticControl, value: u16, max: u16) -> ControlEvent {
        ControlEvent {
            deck: None,
            control,
            value: ControlValue::Absolute { value, max },
        }
    }

    fn replay_profile_bytes() -> [u8; HEADER_SIZE + INPUT_ENTRY_SIZE] {
        let mut bytes = [0u8; HEADER_SIZE + INPUT_ENTRY_SIZE];
        bytes[0..4].copy_from_slice(S3CP_MAGIC);
        bytes[4..6].copy_from_slice(&S3CP_VERSION.to_le_bytes());
        bytes[6..8].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
        let profile_size = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&profile_size.to_le_bytes());
        bytes[16..18].copy_from_slice(&0x2b73u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&0x0030u16.to_le_bytes());
        bytes[24..26].copy_from_slice(&1u16.to_le_bytes());
        bytes[26..28].copy_from_slice(&0u16.to_le_bytes());
        bytes[29] = 2;

        let input = HEADER_SIZE;
        bytes[input] = 0xb0;
        bytes[input + 1] = 0x10;
        bytes[input + 2] = 6;
        bytes[input + 3] = 0xff;
        bytes[input + 4] = 3;
        bytes[input + 5] = 0x52;
        bytes[input + 6..input + 8].copy_from_slice(&INPUT_FLAG_REPLAY.to_le_bytes());

        let checksum = crc32(&bytes[16..]);
        bytes[12..16].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    #[test]
    fn discrete_events_preserve_order() {
        let mut session = ControllerSession::new();
        let play = pressed(DeckId::One, SemanticControl::Play, true);
        let cue = pressed(DeckId::One, SemanticControl::Cue, true);

        assert_eq!(session.schedule_event(play), IngestOutcome::Scheduled);
        assert_eq!(session.schedule_event(cue), IngestOutcome::Scheduled);
        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Control(play))
        );
        session.complete_delivery(true);
        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Control(cue))
        );
        session.complete_delivery(true);
        assert_eq!(session.next_delivery(None), None);
    }

    #[test]
    fn held_state_retries_without_losing_latest_level() {
        let mut session = ControllerSession::new();
        let touch = pressed(DeckId::Two, SemanticControl::JogTouch, true);
        assert_eq!(session.schedule_event(touch), IngestOutcome::Scheduled);

        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Control(touch))
        );
        session.complete_delivery(false);
        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Control(touch))
        );
        session.complete_delivery(true);
        assert_eq!(session.next_delivery(None), None);
        assert_eq!(session.stats().held_reconciliations, 1);
    }

    #[test]
    fn buffered_downstream_failure_uses_retry_slot() {
        let mut session = ControllerSession::new();
        let play = pressed(DeckId::One, SemanticControl::Play, true);
        session.schedule_event(play);

        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Control(play))
        );
        session.complete_delivery(false);
        assert_eq!(session.stats().downstream_retries, 1);
        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Control(play))
        );
        session.complete_delivery(true);
        assert_eq!(session.next_delivery(None), None);
    }

    #[test]
    fn reconnect_cannot_hide_pending_disconnect() {
        let mut session = ControllerSession::new();
        session.set_connected(true);
        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Connection(ConnectionState::Connected))
        );
        session.complete_delivery(true);

        session.set_connected(false);
        session.set_connected(true);

        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Connection(ConnectionState::Disconnected))
        );
        session.complete_delivery(true);
        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Connection(ConnectionState::Connected))
        );
        session.complete_delivery(true);
    }

    #[test]
    fn high_rate_absolute_controls_keep_latest_value() {
        let mut session = ControllerSession::new();
        session.schedule_event(absolute(SemanticControl::Crossfader, 100, 0x3fff));
        session.schedule_event(absolute(SemanticControl::Crossfader, 200, 0x3fff));
        session.schedule_event(absolute(SemanticControl::Crossfader, 300, 0x3fff));

        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Control(absolute(
                SemanticControl::Crossfader,
                300,
                0x3fff
            )))
        );
        session.complete_delivery(true);
        assert_eq!(session.next_delivery(None), None);
    }

    #[test]
    fn jog_deltas_accumulate_but_browse_deltas_remain_discrete() {
        let mut session = ControllerSession::new();
        session.schedule_event(relative(DeckId::One, SemanticControl::JogBend, 12));
        session.schedule_event(relative(DeckId::One, SemanticControl::JogBend, -3));
        session.schedule_event(ControlEvent {
            deck: None,
            control: SemanticControl::BrowseDelta,
            value: ControlValue::Relative(1),
        });
        session.schedule_event(ControlEvent {
            deck: None,
            control: SemanticControl::BrowseDelta,
            value: ControlValue::Relative(2),
        });

        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Control(ControlEvent {
                deck: None,
                control: SemanticControl::BrowseDelta,
                value: ControlValue::Relative(1),
            }))
        );
        session.complete_delivery(true);
        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Control(ControlEvent {
                deck: None,
                control: SemanticControl::BrowseDelta,
                value: ControlValue::Relative(2),
            }))
        );
        session.complete_delivery(true);
        assert_eq!(
            session.next_delivery(None),
            Some(SessionDelivery::Control(relative(
                DeckId::One,
                SemanticControl::JogBend,
                9
            )))
        );
    }

    #[test]
    fn usb_midi_event_ingests_channel_voice_without_transport_leakage() {
        let bytes = replay_profile_bytes();
        let profile = Profile::parse(&bytes).unwrap();
        let mut session = ControllerSession::new();

        let event = UsbMidiEvent::decode([0x0b, 0xb0, 0x10, 64]).unwrap();
        assert_eq!(
            session.process_usb_midi_event(&profile, event),
            Ok(UsbMidiIngestOutcome::Session(IngestOutcome::Scheduled))
        );
        assert_eq!(session.stats().midi_messages, 1);
        assert_eq!(session.stats().mapped_messages, 1);
    }

    #[test]
    fn usb_midi_transfer_batches_packets_without_leaking_transport_framing() {
        let bytes = replay_profile_bytes();
        let profile = Profile::parse(&bytes).unwrap();
        let mut session = ControllerSession::new();
        let transfer = [
            0x0b, 0xb0, 0x10, 64, // mapped CC
            0x0b, 0xb0, 0x11, 32, // valid but unmapped CC
            0x04, 0xf0, 0x01, 0x02, // SysEx, intentionally ignored
            0x1b, 0xb0, 0x10, 65, // non-zero virtual cable
            0x00, 0x00, 0x00, 0x00, // invalid CIN, decode fails closed
        ];

        assert_eq!(
            session.process_usb_midi_transfer(&profile, &transfer),
            Ok(UsbMidiBatchReport {
                packets: 5,
                decoded: 4,
                scheduled: 1,
                dropped: 0,
                no_mapping: 1,
                ignored_non_channel_voice: 1,
                unsupported_cable: 1,
                semantic_errors: 0,
            })
        );
        assert_eq!(session.stats().midi_messages, 2);
        assert_eq!(session.stats().mapped_messages, 1);
    }

    #[test]
    fn usb_midi_transfer_rejects_partial_event_packet() {
        let bytes = replay_profile_bytes();
        let profile = Profile::parse(&bytes).unwrap();
        let mut session = ControllerSession::new();

        assert_eq!(
            session.process_usb_midi_transfer(&profile, &[0x0b, 0xb0, 0x10]),
            Err(UsbMidiBatchError::MisalignedTransferLength(3))
        );
        assert_eq!(session.stats().midi_messages, 0);
    }

    #[test]
    fn usb_midi_event_fails_closed_for_nonzero_cable_and_non_channel_voice() {
        let bytes = replay_profile_bytes();
        let profile = Profile::parse(&bytes).unwrap();
        let mut session = ControllerSession::new();

        let cable_one = UsbMidiEvent::decode([0x1b, 0xb0, 0x10, 64]).unwrap();
        assert_eq!(
            session.process_usb_midi_event(&profile, cable_one),
            Ok(UsbMidiIngestOutcome::UnsupportedCable(1))
        );

        let sysex = UsbMidiEvent::decode([0x04, 0xf0, 0x01, 0x02]).unwrap();
        assert_eq!(
            session.process_usb_midi_event(&profile, sysex),
            Ok(UsbMidiIngestOutcome::IgnoredNonChannelVoice)
        );

        assert_eq!(session.stats().midi_messages, 0);
        assert_eq!(session.stats().mapped_messages, 0);
    }

    #[test]
    fn reconnect_snapshot_replays_cached_profile_state() {
        let bytes = replay_profile_bytes();
        let profile = Profile::parse(&bytes).unwrap();
        let mut session = ControllerSession::new();

        session.set_connected(true);
        assert_eq!(
            session.next_delivery(Some(&profile)),
            Some(SessionDelivery::Connection(ConnectionState::Connected))
        );
        session.complete_delivery(true);
        assert_eq!(session.next_delivery(Some(&profile)), None);

        assert_eq!(
            session.process_midi(&profile, 0xb0, 0x10, 64),
            Ok(IngestOutcome::Scheduled)
        );
        assert_eq!(
            session.next_delivery(Some(&profile)),
            Some(SessionDelivery::Control(absolute(
                SemanticControl::Crossfader,
                64,
                0x3fff
            )))
        );
        session.complete_delivery(true);

        session.set_connected(false);
        assert_eq!(
            session.next_delivery(Some(&profile)),
            Some(SessionDelivery::Connection(ConnectionState::Disconnected))
        );
        session.complete_delivery(true);
        session.set_connected(true);
        assert_eq!(
            session.next_delivery(Some(&profile)),
            Some(SessionDelivery::Connection(ConnectionState::Connected))
        );
        session.complete_delivery(true);

        assert_eq!(
            session.next_delivery(Some(&profile)),
            Some(SessionDelivery::Control(absolute(
                SemanticControl::Crossfader,
                64,
                0x3fff
            )))
        );
        session.complete_delivery(true);
        assert_eq!(session.stats().reconnect_snapshots, 2);
        assert_eq!(session.stats().snapshot_events, 1);
    }
}
