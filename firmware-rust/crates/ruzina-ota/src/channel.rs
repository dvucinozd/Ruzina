//! Pull-OTA channel document (M2.5 `latest.json`).
//!
//! ```json
//! { "schema_version": 1, "release": "M2.5-4-g1a2b3c4d",
//!   "p4": { "url": "M2.5-4-g1a2b3c4d/main-deck-p4.ddjota",
//!           "size": 2147132, "sha256": "83ba98…" } }
//! ```
//!
//! Discovery, not authentication: a tampered document can name the wrong
//! file but the bundle's own signature decides what is installed. The whole
//! document is validated before any field is read: UTF-8, complete JSON,
//! nesting at most [`MAX_DEPTH`], no duplicate (or escaped) keys in any
//! object, nothing after the top-level object. Unknown keys and other
//! targets are allowed, so one channel can serve several boards.

use crate::bundle::FixedText;

pub const DOC_MAX: usize = 4096;
pub const MAX_DEPTH: usize = 8;
pub const MAX_KEYS: usize = 16;
pub const RELEASE_MAX: usize = 48;
pub const URL_MAX: usize = 192;
const SCHEMA_SUPPORTED: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Offer {
    pub release: FixedText<{ RELEASE_MAX + 1 }>,
    /// Relative to the configured channel base URL.
    pub url: FixedText<{ URL_MAX + 1 }>,
    /// Whole bundle: signed header plus image.
    pub size: u32,
    pub sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelError {
    TooLarge,
    Malformed,
    Unsupported,
    /// Well-formed, but nothing for this board.
    NoTarget,
    FieldTooLong,
    BadValue,
}

impl ChannelError {
    pub const fn name(self) -> &'static str {
        match self {
            Self::TooLarge => "channel document too large",
            Self::Malformed => "malformed",
            Self::Unsupported => "unsupported-schema",
            Self::NoTarget => "no-target",
            Self::FieldTooLong => "field-too-long",
            Self::BadValue => "bad-value",
        }
    }
}

struct Cursor<'a> {
    doc: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn ws(&mut self) {
        while matches!(self.doc.get(self.at), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.doc.get(self.at).copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        let hit = self.peek() == Some(byte);
        self.at += usize::from(hit);
        hit
    }

    /// Returns the raw string body; `keys` refuses escapes.
    fn string(&mut self, key: bool) -> Option<&'a [u8]> {
        if !self.eat(b'"') {
            return None;
        }
        let start = self.at;
        loop {
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    return Some(&self.doc[start..self.at - 1]);
                }
                byte if byte < 0x20 => return None,
                b'\\' if key => return None,
                b'\\' => {
                    self.at += 1;
                    match self.peek()? {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.at += 1,
                        b'u' => {
                            let hex = self.doc.get(self.at + 1..self.at + 5)?;
                            if !hex.iter().all(u8::is_ascii_hexdigit) {
                                return None;
                            }
                            self.at += 5;
                        }
                        _ => return None,
                    }
                }
                _ => self.at += 1,
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Option<()> {
        self.eat(b'-');
        match self.peek()? {
            b'0' => self.at += 1,
            b'1'..=b'9' => {
                self.digits();
            }
            _ => return None,
        }
        if self.eat(b'.') && self.digits() == 0 {
            return None;
        }
        if self.eat(b'e') || self.eat(b'E') {
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if self.digits() == 0 {
                return None;
            }
        }
        Some(())
    }

    fn value(&mut self, depth: usize) -> Option<()> {
        self.ws();
        match self.peek()? {
            b'"' => self.string(false).map(drop),
            b'{' | b'[' if depth >= MAX_DEPTH => None,
            b'{' => self.object(depth + 1),
            b'[' => {
                self.at += 1;
                self.ws();
                if self.eat(b']') {
                    return Some(());
                }
                loop {
                    self.value(depth + 1)?;
                    self.ws();
                    if self.eat(b']') {
                        return Some(());
                    }
                    if !self.eat(b',') {
                        return None;
                    }
                }
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => {
                for literal in [&b"true"[..], b"false", b"null"] {
                    if self.doc[self.at..].starts_with(literal) {
                        self.at += literal.len();
                        return Some(());
                    }
                }
                None
            }
        }
    }

    fn object(&mut self, depth: usize) -> Option<()> {
        self.at += 1;
        let mut keys: [&[u8]; MAX_KEYS] = [&[]; MAX_KEYS];
        let mut count = 0;
        self.ws();
        if self.eat(b'}') {
            return Some(());
        }
        loop {
            self.ws();
            let key = self.string(true)?;
            if count == MAX_KEYS || keys[..count].contains(&key) {
                return None;
            }
            keys[count] = key;
            count += 1;
            self.ws();
            if !self.eat(b':') {
                return None;
            }
            self.value(depth)?;
            self.ws();
            if self.eat(b'}') {
                return Some(());
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }
}

/// The value of `key` among the direct members of a validated object.
fn member<'a>(object: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let mut cursor = Cursor { doc: object, at: 1 };
    loop {
        cursor.ws();
        let name = cursor.string(true)?;
        cursor.ws();
        cursor.eat(b':');
        cursor.ws();
        let start = cursor.at;
        cursor.value(0)?;
        if name == key.as_bytes() {
            return Some(&object[start..cursor.at]);
        }
        cursor.ws();
        if !cursor.eat(b',') {
            return None;
        }
    }
}

fn text<const N: usize>(value: &[u8], max: usize) -> Result<FixedText<N>, ChannelError> {
    let body = value
        .strip_prefix(b"\"")
        .and_then(|value| value.strip_suffix(b"\""))
        .ok_or(ChannelError::Malformed)?;
    if body.len() > max {
        return Err(ChannelError::FieldTooLong);
    }
    // No escapes: no field legitimately needs one.
    core::str::from_utf8(body)
        .ok()
        .filter(|body| !body.contains('\\'))
        .and_then(FixedText::new)
        .ok_or(ChannelError::BadValue)
}

fn integer(value: &[u8]) -> Result<u32, ChannelError> {
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return Err(ChannelError::Malformed);
    }
    core::str::from_utf8(value)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value != 0)
        .ok_or(ChannelError::BadValue)
}

