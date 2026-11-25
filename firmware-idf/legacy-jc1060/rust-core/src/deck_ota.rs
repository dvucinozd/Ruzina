//! Firmware updates from the Web Remote: a signed `.ddjota` bundle, streamed by the web
//! task into the update slot through `ruzina-ota`'s [`Push`].
//!
//! Nothing reaches flash before the bundle's 188-byte header is checked against
//! [`EXPECTED`] and its signature against [`PUBLIC_KEY`], the owner's Ruzina key
//! (`ruz-001`; its private half stays outside the repository, see
//! `tools/ota_bundle.py`). The image is then written to the slot the deck is not running
//! (`main/ota_slot.c`), and the boot slot switches only once the image's hash, IDF's check
//! of the written image and its app description all match. The web task then restarts the
//! deck into it ([`restart_due`]); `ota_slot.c` keeps it once the Web Remote's server
//! listens again, else the bootloader goes back to the previous image.
//!
//! The slot is written while USB audio runs, which stalls its fills, so an update starts
//! only with both decks stopped (as a change of the network's password does).
use core::sync::atomic::{AtomicBool, Ordering};
use ruzina_ota::bundle::{ESP32P4_CHIP_ID, Expected, SLOT_IMAGE_SIZE_MAX, TARGET_P4};
use ruzina_ota::install::{Playback, SlotFlash};
pub(crate) use ruzina_ota::push::{Push, PushError};

/// The id of the key [`PUBLIC_KEY`] belongs to, which a bundle's header names.
pub(crate) const KEY_ID: &str = "ruz-001";

/// The owner's Ruzina signing key (P-256, uncompressed SEC1), made on 2026-11-25 with
/// `tools/ota_bundle.py generate`. Its SHA-256 is `bd94eba9…a76a72`.
pub(crate) const PUBLIC_KEY: [u8; 65] = [
    0x04, 0x71, 0xa0, 0xf0, 0x7e, 0xc5, 0x4d, 0xe9, 0xbe, 0x40, 0x77, 0xb8, 0xff, 0x3c, 0x51, 0x41,
    0xe2, 0x36, 0x8c, 0x2a, 0x42, 0xe9, 0x40, 0xf6, 0x63, 0x1c, 0x77, 0xd3, 0x11, 0xe9, 0xda, 0x6f,
    0xda, 0xa9, 0x7d, 0xa2, 0x76, 0xba, 0xdf, 0x51, 0x5e, 0x14, 0xda, 0x26, 0x97, 0xd3, 0x0b, 0x44,
    0x63, 0x84, 0x9c, 0x93, 0x7b, 0x0b, 0xfc, 0xb2, 0x0d, 0xbe, 0xf4, 0x41, 0xa4, 0xeb, 0xaa, 0xcc,
    0x7c,
];

/// What the deck installs: an ESP32-P4 image of this project, no larger than a slot,
/// signed with the Ruzina key.
pub(crate) const EXPECTED: Expected<'static> = Expected {
    target: TARGET_P4,
    chip_id: ESP32P4_CHIP_ID,
    project: "ruzina_legacy_probe",
    max_image_size: SLOT_IMAGE_SIZE_MAX,
    key_id: KEY_ID,
};

/// Set once a new image is in its slot and set to boot: the web task restarts after the
/// answer has gone.
static RESTART: AtomicBool = AtomicBool::new(false);

pub(crate) fn restart_due() -> bool {
    RESTART.load(Ordering::Acquire)
}

/// One upload: the bundle's bytes so far, through `ruzina-ota`'s checks into the slot.
pub(crate) struct Upload {
    push: Push,
}

