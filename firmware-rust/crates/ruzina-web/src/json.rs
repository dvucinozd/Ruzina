//! `/api/status` JSON (M2.5 `api_status_handler`, core fields) written into a
//! caller buffer. Strings use M2.5 `web_api_json_escape`: `"` `\` and the
//! newline, return and tab escapes, other control bytes as `\u00XX`. The
//! M2.5 diagnostics blocks are left to their own routes; Ruzina adds the
//! [`StatusStamp`] so a page drops slower older responses.

use crate::status::StatusStamp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeckStateText {
    Idle,
    Loading,
    Ready,
    Playing,
    Error,
}

impl DeckStateText {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Loading => "LOADING",
            Self::Ready => "READY",
            Self::Playing => "PLAYING",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeckStatus<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    /// Current tempo (track BPM with the fader pitch), whole BPM as M2.5.
    pub bpm: u32,
    pub pitch_centipercent: i16,
    pub position_ms: u32,
    pub duration_ms: u32,
    pub playing: bool,
    pub sync_enabled: bool,
    pub sync_master: bool,
    pub state: DeckStateText,
}

impl DeckStatus<'_> {
    pub const EMPTY: DeckStatus<'static> = DeckStatus {
        title: "",
        artist: "",
        bpm: 0,
        pitch_centipercent: 0,
        position_ms: 0,
        duration_ms: 0,
        playing: false,
        sync_enabled: false,
        sync_master: false,
        state: DeckStateText::Idle,
    };
}

/// Controls on the controller's 14-bit range (M2.5 0..16383).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MixerStatus {
    pub volume: [u16; 2],
    pub crossfader: u16,
    pub master_volume: u16,
    pub headphone_mix: u16,
    /// Low, mid, high per channel.
    pub eq: [[u16; 3]; 2],
    pub filter: [u16; 2],
    pub smart_cfx: bool,
    pub smart_fader: bool,
    pub pfl: [bool; 2],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusSnapshot<'a> {
    pub stamp: StatusStamp,
    pub decks: [DeckStatus<'a>; 2],
    pub mixer: MixerStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Overflow;

/// One `/api/library` row (M2.5 keys; `track_key` is the export track id
/// a load names, `bpm` whole BPM).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LibraryRow<'a> {
    pub track_key: u32,
    pub title: &'a str,
    pub artist: &'a str,
    pub bpm: u32,
    pub duration_ms: u32,
}

/// Room kept for `],"next":4294967295}` after the last row.
const LIBRARY_FOOTER_RESERVE: usize = 32;

/// `/api/library` from one catalog revision (M2.5 `generation`): as many
/// rows from `offset` as fit in `out`, then `next`, the offset of the first
/// row left out, or `null` at the end. M2.5 streams the whole list chunked;
/// pages keep every response a fixed size. A page with no room for even one
/// pending row is an [`Overflow`].
pub fn write_library_page<'a>(
    revision: u32,
    total: usize,
    offset: usize,
    rows: impl IntoIterator<Item = LibraryRow<'a>>,
    out: &mut [u8],
) -> Result<usize, Overflow> {
    let room = out
        .len()
        .checked_sub(LIBRARY_FOOTER_RESERVE)
        .ok_or(Overflow)?;
    let (body, _) = out.split_at_mut(room);
    let mut json = Writer { out: body, len: 0 };
    json.raw("{\"generation\":")?;
    json.unsigned(revision)?;
    json.raw(",\"total\":")?;
    json.unsigned(u32::try_from(total).map_err(|_| Overflow)?)?;
    json.raw(",\"offset\":")?;
    json.unsigned(u32::try_from(offset).map_err(|_| Overflow)?)?;
    json.raw(",\"tracks\":[")?;
    let mut next = offset;
    for row in rows.into_iter().take(total.saturating_sub(offset)) {
        let mark = json.len;
        if write_library_row(&mut json, next, next != offset, &row).is_err() {
            json.len = mark;
            break;
        }
        next += 1;
    }
    if next == offset && offset < total {
        return Err(Overflow);
    }
    let len = json.len;
    let mut json = Writer { out, len };
    json.raw("],\"next\":")?;
    if next < total {
        json.unsigned(u32::try_from(next).map_err(|_| Overflow)?)?;
    } else {
        json.raw("null")?;
    }
    json.raw("}")?;
    Ok(json.len)
}

