//! 邊緣瀏海舞台（PRD §19、BDD edge-notch、INV `inv_edge_notch_click_through`）。
//!
//! 沿用膠囊視窗（label `capsule`）的生命週期：有待決題才顯示。顯示時視窗貼滿目標螢幕（含選單列區域）、
//! 透明、預設穿透、永遠不成為 key window；網頁回報瀏海畫出來的多邊形，原生層輪詢游標
//! （[`crate::edge_hit::decide`]），只有游標在瀏海上、在瀏海上按下還沒放開（[`crate::edge_hit::press_latch`]）或正在
//! 拖曳時才接收滑鼠事件；滑鼠鍵全部放開就由原生層結束接管（網頁回報的拖曳另通知網頁）。輪詢同時負責游標進出通知
//! （網頁可能漏收 pointerleave）與隱私 guard 推送。螢幕以 CGDirectDisplayID 識別、名稱備援（場景 3d）。

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::edge_hit::{self, Grab, HitShape};
use crate::overlay_geometry::{self, Rect};
use crate::windows::CAPSULE;

/// 網頁在拖曳被原生層結束時收到這個事件（滑鼠鍵已放開但網頁漏收 pointerup，或拖曳／按著時舞台被隱藏）。
pub const DRAG_END_EVENT: &str = "kunsu://edge_drag_end";
/// 游標進出命中區（`{inside}`；在命中區內每秒重送）。
pub const POINTER_EVENT: &str = "kunsu://edge_pointer";
/// 隱私 guard 改變（`{active}`）。
pub const PRIVACY_EVENT: &str = "kunsu://edge_privacy";
/// 螢幕配置改變、舞台重新貼合或貼合的螢幕識別改變後通知網頁重讀 `edge_info`（只重新貼合與重讀，不代表位置被重設）。
pub const STAGE_EVENT: &str = "kunsu://edge_stage_changed";
/// 設定頁「回到預設位置」刪掉保存檔後通知網頁：取消待寫的存檔、清掉記憶體位置、回到預設位置（BDD edge-notch 場景 3e）。
pub const POSITION_RESET_EVENT: &str = "kunsu://edge_position_reset";
/// 選單列之上（主選單 24、狀態列 25），低於下拉選單（101）與強制蓋板。
pub const LEVEL_ABOVE_MENU_BAR: isize = 26;
/// 舞台隱藏時的閒置檢查間隔。
const IDLE_POLL_MS: u64 = 500;
/// 舞台可見時隱私 guard 的偵測間隔。
const PRIVACY_POLL_MS: u64 = 500;
/// 系統選單列的標準高度（macOS 11 起非瀏海螢幕 24 pt）：選單列自動隱藏時預設位置仍至少留這麼多（場景 3）。
pub const MENU_BAR_STANDARD: f64 = 24.0;

#[derive(Debug, Default)]
struct EdgeState {
    shapes: Vec<HitShape>,
    dragging: bool,
    /// 原生按下鎖定：在瀏海上按下、滑鼠鍵還沒放開。
    held: bool,
    /// 上一次輪詢看到的滑鼠鍵狀態（舞台剛顯示時當作按著：必須先看到放開才會鎖定）。
    buttons_down: bool,
    /// 目前是否穿透（避免每次輪詢都呼叫 AppKit）。
    ignoring: Option<bool>,
    /// 上次送給網頁的進出狀態與時間。
    pointer: Option<bool>,
    pointer_at_ms: u64,
    /// 上次推送的隱私狀態與偵測時間。
    privacy: Option<bool>,
    privacy_at_ms: Option<u64>,
    /// 記住的螢幕（None＝還沒讀過保存檔）。
    display: Option<SavedDisplay>,
    /// 舞台上次貼合的螢幕識別（None＝還沒貼合過）。
    fitted: Option<StageDisplay>,
}

static STATE: Mutex<EdgeState> = Mutex::new(EdgeState {
    shapes: Vec::new(),
    dragging: false,
    held: false,
    buttons_down: true,
    ignoring: None,
    pointer: None,
    pointer_at_ms: 0,
    privacy: None,
    privacy_at_ms: None,
    display: None,
    fitted: None,
});

fn state() -> std::sync::MutexGuard<'static, EdgeState> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

/// 網頁回報命中區與拖曳狀態。開始拖曳時立刻接管，不等下一次輪詢；命中區被清空（瀏海不再畫出）時立即解除拖曳與按下鎖定並
/// 恢復穿透（[`edge_hit::grab_after_report`]）。
pub fn set_hit(app: &AppHandle, shapes: Vec<HitShape>, dragging: bool) {
    let (start_drag, cleared) = {
        let mut s = state();
        let empty = shapes.is_empty();
        let grab = edge_hit::grab_after_report(Grab { dragging: s.dragging, pressed: s.held }, empty, dragging);
        let start = grab.dragging && !s.dragging;
        s.shapes = shapes;
        s.dragging = grab.dragging;
        s.held = grab.pressed;
        (start, empty)
    };
    if start_drag {
        apply_ignore(app, false);
    } else if cleared {
        apply_ignore(app, true);
    }
}

