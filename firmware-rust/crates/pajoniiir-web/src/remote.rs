//! Web Remote actions as product semantics (M2.5 `/api/control`). A Web
//! action produces the same `ControlEvent`s a controller would, so it passes
//! the same admission (RP-04 LOAD LOCK, RP-03 CUE ownership) and never
//! bypasses the deck core.
//!
//! Buttons are explicit taps, a press followed by its release (RP-03: a CUE
//! tap must not leave a hold-preview running). Continuous values carry the
//! controller's 14-bit range and an ordering key for [`crate::ordering`].

use crate::{HttpStatus, parse};
use pajoniiir_controller_core::{ControlEvent, ControlValue, DeckId, SemanticControl};

/// 14-bit absolute range shared with controller profiles (M2.5 0..16383).
pub const REMOTE_ABSOLUTE_MAX: u16 = 0x3fff;

/// Continuous controls ordered by latest value per client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuousControl {
    Volume(DeckId),
    Crossfader,
    Tempo(DeckId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteCommand {
    /// Applied in order; a tap is two events.
    Controls {
        events: [Option<ControlEvent>; 2],
        ordering: Option<ContinuousControl>,
    },
    Seek {
        deck: DeckId,
        position_ms: u32,
    },
    /// `track` is the export's track id, `revision` the catalog revision the
    /// page listed it from (see [`crate::status::resolve_load`]).
    Load {
        deck: DeckId,
        track: u32,
        revision: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteError {
    UnknownAction,
    Deck,
    Value,
    Track,
    /// A credential in the query string is refused before it can be logged.
    SecretInQuery,
}

impl RemoteError {
    pub const fn status(self) -> HttpStatus {
        HttpStatus::BadRequest
    }
}

/// Parses `action=...&deck=1|2&value=...` (plus `track`/`revision` for
/// `load`).
pub fn parse_remote(query: &str) -> Result<RemoteCommand, RemoteError> {
    if parse::query_carries_secret(query) {
        return Err(RemoteError::SecretInQuery);
    }
    let action = parse::query_value(query, "action").ok_or(RemoteError::UnknownAction)?;
    let deck = || match parse::query_value(query, "deck")
        .and_then(|value| parse::parse_bounded(value, 1, 2))
    {
        Some(1) => Ok(DeckId::One),
        Some(_) => Ok(DeckId::Two),
        None => Err(RemoteError::Deck),
    };
    let absolute = || {
        parse::query_value(query, "value")
            .and_then(|value| parse::parse_bounded(value, 0, u32::from(REMOTE_ABSOLUTE_MAX)))
            .map(|value| ControlValue::Absolute {
                value: value as u16,
                max: REMOTE_ABSOLUTE_MAX,
            })
            .ok_or(RemoteError::Value)
    };
    let tap = |deck: DeckId, control| RemoteCommand::Controls {
        events: [
            Some(ControlEvent {
                deck: Some(deck),
                control,
                value: ControlValue::Pressed(true),
            }),
            Some(ControlEvent {
                deck: Some(deck),
                control,
                value: ControlValue::Pressed(false),
            }),
        ],
        ordering: None,
    };
    let continuous = |deck: Option<DeckId>, control, value, ordering| RemoteCommand::Controls {
        events: [
            Some(ControlEvent {
                deck,
                control,
                value,
            }),
            None,
        ],
        ordering: Some(ordering),
    };
    Ok(match action {
        "play_pause" => tap(deck()?, SemanticControl::Play),
        "cue" => tap(deck()?, SemanticControl::Cue),
        "pfl" => tap(deck()?, SemanticControl::Pfl),
        "sync" => tap(deck()?, SemanticControl::Sync),
        "volume" => {
            let deck = deck()?;
            continuous(
                Some(deck),
                SemanticControl::ChannelVolume,
                absolute()?,
                ContinuousControl::Volume(deck),
            )
        }
        "pitch" => {
            let deck = deck()?;
            continuous(
                Some(deck),
                SemanticControl::Tempo,
                absolute()?,
                ContinuousControl::Tempo(deck),
            )
        }
        "crossfader" => continuous(
            None,
            SemanticControl::Crossfader,
            absolute()?,
            ContinuousControl::Crossfader,
        ),
        "seek" => RemoteCommand::Seek {
            deck: deck()?,
            position_ms: parse::query_value(query, "value")
                .and_then(parse::parse_u32)
                .ok_or(RemoteError::Value)?,
        },
        "load" => RemoteCommand::Load {
            deck: deck()?,
            track: parse::query_value(query, "track")
                .and_then(parse::parse_u32)
                .filter(|track| *track != 0)
                .ok_or(RemoteError::Track)?,
            revision: parse::query_value(query, "revision")
                .and_then(parse::parse_u32)
                .filter(|revision| *revision != 0)
                .ok_or(RemoteError::Track)?,
        },
        _ => return Err(RemoteError::UnknownAction),
    })
}

/// M2.5 clamps a Web seek to the known duration; an unknown length passes.
pub const fn clamp_seek_ms(position_ms: u32, duration_ms: Option<u32>) -> u32 {
    match duration_ms {
        Some(duration) if duration > 0 && position_ms > duration => duration,
        _ => position_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(command: RemoteCommand) -> [Option<ControlEvent>; 2] {
        match command {
            RemoteCommand::Controls { events, .. } => events,
            other => panic!("expected controls, got {other:?}"),
        }
    }

    #[test]
    fn buttons_are_explicit_taps_through_product_semantics() {
        let cue = events(parse_remote("action=cue&deck=2").unwrap());
        assert_eq!(cue[0].unwrap().control, SemanticControl::Cue);
        assert_eq!(cue[0].unwrap().deck, Some(DeckId::Two));
        assert_eq!(cue[0].unwrap().value, ControlValue::Pressed(true));
        assert_eq!(cue[1].unwrap().value, ControlValue::Pressed(false));
        for (action, control) in [
            ("play_pause", SemanticControl::Play),
            ("pfl", SemanticControl::Pfl),
            ("sync", SemanticControl::Sync),
        ] {
            let query = match action {
                "play_pause" => "action=play_pause&deck=1",
                "pfl" => "action=pfl&deck=1",
                _ => "action=sync&deck=1",
            };
            let pair = events(parse_remote(query).unwrap());
            assert_eq!(pair[0].unwrap().control, control);
            assert_eq!(pair[1].unwrap().value, ControlValue::Pressed(false));
        }
    }

    #[test]
    fn continuous_values_use_the_14_bit_range_and_an_ordering_key() {
        let RemoteCommand::Controls { events, ordering } =
            parse_remote("action=volume&deck=1&value=16383").unwrap()
        else {
            panic!()
        };
        assert_eq!(ordering, Some(ContinuousControl::Volume(DeckId::One)));
        assert_eq!(
            events[0].unwrap().value,
            ControlValue::Absolute {
                value: 16_383,
                max: REMOTE_ABSOLUTE_MAX
            }
        );
        assert!(events[1].is_none());
        let RemoteCommand::Controls { events, ordering } =
            parse_remote("action=crossfader&value=0").unwrap()
        else {
            panic!()
        };
        assert_eq!(events[0].unwrap().deck, None);
        assert_eq!(ordering, Some(ContinuousControl::Crossfader));
        assert!(matches!(
            parse_remote("action=pitch&deck=2&value=8192").unwrap(),
            RemoteCommand::Controls {
                ordering: Some(ContinuousControl::Tempo(DeckId::Two)),
                ..
            }
        ));
    }

    #[test]
    fn malformed_requests_are_refused_without_guessing() {
        for (query, error) in [
            ("action=volume&deck=1&value=16384", RemoteError::Value),
            ("action=volume&deck=1", RemoteError::Value),
            ("action=volume&deck=3&value=1", RemoteError::Deck),
            ("action=cue", RemoteError::Deck),
            ("action=cue&deck=01", RemoteError::Deck),
            ("action=eject&deck=1", RemoteError::UnknownAction),
            ("deck=1", RemoteError::UnknownAction),
            ("action=seek&deck=1&value=-5", RemoteError::Value),
            ("action=load&deck=1&track=0&revision=3", RemoteError::Track),
            ("action=load&deck=1&track=7", RemoteError::Track),
            ("action=cue&deck=1&password=x", RemoteError::SecretInQuery),
        ] {
            assert_eq!(parse_remote(query), Err(error), "{query}");
        }
        assert_eq!(
            parse_remote("action=load&deck=2&track=42&revision=3"),
            Ok(RemoteCommand::Load {
                deck: DeckId::Two,
                track: 42,
                revision: 3
            })
        );
        assert_eq!(
            parse_remote("action=seek&deck=1&value=90000"),
            Ok(RemoteCommand::Seek {
                deck: DeckId::One,
                position_ms: 90_000
            })
        );
        assert_eq!(clamp_seek_ms(90_000, Some(60_000)), 60_000);
        assert_eq!(clamp_seek_ms(90_000, None), 90_000);
        assert_eq!(clamp_seek_ms(90_000, Some(0)), 90_000);
    }
}
