//! Optional background index work stops after three consecutive failures.
//! Lifetime is one prepared source; seeking does not reset the budget.
#[derive(Default)]
pub struct IndexScanPolicy {
    failures: u8,
}
impl IndexScanPolicy {
    pub fn can_attempt(&self) -> bool {
        self.failures < 3
    }
    pub fn record_result(&mut self, accepted: bool) {
        if !self.can_attempt() {
            return;
        }
        self.failures = if accepted {
            0
        } else {
            self.failures.saturating_add(1)
        };
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retries_are_bounded_and_only_a_new_source_resets_disabled_work() {
        let mut policy = IndexScanPolicy::default();
        for _ in 0..2 {
            policy.record_result(false);
        }
        policy.record_result(true);
        for _ in 0..2 {
            policy.record_result(false);
        }
        assert!(policy.can_attempt());
        policy.record_result(false);
        assert!(!policy.can_attempt());
        policy.record_result(true);
        assert!(!policy.can_attempt());
        assert!(IndexScanPolicy::default().can_attempt());
    }
}
