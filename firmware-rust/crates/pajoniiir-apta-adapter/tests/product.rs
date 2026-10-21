//! The current firmware product and transport transitions, not the old APTA copy.
#![allow(clippy::result_large_err)]
// Existing Library load API includes one feature-gated analysis argument.
#[allow(dead_code, clippy::too_many_arguments)]
#[path = "../../../apps/m1-firmware/src/product_runtime.rs"]
mod product_runtime;
#[allow(dead_code)]
mod support;
#[allow(dead_code)]
#[path = "../../../apps/m1-firmware/src/transport_owner_state.rs"]
mod transport_owner_state;
mod usb0_library {
    use pajoniiir_media_session::MediaLease;
    std::thread_local! {pub static LEASE:std::cell::Cell<Option<MediaLease>>=const {std::cell::Cell::new(None)};}
    pub fn current_media_lease() -> Option<MediaLease> {
        LEASE.with(|v| v.get())
    }
}
use core::{
    future::Future,
    pin::pin,
    task::{Context, Waker},
};
use pajoniiir_apta_adapter::{
    owner::Owner,
    publication::{Outcome, PendingDelivery, Refusal},
    service::{Command, Selection, Service},
    streaming::StreamLimits,
};
use pajoniiir_controller_core::{ControlEvent, ControlValue, SemanticControl};
use pajoniiir_controller_session::{SessionDelivery, SessionDeliverySink};
use pajoniiir_core::DeckId;
use pajoniiir_deck::DeckEffect;
use pajoniiir_hot_cues::HotCueBank;
use pajoniiir_media_catalog::{CatalogLoadCandidate, CatalogPublicationId};
use pajoniiir_media_fs::analysis::Client;
use pajoniiir_media_session::MediaSession;
use pajoniiir_track_analysis::{
    AnalysisProvider,
    owned::{AnalysisBuilder, AnalysisIdentity},
};
use product_runtime::{BackgroundAnalysisRefusal, ProductAnalysisLoad, ProductRuntime};
use std::rc::Rc;
use support::*;
use transport_owner_state::{TransportCommand, TransportOwnerState};

fn until(future: &mut core::pin::Pin<&mut impl Future>, ready: impl Fn() -> bool) {
    for _ in 0..200_000 {
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        if ready() {
            return;
        }
    }
    panic!("analysis service stalled");
}
fn load(
    product: &mut ProductRuntime,
    media: &MediaSession,
    deck: DeckId,
    generation: u32,
    anlz: bool,
) -> AnalysisIdentity {
    usb0_library::LEASE.with(|v| v.set(media.lease()));
    let c = catalog(media);
    let track = *c.get(0).unwrap();
    let candidate = CatalogLoadCandidate {
        publication: CatalogPublicationId {
            lease: c.lease(),
            revision: c.revision(),
        },
        index: 0,
        track,
    };
    let identity = AnalysisIdentity {
        track: track.track_id,
        lease: c.lease(),
        catalog_revision: c.revision(),
        load_generation: generation,
    };
    let publication = anlz.then(|| {
        AnalysisBuilder::new()
            .finish(identity, AnalysisProvider::RekordboxImport, 12300)
            .unwrap()
    });
    assert!(product.commit_library_load(
        deck,
        candidate,
        HotCueBank::empty(track.track_id),
        None,
        false,
        None,
        ProductAnalysisLoad {
            generation,
            publication
        }
    ));
    identity
}
fn publish(
    product: &mut ProductRuntime,
    deck: DeckId,
    media: &MediaSession,
    idle: bool,
    value: pajoniiir_track_analysis::owned::AnalysisPublication,
) -> Result<(), Refusal> {
    product
        .publish_background_analysis(deck, value, media, idle)
        .map_err(|(why, value)| match why {
            BackgroundAnalysisRefusal::Deferred => Refusal::Deferred(value),
            BackgroundAnalysisRefusal::Rejected => Refusal::Rejected(value),
        })
}

