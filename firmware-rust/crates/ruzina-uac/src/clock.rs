//! USB Audio 2.0 clock negotiation (donor `controller_usb_audio_stream`
//! steps 4-8): which rate a clock source runs, set and confirmed over EP0
//! before its alternate streams.
//!
//! The sequence, each a class request to the clock's AudioControl interface:
//! 1. GET RANGE of the sampling frequency, 2 bytes: how many subranges.
//! 2. GET RANGE again, the whole `2 + 12 n` bytes. The preferred rate
//!    (48 kHz) must lie in a subrange; failing that, 44.1 kHz is used.
//! 3. SET CUR, 4 bytes: the rate.
//! 4. GET CUR, 4 bytes: the rate the clock reports. A clock may take a
//!    moment after SET CUR; the read is repeated every 50 ms for up to 500
//!    ms. If it settles at 44.1 kHz instead of 48 kHz, and the range has
//!    44.1, it is set once more at 44.1 kHz. The deck never streams one rate
//!    labelled as another.
//! 5. GET CUR of clock validity, 1 byte, until it reads 1, in the same window.
//!
//! Only then is the alternate selected. Nothing here touches USB: the
//! adapter sends each [`ClockRequest`] and hands back the reply.

const REQUEST_CUR: u8 = 0x01;
const REQUEST_RANGE: u8 = 0x02;
const CONTROL_FREQUENCY: u16 = 0x0100;
const CONTROL_VALIDITY: u16 = 0x0200;
const CLASS_INTERFACE_IN: u8 = 0xa1;
const CLASS_INTERFACE_OUT: u8 = 0x21;

/// Subranges a clock may name.
pub const MAX_SUBRANGES: usize = 8;
/// Bytes of the largest GET RANGE reply.
pub const MAX_RANGE_BYTES: usize = 2 + 12 * MAX_SUBRANGES;
/// After SET CUR, how long a readback or validity may take to settle.
pub const SETTLE_MS: u64 = 500;
/// Between two reads while it settles.
pub const RETRY_MS: u64 = 50;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockRequest {
    RangeCount,
    Range { bytes: u16 },
    SetRate(u32),
    GetRate,
    GetValidity,
}

/// An 8-byte SETUP packet and, for SET CUR, its 4 data bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockSetup {
    pub bytes: [u8; 12],
    pub len: usize,
    /// Bytes the device returns (IN requests), 0 for SET CUR.
    pub reply: u16,
}

