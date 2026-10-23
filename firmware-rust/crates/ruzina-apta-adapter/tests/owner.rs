// SPDX-License-Identifier: Apache-2.0
#[path = "support/allocation.rs"]
mod allocation;
#[allow(dead_code)]
mod support;
use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use ruzina_apta_adapter::{owner::Owner, streaming::StreamLimits, worker::Pin};
use ruzina_core::DeckId;
use ruzina_deck::DeckProductState;
use ruzina_media_fs::analysis::{Client, Error};
use std::rc::Rc;
use support::*;

#[test]
fn cancellation_at_each_broker_phase_recovers_same_owner_and_reuses_banks() {
    for phase in 0..6 {
        let media = mounted();
        let catalog = catalog(&media);
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
        let mut owner = Owner::new(client, Default::default(), work.borrow());
        owner.select(&media, &catalog, 0).unwrap();
        allocation::start();
        {
            let mut job = pin!(owner.run(&media, StreamLimits::UNLIMITED, 256, a.borrow(), 0x07e0));
            let mut reached = false;
            for _ in 0..100_000 {
                assert!(matches!(
                    job.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                    Poll::Pending
                ));
                reached = match phase {
                    0 => control.opens.get() == 1,
                    1 => control.reads.get() > 0,
                    2 => control.closes.get() == 1,
                    3 => control.opens.get() == 2,
                    4 => control.opens.get() == 2 && control.reads.get() > 313,
                    _ => control.closes.get() == 2,
                };
                if reached {
                    break;
                }
            }
            assert!(reached);
        }
        // Replacement consumes a new generation but cannot reuse pending I/O.
        owner.cancel();
        owner.select(&media, &catalog, 0).unwrap();
        control.pause.set(true);
        {
            let mut recovery = pin!(owner.recover());
            assert!(
                recovery
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
        }
        control.pause.set(false);
        let first =
            drive(owner.run(&media, StreamLimits::UNLIMITED, 256, a.borrow(), 0x07e0)).unwrap();
        let mut decks = DeckProductState::new();
        let mut selected = Pin::load(
            DeckId::One,
            &decks,
            first.completed,
            owner.requests(),
            &media,
        )
        .unwrap();
        assert_eq!(selected.track(&media).unwrap().generation(), 2);
        owner.select(&media, &catalog, 0).unwrap();
        let second =
            drive(owner.run(&media, StreamLimits::UNLIMITED, 256, b.borrow(), 0x07e0)).unwrap();
        assert_eq!(selected.track(&media).unwrap().generation(), 2);
        assert_eq!(first.waveform.columns, second.waveform.columns);
        use ruzina_controller_core::{ControlEvent, ControlValue, SemanticControl};
        let play = ControlEvent {
            deck: Some(DeckId::One),
            control: SemanticControl::Play,
            value: ControlValue::Pressed(true),
        };
        let effects = decks.handle_control_with_analysis(play, [selected.track(&media), None]);
        let Some(ruzina_deck::DeckEffect::PlaybackRequest { request_id, .. }) = effects.items[0]
        else {
            panic!("missing play")
        };
        assert!(
            selected
                .upgrade(&decks, second.completed, owner.requests(), &media)
                .is_err()
        );
        decks.resolve_playback_request(DeckId::One, request_id, true);
        assert!(
            selected
                .upgrade(&decks, second.completed, owner.requests(), &media)
                .is_err()
        );
        assert_eq!(selected.track(&media).unwrap().generation(), 2);
        let effects = decks.handle_control_with_analysis(play, [selected.track(&media), None]);
        let Some(ruzina_deck::DeckEffect::PlaybackRequest { request_id, .. }) = effects.items[0]
        else {
            panic!("missing pause")
        };
        decks.resolve_playback_request(DeckId::One, request_id, true);
        selected
            .upgrade(&decks, second.completed, owner.requests(), &media)
            .unwrap();
        assert_eq!(selected.track(&media).unwrap().generation(), 3);
        assert!(owner.is_idle());
        assert_eq!(control.opens.get(), control.closes.get());
        allocation::finish();
        broker::free_buffer(owner.retire_buffer().unwrap());
    }
}

#[test]
fn quarantine_queue_refusal_stale_media_and_errors_preserve_recovery() {
    for mode in 0..6 {
        let media = mounted();
        let catalog = catalog(&media);
        let control = Rc::new(broker::Control::default());
        let mut fs = Fs::new(&media, wav(320000));
        match mode {
            0 => control.unexpected.set(true),
            1 => control.full.set(true),
            2 => control.stale_on_receive.set(true),
            3 => fs.mode = 1,
            4 => fs.mode = 5,
            _ => fs.bytes[0] = 0,
        };
        let client = Client::new(
            broker::Broker::new(fs, control.clone()),
            media.lease().unwrap(),
            Box::leak(Box::new([0; 4096])),
        );
        let mut work = owner::Work::new();
        let mut bank = owner::Bank::new();
        let mut owner = Owner::new(client, Default::default(), work.borrow());
        owner.select(&media, &catalog, 0).unwrap();
        allocation::start();
        assert!(drive(owner.run(&media, StreamLimits::UNLIMITED, 256, bank.borrow(), 0)).is_err());
        if mode == 0 {
            assert!(matches!(drive(owner.recover()), Err(Error::Unexpected)));
            assert!(owner.take_unexpected().is_some());
        }
        control.full.set(false);
        control.stale.set(false);
        drive(owner.recover()).unwrap();
        assert!(owner.is_idle());
        assert_eq!(control.opens.get(), control.closes.get());
        if mode < 3 {
            owner.select(&media, &catalog, 0).unwrap();
            let result =
                drive(owner.run(&media, StreamLimits::UNLIMITED, 256, bank.borrow(), 0)).unwrap();
            let pin = Pin::load(
                DeckId::One,
                &DeckProductState::new(),
                result.completed,
                owner.requests(),
                &media,
            )
            .unwrap();
            assert_eq!(pin.track(&media).unwrap().generation(), 2);
        }
        allocation::finish();
        broker::free_buffer(owner.retire_buffer().unwrap());
    }
}
