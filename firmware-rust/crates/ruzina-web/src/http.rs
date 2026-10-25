//! Bounded HTTP/1.1 request heads for the Web adapter. The parser reads one
//! complete head from a caller-owned buffer and borrows from it; it never
//! allocates. Anything ambiguous is a refusal rather than a guess: bare LF,
//! folded or malformed header lines, a repeated Host or Content-Length, a
//! non-canonical length and any Transfer-Encoding (no chunked bodies).

use crate::{HttpStatus, guard, parse};

/// Largest request head read (request line plus headers).
pub const HEAD_MAX_BYTES: usize = 2048;
/// Header lines accepted, of any name.
pub const HEADERS_MAX: usize = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Get,
    Post,
}

impl Method {
    /// Requests that change deck or device state (see [`guard`]).
    pub const fn mutates(self) -> bool {
        matches!(self, Self::Post)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Request<'a> {
    pub method: Method,
    pub path: &'a str,
    /// After `?`, or empty.
    pub query: &'a str,
    pub host: Option<&'a str>,
    pub origin: Option<&'a str>,
    pub control_marker: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub content_length: usize,
    /// Bytes of the head including the blank line; the body follows.
    pub head_len: usize,
}

impl Request<'_> {
    /// The [`guard`] view of this request.
    pub const fn guard_head(&self) -> guard::RequestHead<'_> {
        guard::RequestHead {
            host: self.host,
            origin: self.origin,
            control_marker: self.control_marker,
            mutation: self.method.mutates(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpError {
    /// No blank line yet; read more if the head may still fit.
    Incomplete,
    HeadTooLarge,
    TooManyHeaders,
    Malformed,
    UnsupportedMethod,
    UnsupportedVersion,
    /// Transfer-Encoding (chunked bodies) is not accepted.
    TransferEncoding,
}

impl HttpError {
    pub const fn status(self) -> HttpStatus {
        match self {
            Self::Incomplete | Self::Malformed => HttpStatus::BadRequest,
            Self::HeadTooLarge | Self::TooManyHeaders => HttpStatus::HeaderFieldsTooLarge,
            Self::UnsupportedMethod | Self::TransferEncoding => HttpStatus::NotImplemented,
            Self::UnsupportedVersion => HttpStatus::VersionNotSupported,
        }
    }
}

/// Parses the head at the start of `buffer`.
pub fn parse_request(buffer: &[u8]) -> Result<Request<'_>, HttpError> {
    let window = &buffer[..buffer.len().min(HEAD_MAX_BYTES)];
    let Some(end) = window.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
        return Err(if buffer.len() >= HEAD_MAX_BYTES {
            HttpError::HeadTooLarge
        } else {
            HttpError::Incomplete
        });
    };
    let head = core::str::from_utf8(&window[..end]).map_err(|_| HttpError::Malformed)?;
    if !head.is_ascii() {
        return Err(HttpError::Malformed);
    }
    let mut lines = head.split("\r\n");
    let (method, target) = request_line(lines.next().ok_or(HttpError::Malformed)?)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    let mut request = Request {
        method,
        path,
        query,
        host: None,
        origin: None,
        control_marker: None,
        content_type: None,
        content_length: 0,
        head_len: end + 4,
    };
    let mut length_seen = false;
    for (count, line) in lines.enumerate() {
        if count >= HEADERS_MAX {
            return Err(HttpError::TooManyHeaders);
        }
        let (name, value) = header_line(line)?;
        if name.eq_ignore_ascii_case("host") {
            set_once(&mut request.host, value)?;
        } else if name.eq_ignore_ascii_case("origin") {
            set_once(&mut request.origin, value)?;
        } else if name.eq_ignore_ascii_case(guard::CONTROL_MARKER_HEADER) {
            set_once(&mut request.control_marker, value)?;
        } else if name.eq_ignore_ascii_case("content-type") {
            set_once(&mut request.content_type, value)?;
        } else if name.eq_ignore_ascii_case("content-length") {
            if length_seen {
                return Err(HttpError::Malformed);
            }
            length_seen = true;
            request.content_length = parse::parse_u32(value).ok_or(HttpError::Malformed)? as usize;
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(HttpError::TransferEncoding);
        }
    }
    Ok(request)
}

/// A repeated identity header is ambiguous, so it is refused.
fn set_once<'a>(slot: &mut Option<&'a str>, value: &'a str) -> Result<(), HttpError> {
    match slot.replace(value) {
        Some(_) => Err(HttpError::Malformed),
        None => Ok(()),
    }
}

