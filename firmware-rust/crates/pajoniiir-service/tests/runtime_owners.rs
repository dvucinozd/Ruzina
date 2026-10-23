//! Composes the actual firmware ProductRuntime source with real host owners.
//! The default product variant is tested here; library-feature target variants
//! additionally have cross-compile/link gates, not a host filesystem emulation.
#[allow(dead_code, unexpected_cfgs)]
#[path = "../../../apps/m1-firmware/src/product_runtime.rs"]
mod product_runtime;
#[allow(dead_code)]
#[path = "../../../apps/m1-firmware/src/transport_owner_state.rs"]
mod transport_owner_state;

use pajoniiir_controller_core::{ControlEvent, ControlValue, DeckId, SemanticControl};
use pajoniiir_controller_session::{ControllerSession, IngestOutcome, SessionDispatchOutcome};
use pajoniiir_media_session::{DisconnectResult, MediaHandle, MediaSession, MediaSourceId};
use pajoniiir_service::{
    handoff::{Condition, HANDOFF_CAPACITY, Producer, SendError},
    owners::{
        CONTROL_MAX_AGE_MS, ControllerObservation, MEDIA_MAX_AGE_MS, MediaObservation, MediaPhase,
        RuntimeOwners,
    },
};
use product_runtime::ProductRuntime;

fn play() -> ControlEvent {
    ControlEvent {
        deck: Some(DeckId::One),
        control: SemanticControl::Play,
        value: ControlValue::Pressed(true),
    }
}
struct Harness {
    owners: RuntimeOwners,
    product: ProductRuntime,
    controller: ControllerSession,
    media: MediaSession,
    product_tx: Producer,
    controller_tx: Producer,
    media_tx: Producer,
}
impl Harness {
    fn new() -> Self {
        let mut owners = RuntimeOwners::new();
        let product_tx = owners.product.restart().unwrap();
        let controller_tx = owners.controller.restart().unwrap();
        let media_tx = owners.media.initial().unwrap();
        Self {
            owners,
            product: ProductRuntime::new(),
            controller: ControllerSession::new(),
            media: MediaSession::new(),
            product_tx,
            controller_tx,
            media_tx,
        }
    }
    fn publish(&mut self, now: u64, pressure: bool) {
        let sample = self
            .product_tx
            .sample(now, self.product.health_observation(pressure))
            .unwrap();
        self.owners.product.try_send(sample).unwrap();
        let sample = self
            .controller_tx
            .sample(
                now,
                ControllerObservation::capture(&self.controller, pressure),
            )
            .unwrap();
        self.owners.controller.try_send(sample).unwrap();
        let sample = self
            .media_tx
            .sample(
                now,
                MediaObservation::capture(&self.media, MediaPhase::Idle, 0),
            )
            .unwrap();
        self.owners.media.channel.try_send(sample).unwrap();
    }
    fn connect_controller(&mut self) {
        self.controller.set_connected(true);
        assert!(matches!(
            self.controller.dispatch_next(None, &mut self.product),
            SessionDispatchOutcome::Accepted(_)
        ));
    }
    fn trace(&mut self, now: u64, label: &str) {
        let view = self.owners.observe(now);
        let mut report = String::new();
        view.write_to(&mut report).unwrap();
        println!("SCENARIO={label}\n{report}");
    }
}

#[test]
fn composed_idle_missing_publications_and_owner_recovery_are_independent() {
    let mut h = Harness::new();
    h.publish(0, false);
    h.trace(0, "initial-idle");
    assert_eq!(h.owners.view(0).product.condition, Condition::Idle);
    assert_eq!(
        h.owners.view(0).controller.condition,
        Condition::Disconnected
    );
    // Product stops; real controller/media owners continue publishing.
    let now = CONTROL_MAX_AGE_MS + 1;
    let sample = h
        .controller_tx
        .sample(now, ControllerObservation::capture(&h.controller, false))
        .unwrap();
    h.owners.controller.try_send(sample).unwrap();
    let sample = h
        .media_tx
        .sample(
            now,
            MediaObservation::capture(&h.media, MediaPhase::Idle, 0),
        )
        .unwrap();
    h.owners.media.channel.try_send(sample).unwrap();
    h.trace(now, "product-stopped-controller-media-current");
    assert_eq!(h.owners.view(now).product.condition, Condition::Stale);
    assert_eq!(
        h.owners.view(now).controller.condition,
        Condition::Disconnected
    );
    assert_eq!(h.owners.view(now).media.condition, Condition::Disconnected);
    // UI/report reads are not owner progress.
    assert_eq!(
        h.owners
            .view(now)
            .product
            .observation
            .stamp
            .unwrap()
            .sampled_ms,
        0
    );
    h.publish(now + 1, false);
    h.trace(now + 1, "idle-recovery");
    assert_eq!(h.owners.view(now + 1).product.condition, Condition::Idle);
    assert_eq!(
        h.owners
            .view(now + 1 + MEDIA_MAX_AGE_MS + 1)
            .media
            .condition,
        Condition::Stale
    );
}

