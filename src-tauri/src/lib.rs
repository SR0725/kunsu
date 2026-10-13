//! Kunsu Tauri 2 shell（CONTRACT §9）。
//!
//! setup 內嵌啟動 kunsu-server 全套（service+HTTP+timer+reactor+inbox；埠被占
//! ＝已有實例在跑 → 前景實例直接退出避免雙開）。shell 只負責：視窗殼三件套、
//! tray、通知、全域快捷鍵（Cmd+Shift+K = kill-switch 4h）、ServerEvent →
//! Tauri event 轉發（kunsu://execution、kunsu://questions_changed，payload 照
//! CONTRACT §9 P2 段）。

mod canvas_check;
mod canvas_watchdog;
mod webview_native;
mod dnd_cache;
mod edge;
mod edge_hit;
mod first_run;
#[cfg(test)]
mod first_run_tests;
mod frame_gate;
mod guards;
mod ipc;
mod main_window;
mod navigation;
mod overlay_geometry;
mod overlay_presenter;
mod server_boot;
mod tray;
mod windows;

use std::sync::Arc;

use kunsu_core::state_machine::NotifyKind;
use kunsu_core::types::{State as QState, Tier};
use kunsu_server::ServerEvent;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use guards::SharedGuardFlags;
use ipc::{execution_failure, AppCtx};

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // 第一次打開（AC-MU-06）：從 DMG 裝的 Mac 自己準備 CLI、policy、playbooks、外掛與開機啟動。
    if let Ok(exe) = std::env::current_exe() {
        let app = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(|p| p.to_path_buf());
        if let (Some(app), Some(home)) = (app, std::env::var_os("HOME")) {
            if let Err(e) = first_run::setup(&app.join("Contents/Resources"), std::path::Path::new(&home), Some(&app)) {
                tracing::warn!("第一次打開的準備失敗：{e}");
            }
        }
    }

    // 先起內嵌 server（需要 handle 做 managed state）。
    let guard_flags = Arc::new(SharedGuardFlags::default());
    let boot = match tauri::async_runtime::block_on(server_boot::boot(guard_flags.clone())) {
        Ok(b) => b,
        Err(server_boot::BootError::AlreadyRunning(bind)) => {
            // 埠被占 = 已有實例：不是錯誤，前景實例安靜退出（避免雙開）。
            eprintln!("Kunsu 已在跑（{bind} 被占用）——本前景實例退出。");
            return;
        }
        Err(server_boot::BootError::Fatal(e)) => {
            eprintln!("Kunsu server 啟動失敗：{e}");
            std::process::exit(1);
        }
    };

    let ctx = AppCtx {
        handle: boot.handle.clone(),
        key: boot.key.clone(),
        vault_root: boot.vault_root.clone(),
        status_env: kunsu_server::settings::StatusEnv {
            server_addr: boot.bind.clone(),
            home: std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string())),
            kunsu_dir: boot.kunsu_dir.clone(),
        },
        mobile: boot.mobile.clone(),
    };
    let mobile_for_events = boot.mobile.clone();

    // 證據畫布的註冊檢查（PRD §24）：隱藏視窗渲染＋截圖，setup 拿到 AppHandle 後注入 server。
    let canvas_check_state = Arc::new(canvas_check::CheckState::default());
    let renderer_handle = boot.handle.clone();
    let renderer_state = canvas_check_state.clone();

    let shortcut_flags = guard_flags.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(build_shortcut_plugin())
        .manage(ctx)
        .manage(canvas_check_state.clone())
        .invoke_handler(tauri::generate_handler![
            ipc::list_questions,
            ipc::get_question,
            ipc::get_evidence,
            ipc::canvas_check_take,
            ipc::canvas_failure,
            ipc::canvas_active,
            ipc::canvas_inactive,
            ipc::surface_pong,
            ipc::edge_info,
            ipc::edge_hit,
            ipc::edge_save,
            ipc::edge_privacy,
            ipc::defer_reasons,
            ipc::canvas_check_done,
            ipc::answer_question,
            ipc::dismiss_question,
            ipc::snooze_question,
            ipc::reauthorize_question,
            ipc::resolve_money_outcome,
            ipc::edit_option,
            ipc::open_deep_link,
            ipc::open_overlay_from_capsule,
            ipc::resize_capsule,
            ipc::capsule_drag_geometry,
            ipc::move_capsule,
            ipc::snap_capsule,
            ipc::kill_switch,
            ipc::dnd_get,
            ipc::dnd_set,
            ipc::dnd_clear,
            ipc::overlay_ready,
            ipc::overlay_mounted,
            ipc::overlay_escape_ack,
            ipc::overlay_current,
            ipc::edge_anchor,
            ipc::settings_get,
            ipc::settings_set,
            ipc::settings_reset,
            ipc::app_status,
            ipc::edge_reset_position,
            ipc::open_privacy_settings,
            ipc::mobile_status,
            ipc::mobile_start_pairing,
            ipc::mobile_decide_pairing,
            ipc::mobile_revoke,
            ipc::mobile_set_relay_agent,
            ipc::account_status,
            ipc::account_start_login,
            ipc::account_poll,
            ipc::account_logout,
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            windows::create_windows(app.handle())?;
            renderer_handle.set_canvas_renderer(Arc::new(canvas_check::ShellCanvasRenderer::new(
                app.handle().clone(),
                renderer_state.clone(),
            )));
            tray::install(app.handle())?;
            spawn_event_bridge(app.handle().clone());
            // 手機配對狀態一有變化就通知主視窗（有新的待允許配對時主視窗跳出允許／拒絕）。
            if let Some(mobile) = mobile_for_events.clone() {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Emitter;
                    let mut rx = mobile.subscribe_changes();
                    while rx.recv().await.is_ok() {
                        let _ = app_handle.emit_to(windows::MAIN, "mobile_changed", ());
                    }
                });
            }
            spawn_main_thread_poll(app.handle().clone(), shortcut_flags.clone());
            spawn_dnd_reconcile(app.handle().clone());
            spawn_overlay_ticker(app.handle().clone());
            edge::spawn_poller(app.handle().clone());
            let app_for_capsule = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                reconcile_capsule(&app_for_capsule).await;
                reconcile_overlay(&app_for_capsule).await;
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Kunsu Tauri app 啟動失敗")
        .run(|app, event| {
            if main_window::reopen_shows_main(main_window::classify(&event)) {
                windows::show_main(app);
            }
        });
}

