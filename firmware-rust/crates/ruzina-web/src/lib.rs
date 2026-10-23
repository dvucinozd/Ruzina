//! Transport-independent Web Remote and C6 network contracts (RP-12).
//!
//! Nothing here serves HTTP or drives a radio. The pieces are the rules an
//! HTTP adapter and a C6 link must apply, host-tested with a fake transport:
//! request admission ([`guard`]), bounded parsing ([`parse`]), Web actions as
//! the same semantic controls a controller produces ([`remote`]), latest-value
//! ordering of continuous controls ([`ordering`]), generation-stamped status
//! and loads ([`status`]), the MAIN meter from accepted output ([`meter`]),
//! controller-profile upload admission and responses ([`profile`], RP-02) and
//! a generation-fenced C6 link with a single Wi-Fi transition lease
//! ([`link`]).
#![no_std]
#![forbid(unsafe_code)]

pub mod guard;
pub mod link;
pub mod meter;
pub mod ordering;
pub mod parse;
pub mod profile;
pub mod remote;
pub mod status;

/// HTTP status an adapter returns for a refusal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpStatus {
    Ok = 200,
    BadRequest = 400,
    Forbidden = 403,
    Conflict = 409,
    PayloadTooLarge = 413,
    InternalServerError = 500,
}
