use super::*;
use pajoniiir_service::{AnalysisStatus, Health, Snapshot};

fn observed(value: Option<u32>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or("NOT MEASURED".into())
}
pub fn refresh(ui: &MainWindow, s: Snapshot) {
    let board = profile(s.identity.board);
    let (label, severity, recovery) = match s.health() {
        Health::DemoMediaReady => (
            "HOST / DEMO MEDIA READY",
            0,
            "No active media/analysis fault in this host session.",
        ),
        Health::AnalysisFallback => (
            "HOST / ANALYSIS FALLBACK",
            1,
            "Load valid analysis in Library, or continue with BPM fallback.",
        ),
        Health::MediaUnavailable => (
            "HOST / MEDIA REMOVED",
            2,
            "Reconnect media in Library, then explicitly reload each deck.",
        ),
        Health::IdentityMismatch => (
            "UNIT IDENTITY MISMATCH",
            2,
            "Check exact unit, silicon and memory against the selected profile.",
        ),
        Health::BootOnly => (
            "BOOT OBSERVATIONS ONLY",
            1,
            "Peripherals and product runtime are not qualified.",
        ),
    };
    ui.set_health_status(label.into());
    ui.set_health_severity(severity);
    ui.set_health_recovery(recovery.into());
    ui.set_health_identity(format!("IDENTITY / DOCUMENTED CANDIDATE\n{}\nEnvironment: {:?}\nImage ID: {}\nChip observed: {}\nMinimum revision (documented): {}\nFlash observed: {}\nFlash documented: {} MiB\nPSRAM observed: {}\nPSRAM documented: {} MiB\nReset code observed: {}\nIdentity check: {:?}\nProduct acceptance: NOT QUALIFIED", board.model, s.identity.environment, s.identity.image_label(), observed(s.identity.chip_revision.map(u32::from)), board.min_chip_revision, observed(s.identity.flash_bytes), board.flash_bytes / 1024 / 1024, observed(s.identity.psram_bytes), board.psram_bytes / 1024 / 1024, observed(s.identity.reset_code), s.identity.check()).into());
    let analysis = |state| match state {
        AnalysisStatus::Unloaded => "UNLOADED",
        AnalysisStatus::Ready => "PUBLISHED",
        AnalysisStatus::Missing => "MISSING / BPM FALLBACK",
        AnalysisStatus::Rejected => "REJECTED / BPM FALLBACK",
    };
    let c = s.counters;
    ui.set_health_runtime(format!("HOST SESSION / OBSERVED EVENTS\nElapsed input clock: {} ms\nMedia: {:?}\nDeck 1 analysis: {}\nDeck 2 analysis: {}\nSemantic events delivered: {}\nMedia connect / remove: {} / {}\nLoads attempt / complete / blocked: {} / {} / {}\nAnalysis published / missing / rejected: {} / {} / {}\nPlayback rejected / track ends: {} / {}\nHistory retained / overwritten: {} / {}\nAudio / USB / display: NO DEVICE ADAPTER", s.elapsed_ms, s.runtime.media, analysis(s.runtime.analysis[0]), analysis(s.runtime.analysis[1]), c.control_events, c.media_connects, c.media_removals, c.load_attempts, c.loads_completed, c.loads_blocked, c.analysis_ready, c.analysis_missing, c.analysis_rejected, c.playback_rejected, c.end_of_track, s.events.len(), s.events.overwritten).into());
    let events: Vec<slint::SharedString> = (0..s.events.len())
        .filter_map(|i| s.events.get(i))
        .map(|e| {
            format!(
                "#{}  {} ms  {}  {:?}",
                e.sequence,
                e.elapsed_ms,
                if e.kind.warning() { "WARN" } else { "INFO" },
                e.kind
            )
            .into()
        })
        .collect();
    ui.set_health_history(ModelRc::new(VecModel::from(events)));
    ui.set_health_history_label(
        format!(
            "{} recent events · {} overwritten · history is retained after recovery",
            s.events.len(),
            s.events.overwritten
        )
        .into(),
    );
}
