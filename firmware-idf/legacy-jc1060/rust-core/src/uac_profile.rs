//! Exclusive wall-time attribution across a whole six-packet Rust fill.
//! Disabled builds inline these calls away. Enabled state has one USB-task owner;
//! host tests use thread-local state, independently of the process-wide decks.
#[derive(Clone, Copy)]
#[repr(usize)]
#[cfg_attr(not(feature = "uac-profile"), allow(dead_code))]
pub(crate) enum Stage {
    Init,
    Transport,
    Control,
    Graph,
    Progress,
    Copy,
    Pack,
    Other,
}

#[cfg(not(feature = "uac-profile"))]
mod implementation {
    use super::Stage;
    pub(crate) struct Scope;
    pub(crate) struct FillScope;
    #[inline(always)]
    pub(crate) fn fill_scope() -> FillScope {
        FillScope
    }
    #[inline(always)]
    pub(crate) fn scope(_: Stage) -> Scope {
        Scope
    }
    #[inline(always)]
    #[cfg(feature = "deck-mix")]
    pub(crate) fn stage(_: Stage) {}
    #[inline(always)]
    pub(crate) fn packet() {}
    #[inline(always)]
    #[cfg(feature = "deck-mix")]
    pub(crate) fn block() {}
}

#[cfg(feature = "uac-profile")]
mod implementation {
    use super::Stage;
    pub(crate) const WORDS: usize = 20;
    const STAGES: usize = 8;
    const fn clamp(value: u64) -> u32 {
        if value > u32::MAX as u64 {
            u32::MAX
        } else {
            value as u32
        }
    }

    struct Recorder {
        active: bool,
        armed: bool,
        result: [u32; WORDS],
        start: u64,
        last: u64,
        current: Stage,
        us: [u64; STAGES],
        segments: [u32; STAGES],
        packets: u32,
        blocks: u32,
        faults: u32,
    }
    impl Recorder {
        const fn new() -> Self {
            Self {
                active: false,
                armed: false,
                result: [0; WORDS],
                start: 0,
                last: 0,
                current: Stage::Other,
                us: [0; STAGES],
                segments: [0; STAGES],
                packets: 0,
                blocks: 0,
                faults: 0,
            }
        }
        fn begin(&mut self, now: u64) {
            *self = Self::new();
            self.active = true;
            self.start = now;
            self.last = now;
            self.segments[Stage::Other as usize] = 1;
        }
        fn charge(&mut self, now: u64) {
            if let Some(delta) = now.checked_sub(self.last) {
                let i = self.current as usize;
                self.us[i] = self.us[i].saturating_add(delta);
            } else {
                self.faults = self.faults.saturating_add(1);
            }
            self.last = now;
        }
        fn stage(&mut self, next: Stage, now: u64) {
            if !self.active {
                return;
            }
            self.charge(now);
            self.current = next;
            self.segments[next as usize] = self.segments[next as usize].saturating_add(1);
        }
        fn finish(&mut self, now: u64) -> [u32; WORDS] {
            let mut out = [0; WORDS];
            if !self.active {
                out[1] = 1;
                return out;
            }
            self.charge(now);
            self.active = false;
            let total = now.saturating_sub(self.start);
            if self.us.iter().copied().sum::<u64>() != total {
                self.faults = self.faults.saturating_add(1);
            }
            out[0] = clamp(total);
            out[1] = self.faults;
            out[2] = self.packets;
            out[3] = self.blocks;
            for i in 0..STAGES {
                out[4 + i] = clamp(self.us[i]);
                out[12 + i] = self.segments[i];
            }
            out
        }
    }

    #[cfg(not(test))]
    struct Shared(core::cell::UnsafeCell<Recorder>);
    // SAFETY: enabled calls, begin/end and scopes run only on the USB client.
    #[cfg(not(test))]
    unsafe impl Sync for Shared {}
    #[cfg(not(test))]
    static RECORDER: Shared = Shared(core::cell::UnsafeCell::new(Recorder::new()));
    #[cfg(not(test))]
    fn with<R>(f: impl FnOnce(&mut Recorder) -> R) -> R {
        // SAFETY: the C USB client is the only owner; no reference escapes.
        f(unsafe { &mut *RECORDER.0.get() })
    }
    #[cfg(test)]
    std::thread_local! {
        static RECORDER: std::cell::RefCell<Recorder> = const { std::cell::RefCell::new(Recorder::new()) };
        static CLOCK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }
    #[cfg(test)]
    fn with<R>(f: impl FnOnce(&mut Recorder) -> R) -> R {
        RECORDER.with(|r| f(&mut r.borrow_mut()))
    }
    #[cfg(not(test))]
    fn now() -> u64 {
        unsafe extern "C" {
            fn esp_timer_get_time() -> i64;
        }
        // SAFETY: pinned IDF's monotonic, allocation-free systimer read.
        unsafe { esp_timer_get_time() as u64 }
    }
    #[cfg(test)]
    fn now() -> u64 {
        CLOCK.with(|c| c.get())
    }
    #[cfg(test)]
    pub(crate) fn advance(us: u64) {
        CLOCK.with(|c| c.set(c.get() + us));
    }