fn apply_ignore(app: &AppHandle, ignore: bool) {
    {
        let mut s = state();
        if s.ignoring == Some(ignore) {
            return;
        }
        s.ignoring = Some(ignore);
    }
    if let Some(w) = app.get_webview_window(CAPSULE) {
        let _ = w.set_ignore_cursor_events(ignore);
    }
}

/// 舞台該做什麼（場景 3c／15）：隱藏時貼合並顯示；已顯示時只在 frame 和目標螢幕不一致時重新貼合，
/// 絕不重新顯示（重新顯示會 makeKeyAndOrderFront，可能搶走蓋板的鍵盤焦點）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagePlan {
    Keep,
    Show,
    Refit,
}

pub fn stage_plan(visible: bool, actual: Option<Rect>, target: Option<Rect>) -> StagePlan {
    if !visible {
        return StagePlan::Show;
    }
    match (actual, target) {
        (Some(a), Some(t)) if !overlay_geometry::frame_matches(a, t) => StagePlan::Refit,
        _ => StagePlan::Keep,
    }
}

/// 記住的螢幕：保存檔只讀一次，之後由 `save` 更新。
fn saved_display() -> SavedDisplay {
    let mut s = state();
    s.display.get_or_insert_with(|| read_saved().map(|v| SavedDisplay::from_file(&v)).unwrap_or_default()).clone()
}

/// 舞台被隱藏（輪詢看到）或重新顯示前：解除網頁回報的拖曳與按下鎖定，之前有在接管就通知網頁照「沒畫出瀏海」清理
/// （[`edge_hit::grab_after_hide`]；INV `inv_edge_notch_click_through`）。
fn release_on_hide(app: &AppHandle) {
    let notify = {
        let mut s = state();
        let (grab, notify) = edge_hit::grab_after_hide(Grab { dragging: s.dragging, pressed: s.held });
        s.dragging = grab.dragging;
        s.held = grab.pressed;
        notify
    };
    if notify {
        tracing::info!("瀏海舞台在拖曳或按著時被隱藏：解除接管並通知網頁");
        let _ = app.emit_to(CAPSULE, DRAG_END_EVENT, ());
    }
}

/// 舞台準備（顯示前）：貼滿目標螢幕、預設穿透、清掉按下鎖定（隱藏期間輪詢沒看到的拖曳也一併解除並通知網頁）。貼合的
/// 螢幕識別和上次不同（即使尺寸相同）就通知網頁重讀 `edge_info`（AC-EN-03／22）。
pub fn prepare_stage(app: &AppHandle) {
    release_on_hide(app);
    {
        let mut s = state();
        s.buttons_down = true;
        s.ignoring = None;
        s.pointer = None;
        s.privacy = None;
        s.privacy_at_ms = None;
    }
    apply_ignore(app, true);
    #[cfg(target_os = "macos")]
    {
        let Some(w) = app.get_webview_window(CAPSULE) else { return };
        let saved = saved_display();
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let Some(target) = target_screen(&saved) else { return };
            set_stage_frame(&w, target.frame);
            if remember_stage_display(StageDisplay::new(target.id, &target.name)) {
                let _ = handle.emit_to(CAPSULE, STAGE_EVENT, ());
            }
        });
    }
}

/// 舞台貼合的螢幕識別（與網頁 `stageIdentity` 一致：有 CGDirectDisplayID 用它，沒有才用名稱）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageDisplay {
    Id(u32),
    Name(String),
}

impl StageDisplay {
    pub fn new(id: Option<u32>, name: &str) -> Self {
        match id {
            Some(id) => StageDisplay::Id(id),
            None => StageDisplay::Name(name.to_string()),
        }
    }
}

/// 貼合的螢幕和上次不同（含第一次貼合）就要通知網頁重讀 `edge_info`（AC-EN-03／22）：只看識別，不看尺寸——舞台隱藏期間
/// 換到另一台同尺寸的螢幕，網頁收不到 resize，也要改用新螢幕的識別與保存值。
pub fn stage_display_changed(prev: Option<&StageDisplay>, next: &StageDisplay) -> bool {
    prev != Some(next)
}

/// 記下舞台這次貼合的螢幕，回傳是否和上次不同。
#[cfg(target_os = "macos")]
fn remember_stage_display(next: StageDisplay) -> bool {
    let mut s = state();
    let changed = stage_display_changed(s.fitted.as_ref(), &next);
    s.fitted = Some(next);
    changed
}

/// 螢幕配置改變（解析度、睡眠喚醒、插拔；AC-EN-22）：由 250 ms 主執行緒輪詢呼叫。舞台可見且 frame 和目標螢幕
/// 不一致時重新貼合並通知網頁重讀；frame 一致但目標螢幕的識別換了（同尺寸的另一台）也通知。不重新顯示、不取得焦點。
#[cfg(target_os = "macos")]
pub fn realign_stage(app: &AppHandle) {
    let Some(w) = app.get_webview_window(CAPSULE) else { return };
    let visible = w.is_visible().unwrap_or(false);
    if !visible {
        return;
    }
    let Some(target) = target_screen(&saved_display()) else { return };
    let refit = stage_plan(visible, stage_frame(&w), Some(target.frame)) == StagePlan::Refit;
    if refit {
        tracing::info!("螢幕配置改變，瀏海舞台重新貼合 {:?}", target.frame);
        set_stage_frame(&w, target.frame);
    }
    let changed = remember_stage_display(StageDisplay::new(target.id, &target.name));
    if refit || changed {
        let _ = app.emit_to(CAPSULE, STAGE_EVENT, ());
    }
}

