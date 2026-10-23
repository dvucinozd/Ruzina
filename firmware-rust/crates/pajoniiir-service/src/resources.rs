//! Resource reserves and fault counters (RP-14).
//!
//! The deadline side only stores counters the platform already maintains in
//! O(1): allocator free/largest-block values, stack high-water marks, USB
//! loss/recovery and dropped-log counts. No heap walk, formatting or lock
//! happens there. This owner keeps the low-water marks and is formatted
//! later, at low priority, like the service snapshot.

use core::fmt::{self, Write};

pub const MAX_TASKS: usize = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Heap {
    pub free: u32,
    pub largest_block: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceSample {
    pub internal: Heap,
    pub dma: Heap,
    pub psram: Heap,
    /// Minimum free stack bytes per registered task, `None` if unregistered.
    pub stack_free: [Option<u32>; MAX_TASKS],
    pub usb_losses: u32,
    pub usb_recoveries: u32,
    pub dropped_logs: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reserves {
    pub samples: u32,
    pub internal: Heap,
    pub dma: Heap,
    pub psram: Heap,
    pub stack_free: [Option<u32>; MAX_TASKS],
    pub last: Option<ResourceSample>,
}

impl Default for Reserves {
    fn default() -> Self {
        Self::new()
    }
}

fn low(record: &mut Heap, now: Heap, first: bool) {
    if first {
        *record = now;
    } else {
        record.free = record.free.min(now.free);
        record.largest_block = record.largest_block.min(now.largest_block);
    }
}

impl Reserves {
    pub const fn new() -> Self {
        Self {
            samples: 0,
            internal: Heap {
                free: 0,
                largest_block: 0,
            },
            dma: Heap {
                free: 0,
                largest_block: 0,
            },
            psram: Heap {
                free: 0,
                largest_block: 0,
            },
            stack_free: [None; MAX_TASKS],
            last: None,
        }
    }

    pub fn observe(&mut self, sample: ResourceSample) {
        let first = self.samples == 0;
        low(&mut self.internal, sample.internal, first);
        low(&mut self.dma, sample.dma, first);
        low(&mut self.psram, sample.psram, first);
        for (record, now) in self.stack_free.iter_mut().zip(sample.stack_free) {
            *record = match (*record, now) {
                (Some(low), Some(now)) => Some(low.min(now)),
                (low, None) => low,
                (None, now) => now,
            };
        }
        self.samples = self.samples.saturating_add(1);
        self.last = Some(sample);
    }

    /// Low-priority formatting; the caller discards the buffer on error.
    pub fn write_to(&self, output: &mut impl Write) -> fmt::Result {
        writeln!(output, "M1_RESOURCE_RESERVES_V1")?;
        writeln!(output, "samples={}", self.samples)?;
        let Some(last) = self.last else {
            return writeln!(output, "reserves=NOT_MEASURED");
        };
        for (name, low, now) in [
            ("internal", self.internal, last.internal),
            ("dma", self.dma, last.dma),
            ("psram", self.psram, last.psram),
        ] {
            writeln!(
                output,
                "{name}_free={} {name}_free_low={} {name}_largest={} {name}_largest_low={}",
                now.free, low.free, now.largest_block, low.largest_block
            )?;
        }
        for (task, low) in self.stack_free.iter().enumerate() {
            if let Some(low) = low {
                writeln!(output, "task{task}_stack_free_low={low}")?;
            }
        }
        writeln!(
            output,
            "usb_losses={} usb_recoveries={} dropped_logs={}",
            last.usb_losses, last.usb_recoveries, last.dropped_logs
        )
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::String;

    fn sample(internal: u32, largest: u32, stack: Option<u32>) -> ResourceSample {
        let mut stack_free = [None; MAX_TASKS];
        stack_free[2] = stack;
        ResourceSample {
            internal: Heap {
                free: internal,
                largest_block: largest,
            },
            dma: Heap {
                free: 32_768,
                largest_block: 16_384,
            },
            psram: Heap {
                free: 8 << 20,
                largest_block: 4 << 20,
            },
            stack_free,
            usb_losses: 1,
            usb_recoveries: 1,
            dropped_logs: 0,
        }
    }

    #[test]
    fn keeps_low_water_marks_and_the_latest_counters() {
        let mut reserves = Reserves::new();
        let mut text = String::new();
        reserves.write_to(&mut text).unwrap();
        assert!(text.ends_with("reserves=NOT_MEASURED\n"));
        reserves.observe(sample(100_000, 40_000, Some(900)));
        reserves.observe(sample(60_000, 50_000, None));
        reserves.observe(ResourceSample {
            dropped_logs: 3,
            ..sample(80_000, 20_000, Some(1_200))
        });
        assert_eq!(
            reserves.internal,
            Heap {
                free: 60_000,
                largest_block: 20_000
            }
        );
        assert_eq!(reserves.stack_free[2], Some(900));
        assert_eq!(reserves.stack_free[0], None);
        let mut text = String::new();
        reserves.write_to(&mut text).unwrap();
        assert!(text.contains("internal_free=80000 internal_free_low=60000 internal_largest=20000 internal_largest_low=20000\n"));
        assert!(text.contains("task2_stack_free_low=900\n"));
        assert!(text.ends_with("usb_losses=1 usb_recoveries=1 dropped_logs=3\n"));
    }
}
