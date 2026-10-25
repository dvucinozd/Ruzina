//! Transport-independent Web Remote and C6 network contracts (RP-12).
//!
//! Nothing here drives a socket or a radio. The pieces are the rules an HTTP
//! adapter and a C6 link must apply, host-tested with a fake transport:
//! request admission ([`guard`]), bounded parsing ([`parse`]), Web actions as
//! the same semantic controls a controller produces ([`remote`]), latest-value
//! ordering of continuous controls ([`ordering`]), generation-stamped status
//! and loads ([`status`]), the MAIN meter from accepted output ([`meter`]),
//! controller-profile upload admission and responses ([`profile`], RP-02) and
//! a generation-fenced C6 link with a single Wi-Fi transition lease
//! ([`link`]).
//!
//! The HTTP adapter core turns bytes into those rules: a bounded request-head
//! parser ([`http`]), the M2.5 routes ([`router`]), complete responses
//! ([`response`]), the status and library JSON ([`json`]), the built-in Web
//! Remote page ([`assets`]) and one request handler over a product backend
//! ([`server`]).
#![no_std]
#![forbid(unsafe_code)]

pub mod assets;
pub mod guard;
pub mod http;
pub mod json;
pub mod link;
pub mod meter;
pub mod ordering;
pub mod parse;
pub mod profile;
pub mod remote;
pub mod response;
pub mod router;
pub mod server;
pub mod status;

/// HTTP status an adapter returns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpStatus {
    Ok = 200,
    Found = 302,
    BadRequest = 400,
    Forbidden = 403,
    NotFound = 404,
    MethodNotAllowed = 405,
    Conflict = 409,
    PayloadTooLarge = 413,
    HeaderFieldsTooLarge = 431,
    InternalServerError = 500,
    NotImplemented = 501,
    ServiceUnavailable = 503,
    VersionNotSupported = 505,
}

impl HttpStatus {
    pub const fn code(self) -> u16 {
        self as u16
    }

    pub const fn reason(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Found => "Found",
            Self::BadRequest => "Bad Request",
            Self::Forbidden => "Forbidden",
            Self::NotFound => "Not Found",
            Self::MethodNotAllowed => "Method Not Allowed",
            Self::Conflict => "Conflict",
            Self::PayloadTooLarge => "Payload Too Large",
            Self::HeaderFieldsTooLarge => "Request Header Fields Too Large",
            Self::InternalServerError => "Internal Server Error",
            Self::NotImplemented => "Not Implemented",
            Self::ServiceUnavailable => "Service Unavailable",
            Self::VersionNotSupported => "HTTP Version Not Supported",
        }
    }
}
