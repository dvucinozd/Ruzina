//! USB Audio Class 1 output contracts (RP-09/16, M2.5
//! `controller_usb_audio`).
//!
//! Nothing here touches a USB host stack. These are the rules a UAC adapter
//! must follow, host-tested with fake transfers: which descriptor formats are
//! offered ([`descriptor`]), the 1 ms packet cadence and the one place PCM is
//! packed for the wire ([`packet`]), stream ownership, epochs and teardown
//! including EP0 timeouts ([`stream`]) and active-versus-idle fault
//! accounting ([`health`]).
#![no_std]
#![forbid(unsafe_code)]

pub mod descriptor;
pub mod health;
pub mod packet;
pub mod resample;
pub mod stream;
