//! WKWebView 的原生設定（macOS 私有 API，有這個方法才呼叫）。
//!
//! - 引擎層關閉清單（evidence-canvas AC-EC-06／INV-20）：CSP 管不到 WebRTC 與資源提示——畫布文件、srcdoc 子框架、
//!   `blob:` 文件都可能拿到 `RTCPeerConnection`，`<link rel=preconnect|dns-prefetch>` 能對外 TCP 連線、經 DNS 帶資料。
//!   每個 webview 由 `windows::build_webview` 建立後立刻套用 [`harden`]，之後產生的文件、子框架與終止後重新啟動的網頁
//!   行程都生效（2026-10-08／10-09 真 WKWebView 實驗確認）。
//! - 子框架導覽關卡（INV-20；QA 第 5 輪）：wry 的 `on_navigation` 只給網址、分不出主框架與畫布 iframe，畫布框會被一般的
//!   `<a href="#table">` 導到 Kunsu 自己的頁面。[`harden`] 同時在 wry 的 navigation delegate 類別上包一層導覽決策：
//!   目標是子框架就只准停在沙盒文件（`navigation::allowed_subframe_navigation`），其餘交回 wry 原本的實作（`frame_gate`）。
//! - 終止卡住的網頁行程（AC-EC-09b）：畫布無窮迴圈時 `reload()` 救不回來；每個 webview 有自己的 WebContent 行程，
//!   SIGKILL 後重新載入只影響那個視窗（同一實驗確認）。

/// 引擎層要關的功能：`+[WKPreferences _features]` 的 key（INV-20）。WebRTC 另外也用專用的 `_setPeerConnectionEnabled:`。
pub const ENGINE_OFF_FEATURES: &[&str] = &[
    "PeerConnectionEnabled",
    "LinkPreconnect",
    "LinkDNSPrefetchEnabled",
    "LinkPreloadEnabled",
    "LinkPrefetchEnabled",
    "SpeculationRulesPrefetchEnabled",
    "WebTransportEnabled",
];

/// 這個執行環境要關哪些、缺哪些（缺的記 warn，不默默略過）。
#[derive(Debug, Default, PartialEq)]
pub struct EnginePlan {
    pub disable: Vec<&'static str>,
    pub missing: Vec<&'static str>,
}

pub fn plan_engine_off(available: &[&str]) -> EnginePlan {
    let mut plan = EnginePlan::default();
    for key in ENGINE_OFF_FEATURES {
        if available.contains(key) {
            plan.disable.push(key);
        } else {
            plan.missing.push(key);
        }
    }
    plan
}

#[cfg(target_os = "macos")]
fn with_wk(window: &tauri::WebviewWindow, f: impl FnOnce(&objc2::runtime::AnyObject) + Send + 'static) {
    let _ = window.with_webview(move |webview| {
        // SAFETY：Tauri 在 macOS 上的 inner() 就是 WKWebView；with_webview 在主執行緒呼叫。
        let wk: &objc2::runtime::AnyObject = unsafe { &*(webview.inner() as *const objc2::runtime::AnyObject) };
        f(wk);
    });
}

/// 套用引擎關閉清單（WebRTC 與資源提示）並裝上子框架導覽關卡。每個 webview 建立後由 `windows::build_webview` 立刻呼叫。
#[cfg(target_os = "macos")]
pub fn harden(window: &tauri::WebviewWindow) {
    let label = window.label().to_string();
    // SAFETY：with_wk 在主執行緒交出有效的 WKWebView。
    with_wk(window, move |wk| unsafe { harden_wk(wk, &label) });
}

#[cfg(not(target_os = "macos"))]
pub fn harden(_window: &tauri::WebviewWindow) {}

