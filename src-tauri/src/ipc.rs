//! Tauri IPC commands（CONTRACT §9 P2 固化契約；與 `ui/src/ipc.ts` 逐字對齊）。
//!
//! command 名：list_questions / get_question / answer_question / dismiss_question /
//! reauthorize_question / resolve_money_outcome / edit_option / open_deep_link / open_overlay_from_capsule / resize_capsule /
//! capsule_drag_geometry / move_capsule / snap_capsule / kill_switch。
//! invoke 參數鍵 snake_case（`rename_all = "snake_case"`）。
//! **answer_question 是唯一作答入口**（channel=ui、approval 題產簽章；
//! 錢鏈由 server reactor 內部觸發，UI 永不直接碰 executor）。

use std::sync::Arc;

use chrono::Utc;
use kunsu_core::types::{Channel, Question};
use kunsu_server::settings::{AppStatus, SettingsRejection, SettingsSnapshot, StatusEnv};
use kunsu_server::{AnswerInput, KeyProvider, MoneyState, ServerHandle};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

/// IPC 共享狀態（`app.manage`）。
pub struct AppCtx {
    pub handle: ServerHandle,
    pub key: Arc<dyn KeyProvider>,
    pub vault_root: std::path::PathBuf,
    /// 主視窗 `app_status` 的環境（本機服務位址、HOME、`~/.kunsu`）。
    pub status_env: StatusEnv,
    /// 手機橋接（PRD §30.8）。
    pub mobile: Option<kunsu_server::mobile::MobileBridge>,
}

/// 「打開完全取用磁碟設定」唯一會打開的網址（CONTRACT §15）。
pub const PRIVACY_ALL_FILES_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

/// 作答載荷（ui/src/ipc.ts `AnswerPayload` 的 serde 鏡像；tagged union）。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnswerPayload {
    Choice {
        question_id: String,
        choice_id: String,
        #[serde(default)]
        edited_params: Option<String>,
        /// 決策面板的補充說明（AC-DS-06）：與選項一起送出，存成 free_text；不改所見即所簽參數。
        #[serde(default)]
        note: Option<String>,
        /// 作答當下顯示的草稿版本與內容雜湊（AC-EC-05）。
        #[serde(default)]
        revision: Option<u32>,
        #[serde(default)]
        content_hash: Option<String>,
    },
    FreeText {
        question_id: String,
        free_text: String,
        #[serde(default)]
        revision: Option<u32>,
        #[serde(default)]
        content_hash: Option<String>,
    },
    /// 「這題不該問我／退回重擬」→ server 映射 cancelled + 出題品質計分。
    Reject {
        question_id: String,
        #[serde(default)]
        reason: Option<String>,
    },
    /// 題組逐項決定（CONTRACT §14）：每項帶面板顯示時的雜湊；`instruction` 是整體指示（全部退回時就是那段指示）。
    Batch {
        question_id: String,
        items: Vec<kunsu_core::types::ItemDecision>,
        #[serde(default)]
        instruction: Option<String>,
        #[serde(default)]
        revision: Option<u32>,
        #[serde(default)]
        content_hash: Option<String>,
    },
}

