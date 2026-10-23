//! Request admission (M2.5 `api_request_allowed`): a Host allow-list against
//! DNS rebinding on every API request, plus, for mutations, the custom
//! `X-DDJ-Control: 1` marker (no image, link or plain form can send it) and a
//! same-origin check whenever the browser sends `Origin`.

use crate::HttpStatus;

/// The mDNS name the deck always answers to.
pub const CANONICAL_HOSTNAME: &str = "ruzina.local";
/// Header and value M2.5 clients send on every mutation.
pub const CONTROL_MARKER_HEADER: &str = "X-DDJ-Control";
pub const CONTROL_MARKER_VALUE: &str = "1";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RequestHead<'a> {
    pub host: Option<&'a str>,
    pub origin: Option<&'a str>,
    pub control_marker: Option<&'a str>,
    /// POST/PUT/DELETE: anything that changes deck or device state.
    pub mutation: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    InvalidHost,
    MissingControlMarker,
    CrossOrigin,
}

impl Refusal {
    pub const fn status(self) -> HttpStatus {
        HttpStatus::Forbidden
    }
}

/// `host[:port]` is the active AP address or the canonical name (an
/// optional trailing dot and any letter case allowed); the port is 1..65535.
pub fn host_allowed(host_header: &str, ap_ipv4: &str) -> bool {
    if host_header.is_empty() || ap_ipv4.is_empty() {
        return false;
    }
    let (host, port) = match host_header.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (host_header, None),
    };
    if host.is_empty() {
        return false;
    }
    if let Some(port) = port {
        let valid = !port.is_empty()
            && port.len() <= 5
            && port.bytes().all(|byte| byte.is_ascii_digit())
            && port
                .parse::<u32>()
                .is_ok_and(|value| (1..=65_535).contains(&value));
        if !valid {
            return false;
        }
    }
    if host == ap_ipv4 {
        return true;
    }
    let host = host.strip_suffix('.').unwrap_or(host);
    host.eq_ignore_ascii_case(CANONICAL_HOSTNAME)
}

/// `Origin` must name an allowed host over `http://`; `null` and foreign
/// origins are refused for mutations.
pub fn origin_allowed(origin: &str, ap_ipv4: &str) -> bool {
    origin
        .strip_prefix("http://")
        .is_some_and(|host| !host.contains('/') && host_allowed(host, ap_ipv4))
}

pub fn admit(request: &RequestHead<'_>, ap_ipv4: &str) -> Result<(), Refusal> {
    if !request.host.is_some_and(|host| host_allowed(host, ap_ipv4)) {
        return Err(Refusal::InvalidHost);
    }
    if request.mutation {
        if request.control_marker != Some(CONTROL_MARKER_VALUE) {
            return Err(Refusal::MissingControlMarker);
        }
        if let Some(origin) = request.origin
            && !origin_allowed(origin, ap_ipv4)
        {
            return Err(Refusal::CrossOrigin);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AP: &str = "192.168.4.1";

    #[test]
    fn host_allow_list_matches_m25_and_refuses_rebinding() {
        for host in [
            "192.168.4.1",
            "192.168.4.1:80",
            "ruzina.local",
            "RUZINA.LOCAL",
            "ruzina.local.",
            "ruzina.local:8080",
        ] {
            assert!(host_allowed(host, AP), "{host}");
        }
        for host in [
            "",
            ":80",
            "evil.example",
            "192.168.4.10",
            "ruzina.local.evil",
            "ruzina.local:",
            "ruzina.local:0",
            "ruzina.local:65536",
            "ruzina.local:8o",
            "192.168.4.1:80:80",
        ] {
            assert!(!host_allowed(host, AP), "{host}");
        }
        assert!(!host_allowed("ruzina.local", ""));
        // The AP address is read from the active interface, not hard-coded.
        assert!(host_allowed("10.0.0.1", "10.0.0.1"));
    }

    #[test]
    fn mutations_need_the_marker_and_a_same_origin() {
        let read = RequestHead {
            host: Some("ruzina.local"),
            ..RequestHead::default()
        };
        assert_eq!(admit(&read, AP), Ok(()));
        assert_eq!(
            admit(&RequestHead { host: None, ..read }, AP),
            Err(Refusal::InvalidHost)
        );
        let write = RequestHead {
            mutation: true,
            ..read
        };
        assert_eq!(admit(&write, AP), Err(Refusal::MissingControlMarker));
        let marked = RequestHead {
            control_marker: Some("1"),
            ..write
        };
        assert_eq!(admit(&marked, AP), Ok(()));
        assert_eq!(
            admit(
                &RequestHead {
                    control_marker: Some("true"),
                    ..write
                },
                AP
            ),
            Err(Refusal::MissingControlMarker)
        );
        for origin in ["http://ruzina.local", "http://192.168.4.1:80"] {
            let ok = RequestHead {
                origin: Some(origin),
                ..marked
            };
            assert_eq!(admit(&ok, AP), Ok(()), "{origin}");
        }
        for origin in [
            "null",
            "https://evil.example",
            "http://evil.example",
            "http://ruzina.local/x",
        ] {
            let bad = RequestHead {
                origin: Some(origin),
                ..marked
            };
            assert_eq!(admit(&bad, AP), Err(Refusal::CrossOrigin), "{origin}");
        }
        assert_eq!(Refusal::CrossOrigin.status(), HttpStatus::Forbidden);
    }
}
