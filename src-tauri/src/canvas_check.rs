//! 證據畫布的註冊檢查（PRD §24、AC-EC-01～03）：隱藏視窗載入同一個決策面板（檢查模式），
//! 畫布在同一個沙盒 runtime 裡編譯、執行、量版面，回報問題後用 WKWebView 截圖給 agent 看。
//!
//! 流程：server `register_draft` → [`ShellCanvasRenderer::render`] 放一筆待檢查 → 檢查視窗
//! （`index.html?view=canvas-check`）以 `canvas_check_take` 取走、渲染、以 `canvas_check_done` 回報 →
//! 截圖寫到 `png_path`。一次只檢查一筆；每一筆都用新的檢查視窗，定案後終止網頁行程再銷毀（AC-EC-09b）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kunsu_server::draft::BoxFuture;
use kunsu_server::{CanvasRenderer, DraftIssue, RenderReport, RenderRequest};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::oneshot;

pub const CHECK_WINDOW: &str = "canvas-check";
/// 檢查視窗的大小＝常見螢幕；面板版型與使用者看到的一致。
const CHECK_W: f64 = 1440.0;
const CHECK_H: f64 = 900.0;
/// 整筆檢查（含視窗冷啟動）的上限。
const CHECK_TIMEOUT: Duration = Duration::from_secs(12);

/// 檢查頁回報。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CheckDone {
    #[serde(default)]
    pub issues: Vec<DraftIssue>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Default)]
pub struct CheckState {
    pending: Mutex<Option<(String, Value)>>,
    waiters: Mutex<HashMap<String, oneshot::Sender<CheckDone>>>,
    /// 一次只檢查一筆（同一個 label 的檢查視窗）。
    serial: tokio::sync::Mutex<()>,
}

impl CheckState {
    /// 檢查頁取走待檢查的一筆（題目＋證據）。
    pub fn take(&self) -> Option<Value> {
        self.pending.lock().expect("pending lock").as_ref().map(|(_, v)| v.clone())
    }

    pub fn done(&self, id: &str, done: CheckDone) {
        if let Some(tx) = self.waiters.lock().expect("waiters lock").remove(id) {
            let _ = tx.send(done);
        }
    }
}

/// 一筆註冊檢查要交給檢查頁的內容。
pub struct CheckJob {
    pub question: Value,
    pub evidence: Value,
    pub has_canvas: bool,
    pub png_path: std::path::PathBuf,
}

/// 檢查視窗的操作（殼層以 Tauri 視窗實作；測試用假的）。
pub trait CheckHost: Send + Sync {
    /// 檢查視窗還在嗎（前一次沒拆乾淨）。
    fn exists(&self) -> bool;
    /// 建立新的檢查視窗；新網頁掛載後以 `canvas_check_take` 取走待檢查的一筆。
    fn create(&self) -> Result<(), String>;
    /// 放進／移出視窗清單（WebKit 只畫看得到的視窗）。
    fn present_offscreen(&self, on: bool);
    fn snapshot(&self) -> BoxFuture<'_, Result<Vec<u8>, String>>;
    /// 先終止檢查視窗的網頁行程（卡住的畫布不會留在背景耗 CPU），再銷毀視窗並等它從視窗清單消失。
    fn teardown(&self) -> BoxFuture<'_, ()>;
}