impl Upload {
    pub(crate) fn new(total: usize) -> Result<Self, PushError> {
        Ok(Self {
            push: Push::new(total, EXPECTED.max_image_size)?,
        })
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<(), PushError> {
        self.push
            .feed(bytes, &EXPECTED, &PUBLIC_KEY, &mut Slot, &mut Decks)
    }

    /// The whole bundle is in: checks it and sets the slot to boot.
    pub(crate) fn finish(&mut self) -> Result<(), PushError> {
        self.push.finish(&EXPECTED, &mut Slot, &mut Decks)?;
        RESTART.store(true, Ordering::Release);
        Ok(())
    }

    pub(crate) fn cancel(&mut self) {
        self.push.cancel(&mut Slot, &mut Decks);
    }
}

/// The slot the deck is not running (`main/ota_slot.c`).
struct Slot;

#[cfg(not(test))]
mod slot {
    unsafe extern "C" {
        pub(super) fn ruzina_ota_slot_size() -> u32;
        pub(super) fn ruzina_ota_begin() -> i32;
        pub(super) fn ruzina_ota_write(data: *const u8, len: usize) -> i32;
        pub(super) fn ruzina_ota_abort();
        pub(super) fn ruzina_ota_end() -> i32;
        pub(super) fn ruzina_ota_descriptor(project: *mut u8, version: *mut u8) -> i32;
        pub(super) fn ruzina_ota_set_boot() -> i32;
        pub(super) fn ruzina_ota_running_slot() -> *const core::ffi::c_char;
    }
}

/// Host tests: no slot.
#[cfg(test)]
#[allow(non_snake_case, clippy::missing_safety_doc)]
mod slot {
    pub(super) unsafe fn ruzina_ota_slot_size() -> u32 {
        0
    }
    pub(super) unsafe fn ruzina_ota_begin() -> i32 {
        -1
    }
    pub(super) unsafe fn ruzina_ota_write(_: *const u8, _: usize) -> i32 {
        -1
    }
    pub(super) unsafe fn ruzina_ota_abort() {}
    pub(super) unsafe fn ruzina_ota_end() -> i32 {
        -1
    }
    pub(super) unsafe fn ruzina_ota_descriptor(_: *mut u8, _: *mut u8) -> i32 {
        -1
    }
    pub(super) unsafe fn ruzina_ota_set_boot() -> i32 {
        -1
    }
    pub(super) unsafe fn ruzina_ota_running_slot() -> *const core::ffi::c_char {
        c"test".as_ptr()
    }
}

fn result(code: i32) -> Result<(), i32> {
    if code == 0 { Ok(()) } else { Err(code) }
}

// SAFETY (each call): the web task is the only caller of ota_slot.c's slot functions, and
// every pointer is valid for the length passed.
impl SlotFlash for Slot {
    type Error = i32;

    fn update_slot_size(&mut self) -> Option<u32> {
        Some(unsafe { slot::ruzina_ota_slot_size() }).filter(|&size| size != 0)
    }

    fn begin(&mut self, _image_size: u32) -> Result<(), i32> {
        result(unsafe { slot::ruzina_ota_begin() })
    }

    fn write(&mut self, data: &[u8]) -> Result<(), i32> {
        result(unsafe { slot::ruzina_ota_write(data.as_ptr(), data.len()) })
    }

    fn abort(&mut self) {
        unsafe { slot::ruzina_ota_abort() }
    }

    fn end(&mut self) -> Result<(), i32> {
        result(unsafe { slot::ruzina_ota_end() })
    }

    fn descriptor(&mut self) -> Result<([u8; 32], [u8; 32]), i32> {
        let (mut project, mut version) = ([0; 32], [0; 32]);
        result(unsafe { slot::ruzina_ota_descriptor(project.as_mut_ptr(), version.as_mut_ptr()) })?;
        Ok((project, version))
    }