#[cfg(not(target_os = "macos"))]
pub fn realign_stage(_app: &AppHandle) {}

#[cfg(target_os = "macos")]
fn stage_frame(w: &tauri::WebviewWindow) -> Option<Rect> {
    use objc2_app_kit::NSWindow;
    let ptr = w.ns_window().ok()?;
    // SAFETY：Tauri 的 ns_window() 是膠囊視窗的 NSWindow；呼叫端在主執行緒。
    let ns: &NSWindow = unsafe { &*ptr.cast::<NSWindow>() };
    let f = ns.frame();
    Some(Rect::new(f.origin.x, f.origin.y, f.size.width, f.size.height))
}

#[cfg(target_os = "macos")]
fn set_stage_frame(w: &tauri::WebviewWindow, frame: Rect) {
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let Ok(ptr) = w.ns_window() else { return };
    // SAFETY：同上，主執行緒。
    let ns: &NSWindow = unsafe { &*ptr.cast::<NSWindow>() };
    ns.setFrame_display(NSRect::new(NSPoint::new(frame.x, frame.y), NSSize::new(frame.w, frame.h)), true);
}

/// 保存檔記住的螢幕（場景 3d）：新格式有螢幕識別（CGDirectDisplayID）與名稱；舊格式只有名稱。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedDisplay {
    pub id: Option<u32>,
    pub name: Option<String>,
}

impl SavedDisplay {
    pub fn from_file(v: &Value) -> Self {
        SavedDisplay {
            id: v.get("display_id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()),
            name: v.get("display").and_then(Value::as_str).map(str::to_string),
        }
    }
}

/// 目前螢幕清單裡哪一台是記住的螢幕：先比識別；找不到識別（或舊檔只記名稱）時，名稱恰好對到一台才採用（兩台同名不猜）。
/// `screens` 是 `(識別, 名稱)`，第 0 個是主螢幕。
pub fn match_screen(screens: &[(Option<u32>, &str)], saved: &SavedDisplay) -> Option<usize> {
    if let Some(id) = saved.id
        && let Some(i) = screens.iter().position(|s| s.0 == Some(id))
    {
        return Some(i);
    }
    let name = saved.name.as_deref()?;
    let mut named = screens.iter().enumerate().filter(|(_, s)| s.1 == name).map(|(i, _)| i);
    let first = named.next()?;
    named.next().is_none().then_some(first)
}

/// 舞台的目標螢幕：記住的螢幕，否則主螢幕（第 0 個）。
pub fn pick_screen(screens: &[(Option<u32>, &str)], saved: &SavedDisplay) -> usize {
    match_screen(screens, saved).unwrap_or(0)
}

/// 預設位置要避開的上方高度（場景 3）：選單列實際佔用（visibleFrame 推得）、實體瀏海（safeAreaInsets.top）與系統選單列
/// 標準高度（`max(NSStatusBar.system.thickness, 24)`）三者最大。選單列自動隱藏時 visibleFrame 推得 0，仍留標準高度。
pub fn top_inset(menu_bar: f64, safe_top: f64, status_bar: f64) -> f64 {
    menu_bar.max(safe_top).max(status_bar.max(MENU_BAR_STANDARD))
}

#[cfg(target_os = "macos")]
struct ScreenInfo {
    id: Option<u32>,
    name: String,
    frame: Rect,
    /// 預設位置要避開的上方高度（點）。
    top_inset: f64,
}

/// 螢幕識別：`NSScreen.deviceDescription["NSScreenNumber"]`（＝CGDirectDisplayID）。
#[cfg(target_os = "macos")]
fn screen_id(screen: &objc2_app_kit::NSScreen) -> Option<u32> {
    use objc2_foundation::{NSNumber, NSString};
    let desc = screen.deviceDescription();
    let value = desc.objectForKey(&NSString::from_str("NSScreenNumber"))?;
    value.downcast_ref::<NSNumber>().map(|n| n.unsignedIntValue())
}

/// 目前所有螢幕（第 0 個是主螢幕）；不在主執行緒時是空的。
#[cfg(target_os = "macos")]
fn screens_now() -> Vec<ScreenInfo> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSScreen, NSStatusBar};
    let Some(mtm) = MainThreadMarker::new() else { return Vec::new() };
    let status_bar = NSStatusBar::systemStatusBar().thickness();
    NSScreen::screens(mtm)
        .iter()
        .map(|screen| {
            let f = screen.frame();
            let v = screen.visibleFrame();
            let menu = (f.origin.y + f.size.height) - (v.origin.y + v.size.height);
            ScreenInfo {
                id: screen_id(&screen),
                name: screen.localizedName().to_string(),
                frame: Rect::new(f.origin.x, f.origin.y, f.size.width, f.size.height),
                top_inset: top_inset(menu, screen.safeAreaInsets().top, status_bar),
            }
        })
        .collect()
}

