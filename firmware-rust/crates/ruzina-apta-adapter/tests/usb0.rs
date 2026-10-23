//! The current firmware product and transport transitions, not the old APTA copy.
#![allow(clippy::result_large_err)]
// Existing Library load API includes one feature-gated analysis argument.
#[allow(dead_code, unexpected_cfgs, clippy::too_many_arguments)]
#[path = "../../../apps/m1-firmware/src/product_runtime.rs"]
mod product_runtime;
#[allow(dead_code)]
mod support;
#[allow(dead_code)]
#[path = "../../../apps/m1-firmware/src/transport_owner_state.rs"]
mod transport_owner_state;
mod usb0_library {
    use ruzina_media_session::MediaLease;
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
use product_runtime::{ProductAnalysisLoad, ProductRuntime};
use ruzina_apta_adapter::{
    owner::Owner,
    service::{Authority, Command, Retained, Selection, Service, Status},
    streaming::StreamLimits,
};
use ruzina_core::DeckId;
use ruzina_hot_cues::HotCueBank;
use ruzina_media_catalog::{CatalogLoadCandidate, CatalogPublicationId};
use ruzina_media_fs::analysis::Client;
use ruzina_media_session::MediaSession;
use ruzina_track_analysis::{
    AnalysisProvider,
    owned::{AnalysisBuilder, AnalysisIdentity},
};
use std::rc::Rc;
use support::*;

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

#[path = "../../../apps/m1-firmware/src/analysis_io.rs"]
mod analysis_io;
#[path = "../../../apps/m1-firmware/src/analysis_product.rs"]
mod analysis_product;
#[path = "../../../apps/m1-firmware/src/transport_activity.rs"]
mod transport_activity;
use analysis_product::{Driver, Report};
use std::cell::{Cell, RefCell};
struct Bridge<'a> {
    observation: Cell<ruzina_service::analysis::Sample>,
    state: service::State<'a>,
    report: Cell<Report>,
    selected: Cell<Option<Selection>>,
    allowed: Cell<bool>,
    ready: Cell<bool>,
    refuse: Cell<bool>,
    received: RefCell<Vec<&'static str>>,
}
impl<'a> Bridge<'a> {
    fn new(media: MediaSession) -> Self {
        let state = service::State::new(media);
        state.accept.set(false);
        Self {
            observation: Cell::new(ruzina_service::analysis::Sample::DISABLED),
            state,
            report: Cell::new(Report {
                serial: 0,
                selection: None,
                status: None,
                reason: ruzina_service::analysis::Reason::None,
            }),
            selected: Cell::new(None),
            allowed: Cell::new(false),
            ready: Cell::new(true),
            refuse: Cell::new(false),
            received: RefCell::new(Vec::new()),
        }
    }
    fn report(&self, status: Status) {
        let old = self.report.get();
        self.report.set(Report {
            serial: old.serial + 1,
            selection: self.selected.get(),
            status: Some(status),
            reason: ruzina_service::analysis::Reason::None,
        });
    }
}
struct ProductPort<'a>(Rc<Bridge<'a>>);
impl<'a> analysis_product::Port<'a> for ProductPort<'a> {
    fn observe(&mut self, value: ruzina_service::analysis::Sample) {
        self.0.observation.set(value);
    }
    fn ready(&self) -> bool {
        self.0.ready.get()
    }
    fn media(&self) -> MediaSession {
        self.0.state.media.get()
    }
    fn allow_io(&mut self, value: bool) {
        self.0.allowed.set(value);
    }
    fn authority(&self) -> Authority {
        self.0.state.authority.get()
    }
    fn report(&self) -> Report {
        self.0.report.get()
    }
    fn send(&mut self, command: Command<'a>) -> Result<(), Command<'a>> {
        if self.0.refuse.get() {
            Err(command)
        } else {
            self.0.state.send(command)
        }
    }
    fn output(&mut self) -> Option<Retained<'a>> {
        self.0.state.outputs.borrow_mut().pop_front()
    }
}
struct OwnerPort<'a>(Rc<Bridge<'a>>);
impl<'a> ruzina_apta_adapter::service::Port<'a> for OwnerPort<'a> {
    async fn receive(&mut self) -> Command<'a> {
        let command =
            core::future::poll_fn(|cx| match self.0.state.commands.borrow_mut().pop_front() {
                Some(c) => core::task::Poll::Ready(c),
                None => {
                    cx.waker().wake_by_ref();
                    core::task::Poll::Pending
                }
            })
            .await;
        let name = match &command {
            Command::Analyze(s, _) => {
                self.0.selected.set(Some(*s));
                "analyze"
            }
            Command::Cancel => {
                self.0.selected.set(None);
                "cancel"
            }
            Command::Recycle(_) => "recycle",
            Command::RetryRecovery => "retry",
        };
        self.0.received.borrow_mut().push(name);
        command
    }
    fn media(&self) -> MediaSession {
        self.0.state.media.get()
    }
    fn authority(&mut self, value: Authority) {
        self.0.state.authority.set(value);
    }
    fn publish(
        &mut self,
        output: Retained<'a>,
    ) -> Result<(), ruzina_apta_adapter::owner::Output<'a>> {
        if !self.0.state.outputs.borrow().is_empty() {
            return Err(output.reject().ok().unwrap());
        }
        self.0.state.outputs.borrow_mut().push_back(output);
        Ok(())
    }
    fn failure(&mut self, reason: ruzina_apta_adapter::service::Failure) {
        self.0.report(Status::Failed);
        let mut report = self.0.report.get();
        report.reason = analysis_product::failure_reason(reason);
        self.0.report.set(report);
    }
    fn report(&mut self, status: Status) {
        self.0.report(status);
    }
}
macro_rules! scenario {
    ($product:ident,$driver:ident,$port:ident,$state:ident,$control:ident,$task:ident => $body:block) => {scenario!(wav(320000),StreamLimits::UNLIMITED;$product,$driver,$port,$state,$control,$task=>$body)};
    ($bytes:expr,$limits:expr;$product:ident,$driver:ident,$port:ident,$state:ident,$control:ident,$task:ident => $body:block) => {{
        let media = mounted();
        let $control = Rc::new(broker::Control::default());
        $control.yield_receive.set(true);
        let mut fs = Fs::new(&media, $bytes);
        fs.max = 512;
        let io_buffer=Box::leak(vec![0;512].into_boxed_slice());let original_buffer=io_buffer.as_ptr();
        let client = Client::new(broker::Broker::new(fs,$control.clone()),media.lease().unwrap(),io_buffer);
        let mut work = owner::Work::new();
        let mut a = owner::Bank::new();
        let mut b = owner::Bank::new();
        let mut service = Service::new(
            Owner::new(client, Default::default(), work.borrow()),
            [a.borrow(), b.borrow()],
        );
        let $state = Rc::new(Bridge::new(media));
        let mut op = OwnerPort($state.clone());
        let mut $port = ProductPort($state.clone());
        let mut $product = ProductRuntime::new();
        let mut $driver = Driver::new();
        {
            let mut $task=pin!(service.run(&mut op,$limits,256,0x07e0));
            $body
        }
        // Shutdown the test owner explicitly; prove the broker returned the
        // original allocation after all submitted operations/handles drained.
        drive(service.owner_mut().recover()).unwrap();
        let returned=service.owner_mut().retire_buffer().unwrap();assert_eq!(returned.as_ptr(),original_buffer);assert_eq!(returned.len(),512);
        // SAFETY: the exact Box::leak above is exclusively retired by this client.
        unsafe {drop(Box::from_raw(returned as *mut [u8]));}

    }};
}
#[test]
fn missing_analysis_runs_once_serializes_decks_and_keeps_anlz() {
    scenario!(p,d,port,s,c,t=>{
        let media=s.state.media.get();load(&mut p,&media,DeckId::One,7,true);load(&mut p,&media,DeckId::Two,8,false);
        d.turn(&mut p,&mut port,true);until(&mut t,||!s.state.outputs.borrow().is_empty());d.turn(&mut p,&mut port,true);
        assert_eq!(p.analysis_view(DeckId::One).unwrap().provider(),AnalysisProvider::RekordboxImport);
        assert_eq!(p.analysis_view(DeckId::Two).unwrap().bpm_x100(),12000);
        until(&mut t,||s.received.borrow().contains(&"recycle"));
        for _ in 0..10 {d.turn(&mut p,&mut port,true);}
        assert_eq!(s.received.borrow().iter().filter(|&&x|x=="analyze").count(),1);assert_eq!(c.opens.get(),c.closes.get());
    });
}
#[test]
fn transport_defers_completed_delivery_and_full_command_lane_retains_bank() {
    scenario!(p,d,port,s,c,t=>{
        let media=s.state.media.get();load(&mut p,&media,DeckId::One,7,false);
        d.turn(&mut p,&mut port,true);until(&mut t,||!s.state.outputs.borrow().is_empty());
        d.turn(&mut p,&mut port,false);assert!(p.analysis_view(DeckId::One).is_none());assert!(!s.allowed.get());
        s.refuse.set(true);d.turn(&mut p,&mut port,true);assert!(p.analysis_view(DeckId::One).is_some());
        for _ in 0..5{d.turn(&mut p,&mut port,true);}
        assert!(!s.received.borrow().contains(&"recycle"));s.refuse.set(false);d.turn(&mut p,&mut port,true);
        until(&mut t,||s.received.borrow().contains(&"recycle"));assert_eq!(c.opens.get(),2);
    });
}
#[test]
fn active_read_is_cancelled_and_same_client_recovers_before_retry() {
    scenario!(p,d,port,s,c,t=>{
        let media=s.state.media.get();load(&mut p,&media,DeckId::One,7,false);
        d.turn(&mut p,&mut port,true);until(&mut t,||c.reads.get()>0);c.pause.set(true);
        d.turn(&mut p,&mut port,false);until(&mut t,||s.report.get().status==Some(Status::Cancelled));
        d.turn(&mut p,&mut port,false);assert!(!s.allowed.get());assert_eq!(c.opens.get(),1);
        d.turn(&mut p,&mut port,true);until(&mut t,||s.state.commands.borrow().is_empty());assert_eq!(c.opens.get(),1);
        c.pause.set(false);until(&mut t,||!s.state.outputs.borrow().is_empty());d.turn(&mut p,&mut port,true);
        assert!(p.analysis_view(DeckId::One).is_some());assert_eq!(c.opens.get(),c.closes.get());
    });
}
#[test]
fn reload_invalidates_queued_result_and_serializes_new_job() {
    scenario!(p,d,port,s,c,t=>{
        let media=s.state.media.get();load(&mut p,&media,DeckId::One,7,false);
        d.turn(&mut p,&mut port,true);until(&mut t,||!s.state.outputs.borrow().is_empty());
        load(&mut p,&media,DeckId::One,8,false);d.turn(&mut p,&mut port,true);assert!(p.analysis_view(DeckId::One).is_none());
        until(&mut t,||!s.state.outputs.borrow().is_empty());d.turn(&mut p,&mut port,true);
        assert!(p.analysis_view(DeckId::One).is_some());assert_eq!(p.loaded_track(DeckId::One).unwrap().analysis_identity.unwrap().load_generation,8);
        assert_eq!(c.opens.get(),c.closes.get());
    });
}
#[test]
fn detached_media_rejects_pending_result_without_publication() {
    scenario!(p,d,port,s,c,t=>{
        let media=s.state.media.get();load(&mut p,&media,DeckId::One,7,false);
        d.turn(&mut p,&mut port,true);until(&mut t,||!s.state.outputs.borrow().is_empty());d.turn(&mut p,&mut port,false);
        s.state.media.set(MediaSession::new());d.turn(&mut p,&mut port,true);assert!(p.analysis_view(DeckId::One).is_none());assert!(!s.allowed.get());assert_eq!(c.opens.get(),c.closes.get());
    });
}
#[test]
fn startup_refusal_and_recovery_retry_budget_are_bounded() {
    scenario!(p,d,port,s,c,t=>{
        let media=s.state.media.get();load(&mut p,&media,DeckId::One,7,false);s.ready.set(false);
        d.turn(&mut p,&mut port,true);assert!(s.state.commands.borrow().is_empty());assert!(!s.allowed.get());
        s.report(Status::RecoveryBlocked);
        for _ in 0..500 {d.turn(&mut p,&mut port,true);s.state.commands.borrow_mut().clear();}
        // Reset and count commands without running the owner.
        let mut fresh=Driver::new();let mut count=0;
        for _ in 0..500 {fresh.turn(&mut p,&mut port,true);let mut q=s.state.commands.borrow_mut();while let Some(cmd)=q.pop_front(){assert!(matches!(cmd,Command::RetryRecovery));count+=1;}}
        assert_eq!(count,3);s.report(Status::Quarantined);
        for _ in 0..100 {fresh.turn(&mut p,&mut port,true);}
        assert!(s.state.commands.borrow().is_empty());assert_eq!(c.opens.get(),0);assert!(t.as_mut().poll(&mut Context::from_waker(Waker::noop())).is_pending());
    });
}
#[test]
fn command_queue_refusal_retries_selection_without_marking_attempted() {
    scenario!(p,d,port,s,c,t=>{
        let media=s.state.media.get();load(&mut p,&media,DeckId::One,7,false);s.refuse.set(true);
        d.turn(&mut p,&mut port,true);assert!(s.state.commands.borrow().is_empty());s.refuse.set(false);
        d.turn(&mut p,&mut port,true);until(&mut t,||!s.state.outputs.borrow().is_empty());d.turn(&mut p,&mut port,true);
        assert!(p.analysis_view(DeckId::One).is_some());assert_eq!(c.opens.get(),2);
    });
}
#[test]
fn transport_count_covers_queue_full_rollback_stale_and_executing_guard() {
    let activity = transport_activity::TransportActivity::new();
    assert!(activity.idle());
    activity.reserve();
    activity.reserve();
    activity.release();
    assert!(!activity.idle());
    {
        let _running = activity.executing();
        assert!(!activity.idle());
    }
    assert!(activity.idle());
    activity.reserve();
    {
        let _stale = activity.executing();
    }
    assert!(activity.idle());
}
#[test]
fn ready_foreground_precedes_analysis_and_read_policy_caps_actual_io() {
    use core::future::{pending, ready};
    assert_eq!(run(analysis_io::foreground_first(ready(1), ready(2))), 1);
    assert_eq!(run(analysis_io::foreground_first(pending(), ready(2))), 2);
    assert_eq!(analysis_io::read_limit(false, 4096, 4096), None);
    assert_eq!(analysis_io::read_limit(true, 4096, 4096), Some(512));
    assert_eq!(analysis_io::read_limit(true, 17, 4096), Some(17));
    assert_eq!(analysis_io::read_limit(true, 4096, 23), Some(23));
}
#[test]
fn disconnected_control_health_tick_returns_even_without_usb_event() {
    use core::future::{pending, ready};
    for _ in 0..3 {
        assert_eq!(
            run(analysis_io::owner_tick(pending::<u8>(), ready(()))),
            None
        );
    }
    assert_eq!(run(analysis_io::owner_tick(ready(7), ready(()))), Some(7));
    assert_eq!(run(analysis_io::owner_tick(ready(8), pending())), Some(8));
}

