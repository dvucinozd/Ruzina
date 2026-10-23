//! UAC1 playback formats from a configuration descriptor (M2.5
//! `flx4_uac_descriptors`).
//!
//! Only explicitly supported formats are offered: PCM, 16-bit in two bytes or
//! 24-bit packed in three, two or four channels, 44.1 or 48 kHz, one packet
//! per millisecond, synchronous/adaptive without a feedback endpoint and no
//! high-bandwidth packets. Anything else is not an audio route; it never
//! fails MIDI, which the controller crate finds independently.

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
}

impl SampleFormat {
    pub const fn bytes(self) -> usize {
        match self {
            Self::Pcm16 => 2,
            Self::Packed24 => 3,
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
    /// Asynchronous sync or a feedback endpoint: unsupported.
    pub feedback: bool,
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
        self.pcm
            && !self.feedback
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
    let mut offset = 9;
    while offset < total {
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
                // Alternate 0 is the zero-bandwidth setting.
                streaming = d[5] == CLASS_AUDIO && d[6] == SUBCLASS_STREAMING && d[3] != 0;
                current.interface = d[2];
                current.alternate = d[3];
            }
            DESCRIPTOR_CS_INTERFACE if streaming => {
                if len < 3 {
                    return Err(malformed);
                }
                match d[2] {
                    AS_GENERAL if len >= 7 => {
                        current.pcm = le16(d, 5) == FORMAT_TAG_PCM;
                    }
                    AS_FORMAT_TYPE if len >= 8 && d[3] == FORMAT_TYPE_I => {
                        current.channels = d[4];
                        current.subframe_bytes = d[5];
                        current.bits = d[6];
                        let count = usize::from(d[7]);
                        if count > MAX_RATES {
                            return Err(DescriptorError::TooManyRates { offset });
                        }
                        // Zero means a continuous range; only discrete
                        // rates are supported.
                        if len < 8 + count * 3 {
                            return Err(malformed);
                        }
                        for index in 0..count {
                            let at = 8 + index * 3;
                            let rate = u32::from_le_bytes([d[at], d[at + 1], d[at + 2], 0]);
                            if rate != 0 {
                                current.rates[usize::from(current.rate_count)] = rate;
                                current.rate_count += 1;
                            }
                        }
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
                    current.feedback |=
                        (attributes >> 2) & 0x03 == SYNC_ASYNCHRONOUS || (len >= 9 && d[8] != 0);
                } else if attributes & 0x03 == TRANSFER_ISOCHRONOUS && (attributes >> 4) & 0x03 != 0
                {
                    // An explicit feedback endpoint in the playback setting.
                    current.feedback = true;
                }
            }
            DESCRIPTOR_CS_ENDPOINT if streaming && len >= 3 => {
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
    formats
        .as_slice()
        .iter()
        .filter_map(|format| {
            let rate = [48_000, 44_100]
                .into_iter()
                .find(|rate| format.supports(*rate, speed))?;
            let score = u32::from(rate == 48_000) * 1000
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
        for (alt, why) in [
            (
                alternate(1, 1, 2, 2, 16, &[48_000], 0x05, 192, 1),
                "asynchronous",
            ),
            (alternate(1, 1, 2, 4, 32, &[48_000], 0x09, 384, 1), "32-bit"),
            (
                alternate(1, 1, 6, 2, 16, &[48_000], 0x09, 576, 1),
                "six channels",
            ),
            (alternate(1, 1, 2, 2, 16, &[96_000], 0x09, 384, 1), "96 kHz"),
            (
                alternate(1, 1, 2, 2, 16, &[48_000], 0x09, 191, 1),
                "packet too small",
            ),
            (
                alternate(1, 1, 2, 2, 16, &[48_000], 0x09, 192, 2),
                "2 ms interval",
            ),
            (
                alternate(1, 1, 2, 2, 16, &[48_000], 0x09, 0x0800 | 192, 1),
                "high bandwidth",
            ),
            (
                alternate(1, 1, 2, 2, 16, &[], 0x09, 192, 1),
                "continuous rates",
            ),
        ] {
            let formats = parse_playback_formats(&controller(&[alt])).unwrap();
            assert_eq!(select(&formats, UsbSpeed::Full), None, "{why}");
        }
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
