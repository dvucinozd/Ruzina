//! Bounded PCM delivery driven by consumer frame requests, not wall-clock sleeps.
//! A driver serializes access, supplies its consumed-frame clock, and copies PCM
//! into DMA/UAC-owned storage. No hardware ownership or clock correction lives here.
use crate::{
    pcm_stream::PcmStreamProgress,
    render::{RENDER_BLOCK_FRAMES, RenderOutputBlock},
    routing::{HeadphoneRoute, MainRoute, OutputRouter},
};
use core::sync::atomic::{AtomicU32, Ordering};
use pajoniiir_audio_dsp::PcmFrame;

// Constructor identities are never reused in a process/boot, including when
// an adapter recreates an owner and its frame clock starts from zero again.
static NEXT_LANE_INSTANCE: AtomicU32 = AtomicU32::new(1);

pub const MAX_SINK_BLOCKS: usize = 8;
pub const MAX_FADE_FRAMES: u32 = 48_000;
pub const MAX_IN_FLIGHT_RECEIPTS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SinkRole {
    Main,
    Headphones,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Binding {
    Main(MainRoute),
    Headphones(HeadphoneRoute),
    Reference,
}

/// Invalidated by route changes, explicit PCM reset, or consumer-clock restart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SinkToken {
    instance: u32,
    role: SinkRole,
    revision: u64,
    binding: Binding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SinkError {
    Capacity,
    Stale,
    Full,
    Late,
    Order,
    Invalid,
    Overflow,
    InFlightFull,
}

/// Conservative block-boundary transport snapshots, captured by render but
/// published only after completion of the output carrying the whole block.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BlockProgress {
    pub decks: [Option<PcmStreamProgress>; 2],
    pub source_frames: [u16; 2],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Checkpoint {
    pub output_end: u64,
    pub progress: BlockProgress,
}

/// Issued PCM is not consumed PCM. Drivers retain this opaque ticket with their
/// DMA/UAC buffer and return it only on completion, in issue order per lane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SinkReceipt {
    pub(crate) token: SinkToken,
    pub(crate) first: u64,
    pub(crate) end: u64,
    pub(crate) checkpoint: Option<Checkpoint>,
}
impl SinkReceipt {
    pub const fn role(&self) -> SinkRole {
        self.token.role
    }
}

