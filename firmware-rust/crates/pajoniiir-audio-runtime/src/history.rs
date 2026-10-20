//! Bounded, absolute-frame PCM history. Prefetch does not advance playback.
use pajoniiir_audio_dsp::PcmFrame;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryError {
    Capacity,
    Generation,
    Discontinuity,
    CoordinateOverflow,
}

pub struct PcmHistory<'a> {
    storage: &'a mut [PcmFrame],
    generation: u32,
    first: u64,
    end: u64,
}

impl<'a> PcmHistory<'a> {
    pub fn new(storage: &'a mut [PcmFrame]) -> Result<Self, HistoryError> {
        if storage.len() < 2 {
            return Err(HistoryError::Capacity);
        }
        Ok(Self {
            storage,
            generation: 0,
            first: 0,
            end: 0,
        })
    }

    pub fn reset(&mut self, generation: u32, first: u64) {
        self.generation = generation;
        self.first = first;
        self.end = first;
    }

    pub fn append(
        &mut self,
        generation: u32,
        first: u64,
        frames: &[PcmFrame],
    ) -> Result<(), HistoryError> {
        if generation == 0 || generation != self.generation {
            return Err(HistoryError::Generation);
        }
        if first != self.end {
            return Err(HistoryError::Discontinuity);
        }
        let end = first
            .checked_add(frames.len() as u64)
            .ok_or(HistoryError::CoordinateOverflow)?;
        for (offset, frame) in frames.iter().enumerate() {
            let index = (first + offset as u64) % self.storage.len() as u64;
            self.storage[index as usize] = *frame;
        }
        self.end = end;
        self.first = self
            .first
            .max(end.saturating_sub(self.storage.len() as u64));
        Ok(())
    }

    pub fn read(&self, generation: u32, frame: u64) -> Option<PcmFrame> {
        if generation == 0
            || generation != self.generation
            || frame < self.first
            || frame >= self.end
        {
            return None;
        }
        Some(self.storage[(frame % self.storage.len() as u64) as usize])
    }

    pub const fn first(&self) -> u64 {
        self.first
    }
    pub const fn end(&self) -> u64 {
        self.end
    }
    pub fn capacity(&self) -> usize {
        self.storage.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_history_rejects_evicted_and_previous_generation_frames() {
        let mut storage = [PcmFrame::default(); 4];
        let mut history = PcmHistory::new(&mut storage).unwrap();
        history.reset(9, 100);
        for frame in 100..108 {
            history
                .append(
                    9,
                    frame,
                    &[PcmFrame {
                        left: frame as i16,
                        right: 0,
                    }],
                )
                .unwrap();
        }
        assert_eq!(history.first(), 104);
        assert_eq!(history.read(9, 103), None);
        assert_eq!(history.read(9, 104).unwrap().left, 104);
        assert_eq!(history.read(9, 107).unwrap().left, 107);
        assert_eq!(history.read(8, 107), None);
        history.reset(10, 500);
        assert_eq!(history.read(10, 107), None);
        assert_eq!(history.read(9, 107), None);
        assert_eq!(
            history.append(10, 499, &[]),
            Err(HistoryError::Discontinuity)
        );
    }

    #[test]
    fn absolute_coordinates_near_u64_limit_are_checked_before_mutation() {
        let mut storage = [PcmFrame::default(); 4];
        let mut history = PcmHistory::new(&mut storage).unwrap();
        history.reset(1, u64::MAX - 1);
        assert_eq!(
            history.append(1, u64::MAX - 1, &[PcmFrame::default(); 2]),
            Err(HistoryError::CoordinateOverflow)
        );
        assert_eq!(history.end(), u64::MAX - 1);
        history
            .append(1, u64::MAX - 1, &[PcmFrame { left: 7, right: 8 }])
            .unwrap();
        assert_eq!(history.read(1, u64::MAX - 1).unwrap().left, 7);
    }
}