    fn set_boot(&mut self) -> Result<(), i32> {
        result(unsafe { slot::ruzina_ota_set_boot() })
    }
}

/// The decks: an update starts only with both stopped.
struct Decks;

impl Playback for Decks {
    /// With both decks stopped, the screen shows UPDATING FIRMWARE from here until the
    /// restart, or until the update ends unfinished.
    fn stop_for_update(&mut self) -> bool {
        let stopped = !crate::media_signal::playing()
            .into_iter()
            .any(|playing| playing);
        if stopped {
            crate::screen_idle::set_updating(true);
        }
        stopped
    }

    /// The update ended unfinished: the screen comes back. No deck was stopped.
    fn resume(&mut self) {
        crate::screen_idle::set_updating(false);
    }
}

/// The running slot's name (`ota_0` or `ota_1`).
pub(crate) fn running_slot() -> &'static str {
    // SAFETY: IDF's partition label, static and NUL-terminated for the program's life.
    let label = unsafe { core::ffi::CStr::from_ptr(slot::ruzina_ota_running_slot()) };
    label.to_str().unwrap_or("?")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruzina_ota::bundle::{self, HEADER_SIZE};

    /// A bundle `tools/ota_bundle.py` made with the Ruzina key, around a small stand-in
    /// image (not firmware) of this project.
    const SAMPLE: &[u8] = include_bytes!("../tests/fixtures/ruz-001-sample.ddjota");

    #[test]
    fn a_bundle_signed_with_the_ruzina_key_passes_the_header_checks() {
        let header = &SAMPLE[..HEADER_SIZE];
        let manifest = bundle::parse_header(header, &EXPECTED).unwrap();
        assert_eq!(manifest.project.as_str(), "ruzina_legacy_probe");
        assert_eq!(manifest.image_size as usize, SAMPLE.len() - HEADER_SIZE);
        bundle::verify_signature(header, &PUBLIC_KEY).unwrap();
        // The M2.5 release key does not sign for the deck.
        assert!(bundle::verify_signature(header, &bundle::RELEASE_PUBLIC_KEY).is_err());
    }

    #[test]
    fn a_bundle_for_another_key_or_project_is_refused_before_the_slot() {
        let mut other_key = SAMPLE[..HEADER_SIZE].to_vec();
        other_key[116..123].copy_from_slice(b"rel-001");
        assert_eq!(
            bundle::parse_header(&other_key, &EXPECTED).err(),
            Some(bundle::ManifestError::Key)
        );
        let mut upload = Upload::new(SAMPLE.len()).unwrap();
        let mut tampered = SAMPLE.to_vec();
        tampered[100] ^= 1;
        // The host's slot has no room, so even a good header stops there; a tampered
        // one stops at its signature.
        assert_eq!(
            upload.feed(&tampered[..HEADER_SIZE + 24]),
            Err(PushError::Invalid)
        );
        let mut upload = Upload::new(SAMPLE.len()).unwrap();
        assert_eq!(
            upload.feed(&SAMPLE[..HEADER_SIZE + 24]),
            Err(PushError::Flash)
        );
        assert!(!restart_due());
    }

    #[test]
    fn the_screen_shows_the_update_while_it_is_written_and_comes_back_if_it_ends() {
        let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let before = crate::media_signal::playing();
        // A deck plays: refused, and the screen stays.
        crate::media_signal::playback([false, true], [0; 2]);
        assert!(!Decks.stop_for_update());
        assert!(!crate::screen_idle::updating());
        crate::media_signal::playback([false; 2], [0; 2]);
        assert!(Decks.stop_for_update());
        assert!(crate::screen_idle::updating());
        Decks.resume();
        assert!(!crate::screen_idle::updating());
        crate::media_signal::playback(before, [0; 2]);
    }

    #[test]
    fn a_bundle_larger_than_a_slot_is_refused_at_once() {
        assert!(Upload::new(HEADER_SIZE + SLOT_IMAGE_SIZE_MAX as usize).is_ok());
        assert!(Upload::new(HEADER_SIZE + SLOT_IMAGE_SIZE_MAX as usize + 1).is_err());
        assert!(Upload::new(100).is_err());
    }
}