/// 跑一筆檢查（AC-EC-01～03、09b）：每一筆都用新的檢查視窗與網頁行程，定案（截圖完成、失敗或逾時）後立刻拆掉。
/// 「畫完才卡住」的畫布因此不會留在背景，也不會讓下一筆用到同一個卡住的網頁。
pub async fn run_check(host: &dyn CheckHost, state: &CheckState, job: CheckJob, timeout: Duration) -> RenderReport {
    let _serial = state.serial.lock().await;
    if host.exists() {
        host.teardown().await;
    }
    let id = format!("chk-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default());
    let payload = json!({ "id": id, "question": job.question, "evidence": job.evidence });
    let (tx, rx) = oneshot::channel();
    state.waiters.lock().expect("waiters lock").insert(id.clone(), tx);
    *state.pending.lock().expect("pending lock") = Some((id.clone(), payload));
    let report = match host.create() {
        Err(e) => unavailable(&e),
        Ok(()) => {
            // WebKit 只畫「看得到」的視窗：檢查期間放進視窗清單（螢幕外、最後面、不收滑鼠、不搶焦點）。
            host.present_offscreen(true);
            let report = match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(done)) => {
                    *state.pending.lock().expect("pending lock") = None;
                    let preview_written = match host.snapshot().await {
                        Ok(png) => write_preview(&job.png_path, &png).is_ok(),
                        Err(e) => {
                            tracing::warn!("畫布預覽截圖失敗：{e}");
                            false
                        }
                    };
                    RenderReport { issues: done.issues, height: done.height, preview_written }
                }
                // 檢查視窗可能被畫布卡住主執行緒（AC-EC-09b）。
                _ => timeout_report(job.has_canvas),
            };
            host.present_offscreen(false);
            report
        }
    };
    *state.pending.lock().expect("pending lock") = None;
    state.waiters.lock().expect("waiters lock").remove(&id);
    host.teardown().await;
    report
}

pub struct ShellCanvasRenderer {
    app: AppHandle,
    state: Arc<CheckState>,
}

impl ShellCanvasRenderer {
    pub fn new(app: AppHandle, state: Arc<CheckState>) -> Self {
        ShellCanvasRenderer { app, state }
    }
}

impl CheckHost for ShellCanvasRenderer {
    fn exists(&self) -> bool {
        self.app.get_webview_window(CHECK_WINDOW).is_some()
    }

    fn create(&self) -> Result<(), String> {
        crate::windows::build_webview(
            WebviewWindowBuilder::new(&self.app, CHECK_WINDOW, WebviewUrl::App("index.html?view=canvas-check".into()))
                .title("Kunsu Canvas Check")
                .inner_size(CHECK_W, CHECK_H)
                .decorations(false)
                .skip_taskbar(true)
                .visible(false)
                .focused(false),
        )
        .map(|_| ())
        .map_err(|e| format!("建立檢查視窗失敗：{e}"))
    }

    fn present_offscreen(&self, on: bool) {
        present_offscreen(&self.app, on);
    }

    fn snapshot(&self) -> BoxFuture<'_, Result<Vec<u8>, String>> {
        Box::pin(snapshot_png(&self.app))
    }

    fn teardown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let Some(window) = self.app.get_webview_window(CHECK_WINDOW) else { return };
            // 先拿網頁行程的 pid 並終止它：只銷毀視窗時，卡在無窮迴圈的網頁行程會繼續耗 CPU（QA 第 4 輪實測）。
            let (tx, rx) = oneshot::channel();
            crate::webview_native::with_web_process_id(&window, move |pid| {
                let _ = tx.send(pid);
            });
            if let Ok(Ok(Some(pid))) = tokio::time::timeout(Duration::from_secs(1), rx).await {
                crate::webview_native::kill_pid(pid);
            }
            let _ = window.destroy();
            // 等它從視窗清單消失，下一筆才能用同一個 label 建新的。
            for _ in 0..100 {
                if self.app.get_webview_window(CHECK_WINDOW).is_none() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            tracing::warn!("檢查視窗 2 秒內沒有銷毀完成");
        })
    }
}

/// 檢查逾時：有畫布時多半是畫布卡住（無窮迴圈或過重的計算），回 `canvas_timeout` 指出是畫布的問題；沒有畫布才是
/// 檢查器本身沒回應（AC-EC-09b）。
fn timeout_report(has_canvas: bool) -> RenderReport {
    if !has_canvas {
        return unavailable("檢查視窗沒有在時限內回報");
    }
    RenderReport {
        issues: vec![DraftIssue {
            file: "canvas".into(),
            line: None,
            column: None,
            code: "canvas_timeout".into(),
            message: format!("畫布檢查 {} 秒內沒有完成", CHECK_TIMEOUT.as_secs()),
            hint: "多半是無窮迴圈或太重的計算（例如畫完後才開始跑的 setTimeout／setInterval）：拿掉迴圈、減少資料點後重新註冊".into(),
        }],
        height: None,
        preview_written: false,
    }
}