#[derive(Clone, Copy)]
pub struct SinkBlock {
    first_frame: u64,
    frames: [PcmFrame; RENDER_BLOCK_FRAMES],
    progress: BlockProgress,
}
impl SinkBlock {
    pub const EMPTY: Self = Self {
        first_frame: 0,
        frames: [PcmFrame { left: 0, right: 0 }; RENDER_BLOCK_FRAMES],
        progress: BlockProgress {
            decks: [None; 2],
            source_frames: [0; 2],
        },
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SinkStats {
    pub clock_frames: u64,
    pub pcm_frames: u64,
    pub underrun_frames: u64,
    pub underrun_events: u64,
    pub idle_frames: u64,
    pub muted_frames: u64,
    pub late_blocks: u64,
    pub full_blocks: u64,
    pub discarded_frames: u64,
    pub stale_calls: u64,
    pub order_errors: u64,
    pub high_water_blocks: usize,
    pub completed_frames: u64,
    pub in_flight_full: u64,
    pub completion_errors: u64,
    pub revoked_in_flight_frames: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Fade {
    level: i32,
    start: i32,
    target: i32,
    elapsed: u32,
    duration: u32,
}
impl Fade {
    fn set(&mut self, mute: bool, duration: u32) {
        self.start = self.level;
        self.target = if mute { 0 } else { 32_768 };
        self.elapsed = 0;
        self.duration = duration;
        if duration == 0 {
            self.level = self.target;
        }
    }
    fn next(&mut self) -> i32 {
        if self.elapsed < self.duration {
            self.elapsed += 1;
            self.level = self.start
                + ((i64::from(self.target - self.start) * i64::from(self.elapsed))
                    / i64::from(self.duration)) as i32;
        }
        self.level
    }
}

pub struct SinkLane<'a> {
    instance: u32,
    role: SinkRole,
    storage: &'a mut [SinkBlock],
    head: usize,
    count: usize,
    token: Option<SinkToken>,
    revision: u64,
    clock_frame: u64,
    route_revision: u64,
    expected_active: bool,
    starved: bool,
    fade: Fade,
    stats: SinkStats,
    completed_frame: u64,
    in_flight: usize,
}

impl<'a> SinkLane<'a> {
    pub fn new(role: SinkRole, storage: &'a mut [SinkBlock]) -> Result<Self, SinkError> {
        if storage.is_empty() || storage.len() > MAX_SINK_BLOCKS {
            return Err(SinkError::Capacity);
        }
        let instance = NEXT_LANE_INSTANCE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map_err(|_| SinkError::Overflow)?;
        Ok(Self {
            instance,
            role,
            storage,
            head: 0,
            count: 0,
            token: None,
            revision: 0,
            clock_frame: 0,
            route_revision: 0,
            expected_active: false,
            starved: false,
            fade: Fade::default(),
            stats: SinkStats::default(),
            completed_frame: 0,
            in_flight: 0,
        })
    }

    /// Observe committed routes after every availability update, including the
    /// invalidated interval before driver preparation is acknowledged. Unchanged
    /// headphone routes retain their token, queued PCM, clock and envelope.
    pub fn sync_route(&mut self, router: &OutputRouter, first_frame: u64) -> Result<(), SinkError> {
        let routes = router.active();
        let binding = match self.role {
            SinkRole::Main if routes.main != MainRoute::Muted => Some(Binding::Main(routes.main)),
            SinkRole::Headphones if routes.headphones != HeadphoneRoute::Unavailable => {
                Some(Binding::Headphones(routes.headphones))
            }
            _ => None,
        };
        let revision = match self.role {
            SinkRole::Main => router.main_revision(),
            SinkRole::Headphones => router.headphone_revision(),
        };
        if self.token.map(|token| token.binding) == binding && self.route_revision == revision {
            return Ok(());
        }
        self.replace_binding(binding, first_frame)?;
        self.route_revision = revision;
        Ok(())
    }

    /// Seek/source replacement and driver restarts must flush already-rendered
    /// PCM too. A surviving physical stream does not make old PCM current.
    pub fn reset_pcm(&mut self, first_frame: u64) -> Result<(), SinkError> {
        self.replace_binding(self.token.map(|token| token.binding), first_frame)
    }

    pub(crate) fn bind_reference(&mut self, first_frame: u64) -> Result<(), SinkError> {
        if self.role != SinkRole::Main {
            return Err(SinkError::Invalid);
        }
        self.replace_binding(Some(Binding::Reference), first_frame)
    }

    fn replace_binding(
        &mut self,
        binding: Option<Binding>,
        first_frame: u64,
    ) -> Result<(), SinkError> {
        self.stats.discarded_frames = self
            .stats
            .discarded_frames
            .saturating_add(self.queued_frames());
        self.head = 0;
        self.count = 0;
        self.token = None;
        self.expected_active = false;
        self.starved = false;
        self.fade = Fade::default();
        self.stats.revoked_in_flight_frames = self
            .stats
            .revoked_in_flight_frames
            .saturating_add(self.clock_frame.saturating_sub(self.completed_frame));
        self.clock_frame = first_frame;
        self.completed_frame = first_frame;
        self.in_flight = 0;
        self.revision = self.revision.checked_add(1).ok_or(SinkError::Overflow)?;
        self.token = binding.map(|binding| SinkToken {
            instance: self.instance,
            role: self.role,
            revision: self.revision,
            binding,
        });
        Ok(())
    }

    pub const fn token(&self) -> Option<SinkToken> {
        self.token
    }
    pub const fn clock_frame(&self) -> u64 {
        self.clock_frame
    }
    pub const fn queued_blocks(&self) -> usize {
        self.count
    }
    /// Issued buffers whose completion has not returned.
    pub const fn in_flight(&self) -> usize {
        self.in_flight
    }
    pub const fn stats(&self) -> SinkStats {
        self.stats
    }
    pub fn is_muted(&self) -> bool {
        self.fade.level == 0 && self.fade.target == 0
    }

    fn check(&mut self, token: SinkToken) -> Result<(), SinkError> {
        if self.token == Some(token) {
            Ok(())
        } else {
            self.stats.stale_calls = self.stats.stale_calls.saturating_add(1);
            Err(SinkError::Stale)
        }
    }

    pub fn expect_active(&mut self, token: SinkToken, active: bool) -> Result<(), SinkError> {
        self.check(token)?;
        self.expected_active = active;
        if !active {
            self.starved = false;
        }
        Ok(())
    }

    /// Newly bound streams start muted. Prime their queue before fade-in.
    /// Planned switches can fade/drain the old stream before changing routes;
    /// detach invalidates and silences it immediately instead.
    pub fn set_mute(&mut self, token: SinkToken, mute: bool, frames: u32) -> Result<(), SinkError> {
        self.check(token)?;
        if frames > MAX_FADE_FRAMES {
            return Err(SinkError::Invalid);
        }
        self.fade.set(mute, frames);
        Ok(())
    }

    /// The producer checks this BEFORE rendering. Full queues cause no source
    /// consumption. A caller fanning out to multiple lanes chooses one clock
    /// authority; each other lane still rejects full/late packets independently.
    pub fn next_block_frame(&self) -> Option<u64> {
        self.token?;
        if self.count == self.storage.len() {
            return None;
        }
        if self.count != 0 {
            let tail = (self.head + self.count - 1) % self.storage.len();
            return self.storage[tail]
                .first_frame
                .checked_add(RENDER_BLOCK_FRAMES as u64);
        }
        let remainder = self.clock_frame % RENDER_BLOCK_FRAMES as u64;
        self.clock_frame.checked_add(if remainder == 0 {
            0
        } else {
            RENDER_BLOCK_FRAMES as u64 - remainder
        })
    }

    pub fn submit(
        &mut self,
        token: SinkToken,
        first_frame: u64,
        frames: &[PcmFrame; RENDER_BLOCK_FRAMES],
    ) -> Result<(), SinkError> {
        self.submit_tracked(token, first_frame, frames, BlockProgress::default())
    }

    pub fn submit_tracked(
        &mut self,
        token: SinkToken,
        first_frame: u64,
        frames: &[PcmFrame; RENDER_BLOCK_FRAMES],
        progress: BlockProgress,
    ) -> Result<(), SinkError> {
        self.check(token)?;
        first_frame
            .checked_add(RENDER_BLOCK_FRAMES as u64)
            .ok_or(SinkError::Overflow)?;
        if first_frame < self.clock_frame {
            self.stats.late_blocks = self.stats.late_blocks.saturating_add(1);
            return Err(SinkError::Late);
        }
        if self.count != 0 {
            let tail = (self.head + self.count - 1) % self.storage.len();
            if first_frame < self.storage[tail].first_frame + RENDER_BLOCK_FRAMES as u64 {
                self.stats.order_errors = self.stats.order_errors.saturating_add(1);
                return Err(SinkError::Order);
            }
        }
        if self.count == self.storage.len() {
            self.stats.full_blocks = self.stats.full_blocks.saturating_add(1);
            return Err(SinkError::Full);
        }
        let tail = (self.head + self.count) % self.storage.len();
        self.storage[tail] = SinkBlock {
            first_frame,
            frames: *frames,
            progress,
        };
        self.count += 1;
        self.stats.high_water_blocks = self.stats.high_water_blocks.max(self.count);
        Ok(())
    }

    fn queued_frames(&self) -> u64 {
        (0..self.count)
            .map(|index| {
                let block = &self.storage[(self.head + index) % self.storage.len()];
                (block.first_frame + RENDER_BLOCK_FRAMES as u64)
                    .saturating_sub(self.clock_frame.max(block.first_frame))
            })
            .sum()
    }

    /// Immediate-consumer compatibility helper for simulations. Physical
    /// adapters must use issue/complete and acknowledge on actual completion.
    pub fn consume(
        &mut self,
        token: SinkToken,
        first_frame: u64,
        len: usize,
        output: &mut [PcmFrame; RENDER_BLOCK_FRAMES],
    ) -> Result<(), SinkError> {
        if self.in_flight != 0 {
            output.fill(PcmFrame::default());
            return Err(SinkError::Order);
        }
        let receipt = self.issue(token, first_frame, len, output)?;
        self.complete(receipt)
    }

    /// Fill a DMA/UAC-owned buffer. No transport snapshot is confirmed here.
    /// first_frame is the ordered output submission clock. At most eight
    /// completion receipts may be outstanding; exhaustion does not dequeue PCM.
    pub fn issue(
        &mut self,
        token: SinkToken,
        first_frame: u64,
        len: usize,
        output: &mut [PcmFrame; RENDER_BLOCK_FRAMES],
    ) -> Result<SinkReceipt, SinkError> {
        output.fill(PcmFrame::default());
        self.check(token)?;
        if self.in_flight == MAX_IN_FLIGHT_RECEIPTS {
            self.stats.in_flight_full = self.stats.in_flight_full.saturating_add(1);
            return Err(SinkError::InFlightFull);
        }
        if len == 0 || len > RENDER_BLOCK_FRAMES {
            return Err(SinkError::Invalid);
        }
        let end = first_frame
            .checked_add(len as u64)
            .ok_or(SinkError::Overflow)?;
        if first_frame != self.clock_frame {
            self.stats.order_errors = self.stats.order_errors.saturating_add(1);
            return Err(SinkError::Order);
        }
        let mut checkpoint = None;
        for (index, output_frame) in output[..len].iter_mut().enumerate() {
            let frame = first_frame + index as u64;
            while self.count != 0
                && self.storage[self.head].first_frame + RENDER_BLOCK_FRAMES as u64 <= frame
            {
                self.head = (self.head + 1) % self.storage.len();
                self.count -= 1;
            }
            let pcm = if self.count != 0 && self.storage[self.head].first_frame <= frame {
                Some(
                    self.storage[self.head].frames
                        [(frame - self.storage[self.head].first_frame) as usize],
                )
            } else {
                None
            };
            let gain = self.fade.next();
            let muted = gain == 0 && self.fade.target == 0;
            let underrun = pcm.is_none() && self.expected_active && !muted;
            if underrun {
                self.stats.underrun_frames = self.stats.underrun_frames.saturating_add(1);
                if !self.starved {
                    self.stats.underrun_events = self.stats.underrun_events.saturating_add(1);
                }
            } else if muted {
                self.stats.muted_frames = self.stats.muted_frames.saturating_add(1);
            } else if pcm.is_none() {
                self.stats.idle_frames = self.stats.idle_frames.saturating_add(1);
            }
            self.starved = underrun;
            if let Some(pcm) = pcm {
                if frame + 1 == self.storage[self.head].first_frame + RENDER_BLOCK_FRAMES as u64 {
                    checkpoint = Some(Checkpoint {
                        output_end: frame + 1,
                        progress: self.storage[self.head].progress,
                    });
                }
                self.stats.pcm_frames = self.stats.pcm_frames.saturating_add(1);
                *output_frame = PcmFrame {
                    left: scale(pcm.left, gain),
                    right: scale(pcm.right, gain),
                };
            }
        }
        self.clock_frame = end;
        // Release a block ending exactly at the callback boundary immediately.
        while self.count != 0
            && self.storage[self.head].first_frame + RENDER_BLOCK_FRAMES as u64 <= end
        {
            self.head = (self.head + 1) % self.storage.len();
            self.count -= 1;
        }
        self.stats.clock_frames = self.stats.clock_frames.saturating_add(len as u64);
        self.in_flight += 1;
        Ok(SinkReceipt {
            token,
            first: first_frame,
            end,
            checkpoint,
        })
    }

    pub fn complete(&mut self, receipt: SinkReceipt) -> Result<(), SinkError> {
        self.check(receipt.token)?;
        if self.in_flight == 0
            || receipt.first != self.completed_frame
            || receipt.end > self.clock_frame
        {
            self.stats.completion_errors = self.stats.completion_errors.saturating_add(1);
            return Err(SinkError::Order);
        }
        self.in_flight -= 1;
        self.completed_frame = receipt.end;
        self.stats.completed_frames = self
            .stats
            .completed_frames
            .saturating_add(receipt.end - receipt.first);
        Ok(())
    }
}

fn scale(sample: i16, gain: i32) -> i16 {
    (i32::from(sample) * gain / 32_768) as i16
}

/// Independent bounded MAIN and controller-only headphone lanes. This is not a
/// headphone mixer: the caller supplies the agreed cue mix after composing PFL.
pub struct OutputDelivery<'a> {
    pub main: SinkLane<'a>,
    pub headphones: SinkLane<'a>,
}
impl<'a> OutputDelivery<'a> {
    pub fn new(
        main: &'a mut [SinkBlock],
        headphones: &'a mut [SinkBlock],
    ) -> Result<Self, SinkError> {
        Ok(Self {
            main: SinkLane::new(SinkRole::Main, main)?,
            headphones: SinkLane::new(SinkRole::Headphones, headphones)?,
        })
    }
    pub fn sync_routes(
        &mut self,
        router: &OutputRouter,
        first_frame: u64,
    ) -> Result<(), SinkError> {
        self.main.sync_route(router, first_frame)?;
        self.headphones.sync_route(router, first_frame)
    }
    /// Fanout is deliberately non-transactional: a blocked MAIN lane cannot
    /// suppress controller headphones, or vice versa. Inspect both results.
    pub fn submit_render(
        &mut self,
        main: Option<SinkToken>,
        headphones: Option<SinkToken>,
        first_frame: u64,
        block: &RenderOutputBlock,
        cue: &[PcmFrame; RENDER_BLOCK_FRAMES],
    ) -> [Result<(), SinkError>; 2] {
        self.submit_render_tracked(
            main,
            headphones,
            first_frame,
            block,
            cue,
            BlockProgress::default(),
        )
    }

    pub fn submit_render_tracked(
        &mut self,
        main: Option<SinkToken>,
        headphones: Option<SinkToken>,
        first_frame: u64,
        block: &RenderOutputBlock,
        cue: &[PcmFrame; RENDER_BLOCK_FRAMES],
        progress: BlockProgress,
    ) -> [Result<(), SinkError>; 2] {
        if block.len as usize != RENDER_BLOCK_FRAMES {
            return [Err(SinkError::Invalid); 2];
        }
        [
            main.ok_or(SinkError::Stale).and_then(|token| {
                self.main
                    .submit_tracked(token, first_frame, &block.main, progress)
            }),
            headphones.ok_or(SinkError::Stale).and_then(|token| {
                self.headphones
                    .submit_tracked(token, first_frame, cue, progress)
            }),
        ]
    }
}
