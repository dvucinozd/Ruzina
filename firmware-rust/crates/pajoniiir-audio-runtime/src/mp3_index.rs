//! Bounded MP3 frame index for predictable cold seeks and scanned duration
//! (RP-08). A header walk records the exact byte offset of every `stride`-th
//! Layer III frame without decoding; the stride doubles when the table fills,
//! so a stream of any length fits. Seeking still runs through the codec's own
//! reservoir preroll, so output stays sample-identical to a linear decode.
//!
//! The walk follows the decoder's frame numbering: frame 0 is the first
//! confirmed frame after any ID3v2 tags, including a Xing/Info/VBRI tag
//! frame. Junk between frames ends the walk; frames already indexed stay
//! usable, but no duration is reported.

pub const MP3_INDEX_ENTRIES: usize = 2048;
const INITIAL_STRIDE: u32 = 4;
/// The first frame must be found this close to the audio start.
const SYNC_SEARCH_BYTES: u64 = 64 * 1024;
const HEADER_BYTES: usize = 4;
/// Bytes of the first frame that hold a Xing/Info or VBRI tag identifier.
const TAG_PROBE_BYTES: usize = 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mp3FrameHeader {
    /// MPEG version bits: 3 = MPEG-1, 2 = MPEG-2, 0 = MPEG-2.5.
    version: u8,
    sample_rate_index: u8,
    pub bitrate_kbps: u16,
    pub sample_rate_hz: u32,
    pub channels: u8,
    /// Frame length in bytes, header included.
    pub size: u32,
    /// PCM frames per MP3 frame.
    pub samples: u32,
}

impl Mp3FrameHeader {
    /// Layer III only; free-format, reserved and invalid fields are refused
    /// (the decoder is built without Layer I/II).
    pub fn parse(bytes: [u8; 4]) -> Option<Self> {
        if bytes[0] != 0xff || bytes[1] & 0xe0 != 0xe0 {
            return None;
        }
        let version = (bytes[1] >> 3) & 3;
        let layer = (bytes[1] >> 1) & 3;
        let bitrate_index = usize::from(bytes[2] >> 4);
        let sample_rate_index = (bytes[2] >> 2) & 3;
        if version == 1 || layer != 1 || bitrate_index == 0 || bitrate_index == 15 {
            return None;
        }
        if sample_rate_index == 3 {
            return None;
        }
        const MPEG1: [u16; 15] = [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ];
        const MPEG2: [u16; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
        const RATES: [u32; 3] = [44_100, 48_000, 32_000];
        let mpeg1 = version == 3;
        let bitrate_kbps = if mpeg1 { MPEG1 } else { MPEG2 }[bitrate_index];
        let sample_rate_hz = RATES[usize::from(sample_rate_index)]
            >> match version {
                3 => 0,
                2 => 1,
                _ => 2,
            };
        let padding = u32::from((bytes[2] >> 1) & 1);
        let coefficient = if mpeg1 { 144 } else { 72 };
        Some(Self {
            version,
            sample_rate_index,
            bitrate_kbps,
            sample_rate_hz,
            channels: if bytes[3] >> 6 == 3 { 1 } else { 2 },
            size: coefficient * u32::from(bitrate_kbps) * 1000 / sample_rate_hz + padding,
            samples: if mpeg1 { 1152 } else { 576 },
        })
    }

    fn compatible(self, other: Self) -> bool {
        self.version == other.version && self.sample_rate_index == other.sample_rate_index
    }

    /// Bytes between the header and a Xing/Info tag.
    fn side_information(self) -> usize {
        match (self.version == 3, self.channels) {
            (true, 1) => 17,
            (true, _) => 32,
            (false, 1) => 9,
            (false, _) => 17,
        }
    }
}

/// How the decoder positions a seek in this stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mp3TagKind {
    /// No Xing/Info/VBRI frame.
    Plain,
    /// `Info`: constant bitrate, byte positions are exact.
    Constant,
    /// `Xing` or `VBRI`: the decoder trusts the frame index it computed.
    Variable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scan {
    Idle,
    /// Searching for a sync word from `at`.
    Search {
        at: u64,
    },
    /// A candidate at `candidate` waits for its successor's header.
    Confirm {
        candidate: u64,
        header: Mp3FrameHeader,
    },
    /// Reading the first frame's tag identifier.
    Tag,
    /// Walking frame headers from `at`.
    Walk {
        at: u64,
    },
    Complete,
    /// Junk, a broken header or an unsupported stream ended the walk.
    Stopped,
}

#[derive(Clone)]
pub struct Mp3SeekIndex {
    offsets: [u32; MP3_INDEX_ENTRIES],
    len: usize,
    stride: u32,
    frames: u64,
    first: Option<(u64, Mp3FrameHeader)>,
    audio: Option<Mp3FrameHeader>,
    kind: Mp3TagKind,
    variable_bitrate: bool,
    audio_start: u64,
    end: u64,
    state: Scan,
}

impl Default for Mp3SeekIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl Mp3SeekIndex {
    pub const fn new() -> Self {
        Self {
            offsets: [0; MP3_INDEX_ENTRIES],
            len: 0,
            stride: INITIAL_STRIDE,
            frames: 0,
            first: None,
            audio: None,
            kind: Mp3TagKind::Plain,
            variable_bitrate: false,
            audio_start: 0,
            end: 0,
            state: Scan::Idle,
        }
    }

