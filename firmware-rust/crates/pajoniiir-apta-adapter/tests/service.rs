// SPDX-License-Identifier: Apache-2.0
#[path = "support/allocation.rs"]
mod allocation;
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
use pajoniiir_core::DeckId;
use pajoniiir_media_fs::analysis::Client;
use std::rc::Rc;
use support::*;
fn poll_until(future: &mut core::pin::Pin<&mut impl Future>, condition: impl Fn() -> bool) {
    for _ in 0..200_000 {
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        if condition() {
            return;
        }
    }
    panic!("owner loop did not reach condition");
}
#[test]
fn loop_handoff_replacement_retirement_and_bounded_backpressure() {
    let media = mounted();
    let selection = Selection::new(&catalog(&media), 0).unwrap(); // catalog can immediately retire
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
    let mut port = service::HostPort(state.clone());
    assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
    assert!(state.send(Command::Cancel).is_ok());
    assert!(matches!(
        state.send(Command::Analyze(selection, DeckId::One)),
        Err(Command::Analyze(..))
    ));
    allocation::start();
    {
        let mut task = pin!(service.run(&mut port, StreamLimits::UNLIMITED, 256, 0x07e0));
        poll_until(&mut task, || state.status.get() == Some(Status::Cancelled));
        assert_eq!(control.opens.get(), 0);
        assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
        poll_until(&mut task, || control.reads.get() > 0);
        control.pause.set(true);
        assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
        poll_until(&mut task, || state.commands.borrow().is_empty());
        // Cancel during recovery, then explicitly replace. No second open yet.
        assert!(state.send(Command::Cancel).is_ok());
        poll_until(&mut task, || state.commands.borrow().is_empty());
        assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
        poll_until(&mut task, || state.commands.borrow().is_empty());
        assert_eq!(control.opens.get(), 1);
        control.pause.set(false);
        poll_until(&mut task, || state.published.get() == 1);
        assert_eq!(control.opens.get(), control.closes.get());
        let first = state.outputs.borrow_mut().pop_front().unwrap();
        let generation = first.track(&media).unwrap().generation();
        assert_eq!(first.track(&media).unwrap().bpm_x100(), 12000);
        assert_eq!(first.waveform(&media).unwrap().columns.len(), 1250);
        assert!(state.send(Command::Analyze(selection, DeckId::Two)).is_ok());
        poll_until(&mut task, || state.published.get() == 2);
        assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
        poll_until(&mut task, || state.status.get() == Some(Status::NoBank));
        assert_eq!(first.track(&media).unwrap().generation(), generation);
        let second = state.outputs.borrow_mut().pop_front().unwrap();
        assert!(second.track(&media).unwrap().generation() > generation);
        assert_eq!(
            first.waveform(&media).unwrap().columns,
            second.waveform(&media).unwrap().columns
        );
        use pajoniiir_controller_core::{ControlEvent, ControlValue, SemanticControl};
        let play = ControlEvent {
            deck: Some(DeckId::One),
            control: SemanticControl::Play,
            value: ControlValue::Pressed(true),
        };
        let effects = state
            .decks
            .borrow_mut()
            .handle_control_with_analysis(play, [first.track(&media), second.track(&media)]);
        let Some(pajoniiir_deck::DeckEffect::PlaybackRequest { request_id, .. }) = effects.items[0]
        else {
            panic!()
        };
        let first = first
            .retire(&state.decks.borrow())
            .err()
            .expect("pending pin retained");
        state
            .decks
            .borrow_mut()
            .resolve_playback_request(DeckId::One, request_id, true);
        let first = first
            .retire(&state.decks.borrow())
            .err()
            .expect("playing pin retained");
        let effects = state
            .decks
            .borrow_mut()
            .handle_control_with_analysis(play, [first.track(&media), second.track(&media)]);
        let Some(pajoniiir_deck::DeckEffect::PlaybackRequest { request_id, .. }) = effects.items[0]
        else {
            panic!()
        };
        state
            .decks
            .borrow_mut()
            .resolve_playback_request(DeckId::One, request_id, true);
        let free = first.retire(&state.decks.borrow()).ok().unwrap();
        assert!(state.send(Command::Recycle(free)).is_ok());
        // The Analyze received while both banks were owned must resume without
        // an explicit repeat after Recycle. This was the lost-selection defect.
        poll_until(&mut task, || state.published.get() == 3);
        assert_eq!(second.track(&media).unwrap().generation(), generation + 1);
    }
    allocation::finish();
    drive(service.owner_mut().recover()).unwrap();
    broker::free_buffer(service.owner_mut().retire_buffer().unwrap());
}

