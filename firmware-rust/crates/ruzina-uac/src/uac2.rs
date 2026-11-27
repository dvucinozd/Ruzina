//! USB Audio 2.0 playback from a configuration descriptor (donor
//! `controller_uac2_parse`, the S2 MK3).
//!
//! A UAC2 function names its rates on a clock source, read at run time
//! (`GET RANGE`), not in the descriptor. A playback alternate is therefore
//! linked through its AS_GENERAL `bTerminalLink` to a USB-streaming input
//! terminal, and through that terminal's `bCSourceID` to a clock source of
//! the same AudioControl interface. Only what the deck can drive is offered:
//! high speed, PCM in 16-bit two-byte or 24-bit four-byte subslots, two or
//! four channels, one packet per microframe, asynchronous OUT paced by its
//! own explicit 4-byte feedback endpoint, and a clock source whose frequency
//! the host may set and whose validity it may read. Clock selectors and
//! multipliers between terminal and clock are not followed.
//!
//! Like the UAC1 walk, any malformed descriptor refuses the whole
//! configuration; zero padding at the end of `wTotalLength` is not a
//! descriptor.

use crate::descriptor::{DescriptorError, UsbSpeed, zero_tail};

const DESCRIPTOR_CONFIGURATION: u8 = 0x02;
const DESCRIPTOR_INTERFACE: u8 = 0x04;
const DESCRIPTOR_ENDPOINT: u8 = 0x05;
const DESCRIPTOR_CS_INTERFACE: u8 = 0x24;
const CLASS_AUDIO: u8 = 0x01;
const SUBCLASS_CONTROL: u8 = 0x01;
const SUBCLASS_STREAMING: u8 = 0x02;
const PROTOCOL_UAC2: u8 = 0x20;
const AC_INPUT_TERMINAL: u8 = 0x02;
const AC_CLOCK_SOURCE: u8 = 0x0a;
const AS_GENERAL: u8 = 0x01;
const AS_FORMAT_TYPE: u8 = 0x02;
const FORMAT_TYPE_I: u8 = 0x01;
const TERMINAL_USB_STREAMING: u16 = 0x0101;
const TRANSFER_ISOCHRONOUS: u8 = 0x01;
const SYNC_ASYNCHRONOUS: u8 = 0x01;
const USAGE_FEEDBACK: u8 = 0x01;

const MAX_ENTITIES: usize = 8;

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// One UAC2 AudioStreaming alternate with an isochronous OUT endpoint.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Uac2Playback {
    pub interface: u8,
    pub alternate: u8,
    pub endpoint: u8,
    /// Raw `wMaxPacketSize`, including high-bandwidth bits.
    pub max_packet: u16,
    pub interval: u8,
    pub channels: u8,
    pub subslot_bytes: u8,
    pub bits: u8,
    pub pcm: bool,
    pub asynchronous: bool,
    /// The explicit IN feedback endpoint, 0 for none.
    pub feedback_endpoint: u8,
    pub feedback_max_packet: u16,
    pub feedback_interval: u8,
    /// An implicit, second or malformed feedback endpoint.
    pub feedback_unusable: bool,
    /// The USB-streaming input terminal its `bTerminalLink` names.
    pub terminal: u8,
    /// The clock source of that terminal, 0 when not a clock source.
    pub clock_source: u8,
    /// The AudioControl interface the clock source belongs to: class
    /// requests to the clock go there.
    pub control_interface: u8,
    /// The clock's frequency is host programmable (`bmControls` bits 0-1 = 3).
    pub clock_settable: bool,
    /// The clock's validity is readable (`bmControls` bits 2-3 != 0).
    pub clock_validity: bool,
}

impl Uac2Playback {
    /// The bytes each sample takes on the wire: 16 bits in 2, or 24 in 4.
    pub fn subslot_supported(&self) -> bool {
        matches!((self.subslot_bytes, self.bits), (2, 16) | (4, 24))
    }

    /// Frames of the largest packet the endpoint takes.
    pub fn max_frames(&self) -> usize {
        let frame = usize::from(self.channels) * usize::from(self.subslot_bytes);
        usize::from(self.max_packet & 0x07ff)
            .checked_div(frame)
            .unwrap_or(0)
    }

