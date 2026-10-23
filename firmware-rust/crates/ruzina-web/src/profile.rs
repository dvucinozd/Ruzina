//! Controller-profile upload route (M2.5 `POST /api/controller-profile`).
//!
//! The request carries the S3CP bytes as its body, the profile id in
//! `X-DDJ-Profile-ID` and an optional `X-DDJ-Profile-Overwrite: 0|1`. Headers
//! and Content-Length are checked before the body is read; the bytes are then
//! installed by `ruzina-controller-profile-storage`, which validates them
//! with the shared `Profile::parse`. Only one storage operation runs at a
//! time, and none while a profile transfer to the controller is in progress.

use crate::{HttpStatus, parse};
use ruzina_controller_profile::ProfileId;
use ruzina_controller_profile_storage::InstallError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadRequest {
    pub id: ProfileId,
    pub overwrite: bool,
    pub length: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadRefusal {
    InvalidId,
    InvalidOverwrite,
    Size,
    /// Another install or a profile transfer is running.
    Busy,
}

impl UploadRefusal {
    pub const fn status(self) -> HttpStatus {
        match self {
            Self::Busy => HttpStatus::Conflict,
            _ => HttpStatus::BadRequest,
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidId => "Invalid or missing X-DDJ-Profile-ID",
            Self::InvalidOverwrite => "Overwrite must be 0 or 1",
            Self::Size => "Profile size must be 32..16384 bytes",
            Self::Busy => "Profile manager is busy",
        }
    }
}

/// Admits an upload before its body is read.
pub fn admit_upload(
    id: Option<&str>,
    overwrite: Option<&str>,
    content_length: usize,
    busy: bool,
) -> Result<UploadRequest, UploadRefusal> {
    let id = id
        .and_then(|id| ProfileId::parse(id).ok())
        .ok_or(UploadRefusal::InvalidId)?;
    let overwrite = match overwrite {
        None | Some("") | Some("0") => false,
        Some("1") => true,
        Some(_) => return Err(UploadRefusal::InvalidOverwrite),
    };
    if !parse::body_length_allowed(parse::BodyKind::Profile, content_length) {
        return Err(UploadRefusal::Size);
    }
    if busy {
        return Err(UploadRefusal::Busy);
    }
    Ok(UploadRequest {
        id,
        overwrite,
        length: content_length,
    })
}

/// Response for a finished install attempt (M2.5 statuses and texts).
pub fn install_status<E>(result: &Result<(), InstallError<E>>) -> (HttpStatus, &'static str) {
    match result {
        Ok(()) => (HttpStatus::Ok, "Profile installed"),
        Err(InstallError::InvalidId | InstallError::Size | InstallError::InvalidProfile(_)) => {
            (HttpStatus::BadRequest, "Invalid controller profile or CRC")
        }
        Err(InstallError::Exists) => (HttpStatus::Conflict, "Profile exists or manager is busy"),
        Err(InstallError::Io { .. }) => {
            (HttpStatus::InternalServerError, "SD profile install failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruzina_controller_profile::ParseError;
    use ruzina_controller_profile_storage::InstallStep;

    #[test]
    fn headers_and_length_are_checked_before_the_body() {
        let ok = admit_upload(Some("pioneer_ddj_flx4"), Some("1"), 4_096, false).unwrap();
        assert_eq!(ok.id.as_str(), "pioneer_ddj_flx4");
        assert!(ok.overwrite);
        for overwrite in [None, Some(""), Some("0")] {
            assert!(
                !admit_upload(Some("x"), overwrite, 32, false)
                    .unwrap()
                    .overwrite
            );
        }
        for (id, overwrite, length, busy, refusal) in [
            (None, None, 100, false, UploadRefusal::InvalidId),
            (Some("../etc"), None, 100, false, UploadRefusal::InvalidId),
            (Some("a b"), None, 100, false, UploadRefusal::InvalidId),
            (
                Some("ok"),
                Some("true"),
                100,
                false,
                UploadRefusal::InvalidOverwrite,
            ),
            (
                Some("ok"),
                Some("01"),
                100,
                false,
                UploadRefusal::InvalidOverwrite,
            ),
            (Some("ok"), None, 31, false, UploadRefusal::Size),
            (Some("ok"), None, 16_385, false, UploadRefusal::Size),
            (Some("ok"), None, 100, true, UploadRefusal::Busy),
        ] {
            assert_eq!(
                admit_upload(id, overwrite, length, busy),
                Err(refusal),
                "{id:?}"
            );
        }
        assert_eq!(UploadRefusal::Busy.status(), HttpStatus::Conflict);
        assert_eq!(UploadRefusal::Size.status(), HttpStatus::BadRequest);
    }

    #[test]
    fn install_results_map_to_m25_statuses() {
        type R = Result<(), InstallError<()>>;
        let cases: [(R, HttpStatus); 5] = [
            (Ok(()), HttpStatus::Ok),
            (
                Err(InstallError::InvalidProfile(ParseError::Crc)),
                HttpStatus::BadRequest,
            ),
            (Err(InstallError::Size), HttpStatus::BadRequest),
            (Err(InstallError::Exists), HttpStatus::Conflict),
            (
                Err(InstallError::Io {
                    step: InstallStep::Promote,
                    error: Some(()),
                }),
                HttpStatus::InternalServerError,
            ),
        ];
        for (result, status) in cases {
            assert_eq!(install_status(&result).0, status);
        }
    }
}