    pub fn reset(&mut self) {
        self.len = 0;
        self.stride = INITIAL_STRIDE;
        self.frames = 0;
        self.first = None;
        self.audio = None;
        self.kind = Mp3TagKind::Plain;
        self.variable_bitrate = false;
        self.state = Scan::Idle;
    }

    /// Starts a walk of `[audio_start, file_bytes)`. Files of 4 GiB or more
    /// are not indexed.
    pub fn begin(&mut self, audio_start: u64, file_bytes: u64) {
        self.reset();
        self.audio_start = audio_start;
        self.end = file_bytes;
        self.state = if file_bytes > u64::from(u32::MAX) || audio_start >= file_bytes {
            Scan::Stopped
        } else {
            Scan::Search { at: audio_start }
        };
    }

    pub const fn started(&self) -> bool {
        !matches!(self.state, Scan::Idle)
    }

    /// The next byte range the walk needs, at most `maximum` bytes.
    pub fn next_read(&self, maximum: usize) -> Option<(u64, usize)> {
        let at = match self.state {
            Scan::Search { at } | Scan::Walk { at } => at,
            Scan::Confirm { candidate, header } => candidate + u64::from(header.size),
            Scan::Tag => self.first?.0,
            Scan::Idle | Scan::Complete | Scan::Stopped => return None,
        };
        let len = (self.end - at.min(self.end)).min(maximum as u64) as usize;
        (len != 0).then_some((at, len))
    }

    /// Supplies the bytes read at `offset`, which must be the last request.
    pub fn provide(&mut self, offset: u64, bytes: &[u8]) -> bool {
        if self
            .next_read(usize::MAX)
            .is_none_or(|(at, _)| at != offset)
        {
            return false;
        }
        match self.state {
            Scan::Search { at } => self.search(at, bytes),
            Scan::Confirm { candidate, header } => {
                let next = bytes
                    .first_chunk::<4>()
                    .and_then(|bytes| Mp3FrameHeader::parse(*bytes))
                    .filter(|next| next.compatible(header));
                self.state = match next {
                    Some(_) => {
                        self.first = Some((candidate, header));
                        Scan::Tag
                    }
                    None => Scan::Search { at: candidate + 1 },
                };
            }
            Scan::Tag => self.tag(bytes),
            Scan::Walk { at } => self.walk(at, bytes),
            Scan::Idle | Scan::Complete | Scan::Stopped => return false,
        }
        true
    }

