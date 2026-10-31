//! Host shims exercise the concrete Rust C ABI with concurrent owner/client.
//! They do not substitute for FreeRTOS SMP or actual SD DMA evidence.
use super::*;
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread;
use std::time::Duration;
use std::vec::Vec;

static LOCK: AtomicBool = AtomicBool::new(false);
static WORKER: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
static CLIENT: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
static EVENTS: Mutex<Vec<(u32, u32, u32)>> = Mutex::new(Vec::new());

#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_lock() {
    while LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        thread::yield_now();
    }
}
#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_unlock() {
    LOCK.store(false, Ordering::Release);
}
fn notify(pair: &(Mutex<bool>, Condvar)) {
    *pair.0.lock().unwrap() = true;
    pair.1.notify_one();
}
fn wait(pair: &(Mutex<bool>, Condvar)) {
    let pending = pair.0.lock().unwrap();
    let (mut pending, _) = pair
        .1
        .wait_timeout_while(pending, Duration::from_millis(100), |p| !*p)
        .unwrap();
    *pending = false;
}
#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_notify_worker() {
    notify(&WORKER);
}
#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_notify_client() {
    notify(&CLIENT);
}
#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_wait_client() {
    wait(&CLIENT);
}
#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_delay() {
    thread::yield_now();
}
#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_report(pass: u32, event: u32, value: u32) {
    EVENTS.lock().unwrap().push((pass, event, value));
}

#[cfg(feature = "idf-exfat-fixture")]
#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_fs_report(_pass: u32, _event: u32, _value: u32) {}
#[cfg(feature = "idf-exfat-fixture")]
#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_fixture_yield() {
    thread::yield_now();
}
#[cfg(feature = "idf-media-load")]
#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_load_report(_pass: u32, _event: u32, _value: u32) {}

#[repr(align(4))]
struct Bounce([u8; 512]);

#[test]
fn concrete_ffi_worker_client_and_fault_boundaries() {
    assert_eq!(ruzina_legacy_bridge_init(0), -2);
    assert_eq!(ruzina_legacy_bridge_init(100), 0);
    assert_eq!(ruzina_legacy_bridge_init(100), -1);
    let worker = thread::spawn(|| {
        let mut bounce = Bounce([0; 512]);
        for index in 0..17 {
            let (mut first, mut count) = (0, 0);
            loop {
                // SAFETY: disjoint aligned task-local outputs, one worker.
                let taken = unsafe { ruzina_legacy_bridge_take(&mut first, &mut count) };
                if taken == 1 {
                    break;
                }
                assert_eq!(taken, 0);
                wait(&WORKER);
            }
            assert_eq!(count, 1);
            assert_eq!(first, index % 8);
            assert_eq!(
                unsafe { ruzina_legacy_bridge_take(core::ptr::null_mut(), &mut count) },
                -1
            );
            assert_eq!(
                unsafe { ruzina_legacy_bridge_complete(0, core::ptr::null(), 512) },
                -1
            );
            assert_eq!(
                unsafe { ruzina_legacy_bridge_complete(0, bounce.0.as_ptr(), 511) },
                -1
            );
            assert_eq!(
                unsafe { ruzina_legacy_bridge_complete(0, bounce.0.as_ptr().add(1), 512) },
                -1
            );
            if index == 16 {
                // An invalidated in-flight request prevents hardware teardown.
                assert_eq!(ruzina_legacy_bridge_invalidate(), 0);
                assert_eq!(
                    unsafe { ruzina_legacy_bridge_complete(0x107, core::ptr::null(), 0) },
                    0
                );
                assert_eq!(ruzina_legacy_bridge_invalidate(), 1);
                break;
            }
            bounce.0.fill(first as u8);
            assert_eq!(
                unsafe { ruzina_legacy_bridge_complete(0, bounce.0.as_ptr(), 512) },
                0
            );
            // Double completion cannot consume or overwrite another request.
            assert_eq!(
                unsafe { ruzina_legacy_bridge_complete(0, bounce.0.as_ptr(), 512) },
                -1
            );
        }
    });
    assert_eq!(ruzina_legacy_bridge_run(), -2);
    worker.join().unwrap();
    let mut expected = 2166136261u32;
    for sector in 0..8u32 {
        for _ in 0..512 {
            expected = (expected ^ sector).wrapping_mul(16777619);
        }
    }
    assert_eq!(
        *EVENTS.lock().unwrap(),
        [(1, 1, expected), (2, 1, expected), (3, 2, 0)]
    );
    assert_eq!(ruzina_legacy_bridge_invalidate(), 1);
    with_state(|state| {
        assert!(state.as_ref().unwrap().active.is_none());
        assert!(state.as_ref().unwrap().queue.can_deinitialize());
    });
}
