//! The deck link bar's figures, for the deck screen's redesign: how the other deck sits
//! against the master in tempo, in its beat, in its bar and in key. The bar shows them
//! between the lanes and the deck cards; nothing here changes playback.
//!
//! Not on the screen yet: the redesign wires it in.
#![cfg_attr(not(test), allow(dead_code))]

use crate::deck_engine::beat_at;
use core::fmt;
use ruzina_track_analysis::Beat;

/// One deck as the link bar sees it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LinkDeck<'a> {
    pub(crate) beats: &'a [Beat],
    pub(crate) position_ms: u32,
    /// The tempo it plays at, its pitch included, in hundredths of a BPM.
    pub(crate) bpm_x100: u32,
    /// Its key as the export names it (`8A`, `Am`), or empty.
    pub(crate) key: &'a str,
    /// Semitones its pitch moves its key (`key_shift`): 0 with Master Tempo on.
    pub(crate) semitones: i32,
}

/// The other deck against the master. Each figure is `None` when either deck lacks what
/// it needs: a tempo, a place inside its beat grid, a key the bar can read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Link {
    /// Its tempo against the master's, in hundredths of a percent: + faster.
    pub(crate) tempo_x100: Option<i32>,
    /// How far its beat is ahead of the master's, in milliseconds at the master's tempo,
    /// within half a beat: + ahead.
    pub(crate) beat_ms: Option<i32>,
    /// How many whole beats its place in the bar is ahead of the master's, -2 to +1.
    pub(crate) bar_beats: Option<i32>,
    pub(crate) key: Option<KeyFit>,
}

pub(crate) fn link(master: LinkDeck<'_>, other: LinkDeck<'_>) -> Link {
    let master_place = place(master);
    let other_place = place(other);
    let both = master_place.zip(other_place);
    Link {
        tempo_x100: tempo_x100(master.bpm_x100, other.bpm_x100),
        beat_ms: both.and_then(|((_, master_into), (_, other_into))| {
            let ahead = wrap(other_into - master_into, 1.0);
            (master.bpm_x100 != 0).then(|| round(ahead * 6_000_000.0 / master.bpm_x100 as f32))
        }),
        bar_beats: both.map(|((master_beat, master_into), (other_beat, other_into))| {
            let ahead = (other_beat - master_beat) as f32 + other_into - master_into;
            round(wrap(ahead, 4.0)).clamp(-2, 1)
        }),
        key: key_fit(master, other),
    }
}

/// Where a deck is in its bar: the beat's number from 0 to 3, and how far into that beat.
fn place(deck: LinkDeck<'_>) -> Option<(i32, f32)> {
    let (at, into) = beat_at(deck.beats, deck.position_ms)?;
    let number = deck.beats[at].phase;
    (1..=4)
        .contains(&number)
        .then(|| (i32::from(number) - 1, into))
}

fn tempo_x100(master: u32, other: u32) -> Option<i32> {
    if master == 0 || other == 0 {
        return None;
    }
    let difference = (i64::from(other) - i64::from(master)) * 10_000;
    let half = i64::from(master) / 2;
    let rounded = if difference >= 0 {
        (difference + half) / i64::from(master)
    } else {
        (difference - half) / i64::from(master)
    };
    Some(rounded as i32)
}

/// `value` brought into `[-span / 2, span / 2)`.
fn wrap(value: f32, span: f32) -> f32 {
    let mut value = value % span;
    if value >= span / 2.0 {
        value -= span;
    } else if value < -span / 2.0 {
        value += span;
    }
    value
}

fn round(value: f32) -> i32 {
    if value >= 0.0 {
        (value + 0.5) as i32
    } else {
        (value - 0.5) as i32
    }
}

/// The semitones a deck's pitch moves its key: none with Master Tempo on, else the
/// nearest whole semitone of its playback rate (1.0 the track's own tempo).
pub(crate) fn key_shift(rate: f32, master_tempo: bool) -> i32 {
    const SEMITONE: f32 = 1.059_463_1;
    const HALF_SEMITONE: f32 = 1.029_302_2;
    if master_tempo || rate.is_nan() || rate <= 0.0 {
        return 0;
    }
    let mut semitones = 0;
    if rate >= 1.0 {
        let mut edge = HALF_SEMITONE;
        while rate >= edge && semitones < 24 {
            semitones += 1;
            edge *= SEMITONE;
        }
    } else {
        let mut edge = 1.0 / HALF_SEMITONE;
        while rate < edge && semitones > -24 {
            semitones -= 1;
            edge /= SEMITONE;
        }
    }
    semitones
}