/// 全域快捷鍵：Cmd+Shift+K = kill_switch(4h)（CONTRACT §9）。
fn build_shortcut_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_global_shortcut::{Builder, ShortcutState};
    Builder::new()
        .with_shortcuts(["super+shift+k"])
        .expect("cmd+shift+k 快捷鍵定義合法")
        .with_handler(|app, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let ctx = app.state::<AppCtx>();
                match ctx.handle.kill_switch(4, chrono::Utc::now()).await {
                    Ok(_until) => {
                        windows::hide_overlay(&app);
                        windows::hide_capsule(&app);
                    }
                    Err(e) => tracing::error!("快捷鍵 kill-switch 失敗：{e}"),
                }
            });
        })
        .build()
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("系統通知發送失敗：{e}");
    }
}

/// ServerEvent → Tauri event 轉發（P2 IPC 契約）：
/// - `kunsu://execution`：`{question_id, step, status, line?, receipt?}`
/// - `kunsu://questions_changed`：清單變更（膠囊/清單刷新）
///
/// 另負責：warned/T0 通知、forced 蓋板顯示、answered/snoozed 清空即隱藏。
fn spawn_event_bridge(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut rx = {
            let ctx = app.state::<AppCtx>();
            ctx.handle.subscribe()
        };
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("event bridge 落後 {n} 筆，繼續");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            handle_server_event(&app, ev).await;
        }
    });
}