#[test]
fn actual_product_effect_queue_backpressure_retries_once_and_history_recovers() {
    let mut h = Harness::new();
    h.connect_controller();
    h.publish(0, false);
    h.owners.observe(0);
    for _ in 0..16 {
        assert_eq!(
            h.controller.schedule_event(play()),
            IngestOutcome::Scheduled
        );
        assert!(matches!(
            h.controller.dispatch_next(None, &mut h.product),
            SessionDispatchOutcome::Accepted(_)
        ));
    }
    assert_eq!(h.product.pending_transport_effects(), 16);
    h.controller.schedule_event(play());
    assert!(matches!(
        h.controller.dispatch_next(None, &mut h.product),
        SessionDispatchOutcome::Backpressured(_)
    ));
    assert_eq!(h.product.stats().accepted_deck_controls, 16);
    assert_eq!(h.product.stats().effect_backpressure, 1);
    h.publish(10, true);
    h.trace(10, "real-effect-queue-full");
    assert_eq!(
        h.owners.view(10).product.condition,
        Condition::Backpressured
    );
    assert_eq!(
        h.owners.view(10).controller.condition,
        Condition::Backpressured
    );
    // A single completed effect allows the held delivery through, without new input.
    assert!(h.product.complete_transport_effect(true));
    assert!(matches!(
        h.controller.dispatch_next(None, &mut h.product),
        SessionDispatchOutcome::Accepted(_)
    ));
    assert_eq!(h.product.stats().accepted_deck_controls, 17);
    assert!(matches!(
        h.controller.dispatch_next(None, &mut h.product),
        SessionDispatchOutcome::Idle
    ));
    while h.product.complete_transport_effect(true) {}
    h.publish(20, false);
    h.trace(20, "effect-retry-recovered");
    assert_eq!(h.owners.view(20).product.condition, Condition::Idle);
    assert_eq!(
        h.owners
            .view(20)
            .product
            .value
            .unwrap()
            .counters
            .effect_backpressure,
        1
    );
    assert_eq!(
        h.owners
            .view(20)
            .controller
            .value
            .unwrap()
            .counters
            .downstream_retries,
        1
    );
}

#[test]
fn diagnostic_overflow_does_not_block_controls_or_other_owners() {
    let mut h = Harness::new();
    h.connect_controller();
    for now in 0..HANDOFF_CAPACITY as u64 {
        let sample = h
            .product_tx
            .sample(now, h.product.health_observation(false))
            .unwrap();
        h.owners.product.try_send(sample).unwrap();
    }
    let sample = h
        .product_tx
        .sample(2, h.product.health_observation(false))
        .unwrap();
    assert_eq!(h.owners.product.try_send(sample), Err(SendError::Full));
    h.controller.schedule_event(play());
    assert!(matches!(
        h.controller.dispatch_next(None, &mut h.product),
        SessionDispatchOutcome::Accepted(_)
    ));
    assert_eq!(h.product.stats().accepted_deck_controls, 1);
    let now = CONTROL_MAX_AGE_MS + 10;
    let sample = h
        .controller_tx
        .sample(now, ControllerObservation::capture(&h.controller, false))
        .unwrap();
    h.owners.controller.try_send(sample).unwrap();
    let sample = h
        .media_tx
        .sample(
            now,
            MediaObservation::capture(&h.media, MediaPhase::Idle, 0),
        )
        .unwrap();
    h.owners.media.channel.try_send(sample).unwrap();
    h.trace(now, "delayed-observer-and-full-diagnostics");
    let view = h.owners.view(now);
    assert_eq!(view.product.condition, Condition::Stale);
    assert_eq!(view.product.observation.stamp.unwrap().sampled_ms, 1);
    assert_eq!(view.product.handoff.queue_full, 1);
    assert_eq!(view.product.handoff.accepted, HANDOFF_CAPACITY as u64);
    assert_eq!(view.controller.condition, Condition::Idle);
    assert_eq!(view.media.condition, Condition::Disconnected);
    h.publish(now, false);
    h.owners.observe(now);
    assert_eq!(h.owners.view(now).product.condition, Condition::Active);
}

