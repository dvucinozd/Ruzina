//! Displays a bounded captured observation. The simulator never starts APTA.
use super::*;
use pajoniiir_service::analysis::Observation;
pub fn refresh(ui: &MainWindow, value: Observation) {
    let s = value.sample;
    use pajoniiir_service::analysis::State;
    ui.set_analysis_status(
        format!(
            "{} / ANALYSIS {}",
            if value == Observation::DISABLED {
                "CONFIGURATION"
            } else {
                "CAPTURED"
            },
            s.state.label()
        )
        .into(),
    );
    ui.set_analysis_severity(match s.state {
        State::Failed | State::Quarantined | State::RecoveryBlocked | State::Rejected => 2,
        State::Disabled | State::Paused | State::Deferred | State::Waiting => 1,
        _ => 0,
    });
    ui.set_analysis_recovery(s.reason.help().into());
    ui.set_health_analysis_text(format!("NATIVE ANALYSIS / CAPTURED OBSERVATION\n\nState: {}\nReason: {}\nDeck: {}\nLoad generation: {}\nTransition sequence: {}\nTransition time (source clock): {} ms\n\n{}\n\nCaptured record; no live worker connection.\nA transition time is not a heartbeat or percentage.\nAudio timing / hardware acceptance: NOT QUALIFIED",s.state.label(),s.reason.label(),s.deck.map_or("NONE",|d|match d {pajoniiir_core::DeckId::One=>"1",pajoniiir_core::DeckId::Two=>"2"}),s.load_generation.map_or("NONE".into(),|g|g.to_string()),value.serial,value.changed_ms,s.reason.help()).into());
}
pub fn read(path: &std::path::Path) -> Result<Observation, Box<dyn std::error::Error>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(1025)
        .read_to_end(&mut bytes)?;
    Observation::parse(std::str::from_utf8(&bytes)?)
        .map_err(|_| "invalid/truncated/oversized analysis observation".into())
}
