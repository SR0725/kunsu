//! Webview 導覽白名單（PRD §24；INV `inv_canvas_sandboxed`）。CSP 管不到「自己換頁」：畫布 iframe 只要
//! `location.href = 外部網址` 就能把資料帶出去。
//!
//! - 主框架（`allowed_navigation`，經 Tauri 的 `on_navigation`）：app 自己的頁面與沙盒文件。
//! - 子框架（`allowed_subframe_navigation`，經 `webview_native` 包在 wry navigation delegate 上的關卡）：畫布 iframe 與它自己
//!   開的巢狀框架只能停在 `about:srcdoc`、`about:blank`（含同文件片段）與 `blob:`。QA 第 5 輪：srcdoc 的相對網址以上層
//!   頁面為基準，`<a href="#table">`、`location.href = 'index.html'` 都會把畫布框換成 Kunsu 自己的頁面（沒有畫布的 CSP）；
//!   wry 的 `on_navigation` 只給網址、分不出框架，所以子框架另外判斷。

/// `about:srcdoc`／`about:blank` 本身或帶同文件片段（WebKit 回報片段時可能把 `#` 編碼成 `%23`）。
fn sandbox_document(lower: &str) -> bool {
    ["about:srcdoc", "about:blank"].iter().any(|doc| {
        lower.strip_prefix(doc).is_some_and(|rest| rest.is_empty() || rest.starts_with('#') || rest.starts_with("%23"))
    })
}

/// 子框架（畫布 iframe 與巢狀框架）可以停的地方：只有沙盒文件。
pub fn allowed_subframe_navigation(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    sandbox_document(&lower) || lower.starts_with("blob:")
}

/// 子框架關卡：目標是子框架（`Some(false)`）且不是沙盒文件 → 擋。主框架（`Some(true)`）與開新視窗（`None`，targetFrame
/// 為 nil）不由這裡決定，交回 `on_navigation` 的白名單。
pub fn subframe_blocked(target_is_main_frame: Option<bool>, url: &str) -> bool {
    target_is_main_frame == Some(false) && !allowed_subframe_navigation(url)
}

/// 主框架允許的導覽目標。
pub fn allowed_navigation(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if sandbox_document(&lower) || lower.starts_with("blob:") {
        return true;
    }
    if let Some(rest) = lower.strip_prefix("tauri://localhost") {
        return rest.is_empty() || rest.starts_with('/') || rest.starts_with('?');
    }
    for prefix in ["http://tauri.localhost", "https://tauri.localhost"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return rest.is_empty() || rest.starts_with('/') || rest.starts_with('?');
        }
    }
    // 開發模式（pnpm dev 的 vite）才放行本機開發伺服器。
    cfg!(debug_assertions) && (lower.starts_with("http://localhost:1420") || lower.starts_with("http://127.0.0.1:1420"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// INV 20（殼層這一半）：沙盒 iframe 與所有視窗都不能導覽到外部網址；app 自己的頁面、srcdoc、blob 照常。
    #[test]
    fn inv_canvas_sandboxed() {
        for ok in [
            "tauri://localhost/index.html?view=overlay",
            "http://tauri.localhost/index.html",
            "https://tauri.localhost/index.html",
            "about:srcdoc",
            "about:blank",
            "blob:null/2b1c4f0e-0000-4000-8000-000000000000",
        ] {
            assert!(allowed_navigation(ok), "{ok}");
        }
        for bad in [
            "https://tracker.test/leak?data=1",
            "http://127.0.0.1:9999/x",
            "http://tauri.localhost.evil.test/",
            "ftp://x.test/",
            "javascript:alert(1)",
            "data:text/html,<script>1</script>",
            "file:///etc/passwd",
        ] {
            assert!(!allowed_navigation(bad), "{bad}");
        }

        // QA 第 5 輪：畫布框（任何子框架）只能停在自己的沙盒文件。srcdoc 的相對網址以上層頁面為基準，一般的
        // `<a href="#table">`、`location.href = 'index.html'` 都會把畫布框換成 Kunsu 自己的頁面（沒有畫布的 CSP）。
        for ok in [
            "about:srcdoc",
            "ABOUT:SRCDOC",
            "about:srcdoc#table",
            "about:srcdoc%23table", // WebKit 回報同文件片段時把 # 編碼
            "about:blank",
            "about:blank#x",
            "blob:null/2b1c4f0e-0000-4000-8000-000000000000",
        ] {
            assert!(allowed_subframe_navigation(ok), "子框架可以停在 {ok}");
            assert!(!subframe_blocked(Some(false), ok), "{ok}");
        }
        for bad in [
            "tauri://localhost/index.html?view=overlay",
            "tauri://localhost/index.html?view=overlay#table",
            "tauri://localhost",
            "http://tauri.localhost/index.html",
            "https://tauri.localhost/index.html?view=main",
            "http://localhost:1420/index.html",
            "data:text/html,<p>x</p>",
            "https://tracker.test/leak",
            "about:srcdocx",
            "about:srcdoc/../x",
            "about:blankx",
            "javascript:alert(1)",
        ] {
            assert!(!allowed_subframe_navigation(bad), "子框架不能導到 {bad}");
            assert!(subframe_blocked(Some(false), bad), "{bad}");
        }
        // 主框架與開新視窗（targetFrame 為 nil）交回原本的白名單，不由子框架關卡決定。
        assert!(!subframe_blocked(Some(true), "tauri://localhost/index.html?view=overlay"));
        assert!(!subframe_blocked(None, "tauri://localhost/index.html"));
        // 主框架白名單也接受 srcdoc／blank 的同文件片段（兩層判斷一致）。
        for ok in ["about:srcdoc#table", "about:srcdoc%23table", "about:blank%23x"] {
            assert!(allowed_navigation(ok), "{ok}");
        }
    }
}
