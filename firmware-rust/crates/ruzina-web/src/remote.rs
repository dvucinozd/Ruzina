//! Web Remote actions as product semantics (M2.5 `/api/control`). A Web
//! action produces the same `ControlEvent`s a controller would, so it passes
//! the same admission (RP-04 LOAD LOCK, RP-03 CUE ownership) and never
//! bypasses the deck core.
//!
//! Buttons are explicit taps, a press followed by its release (RP-03: a CUE
//! tap must not leave a hold-preview running). Continuous values carry the
//! controller's 14-bit range and an ordering key for [`crate::ordering`].
//!
//! M2.5 actions: `play_pause`, `cue`, `pfl`, `sync`, `volume`, `pitch`,
//! `crossfader`, `seek`, `loop_4` (Beat Loop pad 8, four beats) and
//! `loop_clear` (RELOOP stop). Its `main_sink` switched the JC4880 USB/PCM5102A
//! output and has no Guition counterpart. Ruzina adds the mixer the status
//! already reports: `eq_low`, `eq_mid`, `eq_high`, `filter`, `master_volume`,
//! `headphone_mix`, `smart_cfx` and `smart_fader`.

use crate::{HttpStatus, parse};
use ruzina_controller_core::{
    ControlEvent, ControlValue, DeckExtAction, DeckExtActionValue, DeckId, PadAction, PadMode,
    SemanticControl,
};

/// 14-bit absolute range shared with controller profiles (M2.5 0..16383).
pub const REMOTE_ABSOLUTE_MAX: u16 = 0x3fff;

/// Continuous controls ordered by latest value per client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuousControl {
    Volume(DeckId),
    Crossfader,
    Tempo(DeckId),
    /// An EQ band or the filter of one channel.
    Channel(DeckId, SemanticControl),
    /// Master volume or headphone mix.
    Global(SemanticControl),
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
    let tap_with = |deck: Option<DeckId>, control, value: &dyn Fn(bool) -> ControlValue| {
        RemoteCommand::Controls {
            events: [true, false].map(|pressed| {
                Some(ControlEvent {
                    deck,
                    control,
                    value: value(pressed),
                })
            }),
            ordering: None,
        }
    };
    let tap = |deck: DeckId, control| tap_with(Some(deck), control, &ControlValue::Pressed);
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
        // M2.5 `web_queue_loop_set`: pad 8 in Beat Loop mode.
        "loop_4" => tap_with(Some(deck()?), SemanticControl::PadAction, &|pressed| {
            ControlValue::PadAction(PadAction {
                pad: 7,
                mode: PadMode::BeatLoop,
                shifted: false,
                pressed,
            })
        }),
        "loop_clear" => tap_with(Some(deck()?), SemanticControl::DeckExtAction, &|pressed| {
            ControlValue::DeckExtAction(DeckExtActionValue {
                action: DeckExtAction::ReloopStop,
                pressed,
            })
        }),
        "eq_low" | "eq_mid" | "eq_high" | "filter" => {
            let deck = deck()?;
            let control = match action {
                "eq_low" => SemanticControl::EqLow,
                "eq_mid" => SemanticControl::EqMid,
                "eq_high" => SemanticControl::EqHigh,
                _ => SemanticControl::Filter,
            };
            continuous(
                Some(deck),
                control,
                absolute()?,
                ContinuousControl::Channel(deck, control),
            )
        }
        "master_volume" | "headphone_mix" => {
            let control = if action == "master_volume" {
                SemanticControl::MasterVolume
            } else {
                SemanticControl::HeadphoneMix
            };
            continuous(
                None,
                control,
                absolute()?,
                ContinuousControl::Global(control),
            )
        }
        "smart_cfx" => tap_with(None, SemanticControl::SmartCfx, &ControlValue::Pressed),
        "smart_fader" => tap_with(None, SemanticControl::SmartFader, &ControlValue::Pressed),
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
    fn loops_and_the_mixer_use_controller_semantics() {
        let set = events(parse_remote("action=loop_4&deck=2").unwrap());
        assert_eq!(set[0].unwrap().deck, Some(DeckId::Two));
        assert_eq!(
            set[0].unwrap().value,
            ControlValue::PadAction(PadAction {
                pad: 7,
                mode: PadMode::BeatLoop,
                shifted: false,
                pressed: true
            })
        );
        assert!(matches!(
            set[1].unwrap().value,
            ControlValue::PadAction(PadAction { pressed: false, .. })
        ));
        let clear = events(parse_remote("action=loop_clear&deck=1").unwrap());
        assert_eq!(
            clear[0].unwrap().value,
            ControlValue::DeckExtAction(DeckExtActionValue {
                action: DeckExtAction::ReloopStop,
                pressed: true
            })
        );
        let RemoteCommand::Controls {
            events: pair,
            ordering,
        } = parse_remote("action=eq_mid&deck=2&value=0").unwrap()
        else {
            panic!()
        };
        assert_eq!(pair[0].unwrap().control, SemanticControl::EqMid);
        assert_eq!(
            ordering,
            Some(ContinuousControl::Channel(
                DeckId::Two,
                SemanticControl::EqMid
            ))
        );
        let RemoteCommand::Controls {
            events: pair,
            ordering,
        } = parse_remote("action=master_volume&value=9000").unwrap()
        else {
            panic!()
        };
        assert_eq!(pair[0].unwrap().deck, None);
        assert_eq!(
            ordering,
            Some(ContinuousControl::Global(SemanticControl::MasterVolume))
        );
        let smart = events(parse_remote("action=smart_cfx").unwrap());
        assert_eq!(smart[0].unwrap().control, SemanticControl::SmartCfx);
        assert_eq!(smart[1].unwrap().value, ControlValue::Pressed(false));
        for (query, error) in [
            ("action=loop_4", RemoteError::Deck),
            ("action=filter&value=1", RemoteError::Deck),
            ("action=headphone_mix&value=16384", RemoteError::Value),
            ("action=main_sink&value=usb", RemoteError::UnknownAction),
        ] {
            assert_eq!(parse_remote(query), Err(error), "{query}");
        }
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
