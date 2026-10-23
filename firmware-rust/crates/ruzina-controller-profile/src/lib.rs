#![no_std]
#![forbid(unsafe_code)]

mod registry;
mod semantic;

pub use registry::*;
pub use semantic::{SemanticAdapterError, adapt_profile_event};

#[cfg(test)]
extern crate std;

use ruzina_controller_core::{DeckId, LedUpdate};

pub const S3CP_MAGIC: &[u8; 4] = b"S3CP";
/// Stable M2.5 accepts three binary versions. The compiler emits the oldest
/// version that represents a profile: v2 base tables, v3 adds CC7_TO14, and
/// v4 adds NOTE_SELECT, initial SysEx, LED value scaling and FILTER_ALWAYS.
pub const S3CP_VERSION_LEGACY: u16 = 2;
pub const S3CP_VERSION_SCALED: u16 = 3;
pub const S3CP_VERSION_EXTENDED: u16 = 4;
pub const HEADER_SIZE: usize = 32;
pub const INPUT_ENTRY_SIZE: usize = 16;
pub const OUTPUT_ENTRY_SIZE: usize = 12;

pub const MAX_INPUTS: usize = 320;
pub const MAX_OUTPUTS: usize = 160;
pub const MAX_PAIR_SLOTS: usize = 40;
pub const MAX_INIT_SYSEX: usize = 128;

pub const INPUT_FLAG_REPLAY: u16 = 0x0001;
pub const INPUT_FLAG_PAIR_MEMBER_B: u16 = 0x0002;

pub const DECK_ANY: u8 = 0xff;
pub const PAIR_SLOT_NONE: u8 = 0xff;

pub const PROFILE_FLAG_LED_FEEDBACK: u32 = 1 << 0;
pub const PROFILE_FLAG_USB_AUDIO: u32 = 1 << 1;
pub const PROFILE_FLAG_JOG_TOUCH: u32 = 1 << 2;
pub const PROFILE_FLAG_PITCH_14BIT: u32 = 1 << 3;
/// v4 only: the channel filter is not gated by Smart CFX for this profile.
pub const PROFILE_FLAG_FILTER_ALWAYS: u32 = 1 << 4;

const SEMANTIC_TYPE_PITCH: u8 = 0x03;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    Magic,
    Version,
    Size,
    Crc,
    Bounds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RawType {
    NoteButton = 0,
    NoteValue = 1,
    CcRel64 = 2,
    CcRel2c = 3,
    Cc14Msb = 4,
    Cc14Lsb = 5,
    Cc7Abs = 6,
    NoteStatePair = 7,
    /// v3+: center-preserving 7-bit to 14-bit absolute CC.
    Cc7To14 = 8,
    /// v4: Note On press emits a fixed nonnegative value; release is ignored.
    NoteSelect = 9,
}

impl RawType {
    fn from_byte(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::NoteButton),
            1 => Some(Self::NoteValue),
            2 => Some(Self::CcRel64),
            3 => Some(Self::CcRel2c),
            4 => Some(Self::Cc14Msb),
            5 => Some(Self::Cc14Lsb),
            6 => Some(Self::Cc7Abs),
            7 => Some(Self::NoteStatePair),
            8 => Some(Self::Cc7To14),
            9 => Some(Self::NoteSelect),
            _ => None,
        }
    }

    const fn min_version(self) -> u16 {
        match self {
            Self::Cc7To14 => S3CP_VERSION_SCALED,
            Self::NoteSelect => S3CP_VERSION_EXTENDED,
            _ => S3CP_VERSION_LEGACY,
        }
    }

    fn needs_pair_slot(self) -> bool {
        matches!(self, Self::Cc14Msb | Self::Cc14Lsb | Self::NoteStatePair)
    }
}