/// A key on the Camelot wheel: 1 to 12, A minor or B major.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Camelot {
    pub(crate) number: u8,
    pub(crate) major: bool,
}

impl Camelot {
    /// A key as Rekordbox names it: Camelot (`8A`, `12B`) or classic (`Am`, `C#`, `Ebm`).
    pub(crate) fn parse(key: &str) -> Option<Self> {
        let key = key.trim();
        let (body, last) = key.split_at(key.len().checked_sub(1)?);
        let major = match last {
            "A" | "a" => Some(false),
            "B" | "b" => Some(true),
            _ => None,
        };
        if let (Some(major), Ok(number)) = (major, body.parse::<u8>()) {
            return (1..=12).contains(&number).then_some(Self { number, major });
        }
        let (root, major) = match key.strip_suffix('m') {
            Some(root) => (root, false),
            None => (key, true),
        };
        let semitone: i32 = match root {
            "C" | "B#" => 0,
            "C#" | "Db" => 1,
            "D" => 2,
            "D#" | "Eb" => 3,
            "E" | "Fb" => 4,
            "F" | "E#" => 5,
            "F#" | "Gb" => 6,
            "G" => 7,
            "G#" | "Ab" => 8,
            "A" => 9,
            "A#" | "Bb" => 10,
            "B" | "Cb" => 11,
            _ => return None,
        };
        // A fifth up is one step round the wheel; A minor is 8A, C major 8B.
        let tonic = if major { 0 } else { 9 };
        let number = ((semitone - tonic) * 7 + 7).rem_euclid(12) as u8 + 1;
        Some(Self { number, major })
    }

    /// The key `semitones` higher: seven steps round the wheel each.
    pub(crate) fn shifted(self, semitones: i32) -> Self {
        let number = (i32::from(self.number) - 1 + semitones * 7).rem_euclid(12) as u8 + 1;
        Self { number, ..self }
    }
}

impl fmt::Display for Camelot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.number, if self.major { 'B' } else { 'A' })
    }
}

/// How two keys mix, by the Camelot wheel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyRelation {
    Same,
    /// One step round the wheel, in the same mode.
    Adjacent,
    /// The same number in the other mode: the relative major or minor.
    Relative,
    Clash,
}

/// The two keys as they sound now, each moved by its deck's pitch, and how they mix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct KeyFit {
    pub(crate) master: Camelot,
    pub(crate) other: Camelot,
    pub(crate) relation: KeyRelation,
}

