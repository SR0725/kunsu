//! 視窗三件套（CONTRACT §9 / 設計 §3）。
//!
//! - main：一般視窗（清單 + 董事會 pack），啟動隱藏，tray/膠囊叫出。
//! - capsule：邊緣瀏海舞台（PRD §19，取代浮動膠囊；細節見 `edge` 模組）。decorations:false + transparent +
//!   skipTaskbar + focusable(false)（永遠不成為 key window）；顯示時貼滿目標螢幕、level 26、全 Space，預設穿透，
//!   原生輪詢只在游標落在瀏海畫出來的多邊形上時接收滑鼠事件。只在隱藏時才顯示，已顯示時對帳不再帶到前景。
//! - overlay（蓋板）：尺寸=目標螢幕完整 frame（**非 macOS fullscreen mode**，不觸發
//!   Space 切換）、NSWindow.level=.popUpMenu（刻意不用 CGShieldingWindowLevel，
//!   不蓋系統對話框）、完全透明（不加遮罩或模糊，只看得到 UI 畫的黑色面板，AC-DS-01）。forced 轉移顯示；
//!   收起後再跑一次強制題對帳（AC-OP-07）。

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::overlay_geometry::{self, Rect};
use crate::overlay_presenter::{Cmd, DismissLedger, Presenter, Transition};

pub const MAIN: &str = "main";
pub const CAPSULE: &str = "capsule";
pub const OVERLAY: &str = "overlay";

/// 蓋板呈現握手（overlay-presentation AC-OP-04～07）。所有顯示／隱藏都經過它。
static PRESENTER: LazyLock<Mutex<Presenter>> = LazyLock::new(|| Mutex::new(Presenter::new()));
/// 本次 session 的目標螢幕 frame（Cocoa 全域點座標）；顯示前後用來驗證實際 frame。
static TARGET_FRAME: Mutex<Option<Rect>> = Mutex::new(None);
/// 蓋板 NSWindow 的 windowNumber，給原生 Esc 監聽辨識事件來源。
static OVERLAY_WINDOW_NUMBER: AtomicIsize = AtomicIsize::new(0);
/// 瀏海網頁最後回報的來源圖示中心（舞台本地 CSS 點與大小）；沒有點擊座標時當轉場起點（AC-OP-02）。
static LAST_NOTCH_ANCHOR: Mutex<Option<(f64, f64, f64)>> = Mutex::new(None);
/// 蓋板網頁最後回報的目前題目（原生 dismissal 的範圍，AC-OP-06、AC-EC-09b）；蓋板隱藏時清掉。
static OVERLAY_CURRENT: Mutex<Option<String>> = Mutex::new(None);
/// 同一個蓋板 session 每題只記一次 dismissal 的帳（AC-OP-06）。async 鎖：網頁「稍後再問」與原生 Esc 後備排隊執行、不並行，
/// 先到的記帳，晚到的跳過（`dismiss_once`）。
static DISMISS_LEDGER: LazyLock<tokio::sync::Mutex<DismissLedger>> = LazyLock::new(|| tokio::sync::Mutex::new(DismissLedger::default()));

/// 瀏海回報圖示中心（`edge_anchor`）。
pub fn set_notch_anchor(x: f64, y: f64, size: f64) {
    *LAST_NOTCH_ANCHOR.lock().unwrap_or_else(|p| p.into_inner()) = Some((x, y, size));
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn last_notch_anchor() -> Option<(f64, f64, f64)> {
    *LAST_NOTCH_ANCHOR.lock().unwrap_or_else(|p| p.into_inner())
}

/// 蓋板網頁回報目前顯示的題目（`overlay_current`）。
pub fn set_overlay_current(question_id: Option<String>) {
    *OVERLAY_CURRENT.lock().unwrap_or_else(|p| p.into_inner()) = question_id;
}

fn overlay_current() -> Option<String> {
    OVERLAY_CURRENT.lock().unwrap_or_else(|p| p.into_inner()).clone()
}
static CLOCK: LazyLock<Instant> = LazyLock::new(Instant::now);

/// 單調毫秒時鐘（握手逾時與 Esc 看門狗用；不受系統時間調整影響）。
pub fn now_ms() -> u64 {
    CLOCK.elapsed().as_millis() as u64
}

fn presenter() -> std::sync::MutexGuard<'static, Presenter> {
    PRESENTER.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 畫布卡住主執行緒的看門狗（evidence-canvas 場景 9b）。
static WATCHDOG: LazyLock<Mutex<crate::canvas_watchdog::Watchdog>> =
    LazyLock::new(|| Mutex::new(crate::canvas_watchdog::Watchdog::default()));

fn watchdog() -> std::sync::MutexGuard<'static, crate::canvas_watchdog::Watchdog> {
    WATCHDOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn canvas_active(window: &str, question_id: &str, revision: u32) -> bool {
    watchdog().canvas_active(window, question_id, revision, now_ms())
}

pub fn canvas_inactive(window: &str, question_id: &str, revision: u32) {
    watchdog().canvas_inactive(window, question_id, revision);
}

pub fn canvas_pong(window: &str) {
    watchdog().pong(window, now_ms());
}

pub fn canvas_disabled(question_id: &str, revision: u32) -> bool {
    watchdog().is_disabled(question_id, revision)
}

/// 視窗銷毀或網頁開始重新載入：清掉那個視窗的畫布追蹤（AC-EC-09b；主視窗 ⌘W 不能被誤判成卡住）。
pub fn forget_canvas_window(window: &str) {
    watchdog().forget_window(window);
}

/// 建立 Kunsu 的每一個 webview（INV-20；`webview_native` 的 `inv_canvas_sandboxed` 檢查沒有繞過的 builder）：
/// 導覽白名單；網頁開始（重新）載入時清掉畫布追蹤；建好後立刻套用引擎關閉清單（WebRTC、資源提示）；
/// 視窗銷毀時清掉畫布追蹤。
pub fn build_webview(
    builder: WebviewWindowBuilder<'_, tauri::Wry, AppHandle>,
) -> tauri::Result<tauri::WebviewWindow> {
    let window = builder
        .on_navigation(|url| crate::navigation::allowed_navigation(url.as_str()))
        .on_page_load(|window, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                forget_canvas_window(window.label());
            }
        })
        .build()?;
    crate::webview_native::harden(&window);
    let label = window.label().to_string();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            forget_canvas_window(&label);
        }
    });
    Ok(window)
}

