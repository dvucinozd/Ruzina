//! The Library page's sort, BPM filter and key colours, as the donor's ui-next Browse has
//! them (LB-7, LB-16, LB-18): which rows of a track list show, in what order, and how each
//! key fits the reference deck's. Pure; the screen task builds the view, and `deck_view`
//! maps the level's rows through it, so the browse knob and LOAD follow what the page shows.
#![cfg_attr(not(any(test, feature = "slint-ui")), allow(dead_code))]

use crate::deck_view::LibraryRow;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// A column the list can be sorted on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Column {
    Title,
    Artist,
    Bpm,
    Key,
    Time,
}

impl Column {
    /// The column of the page's heading `index`, left to right.
    pub(crate) const fn from_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::Title),
            1 => Some(Self::Artist),
            2 => Some(Self::Bpm),
            3 => Some(Self::Key),
            4 => Some(Self::Time),
            _ => None,
        }
    }

    pub(crate) const fn index(self) -> i32 {
        match self {
            Self::Title => 0,
            Self::Artist => 1,
            Self::Bpm => 2,
            Self::Key => 3,
            Self::Time => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Sort {
    pub(crate) column: Column,
    pub(crate) descending: bool,
}

/// A tap on a column's heading: that column, ascending; the same column again turns it over.
pub(crate) fn tap(sort: Option<Sort>, column: Column) -> Sort {
    match sort {
        Some(sort) if sort.column == column => Sort {
            column,
            descending: !sort.descending,
        },
        _ => Sort {
            column,
            descending: false,
        },
    }
}

/// How far a track's tempo may be from the reference deck's and still pass the filter.
pub(crate) const BPM_PERCENT: u32 = 4;

/// The rows of a track list that show, in order. `tracks` are each row's position in the
/// level and its catalog index; the result is the positions. Rows the BPM filter leaves out
/// (`bpm_reference_x100`: the reference deck's playing tempo) are dropped, tracks without a
/// tempo too. Equal rows keep the level's order, and an unknown key sorts last either way.
pub(crate) fn build(
    rows: &[LibraryRow],
    tracks: &[(u32, u32)],
    sort: Option<Sort>,
    bpm_reference_x100: Option<u32>,
) -> Vec<u32> {
    let row = |catalog: u32| rows.get(catalog as usize);
    let mut view: Vec<(u32, u32)> = tracks
        .iter()
        .copied()
        .filter(|&(_, catalog)| match bpm_reference_x100 {
            None => true,
            Some(reference) => row(catalog).is_some_and(|row| bpm_fits(row.bpm_x100, reference)),
        })
        .collect();
    if let Some(sort) = sort {
        view.sort_by(|&(_, a), &(_, b)| match (row(a), row(b)) {
            (Some(a), Some(b)) => compare(a, b, sort),
            _ => Ordering::Equal,
        });
    }
    view.into_iter().map(|(position, _)| position).collect()
}

/// Whether `bpm_x100` is within `BPM_PERCENT` of `reference_x100`.
pub(crate) fn bpm_fits(bpm_x100: u32, reference_x100: u32) -> bool {
    bpm_x100 != 0
        && reference_x100 != 0
        && u64::from(bpm_x100.abs_diff(reference_x100)) * 100
            <= u64::from(reference_x100) * u64::from(BPM_PERCENT)
}

fn compare(a: &LibraryRow, b: &LibraryRow, sort: Sort) -> Ordering {
    let order = match sort.column {
        Column::Title => text(shown_title(a), shown_title(b)),
        Column::Artist => text(&a.artist, &b.artist),
        Column::Bpm => a.bpm_x100.cmp(&b.bpm_x100),
        Column::Time => a.duration_ms.cmp(&b.duration_ms),
        Column::Key => match (camelot(&a.key), camelot(&b.key)) {
            (Some(a), Some(b)) => a.order().cmp(&b.order()),
            // Unknown keys last, whichever way the list runs.
            (Some(_), None) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Greater,
            (None, None) => Ordering::Equal,
        },
    };
    if sort.descending {
        order.reverse()
    } else {
        order
    }
}

/// The title the page shows: the artist when the track has no title.
fn shown_title(row: &LibraryRow) -> &str {
    if row.title.is_empty() {
        &row.artist
    } else {
        &row.title
    }
}

/// Text in alphabetical order, case aside.
fn text(a: &str, b: &str) -> Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

/// A key on the Camelot wheel: 1 to 12, minor (A) or major (B).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Camelot {
    number: u8,
    major: bool,
}

impl Camelot {
    const fn order(self) -> u8 {
        self.number * 2 + self.major as u8
    }
}

/// The Camelot key of `text`: "8A" as it is, or a classical name such as "Am", "F#",
/// "Bbm", "C#min" or "Ebmaj".
pub(crate) fn camelot(text: &str) -> Option<Camelot> {
    let text = text.trim();
    let bytes = text.as_bytes();
    if bytes.first()?.is_ascii_digit() {
        let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
        let number: u8 = text[..digits].parse().ok()?;
        let major = match &text[digits..] {
            "A" | "a" => false,
            "B" | "b" => true,
            _ => return None,
        };
        return (1..=12)
            .contains(&number)
            .then_some(Camelot { number, major });
    }
    // The tonic's pitch class (C = 0), then its sharp or flat, then the mode.
    const BASE: [u8; 7] = [9, 11, 0, 2, 4, 5, 7];
    let first = *bytes.first()?;
    let mut pitch = *BASE.get(usize::from(first.checked_sub(b'A')?))?;
    let mut rest = &text[1..];
    if let Some(after) = rest.strip_prefix('#') {
        pitch = (pitch + 1) % 12;
        rest = after;
    } else if let Some(after) = rest.strip_prefix('b') {
        pitch = (pitch + 11) % 12;
        rest = after;
    }
    let minor = match rest {
        "m" | "min" => true,
        "" | "maj" => false,
        _ => return None,
    };
    const MINOR: [u8; 12] = [5, 12, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10];
    const MAJOR: [u8; 12] = [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1];
    let number = if minor { MINOR } else { MAJOR }[usize::from(pitch)];
    Some(Camelot {
        number,
        major: !minor,
    })
}

/// How a track's key sits with the reference deck's.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyFit {
    /// No reference, or a key that does not parse.
    Unknown,
    /// The same key, a step either way on the wheel, or the relative major or minor.
    Fits,
    /// Two steps up: an energy lift.
    Plus2,
    /// Seven steps up.
    Plus7,
    /// None of these.
    Clash,
}

pub(crate) fn key_fit(reference: Option<Camelot>, key: &str) -> KeyFit {
    let (Some(reference), Some(key)) = (reference, camelot(key)) else {
        return KeyFit::Unknown;
    };
    let up = (key.number + 12 - reference.number) % 12;
    if key.major != reference.major {
        return if up == 0 { KeyFit::Fits } else { KeyFit::Clash };
    }
    match up {
        0 | 1 | 11 => KeyFit::Fits,
        2 => KeyFit::Plus2,
        7 => KeyFit::Plus7,
        _ => KeyFit::Clash,
    }
}

#[cfg(test)]
mod tests;