/// 螢幕清單的識別鍵 `(識別, 名稱)`，給 [`match_screen`] 比對。
fn screen_keys<'a>(screens: impl IntoIterator<Item = (Option<u32>, &'a str)>) -> Vec<(Option<u32>, &'a str)> {
    screens.into_iter().collect()
}

/// 目標螢幕：記住的螢幕，否則主螢幕。
#[cfg(target_os = "macos")]
fn target_screen(saved: &SavedDisplay) -> Option<ScreenInfo> {
    let infos = screens_now();
    let index = pick_screen(&screen_keys(infos.iter().map(|s| (s.id, s.name.as_str()))), saved);
    infos.into_iter().nth(index)
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeInfo {
    pub width: f64,
    pub height: f64,
    /// 預設位置要避開的上方高度（選單列、實體瀏海與系統選單列標準高度三者最大）：預設頂端在它下面 8 px。
    pub menubar: f64,
    /// 螢幕名稱（備援識別）。
    pub display: String,
    /// 螢幕識別（CGDirectDisplayID）；取不到為 None。
    pub display_id: Option<u32>,
    /// 上次保存的位置（網頁的 SavedPosition；只回格式正確且記住的螢幕就是這台時的值）。
    pub saved: Option<Value>,
}

/// 舞台資訊：螢幕大小、選單列高度、螢幕名稱與識別、保存的位置。隱私 guard 另由 `edge_privacy` 與推送提供。
pub async fn info(app: &AppHandle) -> EdgeInfo {
    let saved_file = read_saved();
    let saved_display = saved_file.as_ref().map(SavedDisplay::from_file).unwrap_or_default();
    /// 主執行緒讀到的舞台螢幕：大小、上方高度、名稱、識別，以及全部螢幕的識別鍵與舞台螢幕的索引。
    type Stage = (f64, f64, f64, String, Option<u32>, Vec<(Option<u32>, String)>, usize);
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<Stage>>();
    let _ = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        let result = {
            let infos = screens_now();
            let index = pick_screen(&screen_keys(infos.iter().map(|s| (s.id, s.name.as_str()))), &saved_display);
            let keys = infos.iter().map(|s| (s.id, s.name.clone())).collect();
            infos.get(index).map(|s| (s.frame.w, s.frame.h, s.top_inset, s.name.clone(), s.id, keys, index))
        };
        #[cfg(not(target_os = "macos"))]
        let result = {
            let _ = saved_display;
            None
        };
        let _ = tx.send(result);
    });
    let (width, height, menubar, display, display_id, keys, index) =
        rx.await.ok().flatten().unwrap_or((1440.0, 900.0, MENU_BAR_STANDARD, String::new(), None, Vec::new(), 0));
    let saved = saved_for_screen(saved_file, &screen_keys(keys.iter().map(|(id, name)| (*id, name.as_str()))), index);
    EdgeInfo { width, height, menubar, display, display_id, saved }
}

/// 保存的位置只在記住的螢幕就是 `index` 那台時套用（場景 3d），且格式要正確；其餘（別的螢幕、壞值、沒有檔案）一律
/// None＝用預設位置。
pub fn saved_for_screen(file: Option<Value>, screens: &[(Option<u32>, &str)], index: usize) -> Option<Value> {
    let file = file?;
    if match_screen(screens, &SavedDisplay::from_file(&file)) != Some(index) {
        return None;
    }
    file.get("position").filter(|p| valid_position(p)).cloned()
}

/// 保存檔內容：螢幕識別、名稱（備援）與位置。
pub fn saved_body(display_id: Option<u32>, display: &str, position: Value) -> Value {
    serde_json::json!({ "display_id": display_id, "display": display, "position": position })
}

fn ratio_ok(v: &Value) -> bool {
    v.as_f64().is_some_and(|r| r.is_finite() && (0.0..=1.0).contains(&r))
}

/// 位置格式（場景 3b）：`{corner: tl|tr|br|bl}`、`{edge: top|right|bottom|left, head: 0..1}`（直邊頭端比例），或舊格式
/// `{edge, ratio: 0..1}`（中心比例，只讀相容）。有 `head` 欄位就只看它，壞掉就整筆不合格。
pub fn valid_position(v: &Value) -> bool {
    let Some(o) = v.as_object() else { return false };
    if let Some(corner) = o.get("corner") {
        return matches!(corner.as_str(), Some("tl" | "tr" | "br" | "bl"));
    }
    let edge_ok = matches!(o.get("edge").and_then(Value::as_str), Some("top" | "right" | "bottom" | "left"));
    let ratio_ok = match o.get("head") {
        Some(head) => ratio_ok(head),
        None => o.get("ratio").is_some_and(ratio_ok),
    };
    edge_ok && ratio_ok
}