/// 執行看門狗命令：心跳送給該視窗；卡住 → 記 canvas_failed、停用該版畫布並重新載入（蓋板先隱藏，
/// 重新載入的網頁掛載後才再顯示，INV 22）。
fn canvas_watchdog_tick(app: &AppHandle) {
    use crate::canvas_watchdog::{WdCmd, HANG_REASON};
    // 先在不持有看門狗鎖的情況下確認追蹤中的視窗還在（視窗銷毀的回呼也會拿看門狗鎖，避免互等）。
    let tracked = watchdog().tracked_windows();
    let alive: std::collections::HashSet<String> =
        tracked.into_iter().filter(|label| app.get_webview_window(label).is_some()).collect();
    let cmds = watchdog().tick(now_ms(), |label| alive.contains(label));
    for cmd in cmds {
        match cmd {
            WdCmd::Ping { window } => {
                let _ = app.emit_to(window.as_str(), "kunsu://canvas_ping", ());
            }
            WdCmd::Hung { window, question_id, revision } => {
                tracing::warn!("畫布卡住主執行緒（{question_id} 第 {revision} 版，{window}），停用並重新載入");
                let app2 = app.clone();
                tauri::async_runtime::spawn(async move {
                    let ctx = app2.state::<crate::ipc::AppCtx>();
                    let reason = format!("圖表載入失敗：{HANG_REASON}");
                    if let Err(e) = ctx.handle.record_canvas_failure(&question_id, revision, &reason, chrono::Utc::now()).await {
                        tracing::warn!("記錄 canvas_failed 失敗：{e}");
                    }
                });
                // reload() 救不回無窮迴圈：先終止那個視窗的網頁行程（只影響它自己）再重新載入。
                if let Some(w) = app.get_webview_window(&window) {
                    crate::webview_native::kill_web_process(&w);
                }
                if window == OVERLAY {
                    let (cmds, represent) = presenter().hang_recovery();
                    execute(app, cmds);
                    if represent {
                        show_overlay(app);
                    }
                } else if let Some(w) = app.get_webview_window(&window) {
                    let _ = w.reload();
                }
            }
        }
    }
}

/// 網頁（蓋板）掛載時呼叫：標記就緒並回傳目前待播的轉場（主動拉取，事件不會遺失）。
pub fn overlay_web_ready() -> Option<Transition> {
    let mut p = presenter();
    let _ = p.web_ready();
    p.pending_transition()
}

/// 網頁回報該轉場的題目、輸入與關閉控制都已渲染，才把視窗帶到前景。
pub fn overlay_mounted(app: &AppHandle, id: u64) {
    let cmds = presenter().mounted(id);
    execute(app, cmds);
}

pub fn overlay_escape_acked() {
    presenter().escape_acked();
}

/// 每 100ms 由 lib.rs 的 ticker 呼叫：處理握手逾時與 Esc 看門狗。
pub fn overlay_tick(app: &AppHandle) {
    let cmds = presenter().tick(now_ms());
    if !cmds.is_empty() {
        execute(app, cmds);
    }
    canvas_watchdog_tick(app);
}

