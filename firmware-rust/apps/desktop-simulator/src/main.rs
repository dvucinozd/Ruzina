use pajoniiir_core::DeckId;
use pajoniiir_ui_model::{DeckSnapshot, UiSnapshot};

slint::include_modules!();

fn format_bpm(bpm_milli: u32) -> String {
    format!("{:.2} BPM", bpm_milli as f64 / 1000.0)
}

fn format_time(position_ms: u64) -> String {
    let total_seconds = position_ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let millis = position_ms % 1000;
    format!("{minutes:02}:{seconds:02}.{millis:03}")
}

fn deck_status(deck: DeckSnapshot) -> String {
    let transport = if deck.playing { "PLAY" } else { "PAUSE" };
    let sync = if deck.sync { "SYNC ON" } else { "SYNC OFF" };
    let master_tempo = if deck.master_tempo { "MT ON" } else { "MT OFF" };
    format!("{transport}   {sync}   {master_tempo}")
}

fn simulator_snapshot() -> UiSnapshot {
    UiSnapshot {
        deck_one: DeckSnapshot {
            deck: DeckId::One,
            loaded: true,
            playing: true,
            bpm_milli: 128_000,
            position_ms: 93_427,
            duration_ms: 367_000,
            pitch_centi_percent: 0,
            sync: true,
            master_tempo: true,
        },
        deck_two: DeckSnapshot {
            deck: DeckId::Two,
            loaded: true,
            playing: false,
            bpm_milli: 124_500,
            position_ms: 42_015,
            duration_ms: 301_000,
            pitch_centi_percent: -125,
            sync: false,
            master_tempo: true,
        },
        crossfader_milli: 0,
        master_milli: 900,
    }
}

fn apply_snapshot(ui: &MainWindow, snapshot: UiSnapshot) {
    ui.set_deck_one_bpm_text(format_bpm(snapshot.deck_one.bpm_milli).into());
    ui.set_deck_one_time_text(format_time(snapshot.deck_one.position_ms).into());
    ui.set_deck_one_status_text(deck_status(snapshot.deck_one).into());

    ui.set_deck_two_bpm_text(format_bpm(snapshot.deck_two.bpm_milli).into());
    ui.set_deck_two_time_text(format_time(snapshot.deck_two.position_ms).into());
    ui.set_deck_two_status_text(deck_status(snapshot.deck_two).into());

    ui.set_simulator_status_text("HOST / SIMULATOR VERIFIED".into());
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    apply_snapshot(&ui, simulator_snapshot());
    ui.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bpm_from_milli_bpm() {
        assert_eq!(format_bpm(128_250), "128.25 BPM");
    }

    #[test]
    fn formats_transport_time() {
        assert_eq!(format_time(93_427), "01:33.427");
    }

    #[test]
    fn simulator_fixture_is_loaded_on_both_decks() {
        let snapshot = simulator_snapshot();
        assert!(snapshot.deck_one.loaded);
        assert!(snapshot.deck_two.loaded);
        assert_eq!(snapshot.deck_one.deck, DeckId::One);
        assert_eq!(snapshot.deck_two.deck, DeckId::Two);
    }
}
