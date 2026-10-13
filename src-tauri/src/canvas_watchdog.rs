//! 畫布卡住主執行緒的看門狗（docs/bdd/evidence-canvas.md 場景 9b、AC-EC-09b）。
//!
//! 面板掛上畫布時以 IPC 告知（`canvas_active`），殼層每秒送一次心跳，網頁以 `surface_pong` 回應；連續 3 秒
//! 沒回應就判定該視窗卡住：停用這一題這一版的畫布，交給殼層記 `canvas_failed` 並重新載入網頁。
//! 純狀態機：輸入事件與單調時間（毫秒），輸出要殼層執行的命令。停用清單只在記憶體，app 重開即清空。

use std::collections::{HashMap, HashSet};

pub const PING_EVERY_MS: u64 = 1_000;
pub const HANG_AFTER_MS: u64 = 3_000;
pub const HANG_REASON: &str = "畫布沒有回應";

#[derive(Debug, Clone, PartialEq)]
pub enum WdCmd {
    /// 對這個視窗送心跳（`kunsu://canvas_ping`）。
    Ping { window: String },
    /// 這個視窗卡住了：記 canvas_failed、重新載入。
    Hung { window: String, question_id: String, revision: u32 },
}

#[derive(Debug)]
struct Active {
    question_id: String,
    revision: u32,
    last_pong: u64,
    last_ping: u64,
}

#[derive(Debug, Default)]
pub struct Watchdog {
    active: HashMap<String, Active>,
    disabled: HashSet<(String, u32)>,
}

impl Watchdog {
    /// 面板要執行畫布前先問：回 false＝這一版已被停用（卡住過），任何視窗都不再執行；回 true 就開始追蹤。
    pub fn canvas_active(&mut self, window: &str, question_id: &str, revision: u32, now: u64) -> bool {
        if self.is_disabled(question_id, revision) {
            return false;
        }
        self.active.insert(
            window.to_string(),
            Active { question_id: question_id.to_string(), revision, last_pong: now, last_ping: now },
        );
        true
    }

    /// 面板卸下畫布；只移除同一題同一版（避免晚到的卸下把新畫布的追蹤清掉）。
    pub fn canvas_inactive(&mut self, window: &str, question_id: &str, revision: u32) {
        if self.active.get(window).is_some_and(|a| a.question_id == question_id && a.revision == revision) {
            self.active.remove(window);
        }
    }

    pub fn pong(&mut self, window: &str, now: u64) {
        if let Some(a) = self.active.get_mut(window) {
            a.last_pong = a.last_pong.max(now);
        }
    }

    pub fn is_disabled(&self, question_id: &str, revision: u32) -> bool {
        self.disabled.contains(&(question_id.to_string(), revision))
    }

    /// 視窗銷毀或網頁開始重新載入：那個網頁已經不在了，清掉它的追蹤（不是卡住，不停用、不回報）。
    pub fn forget_window(&mut self, window: &str) {
        self.active.remove(window);
    }

    /// 目前追蹤中的視窗（殼層在不持有看門狗鎖的情況下確認它們還在）。
    pub fn tracked_windows(&self) -> Vec<String> {
        let mut windows: Vec<String> = self.active.keys().cloned().collect();
        windows.sort();
        windows
    }