/// SAFETY：只在主執行緒、以有效的 WKWebView 呼叫；每個私有方法都先 respondsToSelector。
#[cfg(target_os = "macos")]
unsafe fn harden_wk(wk: &objc2::runtime::AnyObject, label: &str) {
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{msg_send, sel};
    use objc2_foundation::NSString;

    // SAFETY：同一個主執行緒呼叫、同一個有效的 WKWebView。
    unsafe { crate::frame_gate::install_frame_gate(wk, label) };

    let config: *mut AnyObject = unsafe { msg_send![wk, configuration] };
    if config.is_null() {
        tracing::warn!("{label}：拿不到 WKWebViewConfiguration，引擎關閉清單沒有套用");
        return;
    }
    let prefs: *mut AnyObject = unsafe { msg_send![&*config, preferences] };
    if prefs.is_null() {
        tracing::warn!("{label}：拿不到 WKPreferences，引擎關閉清單沒有套用");
        return;
    }
    let prefs = unsafe { &*prefs };

    // WebRTC：專用 setter（較舊的 WebKit 也有）。
    let peer: bool = unsafe { msg_send![prefs, respondsToSelector: sel!(_setPeerConnectionEnabled:)] };
    if peer {
        let _: () = unsafe { msg_send![prefs, _setPeerConnectionEnabled: false] };
    }

    // 清單上的 feature（含 WebRTC 的開關）：+[WKPreferences _features] 找 key → _setEnabled:NO forFeature:。
    let Some(cls) = AnyClass::get(c"WKPreferences") else {
        tracing::warn!("{label}：找不到 WKPreferences 類別，資源提示沒有關閉");
        return;
    };
    let has_features: bool = unsafe { msg_send![cls, respondsToSelector: sel!(_features)] };
    let can_set: bool = unsafe { msg_send![prefs, respondsToSelector: sel!(_setEnabled:forFeature:)] };
    if !has_features || !can_set {
        tracing::warn!(
            "{label}：WKPreferences 沒有 _features／_setEnabled:forFeature:，{:?} 沒有關閉（WebRTC 專用 setter：{peer}）",
            ENGINE_OFF_FEATURES
        );
        return;
    }
    let features: *mut AnyObject = unsafe { msg_send![cls, _features] };
    if features.is_null() {
        tracing::warn!("{label}：+[WKPreferences _features] 是空的");
        return;
    }
    let count: usize = unsafe { msg_send![&*features, count] };
    let mut keys: Vec<String> = Vec::with_capacity(count);
    let mut objects: Vec<*mut AnyObject> = Vec::with_capacity(count);
    for i in 0..count {
        let feature: *mut AnyObject = unsafe { msg_send![&*features, objectAtIndex: i] };
        if feature.is_null() {
            continue;
        }
        let has_key: bool = unsafe { msg_send![&*feature, respondsToSelector: sel!(key)] };
        if !has_key {
            continue;
        }
        let key: *mut NSString = unsafe { msg_send![&*feature, key] };
        if key.is_null() {
            continue;
        }
        keys.push(unsafe { (*key).to_string() });
        objects.push(feature);
    }
    let names: Vec<&str> = keys.iter().map(String::as_str).collect();
    let plan = plan_engine_off(&names);
    for key in &plan.disable {
        if let Some(i) = keys.iter().position(|k| k == key) {
            let _: () = unsafe { msg_send![prefs, _setEnabled: false, forFeature: &*objects[i]] };
        }
    }
    if !plan.missing.is_empty() {
        tracing::warn!("{label}：這版 WebKit 沒有 {:?}（其餘已關閉：{:?}）", plan.missing, plan.disable);
    }
}

/// 這個 webview 的網頁行程 pid（主執行緒取得後交給 `f`；視窗已不在就不會呼叫）。
#[cfg(target_os = "macos")]
pub fn with_web_process_id(window: &tauri::WebviewWindow, f: impl FnOnce(Option<i32>) + Send + 'static) {
    use objc2::{msg_send, sel};
    with_wk(window, move |wk| {
        // SAFETY：主執行緒上的有效 WKWebView；私有方法先 respondsToSelector。
        let responds: bool = unsafe { msg_send![wk, respondsToSelector: sel!(_webProcessIdentifier)] };
        let pid: i32 = if responds { unsafe { msg_send![wk, _webProcessIdentifier] } } else { 0 };
        f((pid > 0).then_some(pid));
    });
}

#[cfg(not(target_os = "macos"))]
pub fn with_web_process_id(_window: &tauri::WebviewWindow, f: impl FnOnce(Option<i32>) + Send + 'static) {
    f(None);
}

