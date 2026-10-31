//! Shared Library media loading, independent of the filesystem transport.
//!
//! - [`pdb`]: Rekordbox `export.pdb` catalog import into a staged catalog and
//!   playlists (`alloc` adds the heap-backed stage).
//! - With `preflight`: per-track load preflight, i.e. audio stat, ANLZ DAT/EXT
//!   import and source cues.
//!
//! The native USB0 broker client and the legacy IDF exFAT owner implement the
//! file traits, so both platforms run one implementation. The caller still owns
//! the deck's load gate, persisted Hot Cues, audio-source preparation and the
//! final commit.
#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod pdb;

#[cfg(feature = "preflight")]
mod preflight;
#[cfg(feature = "preflight")]
pub use preflight::*;
