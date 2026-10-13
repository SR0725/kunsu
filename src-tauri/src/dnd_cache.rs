//! 勿擾狀態讀取失敗時的決策（docs/bdd/overlay-presentation.md AC-OP-10）。
//!
//! 舊行為：讀取失敗一律當成勿擾中，資料庫一出錯所有主動 surface 每秒被隱藏，強制收件匣
//! 等於失效。新規則：沿用最後一次讀成功的值；從未讀成功時先保守隱藏並重試，持續 60 秒仍
//! 失敗就視為未啟用並告警一次。

pub const NEVER_KNOWN_GRACE_SECS: u64 = 60;

#[derive(Debug, Default)]
pub struct DndCache {
    last_known: Option<bool>,
    first_error_at: Option<u64>,
    alerted: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DndDecision {
    pub active: bool,
    /// 只在第一次判定「從未讀到且已逾 60 秒」時為 true。
    pub alert: bool,
}

impl DndCache {
    pub const fn new() -> Self {
        Self { last_known: None, first_error_at: None, alerted: false }
    }

    pub fn observe(&mut self, read: Result<bool, ()>, now_secs: u64) -> DndDecision {
        match read {
            Ok(active) => {
                self.last_known = Some(active);
                self.first_error_at = None;
                self.alerted = false;
                DndDecision { active, alert: false }
            }
            Err(()) => {
                if let Some(active) = self.last_known {
                    return DndDecision { active, alert: false };
                }
                let since = *self.first_error_at.get_or_insert(now_secs);
                if now_secs.saturating_sub(since) < NEVER_KNOWN_GRACE_SECS {
                    return DndDecision { active: true, alert: false };
                }
                let alert = !self.alerted;
                self.alerted = true;
                DndDecision { active: false, alert }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_reads_pass_through_and_are_remembered() {
        let mut cache = DndCache::default();
        assert_eq!(cache.observe(Ok(true), 0), DndDecision { active: true, alert: false });
        assert_eq!(cache.observe(Ok(false), 1), DndDecision { active: false, alert: false });
    }

    #[test]
    fn read_failure_uses_the_last_known_value() {
        let mut cache = DndCache::default();
        cache.observe(Ok(false), 0);
        assert_eq!(cache.observe(Err(()), 5), DndDecision { active: false, alert: false });
        cache.observe(Ok(true), 6);
        assert_eq!(cache.observe(Err(()), 500), DndDecision { active: true, alert: false });
    }

    #[test]
    fn never_known_hides_conservatively_then_gives_up_after_grace_and_alerts_once() {
        let mut cache = DndCache::default();
        assert_eq!(cache.observe(Err(()), 100), DndDecision { active: true, alert: false });
        assert_eq!(
            cache.observe(Err(()), 100 + NEVER_KNOWN_GRACE_SECS - 1),
            DndDecision { active: true, alert: false }
        );
        assert_eq!(
            cache.observe(Err(()), 100 + NEVER_KNOWN_GRACE_SECS),
            DndDecision { active: false, alert: true }
        );
        assert_eq!(
            cache.observe(Err(()), 100 + NEVER_KNOWN_GRACE_SECS + 30),
            DndDecision { active: false, alert: false }
        );
    }

    #[test]
    fn a_successful_read_resets_the_error_window() {
        let mut cache = DndCache::default();
        cache.observe(Err(()), 0);
        cache.observe(Ok(false), 10);
        assert_eq!(cache.observe(Err(()), 1_000), DndDecision { active: false, alert: false });
    }
}
