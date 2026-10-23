//! Provider-neutral bounded observations, independent of the optional worker.
//! A transition timestamp is not a heartbeat, progress estimate or acceptance.
use core::fmt::{self, Write};
use ruzina_core::DeckId;
macro_rules! labels {
    ($name:ident {$($variant:ident => $label:literal),+ $(,)?}) => {
        #[derive(Clone,Copy,Debug,Eq,PartialEq)]
        pub enum $name {$($variant),+}
        impl $name {
            pub const fn label(self)->&'static str {match self {$(Self::$variant=>$label),+}}
            fn parse(value:&str)->Option<Self> {match value {$($label=>Some(Self::$variant)),+,_=>None}}
        }
    }
}
labels!(State {Disabled=>"DISABLED",Waiting=>"WAITING",Analyzing=>"ANALYZING",Paused=>"PAUSED",Deferred=>"DEFERRED",Published=>"PUBLISHED",Rejected=>"REJECTED",Failed=>"FAILED",RecoveryBlocked=>"RECOVERY_BLOCKED",Quarantined=>"QUARANTINED"});
labels!(Reason {None=>"NONE",NotSelected=>"NOT_SELECTED",Startup=>"STARTUP",Allocation=>"ALLOCATION",NoMedia=>"NO_MEDIA",NoWork=>"NO_WORK",Queued=>"QUEUED",PlaybackOrTransport=>"PLAYBACK_OR_TRANSPORT",QueueFull=>"QUEUE_FULL",Recycle=>"RECYCLE",Stale=>"STALE",Rejected=>"REJECTED",Storage=>"STORAGE",Format=>"FORMAT",Capacity=>"CAPACITY",WorkLimit=>"WORK_LIMIT",Contract=>"CONTRACT",Analysis=>"ANALYSIS",Exhausted=>"EXHAUSTED",NoBank=>"NO_BANK",UnexpectedCompletion=>"UNEXPECTED_COMPLETION"});
impl Reason {
    pub const fn help(self) -> &'static str {
        match self {
            Self::NotSelected => "Native analysis is not selected in this configuration.",
            Self::Startup => "Waiting for service startup and mounted media.",
            Self::Allocation => "Startup allocation failed; inspect memory before restarting.",
            Self::NoMedia => "Mount media and explicitly reload the intended track.",
            Self::NoWork => "No eligible WAV load is waiting; existing analysis keeps priority.",
            Self::Queued => "Waiting for the persistent analysis owner.",
            Self::PlaybackOrTransport => {
                "Stop both decks and wait for transport completion. Active work is cancelled."
            }
            Self::QueueFull | Self::Recycle | Self::NoBank => {
                "Waiting for bounded queue/buffer ownership to drain."
            }
            Self::Rejected => {
                "Request or result was refused; inspect identity and queue/bank ownership."
            }
            Self::Stale => "Result belongs to an older load, media or authority and was rejected.",
            Self::Storage => "Storage operation failed; inspect media and recovery status.",
            Self::Format => "Source framing/format is unsupported or invalid.",
            Self::Capacity => "Source or result exceeds the fixed analysis storage capacity.",
            Self::WorkLimit => "Source exceeds the configured object, duration or work limit.",
            Self::Contract => "Ownership/backend contract failed; inspect the captured evidence.",
            Self::Analysis => "Analysis could not produce a representable result.",
            Self::Exhausted => {
                "A generation/diagnostic counter is exhausted; restart only after investigation."
            }
            Self::UnexpectedCompletion => {
                "Unexpected completion is retained. Do not discard or reissue the endpoint."
            }
            Self::None => "No additional failure reason. No progress percentage is inferred.",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sample {
    pub state: State,
    pub reason: Reason,
    pub deck: Option<DeckId>,
    pub load_generation: Option<u32>,
}
impl Sample {
    pub const DISABLED: Self = Self {
        state: State::Disabled,
        reason: Reason::NotSelected,
        deck: None,
        load_generation: None,
    };
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Observation {
    pub sample: Sample,
    pub serial: u64,
    pub changed_ms: u64,
}
impl Observation {
    pub const DISABLED: Self = Self {
        sample: Sample::DISABLED,
        serial: 0,
        changed_ms: 0,
    };
    pub fn write_to(self, out: &mut impl Write) -> fmt::Result {
        writeln!(out, "M1_ANALYSIS_OBSERVATION_V1")?;
        writeln!(out, "state={}", self.sample.state.label())?;
        writeln!(out, "reason={}", self.sample.reason.label())?;
        writeln!(
            out,
            "deck={}",
            match self.sample.deck {
                None => "NONE",
                Some(DeckId::One) => "1",
                Some(DeckId::Two) => "2",
            }
        )?;
        match self.sample.load_generation {
            Some(v) => writeln!(out, "load_generation={v}")?,
            None => writeln!(out, "load_generation=NONE")?,
        }
        writeln!(out, "serial={}", self.serial)?;
        writeln!(out, "changed_ms={}", self.changed_ms)?;
        writeln!(out, "readiness=NOT_QUALIFIED")
    }
    /// One complete bounded record only. Reject duplicate/missing/unknown fields,
    /// unrecognized enums, zero load identities and appended partial records.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        if text.len() > 1024 {
            return Err(ParseError);
        }
        let mut lines = text.lines();
        if lines.next() != Some("M1_ANALYSIS_OBSERVATION_V1") {
            return Err(ParseError);
        }
        let mut fields = [None; 7];
        for line in lines {
            let (key, value) = line.split_once('=').ok_or(ParseError)?;
            let index = match key {
                "state" => 0,
                "reason" => 1,
                "deck" => 2,
                "load_generation" => 3,
                "serial" => 4,
                "changed_ms" => 5,
                "readiness" => 6,
                _ => return Err(ParseError),
            };
            if fields[index].replace(value).is_some() {
                return Err(ParseError);
            }
        }
        let [
            Some(state),
            Some(reason),
            Some(deck),
            Some(generation),
            Some(serial),
            Some(changed),
            Some("NOT_QUALIFIED"),
        ] = fields
        else {
            return Err(ParseError);
        };
        let deck = match deck {
            "NONE" => None,
            "1" => Some(DeckId::One),
            "2" => Some(DeckId::Two),
            _ => return Err(ParseError),
        };
        let load_generation = if generation == "NONE" {
            None
        } else {
            Some({
                let value = number::<u32>(generation)?;
                if value == 0 {
                    return Err(ParseError);
                }
                value
            })
        };
        if deck.is_some() != load_generation.is_some() {
            return Err(ParseError);
        }
        Ok(Self {
            sample: Sample {
                state: State::parse(state).ok_or(ParseError)?,
                reason: Reason::parse(reason).ok_or(ParseError)?,
                deck,
                load_generation,
            },
            serial: number(serial)?,
            changed_ms: number(changed)?,
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseError;
fn number<T: core::str::FromStr>(value: &str) -> Result<T, ParseError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ParseError);
    }
    value.parse().map_err(|_| ParseError)
}
pub struct Monitor(Observation);
impl Monitor {
    pub const fn new() -> Self {
        Self(Observation::DISABLED)
    }
    pub fn observe(&mut self, sample: Sample, now_ms: u64) {
        if sample != self.0.sample
            && let Some(serial) = self.0.serial.checked_add(1)
        {
            self.0 = Observation {
                sample,
                serial,
                changed_ms: now_ms,
            };
        }
    }
    pub const fn snapshot(&self) -> Observation {
        self.0
    }
}
impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    #[test]
    fn snapshot_reads_do_not_renew_transition_and_fault_reason_survives_roundtrip() {
        let mut monitor = Monitor::new();
        let sample = Sample {
            state: State::Quarantined,
            reason: Reason::UnexpectedCompletion,
            deck: Some(DeckId::Two),
            load_generation: Some(7),
        };
        monitor.observe(sample, 12);
        let first = monitor.snapshot();
        monitor.observe(sample, 10000);
        assert_eq!(monitor.snapshot(), first);
        let mut text = std::string::String::new();
        first.write_to(&mut text).unwrap();
        assert_eq!(Observation::parse(&text), Ok(first));
        for invalid in [
            text.replace("QUARANTINED", "GUESS"),
            text.replace("NOT_QUALIFIED", "PASS"),
            text.clone() + "serial=3\n",
            text.replace("load_generation=7", "load_generation=0"),
            text.replace("deck=2", "deck=NONE"),
            text.replace("changed_ms=12", "changed_ms=-1"),
            text.replace("serial=1\n", ""),
        ] {
            assert!(Observation::parse(&invalid).is_err());
        }
        assert!(Observation::parse(&"x".repeat(1025)).is_err());
    }
}
