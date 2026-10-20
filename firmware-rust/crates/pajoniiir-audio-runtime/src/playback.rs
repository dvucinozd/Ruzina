//! Serialized output/transport owner. MAIN is authority when available,
//! otherwise controller headphones; no output means no confirmed progression.
use crate::{
    pcm_stream::{PcmStreamProgress, PendingRenderProgress},
    render::{RENDER_BLOCK_FRAMES, RenderOutputBlock},
    routing::OutputRouter,
    sink::{
        BlockProgress, OutputDelivery, SinkBlock, SinkError, SinkLane, SinkReceipt, SinkRole,
        SinkToken,
    },
};
use pajoniiir_audio_dsp::PcmFrame;

#[derive(Clone, Copy)]
pub struct RenderPermit {
    revision: u64,
    main: Option<SinkToken>,
    headphones: Option<SinkToken>,
}

pub struct PlaybackOutput<'a> {
    delivery: OutputDelivery<'a>,
    sources: [u32; 2],
    revision: u64,
    pending: [PendingRenderProgress; 2],
    confirmed: [Option<PcmStreamProgress>; 2],
    published_output_end: u64,
    reference: bool,
}
impl<'a> PlaybackOutput<'a> {
    pub fn new(
        main: &'a mut [SinkBlock],
        headphones: &'a mut [SinkBlock],
    ) -> Result<Self, SinkError> {
        Ok(Self {
            delivery: OutputDelivery::new(main, headphones)?,
            sources: [0; 2],
            revision: 0,
            pending: [PendingRenderProgress::default(); 2],
            confirmed: [None; 2],
            published_output_end: 0,
            reference: false,
        })
    }

    /// Explicit null/reference integration, never an advertised physical DAC
    /// or UAC stream. Guition selectors do not activate this owner.
    pub fn new_reference(
        main: &'a mut [SinkBlock],
        headphones: &'a mut [SinkBlock],
    ) -> Result<Self, SinkError> {
        let mut owner = Self::new(main, headphones)?;
        owner.delivery.main.bind_reference(0)?;
        owner.reference = true;
        Ok(owner)
    }

    pub fn sync_routes(
        &mut self,
        router: &OutputRouter,
        first_frame: u64,
    ) -> Result<(), SinkError> {
        if self.reference {
            return Err(SinkError::Invalid);
        }
        if first_frame < self.published_output_end {
            return Err(SinkError::Order);
        }
        self.delivery.sync_routes(router, first_frame)
    }

    /// Apply before issuing or acknowledging work after prepare/seek/release,
    /// source replacement or media invalidation. Combined PCM contains both
    /// decks, so both queues and outstanding receipts are revoked together.
    pub fn sync_sources(&mut self, sources: [u32; 2], first_frame: u64) -> Result<bool, SinkError> {
        if self.sources == sources {
            return Ok(false);
        }
        if first_frame < self.published_output_end {
            return Err(SinkError::Order);
        }
        self.delivery.main.reset_pcm(first_frame)?;
        self.delivery.headphones.reset_pcm(first_frame)?;
        for (index, generation) in sources.into_iter().enumerate() {
            if self.sources[index] != generation {
                self.pending[index].clear();
                self.confirmed[index] = None;
            }
        }
        self.sources = sources;
        self.revision = self.revision.checked_add(1).ok_or(SinkError::Overflow)?;
        // Output coordinates stay monotonic within this owner. An adapter
        // restarting its clock must also start a new serialized owner session.
        self.published_output_end = self.published_output_end.max(first_frame);
        Ok(true)
    }

    pub fn lane(&self, role: SinkRole) -> &SinkLane<'a> {
        match role {
            SinkRole::Main => &self.delivery.main,
            SinkRole::Headphones => &self.delivery.headphones,
        }
    }
    pub fn lane_mut(&mut self, role: SinkRole) -> &mut SinkLane<'a> {
        match role {
            SinkRole::Main => &mut self.delivery.main,
            SinkRole::Headphones => &mut self.delivery.headphones,
        }
    }
    pub fn authority(&self) -> Option<SinkRole> {
        if self.delivery.main.token().is_some() {
            Some(SinkRole::Main)
        } else if self.delivery.headphones.token().is_some() {
            Some(SinkRole::Headphones)
        } else {
            None
        }
    }
    pub fn next_block_frame(&self) -> Option<u64> {
        self.lane(self.authority()?).next_block_frame()
    }
    pub fn render_permit(&self) -> RenderPermit {
        RenderPermit {
            revision: self.revision,
            main: self.delivery.main.token(),
            headphones: self.delivery.headphones.token(),
        }
    }
    pub fn submit_render(
        &mut self,
        permit: RenderPermit,
        first_frame: u64,
        block: &RenderOutputBlock,
        cue: &[PcmFrame; RENDER_BLOCK_FRAMES],
        progress: BlockProgress,
    ) -> [Result<(), SinkError>; 2] {
        if permit.revision != self.revision {
            return [Err(SinkError::Stale); 2];
        }
        for (index, mark) in progress.decks.into_iter().enumerate() {
            if mark.is_some_and(|mark| {
                mark.generation == 0
                    || mark.generation != self.sources[index]
                    || (mark.sample_rate_hz == 0 && !mark.faulted)
            }) {
                return [Err(SinkError::Stale); 2];
            }
        }
        self.delivery.submit_render_tracked(
            permit.main,
            permit.headphones,
            first_frame,
            block,
            cue,
            progress,
        )
    }

    /// Called only after the adapter has consumed the issued PCM. A secondary
    /// lane frees its receipts but cannot publish the same source progress.
    pub fn complete(&mut self, receipt: SinkReceipt) -> Result<(), SinkError> {
        let role = receipt.role();
        self.lane_mut(role).complete(receipt)?;
        if self.authority() != Some(role) {
            return Ok(());
        }
        let Some(checkpoint) = receipt.checkpoint else {
            return Ok(());
        };
        if checkpoint.output_end <= self.published_output_end {
            return Ok(());
        }
        self.published_output_end = checkpoint.output_end;
        for (index, mark) in checkpoint.progress.decks.into_iter().enumerate() {
            if let Some(mark) =
                mark.filter(|mark| mark.generation != 0 && mark.generation == self.sources[index])
            {
                self.confirmed[index] = Some(mark);
                self.pending[index].record(mark, checkpoint.progress.source_frames[index]);
            }
        }
        Ok(())
    }
    pub fn pending_progress(&self, deck: usize) -> Option<(PcmStreamProgress, u16)> {
        self.pending.get(deck)?.peek()
    }
    pub fn clear_progress(&mut self, deck: usize) {
        if let Some(pending) = self.pending.get_mut(deck) {
            pending.clear();
        }
    }
    pub fn confirmed_progress(&self, deck: usize) -> Option<PcmStreamProgress> {
        self.confirmed.get(deck).copied().flatten()
    }
}