fn saved_path() -> std::path::PathBuf {
    crate::server_boot::kunsu_dir().join("edge-notch.json")
}

fn read_saved() -> Option<Value> {
    std::fs::read(saved_path()).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// 保存位置（螢幕識別＋名稱＋角落或邊與頭端比例）。格式不對直接拒絕，不碰檔案。
pub fn save(display_id: Option<u32>, display: &str, position: Value) -> Result<(), String> {
    if !valid_position(&position) {
        return Err(format!("瀏海位置格式不正確：{position}"));
    }
    let body = saved_body(display_id, display, position);
    std::fs::write(saved_path(), serde_json::to_vec_pretty(&body).unwrap_or_default()).map_err(|e| {
        tracing::warn!("瀏海位置保存失敗：{e}");
        format!("瀏海位置保存失敗：{e}")
    })?;
    state().display = Some(SavedDisplay { id: display_id, name: Some(display.to_string()) });
    Ok(())
}

/// 刪掉保存的瀏海位置檔（沒有檔案也算成功）。
pub fn reset_saved_at(path: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("刪除保存的瀏海位置失敗：{e}")),
    }
}

/// 記住的螢幕改回「沒有記住」＝主螢幕（之後不再讀舊檔）。
pub fn forget_saved_display() {
    state().display = Some(SavedDisplay::default());
}

/// 「回到預設位置」的檔案與記憶體部分：刪掉保存檔（沒有也算成功）、記住的螢幕改回主螢幕，成功後以 `notify` 發
/// [`POSITION_RESET_EVENT`]（QA 第 5 輪：不再借 `edge_stage_changed` 讓網頁從保存值變成 null 去猜）。刪檔失敗回錯誤、不通知。
pub fn reset_saved_position(path: &std::path::Path, notify: impl FnOnce(&'static str)) -> Result<(), String> {
    reset_saved_at(path)?;
    forget_saved_display();
    notify(POSITION_RESET_EVENT);
    Ok(())
}

/// 設定頁「回到預設位置」（AC-ST-09、CONTRACT §15 `edge_reset_position`）：刪掉保存的位置、記住的螢幕改回主螢幕、通知瀏海
/// 回到預設位置（[`POSITION_RESET_EVENT`]），舞台可見時立即貼回主螢幕（貼合的螢幕換了，`realign_stage` 另發
/// [`STAGE_EVENT`]）。
pub fn reset_position(app: &AppHandle) -> Result<(), String> {
    reset_saved_position(&saved_path(), |event| {
        let _ = app.emit_to(CAPSULE, event, ());
    })?;
    let app_main = app.clone();
    let _ = app.run_on_main_thread(move || realign_stage(&app_main));
    tracing::info!("瀏海位置已回到預設");
    Ok(())
}

/// 原生游標輪詢（瀏海可見時：間隔依距離 16–250 ms；隱藏時 2 Hz）。
pub fn spawn_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let next = poll_once(&app).await;
            tokio::time::sleep(Duration::from_millis(next)).await;
        }
    });
}

async fn poll_once(app: &AppHandle) -> u64 {
    let Some(w) = app.get_webview_window(CAPSULE) else { return IDLE_POLL_MS };
    if !w.is_visible().unwrap_or(false) {
        // 拖曳或按著時被隱藏（勿擾、⌘⇧K）：解除接管並通知網頁，只通知一次（INV 24）。
        release_on_hide(app);
        let mut s = state();
        s.buttons_down = true;
        s.ignoring = None;
        s.pointer = None;
        s.privacy = None;
        s.privacy_at_ms = None;
        return IDLE_POLL_MS;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let w2 = w.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = tx.send(cursor_sample(&w2));
    });
    let Ok((cursor, buttons_down)) = rx.await else { return IDLE_POLL_MS };
    let now = crate::windows::now_ms();
    let (d, pointer, privacy_due) = {
        let mut s = state();
        // 原生按下鎖定：在瀏海上按下後滑鼠鍵沒放開就持續接管，不等網頁回報開始拖曳（場景 14）。
        let inside = edge_hit::decide(cursor, &s.shapes, Grab::NONE, false).inside;
        s.held = edge_hit::press_latch(s.held, s.buttons_down, buttons_down, inside);
        s.buttons_down = buttons_down;
        let d = edge_hit::decide(cursor, &s.shapes, Grab { dragging: s.dragging, pressed: s.held }, buttons_down);
        if d.end_drag {
            s.dragging = false;
        }
        let pointer = edge_hit::pointer_signal(s.pointer, d.inside, now.saturating_sub(s.pointer_at_ms));
        if pointer.is_some() {
            s.pointer = Some(d.inside);
            s.pointer_at_ms = now;
        }
        let due = s.privacy_at_ms.is_none_or(|at| now.saturating_sub(at) >= PRIVACY_POLL_MS);
        if due {
            s.privacy_at_ms = Some(now);
        }
        (d, pointer, due)
    };
    if d.end_drag {
        let _ = app.emit_to(CAPSULE, DRAG_END_EVENT, ());
    }
    apply_ignore(app, d.ignore_mouse);
    if let Some(inside) = pointer {
        let _ = app.emit_to(CAPSULE, POINTER_EVENT, serde_json::json!({ "inside": inside }));
    }
    if privacy_due {
        push_privacy(app);
    }
    d.next_poll_ms
}

