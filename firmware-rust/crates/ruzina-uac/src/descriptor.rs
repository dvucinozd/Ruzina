//! UAC1 playback formats from a configuration descriptor (M2.5
//! `flx4_uac_descriptors`).
//!
//! Only explicitly supported formats are offered: PCM, 16-bit in two bytes or
//! 24-bit packed in three, two or four channels, 44.1 or 48 kHz, one packet
//! per millisecond and no high-bandwidth packets; synchronous or adaptive, or
//! at full speed asynchronous with an explicit 3-byte feedback endpoint in the
//! same alternate (the Inpulse 500). Anything else is not an audio route; it
//! never fails MIDI, which the controller crate finds independently.

const DESCRIPTOR_CONFIGURATION: u8 = 0x02;
const DESCRIPTOR_INTERFACE: u8 = 0x04;
const DESCRIPTOR_ENDPOINT: u8 = 0x05;
const DESCRIPTOR_CS_INTERFACE: u8 = 0x24;
const DESCRIPTOR_CS_ENDPOINT: u8 = 0x25;
const CLASS_AUDIO: u8 = 0x01;
const SUBCLASS_STREAMING: u8 = 0x02;
const AS_GENERAL: u8 = 0x01;
const AS_FORMAT_TYPE: u8 = 0x02;
const FORMAT_TYPE_I: u8 = 0x01;
const FORMAT_TAG_PCM: u16 = 0x0001;
const TRANSFER_ISOCHRONOUS: u8 = 0x01;
const SYNC_ASYNCHRONOUS: u8 = 0x01;
const USAGE_FEEDBACK: u8 = 0x01;
const PROTOCOL_UAC1: u8 = 0x00;

pub const MAX_FORMATS: usize = 8;
pub const MAX_RATES: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbSpeed {
    Full,
    High,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SampleFormat {
    Pcm16,
    Packed24,
    /// USB Audio 2.0: 24 bits left-justified in a 4-byte subslot.
    Subslot32Bits24,
}

impl SampleFormat {
    pub const fn bytes(self) -> usize {
        match self {
            Self::Pcm16 => 2,
            Self::Packed24 => 3,
            Self::Subslot32Bits24 => 4,
        }
    }
}

/// One AudioStreaming alternate setting with an isochronous OUT endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackFormat {
    pub interface: u8,
    pub alternate: u8,
    pub endpoint: u8,
    /// Raw `wMaxPacketSize`, including high-bandwidth bits.
    pub max_packet: u16,
    pub interval: u8,
    pub channels: u8,
    pub subframe_bytes: u8,
    pub bits: u8,
    rates: [u32; MAX_RATES],
    rate_count: u8,
    pub pcm: bool,
    /// Asynchronous sync, a synch address or a feedback endpoint: the device
    /// sets the pace, supported only through [`PlaybackFormat::explicit_feedback`].
    pub feedback: bool,
    /// The OUT endpoint is asynchronous.
    pub asynchronous: bool,
    /// The OUT endpoint's `bSynchAddress`.
    pub sync_address: u8,
    /// The isochronous IN feedback endpoint of this alternate, 0 for none.
    pub feedback_endpoint: u8,
    pub feedback_interval: u8,
    /// `bRefresh`: a new feedback value every 2^n frames.
    pub feedback_refresh: u8,
    /// An implicit, second or malformed feedback endpoint.
    pub feedback_unusable: bool,
    /// The endpoint accepts SET_CUR sampling frequency.
    pub frequency_control: bool,
}

impl PlaybackFormat {
    const EMPTY: Self = Self {
        interface: 0,
        alternate: 0,
        endpoint: 0,
        max_packet: 0,
        interval: 0,
        channels: 0,
        subframe_bytes: 0,
        bits: 0,
        rates: [0; MAX_RATES],
        rate_count: 0,
        pcm: true,
        feedback: false,
        asynchronous: false,
        sync_address: 0,
        feedback_endpoint: 0,
        feedback_interval: 0,
        feedback_refresh: 0,
        feedback_unusable: false,
        frequency_control: false,
    };

    pub fn rates(&self) -> &[u32] {
        &self.rates[..usize::from(self.rate_count)]
    }