#[test]
fn media_lifecycle_invalidates_suspended_worker_and_matches_event_kind() {
    let mut h = Harness::new();
    h.media.on_connect(MediaSourceId::new(1).unwrap());
    let lease = h.media.lease().unwrap();
    let handle = MediaHandle::new(1).unwrap();
    assert!(h.media.bind_handle(lease, handle));
    h.owners.media.invalidate(lease, handle, false);
    h.media_tx = h.owners.media.claim(lease, handle, false).unwrap();
    let late = h
        .media_tx
        .sample(
            10,
            MediaObservation::capture(&h.media, MediaPhase::Discovering, 0),
        )
        .unwrap();
    h.owners.media.channel.try_send(late).unwrap();
    assert_eq!(
        h.media.on_disconnect(Some(handle)),
        DisconnectResult::Accepted
    );
    h.owners.media.invalidate(lease, handle, true);
    assert_eq!(h.owners.view(11).media.condition, Condition::Awaiting);
    assert_eq!(
        h.owners.media.channel.try_send(late),
        Err(SendError::UnboundOrSuperseded)
    );
    assert!(h.owners.media.claim(lease, handle, false).is_none());
    let mut detached = h.owners.media.claim(lease, handle, true).unwrap();
    let late_detach = detached
        .sample(
            11,
            MediaObservation::capture(&h.media, MediaPhase::Detached, 0),
        )
        .unwrap();
    h.owners.media.channel.try_send(late_detach).unwrap();
    h.owners.observe(11);
    assert_eq!(h.owners.view(11).media.condition, Condition::Disconnected);
    h.media.on_connect(MediaSourceId::new(1).unwrap());
    let new_lease = h.media.lease().unwrap();
    let new_handle = MediaHandle::new(2).unwrap();
    assert!(h.media.bind_handle(new_lease, new_handle));
    h.owners.media.invalidate(new_lease, new_handle, false);
    assert!(h.owners.media.claim(lease, handle, true).is_none());
    assert_eq!(
        h.owners.media.channel.try_send(late_detach),
        Err(SendError::UnboundOrSuperseded)
    );
    h.media_tx = h.owners.media.claim(new_lease, new_handle, false).unwrap();
    assert_eq!(
        h.media.on_disconnect(Some(handle)),
        DisconnectResult::IgnoredForeign
    );
    assert!(h.media.commit_mounted(new_lease));
    let sample = h
        .media_tx
        .sample(
            12,
            MediaObservation::capture(&h.media, MediaPhase::Mounted, 0),
        )
        .unwrap();
    h.owners.media.channel.try_send(sample).unwrap();
    h.trace(12, "media-replaced-late-worker-rejected");
    assert_eq!(h.owners.view(12).media.condition, Condition::Idle);
    assert_eq!(
        h.owners.view(12).media.value.unwrap().lease,
        Some(new_lease)
    );
    assert!(h.owners.view(12).media.handoff.discarded_on_restart > 0);
    assert_eq!(h.owners.view(12).product.condition, Condition::Awaiting);
}