/// SIGKILL 一個網頁行程：不能是 0、不能是自己。
pub fn kill_pid(pid: i32) -> bool {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        const SIGKILL: i32 = 9;
        if pid <= 0 || pid as u32 == std::process::id() {
            return false;
        }
        // SAFETY：只送給 WKWebView 回報的網頁行程 pid。
        let rc = unsafe { kill(pid, SIGKILL) };
        tracing::warn!("終止網頁行程 pid={pid}（結果 {rc}）");
        rc == 0
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// 終止這個 webview 卡住的網頁行程（之後呼叫端要重新載入或銷毀）。
pub fn kill_web_process(window: &tauri::WebviewWindow) {
    with_web_process_id(window, |pid| {
        if let Some(pid) = pid {
            kill_pid(pid);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// INV 20（殼層的引擎層這一半；導覽白名單在 navigation.rs 的同名測試）：CSP 管不到的 WebRTC 與資源提示在引擎
    /// 設定關閉；每個 Kunsu webview 都經同一個建立路徑（`windows::build_webview`），建好後立刻套用整份關閉清單。
    #[test]
    fn inv_canvas_sandboxed() {
        // 1. 關閉清單：WebRTC、preconnect、dns-prefetch、preload（QA 第 4 輪：preconnect 能對外 TCP 連線、經 DNS 帶資料）。
        for key in ["PeerConnectionEnabled", "LinkPreconnect", "LinkDNSPrefetchEnabled", "LinkPreloadEnabled"] {
            assert!(ENGINE_OFF_FEATURES.contains(&key), "引擎關閉清單少了 {key}");
        }

        // 2. 執行環境有的 feature 全部關；沒有的列出來（記 warn），不會默默略過。
        let available = ["Unrelated", "LinkPreloadEnabled", "PeerConnectionEnabled", "LinkPreconnect", "LinkDNSPrefetchEnabled"];
        let plan = plan_engine_off(&available);
        for key in ["PeerConnectionEnabled", "LinkPreconnect", "LinkDNSPrefetchEnabled", "LinkPreloadEnabled"] {
            assert!(plan.disable.contains(&key), "{key} 要關");
            assert!(!plan.missing.contains(&key));
        }
        assert!(!plan.disable.contains(&"Unrelated"), "只關清單上的");
        assert_eq!(plan.disable.len() + plan.missing.len(), ENGINE_OFF_FEATURES.len());
        assert_eq!(plan_engine_off(&[]).missing.len(), ENGINE_OFF_FEATURES.len());

        // 3. 每個 webview 都經 build_webview 建立（main、capsule、overlay、canvas-check），沒有繞過的 builder。
        let needle = concat!("WebviewWindow", "Builder::new(");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut builders = 0;
        for entry in std::fs::read_dir(&dir).expect("src 目錄") {
            let path = entry.expect("檔案").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("讀檔");
            for (at, _) in text.match_indices(needle) {
                builders += 1;
                assert!(
                    text[..at].trim_end().ends_with("build_webview("),
                    "{} 有沒經過 windows::build_webview 的 webview（第 {} 個字元）",
                    path.display(),
                    at
                );
            }
        }
        assert!(builders >= 4, "main、capsule、overlay、canvas-check 都要找得到（實際 {builders}）");

        // 4. build_webview：build() 之後立刻套用引擎關閉清單，才交給呼叫端。
        let windows = include_str!("windows.rs");
        let start = windows.find("pub fn build_webview").expect("windows::build_webview");
        let body = &windows[start..];
        let body = &body[..body.find("\n}\n").expect("函式結尾")];
        let built = body.find(".build()").expect("build_webview 要 build()");
        let hardened = body.find("webview_native::harden(").expect("build_webview 要套用 harden");
        assert!(built < hardened, "build 之後立刻 harden");

        // 5. QA 第 5 輪：harden 一併裝上子框架導覽關卡（wry 的 on_navigation 只給網址、分不出畫布 iframe）。
        let native = include_str!("webview_native.rs");
        let start = native.find("unsafe fn harden_wk").expect("harden_wk");
        let body = &native[start..];
        let body = &body[..body.find("\n}\n").expect("函式結尾")];
        assert!(body.contains("install_frame_gate("), "harden_wk 要裝子框架導覽關卡");
        // 關卡包的是 WKNavigationDelegate 的導覽決策方法，依 targetFrame.isMainFrame 判斷、交給 navigation 的純函數。
        let gate_src = include_str!("frame_gate.rs");
        let gate = &gate_src[gate_src.find("fn install_frame_gate").expect("install_frame_gate")..];
        assert!(gate.contains("webView:decidePolicyForNavigationAction:decisionHandler:"));
        let decide = &gate_src[gate_src.find("extern \"C-unwind\" fn gated_decide_policy").expect("gated_decide_policy")..];
        assert!(decide.contains("isMainFrame") && decide.contains("navigation::subframe_blocked("));
    }
}
