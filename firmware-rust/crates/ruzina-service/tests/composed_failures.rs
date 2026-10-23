//! Fault injection at owner boundaries, using the actual firmware transitions.
//! No P4 USB/flash driver or physical output timing is emulated here.
#[path = "../../../apps/m1-firmware/src/hotcue_access.rs"]
mod hotcue_access;
#[allow(dead_code, unexpected_cfgs)]
#[path = "../../../apps/m1-firmware/src/product_runtime.rs"]
mod product_runtime;
#[allow(dead_code)]
#[path = "../../../apps/m1-firmware/src/transport_owner_state.rs"]
mod transport_owner_state;

use hotcue_access::{HotCueAccess, HotCueLoadResult};
use product_runtime::ProductRuntime;
use ruzina_audio_runtime::{
    pcm_stream::{PcmStreamBlock, PcmStreamProgress},
    playback::PlaybackOutput,
    render::RenderOutputBlock,
    sink::{BlockProgress, SinkBlock, SinkRole},
};
use ruzina_controller_core::{ControlEvent, ControlValue, PadAction, PadMode, SemanticControl};
use ruzina_controller_session::{
    ConnectionState, ControllerSession, DeckId, IngestOutcome, SessionDelivery,
    SessionDeliverySink, SessionDispatchOutcome,
};
use ruzina_core::MediaTrackId;
use ruzina_deck::{DeckEffect, DeckLoadGate, LoadRefusal};
use ruzina_hot_cues::{HotCueBank, HotCueStore};
use ruzina_media_catalog::{CatalogLoadCandidate, CatalogPublicationId, CatalogText, CatalogTrack};
use ruzina_media_session::{MediaSession, MediaSourceId};
use std::{cell::RefCell, rc::Rc};
use transport_owner_state::{TransportCommand, TransportOwnerState};