async fn handle_server_event(app: &AppHandle, ev: ServerEvent) {
    match &ev {
        ServerEvent::Execution { question_id, step, status, line } => {
            let _ = app.emit(
                "kunsu://execution",
                json!({
                    "question_id": question_id,
                    "step": step,
                    "status": status,
                    "line": line,
                }),
            );
        }
        ServerEvent::LedgerAppended { question_id, ledger_type } => {
            // 執行 ledger 行落地 → 收據卡（P2：最後一步 ok 時附 receipt）。
            if ledger_type == "execution" {
                let ctx = app.state::<AppCtx>();
                if let Ok(Some(q)) = ctx.handle.get_question(question_id).await {
                    let summary = q
                        .answer
                        .as_ref()
                        .and_then(|a| a.execution_result.as_deref())
                        .map(|r| r.lines().next().unwrap_or(r).to_string())
                        .unwrap_or_else(|| "✅ 已執行".to_string());
                    if let Some(reason) = execution_failure(&summary) {
                        let _ = app.emit(
                            "kunsu://execution",
                            json!({
                                "question_id": question_id,
                                "step": "ledger",
                                "status": "fail",
                                "line": reason,
                            }),
                        );
                    } else {
                        let _ = app.emit(
                            "kunsu://execution",
                            json!({
                                "question_id": question_id,
                                "step": "ledger",
                                "status": "ok",
                                "receipt": {
                                    "summary": summary,
                                    "decision_ref": q.decision_ref,
                                    "prediction_ref": q.prediction_ref,
                                    "discord_logged": !q.internal,
                                },
                            }),
                        );
                    }
                }
            }
            emit_questions_changed(app);
        }
        ServerEvent::Notify { question_id, kind } => {
            if dnd_active(app).await {
                emit_questions_changed(app);
                return;
            }
            let ctx = app.state::<AppCtx>();
            let q = ctx.handle.get_question(question_id).await.ok().flatten();
            let line = q.as_ref().map(|q| q.question_line.clone()).unwrap_or_default();
            let internal = q.as_ref().map(|q| q.internal).unwrap_or(false);
            match kind {
                NotifyKind::WarnCountdown => {
                    if !internal {
                        notify(app, "⏰ 3 分鐘後強制詢問", &line);
                    }
                }
                NotifyKind::ForcedOverlay => {
                    if !internal {
                        notify(app, "🔴 需要你現在拍板", &line);
                        windows::show_overlay(app);
                    }
                }
                NotifyKind::NewQuestion => {
                    if !internal && q.as_ref().is_some_and(|q| q.tier == Tier::T0) {
                        notify(app, "🔴 新 T0 待決", &line);
                    }
                }
            }
            emit_questions_changed(app);
        }
        ServerEvent::StateChanged { from, to, .. } => {
            // forced 進出都重新對帳：還有 forced 題 → 蓋板；清空 → 隱藏。
            if *from == QState::Forced || *to == QState::Forced {
                reconcile_overlay(app).await;
            }
            emit_questions_changed(app);
        }
        ServerEvent::BatchWindow { window, count } => {
            if dnd_active(app).await {
                emit_questions_changed(app);
                return;
            }
            // T1 批次窗（CONTRACT §6 規則 8）：系統通知 +（guards 全綠時）開蓋板連答。
            notify(app, "🗂 T1 批次窗", &format!("{window} 批次窗：{count} 題等你連答"));
            let ctx = app.state::<AppCtx>();
            let all_green =
                ctx.handle.guards_all_green(chrono::Utc::now()).await.unwrap_or(false);
            if all_green {
                windows::show_overlay(app);
            }
            emit_questions_changed(app);
        }
        ServerEvent::KillSwitch { .. } => {
            windows::hide_overlay(app);
            windows::hide_capsule(app);
            emit_questions_changed(app);
        }
        ServerEvent::DndChanged { active, .. } => {
            if *active {
                windows::hide_overlay(app);
                windows::hide_capsule(app);
            } else {
                reconcile_overlay(app).await;
                reconcile_capsule(app).await;
            }
            emit_questions_changed(app);
        }
        ServerEvent::QuestionCreated { .. }
        | ServerEvent::QuestionTouched { .. }
        | ServerEvent::ActivationQuestionCreated { .. }
        | ServerEvent::ApprovalConsumed { .. }
        | ServerEvent::ReactorEnqueued { .. }
        | ServerEvent::ReactorDead { .. }
        | ServerEvent::DeliveryChanged { .. } => emit_questions_changed(app),
        ServerEvent::DiscordQueued { .. } | ServerEvent::KongkoDuplicate { .. } | ServerEvent::Followup { .. } => {}
    }
    // 所有 server 事件後做一次存量對帳。建立、作答、過期、取消都可能讓膠囊
    // 從 0→1 或 1→0；用 DB 存量而不是猜事件順序，重啟與 lag 也能自癒。
    reconcile_capsule(app).await;
}

fn emit_questions_changed(app: &AppHandle) {
    let _ = app.emit("kunsu://questions_changed", json!({}));
}

/// 強制題對帳的決定（AC-OP-07）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverlayPlan {
    /// 要求顯示（session 已開時 presenter 重送待播轉場或忽略）。
    Show,
    /// 隱藏（沒有 session）。
    Hide,
    /// session 開著（使用者正在看回執或其他題）：不動。
    Keep,
}

/// 還有非 internal 強制題且 guards 全綠 → 顯示；否則 session 開著就不動、沒開就隱藏。蓋板收起後（session 已結束）
/// 再跑一次，蓋板開著或收起當下才到期的強制題就會再呈現。
pub(crate) fn overlay_plan(any_forced: bool, guards_green: bool, session_open: bool) -> OverlayPlan {
    if any_forced && guards_green {
        OverlayPlan::Show
    } else if session_open {
        OverlayPlan::Keep
    } else {
        OverlayPlan::Hide
    }
}

/// forced 存量對帳：任一非 internal 題在 forced → 蓋板現身；否則隱藏（session 開著時不收）。
pub(crate) async fn reconcile_overlay(app: &AppHandle) {
    let ctx = app.state::<AppCtx>();
    let any_forced = ctx
        .handle
        .list_questions(false)
        .await
        .map(|qs| qs.iter().any(|q| q.state == QState::Forced && !q.internal))
        .unwrap_or(false);
    let guards_green = ctx.handle.guards_all_green(chrono::Utc::now()).await.unwrap_or(false);
    match overlay_plan(any_forced, guards_green, windows::overlay_session_open()) {
        OverlayPlan::Show => windows::show_overlay(app),
        OverlayPlan::Hide => windows::hide_overlay(app),
        OverlayPlan::Keep => {}
    }
}