#[test]
fn current_errors_recover_but_totals_survive_and_rejected_samples_do_not_clear_them() {
    let mut h = Harness::new();
    h.publish(0, false);
    h.owners.observe(0);
    h.product.note_library_failure();
    h.publish(1, false);
    h.trace(1, "new-library-error");
    assert_eq!(h.owners.view(1).product.condition, Condition::NewErrors);
    let stamp = h.owners.view(1).product.observation.stamp.unwrap();
    let mut replay = h
        .product_tx
        .sample(2, h.product.health_observation(false))
        .unwrap();
    replay.stamp = stamp;
    h.owners.product.try_send(replay).unwrap();
    h.owners.observe(2);
    assert_eq!(h.owners.view(2).product.condition, Condition::NewErrors);
    assert_eq!(h.owners.view(2).product.handoff.rejected, 1);
    h.publish(3, false);
    h.owners.observe(3);
    assert_eq!(h.owners.view(3).product.condition, Condition::Idle);
    assert_eq!(
        h.owners
            .view(3)
            .product
            .value
            .unwrap()
            .counters
            .library_failures,
        1
    );
    let mut reset = h.product.health_observation(false);
    reset.counters.library_failures = 0;
    h.owners
        .product
        .try_send(h.product_tx.sample(4, reset).unwrap())
        .unwrap();
    h.owners.observe(4);
    assert_eq!(h.owners.view(4).product.condition, Condition::CounterReset);
    let old = h
        .product_tx
        .sample(5, h.product.health_observation(false))
        .unwrap();
    h.product_tx = h.owners.product.restart().unwrap();
    assert_eq!(
        h.owners.product.try_send(old),
        Err(SendError::UnboundOrSuperseded)
    );
    h.publish(5, false);
    h.trace(5, "new-owner-baseline-retains-history");
    assert_eq!(h.owners.view(5).product.condition, Condition::Idle);
    assert_eq!(
        h.owners
            .view(5)
            .product
            .value
            .unwrap()
            .counters
            .library_failures,
        1
    );
}

#[test]
fn reports_remain_bounded_and_unqualified() {
    use pajoniiir_board_profile::diagnostics::ReportBuffer;
    let mut h = Harness::new();
    h.publish(0, false);
    let view = h.owners.observe(0);
    let mut report = ReportBuffer::<8192>::default();
    view.write_to(&mut report).unwrap();
    let text = core::str::from_utf8(report.bytes()).unwrap();
    assert!(text.starts_with("M1_OWNER_HEALTH_V1\n"));
    assert!(text.ends_with("readiness=NOT_QUALIFIED\n"));
    assert!(view.write_to(&mut ReportBuffer::<32>::default()).is_err());
    assert!(core::mem::size_of::<RuntimeOwners>() < 8192);
}

#[test]
fn controller_disconnect_reconnect_rejects_old_generation_without_erasing_errors() {
    let mut h = Harness::new();
    h.controller.set_connected(true);
    assert!(matches!(
        h.controller.dispatch_next(None, &mut h.product),
        SessionDispatchOutcome::Accepted(_)
    ));
    h.publish(0, false);
    h.owners.observe(0);
    // Actual scheduler saturation; coalescing alone would not count as loss.
    for _ in 0..32 {
        assert_eq!(
            h.controller.schedule_event(play()),
            IngestOutcome::Scheduled
        );
    }
    assert_eq!(h.controller.schedule_event(play()), IngestOutcome::Dropped);
    h.publish(1, false);
    h.owners.observe(1);
    assert_eq!(h.owners.view(1).controller.condition, Condition::NewErrors);
    let late = h
        .controller_tx
        .sample(2, ControllerObservation::capture(&h.controller, false))
        .unwrap();
    h.controller.set_connected(false);
    h.controller_tx = h.owners.controller.restart().unwrap();
    assert_eq!(
        h.owners.controller.try_send(late),
        Err(SendError::UnboundOrSuperseded)
    );
    h.publish(3, false);
    h.owners.observe(3);
    assert_eq!(
        h.owners.view(3).controller.condition,
        Condition::Disconnected
    );
    h.controller.set_connected(true);
    h.controller.reset_profile_state();
    h.controller_tx = h.owners.controller.restart().unwrap();
    h.publish(4, false);
    h.trace(4, "controller-reconnected-history-retained");
    assert_eq!(h.owners.view(4).controller.condition, Condition::Idle);
    assert_eq!(
        h.owners
            .view(4)
            .controller
            .value
            .unwrap()
            .scheduler
            .fifo_full,
        1
    );
}