struct Rig {
    product: ProductRuntime,
    transport: TransportOwnerState,
    controller: ControllerSession,
    media: MediaSession,
    gates: &'static [DeckLoadGate; 2],
}
impl Rig {
    fn new() -> Self {
        // Tests run in parallel; each rig owns its gates like one firmware image.
        let gates: &'static [DeckLoadGate; 2] =
            Box::leak(Box::new([DeckLoadGate::new(), DeckLoadGate::new()]));
        let mut rig = Self {
            gates,
            product: ProductRuntime::with_load_gates(gates),
            transport: TransportOwnerState::new(),
            controller: ControllerSession::new(),
            media: MediaSession::new(),
        };
        rig.media.on_connect(MediaSourceId::new(1).unwrap());
        assert!(
            rig.product
                .try_accept(SessionDelivery::Connection(ConnectionState::Connected))
        );
        rig.controller.set_connected(true);
        assert!(matches!(
            rig.controller.dispatch_next(None, &mut rig.product),
            SessionDispatchOutcome::Accepted(_)
        ));
        rig.load(DeckId::One, 1, None);
        rig.load(DeckId::Two, 2, None);
        rig
    }
    fn candidate(&self, track: u8) -> CatalogLoadCandidate {
        CatalogLoadCandidate {
            publication: CatalogPublicationId {
                lease: self.media.lease().unwrap(),
                revision: 1,
            },
            index: u16::from(track),
            track: CatalogTrack {
                track_id: MediaTrackId([track; 16]),
                source_track_id: u32::from(track),
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
        }
    }
    fn sync(&mut self) {
        self.transport.sync_source_epochs([
            self.product.source_epoch(DeckId::One),
            self.product.source_epoch(DeckId::Two),
        ]);
    }
    fn load(&mut self, deck: DeckId, track: u8, persisted: Option<HotCueBank>) {
        let candidate = self.candidate(track);
        assert!(self.product.commit_library_load(
            deck,
            candidate,
            HotCueBank::empty(candidate.track.track_id),
            persisted,
            false,
            None
        ));
        self.sync();
    }
    fn event(&mut self, deck: DeckId, control: SemanticControl, value: ControlValue) {
        assert!(
            self.product
                .try_accept(SessionDelivery::Control(ControlEvent {
                    deck: Some(deck),
                    control,
                    value
                }))
        );
    }
    fn cue(&mut self, deck: DeckId, pad: u8) {
        self.event(
            deck,
            SemanticControl::PadAction,
            ControlValue::PadAction(PadAction {
                pad,
                mode: PadMode::HotCue,
                shifted: false,
                pressed: true,
            }),
        );
    }
    fn next_command(&mut self) -> TransportCommand {
        let effect = self.product.next_transport_effect().unwrap();
        let command = TransportCommand {
            effect,
            source_epoch: self.product.source_epoch(effect.source_deck().unwrap()),
        };
        assert!(self.product.complete_transport_effect(true));
        command
    }
    fn play(&mut self, deck: DeckId) {
        self.event(deck, SemanticControl::Play, ControlValue::Pressed(true));
        let command = self.next_command();
        let completion = self.transport.apply(command).unwrap();
        assert!(self.product.apply_transport_completion(completion, 0));
        assert!(self.product.deck().deck(deck).playing);
    }
}

#[test]
fn failed_seek_unloads_only_its_deck_and_can_reload_without_resetting_other_deck() {
    let mut rig = Rig::new();
    rig.play(DeckId::One);
    rig.play(DeckId::Two);
    rig.event(
        DeckId::One,
        SemanticControl::BeatJumpForward,
        ControlValue::Pressed(true),
    );
    let command = rig.next_command();
    assert!(matches!(command.effect, DeckEffect::Seek { .. }));
    // The target source has revoked its stream after filesystem seek failure.
    let completion = rig.transport.fail_seek(command).unwrap();
    assert!(rig.product.apply_transport_completion(completion, 0));
    rig.sync();
    assert!(rig.product.loaded_track(DeckId::One).is_none());
    assert!(!rig.product.deck().deck(DeckId::One).playing);
    assert!(rig.product.deck().deck(DeckId::Two).playing);
    assert!(rig.transport.decks[1].playing);
    assert_eq!(rig.transport.decks[0].stream_faults, 1);
    rig.load(DeckId::One, 3, None);
    rig.play(DeckId::One);
    assert!(!rig.product.apply_transport_completion(completion, 0));
    assert!(rig.transport.fail_seek(command).is_none());
    assert!(rig.product.deck().deck(DeckId::One).playing);
    assert!(rig.product.deck().deck(DeckId::Two).playing);
}

#[test]
fn source_read_fault_stops_only_its_deck_and_old_eof_cannot_stop_replacement() {
    let mut rig = Rig::new();
    rig.play(DeckId::One);
    rig.play(DeckId::Two);
    let mut block = PcmStreamBlock::empty(10, 48000, 48000);
    block.mark_faulted();
    let completion = rig
        .transport
        .consume_pcm_block(DeckId::One, &block)
        .unwrap();
    assert!(rig.product.apply_transport_completion(completion, 10));
    assert_eq!(rig.product.deck().deck(DeckId::One).position_ms, 1000);
    assert!(!rig.product.deck().deck(DeckId::One).playing);
    assert!(rig.product.loaded_track(DeckId::One).is_some());
    assert!(rig.product.deck().deck(DeckId::Two).playing);
    rig.load(DeckId::One, 3, None);
    rig.play(DeckId::One);
    assert!(!rig.product.apply_transport_completion(completion, 10));
    assert_eq!(rig.product.deck().deck(DeckId::One).position_ms, 0);
    assert!(rig.product.deck().deck(DeckId::One).playing);
}

#[test]
fn rendered_fault_waits_for_sink_receipt_and_does_not_stop_other_deck() {
    let mut rig = Rig::new();
    rig.play(DeckId::One);
    rig.play(DeckId::Two);
    let mut main = [SinkBlock::EMPTY; 2];
    let mut phones = [SinkBlock::EMPTY; 1];
    let mut output = PlaybackOutput::new_reference(&mut main, &mut phones).unwrap();
    output.sync_sources([10, 20], 0).unwrap();
    let progress = BlockProgress {
        decks: [Some(progress(10, true)), Some(progress(20, false))],
        source_frames: [128; 2],
    };
    let block = RenderOutputBlock::silence();
    assert_eq!(
        output.submit_render(output.render_permit(), 0, &block, progress)[0],
        Ok(())
    );
    assert!(output.pending_progress(0).is_none());
    assert!(rig.product.deck().deck(DeckId::One).playing);
    let token = output.lane(SinkRole::Main).token().unwrap();
    let mut pcm = block.main;
    let receipt = output
        .lane_mut(SinkRole::Main)
        .issue(token, 0, 128, &mut pcm)
        .unwrap();
    output.complete(receipt).unwrap();
    let (mark, frames) = output.pending_progress(0).unwrap();
    let completion = rig
        .transport
        .consume_render_progress(DeckId::One, mark, frames, 10)
        .unwrap();
    assert!(rig.product.apply_transport_completion(completion, 10));
    assert!(!rig.product.deck().deck(DeckId::One).playing);
    assert!(rig.product.deck().deck(DeckId::Two).playing);
    assert_eq!(output.confirmed_progress(1).unwrap().generation, 20);
}

fn progress(generation: u32, faulted: bool) -> PcmStreamProgress {
    PcmStreamProgress {
        generation,
        end_frame: 128,
        sample_rate_hz: 48000,
        eof: faulted,
        faulted,
    }
}

#[test]
fn late_terminal_progress_after_seek_is_rejected_even_with_same_load_epoch() {
    let mut rig = Rig::new();
    rig.play(DeckId::One);
    let completion = rig
        .transport
        .consume_render_progress(DeckId::One, progress(10, true), 128, 10)
        .unwrap();
    // A successful seek changes the stream, but not the loaded-track epoch.
    assert!(!rig.product.apply_transport_completion(completion, 11));
    assert!(rig.product.deck().deck(DeckId::One).playing);
    assert!(
        rig.transport
            .consume_render_progress(DeckId::One, progress(10, true), 128, 11)
            .is_none()
    );
}

#[test]
fn failed_preflight_keeps_old_track_but_failed_prepare_retires_only_revoked_source() {
    let mut rig = Rig::new();
    rig.play(DeckId::Two);
    let original = rig.product.loaded_track(DeckId::One);
    rig.product.fail_library_load(None, false); // stat/analysis/cue read before replacement
    assert_eq!(rig.product.loaded_track(DeckId::One), original);
    rig.product.fail_library_load(Some(DeckId::One), false); // Prepare has revoked old PCM
    assert!(rig.product.loaded_track(DeckId::One).is_none());
    assert!(rig.product.deck().deck(DeckId::Two).playing);
    assert_eq!(rig.product.stats().library_failures, 2);
    rig.load(DeckId::One, 3, None);
    rig.play(DeckId::One);
}

#[test]
fn media_detach_cancels_all_affected_sources_but_retains_unconfirmed_cue_write() {
    let mut rig = Rig::new();
    rig.cue(DeckId::One, 0);
    let dirty = rig.product.next_persistence_effect();
    rig.play(DeckId::One);
    rig.play(DeckId::Two);
    assert_eq!(rig.product.invalidate_loaded_media(None), 2);
    rig.sync();
    for deck in [DeckId::One, DeckId::Two] {
        assert!(rig.product.loaded_track(deck).is_none());
        assert!(!rig.product.deck().deck(deck).playing);
    }
    assert_eq!(rig.product.next_persistence_effect(), dirty);
    assert_eq!(rig.product.invalidate_loaded_media(None), 0);
    rig.media.on_disconnect(None);
    rig.media.on_connect(MediaSourceId::new(1).unwrap());
    rig.load(DeckId::One, 1, None);
    assert!(
        rig.product
            .deck()
            .hot_cues(DeckId::One)
            .unwrap()
            .slot(0)
            .is_some()
    );
    rig.play(DeckId::One);
}

#[test]
fn source_invalidation_releases_full_transport_queue_and_preserves_other_deck_order() {
    let mut rig = Rig::new();
    rig.cue(DeckId::One, 0);
    for _ in 0..15 {
        rig.event(
            DeckId::One,
            SemanticControl::Play,
            ControlValue::Pressed(true),
        );
    }
    // Cue creation contributes no transport effect; fill the 16th slot with B.
    rig.event(
        DeckId::Two,
        SemanticControl::Play,
        ControlValue::Pressed(true),
    );
    assert_eq!(rig.product.pending_transport_effects(), 16);
    let held = ControlEvent {
        deck: Some(DeckId::Two),
        control: SemanticControl::Play,
        value: ControlValue::Pressed(true),
    };
    assert_eq!(
        rig.controller.schedule_event(held),
        IngestOutcome::Scheduled
    );
    assert!(matches!(
        rig.controller.dispatch_next(None, &mut rig.product),
        SessionDispatchOutcome::Backpressured(_)
    ));
    let old = TransportCommand {
        effect: rig.product.next_transport_effect().unwrap(),
        source_epoch: rig.product.source_epoch(DeckId::One),
    };
    rig.product.invalidate_deck_source(DeckId::One);
    rig.sync();
    assert!(rig.transport.apply(old).is_none());
    assert_eq!(rig.product.pending_transport_effects(), 1);
    assert_eq!(
        rig.product.next_transport_effect().unwrap().source_deck(),
        Some(DeckId::Two)
    );
    assert_eq!(rig.product.pending_persistence_effects(), 1);
    assert!(matches!(
        rig.controller.dispatch_next(None, &mut rig.product),
        SessionDispatchOutcome::Accepted(_)
    ));
}

#[derive(Default)]
struct StoreState {
    fail_read: bool,
    fail_write: bool,
    reads: usize,
    writes: usize,
    bank: Option<HotCueBank>,
}
struct Store(Rc<RefCell<StoreState>>);
impl HotCueStore for Store {
    type Error = ();
    fn load(&mut self, _: MediaTrackId) -> Result<Option<HotCueBank>, ()> {
        let mut state = self.0.borrow_mut();
        state.reads += 1;
        if state.fail_read {
            Err(())
        } else {
            Ok(state.bank)
        }
    }
    fn save(&mut self, bank: &HotCueBank) -> Result<(), ()> {
        let mut state = self.0.borrow_mut();
        state.writes += 1;
        if state.fail_write {
            Err(())
        } else {
            state.bank = Some(*bank);
            Ok(())
        }
    }
    fn clear(&mut self, _: MediaTrackId) -> Result<(), ()> {
        unreachable!()
    }
}

#[test]
fn write_failure_answers_queued_library_reads_and_rejects_edits_without_holding_controller() {
    let mut rig = Rig::new();
    rig.play(DeckId::Two);
    rig.cue(DeckId::One, 0);
    let dirty = rig.product.next_persistence_effect().unwrap();
    let DeckEffect::PersistHotCues { bank, .. } = dirty else {
        panic!("cue write");
    };
    let shared = Rc::new(RefCell::new(StoreState {
        fail_write: true,
        ..StoreState::default()
    }));
    let mut access = HotCueAccess::new(Some(Store(shared.clone())));
    assert!(!access.save(&bank));
    assert!(access.faulted());
    assert!(rig.product.complete_persistence_effect(false));
    rig.product.note_persistence_fault();
    // A request queued before the write failed still gets a terminal result.
    assert_eq!(access.load(bank.track_id()), HotCueLoadResult::Failed);
    assert!(!access.save(&bank));
    assert_eq!(shared.borrow().reads, 0);
    assert_eq!(shared.borrow().writes, 1);
    assert!(rig.product.persistence_faulted());
    let previous = rig.product.deck().hot_cues(DeckId::One);
    for event in [
        ControlEvent {
            deck: Some(DeckId::One),
            control: SemanticControl::PadAction,
            value: ControlValue::PadAction(PadAction {
                pad: 1,
                mode: PadMode::HotCue,
                shifted: false,
                pressed: true,
            }),
        },
        ControlEvent {
            deck: Some(DeckId::Two),
            control: SemanticControl::Play,
            value: ControlValue::Pressed(true),
        },
    ] {
        assert_eq!(
            rig.controller.schedule_event(event),
            IngestOutcome::Scheduled
        );
    }
    assert!(matches!(
        rig.controller.dispatch_next(None, &mut rig.product),
        SessionDispatchOutcome::Accepted(_)
    ));
    assert_eq!(rig.product.deck().hot_cues(DeckId::One), previous);
    assert_eq!(rig.product.next_persistence_effect(), Some(dirty));
    assert_eq!(rig.product.pending_persistence_effects(), 1);
    assert_eq!(rig.product.stats().deferred_controls, 1);
    assert!(rig.product.deck().deck(DeckId::Two).playing);
    // Ordinary transport after the rejected edit remains usable.
    assert!(matches!(
        rig.controller.dispatch_next(None, &mut rig.product),
        SessionDispatchOutcome::Accepted(_)
    ));
    let command = rig.next_command();
    let completion = rig.transport.apply(command).unwrap();
    assert!(rig.product.apply_transport_completion(completion, 0));
    assert!(!rig.product.deck().deck(DeckId::Two).playing);
}

#[test]
fn unavailable_or_corrupt_cue_store_returns_failure_instead_of_an_empty_bank() {
    let mut unavailable = HotCueAccess::<Store>::new(None);
    assert!(unavailable.faulted());
    assert_eq!(
        unavailable.load(MediaTrackId([1; 16])),
        HotCueLoadResult::Failed
    );
    let shared = Rc::new(RefCell::new(StoreState {
        fail_read: true,
        ..StoreState::default()
    }));
    let mut corrupt = HotCueAccess::new(Some(Store(shared.clone())));
    assert_eq!(
        corrupt.load(MediaTrackId([1; 16])),
        HotCueLoadResult::Failed
    );
    shared.borrow_mut().fail_read = false;
    assert_eq!(
        corrupt.load(MediaTrackId([1; 16])),
        HotCueLoadResult::Failed
    );
    assert_eq!(shared.borrow().reads, 1); // no automatic retry after uncertain storage
    let mut restarted = HotCueAccess::new(Some(Store(shared)));
    assert_eq!(
        restarted.load(MediaTrackId([1; 16])),
        HotCueLoadResult::Ready(None)
    );
}

#[test]
fn loading_same_track_keeps_latest_accepted_bank_before_and_after_write_completion() {
    let mut rig = Rig::new();
    rig.cue(DeckId::One, 0);
    rig.cue(DeckId::One, 1);
    let latest = rig.product.deck().local_hot_cues(DeckId::One).unwrap();
    let stale = HotCueBank::empty(latest.track_id());
    rig.load(DeckId::Two, 1, Some(stale));
    assert_eq!(rig.product.deck().local_hot_cues(DeckId::Two), Some(latest));
    while rig.product.complete_persistence_effect(true) {}
    rig.load(DeckId::One, 1, Some(stale));
    assert_eq!(rig.product.deck().local_hot_cues(DeckId::One), Some(latest));
    assert_eq!(rig.product.deck().local_hot_cues(DeckId::Two), Some(latest));
}

#[test]
fn confirmed_wrapped_position_drives_real_product_loop_controls_and_clear() {
    use ruzina_audio_runtime::{
        wav::{WavPcm16Format, WavPcm16SourceDescriptor},
        wav_stream::{WavStreamChange, WavStreamDecoder},
    };
    let mut rig = Rig::new();
    rig.play(DeckId::One);
    rig.play(DeckId::Two);
    for (frame, ms) in [(48000, 1000), (24000, 500)] {
        let mark = PcmStreamProgress {
            generation: 10,
            end_frame: frame,
            sample_rate_hz: 48000,
            eof: false,
            faulted: false,
        };
        let completion = rig
            .transport
            .consume_render_progress(DeckId::One, mark, 128, 10)
            .unwrap();
        assert!(rig.product.apply_transport_completion(completion, 10));
        assert_eq!(rig.product.deck().deck(DeckId::One).position_ms, ms);
        assert!(rig.product.deck().deck(DeckId::One).playing);
    }
    rig.event(
        DeckId::One,
        SemanticControl::LoopIn,
        ControlValue::Pressed(true),
    );
    let mark = PcmStreamProgress {
        generation: 10,
        end_frame: 36000,
        sample_rate_hz: 48000,
        eof: false,
        faulted: false,
    };
    let completion = rig
        .transport
        .consume_render_progress(DeckId::One, mark, 128, 10)
        .unwrap();
    assert!(rig.product.apply_transport_completion(completion, 10));
    rig.event(
        DeckId::One,
        SemanticControl::LoopOut,
        ControlValue::Pressed(true),
    );
    let command = rig.next_command();
    assert_eq!(
        command.effect,
        DeckEffect::SetLoop {
            deck: DeckId::One,
            start_ms: 500,
            end_ms: 750
        }
    );
    let descriptor = WavPcm16SourceDescriptor::new(
        WavPcm16Format {
            channels: 2,
            sample_rate_hz: 48000,
            byte_rate: 192000,
            block_align: 4,
        },
        44,
        480000 * 4,
    )
    .unwrap();
    let stream = WavStreamDecoder::new(descriptor, 10, 0, None).unwrap();
    let mut looped = stream
        .changed(
            11,
            WavStreamChange::Loop {
                position_frame: rig.transport.decks[0].pcm_cursor.position_frame(),
                region_ms: Some((500, 750)),
            },
        )
        .unwrap();
    assert_eq!(looped.media_frame(), 24000);
    assert!(
        rig.transport
            .bind_source_control(command, 11, looped.media_frame(), 48000)
    );
    let block = looped.decode(&[1u8; 512]).unwrap();
    let completion = rig
        .transport
        .consume_pcm_block(DeckId::One, &block)
        .unwrap();
    assert!(rig.product.apply_transport_completion(completion, 11));
    assert_eq!(rig.product.deck().deck(DeckId::One).position_ms, 502);
    assert!(rig.product.deck().deck(DeckId::Two).playing);
    rig.event(
        DeckId::One,
        SemanticControl::ReloopExit,
        ControlValue::Pressed(true),
    );
    let clear = rig.next_command();
    assert_eq!(clear.effect, DeckEffect::ClearLoop { deck: DeckId::One });
    let linear = looped
        .changed(
            12,
            WavStreamChange::Loop {
                position_frame: rig.transport.decks[0].pcm_cursor.position_frame(),
                region_ms: None,
            },
        )
        .unwrap();
    assert!(linear.timeline().loop_region().is_none());
    assert!(
        rig.transport
            .bind_source_control(clear, 12, linear.media_frame(), 48000)
    );
    assert!(rig.transport.decks[0].loop_region.is_none());
    assert!(
        rig.product
            .deck()
            .deck(DeckId::One)
            .loop_state
            .active
            .is_none()
    );
    assert!(!rig.product.apply_transport_completion(completion, 12));
    assert!(rig.product.deck().deck(DeckId::One).playing);
}

// RP-04 with the actual firmware product owner. The Library worker's
// begin_replacement/finish calls are driven explicitly at its two boundaries:
// immediately before Prepare releases the old session, and after commit.
#[test]
fn play_during_admitted_load_withdraws_it_and_keeps_the_old_session() {
    let mut rig = Rig::new();
    let original = rig.product.loaded_track(DeckId::One);
    let permit = rig.gates[0].reserve().unwrap();
    rig.play(DeckId::One);

    assert_eq!(
        rig.gates[0].begin_replacement(permit),
        Err(LoadRefusal::Superseded)
    );
    rig.product.fail_library_load(None, true);
    assert!(!rig.gates[0].finish(permit));
    assert_eq!(rig.product.loaded_track(DeckId::One), original);
    assert!(rig.product.deck().deck(DeckId::One).playing);
    assert_eq!(rig.product.stats().library_load_superseded, 1);
}

#[test]
fn transport_claims_are_refused_while_a_load_replaces_the_deck() {
    let mut rig = Rig::new();
    let permit = rig.gates[0].reserve().unwrap();
    rig.gates[0].begin_replacement(permit).unwrap();
    for control in [SemanticControl::Play, SemanticControl::JogTouch] {
        rig.event(DeckId::One, control, ControlValue::Pressed(true));
    }
    assert!(rig.product.next_transport_effect().is_none());
    assert!(!rig.product.deck().deck(DeckId::One).jog_touched);
    assert_eq!(rig.product.stats().transport_refused_loading, 2);
    rig.play(DeckId::Two);

    rig.load(DeckId::One, 3, None);
    assert!(rig.gates[0].finish(permit));
    rig.play(DeckId::One);
}

// RP-03: CDJ CUE preview through the actual firmware product and transport owners.
#[test]
fn controller_disconnect_stops_cue_preview_through_the_transport_owner() {
    let mut rig = Rig::new();
    rig.event(
        DeckId::One,
        SemanticControl::Cue,
        ControlValue::Pressed(true),
    );
    let command = rig.next_command();
    assert!(matches!(
        command.effect,
        DeckEffect::PlaybackRequest {
            deck: DeckId::One,
            playing: true,
            ..
        }
    ));
    let completion = rig.transport.apply(command).unwrap();
    assert!(rig.product.apply_transport_completion(completion, 0));
    assert!(rig.product.deck().deck(DeckId::One).cue_previewing());
    rig.play(DeckId::Two);

    assert!(
        rig.product
            .try_accept(SessionDelivery::Connection(ConnectionState::Disconnected))
    );
    assert_eq!(
        rig.next_command().effect,
        DeckEffect::Pause { deck: DeckId::One }
    );
    assert_eq!(
        rig.next_command().effect,
        DeckEffect::Seek {
            deck: DeckId::One,
            position_ms: 0
        }
    );
    assert!(rig.product.next_transport_effect().is_none());
    assert!(!rig.product.deck().deck(DeckId::One).playing);
    assert!(!rig.product.deck().deck(DeckId::One).cue_held());
    assert!(rig.product.deck().deck(DeckId::Two).playing);
}

#[test]
fn cue_press_withdraws_an_admitted_load_and_cdj_touch_does_not() {
    use ruzina_controller_core::{DeckExtAction, DeckExtActionValue};
    let mut rig = Rig::new();
    let permit = rig.gates[0].reserve().unwrap();
    rig.event(
        DeckId::One,
        SemanticControl::Cue,
        ControlValue::Pressed(true),
    );
    assert_eq!(
        rig.gates[0].begin_replacement(permit),
        Err(LoadRefusal::Superseded)
    );

    rig.event(
        DeckId::Two,
        SemanticControl::DeckExtAction,
        ControlValue::DeckExtAction(DeckExtActionValue {
            action: DeckExtAction::JogCdj,
            pressed: true,
        }),
    );
    let permit = rig.gates[1].reserve().unwrap();
    rig.event(
        DeckId::Two,
        SemanticControl::JogTouch,
        ControlValue::Pressed(true),
    );
    assert!(!rig.product.deck().deck(DeckId::Two).jog_touched);
    assert_eq!(rig.gates[1].begin_replacement(permit), Ok(()));
    assert!(rig.gates[1].finish(permit));
}

// RP-05: loop-cue recall and explicit source restore through the product owner.
fn load_with_source(rig: &mut Rig, deck: DeckId, track: u8, source: HotCueBank) {
    let candidate = rig.candidate(track);
    assert!(
        rig.product
            .commit_library_load(deck, candidate, source.as_source(), None, true, None)
    );
    rig.sync();
}

#[test]
fn loop_hot_cue_recall_reaches_transport_as_one_seek_then_loop() {
    let mut rig = Rig::new();
    let mut source = HotCueBank::empty(MediaTrackId([4; 16]));
    assert!(source.set_loop(2, 1_000, 3_000));
    load_with_source(&mut rig, DeckId::One, 4, source);
    rig.cue(DeckId::One, 2);
    assert_eq!(
        rig.next_command().effect,
        DeckEffect::Seek {
            deck: DeckId::One,
            position_ms: 1_000
        }
    );
    assert_eq!(
        rig.next_command().effect,
        DeckEffect::SetLoop {
            deck: DeckId::One,
            start_ms: 1_000,
            end_ms: 3_000
        }
    );
    assert!(rig.product.next_transport_effect().is_none());
}

#[test]
fn source_restore_persists_once_and_refreshes_both_decks_on_that_track() {
    use ruzina_controller_core::{DeckExtAction, DeckExtActionValue};
    let mut rig = Rig::new();
    let track = MediaTrackId([5; 16]);
    let mut source = HotCueBank::empty(track);
    assert!(source.set_single(0, 1_500));
    load_with_source(&mut rig, DeckId::One, 5, source);
    load_with_source(&mut rig, DeckId::Two, 5, source);

    rig.event(
        DeckId::One,
        SemanticControl::PadAction,
        ControlValue::PadAction(PadAction {
            pad: 0,
            mode: PadMode::HotCue,
            shifted: true,
            pressed: true,
        }),
    );
    assert!(rig.product.complete_persistence_effect(true));
    for deck in [DeckId::One, DeckId::Two] {
        assert!(rig.product.deck().hot_cues(deck).unwrap().slot(0).is_none());
    }

    rig.event(
        DeckId::One,
        SemanticControl::DeckExtAction,
        ControlValue::DeckExtAction(DeckExtActionValue {
            action: DeckExtAction::RestoreSourceCues,
            pressed: true,
        }),
    );
    assert_eq!(
        rig.product.next_persistence_effect(),
        Some(DeckEffect::PersistHotCues {
            deck: DeckId::One,
            bank: HotCueBank::empty(track)
        })
    );
    assert!(rig.product.complete_persistence_effect(true));
    assert!(rig.product.next_persistence_effect().is_none());
    for deck in [DeckId::One, DeckId::Two] {
        assert_eq!(
            rig.product.deck().hot_cues(deck).unwrap().slot(0),
            source.slot(0)
        );
    }
}

#[test]
fn final_commit_rechecks_admission_and_preserves_a_playing_deck() {
    let mut rig = Rig::new();
    rig.play(DeckId::One);
    let original = rig.product.loaded_track(DeckId::One);
    let candidate = rig.candidate(3);
    assert!(!rig.product.commit_library_load(
        DeckId::One,
        candidate,
        HotCueBank::empty(candidate.track.track_id),
        None,
        false,
        None
    ));
    assert_eq!(rig.product.loaded_track(DeckId::One), original);
    assert!(rig.product.deck().deck(DeckId::One).playing);
}
