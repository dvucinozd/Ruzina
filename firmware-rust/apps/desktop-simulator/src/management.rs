//! Host management composition: actual settings/profile owners over RAM.
use crate::{
    profile_store::{RamProfiles, ready},
    session::Session,
};
use ruzina_controller_profile::{Profile, ProfileRegistry};
use ruzina_controller_profile_storage::ProfileScanBuffers;
use ruzina_settings::{
    network::PasswordUpdate,
    owner::{RecordStatus, SettingsError},
};
use ruzina_web::{HttpStatus as H, router::Route, upload::UploadKind};

pub struct Management {
    pub profiles: RamProfiles,
    pub registry: ProfileRegistry,
    staging: Option<(UploadKind, usize, Vec<u8>)>,
}
impl Management {
    pub fn new() -> Self {
        Self {
            profiles: RamProfiles::new(),
            registry: ProfileRegistry::new(),
            staging: None,
        }
    }
}
fn copy(text: &str, out: &mut [u8]) -> Result<usize, H> {
    let target = out.get_mut(..text.len()).ok_or(H::InternalServerError)?;
    target.copy_from_slice(text.as_bytes());
    Ok(text.len())
}
fn quoted(text: &str) -> Result<String, H> {
    let mut buf = [0; 1024];
    let n = ruzina_web::json::write_string(text, &mut buf).map_err(|_| H::InternalServerError)?;
    Ok(String::from_utf8(buf[..n].to_vec()).expect("JSON UTF8"))
}
impl Session {
    pub fn management_get(&mut self, route: Route, out: &mut [u8]) -> Result<usize, H> {
        let text=match route {
            Route::OtaConfigGet=>{let n=self.settings.network();format!("{{\"ssid\":{},\"url\":{},\"password_set\":{},\"simulated\":true}}",quoted(n.ssid())?,quoted(n.url())?,n.has_password())}
            Route::ControllerProfiles=>{
                let mut rows=Vec::new();for i in 0..self.management.registry.len()as u8 {let p=self.management.registry.profile(i).unwrap();rows.push(format!("{{\"id\":{},\"version\":{},\"vid\":{},\"pid\":{}}}",quoted(p.id.as_str())?,p.meta.version,p.meta.vid,p.meta.pid));}
                format!("{{\"simulated\":true,\"profiles\":[{}]}}",rows.join(","))
            }
            Route::Resources=>format!("{{\"simulated\":true,\"units\":\"bytes\",\"internal\":null,\"dma\":null,\"psram\":null,\"elapsed_ms\":{}}}",self.diagnostics().elapsed_ms),
            Route::DiagnosticLog=>{
                // Only structured owner data, never untrusted raw log/URL strings.
                let s=self.diagnostics();let mut rows=Vec::new();
                for i in 0..s.events.len(){let e=s.events.get(i).unwrap();rows.push(format!("{{\"sequence\":{},\"elapsed_ms\":{},\"kind\":{}}}",e.sequence,e.elapsed_ms,quoted(&format!("{:?}",e.kind))?));}
                format!("{{\"simulated\":true,\"elapsed_ms\":{},\"health\":{},\"events_capacity\":16,\"overwritten\":{},\"truncated\":{},\"events\":[{}]}}",s.elapsed_ms,quoted(&format!("{:?}",s.health()))?,s.events.overwritten,s.events.overwritten!=0,rows.join(","))
            }
            Route::Firmware=>"{\"simulated\":true,\"running\":\"host-simulator\",\"candidate\":null,\"boot_confirmed\":false,\"trust_configured\":false}".into(),
            _=>return Err(H::NotImplemented),
        };
        copy(&text, out)
    }
    pub fn management_begin(&mut self, kind: UploadKind, len: usize) -> Result<(), H> {
        if self.management.staging.is_some() {
            return Err(H::ServiceUnavailable);
        }
        match kind {
            UploadKind::Config if !(1..=512).contains(&len) => return Err(H::PayloadTooLarge),
            UploadKind::Profile(request)
                if len != request.length || !(32..=16384).contains(&len) =>
            {
                return Err(H::PayloadTooLarge);
            }
            _ => {}
        }
        if matches!(kind, UploadKind::Profile(_))
            && matches!(
                self.management.registry.activation_state(),
                ruzina_controller_profile::ProfileActivationState::Activating
                    | ruzina_controller_profile::ProfileActivationState::Initializing
            )
        {
            return Err(H::Conflict);
        }
        // No production trust/project/flash descriptor adapter is configured in
        // the demo Session. HTTP OTA composition is exercised with explicit test
        // trust and the A/B device model in ruzina-ota integration tests.
        if matches!(kind, UploadKind::Ota) {
            return Err(H::ServiceUnavailable);
        }
        self.management.staging = Some((kind, len, Vec::with_capacity(len)));
        Ok(())
    }
    pub fn management_write(&mut self, bytes: &[u8]) -> Result<(), H> {
        let (_, len, buf) = self.management.staging.as_mut().ok_or(H::Conflict)?;
        if bytes.len() > *len - buf.len() {
            return Err(H::BadRequest);
        }
        buf.extend_from_slice(bytes);
        Ok(())
    }
    pub fn management_cancel(&mut self) {
        self.management.staging = None;
    }
    pub fn management_finish(&mut self) -> Result<(), H> {
        let (kind, len, bytes) = self.management.staging.take().ok_or(H::Conflict)?;
        if bytes.len() != len {
            return Err(H::BadRequest);
        }
        match kind {
            UploadKind::Config => {
                if matches!(
                    self.settings.status().1,
                    RecordStatus::NewerPreserved { .. }
                ) {
                    return Err(H::Conflict);
                }
                let config = ruzina_web::config::parse(&bytes).ok_or(H::BadRequest)?;
                let result = match config {
                    ruzina_web::config::Config::Clear => self.settings.clear_network(),
                    ruzina_web::config::Config::Set {
                        ssid,
                        url,
                        password,
                    } => self.settings.set_network(
                        ssid,
                        match password {
                            None => PasswordUpdate::Keep,
                            Some("") => PasswordUpdate::Open,
                            Some(p) => PasswordUpdate::Set(p),
                        },
                        url,
                    ),
                };
                result.map_err(|e| match e {
                    SettingsError::NewerRecordPreserved => H::Conflict,
                    SettingsError::Store(_) => H::InternalServerError,
                    _ => H::BadRequest,
                })
            }
            UploadKind::Profile(request) => {
                let profile = Profile::parse(&bytes).map_err(|_| H::BadRequest)?;
                for i in 0..profile.input_count() {
                    let entry = profile.input(i).ok_or(H::BadRequest)?;
                    ruzina_controller_profile::adapt_profile_event(
                        ruzina_controller_profile::ProfileEvent {
                            semantic_type: entry.semantic_type,
                            semantic_id: entry.semantic_id,
                            value: 0,
                        },
                    )
                    .map_err(|_| H::BadRequest)?;
                }
                let registry = &self.management.registry;
                let existing = (0..registry.len() as u8)
                    .any(|i| registry.profile(i).is_some_and(|p| p.id == request.id));
                if !existing && registry.len() >= ruzina_controller_profile::MAX_CONTROLLER_PROFILES
                {
                    return Err(H::Conflict);
                }
                let mut buffers = Box::new(ProfileScanBuffers::new());
                let result = ready(buffers.install(
                    &mut self.management.profiles,
                    request.id.as_str(),
                    &bytes,
                    request.overwrite,
                ));
                let status = ruzina_web::profile::install_status(&result.map(|_| ()));
                if status.0 != H::Ok {
                    return Err(status.0);
                }
                let report = ready(buffers.scan(&mut self.management.profiles))
                    .map_err(|_| H::InternalServerError)?;
                self.management.registry.apply_rescan(&report.registry);
                Ok(())
            }
            UploadKind::Ota => Err(H::ServiceUnavailable),
        }
    }
}