#[test]
fn temporary_other_deck_transport_retains_delivery_until_actual_completion() {
    let media = mounted();
    let control = Rc::new(broker::Control::default());
    control.yield_receive.set(true);
    let mut fs = Fs::new(&media, wav(320000));
    fs.max = 4096;
    let client = Client::new(
        broker::Broker::new(fs, control),
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
    let identity = load(&mut product, &media, DeckId::One, 7, false);
    load(&mut product, &media, DeckId::Two, 8, false);
    let selection = Selection::new(&catalog(&media), 0)
        .unwrap()
        .with_revision(7);
    assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
    {
        let mut task = pin!(service.run(&mut port, StreamLimits::UNLIMITED, 256, 0x07e0));
        until(&mut task, || state.published.get() == 1);
        let output = state.outputs.borrow_mut().pop_front().unwrap();
        let mut pending = PendingDelivery::new(output).ok().unwrap();
        assert_eq!(pending.identity(), Some(identity));
        // Real downstream command has been accepted but not executed. Its
        // source epoch and later execution come from current TransportOwnerState.
        let mut transport = TransportOwnerState::new();
        transport.sync_source_epochs([
            product.source_epoch(DeckId::One),
            product.source_epoch(DeckId::Two),
        ]);
        let command = TransportCommand {
            source_epoch: product.source_epoch(DeckId::Two),
            effect: DeckEffect::Seek {
                deck: DeckId::Two,
                position_ms: 900,
            },
        };
        for _ in 0..3 {
            assert!(matches!(
                pending.try_publish(
                    state.authority.get(),
                    &media,
                    Some(identity),
                    |target, value| { publish(&mut product, target, &media, false, value) }
                ),
                Outcome::Deferred
            ));
            assert!(pending.is_pending());
            assert!(product.analysis_view(DeckId::One).is_none());
        }
        transport.apply(command);
        assert_eq!(transport.decks[1].position_ms, 900);
        let epoch = product.source_epoch(DeckId::One);
        let Outcome::Published(bank) = pending.try_publish(
            state.authority.get(),
            &media,
            Some(identity),
            |target, value| publish(&mut product, target, &media, true, value),
        ) else {
            panic!("settled delivery must be retried")
        };
        assert!(!pending.is_pending());
        assert_eq!(product.source_epoch(DeckId::One), epoch);
        let view = product.analysis_view(DeckId::One).unwrap();
        assert_eq!(view.provider(), AnalysisProvider::AptaNative);
        assert_eq!(view.generation(), 7);
        assert_eq!(view.beat_grid().unwrap().beats().len(), 80);
        assert_eq!(view.preview().len(), 400);
        assert!(view.detail().is_empty());
        assert!(state.send(Command::Recycle(bank)).is_ok());
        // Product owns its independent copy after bank reuse by the other deck.
        let id_two = product
            .loaded_track(DeckId::Two)
            .unwrap()
            .analysis_identity
            .unwrap();
        assert!(
            state
                .send(Command::Analyze(selection.with_revision(8), DeckId::Two))
                .is_ok()
        );
        until(&mut task, || state.published.get() == 2);
        let mut next = PendingDelivery::new(state.outputs.borrow_mut().pop_front().unwrap())
            .ok()
            .unwrap();
        assert!(matches!(
            next.try_publish(state.authority.get(), &media, Some(id_two), |target, v| {
                publish(&mut product, target, &media, true, v)
            }),
            Outcome::Published(_)
        ));
        assert_eq!(
            product
                .analysis_view(DeckId::One)
                .unwrap()
                .beat_grid()
                .unwrap()
                .beats()
                .len(),
            80
        );
    }
    drive(service.owner_mut().recover()).unwrap();
    broker::free_buffer(service.owner_mut().retire_buffer().unwrap());
}

#[test]
fn stale_load_media_authority_and_anlz_priority_reject_without_mutation() {
    for mode in 0..5 {
        let media = mounted();
        let mut product = ProductRuntime::new();
        let identity = load(&mut product, &media, DeckId::One, 1, mode == 0);
        let control = Rc::new(broker::Control::default());
        control.yield_receive.set(true);
        let mut fs = Fs::new(&media, wav(320000));
        fs.max = 4096;
        let client = Client::new(
            broker::Broker::new(fs, control),
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
        assert!(
            state
                .send(Command::Analyze(
                    Selection::new(&catalog(&media), 0)
                        .unwrap()
                        .with_revision(1),
                    DeckId::One
                ))
                .is_ok()
        );
        {
            let mut task = pin!(service.run(&mut port, StreamLimits::UNLIMITED, 256, 0));
            until(&mut task, || state.published.get() == 1);
            let mut pending = PendingDelivery::new(state.outputs.borrow_mut().pop_front().unwrap())
                .ok()
                .unwrap();
            if mode == 1 {
                load(&mut product, &media, DeckId::One, 2, false);
            }
            let mut fresh = media;
            if mode == 2 {
                fresh.on_disconnect(media.handle());
            }
            if mode == 3 {
                assert!(state.send(Command::Cancel).is_ok());
                until(&mut task, || state.commands.borrow().is_empty());
            }
            let mut expected = identity;
            if mode == 4 {
                expected.catalog_revision += 1;
            }
            assert!(matches!(
                pending.try_publish(
                    state.authority.get(),
                    &fresh,
                    Some(expected),
                    |target, v| publish(&mut product, target, &fresh, true, v)
                ),
                Outcome::Rejected(_)
            ));
            assert!(!pending.is_pending());
            if mode == 0 {
                assert_eq!(
                    product.analysis_view(DeckId::One).unwrap().provider(),
                    AnalysisProvider::RekordboxImport
                );
            } else {
                assert!(product.analysis_view(DeckId::One).is_none());
            }
        }
        drive(service.owner_mut().recover()).unwrap();
        broker::free_buffer(service.owner_mut().retire_buffer().unwrap());
    }
}

#[test]
fn queued_pending_playing_and_actual_stop_completion_gate_publication() {
    let media = mounted();
    let mut product = ProductRuntime::new();
    let identity = load(&mut product, &media, DeckId::One, 9, false);
    let mut transport = TransportOwnerState::new();
    transport.sync_source_epochs([
        product.source_epoch(DeckId::One),
        product.source_epoch(DeckId::Two),
    ]);
    let press = SessionDelivery::Control(ControlEvent {
        deck: Some(DeckId::One),
        control: SemanticControl::Play,
        value: ControlValue::Pressed(true),
    });
    assert!(product.try_accept(press));
    let publication = AnalysisBuilder::new()
        .finish(identity, AnalysisProvider::AptaNative, 12000)
        .unwrap();
    let Err((BackgroundAnalysisRefusal::Deferred, publication)) =
        product.publish_background_analysis(DeckId::One, publication, &media, true)
    else {
        panic!("queued start must defer")
    };
    let effect = product.next_transport_effect().unwrap();
    assert!(product.complete_transport_effect(true));
    assert!(!product.deck().deck(DeckId::One).playing);
    let Err((BackgroundAnalysisRefusal::Deferred, publication)) =
        product.publish_background_analysis(DeckId::One, publication, &media, true)
    else {
        panic!("enqueue alone cannot resolve pending play")
    };
    let command = TransportCommand {
        source_epoch: product.source_epoch(DeckId::One),
        effect,
    };
    let completion = transport.apply(command).unwrap();
    assert!(product.apply_transport_completion(completion, 0));
    assert!(product.deck().deck(DeckId::One).playing);
    let Err((BackgroundAnalysisRefusal::Deferred, publication)) =
        product.publish_background_analysis(DeckId::One, publication, &media, true)
    else {
        panic!("playing bank must stay pinned")
    };
    assert!(product.try_accept(press));
    let effect = product.next_transport_effect().unwrap();
    product.complete_transport_effect(true);
    let completion = transport
        .apply(TransportCommand {
            source_epoch: product.source_epoch(DeckId::One),
            effect,
        })
        .unwrap();
    assert!(product.apply_transport_completion(completion, 0));
    assert!(!product.deck().deck(DeckId::One).playing);
    assert!(
        product
            .publish_background_analysis(DeckId::One, publication, &media, true)
            .is_ok()
    );
    assert_eq!(
        product.analysis_view(DeckId::One).unwrap().bpm_x100(),
        12000
    );
}
