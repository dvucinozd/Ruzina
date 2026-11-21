use super::*;
use alloc::string::String;
use alloc::vec;

fn row(title: &str, artist: &str, bpm: u32, key: &str, seconds: u32) -> LibraryRow {
    LibraryRow {
        title: String::from(title),
        artist: String::from(artist),
        key: String::from(key),
        bpm_x100: bpm * 100,
        duration_ms: seconds * 1_000,
    }
}

fn rows() -> Vec<LibraryRow> {
    vec![
        row("delta", "Zed", 124, "8A", 300),
        row("Alpha", "anna", 128, "Am", 200),
        row("", "Bert", 0, "", 250),
        row("charlie", "Anna", 120, "3B", 400),
    ]
}

/// The catalog's own list: row i is catalog track i.
fn all(rows: &[LibraryRow]) -> Vec<(u32, u32)> {
    (0..rows.len() as u32).map(|i| (i, i)).collect()
}

fn sorted(column: Column, descending: bool) -> Option<Sort> {
    Some(Sort { column, descending })
}

#[test]
fn a_tap_sorts_and_a_second_tap_turns_it_over() {
    let title = tap(None, Column::Title);
    assert_eq!(
        title,
        Sort {
            column: Column::Title,
            descending: false
        }
    );
    assert!(tap(Some(title), Column::Title).descending);
    assert!(!tap(Some(tap(Some(title), Column::Title)), Column::Bpm).descending);
    for index in 0..5 {
        assert_eq!(Column::from_index(index).map(Column::index), Some(index));
    }
    assert_eq!(Column::from_index(5), None);
}

#[test]
fn columns_sort_with_case_aside_and_unknown_keys_last() {
    let rows = rows();
    let tracks = all(&rows);
    // The track without a title sorts by its artist, as the page shows it.
    assert_eq!(
        build(&rows, &tracks, sorted(Column::Title, false), None),
        [1, 2, 3, 0]
    );
    assert_eq!(
        build(&rows, &tracks, sorted(Column::Title, true), None),
        [0, 3, 2, 1]
    );
    // Equal artists ("anna", "Anna") keep the list's order.
    assert_eq!(
        build(&rows, &tracks, sorted(Column::Artist, false), None),
        [1, 3, 2, 0]
    );
    assert_eq!(
        build(&rows, &tracks, sorted(Column::Bpm, false), None),
        [2, 3, 0, 1]
    );
    assert_eq!(
        build(&rows, &tracks, sorted(Column::Time, true), None),
        [3, 0, 2, 1]
    );
    // Camelot order: 3B, Am (8A), 8A; the empty key last both ways.
    assert_eq!(
        build(&rows, &tracks, sorted(Column::Key, false), None),
        [3, 0, 1, 2]
    );
    assert_eq!(
        build(&rows, &tracks, sorted(Column::Key, true), None),
        [0, 1, 3, 2]
    );
    assert_eq!(build(&rows, &tracks, None, None), [0, 1, 2, 3]);
}

#[test]
fn the_bpm_filter_keeps_tracks_within_four_percent() {
    let rows = rows();
    let tracks = all(&rows);
    // At 125 BPM, 4 % is 5 BPM: 120 to 130 pass. At 130 BPM it is 5.2 BPM: only 128.
    assert_eq!(build(&rows, &tracks, None, Some(12_500)), [0, 1, 3]);
    assert_eq!(build(&rows, &tracks, None, Some(13_000)), [1]);
    // A track without a tempo never passes.
    assert!(!bpm_fits(0, 12_000));
    assert!(!bpm_fits(12_000, 0));
    // A playlist's rows: positions in the playlist, catalog indexes behind them.
    let playlist = [(0, 3), (1, 1)];
    assert_eq!(
        build(&rows, &playlist, sorted(Column::Bpm, false), Some(12_500)),
        [0, 1]
    );
    assert_eq!(
        build(&rows, &playlist, sorted(Column::Bpm, true), Some(12_500)),
        [1, 0]
    );
}

#[test]
fn keys_parse_in_camelot_and_classical_names() {
    let key = |number, major| Some(Camelot { number, major });
    assert_eq!(camelot("8A"), key(8, false));
    assert_eq!(camelot("12b"), key(12, true));
    assert_eq!(camelot("Am"), key(8, false));
    assert_eq!(camelot("C"), key(8, true));
    assert_eq!(camelot("F#m"), key(11, false));
    assert_eq!(camelot("Bbmaj"), key(6, true));
    assert_eq!(camelot("C#min"), key(12, false));
    for bad in ["", "13A", "0A", "8C", "H", "Amm", "8"] {
        assert_eq!(camelot(bad), None, "{bad}");
    }
}

#[test]
fn keys_fit_as_the_donor_colours_them() {
    let reference = camelot("8A");
    assert_eq!(key_fit(reference, "8A"), KeyFit::Fits);
    assert_eq!(key_fit(reference, "9A"), KeyFit::Fits);
    assert_eq!(key_fit(reference, "7A"), KeyFit::Fits);
    assert_eq!(key_fit(reference, "8B"), KeyFit::Fits);
    assert_eq!(key_fit(reference, "10A"), KeyFit::Plus2);
    assert_eq!(key_fit(reference, "3A"), KeyFit::Plus7);
    assert_eq!(key_fit(reference, "9B"), KeyFit::Clash);
    assert_eq!(key_fit(reference, "5A"), KeyFit::Clash);
    // Around the wheel: 12A to 1A is a step.
    assert_eq!(key_fit(camelot("12A"), "1A"), KeyFit::Fits);
    assert_eq!(key_fit(None, "8A"), KeyFit::Unknown);
    assert_eq!(key_fit(reference, "?"), KeyFit::Unknown);
}