fn execute(app: &AppHandle, cmds: Vec<Cmd>) {
    for cmd in cmds {
        match cmd {
            Cmd::Emit(transition) => {
                if let Err(error) = app.emit_to(OVERLAY, "kunsu://overlay_transition", transition) {
                    tracing::warn!("蓋板轉場事件送出失敗：{error}");
                }
            }
            Cmd::ApplyFrame => apply_target_frame(app, false),
            Cmd::ShowAndFocus => apply_target_frame(app, true),
            Cmd::Hide => {
                // session 結束：網頁回報的目前題目不再有效（NativeDismiss 一律排在 Hide 之前，已先取走）。
                set_overlay_current(None);
                if let Some(w) = app.get_webview_window(OVERLAY) {
                    let _ = w.hide();
                }
            }
            Cmd::ReloadWeb => {
                tracing::warn!("蓋板網頁 {}ms 內未回報掛載，重新載入", crate::overlay_presenter::MOUNT_TIMEOUT_MS);
                if let Some(w) = app.get_webview_window(OVERLAY) {
                    let _ = w.reload();
                }
            }
            Cmd::OpenMainFallback => {
                show_main(app);
                let _ = app.emit_to(MAIN, "kunsu://open_flow", ());
            }
            Cmd::Alert(count) => {
                let ctx = app.state::<crate::ipc::AppCtx>();
                ctx.handle.discord().send(kunsu_server::DiscordMessage {
                    question_id: None,
                    reason: "overlay_mount_timeout".to_string(),
                    text: format!("⚠️ Kunsu 強制蓋板連續 {count} 次載入逾時，已改開主視窗；題目仍保持強制資格。"),
                });
            }
            Cmd::NativeDismiss => {
                // 同步取走網頁回報的目前題目：同一批命令接著的 Hide 會清掉它。session 也在這一刻取：之後的新轉場不算。
                let current = overlay_current();
                let session = overlay_session_id();
                let app = app.clone();
                tauri::async_runtime::spawn(async move { native_dismiss(&app, current, session).await });
            }
        }
    }
}

/// 目前（或剛收起的）蓋板 session：最近一次轉場 id。
pub fn overlay_session_id() -> u64 {
    presenter().last_transition_id()
}

/// 蓋板的 dismissal（AC-OP-06）：蓋板網頁的「稍後再問」（`dismiss_question` 來自蓋板視窗）與原生 Esc 後備都經過這裡、排隊
/// 執行，同一個 session 每題只記一次。鎖內先讀最新題目：這個 session 已記過、而且之後沒有再到期就不再記，直接回傳目前
/// 題目（網頁超過 2 秒才成功、部分失敗交給原生後備、網頁晚到的那一次都不會重複）；否則記 dismissal 並記帳。
pub async fn dismiss_once(
    handle: &kunsu_server::ServerHandle,
    id: &str,
    session: u64,
) -> Result<kunsu_core::types::Question, kunsu_server::ServerError> {
    let mut ledger = DISMISS_LEDGER.lock().await;
    ledger.begin(session);
    if let Some(q) = handle.get_question(id).await?
        && ledger.already_dismissed(id, q.cycle_number, q.state == kunsu_core::types::State::Forced)
    {
        return Ok(q);
    }
    let q = handle.dismiss_question(id, chrono::Utc::now()).await?;
    ledger.record(id, q.cycle_number);
    Ok(q)
}

/// 網頁沒處理 Esc（2 秒後備、畫布卡住期間的 Esc）：原生層做與「稍後再問」相同範圍的 dismissal——所有非 internal 的
/// 強制題，加上網頁最後回報的目前題目（`overlay_presenter::dismiss_targets`）；網頁在這個 session 已記過（或正在記）的題
/// 跳過（`dismiss_once`）。
async fn native_dismiss(app: &AppHandle, current: Option<String>, session: u64) {
    use crate::overlay_presenter::{dismiss_targets, DismissCandidate};
    let ctx = app.state::<crate::ipc::AppCtx>();
    let Ok(mut open) = ctx.handle.list_questions(false).await else {
        tracing::error!("原生 Esc dismissal：讀不到題目清單");
        return;
    };
    open.sort_by_key(|q| q.force_at);
    let candidates: Vec<DismissCandidate<'_>> = open
        .iter()
        .map(|q| DismissCandidate {
            id: &q.id,
            forced: q.state == kunsu_core::types::State::Forced,
            terminal: q.state.is_terminal(),
            internal: q.internal,
        })
        .collect();
    for id in dismiss_targets(&candidates, current.as_deref()) {
        match dismiss_once(&ctx.handle, &id, session).await {
            Ok(_) => tracing::warn!("蓋板網頁未處理 Esc，原生層已稍後再問 {id}（網頁已記過的不重複）"),
            Err(error) => tracing::error!("原生 Esc dismissal 失敗（{id}）：{error}"),
        }
    }
    let _ = app.emit("kunsu://questions_changed", serde_json::json!({}));
}

/// 在主執行緒把蓋板 frame 設成目標螢幕完整範圍；`show` 時讀回驗證後才帶到前景。
fn apply_target_frame(app: &AppHandle, show: bool) {
    let Some(w) = app.get_webview_window(OVERLAY) else { return };
    let w2 = w.clone();
    let _ = w.run_on_main_thread(move || {
        let target = *TARGET_FRAME.lock().unwrap_or_else(|p| p.into_inner());
        set_native_frame(&w2, target);
        if show {
            let _ = w2.show();
            let _ = w2.set_focus();
            set_native_frame(&w2, target);
            tracing::info!("蓋板已顯示：目標 {target:?}，實際 {:?}", native_frame(&w2));
        }
    });
}

