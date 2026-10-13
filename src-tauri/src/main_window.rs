//! 主視窗外框（PRD §27、CONTRACT §15、BDD application-shell AC-AS-01）：標題文字隱藏、內容延伸到標題列下
//! （紅綠燈浮在側欄上方），最小 900×600。側欄頂端 44 px 的拖曳區要 `core:window:allow-start-dragging`，
//! 只授權給 main（`capabilities/main-window.json`）。

use tauri::{AppHandle, WebviewWindowBuilder};

/// 主視窗最小寬度（logical px）。
pub const MAIN_MIN_WIDTH: f64 = 900.0;
/// 主視窗最小高度（logical px）。
pub const MAIN_MIN_HEIGHT: f64 = 600.0;

impl<'a> MainWindowChrome for WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    #[cfg(target_os = "macos")]
    fn main_window_chrome(self) -> Self {
        self.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true)
    }

    #[cfg(not(target_os = "macos"))]
    fn main_window_chrome(self) -> Self {
        self
    }
}

/// 主視窗 builder 的外框設定（掛在 `windows::create_windows` 的 main builder 鏈上，main 仍經 `build_webview` 建立）。
pub trait MainWindowChrome {
    fn main_window_chrome(self) -> Self;
}

/// App 層級事件裡 Kunsu 在意的種類。tauri 的 RunEvent 不能在測試裡建構，先轉成這個再判斷。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    /// 點 Dock 上的 Kunsu 圖示（或在 Finder 再打開一次），macOS 送 reopen。
    DockReopen,
    Other,
}

pub fn classify(event: &tauri::RunEvent) -> AppEvent {
    match event {
        tauri::RunEvent::Reopen { .. } => AppEvent::DockReopen,
        _ => AppEvent::Other,
    }
}

/// Kunsu 平常是選單列 App，點 Dock 時要把主視窗叫出來（AC-AS-13）。
pub fn reopen_shows_main(event: AppEvent) -> bool {
    event == AppEvent::DockReopen
}

#[cfg(test)]
mod tests {
    use super::{MAIN_MIN_HEIGHT, MAIN_MIN_WIDTH};
    use serde_json::{json, Value};

    /// AC-AS-13：點 Dock 圖示（macOS 的 reopen）→ 打開主視窗；其他 run 事件不動主視窗。
    #[test]
    fn dock_reopen_shows_the_main_window() {
        assert!(super::reopen_shows_main(super::AppEvent::DockReopen));
        assert!(!super::reopen_shows_main(super::AppEvent::Other));
        assert_eq!(super::classify(&tauri::RunEvent::Ready), super::AppEvent::Other);
        assert_eq!(super::classify(&tauri::RunEvent::Resumed), super::AppEvent::Other);
    }

    /// AC-AS-01：主視窗隱藏標題、內容延伸到標題列下（`TitleBarStyle::Overlay`）、最小 900×600，仍經 `build_webview` 建立；
    /// 側欄拖曳區要的 `core:window:allow-start-dragging` 只授權給 main 視窗。
    #[test]
    fn main_window_hides_the_title_and_keeps_900_by_600() {
        assert_eq!((MAIN_MIN_WIDTH, MAIN_MIN_HEIGHT), (900.0, 600.0));

        let windows = include_str!("windows.rs");
        // 拆開字串：`webview_native` 的 inv_canvas_sandboxed 掃描整個 src 找 builder，不能讓測試字面量被當成 builder。
        let start = windows.find(concat!("WebviewWindow", "Builder::new(app, MAIN,")).expect("main builder");
        assert!(windows[..start].trim_end().ends_with("build_webview("), "main 仍經 build_webview（INV-20）");
        let chain = &windows[start..start + windows[start..].find(")?;").expect("builder 結尾")];
        assert!(chain.contains(".min_inner_size(MAIN_MIN_WIDTH, MAIN_MIN_HEIGHT)"), "{chain}");
        assert!(chain.contains(".main_window_chrome()"), "{chain}");

        let me = include_str!("main_window.rs");
        let body = &me[me.find("fn main_window_chrome(self)").expect("chrome")..];
        let body = &body[..body.find("\n    }\n").expect("函式結尾")];
        assert!(body.contains("TitleBarStyle::Overlay"), "{body}");
        assert!(body.contains("hidden_title(true)"), "{body}");

        let cap: Value = serde_json::from_str(include_str!("../capabilities/main-window.json")).unwrap();
        assert_eq!(cap["windows"], json!(["main"]));
        assert_eq!(cap["permissions"], json!(["core:window:allow-start-dragging"]));
        let default: Value = serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert!(
            !default["permissions"].as_array().unwrap().contains(&json!("core:window:allow-start-dragging")),
            "瀏海、蓋板不能被拖動"
        );
    }
}
