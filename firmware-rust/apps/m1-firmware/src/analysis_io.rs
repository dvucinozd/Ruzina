//! Broker policy, shared by the concrete USB0 worker and host checks.
use core::future::Future;
use embassy_futures::select::{Either, select};
/// The periodic control-health tick must be INSIDE the supervised future, so
/// an absent controller returns to its owner and republishes fresh health.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn owner_tick<F: Future>(
    event: F,
    tick: impl Future<Output = ()>,
) -> Option<F::Output> {
    match select(event, tick).await {
        Either::First(event) => Some(event),
        Either::Second(()) => None,
    }
}
pub(crate) async fn foreground_first<T>(
    foreground: impl Future<Output = T>,
    analysis: impl Future<Output = T>,
) -> T {
    match select(foreground, analysis).await {
        Either::First(value) | Either::Second(value) => value,
    }
}
pub(crate) fn read_limit(allowed: bool, requested: usize, capacity: usize) -> Option<usize> {
    allowed.then_some(requested.min(capacity).min(512))
}
