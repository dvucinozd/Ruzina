//! Complete HTTP/1.1 responses written into a caller buffer. Every response
//! closes the connection, carries its exact length, is never cached and
//! states its type (`nosniff`); nothing is written when it does not fit.

use crate::HttpStatus;

pub const TEXT: &str = "text/plain; charset=utf-8";
pub const JSON: &str = "application/json";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Response<'a> {
    pub status: HttpStatus,
    pub content_type: &'a str,
    pub body: &'a [u8],
    /// Only for redirects.
    pub location: Option<&'a str>,
}

impl<'a> Response<'a> {
    pub const fn text(status: HttpStatus, body: &'a str) -> Self {
        Self {
            status,
            content_type: TEXT,
            body: body.as_bytes(),
            location: None,
        }
    }

    pub const fn json(body: &'a [u8]) -> Self {
        Self {
            status: HttpStatus::Ok,
            content_type: JSON,
            body,
            location: None,
        }
    }

    pub const fn redirect(location: &'a str) -> Self {
        Self {
            status: HttpStatus::Found,
            content_type: TEXT,
            body: b"",
            location: Some(location),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Overflow;

/// Writes `response` to the start of `out` and returns its length.
pub fn write_response(response: &Response<'_>, out: &mut [u8]) -> Result<usize, Overflow> {
    let head = write_head(response, response.body.len(), out)?;
    let end = head.checked_add(response.body.len()).ok_or(Overflow)?;
    out.get_mut(head..end)
        .ok_or(Overflow)?
        .copy_from_slice(response.body);
    Ok(end)
}

/// The status line and headers for a body of `body_len` bytes (the
/// response's own body is not written).
pub fn write_head(
    response: &Response<'_>,
    body_len: usize,
    out: &mut [u8],
) -> Result<usize, Overflow> {
    let mut writer = Writer { out, len: 0 };
    writer.text("HTTP/1.1 ")?;
    writer.number(u32::from(response.status.code()))?;
    writer.text(" ")?;
    writer.text(response.status.reason())?;
    writer.text("\r\nContent-Type: ")?;
    writer.text(response.content_type)?;
    writer.text("\r\nContent-Length: ")?;
    writer.number(u32::try_from(body_len).map_err(|_| Overflow)?)?;
    if let Some(location) = response.location {
        writer.text("\r\nLocation: ")?;
        writer.text(location)?;
    }
    writer.text(
        "\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
    )?;
    Ok(writer.len)
}

struct Writer<'a> {
    out: &'a mut [u8],
    len: usize,
}

impl Writer<'_> {
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), Overflow> {
        let end = self.len.checked_add(bytes.len()).ok_or(Overflow)?;
        self.out
            .get_mut(self.len..end)
            .ok_or(Overflow)?
            .copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<(), Overflow> {
        self.bytes(text.as_bytes())
    }

    fn number(&mut self, mut value: u32) -> Result<(), Overflow> {
        let mut digits = [0u8; 10];
        let mut start = digits.len();
        loop {
            start -= 1;
            digits[start] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        self.bytes(&digits[start..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_are_complete_closed_and_uncached() {
        let mut out = [0u8; 256];
        let len = write_response(
            &Response::text(HttpStatus::Conflict, "Deck is playing"),
            &mut out,
        )
        .unwrap();
        assert_eq!(
            core::str::from_utf8(&out[..len]).unwrap(),
            "HTTP/1.1 409 Conflict\r\nContent-Type: text/plain; charset=utf-8\r\n\
Content-Length: 15\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
Connection: close\r\n\r\nDeck is playing"
        );
        let len = write_response(
            &Response::redirect("http://ruzina.local/index.html"),
            &mut out,
        )
        .unwrap();
        let text = core::str::from_utf8(&out[..len]).unwrap();
        assert!(text.starts_with("HTTP/1.1 302 Found\r\n"));
        assert!(text.contains("\r\nLocation: http://ruzina.local/index.html\r\n"));
        assert!(text.contains("\r\nContent-Length: 0\r\n"));
        assert_eq!(
            write_response(&Response::json(&[b'x'; 200]), &mut out),
            Err(Overflow)
        );
    }
}
