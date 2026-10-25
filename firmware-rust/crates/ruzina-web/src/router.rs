//! M2.5 Web routes (`web_server_start`). Paths are matched exactly (M2.5
//! registers `/api/status*` and the like as prefixes); a wrong method is 405,
//! an unknown GET is the captive-portal redirect and an unknown POST is 404.

use crate::http::Method;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    Index,
    Style,
    App,
    Status,
    Resources,
    Library,
    Control,
    Load,
    ControllerProfiles,
    ControllerProfileUpload,
    Firmware,
    OtaConfigGet,
    OtaConfigSet,
    OtaUpload,
    Recording,
    RecordingStart,
    RecordingStop,
    DiagnosticLog,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteMiss {
    /// The path exists for another method.
    MethodNotAllowed,
    /// M2.5 catch-all: redirect to `http://ruzina.local/index.html`.
    CaptivePortal,
    NotFound,
}

const ROUTES: &[(&str, Method, Route)] = &[
    ("/", Method::Get, Route::Index),
    ("/index.html", Method::Get, Route::Index),
    ("/style.css", Method::Get, Route::Style),
    ("/app.js", Method::Get, Route::App),
    ("/api/status", Method::Get, Route::Status),
    ("/api/resources", Method::Get, Route::Resources),
    ("/api/library", Method::Get, Route::Library),
    ("/api/control", Method::Post, Route::Control),
    ("/api/load", Method::Post, Route::Load),
    (
        "/api/controller-profiles",
        Method::Get,
        Route::ControllerProfiles,
    ),
    (
        "/api/controller-profile",
        Method::Post,
        Route::ControllerProfileUpload,
    ),
    ("/api/firmware", Method::Get, Route::Firmware),
    ("/api/ota/config", Method::Get, Route::OtaConfigGet),
    ("/api/ota/config", Method::Post, Route::OtaConfigSet),
    ("/api/ota/p4", Method::Post, Route::OtaUpload),
    ("/api/recording", Method::Get, Route::Recording),
    ("/api/recording/start", Method::Post, Route::RecordingStart),
    ("/api/recording/stop", Method::Post, Route::RecordingStop),
    ("/api/diagnostic-log", Method::Get, Route::DiagnosticLog),
];

pub fn route(method: Method, path: &str) -> Result<Route, RouteMiss> {
    let mut path_known = false;
    for (known, allowed, route) in ROUTES {
        if *known == path {
            if *allowed == method {
                return Ok(*route);
            }
            path_known = true;
        }
    }
    Err(if path_known {
        RouteMiss::MethodNotAllowed
    } else if method == Method::Get {
        RouteMiss::CaptivePortal
    } else {
        RouteMiss::NotFound
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_match_exact_paths_and_methods() {
        assert_eq!(route(Method::Get, "/"), Ok(Route::Index));
        assert_eq!(route(Method::Get, "/api/status"), Ok(Route::Status));
        assert_eq!(route(Method::Post, "/api/control"), Ok(Route::Control));
        assert_eq!(
            route(Method::Get, "/api/ota/config"),
            Ok(Route::OtaConfigGet)
        );
        assert_eq!(
            route(Method::Post, "/api/ota/config"),
            Ok(Route::OtaConfigSet)
        );
        assert_eq!(
            route(Method::Get, "/api/control"),
            Err(RouteMiss::MethodNotAllowed)
        );
        assert_eq!(
            route(Method::Get, "/api/statusx"),
            Err(RouteMiss::CaptivePortal)
        );
        assert_eq!(
            route(Method::Get, "/generate_204"),
            Err(RouteMiss::CaptivePortal)
        );
        assert_eq!(route(Method::Post, "/api/nope"), Err(RouteMiss::NotFound));
    }
}
