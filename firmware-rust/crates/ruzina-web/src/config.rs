//! Strict bounded config JSON. Reject unsupported escapes/keys rather than
//! extracting a plausible value from malformed or duplicated fields.
#[derive(Eq, PartialEq)]
pub enum Config<'a> {
    Clear,
    Set {
        ssid: &'a str,
        url: &'a str,
        password: Option<&'a str>,
    },
}
impl core::fmt::Debug for Config<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Config(<redacted>)")
    }
}
pub fn parse(body: &[u8]) -> Option<Config<'_>> {
    if body.is_empty() || body.len() > crate::parse::JSON_BODY_MAX {
        return None;
    }
    let mut input = core::str::from_utf8(body).ok()?.trim();
    input = input.strip_prefix('{')?.trim_start();
    let mut ssid = None;
    let mut url = None;
    let mut password = None;
    let mut keep = None;
    let mut clear = None;
    loop {
        let key = string(&mut input)?;
        input = input.trim_start().strip_prefix(':')?.trim_start();
        match key {
            "ssid" => {
                if ssid.is_some() {
                    return None;
                }
                ssid = Some(string(&mut input)?);
            }
            "url" => {
                if url.is_some() {
                    return None;
                }
                url = Some(string(&mut input)?);
            }
            "password" => {
                if password.is_some() {
                    return None;
                }
                password = Some(string(&mut input)?);
            }
            "keep_password" => {
                if keep.is_some() {
                    return None;
                }
                keep = Some(boolean(&mut input)?);
            }
            "clear" => {
                if clear.is_some() {
                    return None;
                }
                clear = Some(boolean(&mut input)?);
            }
            _ => return None,
        }
        input = input.trim_start();
        if let Some(rest) = input.strip_prefix('}') {
            if !rest.trim().is_empty() {
                return None;
            }
            break;
        }
        input = input.strip_prefix(',')?.trim_start();
    }
    if clear == Some(true) {
        return (ssid.is_none() && url.is_none() && password.is_none() && keep.is_none())
            .then_some(Config::Clear);
    }
    if clear.is_some()
        || (password.is_some() && keep.is_some())
        || (password.is_none() && keep != Some(true))
    {
        return None;
    }
    Some(Config::Set {
        ssid: ssid?,
        url: url?,
        password,
    })
}
fn string<'a>(input: &mut &'a str) -> Option<&'a str> {
    let rest = input.strip_prefix('"')?;
    let end = rest.find('"')?;
    let value = &rest[..end];
    if value.bytes().any(|b| b == b'\\' || b < 0x20) {
        return None;
    }
    *input = &rest[end + 1..];
    Some(value)
}
fn boolean(input: &mut &str) -> Option<bool> {
    if let Some(rest) = input.strip_prefix("true") {
        *input = rest;
        Some(true)
    } else if let Some(rest) = input.strip_prefix("false") {
        *input = rest;
        Some(false)
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_partial_ambiguous_or_secret_bearing_garbage() {
        for bad in [
            "{}",
            "{\"clear\":true,}",
            "{\"clear\":true}x",
            "{\"clear\":true,\"password\":\"secret\"}",
            "{\"clear\":true,\"clear\":false}",
            "{\"clear\":trueish}",
        ] {
            assert_eq!(parse(bad.as_bytes()), None);
        }
        assert_eq!(parse(b"{\"clear\":true}"), Some(Config::Clear));
        assert_eq!(
            parse(br#"{"ssid":"Club","url":"https://x","keep_password":true}"#),
            Some(Config::Set {
                ssid: "Club",
                url: "https://x",
                password: None
            })
        );
    }
}
