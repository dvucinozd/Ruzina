//! A new password for the deck's own network, on its way from the Web Remote's SETUP page
//! (or, for the default, the deck's Settings page) to the app main task. That task saves it
//! and restarts the network with it once no deck plays (`ruzina_legacy_settings_service`,
//! `main/wifi_remote.c`). One waits at a time; the password is wiped once taken.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU8, Ordering};

const EMPTY: u8 = 0;
const WRITING: u8 = 1;
const READY: u8 = 2;
const READING: u8 = 3;

/// The longest password WPA2 takes: 64 hex digits.
pub(crate) const PASSWORD_MAX: usize = 64;

struct Pending {
    state: AtomicU8,
    len: UnsafeCell<usize>,
    bytes: UnsafeCell<[u8; PASSWORD_MAX]>,
}

// SAFETY: `state` hands the buffer to one side at a time: a writer between WRITING and
// READY, a reader between READING and EMPTY.
unsafe impl Sync for Pending {}

static PENDING: Pending = Pending {
    state: AtomicU8::new(EMPTY),
    len: UnsafeCell::new(0),
    bytes: UnsafeCell::new([0; PASSWORD_MAX]),
};

/// Whether WPA2 takes `password`: 8 to 63 printable ASCII characters, or 64 hex digits.
pub(crate) fn valid(password: &str) -> bool {
    let bytes = password.as_bytes();
    if bytes.len() == PASSWORD_MAX {
        return bytes.iter().all(u8::is_ascii_hexdigit);
    }
    (8..=63).contains(&bytes.len()) && bytes.iter().all(|&byte| (0x20..0x7f).contains(&byte))
}

/// Hands over a new password, or `None` for the default; false while another waits or for
/// a password WPA2 does not take.
pub(crate) fn put(password: Option<&str>) -> bool {
    if password.is_some_and(|password| !valid(password)) {
        return false;
    }
    if PENDING
        .state
        .compare_exchange(EMPTY, WRITING, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    let bytes = password.map_or(&[][..], str::as_bytes);
    // SAFETY: WRITING gives this side the buffer alone.
    unsafe {
        (&mut *PENDING.bytes.get())[..bytes.len()].copy_from_slice(bytes);
        *PENDING.len.get() = bytes.len();
    }
    PENDING.state.store(READY, Ordering::Release);
    true
}

/// The password waiting, NUL-terminated for C, with a first byte of 0 for the default; once.
pub(crate) fn take() -> Option<[u8; PASSWORD_MAX + 1]> {
    if PENDING
        .state
        .compare_exchange(READY, READING, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return None;
    }
    let mut out = [0u8; PASSWORD_MAX + 1];
    // SAFETY: READING gives this side the buffer alone.
    unsafe {
        let len = *PENDING.len.get();
        let bytes = &mut *PENDING.bytes.get();
        out[..len].copy_from_slice(&bytes[..len]);
        bytes.fill(0);
        *PENDING.len.get() = 0;
    }
    PENDING.state.store(EMPTY, Ordering::Release);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_wpa2_password_is_handed_over_and_one_waits_at_a_time() {
        assert!(!valid("short"));
        assert!(!valid("tab\there!"));
        assert!(!valid("lozinka čaša"));
        assert!(valid("club night 42"));
        assert!(valid(&"ab".repeat(32)));
        assert!(!valid(&"zz".repeat(32)));
        assert!(!put(Some("short")));
        assert!(take().is_none());
        assert!(put(Some("club night 42")));
        assert!(!put(None));
        let taken = take().unwrap();
        assert_eq!(&taken[..14], b"club night 42\0");
        assert!(take().is_none());
        // The default: an empty, NUL-only password.
        assert!(put(None));
        assert_eq!(take().unwrap()[0], 0);
    }
}