#[cfg(target_os = "macos")]
fn native_frame(w: &tauri::WebviewWindow) -> Option<Rect> {
    use objc2_app_kit::NSWindow;
    let ptr = w.ns_window().ok()?;
    let ns: &NSWindow = unsafe { &*ptr.cast::<NSWindow>() };
    let f = ns.frame();
    Some(Rect::new(f.origin.x, f.origin.y, f.size.width, f.size.height))
}

#[cfg(target_os = "macos")]
fn set_native_frame(w: &tauri::WebviewWindow, target: Option<Rect>) {
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let Some(target) = target else { return };
    if native_frame(w).is_some_and(|actual| overlay_geometry::frame_matches(actual, target)) {
        return;
    }
    if let Ok(ptr) = w.ns_window() {
        let ns: &NSWindow = unsafe { &*ptr.cast::<NSWindow>() };
        ns.setFrame_display(
            NSRect::new(NSPoint::new(target.x, target.y), NSSize::new(target.w, target.h)),
            true,
        );
        if let Some(actual) = native_frame(w)
            && !overlay_geometry::frame_matches(actual, target)
        {
            tracing::warn!("蓋板 frame 未對齊：目標 {target:?}，實際 {actual:?}");
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn set_native_frame(_w: &tauri::WebviewWindow, _target: Option<Rect>) {}

#[cfg(not(target_os = "macos"))]
fn native_frame(_w: &tauri::WebviewWindow) -> Option<Rect> {
    None
}

/// 目標螢幕（瀏海／膠囊所在 → 游標所在 → 主螢幕）與轉場起點（蓋板本地點座標）。
#[cfg(target_os = "macos")]
fn native_overlay_geometry(app: &AppHandle, anchor: Option<(f64, f64, f64)>) -> (Option<Rect>, (f64, f64, f64)) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSScreen};
    const FALLBACK: (f64, f64, f64) = (28.0, 28.0, 25.0);
    let Some(mtm) = MainThreadMarker::new() else { return (None, FALLBACK) };
    let screens: Vec<Rect> = NSScreen::screens(mtm)
        .iter()
        .map(|screen| {
            let f = screen.frame();
            Rect::new(f.origin.x, f.origin.y, f.size.width, f.size.height)
        })
        .collect();
    let capsule_frame = app
        .get_webview_window(CAPSULE)
        .filter(|w| w.is_visible().unwrap_or(false))
        .and_then(|w| native_frame(&w));
    let mouse = NSEvent::mouseLocation();
    let Some(index) = overlay_geometry::target_screen(&screens, capsule_frame, Some((mouse.x, mouse.y))) else {
        return (None, FALLBACK);
    };
    let target = screens[index];
    let origin = match capsule_frame {
        Some(frame) => {
            // 點擊 → 瀏海最後回報的圖示中心 → 估計值（AC-OP-02；舊版沒有點擊時固定在左上角附近）。
            let (local_x, local_y, size) = overlay_geometry::overlay_origin(anchor, last_notch_anchor());
            let (x, y) = overlay_geometry::anchor_in_overlay(frame, local_x, local_y, target);
            (x, y, size)
        }
        None => FALLBACK,
    };
    (Some(target), origin)
}

#[cfg(not(target_os = "macos"))]
fn native_overlay_geometry(_app: &AppHandle, _anchor: Option<(f64, f64, f64)>) -> (Option<Rect>, (f64, f64, f64)) {
    (None, (28.0, 28.0, 25.0))
}

fn capsule_drag_log(event: &str) {
    let home = std::env::var_os("KUNSU_HOME_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|value| PathBuf::from(value).join(".kunsu")));
    let Some(home) = home else { return };
    if std::fs::create_dir_all(&home).is_err() {
        return;
    }
    let path = home.join("capsule-drag.log");
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{} {event}", chrono::Utc::now().to_rfc3339());
}

/// NSWindowLevel 常數（AppKit：NSFloatingWindowLevel=3、NSPopUpMenuWindowLevel=101）。
#[cfg(target_os = "macos")]
#[cfg(target_os = "macos")]
const LEVEL_POPUP_MENU: isize = 101;

pub fn create_windows(app: &AppHandle) -> tauri::Result<()> {
    // 每個 webview 都經 build_webview：導覽白名單、引擎關閉清單（WebRTC、資源提示，INV-20）、看門狗追蹤清理。
    // main：一般視窗，啟動不搶焦點。外框（AC-AS-01）：隱藏標題、內容延伸到標題列下、最小 900×600。
    use crate::main_window::{MainWindowChrome, MAIN_MIN_HEIGHT, MAIN_MIN_WIDTH};
    let _main = build_webview(
        WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
            .title("Kunsu 軍師")
            .inner_size(1180.0, 760.0)
            .min_inner_size(MAIN_MIN_WIDTH, MAIN_MIN_HEIGHT)
            .main_window_chrome()
            .visible(false),
    )?;

    // capsule：只有待決題存在時才由 reconcile_capsule 顯示。初始先藏起來，
    // 避免 server 清單載入前閃出空膠囊或留下透明攔截區。
    let capsule = build_webview(
        WebviewWindowBuilder::new(app, CAPSULE, WebviewUrl::App("index.html?view=capsule".into()))
            .title("Kunsu Capsule")
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .accept_first_mouse(true)
            .inner_size(140.0, 44.0)
            .position(8.0, 8.0)
            .visible(false)
            .focused(false)
            // 邊緣瀏海舞台永遠不成為 key window（canBecomeKeyWindow＝NO），不搶蓋板或其他 app 的鍵盤焦點。
            .focusable(false),
    )?;

    // overlay：先建好藏著，forced 時套主螢幕 frame 再現身。
    let overlay = build_webview(
        WebviewWindowBuilder::new(app, OVERLAY, WebviewUrl::App("index.html?view=overlay".into()))
            .title("Kunsu Overlay")
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .inner_size(1280.0, 800.0)
            .visible(false),
    )?;

    apply_macos_config(&capsule, &overlay);
    Ok(())
}

#[cfg(target_os = "macos")]
fn apply_macos_config(capsule: &tauri::WebviewWindow, overlay: &tauri::WebviewWindow) {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};

    let cap = capsule.clone();
    let _ = capsule.run_on_main_thread(move || {
        if let Ok(ptr) = cap.ns_window() {
            let ns: &NSWindow = unsafe { &*ptr.cast::<NSWindow>() };
            // 邊緣瀏海舞台（PRD §19）：選單列之上、下拉選單與蓋板之下；所有 Space 與全螢幕 app 上都看得到；
            // 不進視窗循環。穿透由 edge 模組依游標切換。
            ns.setLevel(crate::edge::LEVEL_ABOVE_MENU_BAR);
            ns.setHasShadow(false);
            ns.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            ns.setIgnoresMouseEvents(true);
        }
    });

    let ov = overlay.clone();
    let _ = overlay.run_on_main_thread(move || {
        // 蓋板視窗完全透明：不加模糊或變暗層，只看得到黑色面板從瀏海展開（AC-DS-01，2026-10-09）。
        if let Ok(ptr) = ov.ns_window() {
            let ns: &NSWindow = unsafe { &*ptr.cast::<NSWindow>() };
            ns.setLevel(LEVEL_POPUP_MENU);
            ns.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
        }
        install_escape_monitor(&ov);
    });
}

#[cfg(not(target_os = "macos"))]
fn apply_macos_config(_capsule: &tauri::WebviewWindow, _overlay: &tauri::WebviewWindow) {}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 邊緣瀏海舞台：只在隱藏時貼合並顯示。已顯示時什麼都不做——每次對帳都 `show()` 會 makeKeyAndOrderFront，
/// 可能搶走蓋板的鍵盤焦點（AC-EN-15）；螢幕配置改變由 `edge::realign_stage` 處理。
pub fn show_capsule(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(CAPSULE) {
        let visible = w.is_visible().unwrap_or(false);
        if crate::edge::stage_plan(visible, None, None) == crate::edge::StagePlan::Show {
            crate::edge::prepare_stage(app);
            let _ = w.show();
        }
    }
}

pub fn hide_capsule(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(CAPSULE) {
        let _ = w.hide();
    }
}

/// 非膠囊路徑（tray / forced）：轉場起點用膠囊 Orb 的固定位置估算。
pub fn show_overlay(app: &AppHandle) {
    show_overlay_with_anchor(app, None);
}

/// 膠囊點擊路徑：傳入 WebView local CSS 座標，原生殼換算成蓋板本地點座標。
pub fn show_overlay_from_capsule(app: &AppHandle, local_x: f64, local_y: f64, size: f64) {
    show_overlay_with_anchor(app, Some((local_x, local_y, size)));
}

/// 手動或 forced 開啟後維持本次作答 session；只有 UI close／勿擾／逾時後備才結束。
pub fn overlay_session_open() -> bool {
    presenter().session_open()
}

fn show_overlay_with_anchor(app: &AppHandle, anchor: Option<(f64, f64, f64)>) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let (target, origin) = native_overlay_geometry(&app2, anchor);
        let source = if anchor.is_some() { "capsule" } else { "system" };
        let cmds = {
            let mut p = presenter();
            if !p.session_open() {
                *TARGET_FRAME.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = target;
            }
            p.request_show(origin, source, now_ms())
        };
        execute(&app2, cmds);
    });
}