    pub fn sample_format(&self) -> Option<SampleFormat> {
        match (self.bits, self.subframe_bytes) {
            (16, 2) => Some(SampleFormat::Pcm16),
            (24, 3) => Some(SampleFormat::Packed24),
            _ => None,
        }
    }

    /// Bytes of the largest 1 ms packet at `rate`.
    pub fn packet_bytes(&self, rate: u32) -> usize {
        rate.div_ceil(1000) as usize * usize::from(self.channels) * usize::from(self.subframe_bytes)
    }

    /// Asynchronous OUT paced by this alternate's own explicit feedback
    /// endpoint, as USB Audio 1.0 3.7.2.2 defines it: 3 bytes every frame,
    /// refreshed every 2 to 512 ms, named by the OUT endpoint's synch address.
    pub fn explicit_feedback(&self) -> bool {
        self.asynchronous
            && !self.feedback_unusable
            && self.feedback_endpoint & 0x80 != 0
            && self.sync_address == self.feedback_endpoint
            && self.feedback_interval == 1
            && (1..=9).contains(&self.feedback_refresh)
    }

    /// Frames of the largest packet the endpoint takes.
    pub fn max_frames(&self) -> usize {
        let frame = usize::from(self.channels) * usize::from(self.subframe_bytes);
        usize::from(self.max_packet & 0x07ff)
            .checked_div(frame)
            .unwrap_or(0)
    }

    fn complete(&self) -> bool {
        self.endpoint != 0
            && self.max_packet != 0
            && self.channels != 0
            && self.subframe_bytes != 0
            && self.rate_count != 0
    }