fn sha256(value: &[u8]) -> Result<[u8; 32], ChannelError> {
    let hex = text::<65>(value, 64)?;
    let hex = hex.as_str().as_bytes();
    if hex.len() != 64 {
        return Err(ChannelError::BadValue);
    }
    let mut out = [0; 32];
    for (byte, pair) in out.iter_mut().zip(hex.chunks(2)) {
        let pair = core::str::from_utf8(pair).map_err(|_| ChannelError::BadValue)?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| ChannelError::BadValue)?;
    }
    Ok(out)
}

/// Relative path segments only: no scheme, host, query, fragment, escapes,
/// colon, backslash, empty, `.` or `..` segment.
pub fn relative_url_valid(url: &str) -> bool {
    !url.is_empty()
        && !url.contains(['?', '#', '%', ':', '\\'])
        && url
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | ".."))
        && url.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

pub fn parse(doc: &[u8], target: &str) -> Result<Offer, ChannelError> {
    if doc.len() > DOC_MAX {
        return Err(ChannelError::TooLarge);
    }
    core::str::from_utf8(doc).map_err(|_| ChannelError::Malformed)?;
    let mut cursor = Cursor { doc, at: 0 };
    cursor.ws();
    let start = cursor.at;
    if cursor.peek() != Some(b'{') {
        return Err(ChannelError::Malformed);
    }
    cursor.value(0).ok_or(ChannelError::Malformed)?;
    let root = &doc[start..cursor.at];
    cursor.ws();
    if cursor.at != doc.len() {
        return Err(ChannelError::Malformed);
    }

    let schema = integer(member(root, "schema_version").ok_or(ChannelError::Malformed)?)?;
    if schema != SCHEMA_SUPPORTED {
        return Err(ChannelError::Unsupported);
    }
    let release = text(
        member(root, "release").ok_or(ChannelError::Malformed)?,
        RELEASE_MAX,
    )?;
    let board = member(root, target).ok_or(ChannelError::NoTarget)?;
    if !board.starts_with(b"{") {
        return Err(ChannelError::Malformed);
    }
    let url = text(
        member(board, "url").ok_or(ChannelError::Malformed)?,
        URL_MAX,
    )?;
    if !relative_url_valid(url.as_str()) {
        return Err(ChannelError::BadValue);
    }
    Ok(Offer {
        release,
        url,
        size: integer(member(board, "size").ok_or(ChannelError::Malformed)?)?,
        sha256: sha256(member(board, "sha256").ok_or(ChannelError::Malformed)?)?,
    })
}

/// Wrap-safe offer expiry (M2.5 offers live ten minutes).
pub const OFFER_TTL_MS: u32 = 10 * 60 * 1000;

