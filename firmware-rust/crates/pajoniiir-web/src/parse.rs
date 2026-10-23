//! Bounded, all-or-nothing request parsing (M2.5 `web_api_helpers`,
//! `p4_ota_pull_config`). Nothing is truncated: an over-long or ambiguous
//! value is a refusal, never a shorter value the caller might act on.

/// Longest query string an adapter reads.
pub const QUERY_MAX: usize = 128;
/// Longest value inside it.
pub const QUERY_VALUE_MAX: usize = 32;
/// JSON bodies (network configuration) are small and read whole.
pub const JSON_BODY_MAX: usize = 512;
/// Controller-profile uploads (M2.5 `WEB_API_PROFILE_*_SIZE`).
pub const PROFILE_UPLOAD_MIN: usize = 32;
pub const PROFILE_UPLOAD_MAX: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BodyKind {
    /// Query-parameter mutations carry no body.
    Empty,
    Json,
    Profile,
}

/// Content-Length admission before anything is read.
pub fn body_length_allowed(kind: BodyKind, length: usize) -> bool {
    match kind {
        BodyKind::Empty => length == 0,
        BodyKind::Json => (1..=JSON_BODY_MAX).contains(&length),
        BodyKind::Profile => (PROFILE_UPLOAD_MIN..=PROFILE_UPLOAD_MAX).contains(&length),
    }
}

/// Canonical decimal only: no sign, space or leading zeros beyond "0".
pub fn parse_u32(value: &str) -> Option<u32> {
    let canonical = !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'));
    canonical.then(|| value.parse().ok()).flatten()
}

/// Like [`parse_u32`] within `minimum..=maximum`.
pub fn parse_bounded(value: &str, minimum: u32, maximum: u32) -> Option<u32> {
    parse_u32(value).filter(|parsed| (minimum..=maximum).contains(parsed))
}

/// One `key=value` from a query string. Values are plain tokens; anything
/// percent-encoded, `+`-encoded, repeated or over-long is refused.
pub fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    if query.len() > QUERY_MAX {
        return None;
    }
    let mut found = None;
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=')?;
        if name != key {
            continue;
        }
        let token = !value.is_empty()
            && value.len() <= QUERY_VALUE_MAX
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
        if !token || found.is_some() {
            return None;
        }
        found = Some(value);
    }
    found
}

/// Credentials belong in a POST body; a query that names one is refused
/// before it can reach a log.
pub fn query_carries_secret(query: &str) -> bool {
    query.split('&').any(|pair| {
        let name = pair.split_once('=').map_or(pair, |(name, _)| name);
        name.eq_ignore_ascii_case("password") || name.eq_ignore_ascii_case("pass")
    })
}

/// A JSON string field `"key": "value"`: no escapes, terminated, at most
/// `capacity` bytes, or nothing at all (M2.5 `p4_ota_cfg_extract_string`).
pub fn json_string<'a>(body: &'a str, key: &str, capacity: usize) -> Option<&'a str> {
    let value = json_value(body, key)?;
    let rest = value.strip_prefix('"')?;
    let end = rest.find('"')?;
    let text = &rest[..end];
    (!text.contains('\\') && text.len() <= capacity).then_some(text)
}

/// Whether a boolean field is present and `true`.
pub fn json_true(body: &str, key: &str) -> bool {
    json_value(body, key).is_some_and(|value| value.starts_with("true"))
}

fn json_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    if body.len() > JSON_BODY_MAX {
        return None;
    }
    let mut search = body;
    while let Some(at) = search.find('"') {
        let after = &search[at + 1..];
        if let Some(rest) = after
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('"'))
        {
            let rest = rest.trim_start_matches([' ', '\t', '\r', '\n']);
            if let Some(value) = rest.strip_prefix(':') {
                return Some(value.trim_start_matches([' ', '\t', '\r', '\n']));
            }
        }
        search = after;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_are_canonical_and_bounded() {
        assert_eq!(parse_u32("0"), Some(0));
        assert_eq!(parse_u32("16383"), Some(16_383));
        assert_eq!(parse_u32("4294967295"), Some(u32::MAX));
        for bad in [
            "",
            "+1",
            " 1",
            "1 ",
            "-1",
            "01",
            "1e3",
            "4294967296",
            "0x10",
        ] {
            assert_eq!(parse_u32(bad), None, "{bad}");
        }
        assert_eq!(parse_bounded("16384", 0, 16_383), None);
        assert_eq!(parse_bounded("2", 1, 2), Some(2));
    }

    #[test]
    fn query_values_are_plain_single_bounded_tokens() {
        let query = "action=play_pause&deck=2&value=100";
        assert_eq!(query_value(query, "action"), Some("play_pause"));
        assert_eq!(query_value(query, "deck"), Some("2"));
        assert_eq!(query_value(query, "missing"), None);
        assert_eq!(query_value("deck=1&deck=2", "deck"), None);
        assert_eq!(query_value("action=play%20pause", "action"), None);
        assert_eq!(query_value("action=a+b", "action"), None);
        assert_eq!(query_value("action", "action"), None);
        assert_eq!(query_value("action=", "action"), None);
        let long = [b'a'; QUERY_VALUE_MAX + 1];
        let long = core::str::from_utf8(&long).unwrap();
        let mut query = [0u8; 64];
        query[..7].copy_from_slice(b"action=");
        query[7..7 + long.len()].copy_from_slice(long.as_bytes());
        let query = core::str::from_utf8(&query[..7 + long.len()]).unwrap();
        assert_eq!(query_value(query, "action"), None);
        assert!(query_carries_secret("ssid=x&password=y"));
        assert!(query_carries_secret("PASS"));
        assert!(!query_carries_secret("action=pfl&deck=1"));
    }

    #[test]
    fn json_fields_are_all_or_nothing() {
        let body = r#"{"ssid":"ZAKLJUCANO", "password" : "secret123","clear":true}"#;
        assert_eq!(json_string(body, "ssid", 32), Some("ZAKLJUCANO"));
        assert_eq!(json_string(body, "password", 64), Some("secret123"));
        assert_eq!(json_string(body, "absent", 64), None);
        assert_eq!(json_string(body, "ssid", 4), None, "refused, not truncated");
        assert!(json_true(body, "clear"));
        assert!(!json_true(body, "ssid"));
        assert_eq!(json_string(r#"{"ssid":"a\"b"}"#, "ssid", 32), None);
        assert_eq!(json_string(r#"{"ssid":"abc"#, "ssid", 32), None);
        for cut in 1..body.len() {
            if let Some(value) = json_string(&body[..cut], "password", 64) {
                assert_eq!(value, "secret123");
            }
        }
    }

    #[test]
    fn body_lengths_are_admitted_per_route() {
        assert!(body_length_allowed(BodyKind::Empty, 0));
        assert!(!body_length_allowed(BodyKind::Empty, 1));
        assert!(!body_length_allowed(BodyKind::Json, 0));
        assert!(!body_length_allowed(BodyKind::Json, JSON_BODY_MAX + 1));
        assert!(body_length_allowed(BodyKind::Profile, PROFILE_UPLOAD_MIN));
        assert!(!body_length_allowed(
            BodyKind::Profile,
            PROFILE_UPLOAD_MIN - 1
        ));
        assert!(!body_length_allowed(
            BodyKind::Profile,
            PROFILE_UPLOAD_MAX + 1
        ));
    }
}
