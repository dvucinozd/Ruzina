//! Existing owners publish through short critical sections. The observer never
//! samples a stopped owner, runs controls, restarts hardware or writes flash.
use crate::{product_runtime::ProductRuntime, usb0_msc::Usb0MscBinding};
use core::cell::RefCell;
use embassy_sync::blocking_mutex::CriticalSectionMutex;
use ruzina_controller_session::ControllerSession;
use ruzina_service::{
    handoff::Producer,
    owners::{ControllerObservation, MediaObservation, OWNER_TICK_MS, RuntimeOwners, RuntimeView},
};

struct State {
    owners: RuntimeOwners,
    product: Option<Producer>,
    controller: Option<Producer>,
}
static STATE: CriticalSectionMutex<RefCell<State>> =
    CriticalSectionMutex::new(RefCell::new(State {
        owners: RuntimeOwners::new(),
        product: None,
        controller: None,
    }));
pub(crate) fn now_ms() -> u64 {
    embassy_time::Instant::now().as_millis()
}

pub(crate) fn restart_control() {
    STATE.lock(|cell| {
        let mut s = cell.borrow_mut();
        s.product = s.owners.product.restart();
        s.controller = s.owners.controller.restart();
    });
}
pub(crate) fn publish_control(
    product: &ProductRuntime,
    controller: &ControllerSession,
    backpressured: bool,
) {
    let product = product.health_observation(backpressured);
    let mut controller = ControllerObservation::capture(controller, backpressured);
    controller.failed = matches!(
        crate::usb1_controller::controller_state(),
        crate::usb1_controller::Usb1ControllerState::Failed(_)
    );
    let now = now_ms();
    STATE.lock(|cell| {
        let mut s = cell.borrow_mut();
        if let Some(sample) = s.product.as_mut().and_then(|p| p.sample(now, product)) {
            let _ = s.owners.product.try_send(sample);
        }
        if let Some(sample) = s
            .controller
            .as_mut()
            .and_then(|p| p.sample(now, controller))
        {
            let _ = s.owners.controller.try_send(sample);
        }
    });
}

/// Root transport lifecycle invalidates the old media publication immediately,
/// even when the media worker is suspended in an older discovery operation.
pub(crate) fn invalidate_media(binding: Usb0MscBinding, detached: bool) {
    STATE.lock(|cell| {
        let mut s = cell.borrow_mut();
        s.owners
            .media
            .invalidate(binding.lease, binding.handle, detached);
    });
}
pub(crate) fn initial_media() -> Option<Producer> {
    STATE.lock(|cell| cell.borrow_mut().owners.media.initial())
}
/// Tokens travel to the worker only for the matching transport lifecycle event.
pub(crate) fn claim_media(binding: Usb0MscBinding, detached: bool) -> Option<Producer> {
    STATE.lock(|cell| {
        let mut s = cell.borrow_mut();
        s.owners
            .media
            .claim(binding.lease, binding.handle, detached)
    })
}
pub(crate) fn publish_media(producer: &mut Producer, value: MediaObservation) {
    if let Some(sample) = producer.sample(now_ms(), value) {
        STATE.lock(|cell| {
            let _ = cell.borrow_mut().owners.media.channel.try_send(sample);
        });
    }
}
/// Consumers see current age even if the observer task has stopped draining.
#[allow(dead_code)]
pub(crate) fn snapshot() -> RuntimeView {
    #[allow(unused_mut)]
    let mut view = STATE.lock(|cell| cell.borrow().owners.view(now_ms()));
    #[cfg(feature = "apta-usb0-bringup")]
    {
        view.analysis = crate::usb0_analysis::diagnostic_snapshot();
    }
    view
}
#[embassy_executor::task]
pub(crate) async fn observer_task() {
    loop {
        STATE.lock(|cell| {
            cell.borrow_mut().owners.observe(now_ms());
        });
        embassy_time::Timer::after_millis(OWNER_TICK_MS).await;
    }
}
