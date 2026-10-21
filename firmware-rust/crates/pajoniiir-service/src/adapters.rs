//! Opt-in, synchronous owner reads. Call under the owner's serialization in the
//! shared monotonic clock domain. Activity is owner intent, not inferred from
//! historical counters. Rebind the receiving Slot on owner reset/replacement;
//! sink adapters must also rebind when the captured route/PCM token changes.
//! Raw lifetime counters preserve idle, muted and active underrun distinctions;
//! a nonzero historical counter is not a current fault declaration.
use crate::publication::{Publication, Stamp};
use pajoniiir_audio_runtime::{
    render::{FixedRateRenderGraph, RenderHealth},
    sink::{SinkLane, SinkStats, SinkToken},
};
use pajoniiir_control_scheduler::{EventScheduler, SchedulerStats};

pub fn scheduler(owner: &EventScheduler, stamp: Stamp) -> Publication<SchedulerStats> {
    Publication {
        stamp,
        value: owner.stats(),
    }
}
pub fn render(owner: &FixedRateRenderGraph, stamp: Stamp) -> Publication<RenderHealth> {
    Publication {
        stamp,
        value: owner.health(),
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SinkObservation {
    pub token: Option<SinkToken>,
    pub stats: SinkStats,
    pub queued_blocks: usize,
    pub clock_frame: u64,
}
pub fn sink(owner: &SinkLane<'_>, stamp: Stamp) -> Publication<SinkObservation> {
    Publication {
        stamp,
        value: SinkObservation {
            token: owner.token(),
            stats: owner.stats(),
            queued_blocks: owner.queued_blocks(),
            clock_frame: owner.clock_frame(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publication::{Activity, Freshness, Slot};
    use pajoniiir_audio_runtime::{
        render::{RenderControl, RenderOutputBlock},
        sink::{SinkBlock, SinkRole},
    };
    use pajoniiir_control_scheduler::DISCRETE_FIFO_CAPACITY;
    use pajoniiir_core::DeckId;

    fn stamp() -> Stamp {
        Stamp {
            generation: 1,
            sequence: 1,
            sampled_ms: 10,
            activity: Activity::Idle,
        }
    }
    #[test]
    fn renderer_idle_and_real_source_starvation_are_separate_from_freshness() {
        let mut owner = FixedRateRenderGraph::new();
        let mut output = RenderOutputBlock::silence();
        owner.render_block(RenderControl::idle(), &mut output, |_| None);
        let idle = render(&owner, stamp());
        assert_eq!(idle.value.rendered_blocks, 1);
        assert_eq!(idle.value.underflow_frames, [0; 2]);
        let mut control = RenderControl::idle();
        control.decks[0].playing = true;
        owner.render_block(control, &mut output, |_| None);
        let active = render(
            &owner,
            Stamp {
                activity: Activity::Active,
                sequence: 2,
                ..stamp()
            },
        );
        assert!(active.value.underflow_frames[0] > 0);
        let mut slot = Slot::new(100);
        assert!(slot.bind(1));
        slot.accept(active, 10).unwrap();
        assert_eq!(slot.observation(110).freshness, Freshness::Fresh);
        assert_eq!(slot.observation(111).freshness, Freshness::Stale);
        assert_eq!(slot.latest().unwrap().value, owner.health());
    }
    #[test]
    fn unbound_sink_has_no_output_token_and_scheduler_counters_are_raw() {
        let mut storage = [SinkBlock::EMPTY; 1];
        let owner = SinkLane::new(SinkRole::Main, &mut storage).unwrap();
        let captured = sink(&owner, stamp());
        assert!(captured.value.token.is_none());
        assert_eq!(captured.value.stats.underrun_frames, 0);
        assert_eq!(captured.value.queued_blocks, 0);
        let mut owner = EventScheduler::new();
        // An actual owner queue saturation, not fabricated health counters.
        let event = pajoniiir_controller_core::ControlEvent {
            deck: Some(DeckId::One),
            control: pajoniiir_controller_core::SemanticControl::Cue,
            value: pajoniiir_controller_core::ControlValue::Pressed(true),
        };
        for _ in 0..DISCRETE_FIFO_CAPACITY {
            assert!(owner.enqueue_discrete(event));
        }
        assert!(!owner.enqueue_discrete(event));
        let captured = scheduler(&owner, stamp());
        assert_eq!(captured.value.fifo_full, 1);
        assert_eq!(captured.value, owner.stats());
    }
    #[test]
    fn sink_capture_preserves_idle_active_loss_and_pcm_reset_identity() {
        use pajoniiir_audio_runtime::routing::{
            ChannelPair, OutputAvailability, OutputRouter, ReadyStream,
        };
        let mut router = OutputRouter::default();
        let transition = router
            .update(OutputAvailability {
                external_dac: Some(ReadyStream {
                    connection_epoch: 1,
                    stream_epoch: 1,
                    interface: 1,
                    channels: 2,
                    pair: ChannelPair { left: 0, right: 1 },
                    sample_rate_hz: 48_000,
                    sample_bits: 16,
                }),
                ..OutputAvailability::default()
            })
            .unwrap();
        assert!(router.commit_prepared(transition));
        let mut storage = [SinkBlock::EMPTY; 1];
        let mut owner = SinkLane::new(SinkRole::Main, &mut storage).unwrap();
        owner.sync_route(&router, 0).unwrap();
        let token = owner.token().unwrap();
        let mut output = RenderOutputBlock::silence();
        owner.consume(token, 0, 128, &mut output.main).unwrap();
        assert_eq!(sink(&owner, stamp()).value.stats.muted_frames, 128);
        owner.set_mute(token, false, 0).unwrap();
        owner.consume(token, 128, 128, &mut output.main).unwrap();
        let idle = sink(&owner, stamp());
        assert_eq!(idle.value.stats.idle_frames, 128);
        assert_eq!(idle.value.stats.underrun_frames, 0);
        owner.expect_active(token, true).unwrap();
        owner.consume(token, 256, 128, &mut output.main).unwrap();
        let active = sink(
            &owner,
            Stamp {
                activity: Activity::Active,
                sequence: 2,
                ..stamp()
            },
        );
        assert_eq!(active.value.stats.underrun_frames, 128);
        assert_eq!(active.value.stats.idle_frames, 128);
        let mut slot = Slot::new(100);
        assert!(slot.bind(1));
        slot.accept(active, 10).unwrap();
        owner.reset_pcm(0).unwrap();
        let reset = sink(
            &owner,
            Stamp {
                generation: 2,
                sequence: 1,
                ..stamp()
            },
        );
        assert_ne!(reset.value.token, active.value.token);
        assert!(slot.bind(2));
        assert_eq!(
            slot.accept(active, 10),
            Err(crate::publication::Rejected::Generation)
        );
        slot.accept(reset, 10).unwrap();
        assert_eq!(slot.latest().unwrap().value.stats, owner.stats());
    }
}