    fn search(&mut self, at: u64, bytes: &[u8]) {
        if at - self.audio_start > SYNC_SEARCH_BYTES || bytes.len() < HEADER_BYTES {
            self.state = Scan::Stopped;
            return;
        }
        for index in 0..=bytes.len() - HEADER_BYTES {
            let header = bytes[index..index + 4].try_into().ok();
            if let Some(header) = header.and_then(Mp3FrameHeader::parse) {
                let candidate = at + index as u64;
                self.state = if candidate + u64::from(header.size) + HEADER_BYTES as u64 > self.end
                {
                    Scan::Stopped
                } else {
                    Scan::Confirm { candidate, header }
                };
                return;
            }
        }
        self.state = Scan::Search {
            at: at + (bytes.len() - HEADER_BYTES + 1) as u64,
        };
    }

    fn tag(&mut self, bytes: &[u8]) {
        let Some((first, header)) = self.first else {
            self.state = Scan::Stopped;
            return;
        };
        let probe = &bytes[..bytes.len().min(TAG_PROBE_BYTES + HEADER_BYTES)];
        let at = HEADER_BYTES + header.side_information();
        if probe.len() < TAG_PROBE_BYTES && (probe.len() as u64) < self.end - first {
            // Too short a read to tell a tag frame from audio: never guess.
            self.state = Scan::Stopped;
            return;
        }
        self.kind = match (probe.get(at..at + 4), probe.get(36..40)) {
            (Some(b"Xing"), _) | (_, Some(b"VBRI")) => Mp3TagKind::Variable,
            (Some(b"Info"), _) => Mp3TagKind::Constant,
            _ => Mp3TagKind::Plain,
        };
        self.state = Scan::Walk { at: first };
    }

    fn walk(&mut self, mut at: u64, bytes: &[u8]) {
        let start = at;
        let Some((_, first)) = self.first else {
            self.state = Scan::Stopped;
            return;
        };
        while at + HEADER_BYTES as u64 <= start + bytes.len() as u64 {
            let index = (at - start) as usize;
            let raw: [u8; 4] = bytes[index..index + 4].try_into().expect("four bytes");
            let Some(header) = Mp3FrameHeader::parse(raw).filter(|h| h.compatible(first)) else {
                self.finish_at(at, &bytes[index..]);
                return;
            };
            if at + u64::from(header.size) > self.end {
                // A truncated last frame: the decoder cannot produce it.
                self.state = Scan::Complete;
                return;
            }
            self.record(at, header);
            at += u64::from(header.size);
            if at == self.end {
                self.state = Scan::Complete;
                return;
            }
        }
        self.state = Scan::Walk { at };
        if self.end - at < HEADER_BYTES as u64 {
            self.state = Scan::Complete;
        }
    }

    /// Trailing ID3v1/APE/Lyrics3 metadata ends the stream cleanly; anything
    /// else stops the walk without a duration.
    fn finish_at(&mut self, at: u64, bytes: &[u8]) {
        let trailing = (bytes.starts_with(b"TAG") && self.end - at == 128)
            || bytes.starts_with(b"APETAGEX")
            || bytes.starts_with(b"LYRICS");
        self.state = if trailing {
            Scan::Complete
        } else {
            Scan::Stopped
        };
    }

    fn record(&mut self, at: u64, header: Mp3FrameHeader) {
        let tagged = self.kind != Mp3TagKind::Plain;
        if !(tagged && self.frames == 0) {
            match self.audio {
                None => self.audio = Some(header),
                Some(audio) if audio.bitrate_kbps != header.bitrate_kbps => {
                    self.variable_bitrate = true;
                }
                Some(_) => {}
            }
        }
        if self.frames.is_multiple_of(u64::from(self.stride)) {
            if self.len == MP3_INDEX_ENTRIES {
                for entry in 0..MP3_INDEX_ENTRIES / 2 {
                    self.offsets[entry] = self.offsets[entry * 2];
                }
                self.len = MP3_INDEX_ENTRIES / 2;
                self.stride *= 2;
            }
            if self.frames.is_multiple_of(u64::from(self.stride)) {
                self.offsets[self.len] = at as u32;
                self.len += 1;
            }
        }
        self.frames += 1;
    }

