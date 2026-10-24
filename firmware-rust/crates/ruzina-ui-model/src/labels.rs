//! Deck and Library text as M2.5 `dj_ui` shows it: row BPM rounded to a whole
//! number, row time as `m:ss`, `--` for a deck without a key, and hot cue
//! names `CUE A`..`CUE H`, `LOOP A` for a loop.
use core::fmt;

/// `(int)(bpm + 0.5)` from the export tempo in hundredths.
pub const fn library_bpm(bpm_x100: u32) -> u32 {
    bpm_x100.saturating_add(50) / 100
}

/// Track length as `m:ss`; minutes are not wrapped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrackTime(pub u32);

impl fmt::Display for TrackTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{:02}", self.0 / 60_000, self.0 / 1000 % 60)
    }
}

/// The deck's key, or `--` when the export has none or no track is loaded.
pub fn deck_key(key: Option<&str>) -> &str {
    match key {
        Some(key) if !key.is_empty() => key,
        _ => "--",
    }
}

/// `CUE A`, or `LOOP A` for a loop cue, for slot 0..8.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotCueName {
    pub slot: u8,
    pub is_loop: bool,
}

impl HotCueName {
    pub const fn letter(slot: u8) -> char {
        (b'A' + slot % 8) as char
    }
}

impl fmt::Display for HotCueName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = if self.is_loop { "LOOP" } else { "CUE" };
        write!(f, "{kind} {}", Self::letter(self.slot))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::ToString;

    #[test]
    fn row_bpm_and_time_follow_m25() {
        assert_eq!(library_bpm(12_849), 128);
        assert_eq!(library_bpm(12_850), 129);
        assert_eq!(library_bpm(0), 0);
        assert_eq!(TrackTime(10_000).to_string(), "0:10");
        assert_eq!(TrackTime(465_999).to_string(), "7:45");
        assert_eq!(TrackTime(3_725_000).to_string(), "62:05");
    }

    #[test]
    fn deck_key_and_cue_names() {
        assert_eq!(deck_key(Some("Am")), "Am");
        assert_eq!(deck_key(Some("")), "--");
        assert_eq!(deck_key(None), "--");
        let name = |slot, is_loop| HotCueName { slot, is_loop }.to_string();
        assert_eq!(name(0, false), "CUE A");
        assert_eq!(name(7, true), "LOOP H");
    }
}