/// 隱私 guard 改變就推送給網頁（場景 12）：hover 中命中立即改回狀態文字。
fn push_privacy(app: &AppHandle) {
    let active = app.state::<crate::ipc::AppCtx>().handle.privacy_guard_active();
    let changed = {
        let mut s = state();
        let changed = s.privacy != Some(active);
        s.privacy = Some(active);
        changed
    };
    if changed {
        let _ = app.emit_to(CAPSULE, PRIVACY_EVENT, serde_json::json!({ "active": active }));
    }
}

/// 游標在舞台本地 CSS 座標（左上原點）的位置，與滑鼠鍵是否按著。
#[cfg(target_os = "macos")]
fn cursor_sample(w: &tauri::WebviewWindow) -> (Option<(f64, f64)>, bool) {
    use objc2_app_kit::{NSEvent, NSWindow};
    let buttons = NSEvent::pressedMouseButtons() != 0;
    let Ok(ptr) = w.ns_window() else { return (None, buttons) };
    // SAFETY：主執行緒、膠囊視窗的 NSWindow。
    let ns: &NSWindow = unsafe { &*ptr.cast::<NSWindow>() };
    let f = ns.frame();
    let m = NSEvent::mouseLocation();
    let local = (m.x - f.origin.x, f.origin.y + f.size.height - m.y);
    let inside_window = local.0 >= 0.0 && local.1 >= 0.0 && local.0 <= f.size.width && local.1 <= f.size.height;
    (inside_window.then_some(local), buttons)
}