#[test]
fn repeated_pending_read_cancel_and_reload_returns_banks_and_eventually_publishes() {
    scenario!(p,d,port,s,c,t=>{
        use ruzina_service::analysis::{State,Reason};
        let media=s.state.media.get();
        for generation in 1..=64 {
            load(&mut p,&media,DeckId::One,generation,false);
            let reads=c.reads.get();d.turn(&mut p,&mut port,true);until(&mut t,||c.reads.get()>reads);
            d.turn(&mut p,&mut port,true);assert_eq!(s.observation.get().state,State::Analyzing);
            c.pause.set(true);d.turn(&mut p,&mut port,false);
            assert_eq!(s.observation.get().reason,Reason::PlaybackOrTransport);
            until(&mut t,||s.report.get().status==Some(Status::Cancelled));
            d.turn(&mut p,&mut port,false);c.pause.set(false);
            until(&mut t,||c.opens.get()==c.closes.get());
            assert!(p.analysis_view(DeckId::One).is_none());
            assert!(s.state.outputs.borrow().is_empty());assert!(s.state.commands.borrow().is_empty());
        }
        load(&mut p,&media,DeckId::One,65,false);d.turn(&mut p,&mut port,true);
        until(&mut t,||!s.state.outputs.borrow().is_empty());
        // Saturate recycle during acceptance, then drain on the SAME service.
        s.refuse.set(true);d.turn(&mut p,&mut port,true);assert!(p.analysis_view(DeckId::One).is_some());
        for _ in 0..64 {d.turn(&mut p,&mut port,true);}
        assert_eq!(s.observation.get().reason,Reason::Recycle);s.refuse.set(false);d.turn(&mut p,&mut port,true);
        until(&mut t,||s.received.borrow().contains(&"recycle"));d.turn(&mut p,&mut port,true);
        assert_eq!(s.observation.get().state,State::Published);assert_eq!(c.opens.get(),c.closes.get());
        if let Ok(path)=std::env::var("M1_APTA_STRESS_RECORD") {
            use std::io::Write;
            let mut monitor=ruzina_service::analysis::Monitor::new();monitor.observe(s.observation.get(),0);
            let mut record=String::new();monitor.snapshot().write_to(&mut record).unwrap();
            let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();file.write_all(record.as_bytes()).unwrap();
        }
        println!("stress cancel/reload: 64 cycles; fixed owner/work/banks; opens={} closes={}; final publication and recycle PASS",c.opens.get(),c.closes.get());
    });
}
#[test]
fn repeated_detach_during_io_never_publishes_and_drains_each_client() {
    for _ in 0..32 {
        scenario!(p,d,port,s,c,t=>{
            let media=s.state.media.get();load(&mut p,&media,DeckId::One,7,false);
            d.turn(&mut p,&mut port,true);until(&mut t,||c.reads.get()>0);c.pause.set(true);
            let mut detached=media;detached.on_disconnect(media.handle());s.state.media.set(detached);
            d.turn(&mut p,&mut port,true);until(&mut t,||s.report.get().status==Some(Status::Cancelled));
            c.pause.set(false);until(&mut t,||c.opens.get()==c.closes.get());d.turn(&mut p,&mut port,true);
            assert!(p.analysis_view(DeckId::One).is_none());assert!(s.state.outputs.borrow().is_empty());assert!(!s.allowed.get());
        });
    }
    println!(
        "stress detach: 32 independent media/client lifetimes; no stale publication; all handles closed PASS"
    );
}
#[test]
fn invalid_format_work_limit_and_capacity_have_real_visible_reasons() {
    use ruzina_service::analysis::{Reason, State};
    for (bytes, limits, expected) in [
        (vec![0; 88], StreamLimits::UNLIMITED, Reason::Format),
        (
            wav(320000),
            StreamLimits {
                object_bytes: 128,
                ..StreamLimits::UNLIMITED
            },
            Reason::WorkLimit,
        ),
        (wav(320001), StreamLimits::UNLIMITED, Reason::Capacity),
    ] {
        scenario!(bytes,limits;p,d,port,s,c,t=>{
            let media=s.state.media.get();load(&mut p,&media,DeckId::One,7,false);d.turn(&mut p,&mut port,true);
            until(&mut t,||s.report.get().status==Some(Status::Failed));d.turn(&mut p,&mut port,true);
            assert_eq!(s.observation.get().state,State::Failed);assert_eq!(s.observation.get().reason,expected);
            assert!(p.analysis_view(DeckId::One).is_none());assert_eq!(c.opens.get(),c.closes.get());
            for _ in 0..100{d.turn(&mut p,&mut port,true);}
            assert!(s.state.commands.borrow().is_empty());
        });
    }
}
#[test]
fn two_missing_decks_are_analyzed_in_order_with_banks_recycled() {
    scenario!(p,d,port,s,c,t=>{
        let media=s.state.media.get();load(&mut p,&media,DeckId::One,7,false);load(&mut p,&media,DeckId::Two,8,false);
        d.turn(&mut p,&mut port,true);assert_eq!(s.state.commands.borrow().len(),1);
        until(&mut t,||!s.state.outputs.borrow().is_empty());d.turn(&mut p,&mut port,true);
        assert!(p.analysis_view(DeckId::One).is_some());assert!(p.analysis_view(DeckId::Two).is_none());
        until(&mut t,||!s.state.outputs.borrow().is_empty());d.turn(&mut p,&mut port,true);
        assert!(p.analysis_view(DeckId::Two).is_some());assert_eq!(c.opens.get(),4);assert_eq!(c.closes.get(),4);
    });
}
#[test]
fn queued_start_on_other_deck_keeps_completed_delivery_pending() {
    use ruzina_controller_core::{ControlEvent, ControlValue, SemanticControl};
    use ruzina_controller_session::{SessionDelivery, SessionDeliverySink};
    use transport_owner_state::{TransportCommand, TransportOwnerState};
    scenario!(p,d,port,s,c,t=>{
        let media=s.state.media.get();load(&mut p,&media,DeckId::One,7,false);load(&mut p,&media,DeckId::Two,8,true);
        d.turn(&mut p,&mut port,true);until(&mut t,||!s.state.outputs.borrow().is_empty());
        let press=SessionDelivery::Control(ControlEvent {deck:Some(DeckId::Two),control:SemanticControl::Play,value:ControlValue::Pressed(true)});
        assert!(p.try_accept(press));d.turn(&mut p,&mut port,true);assert!(p.analysis_view(DeckId::One).is_none());assert!(!s.allowed.get());
        let mut owner=TransportOwnerState::new();owner.sync_source_epochs([p.source_epoch(DeckId::One),p.source_epoch(DeckId::Two)]);
        let effect=p.next_transport_effect().unwrap();assert!(p.complete_transport_effect(true));
        d.turn(&mut p,&mut port,true);assert!(p.analysis_view(DeckId::One).is_none());
        let completion=owner.apply(TransportCommand {source_epoch:p.source_epoch(DeckId::Two),effect}).unwrap();assert!(p.apply_transport_completion(completion,0));
        d.turn(&mut p,&mut port,true);assert!(p.analysis_view(DeckId::One).is_none());assert!(p.deck().deck(DeckId::Two).playing);
        assert!(p.try_accept(press));let effect=p.next_transport_effect().unwrap();assert!(p.complete_transport_effect(true));
        let completion=owner.apply(TransportCommand {source_epoch:p.source_epoch(DeckId::Two),effect}).unwrap();assert!(p.apply_transport_completion(completion,0));
        d.turn(&mut p,&mut port,true);assert!(p.analysis_view(DeckId::One).is_some());assert_eq!(c.opens.get(),2);
    });
}