    pub const fn kind(&self) -> Mp3TagKind {
        self.kind
    }

    /// An audio frame's bitrate differed from the first audio frame.
    pub const fn variable_bitrate(&self) -> bool {
        self.variable_bitrate
    }

    pub const fn complete(&self) -> bool {
        matches!(self.state, Scan::Complete)
    }

    pub const fn stride(&self) -> u32 {
        self.stride
    }

    /// MP3 frames walked so far.
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    pub fn samples_per_frame(&self) -> Option<u64> {
        self.first.map(|(_, header)| u64::from(header.samples))
    }

    /// File offset of frame 0.
    pub fn first_offset(&self) -> Option<u64> {
        self.first.map(|(offset, _)| offset)
    }

    /// The indexed frame at or before `frame`, with its exact file offset.
    pub fn at_or_before(&self, frame: u64) -> Option<(u64, u64)> {
        if self.len == 0 || frame >= self.frames {
            return None;
        }
        let entry = (frame / u64::from(self.stride)).min(self.len as u64 - 1);
        Some((
            entry * u64::from(self.stride),
            u64::from(self.offsets[entry as usize]),
        ))
    }

    /// Output frames of a complete untagged walk: every MP3 frame decodes to
    /// `samples` PCM frames and nothing is trimmed. Tagged streams report
    /// their exact length through the decoder instead.
    pub fn scanned_pcm_frames(&self) -> Option<u64> {
        (self.complete() && self.kind == Mp3TagKind::Plain)
            .then(|| {
                self.samples_per_frame()
                    .map(|samples| self.frames * samples)
            })
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{vec, vec::Vec};

    /// MPEG-1 Layer III, 44.1 kHz, stereo, no CRC.
    fn header(bitrate_index: u8, padding: bool) -> [u8; 4] {
        [
            0xff,
            0xfb,
            bitrate_index << 4 | u8::from(padding) << 1,
            0x00,
        ]
    }

    fn frame(bitrate_index: u8, tag: Option<&[u8; 4]>) -> Vec<u8> {
        let raw = header(bitrate_index, false);
        let size = Mp3FrameHeader::parse(raw).unwrap().size as usize;
        let mut bytes = vec![0u8; size];
        bytes[..4].copy_from_slice(&raw);
        if let Some(tag) = tag {
            bytes[36..40].copy_from_slice(tag);
        }
        bytes
    }

    fn scan(file: &[u8], audio_start: u64, chunk: usize) -> Mp3SeekIndex {
        let mut index = Mp3SeekIndex::new();
        index.begin(audio_start, file.len() as u64);
        for _ in 0..1_000_000 {
            let Some((offset, len)) = index.next_read(chunk) else {
                return index;
            };
            let at = offset as usize;
            assert!(index.provide(offset, &file[at..at + len]));
        }
        panic!("scan did not terminate");
    }

    #[test]
    fn header_sizes_follow_the_layer_iii_tables_and_refuse_other_layers() {
        let h = Mp3FrameHeader::parse(header(9, false)).unwrap();
        assert_eq!(
            (h.bitrate_kbps, h.sample_rate_hz, h.size, h.samples),
            (128, 44_100, 417, 1152)
        );
        assert_eq!(Mp3FrameHeader::parse(header(9, true)).unwrap().size, 418);
        // MPEG-2 24 kHz mono 64 kbit/s: 72 * 64000 / 24000 = 192.
        let h = Mp3FrameHeader::parse([0xff, 0xf3, 0x84, 0xc0]).unwrap();
        assert_eq!(
            (h.sample_rate_hz, h.channels, h.size, h.samples),
            (24_000, 1, 192, 576)
        );
        // Free format, bad bitrate, reserved rate, Layer II and no sync.
        for raw in [
            header(0, false),
            header(15, false),
            [0xff, 0xfb, 0x9c, 0x00],
            [0xff, 0xfd, 0x90, 0x00],
            [0xfe, 0xfb, 0x90, 0x00],
        ] {
            assert_eq!(Mp3FrameHeader::parse(raw), None, "{raw:02x?}");
        }
    }

    #[test]
    fn walk_indexes_exact_offsets_and_decimates_when_full() {
        let mut file = b"junk".to_vec();
        let mut offsets = Vec::new();
        for i in 0..(MP3_INDEX_ENTRIES as u32 * INITIAL_STRIDE + 37) {
            offsets.push(file.len() as u64);
            file.extend(frame(if i % 3 == 0 { 9 } else { 11 }, None));
        }
        for chunk in [512, 40, 4096] {
            let index = scan(&file, 0, chunk);
            assert!(index.complete());
            assert_eq!(index.frames(), offsets.len() as u64);
            assert_eq!(index.stride(), INITIAL_STRIDE * 2);
            assert_eq!(index.kind(), Mp3TagKind::Plain);
            assert!(index.variable_bitrate());
            assert_eq!(index.first_offset(), Some(4));
            for target in [0, 1, 7, 8, 4000, offsets.len() as u64 - 1] {
                let (frame, offset) = index.at_or_before(target).unwrap();
                assert!(frame <= target && target - frame < 8);
                assert_eq!(offset, offsets[frame as usize]);
            }
            assert_eq!(index.at_or_before(offsets.len() as u64), None);
            assert_eq!(
                index.scanned_pcm_frames(),
                Some(offsets.len() as u64 * 1152)
            );
        }
    }

    #[test]
    fn tag_frames_set_the_kind_and_are_frame_zero() {
        for (tag, kind) in [
            (b"Xing", Mp3TagKind::Variable),
            (b"Info", Mp3TagKind::Constant),
        ] {
            let mut file = frame(9, Some(tag));
            for _ in 0..10 {
                file.extend(frame(9, None));
            }
            let index = scan(&file, 0, 512);
            assert_eq!(index.kind(), kind);
            assert_eq!(index.frames(), 11);
            assert!(!index.variable_bitrate());
            // Tagged streams take their length from the decoder.
            assert_eq!(index.scanned_pcm_frames(), None);
        }
    }

    #[test]
    fn trailing_metadata_completes_but_junk_stops_without_duration() {
        let mut body = Vec::new();
        for _ in 0..6 {
            body.extend(frame(9, None));
        }
        let mut tagged = body.clone();
        tagged.extend(b"TAG");
        tagged.extend([0u8; 125]);
        assert!(scan(&tagged, 0, 512).complete());

        let mut junk = body.clone();
        junk.extend([0x55u8; 300]);
        junk.extend(frame(9, None));
        let index = scan(&junk, 0, 512);
        assert!(!index.complete());
        assert_eq!(index.frames(), 6);
        assert_eq!(index.scanned_pcm_frames(), None);
        assert_eq!(index.at_or_before(5).map(|(frame, _)| frame), Some(4));

        // A lone sync without a matching successor is not the first frame.
        let mut false_sync = vec![0xff, 0xfb, 0x90, 0x00, 0, 0];
        false_sync.extend(&body);
        assert_eq!(scan(&false_sync, 0, 512).first_offset(), Some(6));
        assert_eq!(scan(&body[..300], 0, 512).frames(), 0);
    }

    #[test]
    fn audio_start_after_id3_is_honored() {
        let mut file = vec![0xffu8; 100];
        file.extend(frame(9, None));
        file.extend(frame(9, None));
        let index = scan(&file, 100, 512);
        assert_eq!(index.first_offset(), Some(100));
        assert_eq!(index.frames(), 2);
    }
}