    pub(crate) struct Scope(Option<Stage>);
    pub(crate) struct FillScope(bool);
    pub(crate) fn fill_scope() -> FillScope {
        with(|r| {
            let armed = r.armed;
            if armed {
                r.begin(now());
            }
            FillScope(armed)
        })
    }
    impl Drop for FillScope {
        fn drop(&mut self) {
            if self.0 {
                with(|r| {
                    let result = r.finish(now());
                    r.result = result;
                });
            }
        }
    }
    pub(crate) fn scope(next: Stage) -> Scope {
        with(|r| {
            if !r.active {
                return Scope(None);
            }
            let previous = r.current;
            r.stage(next, now());
            Scope(Some(previous))
        })
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            if let Some(previous) = self.0 {
                stage(previous);
            }
        }
    }
    pub(crate) fn stage(next: Stage) {
        with(|r| {
            if r.active {
                r.stage(next, now());
            }
        });
    }
    pub(crate) fn packet() {
        with(|r| {
            if r.active {
                r.packets = r.packets.saturating_add(1);
            }
        });
    }
    pub(crate) fn block() {
        with(|r| {
            if r.active {
                r.blocks = r.blocks.saturating_add(1);
            }
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ruzina_legacy_uac_profile_begin() {
        with(|r| {
            *r = Recorder::new();
            r.armed = true;
            r.result[1] = 1; // invalid until the actual fill's guard finishes
        });
    }
    /// USB client only; scalar u32 slots, no Rust layout crosses FFI.
    /// # Safety
    /// `out` is writable/aligned for exactly WORDS u32s when len is WORDS.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ruzina_legacy_uac_profile_end(out: *mut u32, len: u32) -> u32 {
        let result = with(|r| {
            r.armed = false;
            r.active = false;
            r.result
        });
        if out.is_null() || !(out as usize).is_multiple_of(4) || len as usize != WORDS {
            return 0;
        }
        // SAFETY: valid caller-owned buffer of the exact documented length.
        unsafe {
            core::ptr::copy_nonoverlapping(result.as_ptr(), out, WORDS);
        }
        WORDS as u32
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn nested_scopes_partition_time_without_double_counting() {
            with(|r| r.begin(0));
            CLOCK.with(|c| c.set(0));
            advance(5);
            {
                let _scope = scope(Stage::Copy);
                advance(10);
                {
                    let _inner = scope(Stage::Init);
                    advance(20);
                    stage(Stage::Transport);
                    advance(30);
                    stage(Stage::Graph);
                    advance(40);
                    stage(Stage::Progress);
                    advance(50);
                }
                advance(10);
            }
            {
                let _pack = scope(Stage::Pack);
                advance(15);
            }
            advance(5);
            let out = with(|r| r.finish(now()));
            assert_eq!(out[0], 185);
            assert_eq!(out[1], 0);
            assert_eq!(out[4 + Stage::Copy as usize], 20);
            assert_eq!(out[4 + Stage::Graph as usize], 40);
            assert_eq!(out[4 + Stage::Other as usize], 10);
            assert_eq!(out[4..12].iter().sum::<u32>(), out[0]);
        }
        #[test]
        fn inactive_scopes_and_failed_early_exit_do_not_leak_state() {
            let _inactive = scope(Stage::Graph);
            with(|r| r.begin(0));
            CLOCK.with(|c| c.set(0));
            {
                let _scope = scope(Stage::Pack);
                advance(42);
            }
            packet();
            block();
            let out = with(|r| r.finish(now()));
            assert_eq!(out[0], 42);
            assert_eq!(&out[2..4], &[1, 1]);
            assert_eq!(out[4 + Stage::Pack as usize], 42);
            with(|r| r.begin(now()));
            let out = with(|r| r.finish(now()));
            assert_eq!(out[0], 0);
            assert_eq!(&out[2..4], &[0, 0]);
        }
        #[test]
        fn clock_reversal_is_flagged_and_large_duration_clamps() {
            let mut r = Recorder::new();
            r.begin(10);
            r.stage(Stage::Pack, 5);
            assert_ne!(r.finish(15)[1], 0);
            r.begin(0);
            assert_eq!(r.finish(u32::MAX as u64 + 50)[0], u32::MAX);
        }
    }
}
#[cfg(all(test, feature = "uac-profile"))]
pub(crate) use implementation::advance;
#[cfg(feature = "deck-mix")]
pub(crate) use implementation::{block, stage};
pub(crate) use implementation::{fill_scope, packet, scope};
#[cfg(all(test, feature = "idf-uac-profile"))]
pub(crate) use implementation::{
    ruzina_legacy_uac_profile_begin as begin, ruzina_legacy_uac_profile_end as end,
};