#[test]
fn setup_failure_before_ready_can_claim_a_terminal_producer_and_keep_publishing() {
    let mut h = Harness::new();
    h.media.on_connect(MediaSourceId::new(1).unwrap());
    let lease = h.media.lease().unwrap();
    let handle = MediaHandle::new(1).unwrap();
    assert!(h.media.bind_handle(lease, handle));
    h.owners.media.invalidate(lease, handle, false);
    // No Ready reached the worker: class/LUN/capacity preparation failed first.
    assert_eq!(
        h.media.on_disconnect(Some(handle)),
        DisconnectResult::Accepted
    );
    h.owners.media.invalidate(lease, handle, true);
    assert!(h.owners.media.claim(lease, handle, false).is_none());
    let mut terminal = h.owners.media.claim(lease, handle, true).unwrap();
    for now in [10, 5010, 10010] {
        let sample = terminal
            .sample(
                now,
                MediaObservation::capture(&h.media, MediaPhase::Failed, 1),
            )
            .unwrap();
        h.owners.media.channel.try_send(sample).unwrap();
        let view = h.owners.observe(now);
        assert_eq!(view.media.condition, Condition::Failed);
        assert_eq!(view.media.observation.age_ms, Some(0));
        assert_eq!(view.media.value.unwrap().failures, 1);
    }
    h.media.on_connect(MediaSourceId::new(1).unwrap());
    let lease = h.media.lease().unwrap();
    let handle = MediaHandle::new(2).unwrap();
    assert!(h.media.bind_handle(lease, handle));
    h.owners.media.invalidate(lease, handle, false);
    let mut recovered = h.owners.media.claim(lease, handle, false).unwrap();
    assert!(h.media.commit_mounted(lease));
    let sample = recovered
        .sample(
            10011,
            MediaObservation::capture(&h.media, MediaPhase::Mounted, 1),
        )
        .unwrap();
    h.owners.media.channel.try_send(sample).unwrap();
    assert_eq!(h.owners.observe(10011).media.condition, Condition::Idle);
}

#[test]
fn actual_product_keeps_shared_track_cue_edits_current_across_decks() {
    use pajoniiir_controller_core::{PadAction, PadMode};
    use pajoniiir_core::MediaTrackId;
    use pajoniiir_hot_cues::HotCueBank;
    use pajoniiir_media_catalog::{
        CatalogLoadCandidate, CatalogPublicationId, CatalogText, CatalogTrack,
    };
    let mut h = Harness::new();
    h.connect_controller();
    h.media.on_connect(MediaSourceId::new(1).unwrap());
    let track = MediaTrackId([7; 16]);
    let candidate = CatalogLoadCandidate {
        publication: CatalogPublicationId {
            lease: h.media.lease().unwrap(),
            revision: 1,
        },
        index: 0,
        track: CatalogTrack {
            track_id: track,
            source_track_id: 7,
            bpm_x100: 12000,
            duration_ms: 10000,
            title: CatalogText::EMPTY,
            artist: CatalogText::EMPTY,
            album: CatalogText::EMPTY,
            key: CatalogText::EMPTY,
            audio_path: CatalogText::EMPTY,
            analysis_path: CatalogText::EMPTY,
            artwork_path: CatalogText::EMPTY,
        },
    };
    let mut source = HotCueBank::empty(track);
    source.set_single(0, 1000);
    source.set_loop(1, 2000, 4000);
    for deck in [DeckId::One, DeckId::Two] {
        assert!(
            h.product
                .commit_library_load(deck, candidate, source, None, true, None)
        );
    }
    for (deck, pad, clear) in [(DeckId::One, 0, true), (DeckId::Two, 2, false)] {
        let event = ControlEvent {
            deck: Some(deck),
            control: SemanticControl::PadAction,
            value: ControlValue::PadAction(PadAction {
                pad,
                mode: PadMode::HotCue,
                shifted: clear,
                pressed: true,
            }),
        };
        assert_eq!(h.controller.schedule_event(event), IngestOutcome::Scheduled);
        assert!(matches!(
            h.controller.dispatch_next(None, &mut h.product),
            SessionDispatchOutcome::Accepted(_)
        ));
    }
    for deck in [DeckId::One, DeckId::Two] {
        let cues = h.product.deck().hot_cues(deck).unwrap();
        assert!(cues.slot(0).is_none());
        assert_eq!(cues.slot(1), source.slot(1));
        assert!(cues.slot(2).is_some());
    }
}
