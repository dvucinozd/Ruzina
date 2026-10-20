#![no_std]
#![forbid(unsafe_code)]

mod registry;
mod semantic;

pub use registry::*;
pub use semantic::{SemanticAdapterError, adapt_profile_event};

#[cfg(test)]
extern crate std;

use pajoniiir_controller_core::{DeckId, LedUpdate};

pub const S3CP_MAGIC: &[u8; 4] = b"S3CP";
pub const S3CP_VERSION: u16 = 2;
pub const HEADER_SIZE: usize = 32;
pub const INPUT_ENTRY_SIZE: usize = 16;
pub const OUTPUT_ENTRY_SIZE: usize = 12;

pub const MAX_INPUTS: usize = 320;
pub const MAX_OUTPUTS: usize = 160;
pub const MAX_PAIR_SLOTS: usize = 40;

pub const INPUT_FLAG_REPLAY: u16 = 0x0001;
pub const INPUT_FLAG_PAIR_MEMBER_B: u16 = 0x0002;

pub const DECK_ANY: u8 = 0xff;
pub const PAIR_SLOT_NONE: u8 = 0xff;

pub const PROFILE_FLAG_LED_FEEDBACK: u32 = 1 << 0;
pub const PROFILE_FLAG_USB_AUDIO: u32 = 1 << 1;
pub const PROFILE_FLAG_JOG_TOUCH: u32 = 1 << 2;
pub const PROFILE_FLAG_PITCH_14BIT: u32 = 1 << 3;

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
            _ => None,
        }
    }

    fn needs_pair_slot(self) -> bool {
        matches!(self, Self::Cc14Msb | Self::Cc14Lsb | Self::NoteStatePair)
    }
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
    vid: u16,
    pid: u16,
    flags: u32,
    input_count: usize,
    output_count: usize,
    pair_slot_count: usize,
    decks: u8,
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

        if version != S3CP_VERSION || header_size != HEADER_SIZE {
            return Err(ParseError::Version);
        }
        if profile_size != blob.len() || profile_size < HEADER_SIZE {
            return Err(ParseError::Size);
        }
        if crc32(&blob[16..profile_size]) != expected_crc {
            return Err(ParseError::Crc);
        }

        let input_count = read_u16(blob, 24) as usize;
        let output_count = read_u16(blob, 26) as usize;
        let pair_slot_count = blob[28] as usize;

        if input_count > MAX_INPUTS
            || output_count > MAX_OUTPUTS
            || pair_slot_count > MAX_PAIR_SLOTS
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
            .ok_or(ParseError::Size)?;

        if need != blob.len() {
            return Err(ParseError::Size);
        }

        let profile = Self {
            blob,
            vid: read_u16(blob, 16),
            pid: read_u16(blob, 18),
            flags: read_u32(blob, 20),
            input_count,
            output_count,
            pair_slot_count,
            decks: blob[29],
        };

        for index in 0..profile.input_count {
            let entry = profile.input(index).ok_or(ParseError::Bounds)?;
            if entry.raw_type.needs_pair_slot()
                && (entry.pair_slot == PAIR_SLOT_NONE
                    || entry.pair_slot as usize >= profile.pair_slot_count)
            {
                return Err(ParseError::Bounds);
            }
        }

        for index in 0..profile.output_count {
            if profile.output(index).is_none() {
                return Err(ParseError::Bounds);
            }
        }

        Ok(profile)
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

            let value = match entry.kind {
                OutputKind::CcValue => state & 0x7f,
                OutputKind::NoteOnOff if state == 0 => entry.off_value,
                OutputKind::NoteOnOff if state == 2 => entry.blink_value,
                OutputKind::NoteOnOff => entry.on_value,
            };
            return Some([entry.status, entry.data1, value]);
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

            let state = update.state.wire_value();
            let value = match entry.kind {
                OutputKind::CcValue => state & 0x7f,
                OutputKind::NoteOnOff if state == 0 => entry.off_value,
                OutputKind::NoteOnOff if state == 2 => entry.blink_value,
                OutputKind::NoteOnOff => entry.on_value,
            };
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
                RawType::Cc7Abs => {
                    let value = (data2 & 0x7f) as i16;
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
                RawType::Cc7Abs if self.cc7_valid[index] => Some(self.cc7_value[index]),
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
    use pajoniiir_controller_core::{LedState, SemanticLedId};

    fn minimal_profile() -> [u8; HEADER_SIZE + INPUT_ENTRY_SIZE + OUTPUT_ENTRY_SIZE] {
        let mut bytes = [0u8; HEADER_SIZE + INPUT_ENTRY_SIZE + OUTPUT_ENTRY_SIZE];
        bytes[0..4].copy_from_slice(S3CP_MAGIC);
        bytes[4..6].copy_from_slice(&S3CP_VERSION.to_le_bytes());
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
}
