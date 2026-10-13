//! Guards 快照注入介面。真偵測（ScreenCaptureKit / CoreMediaIO / DND…）在
//! Tauri 層之後接；本階段給 stub。kill-switch 與 forced 預算計數是 server
//! 自身狀態，不歸 probe 管——服務層組 GuardState 時自行補上。

use std::sync::Mutex;

/// 情境旗標快照（GuardState 的硬體/系統面子集）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GuardSnapshot {
    pub screen_shared: bool,
    pub camera_on: bool,
    pub mic_on: bool,
    pub frontmost_fullscreen: bool,
    pub dnd: bool,
}

pub trait GuardProbe: Send + Sync {
    fn snapshot(&self) -> GuardSnapshot;
}

/// 本階段 stub：預設全綠；測試可 `set()` 翻旗標模擬會議中/全螢幕。
#[derive(Default)]
pub struct StubGuardProbe {
    snap: Mutex<GuardSnapshot>,
}

impl StubGuardProbe {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, snap: GuardSnapshot) {
        *self.snap.lock().expect("guard stub lock") = snap;
    }
}

impl GuardProbe for StubGuardProbe {
    fn snapshot(&self) -> GuardSnapshot {
        *self.snap.lock().expect("guard stub lock")
    }
}