/// 題組作答 → server 作答輸入：通道固定 ui（使用者在面板逐項決定）；題組不動錢，沒有簽章。
pub(crate) fn batch_input(
    items: Vec<kunsu_core::types::ItemDecision>,
    instruction: Option<String>,
    revision: Option<u32>,
    content_hash: Option<String>,
) -> AnswerInput {
    AnswerInput {
        choice_id: None,
        free_text: instruction,
        edited_params: None,
        channel: Channel::Ui,
        signature: None,
        revision,
        content_hash,
        items: Some(items),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EditOptionResult {
    pub question: Value,
    pub redline: kunsu_core::redline::RedlineResult,
}

/// 膠囊 WebView 內的 Orb 幾何；window shell 會換算成 overlay 的座標系。
#[derive(Debug, Clone, Deserialize)]
pub struct CapsuleOrbAnchor {
    pub local_x: f64,
    pub local_y: f64,
    pub size: f64,
}

/// Question → UI 面 JSON：附加 `execution_blocked`（P2：Question 對 UI 加
/// `execution_blocked?: string`——所有失敗終態，不只 approval guard 擋下）與伺服器算的 `money_state`（CONTRACT §7、§9：
/// 已作答的錢題才有；網頁的回執一律依它給按鈕）。
fn ui_value(q: &Question, money_state: Option<MoneyState>) -> Value {
    let mut v = serde_json::to_value(q).unwrap_or_else(|_| json!({}));
    if let Some(obj) = v.as_object_mut() {
        if let Some(res) = q.answer.as_ref().and_then(|a| a.execution_result.as_deref())
            && let Some(reason) = execution_failure(res)
        {
            obj.insert("execution_blocked".to_string(), json!(reason));
        }
        if let Some(state) = money_state {
            obj.insert("money_state".to_string(), json!(state));
        }
    }
    v
}

async fn to_ui(handle: &ServerHandle, q: &Question) -> Value {
    // 讀不到就不帶（網頁退回看執行紀錄字串；伺服器端的動作照樣有自己的檢查）。
    let state = handle.money_state(q).await.unwrap_or_else(|e| {
        tracing::warn!("money_state 讀取失敗（{}）：{e}", q.id);
        None
    });
    ui_value(q, state)
}

async fn to_ui_all(handle: &ServerHandle, qs: &[Question]) -> Vec<Value> {
    let mut out = Vec::with_capacity(qs.len());
    for q in qs {
        out.push(to_ui(handle, q).await);
    }
    out
}

/// 錢的後續動作（重新授權、人工對帳）只收主視窗與蓋板（使用者的點擊）；瀏海舞台、畫布檢查與其他視窗一律拒絕（CONTRACT §9）。
pub(crate) fn money_window_allowed(label: &str) -> bool {
    label == crate::windows::MAIN || label == crate::windows::OVERLAY
}

pub(crate) fn execution_failure(result: &str) -> Option<&str> {
    result
        .strip_prefix("EXEC_BLOCKED:")
        .or_else(|| result.strip_prefix("EXEC_FAILED:"))
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::{evidence_view, execution_failure, AnswerPayload};
    use serde_json::json;

    /// 題組（CONTRACT §14）：面板送來的逐項決定原樣交給 server；整體指示存成 free_text；通道一定是 ui。
    #[test]
    fn batch_payload_becomes_an_item_by_item_ui_answer() {
        let payload: AnswerPayload = serde_json::from_value(json!({
            "type": "batch",
            "question_id": "mail-1009",
            "items": [
                {"item_id": "i1", "action": "approve", "text": "  原樣的文字\n", "content_hash": "sha256:1"},
                {"item_id": "i2", "action": "revise", "note": "先道歉", "content_hash": "sha256:2"},
                {"item_id": "i3", "action": "skip", "content_hash": "sha256:3"}
            ],
            "instruction": "語氣正式一點",
            "revision": 2,
            "content_hash": "sha256:v2"
        }))
        .expect("batch 載荷");
        let AnswerPayload::Batch { question_id, items, instruction, revision, content_hash } = payload else {
            panic!("要解析成 Batch");
        };
        assert_eq!(question_id, "mail-1009");
        let input = super::batch_input(items, instruction, revision, content_hash);
        assert_eq!(input.channel, kunsu_core::types::Channel::Ui);
        assert_eq!(input.free_text.as_deref(), Some("語氣正式一點"));
        assert_eq!(input.choice_id, None);
        assert_eq!(input.revision, Some(2));
        assert_eq!(input.content_hash.as_deref(), Some("sha256:v2"));
        let items = input.items.expect("逐項決定");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].text.as_deref(), Some("  原樣的文字\n"), "文字原樣，不修剪");
        assert_eq!(items[1].note.as_deref(), Some("先道歉"));

        let bad = serde_json::from_value::<AnswerPayload>(json!({
            "type": "batch", "question_id": "q", "items": [{"item_id": "i1", "action": "approve", "txt": "typo"}]
        }));
        assert!(bad.is_err(), "項目欄位打錯要大聲失敗");
    }

    /// AC-EC-09b：看門狗停用的版本不再交出畫布，附上原因；資產照常提供。
    #[test]
    fn disabled_canvas_is_withheld_with_its_reason() {
        let bundle = json!({ "canvas": { "kind": "tsx", "source": "while(1){}" }, "assets": { "./assets/a.png": "data:x" } });
        let off = evidence_view(bundle.clone(), Some("畫布沒有回應"));
        assert_eq!(off["canvas"], serde_json::Value::Null);
        assert_eq!(off["canvas_disabled"], "畫布沒有回應");
        assert_eq!(off["assets"]["./assets/a.png"], "data:x");
        assert_eq!(evidence_view(bundle.clone(), None), bundle);
    }

    /// AC-ST-11：「打開完全取用磁碟設定」只打開系統設定的那一頁，不接受任何參數（網頁不能拿它開別的網址）。
    #[test]
    fn open_privacy_settings_opens_only_full_disk_access() {
        assert_eq!(
            super::PRIVACY_ALL_FILES_URL,
            "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
        );
        let src = include_str!("ipc.rs");
        // 定義在這個測試模組之後；rfind 跳過測試裡的同名字面量。
        let start = src.rfind("pub fn open_privacy_settings(").expect("open_privacy_settings command");
        let body = &src[start..];
        let body = &body[..body.find("\n}\n").expect("函式結尾")];
        assert!(body.starts_with("pub fn open_privacy_settings() ->"), "不接受任何參數：{body}");
        assert!(body.contains("PRIVACY_ALL_FILES_URL"), "只打開固定網址");
    }

    /// CONTRACT §15：設定 IPC 的錯誤原樣交給網頁（`{errors: [{key, message}]}`，讓 UI 逐欄顯示），五個主視窗 command 都已註冊。
    #[test]
    fn settings_commands_reject_with_per_key_errors_and_are_registered() {
        let src = include_str!("ipc.rs");
        // 定義在這個測試模組之後；rfind 跳過測試裡的同名字面量。
        for name in ["settings_get", "settings_set", "settings_reset"] {
            let start = src.rfind(&format!("pub async fn {name}(")).unwrap_or_else(|| panic!("{name} command"));
            let sig = &src[start..start + src[start..].find('{').unwrap()];
            assert!(sig.contains("Result<SettingsSnapshot, SettingsRejection>"), "{name}：{sig}");
        }
        let set = &src[src.rfind("pub async fn settings_set(").unwrap()..];
        assert!(set[..set.find('{').unwrap()].contains("changes: serde_json::Map<String, Value>"));
        let reset = &src[src.rfind("pub async fn settings_reset(").unwrap()..];
        assert!(reset[..reset.find('{').unwrap()].contains("keys: Vec<String>"));
        let lib = include_str!("lib.rs");
        for name in ["settings_get", "settings_set", "settings_reset", "app_status", "edge_reset_position", "open_privacy_settings"] {
            assert!(lib.contains(&format!("ipc::{name},")), "lib.rs 要註冊 {name}");
        }
    }

    /// AC-MA-18／19（2026-10-09 錢 QA 第 11 輪 F11）：人工對帳與重新授權只收主視窗與蓋板（使用者的點擊），瀏海舞台、畫布檢查視窗
    /// 與其他視窗一律拒絕；兩個 command 都先過同一道視窗門，也都已註冊。
    #[test]
    fn money_actions_only_from_main_window_and_overlay() {
        use crate::windows::{CAPSULE, MAIN, OVERLAY};
        assert!(super::money_window_allowed(MAIN));
        assert!(super::money_window_allowed(OVERLAY));
        assert!(!super::money_window_allowed(CAPSULE));
        assert!(!super::money_window_allowed(crate::canvas_check::CHECK_WINDOW));
        assert!(!super::money_window_allowed(""));
        let src = include_str!("ipc.rs");
        for name in ["reauthorize_question", "resolve_money_outcome"] {
            // 定義在這個測試模組之後；rfind 跳過測試裡的同名字面量。
            let start = src.rfind(&format!("pub async fn {name}(")).unwrap_or_else(|| panic!("{name} command"));
            let body = &src[start..];
            let body = &body[..body.find("\n}\n").expect("函式結尾")];
            assert!(body.contains("window: tauri::WebviewWindow"), "{name} 要拿到呼叫的視窗：{body}");
            let gate = body.find("money_window_allowed(window.label())").unwrap_or_else(|| panic!("{name} 要先過視窗門：{body}"));
            let call = body.find(".handle").unwrap_or_else(|| panic!("{name} 呼叫服務層：{body}"));
            assert!(gate < call, "{name} 要在呼叫服務層之前擋：{body}");
        }
        let lib = include_str!("lib.rs");
        for name in ["reauthorize_question", "resolve_money_outcome"] {
            assert!(lib.contains(&format!("ipc::{name},")), "lib.rs 要註冊 {name}");
        }
    }

    /// AC-MA-20（F2）：送給網頁的題目附伺服器算的 `money_state`（沒有就不帶這個欄位）；所有回傳題目的 IPC 都走附狀態的 `to_ui`。
    #[test]
    fn ui_questions_carry_the_server_money_state() {
        let req: kunsu_core::types::CreateRequest = serde_json::from_value(json!({
            "id": "pa-ms", "title": "加碼", "question_line": "要不要加碼？", "kind": "choice", "tier": "t1",
            "options": [{"id": "a", "label": "加碼", "recommended": true}, {"id": "b", "label": "維持"}],
            "default_action": {"on_expire": "noop_log"}, "source": {"agent": "claude", "cwd": "/w"}
        }))
        .expect("建題請求");
        let q = kunsu_core::types::Question::from_create(req, chrono::Utc::now());
        let with = super::ui_value(&q, Some(kunsu_server::MoneyState::Unknown));
        assert_eq!(with["money_state"], "unknown");
        assert_eq!(super::ui_value(&q, Some(kunsu_server::MoneyState::Reauthorizable))["money_state"], "reauthorizable");
        assert!(super::ui_value(&q, None).get("money_state").is_none(), "非錢題或還沒作答不帶 money_state");
        let src = include_str!("ipc.rs");
        let body = &src[src.find("async fn to_ui(").expect("to_ui")..];
        assert!(body.starts_with("async fn to_ui(handle: &ServerHandle, q: &Question) -> Value"), "to_ui 要能讀伺服器狀態：{}", body.chars().take(80).collect::<String>());
        for name in [
            "list_questions",
            "get_question",
            "answer_question",
            "reauthorize_question",
            "resolve_money_outcome",
            "dismiss_question",
            "snooze_question",
            "edit_option",
        ] {
            let start = src.rfind(&format!("pub async fn {name}(")).unwrap_or_else(|| panic!("{name} command"));
            let body = &src[start..];
            let body = &body[..body.find("\n}\n").expect("函式結尾")];
            assert!(body.contains("to_ui(&state.handle") || body.contains("to_ui_all(&state.handle"), "{name} 要附 money_state：{body}");
        }
    }

    #[test]
    fn dead_letter_is_an_execution_failure() {
        assert_eq!(
            execution_failure("EXEC_FAILED: 死信（重試 3 次仍失敗）：Claude Code 尚未登入"),
            Some("死信（重試 3 次仍失敗）：Claude Code 尚未登入")
        );
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_questions(state: State<'_, AppCtx>) -> Result<Vec<Value>, String> {
    let qs = state.handle.list_questions(true).await.map_err(|e| e.to_string())?;
    Ok(to_ui_all(&state.handle, &qs).await)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_question(state: State<'_, AppCtx>, id: String) -> Result<Option<Value>, String> {
    let q = state.handle.get_question(&id).await.map_err(|e| e.to_string())?;
    match q {
        Some(q) => Ok(Some(to_ui(&state.handle, &q).await)),
        None => Ok(None),
    }
}

/// 某題某版本的證據畫布與資產（PRD §24）。只讀 Kunsu 自己的版本快照；畫布在 UI 的沙盒 iframe 執行。
#[tauri::command(rename_all = "snake_case")]
pub async fn get_evidence(
    state: State<'_, AppCtx>,
    question_id: String,
    revision: u32,
) -> Result<Option<Value>, String> {
    let bundle = state.handle.evidence_bundle(&question_id, revision).await.map_err(|e| e.to_string())?;
    let disabled = crate::windows::canvas_disabled(&question_id, revision).then_some(crate::canvas_watchdog::HANG_REASON);
    Ok(bundle.map(|b| evidence_view(serde_json::to_value(b).unwrap_or(Value::Null), disabled)))
}

/// 看門狗停用的版本（AC-EC-09b）：不交出畫布、附上原因；資產照常提供。
fn evidence_view(mut bundle: Value, disabled: Option<&str>) -> Value {
    if let (Some(reason), Some(obj)) = (disabled, bundle.as_object_mut()) {
        obj.insert("canvas".to_string(), Value::Null);
        obj.insert("canvas_disabled".to_string(), json!(reason));
    }
    bundle
}

/// 面板掛上／卸下畫布與心跳回應（AC-EC-09b）。
#[tauri::command(rename_all = "snake_case")]
pub fn canvas_active(window: tauri::WebviewWindow, question_id: String, revision: u32) -> bool {
    crate::windows::canvas_active(window.label(), &question_id, revision)
}

#[tauri::command(rename_all = "snake_case")]
pub fn canvas_inactive(window: tauri::WebviewWindow, question_id: String, revision: u32) {
    crate::windows::canvas_inactive(window.label(), &question_id, revision);
}

#[tauri::command(rename_all = "snake_case")]
pub fn surface_pong(window: tauri::WebviewWindow) {
    crate::windows::canvas_pong(window.label());
}

/// 邊緣瀏海舞台資訊（螢幕大小、選單列高度、螢幕名稱、保存的位置）。
#[tauri::command(rename_all = "snake_case")]
pub async fn edge_info(app: tauri::AppHandle) -> Result<crate::edge::EdgeInfo, String> {
    Ok(crate::edge::info(&app).await)
}

/// 當下重新偵測的隱私 guard（AC-EN-12）：網頁每次 hover 開始都讀一次，拿到前一律當作隱私中。
#[tauri::command(rename_all = "snake_case")]
pub async fn edge_privacy(state: State<'_, AppCtx>) -> Result<bool, String> {
    Ok(state.handle.privacy_guard_active())
}

/// 到期被延後時擋住的原因（BDD edge-notch 場景 10c）：瀏海在最早一題到期時讀，每 2 秒重讀。
#[tauri::command(rename_all = "snake_case")]
pub async fn defer_reasons(state: State<'_, AppCtx>) -> Result<kunsu_core::guards::DeferReasons, String> {
    state.handle.defer_reasons(Utc::now()).await.map_err(|e| e.to_string())
}

/// 瀏海回報命中多邊形與拖曳狀態（原生層據此切換點擊穿透）。
#[tauri::command(rename_all = "snake_case")]
pub fn edge_hit(app: tauri::AppHandle, window: tauri::WebviewWindow, shapes: Vec<crate::edge_hit::HitShape>, dragging: bool) {
    if window.label() == crate::windows::CAPSULE {
        crate::edge::set_hit(&app, shapes, dragging);
    }
}

/// 保存瀏海位置（螢幕識別＋名稱＋角落或邊與頭端比例）；格式不對回錯誤、不寫檔。
#[tauri::command(rename_all = "snake_case")]
pub fn edge_save(
    window: tauri::WebviewWindow,
    display: String,
    display_id: Option<u32>,
    position: Value,
) -> Result<(), String> {
    if window.label() != crate::windows::CAPSULE {
        return Ok(());
    }
    crate::edge::save(display_id, &display, position)
}

/// 面板顯示畫布失敗的診斷事件（BDD evidence-canvas 場景 9）。
#[tauri::command(rename_all = "snake_case")]
pub async fn canvas_failure(
    state: State<'_, AppCtx>,
    question_id: String,
    revision: u32,
    reason: String,
) -> Result<(), String> {
    state.handle.record_canvas_failure(&question_id, revision, &reason, Utc::now()).await.map_err(|e| e.to_string())
}

/// 畫布檢查頁取走待檢查的一筆（只有 canvas-check 視窗會呼叫）。
#[tauri::command(rename_all = "snake_case")]
pub fn canvas_check_take(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<crate::canvas_check::CheckState>>,
) -> Option<Value> {
    (window.label() == crate::canvas_check::CHECK_WINDOW).then(|| state.take()).flatten()
}

/// 畫布檢查頁回報結果（問題清單與內容高度）。
#[tauri::command(rename_all = "snake_case")]
pub fn canvas_check_done(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<crate::canvas_check::CheckState>>,
    id: String,
    report: crate::canvas_check::CheckDone,
) {
    if window.label() == crate::canvas_check::CHECK_WINDOW {
        state.done(&id, report);
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn answer_question(
    state: State<'_, AppCtx>,
    payload: AnswerPayload,
) -> Result<Value, String> {
    let now = Utc::now();
    let q = match payload {
        AnswerPayload::Reject { question_id, reason } => state
            .handle
            .reject_question(&question_id, reason, now)
            .await
            .map_err(|e| e.to_string())?,
        AnswerPayload::Choice { question_id, choice_id, edited_params, note, revision, content_hash } => {
            let signature =
                sign_answer(&state, &question_id, &choice_id, edited_params.as_deref()).await;
            let input = AnswerInput {
                choice_id: Some(choice_id),
                free_text: note.filter(|n| !n.trim().is_empty()),
                edited_params,
                channel: Channel::Ui,
                signature,
                revision,
                content_hash,
                items: None,
            };
            state
                .handle
                .answer_question(&question_id, input, now)
                .await
                .map_err(|e| e.to_string())?
        }
        AnswerPayload::Batch { question_id, items, instruction, revision, content_hash } => state
            .handle
            .answer_question(&question_id, batch_input(items, instruction, revision, content_hash), now)
            .await
            .map_err(|e| e.to_string())?,
        AnswerPayload::FreeText { question_id, free_text, revision, content_hash } => {
            let input = AnswerInput {
                choice_id: None,
                free_text: Some(free_text),
                edited_params: None,
                channel: Channel::Ui,
                signature: None,
                revision,
                content_hash,
                items: None,
            };
            state
                .handle
                .answer_question(&question_id, input, now)
                .await
                .map_err(|e| e.to_string())?
        }
    };
    Ok(to_ui(&state.handle, &q).await)
}

/// UI 簽章（approval 題才有，CONTRACT §2 Answer.signature）：
/// `kunsu_server::ui_answer_signature`——**與 approval record 用不同 HMAC 子鍵**
/// （domain separation）+ 每欄位長度編碼，拼接偽造做不出 approval record 的
/// 合法 MAC。這是稽核欄位；錢鏈的法定簽章在 reactor 的 approval record（§6）。
/// key 取不到 → None + log（作答不因稽核簽章失敗被擋；錢鏈屆時自會 fail closed）。
async fn sign_answer(
    state: &State<'_, AppCtx>,
    question_id: &str,
    choice_id: &str,
    edited_params: Option<&str>,
) -> Option<String> {
    let q = state.handle.get_question(question_id).await.ok().flatten()?;
    if !q.is_money_op() {
        return None;
    }
    let params = edited_params
        .map(str::to_string)
        .or_else(|| q.params_line.clone())
        .unwrap_or_default();
    match state
        .key
        .hmac_key()
        .and_then(|key| kunsu_server::ui_answer_signature(&key, question_id, choice_id, &params))
    {
        Ok(sig) => Some(sig),
        Err(e) => {
            tracing::warn!("UI 簽章失敗（signature 留空，錢鏈屆時 fail closed）：{e}");
            None
        }
    }
}

/// 重新授權（決策/授權分離；CONTRACT §6、§7）：被擋或對帳確認沒動的錢題，使用者可一鍵重跑 prepare→比對→簽章→commit——
/// 答案本身一字不動，只更新短效授權時間。只收主視窗與蓋板（使用者的點擊），瀏海舞台不收。
#[tauri::command(rename_all = "snake_case")]
pub async fn reauthorize_question(window: tauri::WebviewWindow, state: State<'_, AppCtx>, id: String) -> Result<Value, String> {
    if !money_window_allowed(window.label()) {
        return Err("重新授權只能在 Kunsu 主視窗或決策面板操作".to_string());
    }
    let q = state
        .handle
        .reauthorize_question(&id, Utc::now())
        .await
        .map_err(|e| e.to_string())?;
    Ok(to_ui(&state.handle, &q).await)
}

/// 使用者人工對帳（CONTRACT §7）：結果不明的錢題標「錢已經動了／錢沒有動」。只收主視窗與蓋板（使用者的點擊），瀏海舞台不收。
#[tauri::command(rename_all = "snake_case")]
pub async fn resolve_money_outcome(
    window: tauri::WebviewWindow,
    state: State<'_, AppCtx>,
    question_id: String,
    executed: bool,
) -> Result<Value, String> {
    if !money_window_allowed(window.label()) {
        return Err("人工對帳只能在 Kunsu 主視窗或決策面板操作".to_string());
    }
    let q = state.handle.resolve_money_outcome(&question_id, executed, Utc::now()).await.map_err(|e| e.to_string())?;
    Ok(to_ui(&state.handle, &q).await)
}

/// 蓋板視窗的「稍後再問」與原生 Esc 後備共用同一本帳（AC-OP-06）：同一個蓋板 session 每題只記一次 dismissal。主視窗照舊。
#[tauri::command(rename_all = "snake_case")]
pub async fn dismiss_question(window: tauri::WebviewWindow, state: State<'_, AppCtx>, id: String) -> Result<Value, String> {
    let q = if window.label() == crate::windows::OVERLAY {
        crate::windows::dismiss_once(&state.handle, &id, crate::windows::overlay_session_id()).await
    } else {
        state.handle.dismiss_question(&id, Utc::now()).await
    }
    .map_err(|e| e.to_string())?;
    Ok(to_ui(&state.handle, &q).await)
}

/// 舊版 UI 相容入口。
#[tauri::command(rename_all = "snake_case")]
pub async fn snooze_question(state: State<'_, AppCtx>, id: String) -> Result<Value, String> {
    let q = state.handle.snooze_question(&id, Utc::now()).await.map_err(|e| e.to_string())?;
    Ok(to_ui(&state.handle, &q).await)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn edit_option(
    state: State<'_, AppCtx>,
    id: String,
    option_id: String,
    new_to: f64,
) -> Result<EditOptionResult, String> {
    let (q, redline) = state
        .handle
        .edit_option(&id, &option_id, new_to, Utc::now())
        .await
        .map_err(|e| e.to_string())?;
    Ok(EditOptionResult { question: to_ui(&state.handle, &q).await, redline })
}

/// 膠囊視窗貼合內容（CONTRACT §9）：UI ResizeObserver 量到的內容 CSS px。
/// 收合態縮到膠囊本體、展開態才放大——確保透明視窗不留大面積攔截區。
#[tauri::command(rename_all = "snake_case")]
pub async fn resize_capsule(app: AppHandle, w: f64, h: f64) -> Result<(), String> {
    if !w.is_finite() || !h.is_finite() {
        return Err(format!("resize_capsule 參數非法：{w}×{h}"));
    }
    crate::windows::resize_capsule(&app, w, h);
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn capsule_drag_geometry(
    app: AppHandle,
) -> Result<crate::windows::CapsuleDragGeometry, String> {
    crate::windows::capsule_drag_geometry(&app)
        .ok_or_else(|| "讀不到膠囊視窗座標，無法開始拖曳".to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn move_capsule(app: AppHandle, x: i32, y: i32) -> Result<(), String> {
    if crate::windows::move_capsule(&app, x, y) {
        Ok(())
    } else {
        Err("膠囊視窗移動失敗".to_string())
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn snap_capsule(app: AppHandle, reduced_motion: bool) -> Result<String, String> {
    crate::windows::snap_capsule(&app, reduced_motion)
        .await
        .map(str::to_string)
        .ok_or_else(|| "找不到膠囊所在螢幕，無法吸附".to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn kill_switch(state: State<'_, AppCtx>) -> Result<(), String> {
    state.handle.kill_switch(4, Utc::now()).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn dnd_get(state: State<'_, AppCtx>) -> Result<kunsu_server::DndStatus, String> {
    state.handle.dnd_status(Utc::now()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dnd_set(
    state: State<'_, AppCtx>,
    duration_minutes: u32,
    actor: Option<String>,
) -> Result<kunsu_server::DndStatus, String> {
    state
        .handle
        .set_dnd(duration_minutes, actor.as_deref().unwrap_or("ui"), Utc::now())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dnd_clear(
    state: State<'_, AppCtx>,
    actor: Option<String>,
) -> Result<kunsu_server::DndStatus, String> {
    state
        .handle
        .clear_dnd(actor.as_deref().unwrap_or("ui"), Utc::now())
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 主視窗設定與狀態（PRD §27、CONTRACT §15、INV-26：只改 policy.local.toml，永不碰題目）
// ---------------------------------------------------------------------------

/// 設定只能從主視窗改（瀏海、蓋板網頁沒有設定頁）。
fn main_window_only(window: &tauri::WebviewWindow) -> Result<(), SettingsRejection> {
    if window.label() == crate::windows::MAIN {
        Ok(())
    } else {
        Err(SettingsRejection {
            errors: vec![kunsu_core::policy::SettingError::new(
                kunsu_server::settings::LOCAL_FILE_KEY,
                "只能從主視窗的設定頁修改設定",
            )],
        })
    }
}

/// 設定頁讀取：生效值、預設值、被覆寫的鍵、local 檔位置與錯誤。
#[tauri::command(rename_all = "snake_case")]
pub async fn settings_get(state: State<'_, AppCtx>) -> Result<SettingsSnapshot, SettingsRejection> {
    state.handle.settings_get()
}

/// 設定頁儲存：`changes` 的鍵是 CONTRACT §15 表上的鍵；不合法整筆拒絕，reject 值是 `{errors: [{key, message}]}`。
#[tauri::command(rename_all = "snake_case")]
pub async fn settings_set(
    window: tauri::WebviewWindow,
    state: State<'_, AppCtx>,
    changes: serde_json::Map<String, Value>,
) -> Result<SettingsSnapshot, SettingsRejection> {
    main_window_only(&window)?;
    state.handle.settings_set(&changes)
}

/// 回復預設：從 local 檔移除這些鍵（`[]`＝全部）。
#[tauri::command(rename_all = "snake_case")]
pub async fn settings_reset(
    window: tauri::WebviewWindow,
    state: State<'_, AppCtx>,
    keys: Vec<String>,
) -> Result<SettingsSnapshot, SettingsRejection> {
    main_window_only(&window)?;
    state.handle.settings_reset(&keys)
}

/// 「Agent 連線」「資料與帳本」「關於」的資料（不含任何 token 或設定內容）。
#[tauri::command(rename_all = "snake_case")]
pub async fn app_status(state: State<'_, AppCtx>) -> Result<AppStatus, String> {
    state.handle.app_status(&state.status_env, Utc::now()).await.map_err(|e| e.to_string())
}

/// 「瀏海回到預設位置」（AC-ST-09）。
#[tauri::command(rename_all = "snake_case")]
pub fn edge_reset_position(app: AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != crate::windows::MAIN {
        return Err("只能從主視窗的設定頁重設瀏海位置".to_string());
    }
    crate::edge::reset_position(&app)
}

/// 「打開完全取用磁碟設定」（AC-ST-11）：只打開系統設定的那一頁，不接受任何參數。
#[tauri::command(rename_all = "snake_case")]
pub fn open_privacy_settings() -> Result<(), String> {
    open::that_detached(PRIVACY_ALL_FILES_URL).map_err(|e| format!("打開系統設定失敗：{e}"))
}

/// open_deep_link（P2）：
/// - http(s) → 預設瀏覽器
/// - claude://code/continue?session=local_… / codex://threads/<id> → 打開原對話（回答延續原對話）
/// - kunsu://discuss/<id> → osascript 開 Terminal 跑 claude 帶脈絡（cwd=vault 根）
/// - kunsu://overlay | kunsu://overlay/close | kunsu://main | kunsu://capsule
///   → 視窗殼控制（UI 的 capsule/overlay view 用）
/// - 其他 → 視為 vault 相對路徑，交給 Finder/預設 app
#[tauri::command(rename_all = "snake_case")]
pub async fn open_deep_link(
    app: AppHandle,
    state: State<'_, AppCtx>,
    target: String,
) -> Result<(), String> {
    if target.starts_with("http://") || target.starts_with("https://") {
        return open::that_detached(&target).map_err(|e| format!("開瀏覽器失敗：{e}"));
    }
    // 回執「打開原對話」：只放行 Claude App／Codex App 的對話 deep link（PRD §20）。
    if target.starts_with("claude://") || target.starts_with("codex://") {
        if !kunsu_core::continuity::is_conversation_link(&target) {
            return Err(format!("不是合法的原對話連結：{target}"));
        }
        return open::that_detached(&target).map_err(|e| format!("開啟原對話失敗：{e}"));
    }
    if let Some(id) = target.strip_prefix("kunsu://discuss/") {
        return open_discussion(&state, id);
    }
    match target.as_str() {
        "kunsu://overlay" => {
            if !state.handle.dnd_status(Utc::now()).await.map_err(|e| e.to_string())?.active {
                crate::windows::show_overlay(&app);
            }
            Ok(())
        }
        "kunsu://overlay/close" => {
            crate::windows::hide_overlay(&app);
            // 收起後再跑一次強制題對帳（AC-OP-07(e)）：蓋板開著或收起當下才到期的強制題立刻再呈現。
            crate::reconcile_overlay(&app).await;
            Ok(())
        }
        "kunsu://main" => {
            crate::windows::show_main(&app);
            Ok(())
        }
        "kunsu://capsule" => {
            if state.handle.dnd_status(Utc::now()).await.map_err(|e| e.to_string())?.active {
                crate::windows::hide_capsule(&app);
                return Ok(());
            }
            let has_actionable = state
                .handle
                .list_questions(false)
                .await
                .map_err(|e| e.to_string())?
                .iter()
                .any(|question| !question.internal);
            if has_actionable {
                crate::windows::show_capsule(&app);
            } else {
                crate::windows::hide_capsule(&app);
            }
            Ok(())
        }
        _ => {
            // vault 相對路徑（decision_ref / prediction_ref 等）。
            let path = state.vault_root.join(&target);
            if path.exists() {
                open::that_detached(&path).map_err(|e| format!("開檔失敗：{e}"))
            } else {
                Err(format!("deep link 無法解析：{target}"))
            }
        }
    }
}

/// 蓋板網頁掛載完成監聽後呼叫：標記就緒並主動拉取待播轉場（AC-OP-04）。
#[tauri::command(rename_all = "snake_case")]
pub async fn overlay_ready() -> Result<Option<crate::overlay_presenter::Transition>, String> {
    Ok(crate::windows::overlay_web_ready())
}

/// 蓋板網頁回報該轉場的題目、輸入與關閉控制都已渲染（AC-OP-04）。
#[tauri::command(rename_all = "snake_case")]
pub async fn overlay_mounted(app: AppHandle, id: u64) -> Result<(), String> {
    crate::windows::overlay_mounted(&app, id);
    Ok(())
}

/// 網頁已接手 Esc：解除原生看門狗（AC-OP-06）。
#[tauri::command(rename_all = "snake_case")]
pub async fn overlay_escape_ack() -> Result<(), String> {
    crate::windows::overlay_escape_acked();
    Ok(())
}

/// 蓋板網頁回報目前顯示的題目（AC-OP-06、AC-EC-09b）：原生 dismissal 除了強制題也要算它。只收蓋板視窗、合法 id。
#[tauri::command(rename_all = "snake_case")]
pub fn overlay_current(window: tauri::WebviewWindow, question_id: Option<String>) -> Result<(), String> {
    if window.label() != crate::windows::OVERLAY {
        return Ok(());
    }
    if let Some(id) = question_id.as_deref()
        && !kunsu_core::validate::valid_id(id)
    {
        return Err(format!("overlay_current 的題目 id 非法：{id:?}"));
    }
    crate::windows::set_overlay_current(question_id);
    Ok(())
}

/// 瀏海回報來源圖示中心（舞台本地 CSS 點，AC-OP-02）：沒有點擊座標時的轉場起點。只收瀏海視窗、有限值。
#[tauri::command(rename_all = "snake_case")]
pub fn edge_anchor(window: tauri::WebviewWindow, x: f64, y: f64, size: f64) -> Result<(), String> {
    if window.label() != crate::windows::CAPSULE {
        return Ok(());
    }
    if !crate::overlay_geometry::valid_anchor(x, y, size) {
        return Err(format!("edge_anchor 參數非法：({x}, {y}) size {size}"));
    }
    crate::windows::set_notch_anchor(x, y, size);
    Ok(())
}

/// 由膠囊開啟 overlay：以 Orb 的精確 local geometry 啟動跨視窗粒子 handoff。
#[tauri::command(rename_all = "snake_case")]
pub async fn open_overlay_from_capsule(
    app: AppHandle,
    state: State<'_, AppCtx>,
    anchor: CapsuleOrbAnchor,
) -> Result<(), String> {
    if !crate::overlay_geometry::valid_anchor(anchor.local_x, anchor.local_y, anchor.size) {
        return Err("膠囊 Orb geometry 非法".into());
    }
    if state.handle.dnd_status(Utc::now()).await.map_err(|e| e.to_string())?.active {
        crate::windows::hide_capsule(&app);
        crate::windows::hide_overlay(&app);
        return Ok(());
    }
    crate::windows::show_overlay_from_capsule(
        &app,
        anchor.local_x,
        anchor.local_y,
        anchor.size,
    );
    Ok(())
}

/// 「轉討論」：osascript 開 Terminal，cwd=vault 根，跑 claude 帶脈絡 prompt。
/// id 白名單過濾後才嵌入 AppleScript（防注入）。
fn open_discussion(state: &State<'_, AppCtx>, id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("discuss deep link 的 id 非法：{id:?}"));
    }
    let vault = state.vault_root.to_string_lossy().to_string();
    let prompt = format!(
        "Kunsu 轉討論：用 kunsu MCP 的 ask_get 拉題目 {id} 的全量快照，\
         向使用者摘要脈絡後展開討論。結論出來後：若使用者已在 Kunsu 卡片作答就不用動；\
         若要改題，走 ask_supersede 建新題。"
    );
    // shell 層跳脫：single-quote prompt；AppleScript 層跳脫：backslash + double quote。
    let shell_cmd = format!("cd {vault} && claude '{}'", prompt.replace('\'', "'\\''"));
    let osa = format!(
        "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
        shell_cmd.replace('\\', "\\\\").replace('"', "\\\"")
    );
    std::process::Command::new("osascript")
        .arg("-e")
        .arg(&osa)
        .spawn()
        .map_err(|e| format!("osascript 啟動失敗：{e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 手機配對與中轉 AI（PRD §30.7、§30.8；BDD mobile-pairing AC-MP-08）：只收主視窗
// ---------------------------------------------------------------------------

fn mobile_bridge<'a>(state: &'a State<'_, AppCtx>) -> Result<&'a kunsu_server::mobile::MobileBridge, String> {
    state.mobile.as_ref().ok_or_else(|| "手機橋接沒有啟動（~/.kunsu 的手機金鑰讀寫失敗，看 Kunsu 的 log）".to_string())
}

fn mobile_main_only(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == crate::windows::MAIN { Ok(()) } else { Err("只能在主視窗操作手機配對".into()) }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mobile_status(state: State<'_, AppCtx>) -> Result<kunsu_server::mobile::MobileStatus, String> {
    Ok(mobile_bridge(&state)?.status())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mobile_start_pairing(window: tauri::WebviewWindow, state: State<'_, AppCtx>) -> Result<kunsu_server::mobile::PairingQr, String> {
    mobile_main_only(&window)?;
    Ok(mobile_bridge(&state)?.start_pairing(Utc::now().timestamp()))
}

/// 使用者在主視窗按允許或拒絕配對（INV-29）。
#[tauri::command(rename_all = "snake_case")]
pub async fn mobile_decide_pairing(
    window: tauri::WebviewWindow,
    state: State<'_, AppCtx>,
    id: String,
    allow: bool,
) -> Result<kunsu_server::mobile::MobileStatus, String> {
    mobile_main_only(&window)?;
    let bridge = mobile_bridge(&state)?;
    bridge.decide_pairing(&id, allow, Utc::now()).await?;
    Ok(bridge.status())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mobile_revoke(
    window: tauri::WebviewWindow,
    state: State<'_, AppCtx>,
    device_id: String,
) -> Result<kunsu_server::mobile::MobileStatus, String> {
    mobile_main_only(&window)?;
    let bridge = mobile_bridge(&state)?;
    bridge.revoke(&device_id)?;
    Ok(bridge.status())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mobile_set_relay_agent(
    window: tauri::WebviewWindow,
    state: State<'_, AppCtx>,
    agent: String,
) -> Result<kunsu_server::mobile::MobileStatus, String> {
    mobile_main_only(&window)?;
    let bridge = mobile_bridge(&state)?;
    bridge.set_relay_agent(&agent)?;
    Ok(bridge.status())
}


// ---------------------------------------------------------------------- 帳號（PRD §31.2；BDD multi-user AC-MU-03）

#[tauri::command(rename_all = "snake_case")]
pub async fn account_status(window: tauri::WebviewWindow, state: State<'_, AppCtx>) -> Result<Option<kunsu_server::mobile::AccountInfo>, String> {
    mobile_main_only(&window)?;
    Ok(mobile_bridge(&state)?.account())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_start_login(window: tauri::WebviewWindow, state: State<'_, AppCtx>, email: String) -> Result<String, String> {
    mobile_main_only(&window)?;
    mobile_bridge(&state)?.account_start_login(&email).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_poll(window: tauri::WebviewWindow, state: State<'_, AppCtx>, poll: String) -> Result<bool, String> {
    mobile_main_only(&window)?;
    mobile_bridge(&state)?.account_poll(&poll).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_logout(window: tauri::WebviewWindow, state: State<'_, AppCtx>) -> Result<(), String> {
    mobile_main_only(&window)?;
    mobile_bridge(&state)?.account_logout().await
}
