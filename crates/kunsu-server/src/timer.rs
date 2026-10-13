//! timer loop：每 1s 掃 `next_transition_at` due rows（CONTRACT §3）。
//! 所有時限落 DB，重啟由 `ServerHandle::tick` 掃描天然重建，不靠記憶體 timer；
//! 重啟期間已 due 的轉移在下一輪掃描補觸發。

use chrono::Utc;

use crate::service::ServerHandle;

pub const TICK_INTERVAL_S: u64 = 1;

/// 背景 timer。錯誤只記 log 不退出（下一輪重試）；handle drop 後 task 繼續持有
/// clone，呼叫端要停用 `JoinHandle::abort`。
pub fn spawn_timer(handle: ServerHandle) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(TICK_INTERVAL_S));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(e) = handle.tick(Utc::now()).await {
                tracing::error!("timer tick 失敗（下一輪重試）：{e}");
            }
        }
    })
}
