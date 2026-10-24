//! Memory cue list text (M2.5 `ui_performance_tabs` MEMORY panel): a title
//! with the deck, count and truncation, then one line per imported memory
//! point or loop in export order. M2.5 lists them only; it has no memory-cue
//! recall or LED.
use core::fmt;
use ruzina_core::DeckId;

pub const MEMORY_CUE_EMPTY: &str = "No memory cues";

/// `D1 MEMORY CUES: 3`, with ` (TRUNCATED)` when the export held more.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryCueTitle {
    pub deck: DeckId,
    pub count: usize,
    pub truncated: bool,
}

impl fmt::Display for MemoryCueTitle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let deck = match self.deck {
            DeckId::One => 1,
            DeckId::Two => 2,
        };
        write!(f, "D{deck} MEMORY CUES: {}", self.count)?;
        if self.truncated {
            f.write_str(" (TRUNCATED)")?;
        }
        Ok(())
    }
}

/// Whole seconds as `HH:MM:SS`, as M2.5 shows cue times.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CueClock(pub u32);

impl fmt::Display for CueClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seconds = self.0 / 1000;
        write!(
            f,
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            seconds % 3600 / 60,
            seconds % 60
        )
    }
}

/// `01  00:01:02  CUE`, or `02  00:01:05  LOOP - 00:01:09` for a loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryCueLine {
    /// One-based position in export order.
    pub number: usize,
    pub start_ms: u32,
    pub end_ms: Option<u32>,
}

impl fmt::Display for MemoryCueLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}  {}  ", self.number, CueClock(self.start_ms))?;
        match self.end_ms {
            Some(end) => write!(f, "LOOP - {}", CueClock(end)),
            None => f.write_str("CUE"),
        }
    }
}

/// Numbered lines for the cues, given as (start, loop end) in export order.
pub fn memory_cue_lines(
    cues: impl IntoIterator<Item = (u32, Option<u32>)>,
) -> impl Iterator<Item = MemoryCueLine> {
    cues.into_iter()
        .enumerate()
        .map(|(index, (start_ms, end_ms))| MemoryCueLine {
            number: index + 1,
            start_ms,
            end_ms,
        })
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{format, string::ToString, vec::Vec};

    #[test]
    fn lines_follow_the_m25_panel_format() {
        let lines: Vec<_> =
            memory_cue_lines([(62_400, None), (65_000, Some(69_999)), (3_725_000, None)])
                .map(|line| line.to_string())
                .collect();
        assert_eq!(
            lines,
            [
                "01  00:01:02  CUE",
                "02  00:01:05  LOOP - 00:01:09",
                "03  01:02:05  CUE"
            ]
        );
    }

    #[test]
    fn title_names_deck_count_and_truncation() {
        let title = |deck, count, truncated| {
            format!(
                "{}",
                MemoryCueTitle {
                    deck,
                    count,
                    truncated
                }
            )
        };
        assert_eq!(title(DeckId::One, 0, false), "D1 MEMORY CUES: 0");
        assert_eq!(
            title(DeckId::Two, 16, true),
            "D2 MEMORY CUES: 16 (TRUNCATED)"
        );
    }
}