#[test]
fn loop_errors_quarantine_stale_delivery_and_cancelled_loop_recovery() {
    for mode in 0..6 {
        let media = mounted();
        let selection = Selection::new(&catalog(&media), 0).unwrap();
        let control = Rc::new(broker::Control::default());
        control.yield_receive.set(true);
        let mut fs = Fs::new(&media, wav(320000));
        fs.max = 4096;
        match mode {
            0 => control.unexpected.set(true),
            1 => control.full.set(true),
            2 => control.stale_on_receive.set(true),
            3 => fs.mode = 1,
            4 => fs.mode = 5,
            _ => {}
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
        assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
        {
            let mut task = pin!(service.run(&mut port, StreamLimits::UNLIMITED, 256, 0));
            if mode == 5 {
                poll_until(&mut task, || control.reads.get() > 0);
                // Drop the entire loop while I/O is pending. Service retains it.
            } else {
                poll_until(&mut task, || {
                    matches!(
                        state.status.get(),
                        Some(Status::Failed | Status::RecoveryBlocked | Status::Quarantined)
                    )
                });
                assert_eq!(state.published.get(), 0);
            }
        }
        if mode == 0 {
            assert!(service.owner_mut().take_unexpected().is_some());
        }
        control.full.set(false);
        control.stale.set(false);
        if mode < 3 || mode == 5 {
            assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
            let mut task = pin!(service.run(&mut port, StreamLimits::UNLIMITED, 256, 0));
            poll_until(&mut task, || state.published.get() == 1);
            let mut output = state.outputs.borrow_mut().pop_front().unwrap();
            let mut stale = media;
            stale.on_disconnect(media.handle());
            assert!(
                output
                    .accept(state.authority.get(), &stale, &state.decks.borrow())
                    .is_err()
            );
            assert!(state.send(Command::Cancel).is_ok());
            poll_until(&mut task, || state.status.get() == Some(Status::Cancelled));
            assert!(
                output
                    .accept(state.authority.get(), &media, &state.decks.borrow())
                    .is_err()
            );
            assert!(output.track(&media).is_none());
        }
        drive(service.owner_mut().recover()).unwrap();
        assert_eq!(control.opens.get(), control.closes.get());
        broker::free_buffer(service.owner_mut().retire_buffer().unwrap());
    }
}

#[test]
fn loop_rechecks_live_media_and_decks_and_recovers_refused_output_bank() {
    for mode in 0..4 {
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
        let mut port = service::HostPort(state.clone());
        assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
        if mode == 1 {
            let mut stale = media;
            stale.on_disconnect(media.handle());
            state.media.set(stale);
        }
        {
            let mut task = pin!(service.run(&mut port, StreamLimits::UNLIMITED, 256, 0));
            if mode != 1 {
                poll_until(&mut task, || control.reads.get() > 0);
            }
            match mode {
                0 => {
                    let mut stale = media;
                    stale.on_disconnect(media.handle());
                    state.media.set(stale);
                }
                2 => {
                    use pajoniiir_controller_core::{ControlEvent, ControlValue, SemanticControl};
                    state.decks.borrow_mut().handle_control_with_analysis(
                        ControlEvent {
                            deck: Some(DeckId::One),
                            control: SemanticControl::Play,
                            value: ControlValue::Pressed(true),
                        },
                        [None, None],
                    );
                    assert!(
                        !state
                            .decks
                            .borrow()
                            .deck(DeckId::One)
                            .can_replace_analysis()
                    );
                }
                3 => state.refuse.set(true),
                _ => {}
            }
            poll_until(&mut task, || state.status.get() == Some(Status::Rejected));
            assert_eq!(state.published.get(), 0);
            if mode == 1 {
                assert_eq!(control.opens.get(), 0);
            }
            if mode == 3 {
                state.refuse.set(false);
                assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
                poll_until(&mut task, || state.published.get() == 1);
                assert_eq!(
                    state.outputs.borrow()[0]
                        .track(&media)
                        .unwrap()
                        .generation(),
                    2
                );
            }
        }
        drive(service.owner_mut().recover()).unwrap();
        assert_eq!(control.opens.get(), control.closes.get());
        broker::free_buffer(service.owner_mut().retire_buffer().unwrap());
    }
}
