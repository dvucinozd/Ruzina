//! UAC stream ownership and teardown (M2.5 `controller_usb_audio_stream`).
//!
//! The owner decides; the USB adapter executes and reports back. Every
//! transfer handed to USB stays owned by USB until its callback returns, so
//! cleanup never frees memory the host controller may still touch: not when
//! an EP0 request times out, not when the device disappears, not when a
//! completion arrives late. Each prime and each stop advances the stream
//! epoch; only a completion of the current streaming epoch credits its
//! frames, so a late callback cannot confirm audio the sink already revoked.

use crate::descriptor::PlaybackFormat;

pub const TRANSFER_COUNT: usize = 3;
/// M2.5 bounds one EP0 request to a second.
pub const CONTROL_TIMEOUT_MS: u64 = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlStep {
    SetAlternate,
    SetSampleRate,
    /// Zero-bandwidth alternate on an orderly stop.
    SetZeroAlternate,
}

/// An 8-byte SETUP packet plus up to three data bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlRequest {
    pub bytes: [u8; 11],
    pub len: usize,
}

impl ControlRequest {
    pub fn build(step: ControlStep, format: &PlaybackFormat, rate: u32) -> Self {
        let mut bytes = [0; 11];
        let len = match step {
            ControlStep::SetAlternate | ControlStep::SetZeroAlternate => {
                // Standard OUT to interface: SET_INTERFACE.
                let alternate = if step == ControlStep::SetAlternate {
                    format.alternate
                } else {
                    0
                };
                bytes[..8].copy_from_slice(&[0x01, 0x0b, alternate, 0, format.interface, 0, 0, 0]);
                8
            }
            ControlStep::SetSampleRate => {
                // Class OUT to endpoint: SET_CUR SAMPLING_FREQ_CONTROL.
                bytes[..8].copy_from_slice(&[0x22, 0x01, 0x00, 0x01, format.endpoint, 0, 3, 0]);
                bytes[8..].copy_from_slice(&rate.to_le_bytes()[..3]);
                11
            }
        };
        Self { bytes, len }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Idle,
    Configuring,
    Streaming,
    Stopping,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IsoStatus {
    /// The URB completed; `lost_frames` counts skipped/short packets in it.
    Completed {
        lost_frames: u32,
    },
    Cancelled,
    NoDevice,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IsoNext {
    /// Refill and resubmit under `epoch`.
    Resubmit { epoch: u32, credit: bool },
    /// Keep the transfer; it is no longer USB-owned.
    Retired { credit: bool },
    /// A callback for a transfer USB did not own: ignore.
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlNext {
    Submit(ControlStep),
    /// Submit every transfer under `epoch`, then accept PCM.
    Prime {
        epoch: u32,
    },
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cleanup {
    /// Not stopping.
    Running,
    /// Halt and flush the endpoint so USB returns outstanding transfers.
    HaltAndFlush,
    Submit(ControlStep),
    /// USB still owns a transfer or the EP0 request.
    Wait,
    /// Nothing is USB-owned: free transfers and release the interface.
    FreeAndRelease,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamStats {
    pub config_failures: u32,
    pub transfer_failures: u32,
    pub control_timeouts: u32,
    pub lost_frames: u64,
}

#[derive(Debug)]
pub struct UacStream {
    phase: Phase,
    epoch: u32,
    frequency_control: bool,
    control: Option<(ControlStep, u64)>,
    control_expired: bool,
    transfers: [Option<u32>; TRANSFER_COUNT],
    device_gone: bool,
    flush_attempted: bool,
    alt_zero_sent: bool,
    faulted: bool,
    stats: StreamStats,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Busy;

impl Default for UacStream {
    fn default() -> Self {
        Self::new()
    }
}

impl UacStream {
    pub const fn new() -> Self {
        Self {
            phase: Phase::Idle,
            epoch: 0,
            frequency_control: false,
            control: None,
            control_expired: false,
            transfers: [None; TRANSFER_COUNT],
            device_gone: false,
            flush_attempted: false,
            alt_zero_sent: false,
            faulted: false,
            stats: StreamStats {
                config_failures: 0,
                transfer_failures: 0,
                control_timeouts: 0,
                lost_frames: 0,
            },
        }
    }

    pub const fn phase(&self) -> Phase {
        self.phase
    }
    pub const fn epoch(&self) -> u32 {
        self.epoch
    }
    pub const fn faulted(&self) -> bool {
        self.faulted
    }
    pub const fn stats(&self) -> StreamStats {
        self.stats
    }
    /// The producer may write PCM only while streaming.
    pub fn accepting(&self) -> bool {
        self.phase == Phase::Streaming
    }
    pub fn usb_owned(&self) -> usize {
        self.transfers.iter().flatten().count() + usize::from(self.control.is_some())
    }

    /// Call after the adapter claimed the interface; submit the returned step.
    pub fn start(&mut self, format: &PlaybackFormat, now_ms: u64) -> Result<ControlStep, Busy> {
        if self.phase != Phase::Idle {
            return Err(Busy);
        }
        *self = Self {
            epoch: self.epoch,
            stats: self.stats,
            ..Self::new()
        };
        self.frequency_control = format.frequency_control;
        self.phase = Phase::Configuring;
        self.control = Some((ControlStep::SetAlternate, now_ms));
        Ok(ControlStep::SetAlternate)
    }

    fn bump(&mut self) {
        self.epoch = self.epoch.wrapping_add(1).max(1);
    }

    fn fault(&mut self, configuration: bool) {
        if configuration {
            self.stats.config_failures = self.stats.config_failures.saturating_add(1);
        } else {
            self.stats.transfer_failures = self.stats.transfer_failures.saturating_add(1);
        }
        self.faulted = true;
        self.request_stop(false);
    }

    /// A submit the adapter could not start is not USB-owned.
    pub fn control_submit_failed(&mut self) {
        self.control = None;
        if self.phase == Phase::Stopping {
            self.control_expired = true;
        } else {
            self.fault(true);
        }
    }

    pub fn control_done(&mut self, ok: bool, now_ms: u64) -> ControlNext {
        let Some((step, _)) = self.control.take() else {
            return ControlNext::None;
        };
        if self.phase != Phase::Configuring {
            if step == ControlStep::SetZeroAlternate && !ok {
                self.control_expired = true;
            }
            return ControlNext::None;
        }
        if !ok {
            self.fault(true);
            return ControlNext::None;
        }
        if step == ControlStep::SetAlternate && self.frequency_control {
            self.control = Some((ControlStep::SetSampleRate, now_ms));
            return ControlNext::Submit(ControlStep::SetSampleRate);
        }
        self.bump();
        self.transfers = [Some(self.epoch); TRANSFER_COUNT];
        self.phase = Phase::Streaming;
        ControlNext::Prime { epoch: self.epoch }
    }

    /// A transfer the adapter failed to submit while priming or refilling.
    pub fn transfer_submit_failed(&mut self, index: usize) {
        if let Some(slot) = self.transfers.get_mut(index) {
            *slot = None;
        }
        self.fault(false);
    }

    pub fn iso_done(&mut self, index: usize, status: IsoStatus) -> IsoNext {
        let Some(epoch) = self.transfers.get_mut(index).and_then(Option::take) else {
            return IsoNext::Unknown;
        };
        let current = self.phase == Phase::Streaming && epoch == self.epoch;
        match status {
            IsoStatus::Completed { lost_frames } => {
                self.stats.lost_frames = self
                    .stats
                    .lost_frames
                    .saturating_add(u64::from(lost_frames));
                if current {
                    self.transfers[index] = Some(self.epoch);
                    IsoNext::Resubmit {
                        epoch: self.epoch,
                        credit: true,
                    }
                } else {
                    IsoNext::Retired { credit: false }
                }
            }
            IsoStatus::Cancelled | IsoStatus::NoDevice => {
                self.request_stop(status == IsoStatus::NoDevice);
                IsoNext::Retired { credit: false }
            }
            IsoStatus::Error => {
                if self.phase != Phase::Stopping {
                    self.fault(false);
                }
                IsoNext::Retired { credit: false }
            }
        }
    }

    /// Stops accepting PCM at once and starts teardown. The new epoch makes
    /// every outstanding completion uncredited.
    pub fn request_stop(&mut self, device_gone: bool) {
        self.device_gone |= device_gone;
        if self.phase == Phase::Idle {
            return;
        }
        if self.phase != Phase::Stopping {
            self.bump();
        }
        self.phase = Phase::Stopping;
    }

    pub fn poll(&mut self, now_ms: u64) -> Cleanup {
        if let Some((_, started)) = self.control
            && !self.control_expired
            && now_ms.saturating_sub(started) >= CONTROL_TIMEOUT_MS
        {
            // The request stays USB-owned until its callback returns.
            self.control_expired = true;
            self.stats.control_timeouts = self.stats.control_timeouts.saturating_add(1);
            if self.phase == Phase::Stopping {
                self.faulted = true;
            } else {
                self.fault(true);
            }
        }
        if self.phase != Phase::Stopping {
            return Cleanup::Running;
        }
        let transfers_owned = self.transfers.iter().any(Option::is_some);
        if !self.device_gone && transfers_owned && !self.flush_attempted {
            self.flush_attempted = true;
            return Cleanup::HaltAndFlush;
        }
        if self.control.is_some() || transfers_owned {
            return Cleanup::Wait;
        }
        if !self.device_gone && !self.control_expired && !self.alt_zero_sent {
            self.alt_zero_sent = true;
            self.control = Some((ControlStep::SetZeroAlternate, now_ms));
            return Cleanup::Submit(ControlStep::SetZeroAlternate);
        }
        self.phase = Phase::Idle;
        Cleanup::FreeAndRelease
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::tests::{alternate, controller};
    use crate::descriptor::{UsbSpeed, parse_playback_formats, select};

    fn format(frequency_control: bool) -> PlaybackFormat {
        let formats = parse_playback_formats(&controller(&[alternate(
            3,
            1,
            4,
            2,
            16,
            &[48_000],
            0x09,
            384,
            1,
        )]))
        .unwrap();
        let mut format = select(&formats, UsbSpeed::Full).unwrap().format;
        format.frequency_control = frequency_control;
        format
    }

    fn streaming(stream: &mut UacStream, frequency: bool) -> u32 {
        stream.start(&format(frequency), 0).unwrap();
        if frequency {
            assert_eq!(
                stream.control_done(true, 1),
                ControlNext::Submit(ControlStep::SetSampleRate)
            );
        }
        let ControlNext::Prime { epoch } = stream.control_done(true, 2) else {
            panic!("expected prime");
        };
        assert!(stream.accepting());
        epoch
    }

    #[test]
    fn setup_packets_match_uac1() {
        let format = format(true);
        assert_eq!(
            ControlRequest::build(ControlStep::SetAlternate, &format, 48_000).bytes[..8],
            [0x01, 0x0b, 1, 0, 3, 0, 0, 0]
        );
        let rate = ControlRequest::build(ControlStep::SetSampleRate, &format, 44_100);
        assert_eq!(rate.len, 11);
        assert_eq!(
            rate.bytes,
            [0x22, 0x01, 0x00, 0x01, 0x01, 0, 3, 0, 0x44, 0xac, 0x00]
        );
        assert_eq!(
            ControlRequest::build(ControlStep::SetZeroAlternate, &format, 48_000).bytes[..8],
            [0x01, 0x0b, 0, 0, 3, 0, 0, 0]
        );
    }

    #[test]
    fn steady_stream_resubmits_and_credits_current_epoch_only() {
        let mut stream = UacStream::new();
        let epoch = streaming(&mut stream, true);
        assert_eq!(stream.usb_owned(), TRANSFER_COUNT);
        assert_eq!(
            stream.iso_done(0, IsoStatus::Completed { lost_frames: 2 }),
            IsoNext::Resubmit {
                epoch,
                credit: true
            }
        );
        assert_eq!(
            stream.stats().lost_frames,
            2,
            "isolated loss does not restart USB"
        );
        assert_eq!(
            stream.iso_done(0, IsoStatus::Completed { lost_frames: 0 }),
            IsoNext::Resubmit {
                epoch,
                credit: true
            }
        );
        stream.request_stop(false);
        assert!(!stream.accepting());
        for index in 1..TRANSFER_COUNT {
            assert_eq!(
                stream.iso_done(index, IsoStatus::Completed { lost_frames: 0 }),
                IsoNext::Retired { credit: false },
                "a completion after stop credits nothing"
            );
        }
        assert_eq!(
            stream.iso_done(1, IsoStatus::Completed { lost_frames: 0 }),
            IsoNext::Unknown
        );
    }

    #[test]
    fn orderly_teardown_flushes_waits_sets_alt_zero_then_frees() {
        let mut stream = UacStream::new();
        streaming(&mut stream, false);
        stream.request_stop(false);
        assert_eq!(stream.poll(10), Cleanup::HaltAndFlush);
        assert_eq!(stream.poll(11), Cleanup::Wait);
        for index in 0..TRANSFER_COUNT {
            stream.iso_done(index, IsoStatus::Cancelled);
        }
        assert_eq!(
            stream.poll(12),
            Cleanup::Submit(ControlStep::SetZeroAlternate)
        );
        assert_eq!(stream.poll(13), Cleanup::Wait);
        assert_eq!(stream.control_done(true, 14), ControlNext::None);
        assert_eq!(stream.poll(15), Cleanup::FreeAndRelease);
        assert_eq!(stream.phase(), Phase::Idle);
        assert_eq!(stream.usb_owned(), 0);
        let restarted = streaming(&mut stream, false);
        assert!(restarted > 1, "a new stream never reuses an old epoch");
    }

    #[test]
    fn ep0_timeout_never_frees_a_request_usb_still_owns() {
        let mut stream = UacStream::new();
        stream.start(&format(true), 0).unwrap();
        assert_eq!(stream.poll(CONTROL_TIMEOUT_MS - 1), Cleanup::Running);
        assert_eq!(
            stream.poll(CONTROL_TIMEOUT_MS),
            Cleanup::Wait,
            "timed out, still owned"
        );
        assert!(stream.faulted());
        assert_eq!(stream.stats().control_timeouts, 1);
        for now in [2_000, 10_000, 60_000] {
            assert_eq!(stream.poll(now), Cleanup::Wait);
        }
        // The host finally cancels it; only then can memory be released.
        assert_eq!(stream.control_done(false, 61_000), ControlNext::None);
        assert_eq!(
            stream.poll(61_001),
            Cleanup::FreeAndRelease,
            "an expired device gets no alt-0 request"
        );
    }

    #[test]
    fn alt_zero_timeout_and_device_loss_still_wait_for_usb() {
        let mut stream = UacStream::new();
        streaming(&mut stream, false);
        stream.request_stop(false);
        assert_eq!(stream.poll(0), Cleanup::HaltAndFlush);
        for index in 0..TRANSFER_COUNT {
            stream.iso_done(index, IsoStatus::Cancelled);
        }
        assert_eq!(
            stream.poll(1),
            Cleanup::Submit(ControlStep::SetZeroAlternate)
        );
        assert_eq!(stream.poll(1 + CONTROL_TIMEOUT_MS), Cleanup::Wait);
        stream.control_done(false, 5_000);
        assert_eq!(stream.poll(5_001), Cleanup::FreeAndRelease);

        let mut stream = UacStream::new();
        streaming(&mut stream, false);
        assert_eq!(
            stream.iso_done(0, IsoStatus::NoDevice),
            IsoNext::Retired { credit: false }
        );
        assert_eq!(
            stream.poll(10),
            Cleanup::Wait,
            "no halt/flush or alt-0 for a gone device"
        );
        stream.iso_done(1, IsoStatus::NoDevice);
        stream.iso_done(2, IsoStatus::NoDevice);
        assert_eq!(stream.poll(11), Cleanup::FreeAndRelease);
    }

    #[test]
    fn configuration_and_transfer_faults_stop_without_leaking_ownership() {
        let mut stream = UacStream::new();
        stream.start(&format(true), 0).unwrap();
        assert_eq!(stream.start(&format(true), 0), Err(Busy));
        assert_eq!(stream.control_done(false, 1), ControlNext::None);
        assert!(stream.faulted());
        assert_eq!(stream.phase(), Phase::Stopping);
        assert_eq!(
            stream.poll(2),
            Cleanup::Submit(ControlStep::SetZeroAlternate)
        );
        stream.control_done(true, 3);
        assert_eq!(stream.poll(4), Cleanup::FreeAndRelease);

        streaming(&mut stream, false);
        stream.transfer_submit_failed(2);
        assert_eq!(stream.phase(), Phase::Stopping);
        assert_eq!(stream.poll(5), Cleanup::HaltAndFlush);
        stream.iso_done(0, IsoStatus::Error);
        stream.iso_done(1, IsoStatus::Completed { lost_frames: 0 });
        assert_eq!(stream.stats().transfer_failures, 1);
        assert_eq!(
            stream.poll(6),
            Cleanup::Submit(ControlStep::SetZeroAlternate)
        );
    }
}
