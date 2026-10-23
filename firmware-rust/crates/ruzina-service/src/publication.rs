//! One bounded owner publication. All timestamps use the observer's boot/process
//! monotonic millisecond domain; receipt time must never replace sample time.
//! Callers serialize access. This is not a cross-task synchronization primitive.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Activity {
    Idle,
    Active,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Freshness {
    Unbound,
    Awaiting,
    Fresh,
    Stale,
    ClockReversed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stamp {
    pub generation: u64,
    pub sequence: u64,
    pub sampled_ms: u64,
    pub activity: Activity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Publication<T> {
    pub stamp: Stamp,
    pub value: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rejected {
    Generation,
    Sequence,
    Future,
    SampleReversed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Observation {
    pub freshness: Freshness,
    pub generation: Option<u64>,
    pub stamp: Option<Stamp>,
    pub age_ms: Option<u64>,
}

/// Generation is assigned by the coordinator, never adopted from a publisher.
/// Rebind requires a strictly greater, nonzero generation even after unbind.
/// Sequence starts at one, increases strictly and never wraps. Exhaustion needs
/// a new generation; generation exhaustion needs a new boot/process domain.
#[derive(Clone, Copy, Debug)]
pub struct Slot<T> {
    generation: u64,
    bound: bool,
    latest: Option<Publication<T>>,
    max_age_ms: u64,
}
impl<T: Copy> Slot<T> {
    pub const fn new(max_age_ms: u64) -> Self {
        Self {
            generation: 0,
            bound: false,
            latest: None,
            max_age_ms,
        }
    }
    pub fn bind(&mut self, generation: u64) -> bool {
        if generation <= self.generation {
            return false;
        }
        self.generation = generation;
        self.bound = true;
        self.latest = None;
        true
    }
    pub fn unbind(&mut self) {
        self.bound = false;
        self.latest = None;
    }
    /// Rejection never changes the retained sample or its freshness deadline.
    pub fn accept(&mut self, sample: Publication<T>, now_ms: u64) -> Result<(), Rejected> {
        if !self.bound || sample.stamp.generation != self.generation {
            return Err(Rejected::Generation);
        }
        if sample.stamp.sequence == 0
            || self
                .latest
                .is_some_and(|old| sample.stamp.sequence <= old.stamp.sequence)
        {
            return Err(Rejected::Sequence);
        }
        if sample.stamp.sampled_ms > now_ms {
            return Err(Rejected::Future);
        }
        if self
            .latest
            .is_some_and(|old| sample.stamp.sampled_ms < old.stamp.sampled_ms)
        {
            return Err(Rejected::SampleReversed);
        }
        self.latest = Some(sample);
        Ok(())
    }
    /// Last value is retained for inspection, including when stale. Consumers
    /// must inspect observation() before using it as current health.
    pub const fn latest(&self) -> Option<Publication<T>> {
        self.latest
    }
    pub fn observation(&self, now_ms: u64) -> Observation {
        let stamp = self.latest.map(|s| s.stamp);
        let age_ms = stamp.and_then(|s| now_ms.checked_sub(s.sampled_ms));
        let freshness = if !self.bound {
            Freshness::Unbound
        } else if stamp.is_none() {
            Freshness::Awaiting
        } else {
            match age_ms {
                None => Freshness::ClockReversed,
                Some(age) if age > self.max_age_ms => Freshness::Stale,
                Some(_) => Freshness::Fresh,
            }
        };
        Observation {
            freshness,
            generation: self.bound.then_some(self.generation),
            stamp,
            age_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(generation: u64, sequence: u64, sampled_ms: u64) -> Publication<u32> {
        Publication {
            stamp: Stamp {
                generation,
                sequence,
                sampled_ms,
                activity: Activity::Idle,
            },
            value: 42,
        }
    }
    #[test]
    fn idle_heartbeat_expires_at_sample_time_and_recovers() {
        let mut slot = Slot::new(100);
        assert_eq!(slot.observation(0).freshness, Freshness::Unbound);
        assert!(!slot.bind(0));
        assert!(slot.bind(1));
        assert_eq!(slot.observation(0).freshness, Freshness::Awaiting);
        slot.accept(sample(1, 1, 5), 20).unwrap();
        assert_eq!(slot.observation(105).freshness, Freshness::Fresh);
        assert_eq!(slot.observation(106).freshness, Freshness::Stale);
        assert_eq!(slot.observation(4).freshness, Freshness::ClockReversed);
        slot.accept(sample(1, 2, 106), 106).unwrap();
        assert_eq!(slot.observation(106).freshness, Freshness::Fresh);
        assert_eq!(slot.latest().unwrap().stamp.activity, Activity::Idle);
        // A delayed higher sequence doesn't receive a new freshness deadline.
        slot.accept(sample(1, 3, 107), 300).unwrap();
        assert_eq!(slot.observation(300).freshness, Freshness::Stale);
    }
    #[test]
    fn restart_replay_invalid_clocks_and_exhaustion_fail_closed() {
        let mut slot = Slot::new(10);
        assert!(slot.bind(1));
        slot.accept(sample(1, 1, 20), 20).unwrap();
        for (input, reason) in [
            (sample(2, 2, 21), Rejected::Generation),
            (sample(1, 1, 21), Rejected::Sequence),
            (sample(1, 0, 21), Rejected::Sequence),
            (sample(1, 2, 31), Rejected::Future),
            (sample(1, 2, 19), Rejected::SampleReversed),
        ] {
            assert_eq!(slot.accept(input, 30), Err(reason));
            assert_eq!(slot.latest(), Some(sample(1, 1, 20)));
        }
        assert!(!slot.bind(1));
        slot.unbind();
        assert_eq!(slot.accept(sample(1, 2, 30), 30), Err(Rejected::Generation));
        assert!(!slot.bind(1));
        assert!(slot.bind(2));
        assert_eq!(slot.observation(30).freshness, Freshness::Awaiting);
        assert_eq!(
            slot.accept(sample(1, u64::MAX, 30), 30),
            Err(Rejected::Generation)
        );
        slot.accept(sample(2, u64::MAX, u64::MAX), u64::MAX)
            .unwrap();
        assert_eq!(
            slot.accept(sample(2, 0, u64::MAX), u64::MAX),
            Err(Rejected::Sequence)
        );
        assert!(slot.bind(u64::MAX));
        slot.unbind();
        assert!(!slot.bind(u64::MAX));
    }
}
