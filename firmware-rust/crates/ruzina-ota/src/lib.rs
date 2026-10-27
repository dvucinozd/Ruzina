//! Signed A/B OTA and candidate readiness (RP-13, M2.5 `ota_manifest`,
//! `p4_ota`, `p4_ota_pull` and `firmware_health`).
//!
//! Nothing here touches flash, a radio or TLS. These are the rules a target
//! adapter must apply, host-tested with fake slots and transports: the
//! byte-compatible signed `.ddjota` header ([`bundle`]), the strict pull
//! channel document ([`channel`]), comparable release versions
//! ([`version`]), the streaming install that verifies everything before the
//! inactive slot is erased or switched ([`install`]), the one-at-a-time pull
//! operation with expiring offers ([`pull`]) and pending-candidate
//! confirmation or rollback ([`boot`]).
#![no_std]
#![forbid(unsafe_code)]
pub mod push;

pub mod boot;
pub mod bundle;
pub mod channel;
pub mod install;
pub mod pull;
pub mod version;