    /// Whether the deck can stream this alternate: everything in the module
    /// summary. 48 kHz at high speed is 6 frames per microframe, 7 with the
    /// feedback's headroom, so the endpoint must take 7.
    pub fn supported(&self, speed: UsbSpeed) -> bool {
        speed == UsbSpeed::High
            && self.pcm
            && self.subslot_supported()
            && matches!(self.channels, 2 | 4)
            && self.interval == 1
            && self.max_packet >> 11 == 0
            && self.max_packet <= 1024
            && self.max_frames() >= 7
            && self.asynchronous
            && !self.feedback_unusable
            && self.feedback_endpoint & 0x80 != 0
            && self.feedback_max_packet == 4
            && (1..=7).contains(&self.feedback_interval)
            && self.clock_source != 0
            && self.clock_settable
            && self.clock_validity
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Clock {
    id: u8,
    interface: u8,
    settable: bool,
    validity: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Terminal {
    id: u8,
    interface: u8,
    clock: u8,
    usb_streaming: bool,
}

/// The configuration's UAC2 playback alternates.
#[derive(Clone, Copy, Debug)]
pub struct Uac2Formats {
    formats: [Uac2Playback; MAX_ENTITIES],
    count: usize,
}

impl Uac2Formats {
    pub fn as_slice(&self) -> &[Uac2Playback] {
        &self.formats[..self.count]
    }
}

/// Walks the whole configuration twice: the AudioControl entities first,
/// then the playback alternates, each linked to its clock.
pub fn parse(config: &[u8]) -> Result<Uac2Formats, DescriptorError> {
    if config.len() < 9 || config[0] != 9 || config[1] != DESCRIPTOR_CONFIGURATION {
        return Err(DescriptorError::Header);
    }
    let total = usize::from(le16(config, 2));
    if total < 9 || total > config.len() {
        return Err(DescriptorError::Header);
    }
    let mut clocks = [Clock::default(); MAX_ENTITIES];
    let mut terminals = [Terminal::default(); MAX_ENTITIES];
    let (mut clock_count, mut terminal_count) = (0, 0);
    let mut out = Uac2Formats {
        formats: [Uac2Playback::default(); MAX_ENTITIES],
        count: 0,
    };

    // First pass: lengths, and the clocks and terminals of each UAC2
    // AudioControl interface.
    let mut control: Option<u8> = None;
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
                control =
                    (d[5] == CLASS_AUDIO && d[6] == SUBCLASS_CONTROL && d[7] == PROTOCOL_UAC2)
                        .then_some(d[2]);
            }
            DESCRIPTOR_CS_INTERFACE if control.is_some() => {
                let interface = control.unwrap_or(0);
                if len < 3 {
                    return Err(malformed);
                }
                match d[2] {
                    AC_CLOCK_SOURCE => {
                        if len < 8 {
                            return Err(malformed);
                        }
                        let slot = clocks
                            .get_mut(clock_count)
                            .ok_or(DescriptorError::Malformed { offset })?;
                        *slot = Clock {
                            id: d[3],
                            interface,
                            settable: d[5] & 0x03 == 0x03,
                            validity: (d[5] >> 2) & 0x03 != 0,
                        };
                        clock_count += 1;
                    }
                    AC_INPUT_TERMINAL => {
                        if len < 17 {
                            return Err(malformed);
                        }
                        let slot = terminals
                            .get_mut(terminal_count)
                            .ok_or(DescriptorError::Malformed { offset })?;
                        *slot = Terminal {
                            id: d[3],
                            interface,
                            clock: d[7],
                            usb_streaming: le16(d, 4) == TERMINAL_USB_STREAMING,
                        };
                        terminal_count += 1;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        offset += len;
    }

    // Second pass: the playback alternates.
    let mut current: Option<Uac2Playback> = None;
    let mut offset = 9;
    let finish = |current: Option<Uac2Playback>, out: &mut Uac2Formats| {
        let Some(mut format) = current.filter(|format| format.endpoint != 0) else {
            return;
        };
        let terminal = terminals[..terminal_count]
            .iter()
            .find(|terminal| terminal.id == format.terminal && terminal.usb_streaming);
        if let Some(clock) = terminal.and_then(|terminal| {
            clocks[..clock_count]
                .iter()
                .find(|clock| clock.id == terminal.clock && clock.interface == terminal.interface)
        }) {
            format.clock_source = clock.id;
            format.control_interface = clock.interface;
            format.clock_settable = clock.settable;
            format.clock_validity = clock.validity;
        }
        if out.count < MAX_ENTITIES {
            out.formats[out.count] = format;
            out.count += 1;
        }
    };
    while offset < total {
        if zero_tail(config, offset, total) {
            break;
        }
        // Lengths were checked by the first pass.
        let len = usize::from(config[offset]);
        let d = &config[offset..offset + len];
        let malformed = DescriptorError::Malformed { offset };
        match d[1] {
            DESCRIPTOR_INTERFACE => {
                finish(current.take(), &mut out);
                // Alternate 0 is the zero-bandwidth setting.
                if d[5] == CLASS_AUDIO
                    && d[6] == SUBCLASS_STREAMING
                    && d[7] == PROTOCOL_UAC2
                    && d[3] != 0
                {
                    current = Some(Uac2Playback {
                        interface: d[2],
                        alternate: d[3],
                        ..Uac2Playback::default()
                    });
                }
            }
            DESCRIPTOR_CS_INTERFACE if current.is_some() => {
                let format = current.as_mut().ok_or(malformed)?;
                if len < 3 {
                    return Err(malformed);
                }
                match d[2] {
                    AS_GENERAL => {
                        if len < 16 {
                            return Err(malformed);
                        }
                        format.terminal = d[3];
                        format.pcm = d[5] == FORMAT_TYPE_I && le32(d, 6) & 1 != 0;
                        format.channels = d[10];
                    }
                    AS_FORMAT_TYPE => {
                        if len < 6 {
                            return Err(malformed);
                        }
                        if d[3] == FORMAT_TYPE_I {
                            format.subslot_bytes = d[4];
                            format.bits = d[5];
                        }
                    }
                    _ => {}
                }
            }
            DESCRIPTOR_ENDPOINT if current.is_some() => {
                let format = current.as_mut().ok_or(malformed)?;
                let attributes = d[3];
                if attributes & 0x03 != TRANSFER_ISOCHRONOUS {
                    // Not audio.
                } else if d[2] & 0x80 == 0 {
                    if format.endpoint != 0 {
                        return Err(DescriptorError::DuplicateEndpoint { offset });
                    }
                    format.endpoint = d[2];
                    format.max_packet = le16(d, 4);
                    format.interval = d[6];
                    format.asynchronous = (attributes >> 2) & 0x03 == SYNC_ASYNCHRONOUS;
                    // An OUT endpoint that is itself feedback data is not ours.
                    format.feedback_unusable |= (attributes >> 4) & 0x03 != 0;
                } else if (attributes >> 4) & 0x03 != 0 {
                    let explicit = (attributes >> 4) & 0x03 == USAGE_FEEDBACK;
                    if !explicit || format.feedback_endpoint != 0 || (attributes >> 2) & 0x03 != 0 {
                        format.feedback_unusable = true;
                    } else {
                        format.feedback_endpoint = d[2];
                        format.feedback_max_packet = le16(d, 4);
                        format.feedback_interval = d[6];
                    }
                }
            }
            _ => {}
        }
        offset += len;
    }
    finish(current.take(), &mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    /// A UAC2 function: AudioControl interface 0 with clock 0x11 (`controls`)
    /// and USB-streaming terminal 1 on it, then streaming interface 1
    /// alternate 1 of four 24-in-32 channels, OUT 0x01 of `mps` bytes and
    /// feedback IN 0x81, and `tail` zero bytes.
    fn function(controls: u8, mps: u16, feedback: &[u8], tail: usize) -> Vec<u8> {
        let mut out = std::vec![9, 2, 0, 0, 2, 1, 0, 0x80, 250];
        out.extend_from_slice(&[9, 4, 0, 0, 0, 1, 1, 0x20, 0]);
        out.extend_from_slice(&[8, 0x24, 0x0a, 0x11, 0x03, controls, 0, 0]);
        out.extend_from_slice(&[
            17, 0x24, 0x02, 0x01, 0x01, 0x01, 0, 0x11, 4, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        out.extend_from_slice(&[9, 4, 1, 0, 0, 1, 2, 0x20, 0]);
        out.extend_from_slice(&[9, 4, 1, 1, 2, 1, 2, 0x20, 0]);
        out.extend_from_slice(&[16, 0x24, 1, 1, 0, 1, 1, 0, 0, 0, 4, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&[6, 0x24, 2, 1, 4, 24]);
        out.extend_from_slice(&[7, 5, 0x01, 0x05, mps as u8, (mps >> 8) as u8, 1]);
        out.extend_from_slice(&[8, 0x25, 1, 0, 0, 2, 8, 0]);
        out.extend_from_slice(feedback);
        out.extend(core::iter::repeat_n(0, tail));
        let total = out.len() as u16;
        out[2..4].copy_from_slice(&total.to_le_bytes());
        out
    }

    const FEEDBACK: [u8; 7] = [7, 5, 0x81, 0x11, 4, 0, 4];

    #[test]
    fn a_driveable_function_links_its_alternate_to_the_clock() {
        let formats = parse(&function(0x07, 112, &FEEDBACK, 8)).unwrap();
        let [format] = formats.as_slice() else {
            panic!("{formats:?}")
        };
        assert_eq!(
            (format.interface, format.alternate, format.endpoint),
            (1, 1, 0x01)
        );
        assert_eq!(
            (format.channels, format.subslot_bytes, format.bits),
            (4, 4, 24)
        );
        assert_eq!(
            (
                format.terminal,
                format.clock_source,
                format.control_interface
            ),
            (1, 0x11, 0)
        );
        assert_eq!(
            (format.feedback_endpoint, format.feedback_interval),
            (0x81, 4)
        );
        assert_eq!(format.max_frames(), 7);
        assert!(format.supported(UsbSpeed::High));
        assert!(!format.supported(UsbSpeed::Full));
    }

    #[test]
    fn what_the_deck_cannot_drive_is_parsed_but_not_supported() {
        let mut implicit = FEEDBACK;
        implicit[3] = 0x21;
        for (why, config) in [
            (
                "clock frequency read-only",
                function(0x05, 112, &FEEDBACK, 0),
            ),
            ("no clock validity", function(0x03, 112, &FEEDBACK, 0)),
            ("6 frames a microframe", function(0x07, 96, &FEEDBACK, 0)),
            ("no feedback", function(0x07, 112, &[], 0)),
            ("implicit feedback", function(0x07, 112, &implicit, 0)),
            (
                "two feedback endpoints",
                function(0x07, 112, &[FEEDBACK, FEEDBACK].concat(), 0),
            ),
        ] {
            let formats = parse(&config).unwrap();
            assert_eq!(formats.as_slice().len(), 1, "{why}");
            assert!(!formats.as_slice()[0].supported(UsbSpeed::High), "{why}");
        }
    }

    #[test]
    fn malformed_and_padded_configurations() {
        // Zero padding is not a descriptor; a zero length before anything
        // else is malformed.
        let mut padded = function(0x07, 112, &FEEDBACK, 8);
        assert!(parse(&padded).is_ok());
        let last = padded.len() - 1;
        padded[last] = 1;
        assert!(matches!(
            parse(&padded),
            Err(DescriptorError::Malformed { .. })
        ));
        let good = function(0x07, 112, &FEEDBACK, 0);
        for end in 0..good.len() {
            assert!(parse(&good[..end]).is_err(), "truncated at {end}");
        }
        // A short AS_GENERAL or Format Type.
        let mut short = good.clone();
        let general = 9 + 9 + 8 + 17 + 9 + 9;
        short[general] = 15;
        assert!(parse(&short).is_err());
    }
}