/// Integer 0/64/127 -> 0/8192/16383 scaling; monotonic between detents.
pub const fn cc7_to14(data2: u8) -> i16 {
    let value = (data2 & 0x7f) as i32;
    let scaled = if value <= 64 {
        value * 128
    } else {
        8192 + ((value - 64) * 8191 + 31) / 63
    };
    scaled as i16
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum OutputKind {
    NoteOnOff = 0,
    CcValue = 1,
}

impl OutputKind {
    fn from_byte(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::NoteOnOff),
            1 => Some(Self::CcValue),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputEntry {
    pub match_status: u8,
    pub match_data1: u8,
    pub raw_type: RawType,
    pub pair_slot: u8,
    pub semantic_type: u8,
    pub semantic_id: u8,
    pub flags: u16,
    pub base_value: i16,
    pub press_mask: u16,
    pub lut: [i8; 4],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputEntry {
    pub led_id: u8,
    pub deck: u8,
    pub kind: OutputKind,
    pub status: u8,
    pub data1: u8,
    pub off_value: u8,
    pub on_value: u8,
    pub blink_value: u8,
    /// v4 only; zero keeps the selected byte. Reserved and ignored in v2/v3.
    pub value_scale: u16,
}

impl OutputEntry {
    fn value_for_state(self, state: u8) -> u8 {
        let value = match self.kind {
            OutputKind::CcValue => state & 0x7f,
            OutputKind::NoteOnOff if state == 0 => self.off_value,
            OutputKind::NoteOnOff if state == 2 => self.blink_value,
            OutputKind::NoteOnOff => self.on_value,
        };
        if self.value_scale == 0 {
            return value;
        }
        (u32::from(value) * u32::from(self.value_scale) / 127).min(127) as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileEvent {
    pub semantic_type: u8,
    pub semantic_id: u8,
    pub value: i16,
}

#[derive(Clone, Copy, Debug)]
pub struct Profile<'a> {
    blob: &'a [u8],
    version: u16,
    crc: u32,
    vid: u16,
    pid: u16,
    flags: u32,
    input_count: usize,
    output_count: usize,
    pair_slot_count: usize,
    decks: u8,
    init_sysex_len: usize,
}

impl<'a> Profile<'a> {
    pub fn parse(blob: &'a [u8]) -> Result<Self, ParseError> {
        if blob.len() < HEADER_SIZE {
            return Err(ParseError::Size);
        }
        if &blob[0..4] != S3CP_MAGIC {
            return Err(ParseError::Magic);
        }

        let version = read_u16(blob, 4);
        let header_size = read_u16(blob, 6) as usize;
        let profile_size = read_u32(blob, 8) as usize;
        let expected_crc = read_u32(blob, 12);

        if !matches!(
            version,
            S3CP_VERSION_LEGACY | S3CP_VERSION_SCALED | S3CP_VERSION_EXTENDED
        ) || header_size != HEADER_SIZE
        {
            return Err(ParseError::Version);
        }
        if profile_size != blob.len() || profile_size < HEADER_SIZE {
            return Err(ParseError::Size);
        }
        if crc32(&blob[16..profile_size]) != expected_crc {
            return Err(ParseError::Crc);
        }

        let flags = read_u32(blob, 20);
        let input_count = read_u16(blob, 24) as usize;
        let output_count = read_u16(blob, 26) as usize;
        let pair_slot_count = blob[28] as usize;
        // Offset 30 is reserved and ignored before v4, as in M2.5.
        let init_sysex_len = if version >= S3CP_VERSION_EXTENDED {
            read_u16(blob, 30) as usize
        } else {
            0
        };

        if input_count > MAX_INPUTS
            || output_count > MAX_OUTPUTS
            || pair_slot_count > MAX_PAIR_SLOTS
            || init_sysex_len > MAX_INIT_SYSEX
            || (version < S3CP_VERSION_EXTENDED && flags & PROFILE_FLAG_FILTER_ALWAYS != 0)
        {
            return Err(ParseError::Bounds);
        }

        let need = HEADER_SIZE
            .checked_add(
                input_count
                    .checked_mul(INPUT_ENTRY_SIZE)
                    .ok_or(ParseError::Size)?,
            )
            .and_then(|value| {
                output_count
                    .checked_mul(OUTPUT_ENTRY_SIZE)
                    .and_then(|outputs| value.checked_add(outputs))
            })
            .and_then(|value| value.checked_add(init_sysex_len))
            .ok_or(ParseError::Size)?;

        if need != blob.len() {
            return Err(ParseError::Size);
        }

        let profile = Self {
            blob,
            version,
            crc: expected_crc,
            vid: read_u16(blob, 16),
            pid: read_u16(blob, 18),
            flags,
            input_count,
            output_count,
            pair_slot_count,
            decks: blob[29],
            init_sysex_len,
        };

        for index in 0..profile.input_count {
            let entry = profile.input(index).ok_or(ParseError::Bounds)?;
            if entry.raw_type.min_version() > version {
                return Err(ParseError::Bounds);
            }
            if entry.raw_type.needs_pair_slot()
                && (entry.pair_slot == PAIR_SLOT_NONE
                    || entry.pair_slot as usize >= profile.pair_slot_count)
            {
                return Err(ParseError::Bounds);
            }
            let invalid = match entry.raw_type {
                RawType::NoteSelect => {
                    entry.match_status & 0xf0 != 0x90
                        || entry.pair_slot != PAIR_SLOT_NONE
                        || entry.flags != 0
                        || entry.press_mask != 0
                        || entry.base_value < 0
                }
                RawType::Cc7To14 => {
                    entry.match_status & 0xf0 != 0xb0
                        || entry.semantic_type != SEMANTIC_TYPE_PITCH
                        || entry.pair_slot != PAIR_SLOT_NONE
                        || entry.flags & !INPUT_FLAG_REPLAY != 0
                }
                _ => false,
            };
            if invalid {
                return Err(ParseError::Bounds);
            }
        }

        for index in 0..profile.output_count {
            if profile.output(index).is_none() {
                return Err(ParseError::Bounds);
            }
        }

        let sysex = profile.init_sysex();
        if let [first, body @ .., last] = sysex {
            if *first != 0xf0 || *last != 0xf7 || body.iter().any(|byte| *byte > 0x7f) {
                return Err(ParseError::Bounds);
            }
        } else if !sysex.is_empty() {
            return Err(ParseError::Bounds);
        }

        Ok(profile)
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub const fn crc(&self) -> u32 {
        self.crc
    }

    /// Bounded initial output SysEx (v4). Empty for v2/v3 and for v4 profiles
    /// without one. MIDI/LED activation must wait until it was enqueued for
    /// the current controller binding.
    pub fn init_sysex(&self) -> &'a [u8] {
        let start = self.blob.len() - self.init_sysex_len;
        &self.blob[start..]
    }

    /// Profile-owned capability: the channel filter bypasses the Smart CFX
    /// enable gate. Absent for every v2/v3 profile.
    pub const fn channel_filter_always(&self) -> bool {
        self.flags & PROFILE_FLAG_FILTER_ALWAYS != 0
    }

    pub const fn vid(&self) -> u16 {
        self.vid
    }

    pub const fn pid(&self) -> u16 {
        self.pid
    }

    pub const fn flags(&self) -> u32 {
        self.flags
    }

    pub const fn input_count(&self) -> usize {
        self.input_count
    }

    pub const fn output_count(&self) -> usize {
        self.output_count
    }

    pub const fn pair_slot_count(&self) -> usize {
        self.pair_slot_count
    }

    pub const fn decks(&self) -> u8 {
        self.decks
    }

    pub fn input(&self, index: usize) -> Option<InputEntry> {
        if index >= self.input_count {
            return None;
        }

        let offset = HEADER_SIZE + index * INPUT_ENTRY_SIZE;
        let bytes = &self.blob[offset..offset + INPUT_ENTRY_SIZE];
        let raw_type = RawType::from_byte(bytes[2])?;

        Some(InputEntry {
            match_status: bytes[0],
            match_data1: bytes[1],
            raw_type,
            pair_slot: bytes[3],
            semantic_type: bytes[4],
            semantic_id: bytes[5],
            flags: u16::from_le_bytes([bytes[6], bytes[7]]),
            base_value: i16::from_le_bytes([bytes[8], bytes[9]]),
            press_mask: u16::from_le_bytes([bytes[10], bytes[11]]),
            lut: [
                bytes[12] as i8,
                bytes[13] as i8,
                bytes[14] as i8,
                bytes[15] as i8,
            ],
        })
    }

    pub fn output(&self, index: usize) -> Option<OutputEntry> {
        if index >= self.output_count {
            return None;
        }

        let offset = HEADER_SIZE + self.input_count * INPUT_ENTRY_SIZE + index * OUTPUT_ENTRY_SIZE;
        let bytes = &self.blob[offset..offset + OUTPUT_ENTRY_SIZE];
        let kind = OutputKind::from_byte(bytes[2])?;

        Some(OutputEntry {
            led_id: bytes[0],
            deck: bytes[1],
            kind,
            status: bytes[3],
            data1: bytes[4],
            off_value: bytes[5],
            on_value: bytes[6],
            blink_value: bytes[7],
            value_scale: if self.version >= S3CP_VERSION_EXTENDED {
                u16::from_le_bytes([bytes[8], bytes[9]])
            } else {
                0
            },
        })
    }

    pub fn map_led(&self, led_id: u8, deck: u8, state: u8) -> Option<[u8; 3]> {
        for index in 0..self.output_count {
            let entry = self.output(index)?;
            if entry.led_id != led_id {
                continue;
            }
            if entry.deck != DECK_ANY && entry.deck != deck {
                continue;
            }

            return Some([entry.status, entry.data1, entry.value_for_state(state)]);
        }

        None
    }

    pub fn map_led_update(&self, update: LedUpdate) -> Option<[u8; 3]> {
        let deck = match update.deck {
            Some(DeckId::One) => 0,
            Some(DeckId::Two) => 1,
            None => DECK_ANY,
        };

        for index in 0..self.output_count {
            let entry = self.output(index)?;
            if entry.led_id != update.led.as_u8() || entry.deck != deck {
                continue;
            }

            let value = entry.value_for_state(update.state.wire_value());
            return Some([entry.status, entry.data1, value]);
        }

        None
    }
}

#[derive(Clone, Copy, Debug)]
struct PairSlot {
    msb: u8,
    lsb: u8,
    msb_valid: bool,
    lsb_valid: bool,
    pair_bits: u8,
}

impl PairSlot {
    const EMPTY: Self = Self {
        msb: 0,
        lsb: 0,
        msb_valid: false,
        lsb_valid: false,
        pair_bits: 0,
    };

    fn value(&self) -> Option<i16> {
        if !self.msb_valid || !self.lsb_valid {
            return None;
        }

        Some((((self.msb & 0x7f) as u16) << 7 | (self.lsb & 0x7f) as u16) as i16)
    }
}

pub struct ProfileRuntime {
    slots: [PairSlot; MAX_PAIR_SLOTS],
    cc7_value: [i16; MAX_INPUTS],
    cc7_valid: [bool; MAX_INPUTS],
}

impl Default for ProfileRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileRuntime {
    pub const fn new() -> Self {
        Self {
            slots: [PairSlot::EMPTY; MAX_PAIR_SLOTS],
            cc7_value: [0; MAX_INPUTS],
            cc7_valid: [false; MAX_INPUTS],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn process(
        &mut self,
        profile: &Profile<'_>,
        status: u8,
        data1: u8,
        data2: u8,
    ) -> Option<ProfileEvent> {
        for index in 0..profile.input_count {
            let entry = profile.input(index)?;
            if entry.match_status != status || entry.match_data1 != data1 {
                continue;
            }

            let pressed = data2 > 0;
            let value = match entry.raw_type {
                RawType::NoteSelect => {
                    if !pressed {
                        return None;
                    }
                    entry.base_value
                }
                RawType::NoteButton => {
                    if pressed {
                        1
                    } else {
                        0
                    }
                }
                RawType::NoteValue => {
                    entry.base_value | if pressed { entry.press_mask as i16 } else { 0 }
                }
                RawType::CcRel64 => {
                    let delta = data2 as i16 - 64;
                    if delta == 0 {
                        return None;
                    }
                    delta
                }
                RawType::CcRel2c => {
                    let value = data2 & 0x7f;
                    if value == 0x00 || value == 0x40 {
                        return None;
                    }
                    if value < 0x40 {
                        value as i16
                    } else {
                        value as i16 - 0x80
                    }
                }
                RawType::Cc14Msb | RawType::Cc14Lsb => {
                    let slot = &mut self.slots[entry.pair_slot as usize];
                    if entry.raw_type == RawType::Cc14Msb {
                        slot.msb = data2 & 0x7f;
                        slot.msb_valid = true;
                    } else {
                        slot.lsb = data2 & 0x7f;
                        slot.lsb_valid = true;
                    }
                    slot.value()?
                }
                RawType::Cc7Abs | RawType::Cc7To14 => {
                    // Replay stores the scaled value, matching M2.5.
                    let value = if entry.raw_type == RawType::Cc7To14 {
                        cc7_to14(data2)
                    } else {
                        (data2 & 0x7f) as i16
                    };
                    self.cc7_value[index] = value;
                    self.cc7_valid[index] = true;
                    value
                }
                RawType::NoteStatePair => {
                    let slot = &mut self.slots[entry.pair_slot as usize];
                    let bit = if entry.flags & INPUT_FLAG_PAIR_MEMBER_B != 0 {
                        0x02
                    } else {
                        0x01
                    };
                    if pressed {
                        slot.pair_bits |= bit;
                    } else {
                        slot.pair_bits &= !bit;
                    }
                    let value = entry.lut[(slot.pair_bits & 0x03) as usize];
                    if value < 0 {
                        return None;
                    }
                    value as i16
                }
            };

            return Some(ProfileEvent {
                semantic_type: entry.semantic_type,
                semantic_id: entry.semantic_id,
                value,
            });
        }

        None
    }

    pub fn emit_snapshot<F>(&self, profile: &Profile<'_>, mut emit: F) -> usize
    where
        F: FnMut(ProfileEvent) -> bool,
    {
        let mut count = 0;

        for index in 0..profile.input_count {
            let Some(entry) = profile.input(index) else {
                continue;
            };
            if entry.flags & INPUT_FLAG_REPLAY == 0 {
                continue;
            }

            let value = match entry.raw_type {
                RawType::Cc14Msb => self.slots[entry.pair_slot as usize].value(),
                RawType::Cc7Abs | RawType::Cc7To14 if self.cc7_valid[index] => {
                    Some(self.cc7_value[index])
                }
                _ => None,
            };

            let Some(value) = value else {
                continue;
            };

            let event = ProfileEvent {
                semantic_type: entry.semantic_type,
                semantic_id: entry.semantic_id,
                value,
            };
            if !emit(event) {
                return count;
            }
            count += 1;
        }

        count
    }
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;

    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }

    crc ^ 0xffff_ffff
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruzina_controller_core::{LedState, SemanticLedId};

    fn minimal_profile() -> [u8; HEADER_SIZE + INPUT_ENTRY_SIZE + OUTPUT_ENTRY_SIZE] {
        let mut bytes = [0u8; HEADER_SIZE + INPUT_ENTRY_SIZE + OUTPUT_ENTRY_SIZE];
        bytes[0..4].copy_from_slice(S3CP_MAGIC);
        bytes[4..6].copy_from_slice(&S3CP_VERSION_LEGACY.to_le_bytes());
        bytes[6..8].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
        let profile_size = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&profile_size.to_le_bytes());
        bytes[16..18].copy_from_slice(&0x1234u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&0x5678u16.to_le_bytes());
        bytes[24..26].copy_from_slice(&1u16.to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[29] = 2;

        let input = HEADER_SIZE;
        bytes[input] = 0x90;
        bytes[input + 1] = 0x0b;
        bytes[input + 2] = RawType::NoteButton as u8;
        bytes[input + 3] = PAIR_SLOT_NONE;
        bytes[input + 4] = 1;
        bytes[input + 5] = 0x10;

        let output = HEADER_SIZE + INPUT_ENTRY_SIZE;
        bytes[output] = 1;
        bytes[output + 1] = 0;
        bytes[output + 2] = OutputKind::NoteOnOff as u8;
        bytes[output + 3] = 0x90;
        bytes[output + 4] = 0x0b;
        bytes[output + 5] = 0x00;
        bytes[output + 6] = 0x7f;
        bytes[output + 7] = 0x7f;

        let checksum = crc32(&bytes[16..]);
        bytes[12..16].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    #[test]
    fn crc_matches_ieee_reference_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn parses_and_executes_minimal_profile() {
        let bytes = minimal_profile();
        let profile = Profile::parse(&bytes).unwrap();
        assert_eq!(profile.vid(), 0x1234);
        assert_eq!(profile.pid(), 0x5678);
        assert_eq!(profile.input_count(), 1);
        assert_eq!(profile.output_count(), 1);

        let mut runtime = ProfileRuntime::new();
        assert_eq!(
            runtime.process(&profile, 0x90, 0x0b, 0x7f),
            Some(ProfileEvent {
                semantic_type: 1,
                semantic_id: 0x10,
                value: 1,
            })
        );
        assert_eq!(profile.map_led(1, 0, 1), Some([0x90, 0x0b, 0x7f]));
        assert_eq!(
            profile.map_led_update(LedUpdate {
                led: SemanticLedId::PLAY,
                deck: Some(DeckId::One),
                state: LedState::On,
            }),
            Some([0x90, 0x0b, 0x7f])
        );
        assert_eq!(
            profile.map_led_update(LedUpdate {
                led: SemanticLedId::PLAY,
                deck: Some(DeckId::Two),
                state: LedState::On,
            }),
            None
        );
    }

    #[test]
    fn typed_led_mapping_requires_exact_deck_scope() {
        let mut bytes = minimal_profile();
        let output = HEADER_SIZE + INPUT_ENTRY_SIZE;
        bytes[output + 1] = DECK_ANY;
        let checksum = crc32(&bytes[16..]);
        bytes[12..16].copy_from_slice(&checksum.to_le_bytes());

        let profile = Profile::parse(&bytes).unwrap();
        assert_eq!(
            profile.map_led_update(LedUpdate {
                led: SemanticLedId::PLAY,
                deck: None,
                state: LedState::On,
            }),
            Some([0x90, 0x0b, 0x7f])
        );
        assert_eq!(
            profile.map_led_update(LedUpdate {
                led: SemanticLedId::PLAY,
                deck: Some(DeckId::One),
                state: LedState::On,
            }),
            None
        );
    }

    #[test]
    fn rejects_s3cp_v1_instead_of_aliasing_new_led_ids() {
        let mut bytes = minimal_profile();
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        let checksum = crc32(&bytes[16..]);
        bytes[12..16].copy_from_slice(&checksum.to_le_bytes());
        assert!(matches!(Profile::parse(&bytes), Err(ParseError::Version)));
    }

    #[test]
    fn rejects_crc_mismatch() {
        let mut bytes = minimal_profile();
        bytes[20] ^= 0x01;
        assert!(matches!(Profile::parse(&bytes), Err(ParseError::Crc)));
    }

    // RP-02 version contract. Header/entry offsets and expected values are
    // literal M2.5 schema numbers, not derived from this parser's constants.
    use std::vec::Vec;

    fn input(status: u8, data1: u8, raw: u8, sem_type: u8, sem_id: u8) -> [u8; 16] {
        let mut entry = [0u8; 16];
        entry[..6].copy_from_slice(&[status, data1, raw, 0xff, sem_type, sem_id]);
        entry[12..].copy_from_slice(&[0xff; 4]);
        entry
    }

    fn output(led: u8, kind: u8, status: u8, data1: u8, scale: u16) -> [u8; 12] {
        let mut entry = [led, 0, kind, status, data1, 0x00, 0x7f, 0x40, 0, 0, 0, 0];
        entry[8..10].copy_from_slice(&scale.to_le_bytes());
        entry
    }

    fn blob(
        version: u16,
        flags: u32,
        inputs: &[[u8; 16]],
        outputs: &[[u8; 12]],
        sysex: &[u8],
    ) -> Vec<u8> {
        let mut bytes = std::vec![0u8; 32];
        bytes[0..4].copy_from_slice(b"S3CP");
        bytes[4..6].copy_from_slice(&version.to_le_bytes());
        bytes[6..8].copy_from_slice(&32u16.to_le_bytes());
        bytes[16..18].copy_from_slice(&0x1209u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&0x0002u16.to_le_bytes());
        bytes[20..24].copy_from_slice(&flags.to_le_bytes());
        bytes[24..26].copy_from_slice(&(inputs.len() as u16).to_le_bytes());
        bytes[26..28].copy_from_slice(&(outputs.len() as u16).to_le_bytes());
        bytes[29] = 2;
        bytes[30..32].copy_from_slice(&(sysex.len() as u16).to_le_bytes());
        inputs
            .iter()
            .for_each(|entry| bytes.extend_from_slice(entry));
        outputs
            .iter()
            .for_each(|entry| bytes.extend_from_slice(entry));
        bytes.extend_from_slice(sysex);
        seal(&mut bytes);
        bytes
    }

    fn seal(bytes: &mut [u8]) {
        let size = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&size.to_le_bytes());
        let checksum = crc32(&bytes[16..]);
        bytes[12..16].copy_from_slice(&checksum.to_le_bytes());
    }

    const SYSEX: [u8; 5] = [0xf0, 0x00, 0x20, 0x7f, 0xf7];

    #[test]
    fn accepts_exactly_versions_two_three_and_four() {
        let button = input(0x90, 0x0b, 0, 0x01, 0x10);
        for version in [2u16, 3, 4] {
            let bytes = blob(version, 0, &[button], &[], &[]);
            assert_eq!(Profile::parse(&bytes).unwrap().version(), version);
        }
        for version in [0u16, 1, 5, 0x0200] {
            let bytes = blob(version, 0, &[button], &[], &[]);
            assert_eq!(Profile::parse(&bytes).err(), Some(ParseError::Version));
        }
    }

    #[test]
    fn new_raw_types_and_filter_capability_require_their_version() {
        let to14 = input(0xb0, 0x13, 8, 0x03, 0x50);
        let select = input(0x90, 0x20, 9, 0x01, 0x77);
        assert_eq!(
            Profile::parse(&blob(2, 0, &[to14], &[], &[])).err(),
            Some(ParseError::Bounds)
        );
        assert!(Profile::parse(&blob(3, 0, &[to14], &[], &[])).is_ok());
        assert_eq!(
            Profile::parse(&blob(3, 0, &[select], &[], &[])).err(),
            Some(ParseError::Bounds)
        );
        assert!(Profile::parse(&blob(4, 0, &[select, to14], &[], &[])).is_ok());
        assert_eq!(
            Profile::parse(&blob(4, 0, &[input(0, 0, 10, 1, 0x10)], &[], &[])).err(),
            Some(ParseError::Bounds)
        );

        for version in [2u16, 3] {
            assert_eq!(
                Profile::parse(&blob(version, 1 << 4, &[], &[], &[])).err(),
                Some(ParseError::Bounds)
            );
        }
        let v4 = blob(4, 1 << 4, &[], &[], &[]);
        assert!(Profile::parse(&v4).unwrap().channel_filter_always());
        assert!(
            !Profile::parse(&blob(4, 0, &[], &[], &[]))
                .unwrap()
                .channel_filter_always()
        );
    }

    #[test]
    fn v2_and_v3_ignore_reserved_sysex_length_and_led_scale_fields() {
        let led = output(1, 0, 0x90, 0x0b, 300);
        for version in [2u16, 3] {
            let mut bytes = blob(version, 0, &[], &[led], &[]);
            bytes[30..32].copy_from_slice(&7u16.to_le_bytes());
            seal(&mut bytes);
            let profile = Profile::parse(&bytes).unwrap();
            assert!(profile.init_sysex().is_empty());
            assert_eq!(profile.output(0).unwrap().value_scale, 0);
            assert_eq!(profile.map_led(1, 0, 1), Some([0x90, 0x0b, 0x7f]));
        }
    }

    #[test]
    fn note_select_and_cc7_to14_entries_are_strictly_bounded() {
        let mut note_select_cases = Vec::new();
        note_select_cases.push(input(0xb0, 0x20, 9, 0x01, 0x77));
        let mut with_slot = input(0x90, 0x20, 9, 0x01, 0x77);
        with_slot[3] = 0;
        note_select_cases.push(with_slot);
        let mut with_flags = input(0x90, 0x20, 9, 0x01, 0x77);
        with_flags[6] = 0x01;
        note_select_cases.push(with_flags);
        let mut with_mask = input(0x90, 0x20, 9, 0x01, 0x77);
        with_mask[10] = 0x80;
        note_select_cases.push(with_mask);
        let mut negative = input(0x90, 0x20, 9, 0x01, 0x77);
        negative[8..10].copy_from_slice(&(-1i16).to_le_bytes());
        note_select_cases.push(negative);
        for entry in note_select_cases {
            let bytes = blob(4, 0, &[entry], &[], &[]);
            assert_eq!(Profile::parse(&bytes).err(), Some(ParseError::Bounds));
        }

        let mut to14_cases = Vec::new();
        to14_cases.push(input(0x90, 0x13, 8, 0x03, 0x50));
        to14_cases.push(input(0xb0, 0x13, 8, 0x01, 0x50));
        let mut with_slot = input(0xb0, 0x13, 8, 0x03, 0x50);
        with_slot[3] = 0;
        to14_cases.push(with_slot);
        let mut member_b = input(0xb0, 0x13, 8, 0x03, 0x50);
        member_b[6] = 0x02;
        to14_cases.push(member_b);
        for entry in to14_cases {
            let bytes = blob(4, 0, &[entry], &[], &[]);
            assert_eq!(Profile::parse(&bytes).err(), Some(ParseError::Bounds));
        }

        let mut replay = input(0xb0, 0x13, 8, 0x03, 0x50);
        replay[6] = 0x01;
        assert!(Profile::parse(&blob(3, 0, &[replay], &[], &[])).is_ok());
    }

    #[test]
    fn v4_init_sysex_is_bounded_framed_and_covered_by_size_and_crc() {
        let profile_bytes = blob(4, 0, &[], &[], &SYSEX);
        let profile = Profile::parse(&profile_bytes).unwrap();
        assert_eq!(profile.init_sysex(), &SYSEX);

        let mut max = std::vec![0x00u8; 128];
        max[0] = 0xf0;
        max[127] = 0xf7;
        assert_eq!(
            Profile::parse(&blob(4, 0, &[], &[], &max))
                .unwrap()
                .init_sysex()
                .len(),
            128
        );
        let mut too_long = std::vec![0x00u8; 129];
        too_long[0] = 0xf0;
        too_long[128] = 0xf7;

        for invalid in [
            &[0xf0][..],
            &[0xf0, 0x01, 0x02][..],
            &[0x00, 0x01, 0xf7][..],
            &[0xf0, 0x80, 0xf7][..],
            &too_long[..],
        ] {
            assert_eq!(
                Profile::parse(&blob(4, 0, &[], &[], invalid)).err(),
                Some(ParseError::Bounds)
            );
        }

        let mut trailing = blob(4, 0, &[], &[], &SYSEX);
        trailing.push(0xf7);
        seal(&mut trailing);
        assert_eq!(Profile::parse(&trailing).err(), Some(ParseError::Size));

        let mut short = blob(4, 0, &[], &[], &SYSEX);
        short.pop();
        seal(&mut short);
        assert_eq!(Profile::parse(&short).err(), Some(ParseError::Size));

        let mut corrupted = profile_bytes.clone();
        let last = corrupted.len() - 2;
        corrupted[last] ^= 0x01;
        assert_eq!(Profile::parse(&corrupted).err(), Some(ParseError::Crc));
    }

    #[test]
    fn cc7_to14_keeps_detents_endpoints_and_monotonic_order() {
        for (input, expected) in [
            (0u8, 0i16),
            (1, 128),
            (63, 8064),
            (64, 8192),
            (65, 8322),
            (126, 16253),
            (127, 16383),
            (0xff, 16383),
        ] {
            assert_eq!(cc7_to14(input), expected, "data2 {input}");
        }
        for value in 1..=127u8 {
            assert!(cc7_to14(value) > cc7_to14(value - 1));
        }
    }

    #[test]
    fn note_select_emits_on_press_only_and_cc7_to14_replays_scaled_value() {
        let mut to14 = input(0xb0, 0x13, 8, 0x03, 0x50);
        to14[6] = 0x01;
        let mut select = input(0x94, 0x12, 9, 0x01, 0x77);
        select[8..10].copy_from_slice(&2i16.to_le_bytes());
        let bytes = blob(4, 0, &[select, to14], &[], &[]);
        let profile = Profile::parse(&bytes).unwrap();
        let mut runtime = ProfileRuntime::new();

        assert_eq!(
            runtime.process(&profile, 0x94, 0x12, 0x01),
            Some(ProfileEvent {
                semantic_type: 0x01,
                semantic_id: 0x77,
                value: 2,
            })
        );
        assert_eq!(runtime.process(&profile, 0x94, 0x12, 0x00), None);
        assert_eq!(
            runtime.process(&profile, 0xb0, 0x13, 64).unwrap().value,
            8192
        );

        let mut replayed = Vec::new();
        runtime.emit_snapshot(&profile, |event| {
            replayed.push(event);
            true
        });
        assert_eq!(
            replayed,
            [ProfileEvent {
                semantic_type: 0x03,
                semantic_id: 0x50,
                value: 8192,
            }]
        );
    }

    #[test]
    fn v4_led_value_scale_applies_after_state_selection_and_clamps() {
        let outputs = [
            output(1, 0, 0x90, 0x0b, 64),
            output(5, 1, 0xb0, 0x02, 254),
            output(0, 0, 0x90, 0x0c, 0),
        ];
        let bytes = blob(4, 0, &[], &outputs, &[]);
        let profile = Profile::parse(&bytes).unwrap();

        // on 0x7f * 64 / 127 = 64; blink 0x40 * 64 / 127 = 32; off stays 0.
        assert_eq!(profile.map_led(1, 0, 1), Some([0x90, 0x0b, 64]));
        assert_eq!(profile.map_led(1, 0, 2), Some([0x90, 0x0b, 32]));
        assert_eq!(profile.map_led(1, 0, 0), Some([0x90, 0x0b, 0]));
        // CC passthrough 0x30 * 254 / 127 = 96; 0x50 doubles past 127 and clamps.
        assert_eq!(profile.map_led(5, 0, 0x30), Some([0xb0, 0x02, 96]));
        assert_eq!(profile.map_led(5, 0, 0x50), Some([0xb0, 0x02, 127]));
        assert_eq!(profile.map_led(0, 0, 1), Some([0x90, 0x0c, 0x7f]));
        assert_eq!(
            profile.map_led_update(LedUpdate {
                led: SemanticLedId::PLAY,
                deck: Some(DeckId::One),
                state: LedState::On,
            }),
            Some([0x90, 0x0b, 64])
        );
    }
}
