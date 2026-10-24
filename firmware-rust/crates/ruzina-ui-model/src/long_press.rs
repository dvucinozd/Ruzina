//! Touch long press (LVGL `LV_EVENT_LONG_PRESSED`, as M2.5 HOLD RESTORE uses
//! it): one owner at a time, the action fires once after the press has been
//! held for [`LONG_PRESS_MS`], and a release or lost press before then does
//! nothing. Times come from the caller's millisecond clock and may wrap.

/// LVGL's default `LV_INDEV_DEF_LONG_PRESS_TIME`, which M2.5 keeps.
pub const LONG_PRESS_MS: u32 = 400;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LongPress<T> {
    held: Option<(T, u32)>,
}

impl<T: Copy + PartialEq> LongPress<T> {
    pub const fn new() -> Self {
        Self { held: None }
    }

    /// The target being held and not yet fired.
    pub fn held(&self) -> Option<T> {
        self.held.map(|(target, _)| target)
    }

    /// A new press takes over from any other (M2.5 keeps one held deck).
    pub fn press(&mut self, target: T, now_ms: u32) {
        self.held = Some((target, now_ms));
    }

    /// Releasing `target` ends its hold; another target's hold is untouched.
    pub fn release(&mut self, target: T) {
        if self.held() == Some(target) {
            self.held = None;
        }
    }

    /// Press lost (page change, focus loss): nothing fires.
    pub fn cancel(&mut self) {
        self.held = None;
    }

    /// The target whose hold reached [`LONG_PRESS_MS`] at `now_ms`, once.
    pub fn tick(&mut self, now_ms: u32) -> Option<T> {
        let (target, since) = self.held?;
        if now_ms.wrapping_sub(since) < LONG_PRESS_MS {
            return None;
        }
        self.held = None;
        Some(target)
    }
}

impl<T: Copy + PartialEq> Default for LongPress<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_once_after_the_hold_time_and_never_after_an_early_release() {
        let mut press = LongPress::new();
        press.press(1u8, 1_000);
        assert_eq!(press.tick(1_399), None);
        assert_eq!(press.tick(1_400), Some(1));
        assert_eq!(press.tick(5_000), None, "once per press");

        press.press(1, 2_000);
        press.release(1);
        assert_eq!(press.tick(3_000), None);
        press.press(0, 2_000);
        press.cancel();
        assert_eq!(press.tick(3_000), None);
    }

    #[test]
    fn one_owner_and_a_wrapping_clock() {
        let mut press = LongPress::new();
        press.press(0u8, 10);
        press.press(1, 200);
        assert_eq!(press.held(), Some(1));
        press.release(0);
        assert_eq!(press.held(), Some(1), "another deck's release is ignored");
        assert_eq!(press.tick(599), None);
        assert_eq!(press.tick(600), Some(1));

        press.press(0, u32::MAX - 100);
        assert_eq!(press.tick(200), None);
        assert_eq!(press.tick(299), Some(0));
    }
}