impl ClockRequest {
    pub fn setup(self, clock: u8, interface: u8) -> ClockSetup {
        let (kind, request, control, length) = match self {
            Self::RangeCount => (CLASS_INTERFACE_IN, REQUEST_RANGE, CONTROL_FREQUENCY, 2),
            Self::Range { bytes } => (CLASS_INTERFACE_IN, REQUEST_RANGE, CONTROL_FREQUENCY, bytes),
            Self::SetRate(_) => (CLASS_INTERFACE_OUT, REQUEST_CUR, CONTROL_FREQUENCY, 4),
            Self::GetRate => (CLASS_INTERFACE_IN, REQUEST_CUR, CONTROL_FREQUENCY, 4),
            Self::GetValidity => (CLASS_INTERFACE_IN, REQUEST_CUR, CONTROL_VALIDITY, 1),
        };
        let mut bytes = [0; 12];
        bytes[0] = kind;
        bytes[1] = request;
        bytes[2..4].copy_from_slice(&control.to_le_bytes());
        bytes[4] = interface;
        bytes[5] = clock;
        bytes[6..8].copy_from_slice(&length.to_le_bytes());
        if let Self::SetRate(rate) = self {
            bytes[8..12].copy_from_slice(&rate.to_le_bytes());
            return ClockSetup {
                bytes,
                len: 12,
                reply: 0,
            };
        }
        ClockSetup {
            bytes,
            len: 8,
            reply: length,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockFailure {
    /// A request failed or stalled.
    Transfer,
    /// GET RANGE named no subrange, more than [`MAX_SUBRANGES`], or a
    /// reply of the wrong length or with an empty subrange.
    Range,
    /// Neither 48 nor 44.1 kHz is in the range.
    NoRate,
    /// The clock never reported the rate that was set.
    Readback { reported: u32 },
    /// The clock never became valid.
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockNext {
    Send(ClockRequest),
    /// Send it again at this time (ms), the clock still settling.
    RetryAt {
        at_ms: u64,
        request: ClockRequest,
    },
    /// The clock runs this rate and is valid.
    Ready(u32),
    Failed(ClockFailure),
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Whether a whole GET RANGE reply contains `rate`; `None` when malformed.
pub fn range_contains(reply: &[u8], rate: u32) -> Option<bool> {
    let count = usize::from(u16::from_le_bytes([*reply.first()?, *reply.get(1)?]));
    if count == 0 || count > MAX_SUBRANGES || reply.len() != 2 + 12 * count {
        return None;
    }
    let mut contains = false;
    for index in 0..count {
        let at = 2 + 12 * index;
        let (min, max, step) = (le32(reply, at), le32(reply, at + 4), le32(reply, at + 8));
        if min == 0 || min > max {
            return None;
        }
        contains |= (min..=max).contains(&rate) && (step == 0 || (rate - min).is_multiple_of(step));
    }
    Some(contains)
}

/// One clock source's negotiation, from [`ClockNegotiation::new`] to
/// [`ClockNext::Ready`] or [`ClockNext::Failed`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockNegotiation {
    rate: u32,
    range_has_44k1: bool,
    fell_back: bool,
    set_at_ms: u64,
    retries: u32,
    reported: u32,
}

impl ClockNegotiation {
    /// Starts at `preferred` (48 kHz); the first request is [`ClockRequest::RangeCount`].
    pub const fn new(preferred: u32) -> (Self, ClockRequest) {
        (
            Self {
                rate: preferred,
                range_has_44k1: false,
                fell_back: false,
                set_at_ms: 0,
                retries: 0,
                reported: 0,
            },
            ClockRequest::RangeCount,
        )
    }

    pub const fn rate(&self) -> u32 {
        self.rate
    }

    /// Reads repeated while the clock settled.
    pub const fn retries(&self) -> u32 {
        self.retries
    }

    /// The last rate the clock reported, 0 before any.
    pub const fn reported(&self) -> u32 {
        self.reported
    }

    /// `request` came back, `ok` when the transfer completed, with `reply`
    /// its data bytes, at `now_ms`.
    pub fn done(
        &mut self,
        request: ClockRequest,
        ok: bool,
        reply: &[u8],
        now_ms: u64,
    ) -> ClockNext {
        if !ok {
            return ClockNext::Failed(ClockFailure::Transfer);
        }
        let settling = now_ms.saturating_sub(self.set_at_ms) < SETTLE_MS;
        let retry = |this: &mut Self, request| {
            this.retries += 1;
            ClockNext::RetryAt {
                at_ms: now_ms + RETRY_MS,
                request,
            }
        };
        match request {
            ClockRequest::RangeCount => match reply {
                [low, high] => {
                    let count = usize::from(u16::from_le_bytes([*low, *high]));
                    if count == 0 || count > MAX_SUBRANGES {
                        ClockNext::Failed(ClockFailure::Range)
                    } else {
                        ClockNext::Send(ClockRequest::Range {
                            bytes: (2 + 12 * count) as u16,
                        })
                    }
                }
                _ => ClockNext::Failed(ClockFailure::Range),
            },
            ClockRequest::Range { .. } => {
                let (Some(preferred), Some(has_44k1)) = (
                    range_contains(reply, self.rate),
                    range_contains(reply, 44_100),
                ) else {
                    return ClockNext::Failed(ClockFailure::Range);
                };
                self.range_has_44k1 = has_44k1;
                if !preferred {
                    if !has_44k1 {
                        return ClockNext::Failed(ClockFailure::NoRate);
                    }
                    self.rate = 44_100;
                }
                ClockNext::Send(ClockRequest::SetRate(self.rate))
            }
            ClockRequest::SetRate(_) => {
                self.set_at_ms = now_ms;
                ClockNext::Send(ClockRequest::GetRate)
            }
            ClockRequest::GetRate => {
                let [a, b, c, d] = reply else {
                    return ClockNext::Failed(ClockFailure::Readback { reported: 0 });
                };
                self.reported = u32::from_le_bytes([*a, *b, *c, *d]);
                if self.reported == self.rate {
                    return ClockNext::Send(ClockRequest::GetValidity);
                }
                if settling {
                    return retry(self, ClockRequest::GetRate);
                }
                if self.reported == 44_100
                    && self.rate != 44_100
                    && self.range_has_44k1
                    && !self.fell_back
                {
                    // One explicit attempt at the rate the clock settled on.
                    self.fell_back = true;
                    self.rate = 44_100;
                    return ClockNext::Send(ClockRequest::SetRate(44_100));
                }
                ClockNext::Failed(ClockFailure::Readback {
                    reported: self.reported,
                })
            }
            ClockRequest::GetValidity => match reply {
                [1] => ClockNext::Ready(self.rate),
                [0] if settling => retry(self, ClockRequest::GetValidity),
                _ => ClockNext::Failed(ClockFailure::Invalid),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    /// A GET RANGE reply of `(min, max, step)` subranges.
    fn range(subranges: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut out = (subranges.len() as u16).to_le_bytes().to_vec();
        for (min, max, step) in subranges {
            for value in [min, max, step] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out
    }

    /// Runs the negotiation against a clock answering `reply(request, time)`;
    /// returns the outcome and the requests it saw.
    fn run(
        mut reply: impl FnMut(ClockRequest, u64) -> (bool, Vec<u8>),
    ) -> (ClockNext, Vec<ClockRequest>) {
        let (mut clock, mut request) = ClockNegotiation::new(48_000);
        let mut now = 1_000;
        let mut seen = Vec::new();
        for _ in 0..64 {
            seen.push(request);
            let (ok, bytes) = reply(request, now);
            match clock.done(request, ok, &bytes, now) {
                ClockNext::Send(next) => request = next,
                ClockNext::RetryAt {
                    at_ms,
                    request: next,
                } => {
                    assert!(at_ms > now);
                    now = at_ms;
                    request = next;
                }
                done => return (done, seen),
            }
            now += 1;
        }
        panic!("no outcome: {seen:?}")
    }

    #[test]
    fn setup_packets_address_the_clock_on_its_interface() {
        let range = ClockRequest::RangeCount.setup(0x11, 0);
        assert_eq!(&range.bytes[..range.len], &[0xa1, 2, 0, 1, 0, 0x11, 2, 0]);
        assert_eq!(range.reply, 2);
        let set = ClockRequest::SetRate(48_000).setup(0x11, 0);
        assert_eq!(
            &set.bytes[..set.len],
            &[0x21, 1, 0, 1, 0, 0x11, 4, 0, 0x80, 0xbb, 0, 0]
        );
        assert_eq!(set.reply, 0);
        let valid = ClockRequest::GetValidity.setup(0x11, 0);
        assert_eq!(&valid.bytes[..valid.len], &[0xa1, 1, 0, 2, 0, 0x11, 1, 0]);
        let full = ClockRequest::Range { bytes: 26 }.setup(0x11, 0);
        assert_eq!(full.reply, 26);
    }

    #[test]
    fn a_clock_that_takes_48k_at_once_is_ready() {
        let reply = range(&[(44_100, 44_100, 0), (48_000, 48_000, 0)]);
        let (outcome, seen) = run(|request, _| match request {
            ClockRequest::RangeCount => (true, reply[..2].to_vec()),
            ClockRequest::Range { bytes } => {
                assert_eq!(usize::from(bytes), reply.len());
                (true, reply.clone())
            }
            ClockRequest::SetRate(_) => (true, Vec::new()),
            ClockRequest::GetRate => (true, 48_000u32.to_le_bytes().to_vec()),
            ClockRequest::GetValidity => (true, std::vec![1]),
        });
        assert_eq!(outcome, ClockNext::Ready(48_000));
        assert_eq!(
            seen,
            [
                ClockRequest::RangeCount,
                ClockRequest::Range { bytes: 26 },
                ClockRequest::SetRate(48_000),
                ClockRequest::GetRate,
                ClockRequest::GetValidity,
            ]
        );
    }

    #[test]
    fn a_slow_clock_is_read_again_until_it_settles() {
        let reply = range(&[(44_100, 48_000, 3_900)]);
        let mut set_at = 0;
        let (outcome, seen) = run(|request, now| match request {
            ClockRequest::RangeCount => (true, reply[..2].to_vec()),
            ClockRequest::Range { .. } => (true, reply.clone()),
            ClockRequest::SetRate(_) => {
                set_at = now;
                (true, Vec::new())
            }
            // The old rate for 120 ms, then invalid for another 100.
            ClockRequest::GetRate if now < set_at + 120 => (true, 44_100u32.to_le_bytes().to_vec()),
            ClockRequest::GetRate => (true, 48_000u32.to_le_bytes().to_vec()),
            ClockRequest::GetValidity => (true, std::vec![u8::from(now >= set_at + 220)]),
        });
        assert_eq!(outcome, ClockNext::Ready(48_000));
        assert!(seen.iter().filter(|r| **r == ClockRequest::GetRate).count() >= 3);
        assert!(
            seen.iter()
                .filter(|r| **r == ClockRequest::GetValidity)
                .count()
                >= 2
        );
    }

    #[test]
    fn a_clock_stuck_at_44k1_is_set_there_once() {
        let reply = range(&[(44_100, 44_100, 0), (48_000, 48_000, 0)]);
        let (outcome, seen) = run(|request, _| match request {
            ClockRequest::RangeCount => (true, reply[..2].to_vec()),
            ClockRequest::Range { .. } => (true, reply.clone()),
            ClockRequest::SetRate(_) => (true, Vec::new()),
            ClockRequest::GetRate => (true, 44_100u32.to_le_bytes().to_vec()),
            ClockRequest::GetValidity => (true, std::vec![1]),
        });
        assert_eq!(outcome, ClockNext::Ready(44_100));
        assert!(seen.contains(&ClockRequest::SetRate(44_100)));
        // Without 44.1 kHz in the range that is a failure, not a relabel.
        let only_48k = range(&[(48_000, 48_000, 0)]);
        let (outcome, _) = run(|request, _| match request {
            ClockRequest::RangeCount => (true, only_48k[..2].to_vec()),
            ClockRequest::Range { .. } => (true, only_48k.clone()),
            ClockRequest::SetRate(_) => (true, Vec::new()),
            ClockRequest::GetRate => (true, 44_100u32.to_le_bytes().to_vec()),
            ClockRequest::GetValidity => (true, std::vec![1]),
        });
        assert_eq!(
            outcome,
            ClockNext::Failed(ClockFailure::Readback { reported: 44_100 })
        );
    }

    #[test]
    fn a_range_without_48k_starts_at_44k1() {
        let reply = range(&[(44_100, 44_100, 0)]);
        let (outcome, seen) = run(|request, _| match request {
            ClockRequest::RangeCount => (true, reply[..2].to_vec()),
            ClockRequest::Range { .. } => (true, reply.clone()),
            ClockRequest::SetRate(_) => (true, Vec::new()),
            ClockRequest::GetRate => (true, 44_100u32.to_le_bytes().to_vec()),
            ClockRequest::GetValidity => (true, std::vec![1]),
        });
        assert_eq!(outcome, ClockNext::Ready(44_100));
        assert!(seen.contains(&ClockRequest::SetRate(44_100)));
        assert!(!seen.contains(&ClockRequest::SetRate(48_000)));
    }

    #[test]
    fn what_cannot_be_negotiated_fails() {
        let wrong = range(&[(96_000, 192_000, 0)]);
        let cases: [(&str, ClockFailure, Vec<u8>); 4] = [
            ("no subrange", ClockFailure::Range, std::vec![0, 0]),
            ("nine subranges", ClockFailure::Range, std::vec![9, 0]),
            ("only high rates", ClockFailure::NoRate, wrong),
            (
                "empty subrange",
                ClockFailure::Range,
                range(&[(48_000, 44_100, 0)]),
            ),
        ];
        for (why, failure, reply) in cases {
            let (outcome, _) = run(|request, _| match request {
                ClockRequest::RangeCount => (true, reply[..2].to_vec()),
                ClockRequest::Range { .. } => (true, reply.clone()),
                _ => (true, Vec::new()),
            });
            assert_eq!(outcome, ClockNext::Failed(failure), "{why}");
        }
        // A stall, and a clock that never becomes valid.
        let (outcome, _) = run(|_, _| (false, Vec::new()));
        assert_eq!(outcome, ClockNext::Failed(ClockFailure::Transfer));
        let reply = range(&[(48_000, 48_000, 0)]);
        let (outcome, _) = run(|request, _| match request {
            ClockRequest::RangeCount => (true, reply[..2].to_vec()),
            ClockRequest::Range { .. } => (true, reply.clone()),
            ClockRequest::SetRate(_) => (true, Vec::new()),
            ClockRequest::GetRate => (true, 48_000u32.to_le_bytes().to_vec()),
            ClockRequest::GetValidity => (true, std::vec![0]),
        });
        assert_eq!(outcome, ClockNext::Failed(ClockFailure::Invalid));
    }

    #[test]
    fn range_replies_are_checked_whole() {
        let reply = range(&[(44_100, 48_000, 3_900)]);
        assert_eq!(range_contains(&reply, 48_000), Some(true));
        assert_eq!(range_contains(&reply, 44_100), Some(true));
        assert_eq!(range_contains(&reply, 46_000), Some(false));
        assert_eq!(range_contains(&reply[..13], 48_000), None);
        assert_eq!(range_contains(&[], 48_000), None);
    }
}
