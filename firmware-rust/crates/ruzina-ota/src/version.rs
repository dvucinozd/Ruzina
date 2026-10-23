//! Comparable release versions (M2.5 `p4_ota_pull_release_compare`).
//!
//! Release versions keep the M2.5 `git describe` scheme:
//! `RC<n>-<d>-g<hash>`, `M<n>-<d>-g<hash>`, `M<major>.<minor>-<d>-g<hash>`,
//! optionally chained (`M2.2-13-ge0f9add-11-ga1cc05c`) and `-dirty`. The
//! milestone family sorts after RC; within a family major, minor and the sum
//! of chained distances are monotonic. The same position with a different
//! hash is unordered: the histories disagree.

use core::cmp::Ordering;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseOrder {
    Same,
    Newer,
    Older,
    Unordered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Position {
    family: u8,
    major: u32,
    minor: u32,
    distance: u32,
}

fn number(text: &[u8], at: &mut usize) -> Option<u32> {
    let start = *at;
    let mut value = 0u32;
    while let Some(digit) = text.get(*at).filter(|byte| byte.is_ascii_digit()) {
        value = value
            .checked_mul(10)?
            .checked_add(u32::from(digit - b'0'))?;
        *at += 1;
    }
    (*at > start).then_some(value)
}

fn position(text: &str) -> Option<Position> {
    let bytes = text.as_bytes();
    let (family, mut at) = if bytes.starts_with(b"RC") {
        (1, 2)
    } else if bytes.starts_with(b"M") {
        (2, 1)
    } else {
        return None;
    };
    let major = number(bytes, &mut at)?;
    let mut minor = 0;
    if family == 2 && bytes.get(at) == Some(&b'.') {
        at += 1;
        minor = number(bytes, &mut at)?;
    }
    let mut distance = 0u32;
    while at < bytes.len() && &bytes[at..] != b"-dirty" {
        if bytes[at] != b'-' {
            return None;
        }
        at += 1;
        distance = distance.checked_add(number(bytes, &mut at)?)?;
        if bytes.get(at..at + 2) != Some(b"-g") {
            return None;
        }
        at += 2;
        let hash = bytes[at..]
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        if hash < 7 {
            return None;
        }
        at += hash;
    }
    Some(Position {
        family,
        major,
        minor,
        distance,
    })
}

pub fn compare(offered: &str, running: &str) -> ReleaseOrder {
    if offered == running {
        return ReleaseOrder::Same;
    }
    let (Some(offered), Some(running)) = (position(offered), position(running)) else {
        return ReleaseOrder::Unordered;
    };
    match (
        offered.family,
        offered.major,
        offered.minor,
        offered.distance,
    )
        .cmp(&(
            running.family,
            running.major,
            running.minor,
            running.distance,
        )) {
        Ordering::Greater => ReleaseOrder::Newer,
        Ordering::Less => ReleaseOrder::Older,
        Ordering::Equal => ReleaseOrder::Unordered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_version_shapes_order_like_m25() {
        for (offered, running, order) in [
            ("M2.5-4-g1a2b3c4d", "M2.5-0-g20f1c3f", ReleaseOrder::Newer),
            ("M2.5-0-g20f1c3f", "M2.5-4-g1a2b3c4d", ReleaseOrder::Older),
            ("M2.5-0-g20f1c3f", "M2.5-0-g20f1c3f", ReleaseOrder::Same),
            (
                "M2.5-0-g20f1c3f",
                "M2.5-0-gdeadbee",
                ReleaseOrder::Unordered,
            ),
            ("M3-0-g1234567", "M2.9-99-g1234567", ReleaseOrder::Newer),
            ("M1-0-g1234567", "RC9-400-g1234567", ReleaseOrder::Newer),
            ("RC2-1-g1234567", "RC1-237-g7bf0fd3c", ReleaseOrder::Newer),
            (
                "M2.2-13-ge0f9add-11-ga1cc05c",
                "M2.2-20-g0000000",
                ReleaseOrder::Newer,
            ),
            (
                "M2.5-4-g1a2b3c4d-dirty",
                "M2.5-3-g1a2b3c4",
                ReleaseOrder::Newer,
            ),
            ("M2.5-4-g1a2b3c", "M2.5-3-g1a2b3c4", ReleaseOrder::Unordered),
            ("v2.5", "M2.5-3-g1a2b3c4", ReleaseOrder::Unordered),
            (
                "M2.5-4-g1a2b3c4-extra",
                "M2.5-3-g1a2b3c4",
                ReleaseOrder::Unordered,
            ),
            (
                "M2.5-99999999999-g1a2b3c4",
                "M2.5-3-g1a2b3c4",
                ReleaseOrder::Unordered,
            ),
            (
                "RC1.2-0-g1234567",
                "RC1-0-g1234567",
                ReleaseOrder::Unordered,
            ),
        ] {
            assert_eq!(compare(offered, running), order, "{offered} vs {running}");
        }
    }
}