fn key_fit(master: LinkDeck<'_>, other: LinkDeck<'_>) -> Option<KeyFit> {
    let master = Camelot::parse(master.key)?.shifted(master.semitones);
    let other = Camelot::parse(other.key)?.shifted(other.semitones);
    let steps = (i32::from(other.number) - i32::from(master.number)).rem_euclid(12);
    let relation = match (master.major == other.major, steps) {
        (true, 0) => KeyRelation::Same,
        (true, 1 | 11) => KeyRelation::Adjacent,
        (false, 0) => KeyRelation::Relative,
        _ => KeyRelation::Clash,
    };
    Some(KeyFit {
        master,
        other,
        relation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::vec::Vec;

    /// A grid at `bpm`, from `first_ms`, its first beat numbered `first_number`.
    fn grid(bpm: u32, first_ms: u32, first_number: u16, beats: usize) -> Vec<Beat> {
        let length = 60_000 / bpm;
        (0..beats)
            .map(|i| Beat {
                time_ms: first_ms + i as u32 * length,
                phase: (first_number - 1 + i as u16) % 4 + 1,
                bpm_x100: bpm * 100,
            })
            .collect()
    }

    fn deck<'a>(beats: &'a [Beat], position_ms: u32, bpm_x100: u32, key: &'a str) -> LinkDeck<'a> {
        LinkDeck {
            beats,
            position_ms,
            bpm_x100,
            key,
            semitones: 0,
        }
    }

    #[test]
    fn tempo_is_the_other_deck_against_the_master() {
        assert_eq!(tempo_x100(12_400, 12_450), Some(40));
        assert_eq!(tempo_x100(12_400, 12_350), Some(-40));
        assert_eq!(tempo_x100(12_000, 12_000), Some(0));
        assert_eq!(tempo_x100(0, 12_000), None);
    }

    #[test]
    fn beat_offset_is_within_half_a_beat_at_the_master_tempo() {
        // 120 BPM: a beat is 500 ms.
        let beats = grid(120, 1_000, 1, 64);
        let master = deck(&beats, 10_000, 12_000, "");
        // 60 ms later in the same grid: 60 ms ahead.
        assert_eq!(
            link(master, deck(&beats, 10_060, 12_000, "")).beat_ms,
            Some(60)
        );
        assert_eq!(
            link(master, deck(&beats, 9_940, 12_000, "")).beat_ms,
            Some(-60)
        );
        // 440 ms ahead is 60 ms behind the next beat.
        assert_eq!(
            link(master, deck(&beats, 10_440, 12_000, "")).beat_ms,
            Some(-60)
        );
        // Outside a grid: no figure.
        assert_eq!(link(master, deck(&beats, 500, 12_000, "")).beat_ms, None);
    }

    #[test]
    fn bar_offset_counts_whole_beats_from_minus_two_to_one() {
        let beats = grid(120, 0, 1, 64);
        // 10 000 ms is beat 20: number 1 of its bar.
        let master = deck(&beats, 10_000, 12_000, "");
        for (position, ahead) in [
            (10_000, 0),
            (10_500, 1),
            (11_000, -2),
            (11_500, -1),
            (12_000, 0),
        ] {
            assert_eq!(
                link(master, deck(&beats, position, 12_000, "")).bar_beats,
                Some(ahead),
                "{position}"
            );
        }
        // A grid without beat numbers says nothing about bars.
        let unnumbered: Vec<Beat> = beats
            .iter()
            .map(|beat| Beat { phase: 0, ..*beat })
            .collect();
        assert_eq!(
            link(master, deck(&unnumbered, 10_000, 12_000, "")).bar_beats,
            None
        );
    }

    #[test]
    fn keys_parse_in_camelot_and_classic_names() {
        let camelot = |number, major| Some(Camelot { number, major });
        assert_eq!(Camelot::parse("8A"), camelot(8, false));
        assert_eq!(Camelot::parse("12B"), camelot(12, true));
        assert_eq!(Camelot::parse("Am"), camelot(8, false));
        assert_eq!(Camelot::parse("C"), camelot(8, true));
        assert_eq!(Camelot::parse("Em"), camelot(9, false));
        assert_eq!(Camelot::parse("Dbm"), camelot(12, false));
        assert_eq!(Camelot::parse("C#m"), camelot(12, false));
        assert_eq!(Camelot::parse("F#"), camelot(2, true));
        assert_eq!(Camelot::parse("Ab"), camelot(4, true));
        for bad in ["", "13A", "0B", "H", "8C", "m"] {
            assert_eq!(Camelot::parse(bad), None, "{bad}");
        }
        assert_eq!(format!("{}", Camelot::parse("Am").unwrap()), "8A");
    }

    #[test]
    fn pitch_moves_the_key_unless_master_tempo_holds_it() {
        assert_eq!(key_shift(1.0, false), 0);
        assert_eq!(key_shift(1.02, false), 0);
        assert_eq!(key_shift(1.06, false), 1);
        assert_eq!(key_shift(1.12, false), 2);
        assert_eq!(key_shift(0.94, false), -1);
        assert_eq!(key_shift(1.12, true), 0);
        // A semitone up is seven steps round the wheel: 8A to 3A.
        assert_eq!(
            Camelot::parse("8A").unwrap().shifted(1),
            Camelot::parse("3A").unwrap()
        );
        assert_eq!(
            Camelot::parse("8A").unwrap().shifted(-1),
            Camelot::parse("1A").unwrap()
        );
    }

    #[test]
    fn keys_mix_by_the_camelot_wheel() {
        let beats = grid(120, 0, 1, 8);
        let relation = |master: &str, other: &str| {
            link(
                deck(&beats, 1_000, 12_000, master),
                deck(&beats, 1_000, 12_000, other),
            )
            .key
            .map(|fit| fit.relation)
        };
        assert_eq!(relation("8A", "8A"), Some(KeyRelation::Same));
        assert_eq!(relation("8A", "9A"), Some(KeyRelation::Adjacent));
        assert_eq!(relation("1A", "12A"), Some(KeyRelation::Adjacent));
        assert_eq!(relation("8A", "8B"), Some(KeyRelation::Relative));
        assert_eq!(relation("8A", "10A"), Some(KeyRelation::Clash));
        assert_eq!(relation("8A", "9B"), Some(KeyRelation::Clash));
        assert_eq!(relation("8A", ""), None);
        // The other deck pitched a semitone up without Master Tempo: 8A sounds as 3A.
        let other = LinkDeck {
            semitones: 1,
            ..deck(&beats, 1_000, 12_700, "8A")
        };
        let fit = link(deck(&beats, 1_000, 12_000, "8A"), other).key.unwrap();
        assert_eq!(format!("{}", fit.other), "3A");
        assert_eq!(fit.relation, KeyRelation::Clash);
    }
}