fn unavailable(reason: &str) -> RenderReport {
    RenderReport {
        issues: vec![DraftIssue {
            file: "canvas".into(),
            line: None,
            column: None,
            code: "renderer_unavailable".into(),
            message: format!("Kunsu 的畫布檢查沒有完成：{reason}"),
            hint: "稍後重新註冊；持續失敗時先拿掉畫布、只用 Markdown 註冊，並告訴使用者".into(),
        }],
        height: None,
        preview_written: false,
    }
}

impl CanvasRenderer for ShellCanvasRenderer {
    fn render(&self, req: RenderRequest) -> BoxFuture<'_, RenderReport> {
        Box::pin(async move {
            let job = CheckJob {
                question: serde_json::to_value(&req.question).unwrap_or(Value::Null),
                evidence: serde_json::to_value(&req.evidence).unwrap_or(Value::Null),
                has_canvas: req.evidence.canvas.is_some(),
                png_path: req.png_path,
            };
            run_check(self, &self.state, job, CHECK_TIMEOUT).await
        })
    }
}

/// 檢查視窗放進／移出視窗清單。放進時：螢幕外、最後面、不收滑鼠、不進 Mission Control 與視窗循環；
/// 關掉 WKWebView 的遮蔽偵測，讓它在螢幕外也照常繪製。使用者看不到、焦點不動。
#[cfg(target_os = "macos")]
fn present_offscreen(app: &AppHandle, on: bool) {
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, sel};
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let Some(window) = app.get_webview_window(CHECK_WINDOW) else { return };
    let Ok(ptr) = window.ns_window() else { return };
    let ptr = ptr as usize;
    let _ = app.run_on_main_thread(move || {
        // SAFETY：Tauri 的 ns_window() 是這個視窗的 NSWindow；只在主執行緒使用。
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        if on {
            ns.setIgnoresMouseEvents(true);
            ns.setCollectionBehavior(
                NSWindowCollectionBehavior::Transient | NSWindowCollectionBehavior::IgnoresCycle,
            );
            ns.setFrame_display(
                NSRect::new(NSPoint::new(-30_000.0, -30_000.0), NSSize::new(CHECK_W, CHECK_H)),
                false,
            );
            if let Some(view) = ns.contentView() {
                disable_occlusion(&view);
            }
            ns.orderBack(None);
        } else {
            ns.orderOut(None);
        }
    });

    /// 對 contentView 底下的 WKWebView 呼叫 `_setWindowOcclusionDetectionEnabled:NO`（有這個方法才呼叫）。
    fn disable_occlusion(view: &objc2_app_kit::NSView) {
        let obj: &AnyObject = view.as_ref();
        let responds: bool = unsafe { msg_send![obj, respondsToSelector: sel!(_setWindowOcclusionDetectionEnabled:)] };
        if responds {
            let _: () = unsafe { msg_send![obj, _setWindowOcclusionDetectionEnabled: false] };
        }
        for sub in view.subviews().iter() {
            disable_occlusion(&sub);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn present_offscreen(_app: &AppHandle, _on: bool) {}

/// 檢查視窗的截圖（PNG）。
#[cfg(target_os = "macos")]
async fn snapshot_png(app: &AppHandle) -> Result<Vec<u8>, String> {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError};
    use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};

    let window = app.get_webview_window(CHECK_WINDOW).ok_or("檢查視窗不存在")?;
    let (tx, rx) = oneshot::channel::<Result<Vec<u8>, String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    window
        .with_webview(move |webview| {
            let Some(mtm) = MainThreadMarker::new() else { return };
            // SAFETY：Tauri 在 macOS 上的 inner() 就是 WKWebView；只在主執行緒使用。
            let wk: &WKWebView = unsafe { &*(webview.inner() as *const WKWebView) };
            let config = unsafe { WKSnapshotConfiguration::new(mtm) };
            unsafe { config.setAfterScreenUpdates(true) };
            let tx = tx.clone();
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                let result = if image.is_null() {
                    let reason = if error.is_null() {
                        "WKWebView 沒有回傳影像".to_string()
                    } else {
                        // SAFETY：completion handler 給的 NSError 在呼叫期間有效。
                        unsafe { (*error).localizedDescription().to_string() }
                    };
                    Err(reason)
                } else {
                    // SAFETY：非空的 NSImage 在呼叫期間有效。
                    let image = unsafe { &*image };
                    image
                        .TIFFRepresentation()
                        .and_then(|tiff| NSBitmapImageRep::imageRepWithData(&tiff))
                        .and_then(|rep| unsafe {
                            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
                        })
                        .map(|png| png.to_vec())
                        .ok_or_else(|| "PNG 轉換失敗".to_string())
                };
                if let Some(tx) = tx.lock().expect("snapshot tx").take() {
                    let _ = tx.send(result);
                }
            });
            unsafe { wk.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block) };
        })
        .map_err(|e| format!("with_webview 失敗：{e}"))?;
    tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .map_err(|_| "截圖逾時".to_string())?
        .map_err(|_| "截圖通道關閉".to_string())?
}