#[cfg(not(target_os = "macos"))]
fn cursor_sample(_w: &tauri::WebviewWindow) -> (Option<(f64, f64)>, bool) {
    (None, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BUILT_IN: (Option<u32>, &str) = (Some(1), "Built-in Retina Display");

    /// AC-EN-03：保存值只接受已知角落、「已知邊＋0..1 頭端比例」或舊格式「已知邊＋0..1 中心比例」；壞值拒收（不碰檔案）、
    /// 讀到時當作沒有。頭端欄位存在但壞掉就整筆不用（不退回舊格式）。
    #[test]
    fn saved_position_is_validated() {
        for ok in [
            json!({"corner": "tl"}),
            json!({"corner": "br"}),
            json!({"edge": "left", "head": 0.0}),
            json!({"edge": "top", "head": 1.0}),
            json!({"edge": "bottom", "head": 0.42, "ratio": 0.9}),
            json!({"edge": "left", "ratio": 0.0}),
            json!({"edge": "top", "ratio": 1.0}),
            json!({"edge": "right", "ratio": 0.42}),
        ] {
            assert!(valid_position(&ok), "{ok}");
        }
        let screens = [BUILT_IN];
        for bad in [
            json!({"corner": "xx"}),
            json!({"edge": "middle", "ratio": 0.3}),
            json!({"edge": "left", "ratio": 2.0}),
            json!({"edge": "left", "ratio": -0.1}),
            json!({"edge": "left", "head": 1.5}),
            json!({"edge": "left", "head": "0.3", "ratio": 0.3}),
            json!({"edge": "left"}),
            json!({"edge": "left", "ratio": "0.3"}),
            json!({"ratio": 0.5}),
            json!({"head": 0.5}),
            json!("left"),
            json!(null),
            json!([]),
            json!({}),
        ] {
            assert!(!valid_position(&bad), "{bad}");
            assert!(save(Some(1), "Built-in Retina Display", bad.clone()).is_err(), "壞值不得寫檔：{bad}");
            let file = saved_body(Some(1), "Built-in Retina Display", bad.clone());
            assert_eq!(saved_for_screen(Some(file), &screens, 0), None, "讀到壞值當作沒有");
        }
        assert_eq!(saved_for_screen(None, &screens, 0), None);
    }

    /// AC-EN-03：記住的螢幕不在了就用主螢幕（第 0 個）；只記名稱的舊檔照名稱找。
    #[test]
    fn missing_display_falls_back_to_main() {
        let screens = [BUILT_IN, (Some(5), "LG UltraFine")];
        let legacy = |name: &str| SavedDisplay { id: None, name: Some(name.to_string()) };
        assert_eq!(pick_screen(&screens, &legacy("LG UltraFine")), 1);
        assert_eq!(pick_screen(&screens, &legacy("Dell U2720Q")), 0);
        assert_eq!(pick_screen(&screens, &SavedDisplay::default()), 0);
        assert_eq!(pick_screen(&screens, &SavedDisplay { id: Some(9), name: None }), 0);
    }

    /// AC-EN-03（場景 3d）：螢幕以 CGDirectDisplayID 識別、名稱備援；兩台同型號螢幕分得開；名稱對到不只一台時不猜。
    #[test]
    fn display_identity_prefers_id_then_unique_name() {
        let screens = [BUILT_IN, (Some(3), "DELL U2723QE"), (Some(4), "DELL U2723QE"), (Some(7), "LG UltraFine")];
        let saved = |id: Option<u32>, name: Option<&str>| SavedDisplay { id, name: name.map(str::to_string) };
        assert_eq!(match_screen(&screens, &saved(Some(4), Some("DELL U2723QE"))), Some(2), "同名的兩台以識別區分");
        assert_eq!(match_screen(&screens, &saved(Some(3), Some("DELL U2723QE"))), Some(1));
        assert_eq!(match_screen(&screens, &saved(Some(7), Some("改過名字"))), Some(3), "識別優先於名稱");
        assert_eq!(match_screen(&screens, &saved(Some(9), Some("LG UltraFine"))), Some(3), "識別變了：名稱恰好一台才採用");
        assert_eq!(match_screen(&screens, &saved(Some(9), Some("DELL U2723QE"))), None, "名稱對到兩台：不猜");
        assert_eq!(match_screen(&screens, &saved(None, Some("DELL U2723QE"))), None, "舊檔只有名稱且對到兩台：不猜");
        assert_eq!(match_screen(&screens, &saved(None, Some("Built-in Retina Display"))), Some(0));
        assert_eq!(match_screen(&screens, &saved(None, None)), None);
        assert_eq!(pick_screen(&screens, &saved(Some(9), Some("DELL U2723QE"))), 0, "對不到就主螢幕");
        // 保存檔欄位：新格式有 display_id，舊格式只有 display
        assert_eq!(
            SavedDisplay::from_file(&json!({"display_id": 4, "display": "DELL U2723QE", "position": {"corner": "tl"}})),
            saved(Some(4), Some("DELL U2723QE"))
        );
        assert_eq!(SavedDisplay::from_file(&json!({"display": "LG UltraFine"})), saved(None, Some("LG UltraFine")));
        assert_eq!(SavedDisplay::from_file(&json!({"display_id": -1, "display": 3})), saved(None, None));
    }

    /// AC-EN-03：保存的位置只套用在記住的那台螢幕；新格式寫入識別與名稱，舊格式照樣讀得到。
    #[test]
    fn saved_position_applies_only_to_the_matched_display() {
        let screens = [BUILT_IN, (Some(3), "DELL U2723QE"), (Some(4), "DELL U2723QE")];
        let pos = json!({"edge": "left", "head": 0.42});
        let body = saved_body(Some(4), "DELL U2723QE", pos.clone());
        assert_eq!(body, json!({"display_id": 4, "display": "DELL U2723QE", "position": {"edge": "left", "head": 0.42}}));
        assert_eq!(saved_for_screen(Some(body.clone()), &screens, 2), Some(pos.clone()));
        assert_eq!(saved_for_screen(Some(body.clone()), &screens, 1), None, "同名的另一台不套用");
        assert_eq!(saved_for_screen(Some(body), &screens, 0), None);
        let legacy = json!({"display": "Built-in Retina Display", "position": {"edge": "left", "ratio": 0.3}});
        assert_eq!(saved_for_screen(Some(legacy), &screens, 0), Some(json!({"edge": "left", "ratio": 0.3})));
    }

    /// AC-EN-03：預設位置避開的上方高度＝選單列實際佔用、實體瀏海、系統選單列標準高度三者最大；自動隱藏選單列（visibleFrame
    /// 推得 0）又沒有實體瀏海時仍留出標準選單列高度。
    #[test]
    fn menu_bar_inset_keeps_the_standard_height() {
        assert_eq!(MENU_BAR_STANDARD, 24.0);
        assert_eq!(top_inset(0.0, 0.0, 22.0), 24.0, "自動隱藏選單列、沒有瀏海");
        assert_eq!(top_inset(25.0, 0.0, 22.0), 25.0, "選單列可見");
        assert_eq!(top_inset(0.0, 38.0, 22.0), 38.0, "實體瀏海＋自動隱藏選單列");
        assert_eq!(top_inset(0.0, 0.0, 30.0), 30.0, "系統選單列較高");
        assert_eq!(top_inset(-5.0, f64::NAN, f64::NAN), 24.0, "讀到怪值仍至少標準高度");
    }

    /// AC-EN-15：舞台只在隱藏時才顯示；已顯示時對帳不做任何事（不再 makeKeyAndOrderFront 搶蓋板焦點）。
    #[test]
    fn stage_shows_only_when_hidden() {
        let a = Rect::new(0.0, 0.0, 1440.0, 900.0);
        assert_eq!(stage_plan(false, None, Some(a)), StagePlan::Show);
        assert_eq!(stage_plan(false, Some(a), Some(a)), StagePlan::Show);
        assert_eq!(stage_plan(true, Some(a), Some(a)), StagePlan::Keep);
        assert_eq!(stage_plan(true, None, None), StagePlan::Keep);
        assert_eq!(stage_plan(true, Some(a), None), StagePlan::Keep);
    }

    /// AC-EN-22：螢幕配置改變（解析度、插拔、睡眠喚醒）→ 已顯示的舞台重新貼合，不重新顯示。
    #[test]
    fn stage_refits_when_display_changes() {
        let old = Rect::new(0.0, 0.0, 1440.0, 900.0);
        assert_eq!(stage_plan(true, Some(old), Some(Rect::new(0.0, 0.0, 1512.0, 982.0))), StagePlan::Refit);
        assert_eq!(stage_plan(true, Some(old), Some(Rect::new(1440.0, 0.0, 1440.0, 900.0))), StagePlan::Refit);
        assert_eq!(stage_plan(true, Some(old), Some(Rect::new(0.2, 0.0, 1440.0, 900.3))), StagePlan::Keep, "0.5 pt 內視為一致");
    }

    /// AC-EN-03／22（QA 第 4 輪）：舞台貼合的螢幕識別和上次不同就通知網頁重讀 `edge_info`——尺寸相同也一樣（例如勿擾中
    /// 換接另一台同尺寸的螢幕）。識別與網頁 `stageIdentity` 一致：有 CGDirectDisplayID 用它，沒有才用名稱。
    #[test]
    fn stage_event_when_display_identity_changes() {
        let dell3 = StageDisplay::new(Some(3), "DELL U2723QE");
        let dell4 = StageDisplay::new(Some(4), "DELL U2723QE");
        assert!(stage_display_changed(None, &dell3), "第一次貼合：通知網頁");
        assert!(!stage_display_changed(Some(&dell3), &dell3), "同一台：不通知");
        assert!(stage_display_changed(Some(&dell3), &dell4), "同名同尺寸的另一台：通知");
        assert!(!stage_display_changed(Some(&dell3), &StageDisplay::new(Some(3), "改過名字")), "識別相同就是同一台");
        let lg = StageDisplay::new(None, "LG UltraFine");
        assert!(!stage_display_changed(Some(&lg), &StageDisplay::new(None, "LG UltraFine")), "取不到識別：同名視為同一台");
        assert!(stage_display_changed(Some(&lg), &StageDisplay::new(None, "Dell")));
        assert!(stage_display_changed(Some(&lg), &StageDisplay::new(Some(5), "LG UltraFine")), "拿到識別：網頁的舞台識別也變了");
        assert_eq!(StageDisplay::new(Some(3), "x"), StageDisplay::new(Some(3), "y"));
    }

    /// AC-EN-15：層級在選單列（24）與狀態列（25）之上、下拉選單（101）之下。
    #[test]
    fn stage_level_sits_between_menu_bar_and_popups() {
        const NS_STATUS_WINDOW_LEVEL: isize = 25;
        const NS_POP_UP_MENU_WINDOW_LEVEL: isize = 101;
        assert!((NS_STATUS_WINDOW_LEVEL + 1..NS_POP_UP_MENU_WINDOW_LEVEL).contains(&LEVEL_ABOVE_MENU_BAR));
    }

    /// AC-ST-09：「回到預設位置」刪掉保存的位置（沒有檔案也算成功），記住的螢幕改回主螢幕，刪檔成功後發
    /// `kunsu://edge_position_reset`（QA 第 5 輪：不再借 `edge_stage_changed`＋保存值 null 讓網頁猜）；之後讀到的保存值是 None
    /// （網頁用預設位置）。刪檔失敗回錯誤、不發事件。
    #[test]
    fn edge_reset_position_forgets_the_saved_position() {
        assert_eq!(POSITION_RESET_EVENT, "kunsu://edge_position_reset");
        assert_ne!(POSITION_RESET_EVENT, STAGE_EVENT);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge-notch.json");
        let screens = [(Some(7), "Studio Display"), BUILT_IN];
        std::fs::write(&path, saved_body(Some(1), "Built-in Retina Display", json!({"corner": "br"})).to_string()).unwrap();
        let before: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(pick_screen(&screens, &SavedDisplay::from_file(&before)), 1, "重設前記住的是第二台");

        let mut sent = Vec::new();
        reset_saved_position(&path, |event| sent.push(event)).expect("刪掉保存的位置");
        assert!(!path.exists());
        assert_eq!(sent, [POSITION_RESET_EVENT], "刪檔後通知瀏海回到預設位置（只發這一個）");
        assert_eq!(saved_display(), SavedDisplay::default(), "記住的螢幕清掉，不再讀舊檔");
        assert_eq!(pick_screen(&screens, &saved_display()), 0, "舞台回主螢幕");

        reset_saved_position(&path, |event| sent.push(event)).expect("沒有檔案也算成功");
        assert_eq!(sent, [POSITION_RESET_EVENT, POSITION_RESET_EVENT]);
        let after = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        assert_eq!(saved_for_screen(after, &screens, 0), None, "回到預設位置");

        // 刪不掉（路徑是資料夾）：回錯誤、不通知，瀏海不動
        let busy = dir.path().join("busy");
        std::fs::create_dir(&busy).unwrap();
        std::fs::write(busy.join("x"), "x").unwrap();
        let mut none: Vec<&str> = Vec::new();
        assert!(reset_saved_position(&busy, |event| none.push(event)).is_err());
        assert!(none.is_empty(), "刪檔失敗不發事件");
    }
}