pub const fn offer_fresh(now_ms: u32, offered_at_ms: u32, ttl_ms: u32) -> bool {
    ttl_ms > 0 && now_ms.wrapping_sub(offered_at_ms) <= ttl_ms
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::format;
    use std::string::String;

    const HASH: &str = "83ba98aa00112233445566778899aabbccddeeff00112233445566778899ae22";

    fn doc(body: &str) -> Result<Offer, ChannelError> {
        parse(body.as_bytes(), "p4")
    }

    fn channel(p4: &str) -> String {
        format!(r#"{{"schema_version": 1, "release": "M2.5-4-g1a2b3c4d", "p4": {p4}}}"#)
    }

    fn target(url: &str, size: &str, sha: &str) -> String {
        format!(r#"{{"url": "{url}", "size": {size}, "sha256": "{sha}"}}"#)
    }

    #[test]
    fn accepts_the_released_shape_and_ignores_other_targets() {
        let body = format!(
            r#" {{"schema_version":1,"release":"M2.5-4-g1a2b3c4d","s3":{{"url":"x/y.ddjota","size":9,"sha256":"{HASH}"}},"notes":["a",{{"b":null}},-1.5e3,true],
"p4":{}}}
"#,
            target("M2.5-4-g1a2b3c4d/main-deck-p4.ddjota", "2147132", HASH)
        );
        let offer = doc(&body).unwrap();
        assert_eq!(offer.release.as_str(), "M2.5-4-g1a2b3c4d");
        assert_eq!(offer.url.as_str(), "M2.5-4-g1a2b3c4d/main-deck-p4.ddjota");
        assert_eq!(offer.size, 2_147_132);
        assert_eq!(offer.sha256[0], 0x83);
        assert_eq!(offer.sha256[31], 0x22);
        assert_eq!(
            parse(body.as_bytes(), "jc8012"),
            Err(ChannelError::NoTarget)
        );
    }

    #[test]
    fn rejects_incomplete_duplicate_or_deep_documents() {
        let good = target("a/b.ddjota", "100", HASH);
        assert!(doc(&channel(&good)).is_ok());
        let nested = |levels: usize| {
            channel(&format!(
                "{good}, \"x\": {}",
                "[".repeat(levels) + &"]".repeat(levels)
            ))
        };
        assert!(doc(&nested(MAX_DEPTH - 1)).is_ok(), "M2.5 depth bound");
        for (body, error) in [
            (channel(&good)[..40].into(), ChannelError::Malformed),
            (format!("{} x", channel(&good)), ChannelError::Malformed),
            (format!("[{}]", channel(&good)), ChannelError::Malformed),
            (
                channel(&good).replacen(
                    "\"release\"",
                    "\"release\": \"M1-0-g1234567\", \"release\"",
                    1,
                ),
                ChannelError::Malformed,
            ),
            (
                channel(&target("a/b.ddjota", "100, \"size\": 5", HASH)),
                ChannelError::Malformed,
            ),
            (
                channel(&good).replace("\"release\"", "\"rel\\u0065ase\""),
                ChannelError::Malformed,
            ),
            (nested(MAX_DEPTH), ChannelError::Malformed),
            (
                channel(&good).replace("1,", "2,"),
                ChannelError::Unsupported,
            ),
            (
                channel(&good).replace("1,", "1.0,"),
                ChannelError::Malformed,
            ),
            (channel("\"a/b\""), ChannelError::Malformed),
            (
                channel(&target("a/b.ddjota", "0", HASH)),
                ChannelError::BadValue,
            ),
            (
                channel(&target("a/b.ddjota", "4294967296", HASH)),
                ChannelError::BadValue,
            ),
            (
                channel(&target("a/b.ddjota", "-1", HASH)),
                ChannelError::Malformed,
            ),
            (
                channel(&target("a/b.ddjota", "100", &HASH[1..])),
                ChannelError::BadValue,
            ),
            (
                channel(&target("a/b.ddjota", "100", &HASH.replace('8', "g"))),
                ChannelError::BadValue,
            ),
            (
                channel(&target(&"a".repeat(193), "100", HASH)),
                ChannelError::FieldTooLong,
            ),
            (
                channel(&target("a\\/b", "100", HASH)),
                ChannelError::BadValue,
            ),
            (channel(&target("", "100", HASH)), ChannelError::BadValue),
        ] {
            assert_eq!(doc(&body), Err(error), "{body}");
        }
        assert_eq!(
            parse(&[b' '; DOC_MAX + 1], "p4"),
            Err(ChannelError::TooLarge)
        );
        assert_eq!(
            parse(b"{\"a\":\"\xff\"}", "p4"),
            Err(ChannelError::Malformed)
        );
        let mut wide = String::from("{");
        for key in 0..=MAX_KEYS {
            wide += &format!("\"k{key}\":0,");
        }
        wide += "\"schema_version\":1}";
        assert_eq!(doc(&wide), Err(ChannelError::Malformed));
    }

    #[test]
    fn bundle_urls_stay_relative_to_the_channel() {
        assert!(relative_url_valid("M2.5/main-deck-p4.ddjota"));
        for url in [
            "/abs.ddjota",
            "https://evil/x",
            "a/../b",
            "./a",
            "a//b",
            "a/",
            "a?b",
            "a#b",
            "a%2fb",
            "a b",
            "a\\b",
            "host:80/a",
        ] {
            assert!(!relative_url_valid(url), "{url}");
        }
    }

    #[test]
    fn offers_expire_wrap_safely() {
        assert!(offer_fresh(1_000, 1_000, OFFER_TTL_MS));
        assert!(offer_fresh(OFFER_TTL_MS + 5, 5, OFFER_TTL_MS));
        assert!(!offer_fresh(OFFER_TTL_MS + 6, 5, OFFER_TTL_MS));
        assert!(offer_fresh(10, u32::MAX - 10, OFFER_TTL_MS));
        assert!(!offer_fresh(10, 10, 0));
    }
}