    /// `alive`：視窗還在嗎？不在的只丟掉追蹤，不送心跳、不判定卡住（銷毀事件晚到時的保險）。
    pub fn tick(&mut self, now: u64, alive: impl Fn(&str) -> bool) -> Vec<WdCmd> {
        let mut cmds = Vec::new();
        let mut hung = Vec::new();
        for window in self.tracked_windows() {
            if !alive(&window) {
                self.active.remove(&window);
                continue;
            }
            let a = self.active.get_mut(&window).expect("key exists");
            if now.saturating_sub(a.last_pong) >= HANG_AFTER_MS {
                hung.push(window);
            } else if now.saturating_sub(a.last_ping) >= PING_EVERY_MS {
                a.last_ping = now;
                cmds.push(WdCmd::Ping { window });
            }
        }
        for window in hung {
            if let Some(a) = self.active.remove(&window) {
                self.disabled.insert((a.question_id.clone(), a.revision));
                cmds.push(WdCmd::Hung { window, question_id: a.question_id, revision: a.revision });
            }
        }
        cmds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active(w: &mut Watchdog, now: u64) {
        w.canvas_active("overlay", "q1", 2, now);
    }

    /// 所有視窗都還在。
    fn all(_: &str) -> bool {
        true
    }

    #[test]
    fn pings_every_second_only_while_a_canvas_is_active() {
        let mut w = Watchdog::default();
        assert!(w.tick(5_000, all).is_empty(), "沒有畫布就不送心跳");
        active(&mut w, 0);
        assert_eq!(w.tick(999, all), vec![]);
        assert_eq!(w.tick(1_000, all), vec![WdCmd::Ping { window: "overlay".into() }]);
        assert_eq!(w.tick(1_500, all), vec![]);
        w.pong("overlay", 1_600);
        assert_eq!(w.tick(2_000, all), vec![WdCmd::Ping { window: "overlay".into() }]);
    }

    #[test]
    fn pongs_keep_the_canvas_alive() {
        let mut w = Watchdog::default();
        active(&mut w, 0);
        for t in (1_000..20_000).step_by(1_000) {
            let cmds = w.tick(t, all);
            assert!(!cmds.iter().any(|c| matches!(c, WdCmd::Hung { .. })), "t={t}");
            w.pong("overlay", t + 100);
        }
        assert!(!w.is_disabled("q1", 2));
    }

    #[test]
    fn three_seconds_without_pong_disables_that_revision_once() {
        let mut w = Watchdog::default();
        active(&mut w, 0);
        w.tick(1_000, all);
        w.tick(2_000, all);
        assert_eq!(w.tick(2_999, all).iter().filter(|c| matches!(c, WdCmd::Hung { .. })).count(), 0);
        let cmds = w.tick(3_000, all);
        assert_eq!(
            cmds,
            vec![WdCmd::Hung { window: "overlay".into(), question_id: "q1".into(), revision: 2 }]
        );
        assert!(w.is_disabled("q1", 2));
        assert!(!w.is_disabled("q1", 3), "新版本不受影響");
        assert!(!w.is_disabled("q2", 2));
        assert!(w.tick(10_000, all).is_empty(), "判定後不再追蹤、不重複回報");
    }

    #[test]
    fn inactive_or_replaced_canvas_is_not_judged() {
        let mut w = Watchdog::default();
        active(&mut w, 0);
        w.canvas_inactive("overlay", "q1", 2);
        assert!(w.tick(10_000, all).is_empty());

        active(&mut w, 10_000);
        w.canvas_active("overlay", "q9", 1, 11_000);
        w.canvas_inactive("overlay", "q1", 2);
        let hung: Vec<_> = w.tick(14_000, all).into_iter().filter(|c| matches!(c, WdCmd::Hung { .. })).collect();
        assert_eq!(hung, vec![WdCmd::Hung { window: "overlay".into(), question_id: "q9".into(), revision: 1 }]);
    }

    #[test]
    fn disabled_revision_is_not_tracked_again() {
        let mut w = Watchdog::default();
        active(&mut w, 0);
        w.tick(3_000, all);
        active(&mut w, 4_000);
        assert!(w.tick(9_000, all).is_empty());
    }

    /// AC-EC-09b：停用的版本在任何視窗都不再執行（殼層回 false）。
    #[test]
    fn disabled_revision_is_refused_in_any_window() {
        let mut w = Watchdog::default();
        assert!(w.canvas_active("overlay", "q1", 2, 0), "第一次可以執行");
        w.tick(3_000, all);
        assert!(w.is_disabled("q1", 2));
        assert!(!w.canvas_active("main", "q1", 2, 4_000), "主視窗用舊快取也不能再跑這一版");
        assert!(w.canvas_active("main", "q1", 3, 4_000), "新版本照常");
    }

    /// QA 第 4 輪（AC-EC-09b）：主視窗顯示畫布時按 ⌘W（視窗銷毀）或網頁重新載入——那個網頁已經不在了，殼層清掉
    /// 它的追蹤；之後不送心跳、不判定卡住、不停用這一版。
    #[test]
    fn closed_or_reloaded_window_is_forgotten() {
        let mut w = Watchdog::default();
        assert!(w.canvas_active("main", "q1", 2, 0));
        assert!(w.canvas_active("overlay", "q1", 2, 0));
        w.forget_window("main");
        let cmds = w.tick(10_000, all);
        assert!(
            cmds.iter().all(|c| !matches!(c, WdCmd::Ping { window } | WdCmd::Hung { window, .. } if window == "main")),
            "{cmds:?}"
        );
        // 另一個視窗照常判定。
        assert!(cmds.contains(&WdCmd::Hung { window: "overlay".into(), question_id: "q1".into(), revision: 2 }));

        let mut w = Watchdog::default();
        assert!(w.canvas_active("main", "q1", 2, 0));
        w.forget_window("main");
        assert!(w.tick(10_000, all).is_empty());
        assert!(!w.is_disabled("q1", 2), "關掉視窗不是畫布卡住");
        assert!(w.canvas_active("main", "q1", 2, 11_000), "重新載入後的面板可以再執行同一版");
    }

    /// QA 第 4 輪（AC-EC-09b）：判定卡住前先確認視窗還在；不在（銷毀事件還沒到）就只丟掉追蹤，不停用、不回報。
    #[test]
    fn hang_is_only_judged_for_windows_that_still_exist() {
        let mut w = Watchdog::default();
        assert!(w.canvas_active("main", "q1", 2, 0));
        let gone = |window: &str| window != "main";
        assert!(w.tick(1_000, gone).is_empty(), "不存在的視窗不送心跳");
        assert!(w.tick(5_000, gone).is_empty());
        assert!(!w.is_disabled("q1", 2));
        assert!(w.tick(9_000, all).is_empty(), "追蹤已丟掉，不會之後才回報");
    }
}
