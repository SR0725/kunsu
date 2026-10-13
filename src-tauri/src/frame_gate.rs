//! 子框架導覽關卡（INV-20 `inv_canvas_sandboxed`；CONTRACT §12「導覽白名單分主框架與子框架」；QA 第 5 輪）。
//!
//! wry 的 `on_navigation` 只給網址、分不出主框架與畫布 iframe；srcdoc 的相對網址以上層頁面為基準，一般的
//! `<a href="#table">`、`location.href = 'index.html'` 都會把畫布框換成 Kunsu 自己的頁面（沒有畫布的 CSP）。這裡在 wry 的
//! navigation delegate 類別上包一層 `webView:decidePolicyForNavigationAction:decisionHandler:`（整個行程一次，所有 Kunsu
//! webview 共用同一個類別）：`targetFrame` 是子框架、又不是沙盒文件就直接 Cancel，其餘原樣交回 wry（主框架白名單照舊）。
//! 由 `webview_native::harden` 在每個 webview 建立後呼叫（`windows::build_webview` 同一個建立路徑）。只依賴 objc2／block2，
//! 方便在 Tauri 之外用 wry 直接做原生實驗。

/// wry 原本的 `webView:decidePolicyForNavigationAction:decisionHandler:` 實作（0＝還沒包）。
#[cfg(target_os = "macos")]
static GATE_ORIGINAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// 包過的 navigation delegate 類別（`None`＝類別沒有這個方法，沒包）。整個行程只包一次：所有 Kunsu webview 共用 wry 的類別。
#[cfg(target_os = "macos")]
static GATE_CLASS: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();

/// `- (void)webView:(WKWebView *)webView decidePolicyForNavigationAction:(WKNavigationAction *)action
///    decisionHandler:(void (^)(WKNavigationActionPolicy))handler`
#[cfg(target_os = "macos")]
type DecidePolicyImp = unsafe extern "C-unwind" fn(
    *mut objc2::runtime::AnyObject,
    objc2::runtime::Sel,
    *mut objc2::runtime::AnyObject,
    *mut objc2::runtime::AnyObject,
    *mut objc2::runtime::AnyObject,
);

/// 子框架導覽關卡（INV-20）：在 wry 的 navigation delegate 類別上包一層導覽決策。
/// SAFETY：只在主執行緒、以有效的 WKWebView 呼叫。
#[cfg(target_os = "macos")]
pub unsafe fn install_frame_gate(wk: &objc2::runtime::AnyObject, label: &str) {
    use objc2::runtime::{AnyClass, AnyObject, Imp};
    use objc2::{msg_send, sel};
    use std::sync::atomic::Ordering;

    let delegate: *mut AnyObject = unsafe { msg_send![wk, navigationDelegate] };
    if delegate.is_null() {
        tracing::warn!("{label}：沒有 navigation delegate，子框架導覽關卡沒有裝上（只剩 base URL 與 pagehide 兩層）");
        return;
    }
    let class = unsafe { &*delegate }.class();
    let installed = GATE_CLASS.get_or_init(|| {
        let method = class.instance_method(sel!(webView:decidePolicyForNavigationAction:decisionHandler:))?;
        // 先記下原本的實作，再換上關卡：關卡一生效就找得到要交回的實作。
        GATE_ORIGINAL.store(method.implementation() as usize, Ordering::SeqCst);
        let gate: DecidePolicyImp = gated_decide_policy;
        // SAFETY：gated_decide_policy 與原本的方法簽名相同（同一個 WKNavigationDelegate 方法），參數原樣交回原本的實作；
        // 唯一的差別是子框架要導到沙盒文件以外時，自己呼叫一次 decisionHandler(Cancel) 後不交回。
        unsafe { method.set_implementation(std::mem::transmute::<DecidePolicyImp, Imp>(gate)) };
        Some(class as *const AnyClass as usize)
    });
    match installed {
        None => tracing::warn!(
            "{label}：navigation delegate（{:?}）沒有 webView:decidePolicyForNavigationAction:decisionHandler:，子框架導覽關卡沒有裝上",
            class.name()
        ),
        Some(cls) if *cls != class as *const AnyClass as usize => tracing::warn!(
            "{label}：navigation delegate 類別（{:?}）和第一個 webview 不同，子框架導覽關卡沒有套用",
            class.name()
        ),
        Some(_) => {}
    }
}

/// 關卡本體：目標是子框架、又不是沙盒文件（about:srcdoc／about:blank／blob:）→ `decisionHandler(Cancel)`；其餘交回 wry。
#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn gated_decide_policy(
    this: *mut objc2::runtime::AnyObject,
    cmd: objc2::runtime::Sel,
    webview: *mut objc2::runtime::AnyObject,
    action: *mut objc2::runtime::AnyObject,
    handler: *mut objc2::runtime::AnyObject,
) {
    /// WKNavigationActionPolicyCancel／Allow（NSInteger）。
    const CANCEL: isize = 0;
    const ALLOW: isize = 1;
    // SAFETY：WebKit 交來有效的 action；decisionHandler 是 `void (^)(WKNavigationActionPolicy)`，每次決策只呼叫一次。
    let (main, url) = unsafe { navigation_target(action) };
    let decide = |policy: isize| {
        if !handler.is_null() {
            let block = unsafe { &*(handler as *const block2::Block<dyn Fn(isize)>) };
            block.call((policy,));
        }
    };
    if crate::navigation::subframe_blocked(main, &url) {
        let shown: String = url.chars().take(120).collect();
        tracing::warn!("畫布框（子框架）只能停在自己的沙盒文件，擋下導覽：{shown}");
        decide(CANCEL);
        return;
    }
    let raw = GATE_ORIGINAL.load(std::sync::atomic::Ordering::SeqCst);
    if raw == 0 {
        // 不會發生（換上關卡前就記下原本的實作）；保守處理：子框架擋、其餘照 Kunsu 的主框架白名單。
        decide(if main == Some(false) || !crate::navigation::allowed_navigation(&url) { CANCEL } else { ALLOW });
        return;
    }
    // SAFETY：raw 是同一個方法原本的 IMP，簽名與 DecidePolicyImp 相同。
    let original = unsafe { std::mem::transmute::<usize, DecidePolicyImp>(raw) };
    unsafe { original(this, cmd, webview, action, handler) }
}

/// 目標框架（`None`＝開新視窗，targetFrame 為 nil）與網址。
/// SAFETY：`action` 是 WebKit 交來的有效 WKNavigationAction（或 null）。
#[cfg(target_os = "macos")]
unsafe fn navigation_target(action: *mut objc2::runtime::AnyObject) -> (Option<bool>, String) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSString;
    if action.is_null() {
        return (None, String::new());
    }
    let action = unsafe { &*action };
    let frame: *mut AnyObject = unsafe { msg_send![action, targetFrame] };
    let main = if frame.is_null() {
        None
    } else {
        let main: bool = unsafe { msg_send![&*frame, isMainFrame] };
        Some(main)
    };
    let request: *mut AnyObject = unsafe { msg_send![action, request] };
    let url: *mut AnyObject = if request.is_null() { std::ptr::null_mut() } else { unsafe { msg_send![&*request, URL] } };
    let text: *mut NSString = if url.is_null() { std::ptr::null_mut() } else { unsafe { msg_send![&*url, absoluteString] } };
    (main, if text.is_null() { String::new() } else { unsafe { (*text).to_string() } })
}
