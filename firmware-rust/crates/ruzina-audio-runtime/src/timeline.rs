//! Monotonic decoded PCM coordinates with a possibly repeating media timeline.
//! A loop edit/seek creates a new generation; a steady loop wrap does not.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameLoop {
    start: u64,
    end: u64,
}

impl FrameLoop {
    pub fn new(start: u64, end: u64, total_frames: u64) -> Option<Self> {
        (start < end && end <= total_frames).then_some(Self { start, end })
    }

    pub fn from_ms(start: u32, end: u32, sample_rate: u32, total: u64) -> Option<Self> {
        if sample_rate == 0 {
            return None;
        }
        let start = u64::from(start) * u64::from(sample_rate) / 1000;
        let end = u64::from(end) * u64::from(sample_rate) / 1000;
        Self::new(start, end, total)
    }

    pub const fn start(self) -> u64 {
        self.start
    }
    pub const fn end(self) -> u64 {
        self.end
    }

    pub fn normalize(self, frame: u64) -> u64 {
        if frame < self.end {
            frame
        } else {
            self.start + (frame - self.start) % (self.end - self.start)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PcmTimeline {
    origin: u64,
    loop_region: Option<FrameLoop>,
}

impl PcmTimeline {
    pub const fn linear() -> Self {
        Self {
            origin: 0,
            loop_region: None,
        }
    }

    /// The first monotonic PCM coordinate equals the normalized media position.
    pub fn at(frame: u64, loop_region: Option<FrameLoop>) -> Self {
        Self {
            origin: loop_region.map_or(frame, |region| region.normalize(frame)),
            loop_region,
        }
    }

    pub const fn origin(self) -> u64 {
        self.origin
    }
    pub const fn loop_region(self) -> Option<FrameLoop> {
        self.loop_region
    }

    pub fn media_frame(self, pcm_frame: u64) -> Option<u64> {
        let delta = pcm_frame.checked_sub(self.origin)?;
        let Some(region) = self.loop_region else {
            return self.origin.checked_add(delta);
        };
        let prefix = region.end - self.origin;
        if delta < prefix {
            self.origin.checked_add(delta)
        } else {
            Some(region.start + (delta - prefix) % (region.end - region.start))
        }
    }
}
