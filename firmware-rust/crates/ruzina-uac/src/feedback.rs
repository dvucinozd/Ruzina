//! UAC1 explicit feedback (USB Audio 1.0 section 3.7.2.2, donor
//! `flx4_uac_feedback_q16`).
//!
//! At full speed an asynchronous device reports, on its feedback endpoint, how
//! many samples it takes per 1 ms frame: 3 bytes, 10.14 fixed point. The deck
//! keeps it as Q16 frames per millisecond for [`crate::packet::Cadence`].
//! Between refreshes the endpoint may answer with no bytes; that is no new
//! value, not an error.

/// Q16 frames per millisecond at `rate`.
pub const fn nominal_q16(rate: u32) -> u32 {
    (((rate as u64) << 16) / 1000) as u32
}

/// Q16 frames per 125 us microframe at `rate`.
pub const fn nominal_microframe_q16(rate: u32) -> u32 {
    (((rate as u64) << 16) / 8000) as u32
}

/// Decodes one USB Audio 2.0 high-speed feedback packet: 4 bytes, 16.16
/// frames per microframe (donor `controller_uac2_feedback`), within 5 % of
/// `rate` and no more than `max_frames` a packet. UAC2 has no empty replies.
pub fn decode_microframe(bytes: &[u8], rate: u32, max_frames: usize) -> Feedback {
    let [a, b, c, d] = bytes else {
        return Feedback::Invalid;
    };
    let value = u32::from_le_bytes([*a, *b, *c, *d]);
    let nominal = nominal_microframe_q16(rate);
    let largest = usize::try_from(value.div_ceil(1 << 16)).unwrap_or(usize::MAX);
    if rate == 0
        || value < nominal - nominal / 20
        || value > nominal + nominal / 20
        || largest > max_frames
    {
        return Feedback::Invalid;
    }
    Feedback::Rate(value)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Feedback {
    /// A new rate, Q16 frames per millisecond.
    Rate(u32),
    /// No bytes: the device has nothing new since its last refresh.
    Empty,
    /// Wrong length, more than 5 % from `rate`, or more frames than a packet
    /// holds.
    Invalid,
}

/// Decodes one feedback packet for a stream at `rate` whose packets hold at
/// most `max_frames` frames.
pub fn decode(bytes: &[u8], rate: u32, max_frames: usize) -> Feedback {
    let [low, mid, high] = match bytes {
        [] => return Feedback::Empty,
        [low, mid, high] => [*low, *mid, *high],
        _ => return Feedback::Invalid,
    };
    let value = u32::from_le_bytes([low, mid, high, 0]) << 2;
    let nominal = nominal_q16(rate);
    let largest = usize::try_from(value.div_ceil(1 << 16)).unwrap_or(usize::MAX);
    if rate == 0
        || value < nominal - nominal / 20
        || value > nominal + nominal / 20
        || largest > max_frames
    {
        return Feedback::Invalid;
    }
    Feedback::Rate(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10.14 bytes for `frames_per_ms`.
    fn wire(frames_per_ms: f64) -> [u8; 3] {
        let value = (frames_per_ms * 16_384.0).round() as u32;
        [value as u8, (value >> 8) as u8, (value >> 16) as u8]
    }

    #[test]
    fn a_44k1_reading_becomes_q16_frames_per_millisecond() {
        // 44.1 frames per ms is 0x0B0666 in 10.14.
        assert_eq!(wire(44.1), [0x66, 0x06, 0x0b]);
        let Feedback::Rate(q16) = decode(&wire(44.1), 44_100, 45) else {
            panic!()
        };
        assert!((i64::from(q16) - i64::from(nominal_q16(44_100))).abs() <= 4);
        // A device clock 0.1 % fast.
        let Feedback::Rate(fast) = decode(&wire(44.1441), 44_100, 45) else {
            panic!()
        };
        assert!(fast > q16);
    }

    #[test]
    fn uac2_readings_are_16_16_frames_per_microframe() {
        // 6.0 frames per microframe at 48 kHz.
        let six = (6u32 << 16).to_le_bytes();
        assert_eq!(decode_microframe(&six, 48_000, 7), Feedback::Rate(6 << 16));
        // 5.5125 at 44.1 kHz.
        let q16 = nominal_microframe_q16(44_100);
        assert_eq!(
            decode_microframe(&q16.to_le_bytes(), 44_100, 7),
            Feedback::Rate(q16)
        );
        // Empty, short, 10 % off, or 8 frames for a 7-frame packet.
        assert_eq!(decode_microframe(&[], 48_000, 7), Feedback::Invalid);
        assert_eq!(decode_microframe(&six[..3], 48_000, 7), Feedback::Invalid);
        let far = ((66u32 << 16) / 10).to_le_bytes();
        assert_eq!(decode_microframe(&far, 48_000, 7), Feedback::Invalid);
        assert_eq!(decode_microframe(&six, 48_000, 5), Feedback::Invalid);
    }

    #[test]
    fn empty_wrong_and_far_readings_are_not_rates() {
        assert_eq!(decode(&[], 44_100, 45), Feedback::Empty);
        assert_eq!(decode(&[1, 2], 44_100, 45), Feedback::Invalid);
        assert_eq!(decode(&[1, 2, 3, 4], 44_100, 45), Feedback::Invalid);
        // More than 5 % off.
        assert_eq!(decode(&wire(41.0), 44_100, 45), Feedback::Invalid);
        assert_eq!(decode(&wire(47.0), 44_100, 48), Feedback::Invalid);
        // Within 5 %, but a packet of 46 frames would not fit 540 bytes.
        assert_eq!(decode(&wire(45.5), 44_100, 45), Feedback::Invalid);
        assert!(matches!(decode(&wire(45.5), 44_100, 46), Feedback::Rate(_)));
        assert_eq!(decode(&wire(44.1), 0, 45), Feedback::Invalid);
    }
}
