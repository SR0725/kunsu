//! 情境守門（CONTRACT §3 Guards）。任一命中 → 禁 warned/forced（順延，不取消）。
//! 偵測本身（CoreMediaIO / ScreenCaptureKit / DND…）是 server/shell 層的事；
//! core 只拿到快照布林值做純判斷。

use serde::{Deserialize, Serialize};

use crate::policy::Policy;
use crate::time::Ts;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GuardState {
    pub screen_shared: bool,
    pub camera_on: bool,
    pub mic_on: bool,
    pub frontmost_fullscreen: bool,
    /// server 依 policy 預先算好的 quiet hours 快照；狀態機另外用 policy+now
    /// 再算一次（雙保險，兩者任一命中即擋）。
    pub quiet_hours: bool,
    pub dnd: bool,
    /// 全域快捷鍵 kill-switch 的失效時間；now < 此值即擋。
    pub kill_switch_until: Option<Ts>,
    /// 今日已消耗的 forced 預算（server 統計後注入；非情境旗標，
    /// 但 forced 准入判斷需要，透過 GuardState 一起傳進純函數）。
    pub forced_today_count: u32,
}

impl GuardState {
    /// 情境旗標任一命中（不含預算——預算是另一個獨立准入條件）。
    pub fn any_situational(&self, now: Ts) -> bool {
        self.screen_shared
            || self.camera_on
            || self.mic_on
            || self.frontmost_fullscreen
            || self.quiet_hours
            || self.dnd
            || self.kill_switch_until.is_some_and(|until| now < until)
    }

    /// 完整守門判斷：情境旗標 ∪ policy quiet hours（雙保險）。
    /// true = 本刻禁止 warned/forced。
    pub fn blocks_force(&self, now: Ts, policy: &Policy) -> bool {
        self.any_situational(now) || policy.in_quiet_hours(now)
    }

    /// 到期被延後時瀏海要說的原因（BDD edge-notch 場景 10c），依顯示優先序：分享螢幕、鏡頭、麥克風、前景全螢幕、
    /// 勿擾（Kunsu 勿擾含 kill switch；系統專注模式旗標也算）、安靜時段。與 `blocks_force` 同一份判斷。
    pub fn defer_reasons(&self, now: Ts, policy: &Policy) -> DeferReasons {
        let mut out = DeferReasons::default();
        let flags = [
            (self.screen_shared, DeferReason::ScreenShared),
            (self.camera_on, DeferReason::CameraOn),
            (self.mic_on, DeferReason::MicOn),
            (self.frontmost_fullscreen, DeferReason::FrontmostFullscreen),
        ];
        out.reasons.extend(flags.into_iter().filter(|(on, _)| *on).map(|(_, r)| r));
        let dnd_until = self.kill_switch_until.filter(|until| now < *until);
        if self.dnd || dnd_until.is_some() {
            out.reasons.push(DeferReason::Dnd);
            out.dnd_until = dnd_until;
        }
        if self.quiet_hours || policy.in_quiet_hours(now) {
            out.reasons.push(DeferReason::QuietHours);
            let [_, end] = policy.windows.quiet_hours;
            out.quiet_until = Some(crate::time::next_taipei_occurrence(now, end.0));
        }
        out
    }
}

/// 延後原因碼（snake_case 給 UI 對照文字）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeferReason {
    ScreenShared,
    CameraOn,
    MicOn,
    FrontmostFullscreen,
    Dnd,
    QuietHours,
}

/// `defer_reasons` 的回應：原因（依顯示優先序）與勿擾、安靜時段的結束時刻。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeferReasons {
    pub reasons: Vec<DeferReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dnd_until: Option<Ts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_until: Option<Ts>,
}