fn write_library_row(
    json: &mut Writer<'_>,
    index: usize,
    separator: bool,
    row: &LibraryRow<'_>,
) -> Result<(), Overflow> {
    if separator {
        json.raw(",")?;
    }
    json.raw("{\"index\":")?;
    json.unsigned(u32::try_from(index).map_err(|_| Overflow)?)?;
    json.raw(",\"track_key\":")?;
    json.unsigned(row.track_key)?;
    json.raw(",\"title\":")?;
    json.string(row.title)?;
    json.raw(",\"artist\":")?;
    json.string(row.artist)?;
    json.raw(",\"bpm\":")?;
    json.unsigned(row.bpm)?;
    json.raw(",\"duration_ms\":")?;
    json.unsigned(row.duration_ms)?;
    json.raw("}")
}

pub fn write_status(status: &StatusSnapshot<'_>, out: &mut [u8]) -> Result<usize, Overflow> {
    let mut json = Writer { out, len: 0 };
    let stamp = status.stamp;
    json.raw("{\"stamp\":{\"boot\":")?;
    json.unsigned(stamp.boot)?;
    json.raw(",\"sequence\":")?;
    json.unsigned(stamp.sequence)?;
    json.raw(",\"library_revision\":")?;
    match stamp.catalog {
        Some(catalog) => json.unsigned(catalog.revision)?,
        None => json.raw("null")?,
    }
    json.raw(",\"load_generations\":[")?;
    json.unsigned(stamp.load_generations[0])?;
    json.raw(",")?;
    json.unsigned(stamp.load_generations[1])?;
    json.raw("]}")?;
    for (index, deck) in status.decks.iter().enumerate() {
        json.raw(if index == 0 {
            ",\"deck1\":{\"title\":"
        } else {
            ",\"deck2\":{\"title\":"
        })?;
        json.string(deck.title)?;
        json.raw(",\"artist\":")?;
        json.string(deck.artist)?;
        json.raw(",\"bpm\":")?;
        json.unsigned(deck.bpm)?;
        json.raw(",\"pitch_percent\":")?;
        json.hundredths(deck.pitch_centipercent)?;
        json.raw(",\"position_ms\":")?;
        json.unsigned(deck.position_ms)?;
        json.raw(",\"duration_ms\":")?;
        json.unsigned(deck.duration_ms)?;
        json.raw(",\"playing\":")?;
        json.boolean(deck.playing)?;
        json.raw(",\"sync_enabled\":")?;
        json.boolean(deck.sync_enabled)?;
        json.raw(",\"sync_master\":")?;
        json.boolean(deck.sync_master)?;
        json.raw(",\"state_text\":\"")?;
        json.raw(deck.state.as_str())?;
        json.raw("\"}")?;
    }
    let mixer = status.mixer;
    let fields: [(&str, u16); 13] = [
        ("{\"volume1\":", mixer.volume[0]),
        (",\"volume2\":", mixer.volume[1]),
        (",\"crossfader\":", mixer.crossfader),
        (",\"master_volume\":", mixer.master_volume),
        (",\"headphone_mix\":", mixer.headphone_mix),
        (",\"eq1_low\":", mixer.eq[0][0]),
        (",\"eq1_mid\":", mixer.eq[0][1]),
        (",\"eq1_high\":", mixer.eq[0][2]),
        (",\"eq2_low\":", mixer.eq[1][0]),
        (",\"eq2_mid\":", mixer.eq[1][1]),
        (",\"eq2_high\":", mixer.eq[1][2]),
        (",\"filter1\":", mixer.filter[0]),
        (",\"filter2\":", mixer.filter[1]),
    ];
    json.raw(",\"mixer\":")?;
    for (key, value) in fields {
        json.raw(key)?;
        json.unsigned(u32::from(value))?;
    }
    for (key, value) in [
        (",\"smart_cfx\":", mixer.smart_cfx),
        (",\"smart_fader\":", mixer.smart_fader),
        (",\"pfl1\":", mixer.pfl[0]),
        (",\"pfl2\":", mixer.pfl[1]),
    ] {
        json.raw(key)?;
        json.boolean(value)?;
    }
    json.raw("}}")?;
    Ok(json.len)
}