#[cfg(not(target_os = "macos"))]
async fn snapshot_png(_app: &AppHandle) -> Result<Vec<u8>, String> {
    Err("只有 macOS 支援截圖".into())
}

/// 寫預覽截圖：不跟著 symlink 寫到草稿資料夾外面（先寫同資料夾的新暫存檔，再 rename 蓋過去）。
pub fn write_preview(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().ok_or_else(|| std::io::Error::other("預覽路徑沒有資料夾"))?;
    let tmp = dir.join(format!(".kunsu-preview-{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp); // 只移除暫存檔名本身（symlink 也只刪連結），不碰它指向的檔案
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path) // rename 取代的是路徑本身（含 symlink），不會寫進 symlink 指向的檔案
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_never_follows_a_planted_symlink() {
        let dir = tempfile::TempDir::new().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "keep me").unwrap();
        let drafts = dir.path().join("draft");
        std::fs::create_dir_all(&drafts).unwrap();
        let preview = drafts.join(".kunsu-preview.png");
        std::os::unix::fs::symlink(&victim, &preview).unwrap();
        write_preview(&preview, b"PNG").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me", "symlink 指到的檔案不能被覆寫");
        assert_eq!(std::fs::read(&preview).unwrap(), b"PNG");
        assert!(!std::fs::symlink_metadata(&preview).unwrap().file_type().is_symlink());
    }

    /// AC-EC-09b：檢查時畫布卡住 → 回 canvas_timeout（指出是畫布的問題）；沒有畫布才是檢查器的問題。
    #[test]
    fn check_timeout_blames_the_canvas_when_there_is_one() {
        let with_canvas = super::timeout_report(true);
        assert_eq!(with_canvas.issues[0].code, "canvas_timeout");
        assert!(with_canvas.issues[0].hint.contains("無窮迴圈"));
        assert_eq!(super::timeout_report(false).issues[0].code, "renderer_unavailable");
    }

    /// 假的檢查視窗：記下殼層對它做了什麼；`answers` 為 false 時模擬畫布卡住（網頁永遠不回報）。
    struct FakeHost {
        state: Arc<CheckState>,
        log: Mutex<Vec<String>>,
        alive: std::sync::atomic::AtomicBool,
        answers: std::sync::atomic::AtomicBool,
    }

    impl FakeHost {
        fn new(state: Arc<CheckState>) -> Self {
            FakeHost {
                state,
                log: Mutex::new(Vec::new()),
                alive: std::sync::atomic::AtomicBool::new(false),
                answers: std::sync::atomic::AtomicBool::new(true),
            }
        }
        fn push(&self, s: &str) {
            self.log.lock().unwrap().push(s.to_string());
        }
        fn take_log(&self) -> Vec<String> {
            std::mem::take(&mut *self.log.lock().unwrap())
        }
        fn alive(&self) -> bool {
            self.alive.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl CheckHost for FakeHost {
        fn exists(&self) -> bool {
            self.alive()
        }
        fn create(&self) -> Result<(), String> {
            if self.alive.swap(true, std::sync::atomic::Ordering::SeqCst) {
                return Err("檢查視窗已存在".into());
            }
            self.push("create");
            if self.answers.load(std::sync::atomic::Ordering::SeqCst) {
                // 新網頁掛載後自己取走這一筆並回報（canvas_check_take → canvas_check_done）。
                let state = self.state.clone();
                tokio::spawn(async move {
                    tokio::task::yield_now().await;
                    if let Some(job) = state.take() {
                        let id = job["id"].as_str().unwrap().to_string();
                        state.done(&id, CheckDone { issues: vec![], height: Some(120) });
                    }
                });
            }
            Ok(())
        }
        fn present_offscreen(&self, on: bool) {
            self.push(if on { "present" } else { "order_out" });
        }
        fn snapshot(&self) -> BoxFuture<'_, Result<Vec<u8>, String>> {
            self.push("snapshot");
            Box::pin(async { Ok(b"PNG".to_vec()) })
        }
        fn teardown(&self) -> BoxFuture<'_, ()> {
            if self.alive() {
                self.push("teardown");
            }
            self.alive.store(false, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async {})
        }
    }

    fn job(dir: &std::path::Path) -> CheckJob {
        CheckJob {
            question: json!({ "id": "q1" }),
            evidence: json!({ "canvas": { "kind": "tsx", "source": "export default () => null" } }),
            has_canvas: true,
            png_path: dir.join(".kunsu-preview.png"),
        }
    }

    /// QA 第 4 輪（AC-EC-09b）：每一筆檢查都用新的檢查視窗；截圖完成後立刻拆掉（終止網頁行程再銷毀），
    /// 「畫完才卡住」的畫布不會留在背景，也不會讓下一筆用到同一個網頁。
    #[tokio::test]
    async fn check_window_is_fresh_for_every_check_and_torn_down_after() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = Arc::new(CheckState::default());
        let host = FakeHost::new(state.clone());
        for round in 0..2 {
            let report = run_check(&host, &state, job(dir.path()), Duration::from_secs(2)).await;
            assert!(report.issues.is_empty(), "round {round}: {:?}", report.issues);
            assert!(report.preview_written);
            assert_eq!(report.height, Some(120));
            assert_eq!(host.take_log(), vec!["create", "present", "snapshot", "order_out", "teardown"], "round {round}");
            assert!(!host.exists(), "檢查完不留檢查視窗");
            assert!(state.take().is_none(), "不留待檢查的一筆");
        }
    }

    /// QA 第 4 輪（AC-EC-09b）：檢查逾時（畫布卡住）→ 回 canvas_timeout，並拆掉卡住的視窗與網頁行程；下一筆照常。
    #[tokio::test]
    async fn check_timeout_tears_down_the_hung_window() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = Arc::new(CheckState::default());
        let host = FakeHost::new(state.clone());
        host.answers.store(false, std::sync::atomic::Ordering::SeqCst);
        let report = run_check(&host, &state, job(dir.path()), Duration::from_millis(200)).await;
        assert_eq!(report.issues[0].code, "canvas_timeout");
        assert!(!report.preview_written);
        assert_eq!(host.take_log(), vec!["create", "present", "order_out", "teardown"]);
        assert!(!host.exists(), "卡住的檢查視窗要拆掉");
        assert!(state.take().is_none());

        host.answers.store(true, std::sync::atomic::Ordering::SeqCst);
        let report = run_check(&host, &state, job(dir.path()), Duration::from_secs(2)).await;
        assert!(report.issues.is_empty(), "{:?}", report.issues);
        assert_eq!(host.take_log()[0], "create", "下一筆用新的檢查視窗");
    }

    /// 前一次留下的檢查視窗（例如拆到一半 app 被打斷）：先拆掉再建新的。
    #[tokio::test]
    async fn leftover_check_window_is_torn_down_before_a_new_check() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = Arc::new(CheckState::default());
        let host = FakeHost::new(state.clone());
        host.alive.store(true, std::sync::atomic::Ordering::SeqCst);
        let report = run_check(&host, &state, job(dir.path()), Duration::from_secs(2)).await;
        assert!(report.issues.is_empty(), "{:?}", report.issues);
        assert_eq!(host.take_log()[..2], ["teardown".to_string(), "create".to_string()]);
    }
}