    /// Whether this format streams `rate` at one packet per millisecond.
    pub fn supports(&self, rate: u32, speed: UsbSpeed) -> bool {
        let (interval_ok, payload_max) = match speed {
            // Full speed: bInterval counts frames. High speed: 2^(n-1)
            // microframes, so 4 is one millisecond.
            UsbSpeed::Full => (self.interval == 1, 1023),
            UsbSpeed::High => (self.interval == 4, 1024),
        };
        let high_bandwidth = self.max_packet >> 11 != 0;
        // Feedback is read at full speed only: 10.14 samples per 1 ms frame.
        let paced = !self.feedback || (speed == UsbSpeed::Full && self.explicit_feedback());
        self.pcm
            && paced
            && interval_ok
            && !high_bandwidth
            && self.sample_format().is_some()
            && matches!(self.channels, 2 | 4)
            && matches!(rate, 44_100 | 48_000)
            && self.rates().contains(&rate)
            && usize::from(self.max_packet) <= payload_max
            && self.packet_bytes(rate) <= usize::from(self.max_packet)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorError {
    Header,
    Malformed {
        offset: usize,
    },
    /// Two isochronous OUT endpoints in one alternate setting.
    DuplicateEndpoint {
        offset: usize,
    },
    TooManyRates {
        offset: usize,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct PlaybackFormats {
    formats: [PlaybackFormat; MAX_FORMATS],
    count: usize,
}

impl PlaybackFormats {
    pub fn as_slice(&self) -> &[PlaybackFormat] {
        &self.formats[..self.count]
    }

    fn push(&mut self, format: &PlaybackFormat) {
        if self.count < MAX_FORMATS && format.complete() {
            self.formats[self.count] = *format;
            self.count += 1;
        }
    }
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// Zero bytes from `offset` to the end of the configuration: padding inside
/// `wTotalLength` (the S2 MK3 ends in eight), not a descriptor. A zero length
/// followed by anything else is still malformed.
pub(crate) fn zero_tail(config: &[u8], offset: usize, total: usize) -> bool {
    config[offset..total].iter().all(|byte| *byte == 0)
}

/// Walks the whole configuration; any malformed descriptor refuses all
/// audio formats rather than yielding a partial result.
pub fn parse_playback_formats(config: &[u8]) -> Result<PlaybackFormats, DescriptorError> {
    if config.len() < 9 || config[0] != 9 || config[1] != DESCRIPTOR_CONFIGURATION {
        return Err(DescriptorError::Header);
    }
    let total = usize::from(le16(config, 2));
    if total < 9 || total > config.len() {
        return Err(DescriptorError::Header);
    }
    let mut out = PlaybackFormats {
        formats: [PlaybackFormat::EMPTY; MAX_FORMATS],
        count: 0,
    };
    let mut current = PlaybackFormat::EMPTY;
    let mut streaming = false;
    let mut seen_format_type = false;
    let mut offset = 9;
    while offset < total {
        if zero_tail(config, offset, total) {
            break;
        }
        let malformed = DescriptorError::Malformed { offset };
        if offset + 2 > total {
            return Err(malformed);
        }
        let len = usize::from(config[offset]);
        let kind = config[offset + 1];
        if len < 2
            || offset + len > total
            || (kind == DESCRIPTOR_INTERFACE && len < 9)
            || (kind == DESCRIPTOR_ENDPOINT && len < 7)
        {
            return Err(malformed);
        }
        let d = &config[offset..offset + len];
        match kind {
            DESCRIPTOR_INTERFACE => {
                if streaming {
                    out.push(&current);
                }
                current = PlaybackFormat::EMPTY;
                seen_format_type = false;
                // Alternate 0 is the zero-bandwidth setting. A USB Audio 2.0
                // alternate (protocol 0x20) has other class descriptors: it is
                // `uac2`'s, not walked here.
                streaming = d[5] == CLASS_AUDIO
                    && d[6] == SUBCLASS_STREAMING
                    && d[7] == PROTOCOL_UAC1
                    && d[3] != 0;
                current.interface = d[2];
                current.alternate = d[3];
            }
            DESCRIPTOR_CS_INTERFACE if streaming => {
                if len < 3 {
                    return Err(malformed);
                }
                match d[2] {
                    AS_GENERAL if len < 7 => return Err(malformed),
                    AS_FORMAT_TYPE if len < 8 || seen_format_type => return Err(malformed),
                    AS_FORMAT_TYPE => {
                        seen_format_type = true;
                        if d[3] != FORMAT_TYPE_I {
                            offset += len;
                            continue;
                        }
                        current.channels = d[4];
                        current.subframe_bytes = d[5];
                        current.bits = d[6];
                        let count = usize::from(d[7]);
                        if count > MAX_RATES {
                            return Err(DescriptorError::TooManyRates { offset });
                        }
                        if len < 8 + count * 3 {
                            return Err(malformed);
                        }
                        for index in 0..count {
                            let at = 8 + index * 3;
                            let rate = u32::from_le_bytes([d[at], d[at + 1], d[at + 2], 0]);
                            if rate != 0 {
                                let slot = current
                                    .rates
                                    .get_mut(usize::from(current.rate_count))
                                    .ok_or(DescriptorError::TooManyRates { offset })?;
                                *slot = rate;
                                current.rate_count += 1;
                            }
                        }
                    }
                    AS_GENERAL if len >= 7 => {
                        current.pcm = le16(d, 5) == FORMAT_TAG_PCM;
                    }
                    _ => {}
                }
            }
            DESCRIPTOR_ENDPOINT if streaming => {
                let attributes = d[3];
                if attributes & 0x03 == TRANSFER_ISOCHRONOUS && d[2] & 0x80 == 0 {
                    if current.endpoint != 0 {
                        return Err(DescriptorError::DuplicateEndpoint { offset });
                    }
                    current.endpoint = d[2];
                    current.max_packet = le16(d, 4);
                    current.interval = d[6];
                    current.asynchronous = (attributes >> 2) & 0x03 == SYNC_ASYNCHRONOUS;
                    current.sync_address = if len >= 9 { d[8] } else { 0 };
                    current.feedback |= current.asynchronous
                        || current.sync_address != 0
                        || (attributes >> 4) & 0x03 != 0;
                } else if attributes & 0x03 == TRANSFER_ISOCHRONOUS && (attributes >> 4) & 0x03 != 0
                {
                    // A feedback endpoint in the playback setting. Only one
                    // explicit IN endpoint of 3 bytes without its own sync
                    // type paces the stream; anything else is unusable.
                    current.feedback = true;
                    let explicit = (attributes >> 4) & 0x03 == USAGE_FEEDBACK && d[2] & 0x80 != 0;
                    if !explicit
                        || current.feedback_endpoint != 0
                        || len < 9
                        || (attributes >> 2) & 0x03 != 0
                        || le16(d, 4) != 3
                    {
                        current.feedback_unusable = true;
                    } else {
                        current.feedback_endpoint = d[2];
                        current.feedback_interval = d[6];
                        current.feedback_refresh = d[7];
                    }
                }
            }
            DESCRIPTOR_CS_ENDPOINT if streaming => {
                if len < 4 {
                    return Err(malformed);
                }
                current.frequency_control = d[3] & 0x01 != 0;
            }
            _ => {}
        }
        offset += len;
    }
    if streaming {
        out.push(&current);
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Selected {
    pub format: PlaybackFormat,
    pub rate: u32,
}

/// Prefers 48 kHz (the render rate), then four channels, then the higher
/// alternate setting (M2.5 scoring).
pub fn select(formats: &PlaybackFormats, speed: UsbSpeed) -> Option<Selected> {
    select_preferring(formats, speed, 48_000)
}

/// [`select`] with `preferred` (44.1 or 48 kHz) first instead of 48 kHz.
pub fn select_preferring(
    formats: &PlaybackFormats,
    speed: UsbSpeed,
    preferred: u32,
) -> Option<Selected> {
    let rates = if preferred == 44_100 {
        [44_100, 48_000]
    } else {
        [48_000, 44_100]
    };
    formats
        .as_slice()
        .iter()
        .filter_map(|format| {
            let rate = rates
                .into_iter()
                .find(|rate| format.supports(*rate, speed))?;
            let score = u32::from(rate == rates[0]) * 1000
                + u32::from(format.channels == 4) * 40
                + u32::from(format.alternate);
            Some((
                score,
                Selected {
                    format: *format,
                    rate,
                },
            ))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, selected)| selected)
}

/// Why a configuration offers no playback format, for the log. It changes
/// nothing in the choice: [`select`] alone decides.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rejection {
    /// The walk failed; no format of this configuration is offered.
    Descriptor(DescriptorError),
    /// No AudioStreaming alternate with an isochronous OUT endpoint.
    NoStreaming,
    /// Not PCM in 16/2 or 24/3, or not two or four channels.
    Format { interface: u8, alternate: u8 },
    /// Asynchronous OUT or feedback the deck cannot follow: not an explicit
    /// 3-byte endpoint of the same alternate, or not at full speed.
    Feedback { interface: u8, alternate: u8 },
    /// `bInterval` is not one packet per millisecond at this speed, or a
    /// high-bandwidth or oversize `wMaxPacketSize`.
    Interval { interface: u8, alternate: u8 },
    /// Neither 44.1 nor 48 kHz among the discrete rates.
    Rate { interface: u8, alternate: u8 },
    /// `wMaxPacketSize` is below the largest 1 ms packet.
    PacketSize { interface: u8, alternate: u8 },
    /// A USB Audio 2.0 alternate the deck can drive (`uac2`), once it speaks
    /// UAC2: it has no UAC1 alternate to use meanwhile.
    Uac2 { interface: u8, alternate: u8 },
    /// Only USB Audio 2.0 alternates, none of them one the deck can drive.
    Uac2Unsupported { interface: u8, alternate: u8 },
}

impl PlaybackFormat {
    /// The first check [`PlaybackFormat::supports`] fails for both rates, in
    /// that function's order; `None` when one rate is supported.
    pub fn rejection(&self, speed: UsbSpeed) -> Option<Rejection> {
        if [48_000, 44_100]
            .into_iter()
            .any(|rate| self.supports(rate, speed))
        {
            return None;
        }
        let (interface, alternate) = (self.interface, self.alternate);
        let interval_ok = match speed {
            UsbSpeed::Full => self.interval == 1 && self.max_packet <= 1023,
            UsbSpeed::High => self.interval == 4 && self.max_packet <= 1024,
        };
        Some(
            if !self.pcm || self.sample_format().is_none() || !matches!(self.channels, 2 | 4) {
                Rejection::Format {
                    interface,
                    alternate,
                }
            } else if self.feedback && !(speed == UsbSpeed::Full && self.explicit_feedback()) {
                Rejection::Feedback {
                    interface,
                    alternate,
                }
            } else if !interval_ok || self.max_packet >> 11 != 0 {
                Rejection::Interval {
                    interface,
                    alternate,
                }
            } else if !self
                .rates()
                .iter()
                .any(|rate| matches!(rate, 44_100 | 48_000))
            {
                Rejection::Rate {
                    interface,
                    alternate,
                }
            } else {
                Rejection::PacketSize {
                    interface,
                    alternate,
                }
            },
        )
    }
}

/// [`select`] on `config`, or why it offers nothing. A four-channel
/// alternate's reason is reported before others', the highest alternate's
/// first: that is the setting the deck would have used.
pub fn choose(config: &[u8], speed: UsbSpeed) -> Result<Selected, Rejection> {
    choose_preferring(config, speed, 48_000)
}

/// Why a configuration without UAC1 playback offers nothing: its UAC2
/// alternates, the first one the deck can drive before any other.
fn uac2_rejection(config: &[u8], speed: UsbSpeed) -> Rejection {
    let formats = match crate::uac2::parse(config) {
        Ok(formats) => formats,
        Err(error) => return Rejection::Descriptor(error),
    };
    let formats = formats.as_slice();
    if let Some(format) = formats.iter().find(|format| format.supported(speed)) {
        return Rejection::Uac2 {
            interface: format.interface,
            alternate: format.alternate,
        };
    }
    formats.first().map_or(Rejection::NoStreaming, |format| {
        Rejection::Uac2Unsupported {
            interface: format.interface,
            alternate: format.alternate,
        }
    })
}

/// [`choose`] with `preferred` first, as [`select_preferring`].
pub fn choose_preferring(
    config: &[u8],
    speed: UsbSpeed,
    preferred: u32,
) -> Result<Selected, Rejection> {
    let formats = parse_playback_formats(config).map_err(Rejection::Descriptor)?;
    if let Some(selected) = select_preferring(&formats, speed, preferred) {
        return Ok(selected);
    }
    if formats.as_slice().is_empty() {
        return Err(uac2_rejection(config, speed));
    }
    Err(formats
        .as_slice()
        .iter()
        .max_by_key(|format| (format.channels == 4, format.alternate))
        .and_then(|format| format.rejection(speed))
        .unwrap_or(Rejection::NoStreaming))
}

#[cfg(test)]
pub(crate) mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    /// AS alternate with a Type I format and an isochronous OUT endpoint.
    #[allow(clippy::too_many_arguments)]
    pub fn alternate(
        interface: u8,
        alternate: u8,
        channels: u8,
        bytes: u8,
        bits: u8,
        rates: &[u32],
        endpoint_attributes: u8,
        max_packet: u16,
        interval: u8,
    ) -> Vec<u8> {
        let mut out = std::vec![9, 4, interface, alternate, 1, 1, 2, 0, 0];
        out.extend_from_slice(&[7, 0x24, 1, 1, 1, 1, 0]);
        let mut format = std::vec![
            8 + rates.len() as u8 * 3,
            0x24,
            2,
            1,
            channels,
            bytes,
            bits,
            rates.len() as u8
        ];
        for rate in rates {
            format.extend_from_slice(&rate.to_le_bytes()[..3]);
        }
        out.extend_from_slice(&format);
        out.extend_from_slice(&[
            9,
            5,
            0x01,
            endpoint_attributes,
            max_packet as u8,
            (max_packet >> 8) as u8,
            interval,
            0,
            0,
        ]);
        out.extend_from_slice(&[7, 0x25, 1, 1, 0, 0, 0]);
        out
    }

    pub fn config(parts: &[&[u8]]) -> Vec<u8> {
        let mut out = std::vec![9, 2, 0, 0, 2, 1, 0, 0x80, 50];
        for part in parts {
            out.extend_from_slice(part);
        }
        let total = out.len() as u16;
        out[2..4].copy_from_slice(&total.to_le_bytes());
        out
    }

    /// AudioControl, zero-bandwidth alternate 0, then the given alternates,
    /// then a MIDI streaming interface.
    pub fn controller(alternates: &[Vec<u8>]) -> Vec<u8> {
        let mut parts: Vec<&[u8]> =
            std::vec![&[9, 4, 0, 0, 0, 1, 1, 0, 0], &[9, 4, 1, 0, 0, 1, 2, 0, 0]];
        for alt in alternates {
            parts.push(alt);
        }
        parts.push(&[9, 4, 2, 0, 2, 1, 3, 0, 0]);
        parts.push(&[7, 5, 0x02, 0x02, 64, 0, 0]);
        parts.push(&[7, 5, 0x82, 0x02, 64, 0, 0]);
        config(&parts)
    }

    #[test]
    fn malformed_class_descriptors_are_errors() {
        let interface = [9, 4, 1, 1, 1, 1, 2, 0, 0];
        for short in [&[3, 0x25, 1][..], &[3, 0x24, 1], &[3, 0x24, 2]] {
            assert!(parse_playback_formats(&config(&[&interface, short])).is_err());
        }
    }

    #[test]
    fn repeated_format_type_is_rejected_without_overflow() {
        let interface = [9, 4, 1, 1, 1, 1, 2, 0, 0];
        let mut format = std::vec![32, 0x24, 2, 1, 4, 2, 16, 8];
        for _ in 0..8 {
            format.extend_from_slice(&[0x80, 0xbb, 0]);
        }
        assert!(parse_playback_formats(&config(&[&interface, &format, &format])).is_err());
    }

    #[test]
    fn every_truncated_configuration_is_rejected() {
        let bytes = controller(&[alternate(1, 1, 2, 2, 16, &[48_000], 0x09, 192, 1)]);
        for end in 0..bytes.len() {
            assert!(parse_playback_formats(&bytes[..end]).is_err());
        }
    }

    #[test]
    fn parses_and_prefers_48k_four_channel_formats() {
        let bytes = controller(&[
            alternate(1, 1, 2, 2, 16, &[44_100, 48_000], 0x09, 192, 1),
            alternate(1, 2, 4, 3, 24, &[48_000], 0x09, 576, 1),
            alternate(1, 3, 4, 2, 16, &[44_100], 0x0d, 180, 1),
        ]);
        let formats = parse_playback_formats(&bytes).unwrap();
        assert_eq!(formats.as_slice().len(), 3);
        let best = select(&formats, UsbSpeed::Full).unwrap();
        assert_eq!((best.format.alternate, best.rate), (2, 48_000));
        assert_eq!(best.format.sample_format(), Some(SampleFormat::Packed24));
        assert!(best.format.frequency_control);
        assert_eq!(best.format.packet_bytes(44_100), 45 * 4 * 3);
    }

    #[test]
    fn unsupported_audio_yields_no_route() {
        let (i, a) = (1, 1);
        let format = Rejection::Format {
            interface: i,
            alternate: a,
        };
        let interval = Rejection::Interval {
            interface: i,
            alternate: a,
        };
        let rate = Rejection::Rate {
            interface: i,
            alternate: a,
        };
        for (alt, why, reason) in [
            (
                alternate(1, 1, 2, 2, 16, &[48_000], 0x05, 192, 1),
                "asynchronous",
                Rejection::Feedback {
                    interface: i,
                    alternate: a,
                },
            ),
            (
                alternate(1, 1, 2, 4, 32, &[48_000], 0x09, 384, 1),
                "32-bit",
                format,
            ),
            (
                alternate(1, 1, 6, 2, 16, &[48_000], 0x09, 576, 1),
                "six channels",
                format,
            ),
            (
                alternate(1, 1, 2, 2, 16, &[96_000], 0x09, 384, 1),
                "96 kHz",
                rate,
            ),
            (
                alternate(1, 1, 2, 2, 16, &[48_000], 0x09, 191, 1),
                "packet too small",
                Rejection::PacketSize {
                    interface: i,
                    alternate: a,
                },
            ),
            (
                alternate(1, 1, 2, 2, 16, &[48_000], 0x09, 192, 2),
                "2 ms interval",
                interval,
            ),
            (
                alternate(1, 1, 2, 2, 16, &[48_000], 0x09, 0x0800 | 192, 1),
                "high bandwidth",
                interval,
            ),
            (
                alternate(1, 1, 2, 2, 16, &[], 0x09, 192, 1),
                // Without a discrete rate the alternate is not offered at all.
                "continuous rates",
                Rejection::NoStreaming,
            ),
        ] {
            let bytes = controller(&[alt]);
            let formats = parse_playback_formats(&bytes).unwrap();
            assert_eq!(select(&formats, UsbSpeed::Full), None, "{why}");
            assert_eq!(choose(&bytes, UsbSpeed::Full), Err(reason), "{why}");
        }
    }

    /// The Inpulse 500's alternate: asynchronous 540-byte OUT 0x01 at 44.1 kHz
    /// naming `sync`, then the given feedback endpoints.
    fn asynchronous(sync: u8, feedback: &[&[u8]]) -> Vec<u8> {
        let mut alt = alternate(1, 1, 4, 3, 24, &[44_100], 0x05, 540, 1);
        let out_at = alt.len() - 7 - 9;
        alt[out_at + 8] = sync;
        for endpoint in feedback {
            alt.extend_from_slice(endpoint);
        }
        alt
    }

    const FEEDBACK: [u8; 9] = [9, 5, 0x81, 0x11, 3, 0, 1, 5, 0];

    #[test]
    fn explicit_feedback_is_followed_only_as_uac1_defines_it() {
        let good = controller(&[asynchronous(0x81, &[&FEEDBACK])]);
        let chosen = choose(&good, UsbSpeed::Full).unwrap();
        assert!(chosen.format.explicit_feedback());
        assert_eq!(chosen.rate, 44_100);
        let refused = Err(Rejection::Feedback {
            interface: 1,
            alternate: 1,
        });
        // Not at high speed, where feedback counts microframes.
        assert_eq!(choose(&good, UsbSpeed::High), refused);
        let mut refresh_zero = FEEDBACK;
        refresh_zero[7] = 0;
        let mut four_bytes = FEEDBACK;
        four_bytes[4] = 4;
        let mut every_2ms = FEEDBACK;
        every_2ms[6] = 2;
        let mut implicit = FEEDBACK;
        implicit[3] = 0x21;
        let mut own_sync = FEEDBACK;
        own_sync[3] = 0x15;
        for (why, alt) in [
            ("no feedback endpoint", asynchronous(0x81, &[])),
            ("synch address elsewhere", asynchronous(0x82, &[&FEEDBACK])),
            ("no synch address", asynchronous(0, &[&FEEDBACK])),
            ("refresh 0", asynchronous(0x81, &[&refresh_zero])),
            ("4-byte feedback", asynchronous(0x81, &[&four_bytes])),
            ("every 2 ms", asynchronous(0x81, &[&every_2ms])),
            ("implicit feedback", asynchronous(0x81, &[&implicit])),
            (
                "feedback with its own sync type",
                asynchronous(0x81, &[&own_sync]),
            ),
            (
                "two feedback endpoints",
                asynchronous(0x81, &[&FEEDBACK, &FEEDBACK]),
            ),
        ] {
            assert_eq!(
                choose(&controller(&[alt]), UsbSpeed::Full),
                refused,
                "{why}"
            );
        }
    }

    #[test]
    fn a_preferred_44k1_wins_where_the_alternate_has_it() {
        let both = controller(&[alternate(1, 1, 4, 2, 16, &[44_100, 48_000], 0x09, 384, 1)]);
        let formats = parse_playback_formats(&both).unwrap();
        assert_eq!(select(&formats, UsbSpeed::Full).unwrap().rate, 48_000);
        assert_eq!(
            select_preferring(&formats, UsbSpeed::Full, 44_100)
                .unwrap()
                .rate,
            44_100
        );
        assert_eq!(
            choose_preferring(&both, UsbSpeed::Full, 44_100)
                .unwrap()
                .rate,
            44_100
        );
        // Without 44.1 kHz the preference falls back to 48 kHz.
        let only = controller(&[alternate(1, 1, 4, 2, 16, &[48_000], 0x09, 384, 1)]);
        assert_eq!(
            choose_preferring(&only, UsbSpeed::Full, 44_100)
                .unwrap()
                .rate,
            48_000
        );
    }

    #[test]
    fn choose_agrees_with_select_and_names_the_failure() {
        let good = controller(&[alternate(1, 1, 4, 2, 16, &[48_000], 0x09, 384, 1)]);
        let formats = parse_playback_formats(&good).unwrap();
        assert_eq!(
            choose(&good, UsbSpeed::Full).ok(),
            select(&formats, UsbSpeed::Full)
        );
        let mut broken = good.clone();
        broken[9] = 0;
        assert_eq!(
            choose(&broken, UsbSpeed::Full),
            Err(Rejection::Descriptor(DescriptorError::Malformed {
                offset: 9
            }))
        );
        let midi_only = config(&[&[9, 4, 0, 0, 2, 1, 3, 0, 0]]);
        assert_eq!(
            choose(&midi_only, UsbSpeed::Full),
            Err(Rejection::NoStreaming)
        );
        // The four-channel alternate's reason wins over a two-channel one's.
        let mixed = controller(&[
            alternate(1, 1, 4, 2, 16, &[48_000], 0x05, 384, 1),
            alternate(1, 2, 2, 2, 16, &[96_000], 0x09, 384, 1),
        ]);
        assert_eq!(
            choose(&mixed, UsbSpeed::Full),
            Err(Rejection::Feedback {
                interface: 1,
                alternate: 1
            })
        );
    }

    #[test]
    fn high_speed_uac1_uses_interval_four_for_one_millisecond() {
        let formats = parse_playback_formats(&controller(&[alternate(
            1,
            1,
            4,
            2,
            16,
            &[44_100],
            0x09,
            360,
            4,
        )]))
        .unwrap();
        assert!(select(&formats, UsbSpeed::High).is_some());
        assert!(select(&formats, UsbSpeed::Full).is_none());
        let full = parse_playback_formats(&controller(&[alternate(
            1,
            1,
            4,
            2,
            16,
            &[44_100],
            0x09,
            360,
            1,
        )]))
        .unwrap();
        assert!(select(&full, UsbSpeed::High).is_none());
        let short = parse_playback_formats(&controller(&[alternate(
            1,
            1,
            4,
            2,
            16,
            &[44_100],
            0x09,
            359,
            4,
        )]))
        .unwrap();
        assert!(
            select(&short, UsbSpeed::High).is_none(),
            "45 frames x 4 ch x 2 bytes"
        );
    }

    #[test]
    fn zero_padding_ends_the_walk_and_uac2_alternates_are_not_walked() {
        let mut padded = controller(&[alternate(1, 1, 4, 2, 16, &[48_000], 0x09, 384, 1)]);
        padded.extend_from_slice(&[0; 8]);
        let total = padded.len() as u16;
        padded[2..4].copy_from_slice(&total.to_le_bytes());
        assert_eq!(parse_playback_formats(&padded).unwrap().as_slice().len(), 1);
        let last = padded.len() - 1;
        padded[last] = 1;
        assert!(parse_playback_formats(&padded).is_err());
        // The same alternate marked protocol 0x20, with a UAC2-sized Format
        // Type: not a UAC1 format, and not malformed.
        let mut uac2 = alternate(1, 1, 4, 4, 24, &[], 0x05, 112, 1);
        uac2[7] = 0x20;
        uac2.splice(9 + 7..9 + 7 + 8, [6, 0x24, 2, 1, 4, 24]);
        let formats = parse_playback_formats(&controller(&[uac2])).unwrap();
        assert!(formats.as_slice().is_empty());
    }

    #[test]
    fn malformed_descriptors_refuse_every_format() {
        let good = controller(&[alternate(1, 1, 2, 2, 16, &[48_000], 0x09, 192, 1)]);
        assert!(parse_playback_formats(&good).is_ok());
        let mut short = good.clone();
        short[2] = 200;
        assert_eq!(
            parse_playback_formats(&short).err(),
            Some(DescriptorError::Header)
        );
        let mut zero = good.clone();
        zero[9] = 0;
        assert_eq!(
            parse_playback_formats(&zero).err(),
            Some(DescriptorError::Malformed { offset: 9 })
        );
        let mut overrun = good.clone();
        let format_at = 9 + 9 + 9 + 9 + 7;
        overrun[format_at + 7] = 3;
        assert!(matches!(
            parse_playback_formats(&overrun),
            Err(DescriptorError::Malformed { .. })
        ));
        let mut many = good.clone();
        many[format_at + 7] = 9;
        assert!(matches!(
            parse_playback_formats(&many),
            Err(DescriptorError::TooManyRates { .. }) | Err(DescriptorError::Malformed { .. })
        ));
        let mut doubled = alternate(1, 1, 2, 2, 16, &[48_000], 0x09, 192, 1);
        doubled.extend_from_slice(&[9, 5, 0x03, 0x09, 192, 0, 1, 0, 0]);
        assert!(matches!(
            parse_playback_formats(&controller(&[doubled])),
            Err(DescriptorError::DuplicateEndpoint { .. })
        ));
    }
}
