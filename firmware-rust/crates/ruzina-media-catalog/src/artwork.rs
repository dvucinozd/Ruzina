//! Bounded deck cover ownership (RP-07, M2.5 `ui_artwork*.c`).
//!
//! Loading a track clears the deck's cover at once, so an old cover is never
//! shown on a new track. Each read/decode is issued a ticket bound to the
//! deck, the track identity and a per-deck sequence; only the current ticket
//! may publish. Missing, oversized, unreadable, non-baseline or undecodable
//! covers degrade to an explicit fallback that presentation renders as the
//! Ruzina emblem.

use ruzina_core::MediaTrackId;
use ruzina_media_session::MediaLease;

/// Rekordbox covers are 2-8 KiB; anything above this is refused unread.
pub const ARTWORK_MAX_FILE_BYTES: u64 = 64 * 1024;
/// Larger source dimensions are refused before decoding.
pub const ARTWORK_MAX_SOURCE_EDGE: u16 = 2048;
/// Deck-header thumbnail edge (RGB565, square).
pub const ARTWORK_DECK_EDGE: u16 = 34;
pub const ARTWORK_DECK_PIXELS: usize = ARTWORK_DECK_EDGE as usize * ARTWORK_DECK_EDGE as usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JpegVerdict {
    Baseline,
    NotJpeg,
    /// Progressive, lossless, hierarchical or arithmetic-coded frame.
    NotBaseline,
    /// Chroma layout the bounded decoder does not handle.
    Sampling,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JpegInfo {
    pub verdict: JpegVerdict,
    /// Frame marker (0xC0..=0xCF), zero when none was found.
    pub sof: u8,
    pub components: u8,
    /// `H << 4 | V` of the first three components.
    pub sampling: [u8; 3],
    pub width: u16,
    pub height: u16,
}

/// Reads markers up to the first frame header without decoding (TJpgDec's
/// `jd_prepare` checks, in its order).
pub fn probe_jpeg(jpeg: &[u8]) -> JpegInfo {
    let mut info = JpegInfo {
        verdict: JpegVerdict::NotJpeg,
        sof: 0,
        components: 0,
        sampling: [0; 3],
        width: 0,
        height: 0,
    };
    if jpeg.len() < 4 || jpeg[0] != 0xff || jpeg[1] != 0xd8 {
        return info;
    }
    let mut pos = 2;
    while pos + 4 <= jpeg.len() {
        if jpeg[pos] != 0xff {
            return info;
        }
        let marker = jpeg[pos + 1];
        if marker == 0xff {
            pos += 1;
            continue;
        }
        if marker == 0xda || marker == 0xd9 {
            return info;
        }
        let segment = usize::from(u16::from_be_bytes([jpeg[pos + 2], jpeg[pos + 3]]));
        if segment < 2 || pos + 2 + segment > jpeg.len() {
            return info;
        }
        // SOF0..SOF15 except DHT (C4), JPG (C8) and DAC (CC).
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let frame = &jpeg[pos + 4..pos + 2 + segment];
            if segment < 8 {
                return info;
            }
            info.sof = marker;
            info.height = u16::from_be_bytes([frame[1], frame[2]]);
            info.width = u16::from_be_bytes([frame[3], frame[4]]);
            info.components = frame[5];
            if 8 + 3 * usize::from(info.components) > segment {
                return info;
            }
            for component in 0..usize::from(info.components).min(3) {
                info.sampling[component] = frame[6 + 3 * component + 1];
            }
            info.verdict = judge(&info);
            return info;
        }
        pos += 2 + segment;
    }
    info
}

fn judge(info: &JpegInfo) -> JpegVerdict {
    if info.sof != 0xc0 {
        return JpegVerdict::NotBaseline;
    }
    match info.components {
        1 => JpegVerdict::Baseline,
        3 if matches!(info.sampling[0], 0x11 | 0x21 | 0x22)
            && info.sampling[1] == 0x11
            && info.sampling[2] == 0x11 =>
        {
            JpegVerdict::Baseline
        }
        _ => JpegVerdict::Sampling,
    }
}

