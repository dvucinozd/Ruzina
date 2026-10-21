use super::smoke::{capture, click, nav};
use super::*;
use pajoniiir_service::{AnalysisStatus, Health};
use std::path::Path;

pub fn capture_health(
    ui: &MainWindow,
    state: &RefCell<Session>,
    output: &Path,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut report = pajoniiir_board_profile::diagnostics::ReportBuffer::<4096>::default();
    state
        .borrow()
        .diagnostics()
        .write_to(&mut report)
        .map_err(|_| "diagnostic report overflow")?;
    std::fs::write(output.join(format!("{name}.txt")), report.bytes())?;
    capture(ui, output, name)
}
pub fn recovery(
    ui: &MainWindow,
    state: &Rc<RefCell<Session>>,
    output: &Path,
    name: &str,
    width: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    nav(ui, width, 6);
    assert_eq!(ui.get_page(), 6);
    assert_eq!(
        state.borrow().diagnostics().health(),
        Health::MediaUnavailable
    );
    assert_eq!(ui.get_health_severity(), 2);
    capture_health(ui, state, output, &format!("{name}-health-removed"))?;
    click(ui, width / 2.0, 144.0);
    assert!(ui.get_health_events());
    if !ui.get_portrait() {
        let before = ui.window().take_snapshot()?;
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(180.0, 400.0),
                delta_x: 0.0,
                delta_y: -200.0,
            });
        slint::platform::update_timers_and_animations();
        let after = ui.window().take_snapshot()?;
        assert_ne!(before.as_slice(), after.as_slice());
        // Recreate the event list at its deterministic initial scroll position.
        ui.set_health_events(false);
        ui.window().take_snapshot()?;
        ui.set_health_events(true);
        ui.window().take_snapshot()?;
    }
    capture_health(ui, state, output, &format!("{name}-health-events"))?;
    let previous_rejections = state.borrow().diagnostics().counters.analysis_rejected;
    click(ui, width * 0.82, 144.0);
    assert_eq!(ui.get_page(), 1);
    // Recover through the same media action exposed by Library.
    ui.invoke_media_action();
    nav(ui, width, 6);
    ui.set_health_events(false);
    assert_eq!(
        state.borrow().diagnostics().health(),
        Health::DemoMediaReady
    );
    assert_eq!(ui.get_health_severity(), 0);
    assert_eq!(
        state.borrow().diagnostics().counters.analysis_rejected,
        previous_rejections
    );
    capture_health(ui, state, output, &format!("{name}-health-recovered"))?;
    ui.invoke_select_track(3);
    ui.invoke_load_track(0);
    assert_eq!(
        state.borrow().diagnostics().runtime.analysis[0],
        AnalysisStatus::Rejected
    );
    assert_eq!(ui.get_health_severity(), 1);
    capture_health(ui, state, output, &format!("{name}-health-fallback"))?;
    println!(
        "Health candidate/unknowns, events, removed media/recovery and actual parser rejection: PASS at width {width}"
    );
    Ok(())
}