fn request_line(line: &str) -> Result<(Method, &str), HttpError> {
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(HttpError::Malformed);
    };
    let method = match method {
        "GET" => Method::Get,
        "POST" => Method::Post,
        _ if !method.is_empty() && method.bytes().all(|byte| byte.is_ascii_uppercase()) => {
            return Err(HttpError::UnsupportedMethod);
        }
        _ => return Err(HttpError::Malformed),
    };
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(if version.starts_with("HTTP/") {
            HttpError::UnsupportedVersion
        } else {
            HttpError::Malformed
        });
    }
    let printable = |byte: u8| byte.is_ascii_graphic() && byte != b'#';
    if !target.starts_with('/') || !target.bytes().all(printable) {
        return Err(HttpError::Malformed);
    }
    Ok((method, target))
}

fn header_line(line: &str) -> Result<(&str, &str), HttpError> {
    let (name, value) = line.split_once(':').ok_or(HttpError::Malformed)?;
    let token = |byte: u8| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte);
    if name.is_empty() || !name.bytes().all(token) {
        // Also refuses obsolete line folding (a line starting with space).
        return Err(HttpError::Malformed);
    }
    let value = value.trim_matches([' ', '\t']);
    if value
        .bytes()
        .any(|byte| byte.is_ascii_control() && byte != b'\t')
    {
        return Err(HttpError::Malformed);
    }
    Ok((name, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_head_is_borrowed_with_its_fields() {
        let raw = b"POST /api/control?action=cue&deck=1 HTTP/1.1\r\nHost: ruzina.local\r\n\
X-DDJ-Control: 1\r\nOrigin: http://ruzina.local\r\ncontent-length: 0\r\n\r\nrest";
        let request = parse_request(raw).unwrap();
        assert_eq!(request.method, Method::Post);
        assert_eq!(request.path, "/api/control");
        assert_eq!(request.query, "action=cue&deck=1");
        assert_eq!(request.host, Some("ruzina.local"));
        assert_eq!(request.origin, Some("http://ruzina.local"));
        assert_eq!(request.control_marker, Some("1"));
        assert_eq!(request.content_length, 0);
        assert_eq!(&raw[request.head_len..], b"rest");
        assert!(request.guard_head().mutation);
    }

    #[test]
    fn ambiguous_or_unsupported_heads_are_refused() {
        let cases: [(&[u8], HttpError); 13] = [
            (b"GET / HTTP/1.1\r\nHost: a\r\n", HttpError::Incomplete),
            (b"GET / HTTP/1.1\nHost: a\n\n", HttpError::Incomplete),
            (b"GET / HTTP/2.0\r\n\r\n", HttpError::UnsupportedVersion),
            (b"PUT / HTTP/1.1\r\n\r\n", HttpError::UnsupportedMethod),
            (b"get / HTTP/1.1\r\n\r\n", HttpError::Malformed),
            (b"GET  / HTTP/1.1\r\n\r\n", HttpError::Malformed),
            (b"GET http://x/ HTTP/1.1\r\n\r\n", HttpError::Malformed),
            (b"GET /a#b HTTP/1.1\r\n\r\n", HttpError::Malformed),
            (
                b"GET / HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n",
                HttpError::Malformed,
            ),
            (
                b"GET / HTTP/1.1\r\nContent-Length: 01\r\n\r\n",
                HttpError::Malformed,
            ),
            (
                b"GET / HTTP/1.1\r\nContent-Length: 1\r\ncontent-length: 1\r\n\r\n",
                HttpError::Malformed,
            ),
            (
                b"GET / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
                HttpError::TransferEncoding,
            ),
            (
                b"GET / HTTP/1.1\r\nHost: a\r\n folded\r\n\r\n",
                HttpError::Malformed,
            ),
        ];
        for (raw, error) in cases {
            assert_eq!(
                parse_request(raw),
                Err(error),
                "{}",
                core::str::from_utf8(raw).unwrap()
            );
        }
    }

    #[test]
    fn heads_are_bounded_in_bytes_and_lines() {
        let mut long = [b'a'; HEAD_MAX_BYTES + 8];
        long[..16].copy_from_slice(b"GET / HTTP/1.1\r\n");
        assert_eq!(parse_request(&long), Err(HttpError::HeadTooLarge));

        let mut many = [0u8; 1024];
        let mut len = 0;
        let mut push = |bytes: &[u8]| {
            many[len..len + bytes.len()].copy_from_slice(bytes);
            len += bytes.len();
        };
        push(b"GET / HTTP/1.1\r\n");
        for _ in 0..=HEADERS_MAX {
            push(b"X-A: 1\r\n");
        }
        push(b"\r\n");
        assert_eq!(parse_request(&many[..len]), Err(HttpError::TooManyHeaders));
        assert_eq!(
            HttpError::TooManyHeaders.status(),
            HttpStatus::HeaderFieldsTooLarge
        );
    }
}
