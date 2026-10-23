//! Service-network (pull-OTA Wi-Fi) configuration, validated with the M2.5
//! `p4_ota_pull_config` rules. It is its own record so the passphrase never
//! travels with ordinary settings, and `Debug` never shows it.

use core::fmt;

pub const NETWORK_FORMAT_VERSION: u8 = 1;
pub const SSID_MAX: usize = 32;
pub const PASSPHRASE_MIN: usize = 8;
pub const PASSPHRASE_MAX: usize = 63;
pub const PSK_HEX_LEN: usize = 64;
pub const URL_MAX: usize = 160;
pub const NETWORK_PAYLOAD_BYTES: usize = 3 + SSID_MAX + PSK_HEX_LEN + URL_MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkField {
    Ssid,
    Password,
    Url,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkConfigError {
    Empty(NetworkField),
    TooLong(NetworkField),
    TooShort(NetworkField),
    BadCharacters(NetworkField),
    NotHttps,
    UrlHasCredentials,
    /// A stored record does not decode (length, padding, encoding).
    Malformed,
}

/// 1..32 octets without control characters; UTF-8 is allowed.
pub fn check_ssid(ssid: &str) -> Result<(), NetworkConfigError> {
    let field = NetworkField::Ssid;
    if ssid.is_empty() {
        return Err(NetworkConfigError::Empty(field));
    }
    if ssid.len() > SSID_MAX {
        return Err(NetworkConfigError::TooLong(field));
    }
    if ssid.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        return Err(NetworkConfigError::BadCharacters(field));
    }
    Ok(())
}

/// WPA2-PSK: 8..63 printable ASCII, or exactly 64 hex digits. An open
/// network is an empty password, set explicitly, never a short one.
pub fn check_password(password: &str) -> Result<(), NetworkConfigError> {
    let field = NetworkField::Password;
    let len = password.len();
    if len == 0 {
        return Err(NetworkConfigError::Empty(field));
    }
    if len > PSK_HEX_LEN {
        return Err(NetworkConfigError::TooLong(field));
    }
    if len == PSK_HEX_LEN {
        return if password.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(())
        } else {
            Err(NetworkConfigError::BadCharacters(field))
        };
    }
    if len < PASSPHRASE_MIN {
        return Err(NetworkConfigError::TooShort(field));
    }
    if len > PASSPHRASE_MAX {
        return Err(NetworkConfigError::TooLong(field));
    }
    if password.bytes().any(|byte| !(0x20..0x7f).contains(&byte)) {
        return Err(NetworkConfigError::BadCharacters(field));
    }
    Ok(())
}

/// `https://host...`, printable ASCII without spaces, at most 160 bytes, and
/// no `user:pass@` in the authority (it would reach logs and status).
pub fn check_url(url: &str) -> Result<(), NetworkConfigError> {
    let field = NetworkField::Url;
    if url.is_empty() {
        return Err(NetworkConfigError::Empty(field));
    }
    if url.len() > URL_MAX {
        return Err(NetworkConfigError::TooLong(field));
    }
    if url.bytes().any(|byte| !(0x21..0x7f).contains(&byte)) {
        return Err(NetworkConfigError::BadCharacters(field));
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(NetworkConfigError::NotHttps);
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(NetworkConfigError::UrlHasCredentials);
    }
    if authority.is_empty() {
        return Err(NetworkConfigError::Empty(field));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Text<const N: usize> {
    bytes: [u8; N],
    len: u8,
}

impl<const N: usize> Text<N> {
    const EMPTY: Self = Self {
        bytes: [0; N],
        len: 0,
    };

    fn new(value: &str) -> Self {
        let mut text = Self::EMPTY;
        text.bytes[..value.len()].copy_from_slice(value.as_bytes());
        text.len = value.len() as u8;
        text
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..usize::from(self.len)])
            .expect("constructed from validated UTF-8")
    }
}

/// A validated configuration. The empty value is "no service network".
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NetworkConfig {
    ssid: Text<SSID_MAX>,
    password: Text<PSK_HEX_LEN>,
    url: Text<URL_MAX>,
}

impl NetworkConfig {
    pub const NONE: Self = Self {
        ssid: Text::EMPTY,
        password: Text::EMPTY,
        url: Text::EMPTY,
    };

    /// `password` empty means an open network.
    pub fn new(ssid: &str, password: &str, url: &str) -> Result<Self, NetworkConfigError> {
        check_ssid(ssid)?;
        if !password.is_empty() {
            check_password(password)?;
        }
        check_url(url)?;
        Ok(Self {
            ssid: Text::new(ssid),
            password: Text::new(password),
            url: Text::new(url),
        })
    }

    pub fn is_configured(&self) -> bool {
        self.ssid.len != 0
    }

    pub fn ssid(&self) -> &str {
        self.ssid.as_str()
    }

    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    /// What status and Web APIs may report; the passphrase is never returned.
    pub fn has_password(&self) -> bool {
        self.password.len != 0
    }

    /// For the Wi-Fi association path only. Never log or serialize it.
    pub fn password_for_association(&self) -> &str {
        self.password.as_str()
    }

    pub(crate) fn encode(&self) -> [u8; NETWORK_PAYLOAD_BYTES] {
        let mut out = [0u8; NETWORK_PAYLOAD_BYTES];
        let mut at = 0;
        for (len, bytes) in [
            (self.ssid.len, &self.ssid.bytes[..]),
            (self.password.len, &self.password.bytes[..]),
            (self.url.len, &self.url.bytes[..]),
        ] {
            out[at] = len;
            out[at + 1..at + 1 + bytes.len()].copy_from_slice(bytes);
            at += 1 + bytes.len();
        }
        out
    }