struct Writer<'a> {
    out: &'a mut [u8],
    len: usize,
}

impl Writer<'_> {
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), Overflow> {
        let end = self.len.checked_add(bytes.len()).ok_or(Overflow)?;
        self.out
            .get_mut(self.len..end)
            .ok_or(Overflow)?
            .copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }

    fn raw(&mut self, text: &str) -> Result<(), Overflow> {
        self.bytes(text.as_bytes())
    }

    fn boolean(&mut self, value: bool) -> Result<(), Overflow> {
        self.raw(if value { "true" } else { "false" })
    }

    fn unsigned(&mut self, mut value: u32) -> Result<(), Overflow> {
        let mut digits = [0u8; 10];
        let mut start = digits.len();
        loop {
            start -= 1;
            digits[start] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        self.bytes(&digits[start..])
    }

    /// `-6.25` from -625 (M2.5 `%.2f` of the pitch percent).
    fn hundredths(&mut self, value: i16) -> Result<(), Overflow> {
        if value < 0 {
            self.raw("-")?;
        }
        let magnitude = value.unsigned_abs();
        self.unsigned(u32::from(magnitude / 100))?;
        let fraction = magnitude % 100;
        self.bytes(&[
            b'.',
            b'0' + (fraction / 10) as u8,
            b'0' + (fraction % 10) as u8,
        ])
    }

    fn string(&mut self, text: &str) -> Result<(), Overflow> {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        self.raw("\"")?;
        for byte in text.bytes() {
            match byte {
                b'"' => self.raw("\\\"")?,
                b'\\' => self.raw("\\\\")?,
                b'\n' => self.raw("\\n")?,
                b'\r' => self.raw("\\r")?,
                b'\t' => self.raw("\\t")?,
                0..0x20 => self.bytes(&[
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX[usize::from(byte >> 4)],
                    HEX[usize::from(byte & 15)],
                ])?,
                _ => self.bytes(&[byte])?,
            }
        }
        self.raw("\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruzina_media_catalog::CatalogPublicationId;
    use ruzina_media_session::{MediaGeneration, MediaLease, MediaSourceId};

    fn snapshot<'a>(title: &'a str, catalog: bool) -> StatusSnapshot<'a> {
        StatusSnapshot {
            stamp: StatusStamp {
                boot: 7,
                sequence: 42,
                catalog: catalog.then(|| CatalogPublicationId {
                    lease: MediaLease {
                        generation: MediaGeneration::initial(),
                        source: MediaSourceId::new(1).unwrap(),
                    },
                    revision: 3,
                }),
                load_generations: [5, 0],
            },
            decks: [
                DeckStatus {
                    title,
                    artist: "Grid Lab",
                    bpm: 113,
                    pitch_centipercent: -625,
                    position_ms: 1770,
                    duration_ms: 10_000,
                    playing: true,
                    sync_enabled: false,
                    sync_master: true,
                    state: DeckStateText::Playing,
                },
                DeckStatus::EMPTY,
            ],
            mixer: MixerStatus {
                volume: [16383, 0],
                crossfader: 8192,
                pfl: [true, false],
                ..MixerStatus::default()
            },
        }
    }

    #[test]
    fn status_json_has_the_m25_core_fields_and_the_stamp() {
        let mut out = [0u8; 1024];
        let len = write_status(&snapshot("Helpless \"live\"\\\u{1}", true), &mut out).unwrap();
        assert_eq!(
            core::str::from_utf8(&out[..len]).unwrap(),
            concat!(
                "{\"stamp\":{\"boot\":7,\"sequence\":42,\"library_revision\":3,",
                "\"load_generations\":[5,0]},",
                "\"deck1\":{\"title\":\"Helpless \\\"live\\\"\\\\\\u0001\",\"artist\":\"Grid Lab\",",
                "\"bpm\":113,\"pitch_percent\":-6.25,\"position_ms\":1770,\"duration_ms\":10000,",
                "\"playing\":true,\"sync_enabled\":false,\"sync_master\":true,",
                "\"state_text\":\"PLAYING\"},",
                "\"deck2\":{\"title\":\"\",\"artist\":\"\",\"bpm\":0,\"pitch_percent\":0.00,",
                "\"position_ms\":0,\"duration_ms\":0,\"playing\":false,\"sync_enabled\":false,",
                "\"sync_master\":false,\"state_text\":\"IDLE\"},",
                "\"mixer\":{\"volume1\":16383,\"volume2\":0,\"crossfader\":8192,",
                "\"master_volume\":0,\"headphone_mix\":0,\"eq1_low\":0,\"eq1_mid\":0,",
                "\"eq1_high\":0,\"eq2_low\":0,\"eq2_mid\":0,\"eq2_high\":0,\"filter1\":0,",
                "\"filter2\":0,\"smart_cfx\":false,\"smart_fader\":false,\"pfl1\":true,",
                "\"pfl2\":false}}"
            )
        );
    }

    #[test]
    fn unicode_passes_through_and_small_buffers_overflow() {
        let mut out = [0u8; 1024];
        let len = write_status(&snapshot("Jura Stublić · 周防", false), &mut out).unwrap();
        let text = core::str::from_utf8(&out[..len]).unwrap();
        assert!(text.contains("\"title\":\"Jura Stublić · 周防\""));
        assert!(text.contains("\"library_revision\":null"));
        assert_eq!(
            write_status(&snapshot("x", true), &mut [0u8; 64]),
            Err(Overflow)
        );
    }

    fn row(track_key: u32) -> LibraryRow<'static> {
        LibraryRow {
            track_key,
            title: "Helpless",
            artist: "The Flirts",
            bpm: 118,
            duration_ms: 465_000,
        }
    }

    #[test]
    fn library_pages_hold_whole_rows_and_name_the_next_offset() {
        let mut out = [0u8; 1024];
        let len = write_library_page(3, 2, 0, [row(10), row(42)], &mut out).unwrap();
        assert_eq!(
            core::str::from_utf8(&out[..len]).unwrap(),
            concat!(
                "{\"generation\":3,\"total\":2,\"offset\":0,\"tracks\":[",
                "{\"index\":0,\"track_key\":10,\"title\":\"Helpless\",\"artist\":\"The Flirts\",",
                "\"bpm\":118,\"duration_ms\":465000},",
                "{\"index\":1,\"track_key\":42,\"title\":\"Helpless\",\"artist\":\"The Flirts\",",
                "\"bpm\":118,\"duration_ms\":465000}],\"next\":null}"
            )
        );

        // A small buffer ends the page after whole rows and names the next one.
        let mut small = [0u8; 340];
        let len = write_library_page(3, 5, 1, (1..5).map(row), &mut small).unwrap();
        let text = core::str::from_utf8(&small[..len]).unwrap();
        assert!(text.contains("\"offset\":1,"));
        assert!(text.contains("\"index\":1,\"track_key\":1,"));
        assert!(text.ends_with("}],\"next\":3}"), "{text}");

        // Past the end is an empty last page; no room for one row overflows.
        let len = write_library_page(3, 2, 2, [], &mut out).unwrap();
        assert!(
            core::str::from_utf8(&out[..len])
                .unwrap()
                .ends_with("\"tracks\":[],\"next\":null}")
        );
        assert_eq!(
            write_library_page(3, 2, 0, [row(10)], &mut [0u8; 90]),
            Err(Overflow)
        );
    }
}
