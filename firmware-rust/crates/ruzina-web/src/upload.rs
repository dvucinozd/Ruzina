//! Bounded management upload identities. The server owns the lease, the backend
//! owns staging. Only finish may publish a config/profile or boot candidate.
use crate::{
    HttpStatus,
    http::{Method, Request},
    parse, profile,
    router::{self, Route},
};

#[derive(Clone, Copy, Debug)]
pub enum UploadKind {
    Config,
    Profile(profile::UploadRequest),
    Ota,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadLease(pub(crate) u64);
pub(crate) struct Active {
    pub lease: UploadLease,
    pub remaining: usize,
}

pub fn kind(request: &Request<'_>) -> Result<Option<UploadKind>, HttpStatus> {
    let route = router::route(request.method, request.path).ok();
    if matches!(
        route,
        Some(Route::OtaConfigSet | Route::ControllerProfileUpload | Route::OtaUpload)
    ) && !request.query.is_empty()
    {
        return Err(HttpStatus::BadRequest);
    }
    let kind = match route {
        Some(Route::OtaConfigSet) => {
            if !parse::body_length_allowed(parse::BodyKind::Json, request.content_length) {
                return Err(HttpStatus::PayloadTooLarge);
            }
            if request.content_type != Some("application/json") {
                return Err(HttpStatus::BadRequest);
            }
            UploadKind::Config
        }
        Some(Route::ControllerProfileUpload) => {
            if request.content_type != Some("application/octet-stream") {
                return Err(HttpStatus::BadRequest);
            }
            UploadKind::Profile(
                profile::admit_upload(
                    request.profile_id,
                    request.profile_overwrite,
                    request.content_length,
                    false,
                )
                .map_err(|e| e.status())?,
            )
        }
        Some(Route::OtaUpload) => {
            if !(212..=4_194_492).contains(&request.content_length) {
                return Err(HttpStatus::PayloadTooLarge);
            }
            if request.content_type != Some("application/octet-stream") {
                return Err(HttpStatus::BadRequest);
            }
            UploadKind::Ota
        }
        _ => return Ok(None),
    };
    if request.method != Method::Post {
        return Ok(None);
    }
    Ok(Some(kind))
}