pub fn hide_overlay(app: &AppHandle) {
    let cmds = presenter().hide();
    execute(app, cmds);
}

/// 螢幕重排／插拔／睡眠喚醒（AC-OP-03）：蓋板可見時由 250ms 主執行緒輪詢重新對齊。
#[cfg(target_os = "macos")]
fn realign_visible_overlay(app: &AppHandle) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSScreen};
    let Some(w) = app.get_webview_window(OVERLAY) else { return };
    if !w.is_visible().unwrap_or(false) {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let screens: Vec<Rect> = NSScreen::screens(mtm)
        .iter()
        .map(|screen| {
            let f = screen.frame();
            Rect::new(f.origin.x, f.origin.y, f.size.width, f.size.height)
        })
        .collect();
    let mut target_slot = TARGET_FRAME.lock().unwrap_or_else(|p| p.into_inner());
    let still_present = target_slot.is_some_and(|t| screens.iter().any(|s| overlay_geometry::frame_matches(*s, t)));
    if !still_present {
        let mouse = NSEvent::mouseLocation();
        let index = overlay_geometry::target_screen(&screens, native_frame(&w), Some((mouse.x, mouse.y)));
        *target_slot = index.map(|i| screens[i]);
        tracing::warn!("螢幕配置改變，蓋板改對齊 {:?}", *target_slot);
    }
    let target = *target_slot;
    drop(target_slot);
    set_native_frame(&w, target);
}

