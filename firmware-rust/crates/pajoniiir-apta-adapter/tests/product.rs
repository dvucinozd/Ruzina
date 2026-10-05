// Bounded messages return ownership without allocating on refusal.
#![allow(clippy::result_large_err)]
// Execute the exact firmware owner, including control staging and effect acks.
#[path = "support/allocation.rs"]
mod allocation;
#[allow(dead_code)]
#[path = "../../../apps/m1-firmware/src/product_runtime.rs"]
mod product_runtime;
#[allow(dead_code)]
mod support;
use core::{
    future::Future,
    pin::pin,
    task::{Context, Waker},
};
use pajoniiir_apta_adapter::{
    owner::Owner,
    service::{Command, Selection, Service, Status},
    streaming::StreamLimits,
};
use pajoniiir_controller_core::{ControlEvent, ControlValue, SemanticControl};
use pajoniiir_controller_session::{SessionDelivery, SessionDeliverySink};
use pajoniiir_core::DeckId;
use pajoniiir_deck::DeckEffect;
use pajoniiir_media_fs::analysis::Client;
use product_runtime::ProductRuntime;
use std::rc::Rc;
use support::*;
fn until(f: &mut core::pin::Pin<&mut impl Future>, ready: impl Fn() -> bool) {
    for _ in 0..200_000 {
        assert!(
            f.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        if ready() {
            return;
        }
    }
    panic!("product loop stalled");
}
fn press(product: &mut ProductRuntime<'_>, deck: DeckId, control: SemanticControl) {
    assert!(product.try_accept(SessionDelivery::Control(ControlEvent {
        deck: Some(deck),
        control,
        value: ControlValue::Pressed(true),
    })));
}
fn acknowledge_completed(product: &mut ProductRuntime<'_>) {
    let ticket = product.next_transport_ticket().unwrap();
    assert!(product.complete_transport_effect(true));
    assert!(product.inflight_transport_effect().is_some());
    // Queue acceptance alone cannot release a bank, even when logical state stopped.
    assert!(!product.retire_analysis(DeckId::One));
    assert!(!product.finish_transport_effect(ticket.wrapping_add(1), true));
    assert!(!product.finish_transport_effect(ticket.saturating_sub(1), true));
    assert!(!product.finish_transport_effect(ticket, false));
    assert!(product.finish_transport_effect(ticket, true));
    assert!(!product.finish_transport_effect(ticket, true));
}
#[test]
fn actual_product_lifecycle_with_service_and_transport_acknowledgement() {
    let media = mounted();
    let selection = Selection::new(&catalog(&media), 0).unwrap();
    let control = Rc::new(broker::Control::default());
    control.yield_receive.set(true);
    let mut fs = Fs::new(&media, wav(320000));
    fs.max = 4096;
    let client = Client::new(
        broker::Broker::new(fs, control.clone()),
        media.lease().unwrap(),
        Box::leak(Box::new([0; 4096])),
    );
    let mut work = owner::Work::new();
    let mut a = owner::Bank::new();
    let mut b = owner::Bank::new();
    let mut service = Service::new(
        Owner::new(client, Default::default(), work.borrow()),
        [a.borrow(), b.borrow()],
    );
    let state = Rc::new(service::State::new(media));
    state.accept.set(false);
    let mut port = service::HostPort(state.clone());
    let mut product = ProductRuntime::new();
    allocation::start();
    {
        let mut task = pin!(service.run(&mut port, StreamLimits::UNLIMITED, 256, 0x07e0));
        assert!(product.select_analysis(selection, DeckId::One, &media, |c| state.send(c)));
        until(&mut task, || state.published.get() == 1);
        // Same-track retry BEFORE the loop receives it must reject old output.
        assert!(product.select_analysis(selection, DeckId::One, &media, |c| state.send(c)));
        let old = state.outputs.borrow_mut().pop_front().unwrap();
        let old = product
            .accept_analysis(old, state.authority.get(), &media)
            .err()
            .unwrap();
        assert!(product.reject_analysis(old).is_ok());
        assert!(product.flush_analysis_recycle(|c| state.send(c)));
        until(&mut task, || state.published.get() == 2);
        assert!(
            product
                .accept_analysis(
                    state.outputs.borrow_mut().pop_front().unwrap(),
                    state.authority.get(),
                    &media
                )
                .is_ok()
        );
        let generation = product
            .analysis_track(DeckId::One, &media)
            .unwrap()
            .generation();
        assert_eq!(
            product
                .analysis_waveform(DeckId::One, &media)
                .unwrap()
                .columns
                .len(),
            1250
        );
        assert!(product.select_analysis(selection, DeckId::Two, &media, |c| state.send(c)));
        until(&mut task, || state.published.get() == 3);
        assert!(
            product
                .accept_analysis(
                    state.outputs.borrow_mut().pop_front().unwrap(),
                    state.authority.get(),
                    &media
                )
                .is_ok()
        );
        assert_eq!(
            product
                .analysis_track(DeckId::One, &media)
                .unwrap()
                .generation(),
            generation
        );
        assert!(
            product
                .analysis_track(DeckId::Two, &media)
                .unwrap()
                .generation()
                > generation
        );
        assert_eq!(
            product
                .analysis_track(DeckId::Two, &media)
                .unwrap()
                .beat_grid()
                .unwrap()
                .beats()
                .len(),
            80
        );
        // Both banks remain owned: service rejects new work without eviction.
        assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
        until(&mut task, || state.status.get() == Some(Status::NoBank));
        product.analysis_media(media, state.status.get());
        assert_eq!(product.analysis_status(), Some(Status::NoBank));
        assert_eq!(
            product
                .analysis_track(DeckId::One, &media)
                .unwrap()
                .generation(),
            generation
        );
        // Sync emits pitch plus grid phase alignment through the actual owner.
        press(&mut product, DeckId::Two, SemanticControl::Sync);
        assert!(matches!(
            product.next_transport_effect(),
            Some(DeckEffect::SetPitchCentipercent { .. })
        ));
        acknowledge_completed(&mut product);
        assert!(matches!(
            product.next_transport_effect(),
            Some(DeckEffect::Seek { .. })
        ));
        acknowledge_completed(&mut product);
        // Actual control path consumes retained neutral grid; one beat is 500 ms.
        let track = product.analysis_track(DeckId::One, &media).unwrap();
        let target = pajoniiir_track_analysis::beat_jump_target_ms(
            0,
            track.bpm_x100(),
            1,
            1,
            track.beat_grid(),
        );
        assert_ne!(target, 500); // grid alignment differs from BPM-only fallback
        press(&mut product, DeckId::One, SemanticControl::BeatJumpForward);
        assert!(
            matches!(product.next_transport_effect(),Some(DeckEffect::Seek { position_ms,.. }) if position_ms == target)
        );
        let effect = product.next_transport_effect();
        assert!(!product.complete_transport_effect(false));
        assert_eq!(product.next_transport_effect(), effect);
        assert!(!product.retire_analysis(DeckId::One));
        acknowledge_completed(&mut product);
        press(&mut product, DeckId::One, SemanticControl::Play);
        assert!(!product.retire_analysis(DeckId::One));
        assert!(!product.complete_transport_effect(false));
        assert!(!product.deck().deck(DeckId::One).playing);
        acknowledge_completed(&mut product);
        assert!(product.deck().deck(DeckId::One).playing);
        assert!(!product.retire_analysis(DeckId::One));
        // Media invalidation hides output, but doesn't free the playing bank.
        let mut stale = media;
        stale.on_disconnect(pajoniiir_media_session::MediaHandle::new(1));
        product.analysis_media(stale, Some(Status::RecoveryBlocked));
        assert_eq!(product.analysis_status(), Some(Status::RecoveryBlocked));
        assert!(product.analysis_track(DeckId::One, &stale).is_none());
        assert!(!product.retire_analysis(DeckId::One));
        press(&mut product, DeckId::One, SemanticControl::Cue);
        assert!(!product.deck().deck(DeckId::One).playing);
        assert!(!product.retire_analysis(DeckId::One));
        assert!(!product.complete_transport_effect(false));
        assert!(!product.retire_analysis(DeckId::One));
        while product.next_transport_effect().is_some() {
            acknowledge_completed(&mut product);
        }
        assert!(product.retire_analysis(DeckId::One));
        // Queue full: the product keeps the exact free bank for a later turn.
        assert!(state.send(Command::RetryRecovery).is_ok());
        assert!(state.send(Command::RetryRecovery).is_ok());
        assert!(!product.flush_analysis_recycle(|c| state.send(c)));
        until(&mut task, || state.commands.borrow().is_empty());
        assert!(product.flush_analysis_recycle(|c| state.send(c)));
        assert!(product.retire_analysis(DeckId::Two));
        assert!(product.flush_analysis_recycle(|c| state.send(c)));
        until(&mut task, || state.commands.borrow().is_empty());
        assert!(product.cancel_analysis(|c| state.send(c)));
        until(&mut task, || state.status.get() == Some(Status::Cancelled));
    }
    allocation::finish();
    drive(service.owner_mut().recover()).unwrap();
    broker::free_buffer(service.owner_mut().retire_buffer().unwrap());
}

#[test]
fn product_selection_recovery_and_fresh_media_refusal() {
    for mode in 0..6 {
        let media = mounted();
        let selection = Selection::new(&catalog(&media), 0).unwrap();
        let mut changed = *catalog(&media).get(0).unwrap();
        changed.track_id = pajoniiir_core::MediaTrackId([9; 16]);
        let mut catalog_b =
            pajoniiir_media_catalog::CatalogBuilder::<1>::new(media.lease().unwrap());
        catalog_b.push(changed).unwrap();
        let different = Selection::new(&catalog_b.finish(1), 0).unwrap();
        let control = Rc::new(broker::Control::default());
        control.yield_receive.set(true);
        let mut fs = Fs::new(&media, wav(320000));
        fs.max = 4096;
        if mode == 0 {
            control.unexpected.set(true);
        }
        if mode == 1 {
            fs.mode = 1;
        }
        if mode == 2 {
            fs.mode = 5;
        }
        let client = Client::new(
            broker::Broker::new(fs, control.clone()),
            media.lease().unwrap(),
            Box::leak(Box::new([0; 4096])),
        );
        let mut work = owner::Work::new();
        let mut a = owner::Bank::new();
        let mut b = owner::Bank::new();
        let mut service = Service::new(
            Owner::new(client, Default::default(), work.borrow()),
            [a.borrow(), b.borrow()],
        );
        let state = Rc::new(service::State::new(media));
        state.accept.set(false);
        let mut port = service::HostPort(state.clone());
        let mut product = ProductRuntime::new();
        assert!(product.queue_catalog_selection(selection, DeckId::One));
        assert!(!product.queue_catalog_selection(different, DeckId::Two));
        // Bounded enqueue refusal retains intent without changing Deck identity.
        assert!(state.send(Command::RetryRecovery).is_ok());
        assert!(state.send(Command::RetryRecovery).is_ok());
        product.advance_analysis(&media, |c| state.send(c));
        assert!(!product.needs_catalog_selection());
        assert!(product.deck().hot_cues(DeckId::One).is_none());
        {
            let mut task = pin!(service.run(&mut port, StreamLimits::UNLIMITED, 256, 0x07e0));
            until(&mut task, || state.commands.borrow().is_empty());
            product.advance_analysis(&media, |c| state.send(c));
            assert!(product.needs_catalog_selection());
            if mode < 3 {
                until(&mut task, || {
                    matches!(
                        state.status.get(),
                        Some(Status::Failed | Status::RecoveryBlocked | Status::Quarantined)
                    )
                });
                if mode == 0 {
                    until(&mut task, || {
                        state.status.get() == Some(Status::Quarantined)
                    });
                }
                product.analysis_media(media, state.status.get());
                assert!(matches!(
                    product.analysis_status(),
                    Some(Status::Failed | Status::RecoveryBlocked | Status::Quarantined)
                ));
                assert!(product.analysis_track(DeckId::One, &media).is_none());
                assert_eq!(state.published.get(), 0);
            } else {
                until(&mut task, || control.reads.get() > 0);
                control.pause.set(true);
                assert!(product.select_analysis(different, DeckId::Two, &media, |c| state.send(c)));
                until(&mut task, || state.commands.borrow().is_empty());
                assert!(product.cancel_analysis(|c| state.send(c)));
                until(&mut task, || state.status.get() == Some(Status::Cancelled));
                assert!(product.select_analysis(different, DeckId::Two, &media, |c| state.send(c)));
                until(&mut task, || state.commands.borrow().is_empty());
                assert_eq!(control.opens.get(), 1);
                control.pause.set(false);
                until(&mut task, || state.published.get() == 1);
                assert_eq!(
                    product.deck().hot_cues(DeckId::Two).unwrap().track_id(),
                    different.track_id()
                );
                let mut stale = media;
                match mode {
                    3 => stale.mark_unmounted(),
                    4 => {
                        stale.on_disconnect(media.handle());
                    }
                    _ => {
                        stale.on_disconnect(media.handle());
                        stale.on_connect(pajoniiir_media_session::MediaSourceId::new(2).unwrap());
                        let lease = stale.lease().unwrap();
                        stale.bind_handle(
                            lease,
                            pajoniiir_media_session::MediaHandle::new(2).unwrap(),
                        );
                        stale.commit_mounted(lease);
                    }
                }
                let bank = state.outputs.borrow_mut().pop_front().unwrap();
                let bank = product
                    .accept_analysis(bank, state.authority.get(), &stale)
                    .err()
                    .unwrap();
                assert!(product.reject_analysis(bank).is_ok());
                assert!(product.flush_analysis_recycle(|c| state.send(c)));
                until(&mut task, || state.commands.borrow().is_empty());
                assert!(product.analysis_waveform(DeckId::Two, &stale).is_none());
            }
        }
        if mode == 0 {
            // The actual Service still owns quarantine and its pending completion.
            assert!(!service.owner_mut().is_idle());
            assert!(service.owner_mut().take_unexpected().is_some());
        }
        drive(service.owner_mut().recover()).unwrap();
        assert_eq!(control.opens.get(), control.closes.get());
        broker::free_buffer(service.owner_mut().retire_buffer().unwrap());
    }
}