/// Bounded baseline decoder scaling into a square RGB565 thumbnail by area
/// averaging. Implementations own their scratch; `false` publishes nothing.
pub trait ArtworkDecoder {
    fn decode(&mut self, jpeg: &[u8], info: JpegInfo, edge: u16, out: &mut [u16]) -> bool;
}

/// M2.5 `DJ_ART_ROW_PX`: Library row thumbnails.
pub const ARTWORK_ROW_EDGE: u16 = 40;
pub const ARTWORK_ROW_PIXELS: usize = ARTWORK_ROW_EDGE as usize * ARTWORK_ROW_EDGE as usize;

/// One complete cover file to an `edge`-square RGB565 thumbnail under the
/// deck rules (size, baseline JPEG, source dimensions); any refusal is the
/// reason the logo is shown instead.
pub fn decode_thumbnail<D: ArtworkDecoder>(
    jpeg: &[u8],
    edge: u16,
    out: &mut [u16],
    decoder: &mut D,
) -> Result<(), ArtworkFallback> {
    if jpeg.len() as u64 > ARTWORK_MAX_FILE_BYTES {
        return Err(ArtworkFallback::TooLarge);
    }
    let info = probe_jpeg(jpeg);
    match info.verdict {
        JpegVerdict::Baseline => {}
        JpegVerdict::NotJpeg => return Err(ArtworkFallback::NotJpeg),
        JpegVerdict::NotBaseline => return Err(ArtworkFallback::NotBaseline),
        JpegVerdict::Sampling => return Err(ArtworkFallback::Sampling),
    }
    let source = 1..=ARTWORK_MAX_SOURCE_EDGE;
    if !source.contains(&info.width) || !source.contains(&info.height) {
        return Err(ArtworkFallback::Dimensions);
    }
    if decoder.decode(jpeg, info, edge, out) {
        Ok(())
    } else {
        Err(ArtworkFallback::DecodeFailed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtworkIdentity {
    pub track: MediaTrackId,
    pub lease: MediaLease,
    pub load_generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtworkFallback {
    /// The export has no cover for this track.
    NoArtwork,
    TooLarge,
    ReadFailed,
    NotJpeg,
    NotBaseline,
    Sampling,
    /// Zero or more than [`ARTWORK_MAX_SOURCE_EDGE`] pixels on an edge.
    Dimensions,
    DecodeFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtworkView<'a> {
    Empty,
    Loading,
    Cover {
        edge: u16,
        pixels: &'a [u16],
    },
    /// Render the Ruzina emblem.
    Fallback(ArtworkFallback),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtworkTicket {
    deck: usize,
    sequence: u32,
    identity: ArtworkIdentity,
}

impl ArtworkTicket {
    pub const fn identity(&self) -> ArtworkIdentity {
        self.identity
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SlotState {
    Empty,
    Loading,
    Ready,
    Fallback(ArtworkFallback),
}

struct ArtworkSlot {
    identity: Option<ArtworkIdentity>,
    sequence: u32,
    state: SlotState,
    pixels: [u16; ARTWORK_DECK_PIXELS],
}

impl ArtworkSlot {
    const EMPTY: Self = Self {
        identity: None,
        sequence: 0,
        state: SlotState::Empty,
        pixels: [0; ARTWORK_DECK_PIXELS],
    };
}

pub struct ArtworkDecks {
    slots: [ArtworkSlot; 2],
}

impl Default for ArtworkDecks {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtworkDecks {
    pub const fn new() -> Self {
        Self {
            slots: [ArtworkSlot::EMPTY, ArtworkSlot::EMPTY],
        }
    }

    /// A track was committed to `deck`. The previous cover is dropped now;
    /// a ticket is returned when the track has a cover to read.
    pub fn begin(
        &mut self,
        deck: usize,
        identity: ArtworkIdentity,
        has_artwork: bool,
    ) -> Option<ArtworkTicket> {
        let slot = self.slots.get_mut(deck)?;
        slot.sequence = slot.sequence.wrapping_add(1);
        slot.identity = Some(identity);
        if !has_artwork {
            slot.state = SlotState::Fallback(ArtworkFallback::NoArtwork);
            return None;
        }
        slot.state = SlotState::Loading;
        Some(ArtworkTicket {
            deck,
            sequence: slot.sequence,
            identity,
        })
    }

    /// Size admission before reading: an oversized file is refused unread.
    pub fn admit_file(&mut self, ticket: ArtworkTicket, file_len: u64) -> bool {
        if file_len == 0 || file_len > ARTWORK_MAX_FILE_BYTES {
            let reason = if file_len == 0 {
                ArtworkFallback::ReadFailed
            } else {
                ArtworkFallback::TooLarge
            };
            let _ = self.fail(ticket, reason);
            return false;
        }
        self.current(ticket).is_some()
    }

    /// Probes and decodes the complete file read for `ticket`. Returns true
    /// when this ticket was still current (whether it produced a cover or a
    /// fallback); a stale ticket changes nothing.
    pub fn complete<D: ArtworkDecoder>(
        &mut self,
        ticket: ArtworkTicket,
        jpeg: &[u8],
        decoder: &mut D,
    ) -> bool {
        if self.current(ticket).is_none() {
            return false;
        }
        let slot = &mut self.slots[ticket.deck];
        match decode_thumbnail(jpeg, ARTWORK_DECK_EDGE, &mut slot.pixels, decoder) {
            Ok(()) => {
                slot.state = SlotState::Ready;
                true
            }
            Err(reason) => self.fail(ticket, reason),
        }
    }

    pub fn fail(&mut self, ticket: ArtworkTicket, reason: ArtworkFallback) -> bool {
        match self.current(ticket) {
            Some(slot) => {
                slot.state = SlotState::Fallback(reason);
                true
            }
            None => false,
        }
    }

    /// Unload or deck reset.
    pub fn clear(&mut self, deck: usize) {
        if let Some(slot) = self.slots.get_mut(deck) {
            slot.sequence = slot.sequence.wrapping_add(1);
            slot.identity = None;
            slot.state = SlotState::Empty;
        }
    }

    /// Media removal or replacement.
    pub fn invalidate(&mut self, lease: MediaLease) {
        for deck in 0..self.slots.len() {
            if self.slots[deck]
                .identity
                .is_some_and(|identity| identity.lease == lease)
            {
                self.clear(deck);
            }
        }
    }

    /// The cover for exactly this loaded identity; anything else is Empty.
    pub fn view(&self, deck: usize, identity: ArtworkIdentity) -> ArtworkView<'_> {
        let Some(slot) = self
            .slots
            .get(deck)
            .filter(|slot| slot.identity == Some(identity))
        else {
            return ArtworkView::Empty;
        };
        match slot.state {
            SlotState::Empty => ArtworkView::Empty,
            SlotState::Loading => ArtworkView::Loading,
            SlotState::Ready => ArtworkView::Cover {
                edge: ARTWORK_DECK_EDGE,
                pixels: &slot.pixels,
            },
            SlotState::Fallback(reason) => ArtworkView::Fallback(reason),
        }
    }

    fn current(&mut self, ticket: ArtworkTicket) -> Option<&mut ArtworkSlot> {
        self.slots.get_mut(ticket.deck).filter(|slot| {
            slot.sequence == ticket.sequence
                && slot.identity == Some(ticket.identity)
                && slot.state == SlotState::Loading
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruzina_media_session::{MediaGeneration, MediaSourceId};

    fn identity(track: u8, source: u32, generation: u32) -> ArtworkIdentity {
        ArtworkIdentity {
            track: MediaTrackId([track; 16]),
            lease: MediaLease {
                generation: MediaGeneration::initial(),
                source: MediaSourceId::new(source).unwrap(),
            },
            load_generation: generation,
        }
    }

    /// SOI, APP0 stub, SOFn with the given components, then SOS.
    fn jpeg(sof: u8, components: &[u8]) -> [u8; 64] {
        let mut bytes = [0u8; 64];
        bytes[..2].copy_from_slice(&[0xff, 0xd8]);
        bytes[2..8].copy_from_slice(&[0xff, 0xe0, 0x00, 0x04, 0x00, 0x00]);
        let length = 8 + 3 * components.len() as u16;
        bytes[8] = 0xff;
        bytes[9] = sof;
        bytes[10..12].copy_from_slice(&length.to_be_bytes());
        bytes[12] = 8;
        bytes[13..15].copy_from_slice(&480u16.to_be_bytes());
        bytes[15..17].copy_from_slice(&640u16.to_be_bytes());
        bytes[17] = components.len() as u8;
        for (index, sampling) in components.iter().enumerate() {
            bytes[18 + index * 3] = index as u8 + 1;
            bytes[19 + index * 3] = *sampling;
        }
        let sos = 10 + usize::from(length);
        bytes[sos..sos + 2].copy_from_slice(&[0xff, 0xda]);
        bytes
    }

    struct Fill(Option<u16>, u32);
    impl ArtworkDecoder for Fill {
        fn decode(&mut self, _: &[u8], info: JpegInfo, edge: u16, out: &mut [u16]) -> bool {
            self.1 += 1;
            assert_eq!(
                (info.width, info.height, edge),
                (640, 480, ARTWORK_DECK_EDGE)
            );
            match self.0 {
                Some(color) => {
                    out.fill(color);
                    true
                }
                None => false,
            }
        }
    }

    #[test]
    fn probe_accepts_baseline_layouts_and_names_each_refusal() {
        let info = probe_jpeg(&jpeg(0xc0, &[0x22, 0x11, 0x11]));
        assert_eq!(info.verdict, JpegVerdict::Baseline);
        assert_eq!((info.width, info.height, info.components), (640, 480, 3));
        assert_eq!(
            probe_jpeg(&jpeg(0xc0, &[0x11])).verdict,
            JpegVerdict::Baseline
        );
        assert_eq!(
            probe_jpeg(&jpeg(0xc2, &[0x22, 0x11, 0x11])).verdict,
            JpegVerdict::NotBaseline
        );
        assert_eq!(
            probe_jpeg(&jpeg(0xc0, &[0x12, 0x11, 0x11])).verdict,
            JpegVerdict::Sampling
        );
        assert_eq!(
            probe_jpeg(&jpeg(0xc0, &[0x22, 0x21, 0x11])).verdict,
            JpegVerdict::Sampling
        );
        assert_eq!(probe_jpeg(b"\x89PNG....").verdict, JpegVerdict::NotJpeg);
        // A scan before any frame header, or a segment past the end.
        let mut scan_first = jpeg(0xc0, &[0x11]);
        scan_first[9] = 0xda;
        assert_eq!(probe_jpeg(&scan_first).verdict, JpegVerdict::NotJpeg);
        assert_eq!(
            probe_jpeg(&jpeg(0xc0, &[0x11])[..14]).verdict,
            JpegVerdict::NotJpeg
        );
    }

    #[test]
    fn new_track_drops_the_old_cover_and_stale_tickets_cannot_publish() {
        let mut decks = ArtworkDecks::new();
        let a = identity(1, 1, 1);
        let ticket_a = decks.begin(0, a, true).unwrap();
        assert_eq!(decks.view(0, a), ArtworkView::Loading);
        assert!(decks.admit_file(ticket_a, 4096));
        assert!(decks.complete(ticket_a, &jpeg(0xc0, &[0x11]), &mut Fill(Some(0xf800), 0)));
        let ArtworkView::Cover { edge, pixels } = decks.view(0, a) else {
            panic!("decoded cover must publish");
        };
        assert_eq!(edge, ARTWORK_DECK_EDGE);
        assert!(pixels.iter().all(|pixel| *pixel == 0xf800));

        // Track B replaces A: A's cover is gone before B's read completes,
        // and a late completion for A cannot overwrite B.
        let b = identity(2, 1, 2);
        let ticket_b = decks.begin(0, b, true).unwrap();
        assert_eq!(decks.view(0, a), ArtworkView::Empty);
        assert_eq!(decks.view(0, b), ArtworkView::Loading);
        let mut decoder = Fill(Some(0x001f), 0);
        assert!(!decks.complete(ticket_a, &jpeg(0xc0, &[0x11]), &mut decoder));
        assert!(!decks.fail(ticket_a, ArtworkFallback::ReadFailed));
        assert_eq!(decoder.1, 0);
        assert_eq!(decks.view(0, b), ArtworkView::Loading);

        // Reloading the same track is a new load generation and a new ticket.
        let b_again = identity(2, 1, 3);
        let ticket_b2 = decks.begin(0, b_again, true).unwrap();
        assert!(!decks.complete(ticket_b, &jpeg(0xc0, &[0x11]), &mut decoder));
        assert!(decks.complete(ticket_b2, &jpeg(0xc0, &[0x11]), &mut decoder));
        assert!(matches!(decks.view(0, b_again), ArtworkView::Cover { .. }));
        assert_eq!(decks.view(1, b_again), ArtworkView::Empty);
    }

    #[test]
    fn missing_or_unusable_covers_degrade_to_an_explicit_fallback() {
        let mut decks = ArtworkDecks::new();
        let a = identity(1, 1, 1);
        assert!(decks.begin(1, a, false).is_none());
        assert_eq!(
            decks.view(1, a),
            ArtworkView::Fallback(ArtworkFallback::NoArtwork)
        );

        let cases: [(&[u8], u64, ArtworkFallback); 5] = [
            (&[], ARTWORK_MAX_FILE_BYTES + 1, ArtworkFallback::TooLarge),
            (&[], 0, ArtworkFallback::ReadFailed),
            (b"not a jpeg", 10, ArtworkFallback::NotJpeg),
            (&jpeg(0xc2, &[0x11]), 64, ArtworkFallback::NotBaseline),
            (
                &jpeg(0xc0, &[0x12, 0x11, 0x11]),
                64,
                ArtworkFallback::Sampling,
            ),
        ];
        let mut decoder = Fill(Some(1), 0);
        for (bytes, len, expected) in cases {
            let ticket = decks.begin(1, a, true).unwrap();
            if decks.admit_file(ticket, len) {
                assert!(decks.complete(ticket, bytes, &mut decoder));
            }
            assert_eq!(decks.view(1, a), ArtworkView::Fallback(expected));
        }
        assert_eq!(decoder.1, 0);

        let mut huge = jpeg(0xc0, &[0x11]);
        huge[13..15].copy_from_slice(&4096u16.to_be_bytes());
        let ticket = decks.begin(1, a, true).unwrap();
        assert!(decks.complete(ticket, &huge, &mut decoder));
        assert_eq!(
            decks.view(1, a),
            ArtworkView::Fallback(ArtworkFallback::Dimensions)
        );

        let ticket = decks.begin(1, a, true).unwrap();
        assert!(decks.complete(ticket, &jpeg(0xc0, &[0x11]), &mut Fill(None, 0)));
        assert_eq!(
            decks.view(1, a),
            ArtworkView::Fallback(ArtworkFallback::DecodeFailed)
        );
    }

    #[test]
    fn unload_and_media_removal_clear_covers() {
        let mut decks = ArtworkDecks::new();
        let a = identity(1, 1, 1);
        let b = identity(2, 2, 1);
        let ticket = decks.begin(0, a, true).unwrap();
        assert!(decks.complete(ticket, &jpeg(0xc0, &[0x11]), &mut Fill(Some(7), 0)));
        let pending = decks.begin(1, b, true).unwrap();

        decks.invalidate(b.lease);
        assert!(matches!(decks.view(0, a), ArtworkView::Cover { .. }));
        assert_eq!(decks.view(1, b), ArtworkView::Empty);
        assert!(!decks.complete(pending, &jpeg(0xc0, &[0x11]), &mut Fill(Some(7), 0)));

        decks.clear(0);
        assert_eq!(decks.view(0, a), ArtworkView::Empty);
    }
}
