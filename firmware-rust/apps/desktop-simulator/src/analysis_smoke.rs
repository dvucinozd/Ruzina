use super::*;
use pajoniiir_service::analysis::{Observation, Reason, Sample, State};
pub fn run(
    ui: &MainWindow,
    state: &RefCell<Session>,
    output: &std::path::Path,
    name: &str,
    width: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    use super::smoke::{capture, click};
    ui.set_page(6);
    ui.set_health_events(false);
    ui.set_health_analysis(false);
    click(ui, width * 0.62, 144.0);
    assert!(ui.get_health_analysis());
    for (index, (status, reason)) in [
        (State::Waiting, Reason::Queued),
        (State::Analyzing, Reason::None),
        (State::Paused, Reason::PlaybackOrTransport),
        (State::Deferred, Reason::PlaybackOrTransport),
        (State::Published, Reason::None),
        (State::Failed, Reason::Format),
        (State::RecoveryBlocked, Reason::Storage),
        (State::Quarantined, Reason::UnexpectedCompletion),
    ]
    .into_iter()
    .enumerate()
    {
        let value = Observation {
            sample: Sample {
                state: status,
                reason,
                deck: Some(pajoniiir_core::DeckId::One),
                load_generation: Some(7),
            },
            serial: index as u64 + 1,
            changed_ms: 100 + index as u64,
        };
        // An explicit recorded fixture, never an active simulator worker.
        state.borrow_mut().analysis_observation = value;
        refresh(ui, &state.borrow());
        assert!(ui.get_health_analysis_text().contains(status.label()));
        assert!(ui.get_health_analysis_text().contains(reason.help()));
        let id = format!("{name}-analysis-{}", status.label().to_ascii_lowercase());
        let mut report = String::new();
        value.write_to(&mut report)?;
        std::fs::write(output.join(format!("{id}.txt")), report)?;
        capture(ui, output, &id)?;
    }
    state.borrow_mut().analysis_observation = Observation::DISABLED;
    ui.set_health_analysis(false);
    super::smoke::nav(ui, width, 6);
    refresh(ui, &state.borrow());
    println!("Analysis observation replay / 8 states / {name}: PASS (no live worker)");
    Ok(())
}