/// 原生 Esc 監聽：不吞事件（輸入法取消組字照常），只啟動看門狗；網頁 2 秒內沒回應才由原生完成。
#[cfg(target_os = "macos")]
fn install_escape_monitor(overlay: &tauri::WebviewWindow) {
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask, NSWindow};
    use std::ptr::NonNull;
    if let Ok(ptr) = overlay.ns_window() {
        let ns: &NSWindow = unsafe { &*ptr.cast::<NSWindow>() };
        OVERLAY_WINDOW_NUMBER.store(ns.windowNumber(), Ordering::Relaxed);
    }
    let block = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        let ev: &NSEvent = unsafe { event.as_ref() };
        if ev.keyCode() == 53 && ev.windowNumber() == OVERLAY_WINDOW_NUMBER.load(Ordering::Relaxed) {
            presenter().escape_pressed(now_ms());
        }
        event.as_ptr()
    });
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block) };
    // 監聽器在 app 生命週期內常駐。
    std::mem::forget(monitor);
}

/// 膠囊視窗貼合內容（`resize_capsule` IPC，CONTRACT §9）：
/// UI ResizeObserver 回報內容 CSS px → LogicalSize（Tauri 自處理 scale factor）。
/// clamp 到理智範圍，杜絕任何情況下留大面積攔截區。
pub fn resize_capsule(app: &AppHandle, w: f64, h: f64) {
    if let Some(win) = app.get_webview_window(CAPSULE) {
        let w = w.clamp(60.0, 480.0);
        let h = h.clamp(32.0, 400.0);
        let old_position = win.outer_position().ok();
        let old_size = win.outer_size().ok();
        let monitor = win.current_monitor().ok().flatten();
        let scale = win.scale_factor().unwrap_or(1.0);
        let physical = tauri::LogicalSize::new(w, h).to_physical::<u32>(scale);
        if let Err(e) = win.set_size(tauri::LogicalSize::new(w, h)) {
            tracing::warn!("膠囊視窗貼合內容失敗：{e}");
        }
        // 在右/下半邊時從該側向內長，避免 hover 預覽跑出螢幕。
        if let (Some(position), Some(old), Some(monitor)) = (old_position, old_size, monitor) {
            let frame = monitor.position();
            let screen = monitor.size();
            let right = position.x + old.width as i32 / 2 > frame.x + screen.width as i32 / 2;
            let bottom = position.y + old.height as i32 / 2 > frame.y + screen.height as i32 / 2;
            let x = if right { position.x + old.width as i32 - physical.width as i32 } else { position.x };
            let y = if bottom { position.y + old.height as i32 - physical.height as i32 } else { position.y };
            let max_x = frame.x + screen.width as i32 - physical.width as i32 - 8;
            let max_y = frame.y + screen.height as i32 - physical.height as i32 - 8;
            let _ = win.set_position(tauri::PhysicalPosition::new(
                x.clamp(frame.x + 8, max_x.max(frame.x + 8)),
                y.clamp(frame.y + 8, max_y.max(frame.y + 8)),
            ));
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CapsuleDragGeometry {
    pub x: i32,
    pub y: i32,
    pub scale_factor: f64,
}

pub fn capsule_drag_geometry(app: &AppHandle) -> Option<CapsuleDragGeometry> {
    let Some(win) = app.get_webview_window(CAPSULE) else {
        capsule_drag_log("geometry error=no_capsule_window");
        return None;
    };
    let Ok(position) = win.outer_position() else {
        capsule_drag_log("geometry error=no_outer_position");
        return None;
    };
    let Ok(scale_factor) = win.scale_factor() else {
        capsule_drag_log("geometry error=no_scale_factor");
        return None;
    };
    capsule_drag_log(&format!(
        "pointer_down geometry_x={} geometry_y={} scale_factor={scale_factor}",
        position.x, position.y
    ));
    Some(CapsuleDragGeometry { x: position.x, y: position.y, scale_factor })
}

pub fn move_capsule(app: &AppHandle, x: i32, y: i32) -> bool {
    let Some(win) = app.get_webview_window(CAPSULE) else {
        capsule_drag_log(&format!("move x={x} y={y} result=no_capsule_window"));
        return false;
    };
    let result = win.set_position(tauri::PhysicalPosition::new(x, y));
    capsule_drag_log(&format!(
        "move x={x} y={y} result={}",
        if result.is_ok() { "ok" } else { "error" }
    ));
    result.is_ok()
}

fn cubic_bezier_coordinate(t: f64, p1: f64, p2: f64) -> f64 {
    let inv = 1.0 - t;
    3.0 * inv * inv * t * p1 + 3.0 * inv * t * t * p2 + t * t * t
}

fn cubic_bezier_derivative(t: f64, p1: f64, p2: f64) -> f64 {
    let inv = 1.0 - t;
    3.0 * inv * inv * p1 + 6.0 * inv * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

/// DESIGN.md surface curve：cubic-bezier(.16, 1, .3, 1)。
fn capsule_snap_ease(progress: f64) -> f64 {
    let x = progress.clamp(0.0, 1.0);
    let mut t = x;
    for _ in 0..6 {
        let error = cubic_bezier_coordinate(t, 0.16, 0.3) - x;
        let derivative = cubic_bezier_derivative(t, 0.16, 0.3);
        if derivative.abs() < 1e-6 {
            break;
        }
        t = (t - error / derivative).clamp(0.0, 1.0);
    }
    cubic_bezier_coordinate(t, 1.0, 1.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CapsuleDockTarget {
    x: i32,
    y: i32,
    dock: &'static str,
}

#[allow(clippy::too_many_arguments)]
fn nearest_edge_dock_target(
    frame_x: i32,
    frame_y: i32,
    screen_width: i32,
    screen_height: i32,
    window_x: i32,
    window_y: i32,
    window_width: i32,
    window_height: i32,
    corner_magnet: i32,
) -> CapsuleDockTarget {
    const MARGIN: i32 = 12;
    let min_x = frame_x + MARGIN;
    let min_y = frame_y + MARGIN;
    let max_x = (frame_x + screen_width - window_width - MARGIN).max(min_x);
    let max_y = (frame_y + screen_height - window_height - MARGIN).max(min_y);
    let axis_x = window_x.clamp(min_x, max_x);
    let axis_y = window_y.clamp(min_y, max_y);

    let distances = [
        (window_y.abs_diff(min_y), "top"),
        (window_x.abs_diff(max_x), "right"),
        (window_y.abs_diff(max_y), "bottom"),
        (window_x.abs_diff(min_x), "left"),
    ];
    let edge = distances
        .into_iter()
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, edge)| edge)
        .unwrap_or("top");

    let snap_horizontal_corner = |x: i32| {
        if x - min_x <= corner_magnet {
            min_x
        } else if max_x - x <= corner_magnet {
            max_x
        } else {
            x
        }
    };
    let snap_vertical_corner = |y: i32| {
        if y - min_y <= corner_magnet {
            min_y
        } else if max_y - y <= corner_magnet {
            max_y
        } else {
            y
        }
    };

    let mid_x = min_x + (max_x - min_x) / 2;
    let mid_y = min_y + (max_y - min_y) / 2;
    match edge {
        "right" => {
            let y = snap_vertical_corner(axis_y);
            CapsuleDockTarget {
                x: max_x,
                y,
                dock: if y <= mid_y {
                    "right-top"
                } else {
                    "right-bottom"
                },
            }
        }
        "bottom" => {
            let x = snap_horizontal_corner(axis_x);
            CapsuleDockTarget {
                x,
                y: max_y,
                dock: if x <= mid_x {
                    "bottom-left"
                } else {
                    "bottom-right"
                },
            }
        }
        "left" => {
            let y = snap_vertical_corner(axis_y);
            CapsuleDockTarget {
                x: min_x,
                y,
                dock: if y <= mid_y {
                    "left-top"
                } else {
                    "left-bottom"
                },
            }
        }
        _ => {
            let x = snap_horizontal_corner(axis_x);
            CapsuleDockTarget {
                x,
                y: min_y,
                dock: if x <= mid_x { "top-left" } else { "top-right" },
            }
        }
    }
}

/// 拖曳結束後以 320ms surface curve 投影到最近邊界；接近角落才磁吸到角落。
/// reduced motion 立即定位。
pub async fn snap_capsule(app: &AppHandle, reduced_motion: bool) -> Option<&'static str> {
    let Some(win) = app.get_webview_window(CAPSULE) else {
        return None;
    };
    let Ok(Some(monitor)) = win.current_monitor() else {
        return None;
    };
    let Ok(position) = win.outer_position() else {
        return None;
    };
    let Ok(size) = win.outer_size() else {
        return None;
    };
    let frame = monitor.position();
    let screen = monitor.size();
    let scale_factor = win.scale_factor().unwrap_or(1.0);
    let target = nearest_edge_dock_target(
        frame.x,
        frame.y,
        screen.width as i32,
        screen.height as i32,
        position.x,
        position.y,
        size.width as i32,
        size.height as i32,
        (64.0 * scale_factor).round() as i32,
    );
    let x = target.x;
    let y = target.y;
    let dock = target.dock;
    let start_x = position.x;
    let start_y = position.y;
    capsule_drag_log(&format!(
        "snap_start from_x={start_x} from_y={start_y} to_x={x} to_y={y} dock={dock} reduced={reduced_motion}"
    ));
    if reduced_motion {
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    } else {
        let started = std::time::Instant::now();
        let duration = std::time::Duration::from_millis(320);
        loop {
            let progress = (started.elapsed().as_secs_f64() / duration.as_secs_f64()).min(1.0);
            let eased = capsule_snap_ease(progress);
            let next_x = start_x as f64 + (x - start_x) as f64 * eased;
            let next_y = start_y as f64 + (y - start_y) as f64 * eased;
            let _ = win.set_position(tauri::PhysicalPosition::new(
                next_x.round() as i32,
                next_y.round() as i32,
            ));
            if progress >= 1.0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
        }
    }
    capsule_drag_log(&format!("snap_end x={x} y={y} dock={dock}"));
    Some(dock)
}

#[cfg(test)]
mod motion_tests {
    use super::{CapsuleDockTarget, capsule_snap_ease, nearest_edge_dock_target};

    #[test]
    fn capsule_snap_curve_has_exact_endpoints_and_monotonic_progress() {
        assert_eq!(capsule_snap_ease(0.0), 0.0);
        assert_eq!(capsule_snap_ease(1.0), 1.0);
        let samples: Vec<f64> =
            (0..=20).map(|step| capsule_snap_ease(step as f64 / 20.0)).collect();
        assert!(samples.windows(2).all(|pair| pair[1] >= pair[0]));
        assert!(capsule_snap_ease(0.5) > 0.85, "surface curve 前快後柔和");
    }

    #[test]
    fn capsule_projects_to_nearest_edge_without_losing_axis_position() {
        let top = nearest_edge_dock_target(0, 0, 1000, 800, 400, 100, 150, 50, 64);
        assert_eq!(
            top,
            CapsuleDockTarget {
                x: 400,
                y: 12,
                dock: "top-left"
            }
        );

        let right = nearest_edge_dock_target(0, 0, 1000, 800, 600, 300, 150, 50, 64);
        assert_eq!(
            right,
            CapsuleDockTarget {
                x: 838,
                y: 300,
                dock: "right-top"
            }
        );

        let bottom = nearest_edge_dock_target(0, 0, 1000, 800, 240, 710, 150, 50, 64);
        assert_eq!(
            bottom,
            CapsuleDockTarget {
                x: 240,
                y: 738,
                dock: "bottom-left"
            }
        );
    }

    #[test]
    fn capsule_uses_corner_magnet_only_near_edge_endpoints() {
        let corner = nearest_edge_dock_target(0, 0, 1000, 800, 30, 25, 150, 50, 64);
        assert_eq!(
            corner,
            CapsuleDockTarget {
                x: 12,
                y: 12,
                dock: "top-left"
            }
        );

        let free_edge = nearest_edge_dock_target(0, 0, 1000, 800, 200, 25, 150, 50, 64);
        assert_eq!(
            free_edge,
            CapsuleDockTarget {
                x: 200,
                y: 12,
                dock: "top-left"
            }
        );
    }

}

/// main-thread 輪詢（250ms）：前景全螢幕 guard——NSWorkspace 前景 app
/// （bundle id 黑名單）+ CGWindowList 蓋滿主螢幕判定，寫入 SharedGuardFlags
/// 原子旗標。（膠囊 hover 穿透輪詢已移除：視窗貼合內容後全程可點。）
#[cfg(target_os = "macos")]
pub fn poll_main_thread(_app: &AppHandle, flags: &crate::guards::SharedGuardFlags) {
    use objc2_app_kit::NSWorkspace;

    // --- 前景全螢幕 guard（每項獨立 try，失敗=false+log）---
    let fullscreen = std::panic::catch_unwind(|| -> bool {
        let ws = NSWorkspace::sharedWorkspace();
        let Some(front) = ws.frontmostApplication() else {
            return false;
        };
        if let Some(bundle) = front.bundleIdentifier() {
            let bundle = bundle.to_string();
            if crate::guards::FULLSCREEN_BLACKLIST.iter().any(|b| bundle == *b) {
                return true;
            }
            // 自家蓋板不算前景全螢幕。
            if bundle == "me.ray.kunsu" {
                return false;
            }
        }
        let pid = front.processIdentifier();
        match crate::guards::ffi::pid_has_fullscreen_window(pid) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("fullscreen 判定失敗（視為 false）：{e}");
                false
            }
        }
    })
    .unwrap_or_else(|_| {
        tracing::error!("fullscreen 判定 panic（已吸收，視為 false）");
        false
    });
    flags.frontmost_fullscreen.store(fullscreen, Ordering::Relaxed);
    realign_visible_overlay(_app);
    // 螢幕配置改變時邊緣瀏海舞台也重新貼合（AC-EN-22）。
    crate::edge::realign_stage(_app);
}

#[cfg(not(target_os = "macos"))]
pub fn poll_main_thread(_app: &AppHandle, _flags: &crate::guards::SharedGuardFlags) {}