    /// The same rules as entry: a record that fails them is not trusted.
    pub(crate) fn decode(payload: &[u8]) -> Result<Self, NetworkConfigError> {
        if payload.len() != NETWORK_PAYLOAD_BYTES {
            return Err(NetworkConfigError::Malformed);
        }
        let mut at = 0;
        let mut field = |capacity: usize| -> Result<&str, NetworkConfigError> {
            let len = usize::from(payload[at]);
            let bytes = &payload[at + 1..at + 1 + capacity];
            at += 1 + capacity;
            if len > capacity || bytes[len..].iter().any(|byte| *byte != 0) {
                return Err(NetworkConfigError::Malformed);
            }
            core::str::from_utf8(&bytes[..len]).map_err(|_| NetworkConfigError::Malformed)
        };
        let ssid = field(SSID_MAX)?;
        let password = field(PSK_HEX_LEN)?;
        let url = field(URL_MAX)?;
        if ssid.is_empty() && password.is_empty() && url.is_empty() {
            return Ok(Self::NONE);
        }
        Self::new(ssid, password, url)
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self::NONE
    }
}

impl fmt::Debug for NetworkConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetworkConfig")
            .field("ssid", &self.ssid())
            .field("url", &self.url())
            .field(
                "password",
                &if self.has_password() {
                    "<set>"
                } else {
                    "<none>"
                },
            )
            .finish()
    }
}

/// A change from the Web/UI form. `KeepPassword` lets the SSID or URL be
/// corrected without retyping the passphrase (M2.5 NULL password).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PasswordUpdate<'a> {
    Keep,
    Open,
    Set(&'a str),
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::format;

    #[test]
    fn entry_rules_match_m25_and_refuse_silent_failures() {
        assert_eq!(check_ssid("ZAKLJUCANO"), Ok(()));
        assert_eq!(check_ssid("Kavana ČŠŽ"), Ok(()));
        assert_eq!(
            check_ssid(""),
            Err(NetworkConfigError::Empty(NetworkField::Ssid))
        );
        assert_eq!(
            check_ssid(&"a".repeat(33)),
            Err(NetworkConfigError::TooLong(NetworkField::Ssid))
        );
        assert_eq!(
            check_ssid("bad\tname"),
            Err(NetworkConfigError::BadCharacters(NetworkField::Ssid))
        );

        assert_eq!(check_password("12345678"), Ok(()));
        assert_eq!(
            check_password("1234567"),
            Err(NetworkConfigError::TooShort(NetworkField::Password))
        );
        assert_eq!(check_password(&"x".repeat(63)), Ok(()));
        assert_eq!(check_password(&"a".repeat(64)), Ok(()));
        assert_eq!(
            check_password(&"z".repeat(64)),
            Err(NetworkConfigError::BadCharacters(NetworkField::Password))
        );
        assert_eq!(
            check_password(&"a".repeat(65)),
            Err(NetworkConfigError::TooLong(NetworkField::Password))
        );
        assert_eq!(
            check_password("has\nnewline!"),
            Err(NetworkConfigError::BadCharacters(NetworkField::Password))
        );

        assert_eq!(check_url("https://ota.ruzina.example"), Ok(()));
        assert_eq!(
            check_url("http://ota.ruzina.example"),
            Err(NetworkConfigError::NotHttps)
        );
        assert_eq!(
            check_url("https://"),
            Err(NetworkConfigError::Empty(NetworkField::Url))
        );
        assert_eq!(
            check_url("https:///ota"),
            Err(NetworkConfigError::Empty(NetworkField::Url))
        );
        assert_eq!(
            check_url("https://ho st/ota"),
            Err(NetworkConfigError::BadCharacters(NetworkField::Url))
        );
        assert_eq!(
            check_url("https://user:pass@host/ota"),
            Err(NetworkConfigError::UrlHasCredentials)
        );
        assert_eq!(check_url("https://host/ota@v2"), Ok(()));
        assert_eq!(check_url("https://host#frag@x"), Ok(()));
        let long = format!("https://h/{}", "a".repeat(URL_MAX - 10));
        assert_eq!(check_url(&long), Ok(()));
        assert_eq!(
            check_url(&format!("{long}a")),
            Err(NetworkConfigError::TooLong(NetworkField::Url))
        );
    }

    #[test]
    fn records_round_trip_strictly_and_debug_never_shows_the_secret() {
        let config =
            NetworkConfig::new("Deck net", "s3cret-pass", "https://ota.example/m1").unwrap();
        let bytes = config.encode();
        assert_eq!(NetworkConfig::decode(&bytes), Ok(config));
        assert_eq!(
            NetworkConfig::decode(&NetworkConfig::NONE.encode()),
            Ok(NetworkConfig::NONE)
        );
        let shown = format!("{config:?}");
        assert!(shown.contains("Deck net") && shown.contains("<set>"));
        assert!(!shown.contains("s3cret"));

        let mut padded = bytes;
        padded[1 + 20] = b'x'; // beyond the 8-byte SSID
        assert_eq!(
            NetworkConfig::decode(&padded),
            Err(NetworkConfigError::Malformed)
        );
        let mut short_password = bytes;
        short_password[1 + SSID_MAX] = 3;
        short_password[1 + SSID_MAX + 1 + 3..1 + SSID_MAX + 1 + 11].fill(0);
        assert_eq!(
            NetworkConfig::decode(&short_password),
            Err(NetworkConfigError::TooShort(NetworkField::Password))
        );
        assert_eq!(
            NetworkConfig::decode(&bytes[1..]),
            Err(NetworkConfigError::Malformed)
        );
        let open = NetworkConfig::new("Open", "", "https://h").unwrap();
        assert!(!open.has_password());
    }
}