/// 非 internal 待決存量對帳：有題才顯示膠囊，零題完全隱藏。
async fn reconcile_capsule(app: &AppHandle) {
    let ctx = app.state::<AppCtx>();
    if dnd_active(app).await {
        windows::hide_capsule(app);
        return;
    }
    let has_actionable = ctx
        .handle
        .list_questions(false)
        .await
        .map(|questions| questions.iter().any(|question| !question.internal))
        .unwrap_or(false);
    if has_actionable {
        windows::show_capsule(app);
    } else {
        windows::hide_capsule(app);
    }
}

/// main-thread 輪詢（250ms）：前景全螢幕 guard 旗標。
/// （膠囊 hover 穿透切換已移除——膠囊視窗經 resize_capsule 貼合內容，全程可點。）
fn spawn_main_thread_poll(app: AppHandle, flags: Arc<SharedGuardFlags>) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let app2 = app.clone();
            let flags2 = flags.clone();
            let _ = app.run_on_main_thread(move || {
                windows::poll_main_thread(&app2, &flags2);
            });
        }
    });
}

/// 勿擾 expiry 沒有依賴 renderer 或問題 timer event；每秒以持久化 wall clock 對帳三個 surface。
fn spawn_dnd_reconcile(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut was_active: Option<bool> = None;
        loop {
            interval.tick().await;
            let active = dnd_active(&app).await;
            if active {
                windows::hide_overlay(&app);
                windows::hide_capsule(&app);
            } else if was_active == Some(true) {
                reconcile_overlay(&app).await;
                reconcile_capsule(&app).await;
                emit_questions_changed(&app);
            }
            was_active = Some(active);
        }
    });
}

static DND_CACHE: std::sync::Mutex<dnd_cache::DndCache> = std::sync::Mutex::new(dnd_cache::DndCache::new());

/// 勿擾狀態讀取（AC-OP-10）：讀取失敗沿用最後已知值；從未讀到時 60 秒後視為未啟用並告警一次。
async fn dnd_active(app: &AppHandle) -> bool {
    let ctx = app.state::<AppCtx>();
    let read = ctx.handle.dnd_status(chrono::Utc::now()).await;
    if let Err(error) = &read {
        tracing::warn!("讀取勿擾狀態失敗：{error}");
    }
    let decision = DND_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .observe(read.map(|status| status.active).map_err(|_| ()), windows::now_ms() / 1000);
    if decision.alert {
        ctx.handle.discord().send(kunsu_server::DiscordMessage {
            question_id: None,
            reason: "dnd_status_unreadable".to_string(),
            text: "⚠️ Kunsu 連續 60 秒讀不到勿擾狀態，已暫時視為未啟用以免強制收件匣失效；請檢查資料庫。".to_string(),
        });
    }
    decision.active
}

#[cfg(test)]
mod tests {
    use super::{overlay_plan, OverlayPlan};

    /// AC-OP-07(e)（決策面板 QA 第 2 輪）：蓋板收起後 session 已結束，再跑一次強制題對帳——還有強制題且 guards 全綠就
    /// 再呈現；session 開著時（例如使用者正在看回執）沒有強制題也不收；其他情況隱藏。舊版收起只隱藏不重新檢查，
    /// 蓋板開著時才到期的題永遠不會跳出來。
    #[test]
    fn overlay_plan_after_close_represents_remaining_forced_questions() {
        // 收起之後（session 已結束）。
        assert_eq!(overlay_plan(true, true, false), OverlayPlan::Show);
        assert_eq!(overlay_plan(false, true, false), OverlayPlan::Hide);
        assert_eq!(overlay_plan(true, false, false), OverlayPlan::Hide, "guards 命中（分享螢幕、勿擾…）不呈現");
        // session 開著：有強制題照樣要求顯示（presenter 會重送待播轉場或忽略），沒有也不收。
        assert_eq!(overlay_plan(true, true, true), OverlayPlan::Show);
        assert_eq!(overlay_plan(false, true, true), OverlayPlan::Keep);
        assert_eq!(overlay_plan(false, false, true), OverlayPlan::Keep);
        assert_eq!(overlay_plan(true, false, true), OverlayPlan::Keep, "勿擾另外直接隱藏，對帳不收開著的 session");
    }
}

/// 蓋板握手逾時與 Esc 看門狗（AC-OP-05／06）。
fn spawn_overlay_ticker(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            windows::overlay_tick(&app);
        }
    });
}
