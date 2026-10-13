//! ServerHandle — 服務層核心。之後的 HTTP/MCP 層與 Tauri IPC 都呼叫這裡。
//!
//! 職責邊界：狀態機/驗證/upsert 全委派 kunsu-core 純函數；本層負責
//! 儲存（SQLite）、真相層（vault ledger）、效果解釋（Effect → I/O）、
//! 事件廣播、單一寫者序列化。`now` 一律由呼叫端傳入。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use chrono::Duration;
use kunsu_core::continuity::{AgentReport, Delivery, DeliveryState, ReportStatus};
use kunsu_core::guards::GuardState;
use kunsu_core::policy::Policy;
use kunsu_core::state_machine::{transition, DiscordReason, Effect, Event};
use kunsu_core::time::{taipei_date, Ts};
use kunsu_core::types::{
    AgentKind, Answer, Channel, CreateRequest, DefaultAction, EvidenceLine, Gate, Kind, Money,
    OnExpire, Opt, Question, Source, State, Tier, Wake, WakeMode,
};
use kunsu_core::upsert::apply_upsert;
use kunsu_core::validate::{validate_create, RejectReason};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

use crate::discord::{DiscordMessage, DiscordSender};
use crate::error::ServerError;
use crate::events::ServerEvent;
use crate::guards_probe::GuardProbe;
use crate::ledger::{Ledger, LedgerRow, LedgerType};
use crate::store;

/// 服務層組態。vault 根與 DB 路徑皆可注入（測試 tempdir、prod 用 CONTRACT §0 常數）。
pub struct KunsuConfig {
    pub db_path: std::path::PathBuf,
    pub vault_root: std::path::PathBuf,
    pub policy: Policy,
    pub discord: Arc<dyn DiscordSender>,
    pub guards: Arc<dyn GuardProbe>,
}

struct Inner {
    pool: SqlitePool,
    ledger: Ledger,
    policy: RwLock<Policy>,
    discord: Arc<dyn DiscordSender>,
    guards: Arc<dyn GuardProbe>,
    events_tx: broadcast::Sender<ServerEvent>,
    /// 單一寫者：所有 read-modify-write 序列化。
    write_gate: Mutex<()>,
    started_at: Instant,
    /// vault 帳本是否處於降級（寫不進去）狀態——權限未給時為 true，只警告一次。
    ledger_degraded: std::sync::atomic::AtomicBool,
    /// 上次在 tick 裡嘗試補寫帳本待補寫佇列的時間（每 30 秒最多一次）。
    last_backlog_attempt: std::sync::Mutex<Option<Ts>>,
    /// OPEN.json 上次重寫失敗、還沒補寫成功（同步存在 settings `open_json_dirty`，重啟後仍記得）。待重寫時拒絕重建。
    open_dirty: std::sync::atomic::AtomicBool,
    /// 「帳本讀寫卡住，請重啟」已經對哪一次卡住（卡住的開始時刻）發過；換了一次卡住就再發。
    ledger_stuck_alerted: std::sync::Mutex<Option<Ts>>,
    /// 原對話等待者（ask_wait／`kunsu wait`）登記：題目 id → 在線數與最後活動時間。
    waiters: std::sync::Mutex<HashMap<String, WaiterSlot>>,
    /// 手機「問本人」的追問佇列（只在記憶體，重啟就沒有；INV-31）。
    followups: std::sync::Mutex<Vec<FollowupSlot>>,
    /// 草稿與版本快照根目錄（PRD §24）；測試改指 tempdir。
    drafts: std::sync::RwLock<crate::draft::DraftPaths>,
    /// 沙盒渲染檢查器（app 殼層注入；未注入時有畫布的草稿回 renderer_unavailable）。
    renderer: std::sync::RwLock<Option<Arc<dyn crate::draft::CanvasRenderer>>>,
    /// SQLite 路徑（主視窗「資料與帳本」顯示用）。
    db_path: std::path::PathBuf,
    /// 設定檔路徑（policy.toml／policy.local.toml；shell 注入，沒注入時 settings_* 回錯誤、不碰任何檔案）。
    settings_paths: std::sync::RwLock<Option<crate::settings::SettingsPaths>>,
    /// 設定讀改寫序列化（兩個儲存不會互相覆蓋）。
    settings_gate: std::sync::Mutex<()>,
}

/// 原 AI 回覆追問的上限（字）。
pub const FOLLOWUP_REPLY_MAX_CHARS: usize = 4000;

/// 一則「問本人」的追問。
#[derive(Debug, Clone)]
struct FollowupSlot {
    question_id: String,
    id: String,
    text: String,
    asked_at: Ts,
    delivered: bool,
}

#[derive(Debug, Default, Clone, Copy)]
struct WaiterSlot {
    in_flight: u32,
    last_seen: Option<Ts>,
}

/// 等待者兩輪 long-poll 之間的租約（AC-CC-05）。
pub const WAITER_LEASE_SECONDS: i64 = 10;
/// held 交付的重檢間隔（AC-CC-07）。
pub const HELD_RECHECK_SECONDS: i64 = 60;

#[derive(Clone)]
pub struct ServerHandle {
    inner: Arc<Inner>,
}

const DND_SETTING_KEY: &str = "kunsu_dnd";
const DND_MAX_MINUTES: u32 = 24 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DndRecord {
    starts_at: Ts,
    ends_at: Ts,
    actor: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DndStatus {
    pub active: bool,
    pub starts_at: Option<Ts>,
    pub ends_at: Option<Ts>,
    pub remaining_seconds: i64,
    pub actor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Dedup {
    Created,
    TouchedExisting,
}

/// `ask_create` 回應（CONTRACT §4）。已終態題 re-create 時帶回既有答案
/// （harness 場景 11：agent 必須先讀答案再決定下一步）。
#[derive(Debug, Clone, Serialize)]
pub struct CreateOutcome {
    pub question_id: String,
    pub state: State,
    pub dedup: Dedup,
    pub tier: Tier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier_downgrade_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<Answer>,
}

/// 作答輸入（Tauri IPC / Discord 讀回 / CLI 的共用面；MCP 面永不暴露）。
#[derive(Debug, Clone, Deserialize)]
pub struct AnswerInput {
    pub choice_id: Option<String>,
    pub free_text: Option<String>,
    pub edited_params: Option<String>,
    pub channel: Channel,
    pub signature: Option<String>,
    /// UI 作答當下顯示的草稿版本（AC-EC-05）；與目前版本不同 → 拒收請使用者重看。
    #[serde(default)]
    pub revision: Option<u32>,
    /// 同一版本的內容雜湊（AC-EC-05）：面板作答必須和 revision 一起帶。
    #[serde(default)]
    pub content_hash: Option<String>,
    /// 題組逐項決定（CONTRACT §14）；題組的 `free_text` 是整體指示。只有面板送得出來。
    #[serde(default)]
    pub items: Option<Vec<kunsu_core::types::ItemDecision>>,
}

/// `ask_answers_since` 回應。
#[derive(Debug, Clone, Serialize)]
pub struct AnswerBatch {
    pub events: Vec<TerminalEvent>,
    pub next_cursor: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TerminalEvent {
    pub seq: i64,
    pub terminal_type: String,
    pub question: Question,
}

/// 批次窗/蓋板清單資料面（UI 的 board_pack IPC）。
#[derive(Debug, Clone, Serialize)]
pub struct BoardPack {
    pub generated_at: Ts,
    /// 強制候選（T0 非終態），紅燈/錢優先、資歷久優先。
    pub t0: Vec<Question>,
    /// 批次窗清單（T1+ 非終態），同排序。
    pub batch: Vec<Question>,
}

/// `/health` 資料面（P1 定案 4：含 oldest_pending_days）。
#[derive(Debug, Clone, Serialize)]
pub struct Health {
    pub version: String,
    pub uptime_s: u64,
    pub pending_count: u32,
    /// 已超過 force_at、仍未進終態的非 internal 問題數。
    pub overdue_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_answer_at: Option<Ts>,
    pub oldest_pending_days: i64,
    /// 帳本待補寫佇列的行數（app 行程寫不進 vault 的變更；大於 0 時不要刪資料庫）。
    pub ledger_backlog: u64,
    /// OPEN.json 上次重寫失敗、還沒補寫成功。
    pub open_json_dirty: bool,
    /// 在途的 vault 讀寫卡住多少秒（沒有卡住為 0）；超過 2 分鐘要重啟 Kunsu。
    pub ledger_stuck_s: i64,
}

/// `POST /events/kongko` payload（CONTRACT §5；欄位名沿用 Kongko camelCase）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KongkoPublication {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(rename = "publicationId")]
    pub publication_id: String,
    #[serde(rename = "batchId")]
    pub batch_id: String,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub ads: Vec<serde_json::Value>,
    #[serde(rename = "confirmedAt", default)]
    pub confirmed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KongkoOutcome {
    /// 同 publicationId 重複投遞被冪等吸收（P1 定案 5）。
    pub duplicate: bool,
    pub closed_question_id: Option<String>,
    pub activation_question_id: Option<String>,
}

/// `consume_approval` 結果（INV-9）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Consumed,
    AlreadyConsumed,
    QuestionMissing,
    /// 這次授權在 `money_outcomes` 已有結果（例如 Kunsu 已記成 `EXEC_BLOCKED`）：Kunsu 已判定結束，事後花不掉。
    AlreadySettled,
    /// 這題有比它更新的簽發（被重新授權取代）。
    Superseded,
}

/// `ask_update` 允許的補充欄位（保守集合：資訊面，不碰倒數/狀態/tier/錢）。
/// `deny_unknown_fields`（CONTRACT §6 規則 11）：未知欄位（含 `answer`）一律
/// 400 大聲失敗，不准無聲吞掉——ask_update 永不是變相作答面。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePatch {
    #[serde(default)]
    pub evidence: Option<Vec<EvidenceLine>>,
    #[serde(default)]
    pub body_md: Option<String>,
    #[serde(default)]
    pub deep_link: Option<String>,
    // 說人話重寫面（2026-08-01 作者拍板「卡片必須人話」後開放）：純文案欄位，
    // 不碰 state/tier/deadline/money——錢題的參數綁定（params_line/prepare_cmd）另有專用路徑。
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub question_line: Option<String>,
    /// 選項文案重寫。錢題拒絕（選項承載批准語義，錢題選項只能走 edit_option 的紅線重檢路徑）。
    #[serde(default)]
    pub options: Option<Vec<kunsu_core::types::Opt>>,
    /// 答後喚醒設定補掛（遷移舊卡 wake=none 造成「已讀不回」體感的修正通道）。
    #[serde(default)]
    pub wake_on_answer: Option<kunsu_core::types::Wake>,
}

/// 已作答錢題的錢的狀態（CONTRACT §7「錢的狀態 `money_state`」）：伺服器算好，shell 附在送給網頁的題目上，回執一律依它給按鈕。
/// 依序取第一個成立的：錢鏈在跑 → 結果不明（可人工對帳）→ 可重新授權 → 其餘已定案。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoneyState {
    /// 錢鏈適用、這個答案的 reactor 列還是 queued／executing（internal 題的 dry_run 同）。
    Running,
    /// 不在跑、有已被 executor 消耗卻沒有結果（或結果是 `EXEC_UNKNOWN`）的授權：使用者可人工對帳。
    Unknown,
    /// 重新授權的所有檢查都過（`reauthorize_refusal` 回 None）。
    Reauthorizable,
    /// 其餘：已執行完成、紅燈、安全選項、非 UI 作答、對帳確認已動……沒有後續動作。
    Settled,
}

/// 舊資料補結果時推定的錢鏈結果開頭（CONTRACT §7）：一旦是這兩者，重新授權永遠被拒、之後不會再有新的消耗。
const BACKFILL_PREFIXES: [&str; 2] = ["COMMIT_OK", "RECONCILED_EXECUTED"];
/// executor 驗證授權之前中斷（重啟恢復第 2 種，CONTRACT §7）的錢鏈結果與執行紀錄。
pub const INTERRUPTED_BEFORE_VERIFY: &str = "EXEC_BLOCKED: Kunsu 在 executor 驗證授權之前中斷，錢沒有動";

/// 執行紀錄（或某一次授權的結果）是否以「已執行完成」開頭。
fn executed_text(s: &str) -> bool {
    let s = s.trim_start();
    BACKFILL_PREFIXES.iter().any(|p| s.starts_with(p))
}

/// 這題的錢是否已執行完成：任一次授權的結果是 `COMMIT_OK`／`RECONCILED_EXECUTED`，或執行紀錄以它們開頭（CONTRACT §7）。
async fn money_executed(pool: &sqlx::SqlitePool, q: &Question) -> Result<bool, ServerError> {
    if q.answer.as_ref().and_then(|a| a.execution_result.as_deref()).is_some_and(executed_text) {
        return Ok(true);
    }
    store::executed_outcome_exists(pool, &q.id).await
}

/// 錢鏈列還沒跑完：錢鏈適用、這個答案的 reactor 列還是 queued／executing（internal 題 dry_run）——含錢鏈跑完後同一列的交付。
/// `money_state = running` 與人工對帳、重新授權的拒絕都以這個為準（列結束前不能動）。
async fn money_chain_running(pool: &sqlx::SqlitePool, q: &Question) -> Result<bool, ServerError> {
    let Some(a) = q.answer.as_ref() else { return Ok(false) };
    if !crate::reactor::money_chain_applies_to(q, a) {
        return Ok(false);
    }
    store::reactor_run_open(pool, &a.answer_id).await
}

/// 這一輪的錢鏈結果已寫進執行紀錄：執行紀錄以錢鏈結果開頭，而且 `executed_at` 不早於這一輪的授權時間（重新授權過的題以
/// `reauthorized_at` 為準）。重新授權不會清掉上一輪的紀錄，所以要看時間分辨是不是這一輪的。
fn chain_result_written(q: &Question) -> bool {
    let Some(a) = q.answer.as_ref() else { return false };
    let authorized_at = a.reauthorized_at.unwrap_or(a.answered_at);
    a.execution_result.as_deref().is_some_and(|r| crate::reactor::is_money_result(r.trim_start()))
        && a.executed_at.is_some_and(|t| t >= authorized_at)
}

/// 重新授權為什麼不行（None＝全部檢查都過，CONTRACT §7「重新授權」）。`money_state` 與 `reauthorize_question` 共用。
async fn reauthorize_refusal(pool: &sqlx::SqlitePool, q: &Question) -> Result<Option<String>, ServerError> {
    let id = &q.id;
    let Some(ans) = q.answer.as_ref() else {
        return Ok(Some(format!("題目 {id} 尚未作答，沒有可重新授權的決策")));
    };
    if !q.is_money_op() {
        return Ok(Some(format!("題目 {id} 非錢題，無 approval 可言")));
    }
    if !ans.channel.is_ray_device() {
        return Ok(Some("只有使用者在 Kunsu app 或已配對手機上點擊的答案能重新授權".to_string()));
    }
    if q.gate == Some(Gate::Red) {
        return Ok(Some("紅燈題永不進一鍵通道，不存在重新授權".to_string()));
    }
    let is_exec_choice = ans
        .choice_id
        .as_ref()
        .is_some_and(|cid| q.options.iter().any(|o| &o.id == cid && o.recommended == Some(true)));
    if !is_exec_choice {
        return Ok(Some("原答案不是執行型選項（recommended），沒有待執行的錢操作".to_string()));
    }
    if !crate::reactor::money_chain_applies_to(q, ans) {
        return Ok(Some("這題沒有 executor prepare 命令，錢鏈不會跑，不能重新授權".to_string()));
    }
    // 錢已經動了（任一次授權 COMMIT_OK／RECONCILED_EXECUTED，不只看執行紀錄開頭）就不能再授權一次（AC-CC-14、AC-MA-25）。
    if money_executed(pool, q).await? {
        return Ok(Some("這筆錢已經執行完成，不能重新授權（錢不會被再動一次）".to_string()));
    }
    if store::reactor_run_open(pool, &ans.answer_id).await? {
        return Ok(Some("錢鏈還在跑，不能重新授權（以 Kunsu 回執為準）".to_string()));
    }
    // 錢的結果不明：executor 可能已經 commit（例如 commit 中途當掉），錢可能已經動過——只能人工對帳（重建前後一樣）。
    if !store::unknown_money_nonces(pool, id).await?.is_empty() {
        return Ok(Some(
            "這筆的 approval 已經被 executor 用掉，卻沒有之後的執行結果——錢可能已經動過，不能重新授權，請先人工對帳".to_string(),
        ));
    }
    // 只有錢鏈明確沒有動錢的題才能重新授權：被擋（EXEC_BLOCKED，含 executor 驗證前中斷）或使用者對帳確認沒動（AC-MA-19）。
    // 乾跑、從沒跑過的舊答案不給（重新授權不是補跑舊決定的通道）。
    let unmoved = ans
        .execution_result
        .as_deref()
        .map(str::trim_start)
        .is_some_and(|r| r.starts_with("EXEC_BLOCKED") || r.starts_with("RECONCILED_NOT_EXECUTED"));
    if !unmoved {
        return Ok(Some("只有被擋下或對帳確認錢沒有動的錢題能重新授權".to_string()));
    }
    // 冪等 claim 的另一面：只有 done/dead（或重建後沒有列）能重新入列；needs_reconcile 必須先由人對帳。
    if let Some(status) = store::reactor_run_status(pool, &ans.answer_id).await?
        && status != "done"
        && status != "dead"
    {
        return Ok(Some("執行尚未終結（仍在執行或等待人工對帳），不能重新授權".to_string()));
    }
    Ok(None)
}

/// 錢的結果不明（CONTRACT §7）：以每一次授權（nonce）判斷——這題有已被 executor 消耗卻沒有錢鏈結果的 nonce，或結果是
/// `EXEC_UNKNOWN`。之後交付、追加、回報改寫執行紀錄或時間都不影響。
async fn money_outcome_unknown(pool: &sqlx::SqlitePool, q: &Question) -> Result<bool, ServerError> {
    if !q.is_money_op() {
        return Ok(false);
    }
    Ok(!store::unknown_money_nonces(pool, &q.id).await?.is_empty())
}

/// settings 鍵：OPEN.json 待重寫（值為第一次失敗時間）。
const OPEN_DIRTY_KEY: &str = "open_json_dirty";
/// 重建後自動重新排進 reactor 的答案年齡上限；更舊的列進報告請使用者確認（避免幾十天前的答案再喚醒一次）。
const REBUILD_REQUEUE_WINDOW_HOURS: i64 = 24;
/// 在途的 vault 讀寫卡住多久就請使用者重啟 Kunsu（核心層卡住的呼叫不會自己回來）。
const LEDGER_STUCK_ALERT_SECS: i64 = 120;

/// 重建報告裡列給使用者確認的一題。
#[derive(Debug, Clone, Serialize)]
pub struct RebuildItem {
    pub id: String,
    pub title: String,
    pub answered_at: Ts,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RebuildReport {
    pub pending_restored: u32,
    pub terminal_rows: u32,
    pub execution_rows: u32,
    pub approval_rows: u32,
    /// 重建後重新排進 reactor 的答案數。
    pub requeued: u32,
    /// approval 已消耗卻沒有執行紀錄、等人工對帳的錢題數。
    pub reconcile: u32,
    /// 超過 24 小時、帳本沒有處理紀錄的答案數：不自動重做，發 Discord 請使用者確認。
    pub stale_unexecuted: u32,
    /// 上面那些答案是哪幾題。
    pub stale: Vec<RebuildItem>,
    /// 等人工對帳的錢題是哪幾題。
    pub reconcile_items: Vec<RebuildItem>,
    pub bad_lines: Vec<String>,
}

impl ServerHandle {
    pub async fn new(cfg: KunsuConfig) -> Result<Self, ServerError> {
        let pool = crate::db::open_pool(&cfg.db_path).await?;
        let (events_tx, _) = broadcast::channel(256);
        let handle = ServerHandle {
            inner: Arc::new(Inner {
                pool,
                ledger: Ledger::new(cfg.vault_root),
                policy: RwLock::new(cfg.policy),
                discord: cfg.discord,
                guards: cfg.guards,
                events_tx,
                write_gate: Mutex::new(()),
                started_at: Instant::now(),
                ledger_degraded: std::sync::atomic::AtomicBool::new(false),
                last_backlog_attempt: std::sync::Mutex::new(None),
                open_dirty: std::sync::atomic::AtomicBool::new(false),
                ledger_stuck_alerted: std::sync::Mutex::new(None),
                waiters: std::sync::Mutex::new(HashMap::new()),
                followups: std::sync::Mutex::new(Vec::new()),
                drafts: std::sync::RwLock::new(crate::draft::DraftPaths::default_home()),
                renderer: std::sync::RwLock::new(None),
                db_path: cfg.db_path,
                settings_paths: std::sync::RwLock::new(None),
                settings_gate: std::sync::Mutex::new(()),
            }),
        };
        if store::get_setting(&handle.inner.pool, OPEN_DIRTY_KEY).await?.is_some() {
            handle.inner.open_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        // 舊 JSON 沒有 cycle 欄位時以 created_at 回填；已存在的週期永不重設。
        let policy = handle.policy();
        for mut question in store::list_open(&handle.inner.pool).await? {
            if question.ensure_force_cycle(&policy) {
                store::put_question(&handle.inner.pool, &question).await?;
            }
        }
        // 錢的舊資料（CONTRACT §7，冪等）：migration 0007 之前已執行完成的授權補結果；舊版把 executor 驗證前中斷標成 needs_reconcile
        // 的錢鏈列轉成被擋（錢沒有動、可重新授權）。
        {
            let _g = handle.inner.write_gate.lock().await;
            let now = chrono::Utc::now();
            let filled = handle.backfill_money_outcomes_locked(now).await?;
            let settled = handle.settle_legacy_interrupted_money_locked(now).await?;
            if filled + settled > 0 {
                tracing::info!("錢的舊資料補正：補結果 {filled} 筆、驗證前中斷轉被擋 {settled} 題");
            }
        }
        Ok(handle)
    }

    /// 指定草稿與版本快照根目錄（預設 `~/.kunsu/drafts`、`~/.kunsu/revisions`）。
    pub fn set_draft_paths(&self, paths: crate::draft::DraftPaths) {
        *self.inner.drafts.write().expect("drafts lock") = paths;
    }

    pub fn draft_paths(&self) -> crate::draft::DraftPaths {
        self.inner.drafts.read().expect("drafts lock").clone()
    }

    /// 注入沙盒渲染檢查器（app 殼層）。
    pub fn set_canvas_renderer(&self, renderer: Arc<dyn crate::draft::CanvasRenderer>) {
        *self.inner.renderer.write().expect("renderer lock") = Some(renderer);
    }

    fn canvas_renderer(&self) -> Option<Arc<dyn crate::draft::CanvasRenderer>> {
        self.inner.renderer.read().expect("renderer lock").clone()
    }

    /// 註冊草稿（`ask_register`／`kunsu ask <dir>`，AC-EC-01～04）：檢查 → 沙盒渲染 → 建題或升版。
    pub async fn register_draft(
        &self,
        dir: &std::path::Path,
        opts: crate::draft::RegisterOptions,
        now: Ts,
    ) -> Result<crate::draft::RegisterOutcome, ServerError> {
        use crate::draft::{DraftIssue, RegisterOutcome, RenderRequest};
        let paths = self.draft_paths();
        let dry_run = opts.dry_run;
        let fail = |qid: Option<String>, issues: Vec<DraftIssue>, preview: Option<String>| RegisterOutcome {
            dry_run,
            ..RegisterOutcome::failed(qid, issues, preview)
        };
        let loaded = match crate::draft::load(dir, &paths, opts.source.as_ref()) {
            Ok(d) => d,
            Err(issues) => return Ok(fail(None, issues, None)),
        };
        let qid = loaded.req.id.clone();
        let existing = store::get_question(&self.inner.pool, &qid).await?;
        if let Some(q) = existing.as_ref().filter(|q| q.state.is_terminal()) {
            return Ok(RegisterOutcome {
                state: Some(q.state),
                answer: q.answer.clone(),
                ..fail(
                    Some(qid.clone()),
                    vec![DraftIssue::new(
                        "question.md",
                        None,
                        None,
                        "question_closed",
                        format!("題目 {qid} 已結案（{:?}），不能再更新", q.state),
                        "已回答的題不會重問：看 answer 照使用者的決定做；真的要新決定就換一個新 id",
                    )],
                    None,
                )
            });
        }
        let current = existing.as_ref().and_then(|q| q.draft.clone());
        let preview = loaded.preview_path();
        let preview_str = || preview.to_string_lossy().to_string();
        if !opts.dry_run
            && let (Some(q), Some(d)) = (existing.as_ref(), current.as_ref())
            && d.content_hash == loaded.content_hash
        {
            // 內容沒變：不升版、不重渲染，只 touch。
            let _g = self.inner.write_gate.lock().await;
            if let Some(mut fresh) = store::get_question(&self.inner.pool, &qid).await? {
                fresh.last_seen = now;
                store::put_question(&self.inner.pool, &fresh).await?;
            }
            return Ok(RegisterOutcome {
                ok: true,
                question_id: Some(qid),
                revision: Some(d.revision),
                dedup: Some("unchanged".into()),
                state: Some(q.state),
                preview_png: preview.exists().then(preview_str),
                ..Default::default()
            });
        }
        let revision = current.as_ref().map(|d| d.revision + 1).unwrap_or(1);
        let draft_ref = loaded.draft_ref(revision, now);

        // 沙盒渲染檢查（使用者會看到的面板＋畫布）。
        let mut candidate = match existing.as_ref() {
            Some(q) => kunsu_core::upsert::apply_draft_update(q, &loaded.req, draft_ref.clone(), now),
            None => {
                let mut q = Question::from_create(loaded.req.clone(), now);
                q.draft = Some(draft_ref.clone());
                q
            }
        };
        candidate.answer = None;
        let mut preview_png = None;
        match self.canvas_renderer() {
            Some(renderer) => {
                let report = renderer
                    .render(RenderRequest { question: candidate, evidence: loaded.evidence(), png_path: preview.clone() })
                    .await;
                if report.preview_written {
                    preview_png = Some(preview_str());
                }
                let mut issues = report.issues;
                if let Some(h) = report.height.filter(|h| *h > crate::draft::MAX_CANVAS_HEIGHT)
                    && !issues.iter().any(|i| i.code == "canvas_too_tall")
                {
                    issues.push(DraftIssue::new(
                        loaded.canvas.as_ref().map(|c| c.kind.file_name()).unwrap_or("question.md"),
                        None,
                        None,
                        "canvas_too_tall",
                        format!("畫布高 {h} px，超過 {} px", crate::draft::MAX_CANVAS_HEIGHT),
                        "只留決策需要的圖；多張圖改成並排或分頁籤",
                    ));
                }
                if !issues.is_empty() {
                    return Ok(fail(Some(qid), issues, preview_png));
                }
            }
            None if loaded.canvas.is_some() => {
                return Ok(fail(
                    Some(qid),
                    vec![DraftIssue::new(
                        loaded.canvas.as_ref().map(|c| c.kind.file_name()).unwrap_or("canvas.tsx"),
                        None,
                        None,
                        "renderer_unavailable",
                        "Kunsu app 的畫布檢查器沒有啟動，無法確認畫布能正常顯示",
                        "確認 Kunsu.app 正在執行（kunsu doctor）；或先拿掉畫布，只用 Markdown 註冊",
                    )],
                    None,
                ));
            }
            None => {}
        }
        if opts.dry_run {
            return Ok(RegisterOutcome {
                ok: true,
                dry_run: true,
                question_id: Some(qid),
                revision: Some(revision),
                preview_png,
                ..Default::default()
            });
        }

        // 版本號在寫入鎖內決定、快照原子寫入：併發的重新註冊不會共用版本號，已存在的版本（包括使用者作答的那一版）
        // 永不被覆寫；鎖內才確認題目沒有結案（AC-EC-04／05）。
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let fresh = store::get_question(pool, &qid).await?;
        if let Some(f) = fresh.as_ref().filter(|q| q.state.is_terminal()) {
            return Ok(RegisterOutcome {
                state: Some(f.state),
                answer: f.answer.clone(),
                ..fail(
                    Some(qid.clone()),
                    vec![DraftIssue::new("question.md", None, None, "question_closed", format!("題目 {qid} 剛剛結案，不能再更新"), "看 answer 照使用者的決定做")],
                    preview_png,
                )
            });
        }
        if let Some((f, d)) = fresh
            .as_ref()
            .and_then(|q| q.draft.as_ref().map(|d| (q, d)))
            .filter(|(_, d)| d.content_hash == loaded.content_hash)
        {
            return Ok(RegisterOutcome {
                ok: true,
                question_id: Some(qid),
                revision: Some(d.revision),
                dedup: Some("unchanged".into()),
                state: Some(f.state),
                preview_png,
                ..Default::default()
            });
        }
        let next = fresh.as_ref().and_then(|q| q.draft.as_ref()).map(|d| d.revision + 1).unwrap_or(1);
        let draft_ref = crate::draft::commit_snapshot(&paths, &loaded, next, now)
            .map_err(|e| ServerError::BadDraft(format!("版本快照寫入失敗：{e}")))?;
        let revision = draft_ref.revision;
        let Some(fresh) = fresh else {
            let mut req = loaded.req.clone();
            Self::normalize_continuation(&mut req);
            let outcome = self.create_inner(req, Some(draft_ref), now).await?;
            self.rewrite_open(now).await?;
            return Ok(RegisterOutcome {
                ok: true,
                question_id: Some(outcome.question_id),
                revision: Some(revision),
                dedup: Some(match outcome.dedup {
                    Dedup::Created => "created".into(),
                    Dedup::TouchedExisting => "updated".into(),
                }),
                state: Some(outcome.state),
                preview_png,
                ..Default::default()
            });
        };
        // 升版：只換內容，不重設週期、不發通知（inv_upsert_no_reset）。
        let updated = kunsu_core::upsert::apply_draft_update(&fresh, &loaded.req, draft_ref, now);
        store::put_question(pool, &updated).await?;
        self.emit(ServerEvent::QuestionTouched { question_id: updated.id.clone() });
        self.rewrite_open(now).await?;
        Ok(RegisterOutcome {
            ok: true,
            question_id: Some(qid),
            revision: Some(revision),
            dedup: Some("updated".into()),
            state: Some(updated.state),
            preview_png,
            ..Default::default()
        })
    }

    /// 某題某版本的證據（畫布＋資產 data URI）；版本不存在回 None。
    pub async fn evidence_bundle(
        &self,
        question_id: &str,
        revision: u32,
    ) -> Result<Option<crate::draft::EvidenceBundle>, ServerError> {
        crate::draft::read_snapshot(&self.draft_paths(), question_id, revision)
            .map_err(|e| ServerError::BadDraft(format!("讀版本快照失敗：{e}")))
    }

    /// 等待者登記（ask_wait／`kunsu wait` 進入 long-poll）。AC-CC-04/05。
    pub fn waiter_enter(&self, question_id: &str, now: Ts) {
        let mut m = self.inner.waiters.lock().expect("waiters lock");
        let slot = m.entry(question_id.to_string()).or_default();
        slot.in_flight += 1;
        slot.last_seen = Some(now);
    }

    /// 等待者離開（一輪 long-poll 結束）；10 秒租約從此刻起算。
    pub fn waiter_exit(&self, question_id: &str, now: Ts) {
        let mut m = self.inner.waiters.lock().expect("waiters lock");
        let slot = m.entry(question_id.to_string()).or_default();
        slot.in_flight = slot.in_flight.saturating_sub(1);
        slot.last_seen = Some(now);
    }

    /// (等待者在線, 10 秒租約內)。
    pub fn waiter_status(&self, question_id: &str, now: Ts) -> (bool, bool) {
        let m = self.inner.waiters.lock().expect("waiters lock");
        match m.get(question_id) {
            None => (false, false),
            Some(slot) => {
                let lease = slot.last_seen.is_some_and(|t| {
                    let age = (now - t).num_milliseconds();
                    (0..WAITER_LEASE_SECONDS * 1000).contains(&age)
                });
                (slot.in_flight > 0, lease)
            }
        }
    }

    /// 手機「問本人」：放進追問佇列並叫醒等待者。不碰題目（INV-31）。
    pub fn followup_ask(&self, question_id: &str, followup_id: &str, text: &str, now: Ts) -> Result<(), ServerError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ServerError::BadFollowup("追問是空的".into()));
        }
        self.inner.followups.lock().expect("followups lock").push(FollowupSlot {
            question_id: question_id.to_string(),
            id: followup_id.to_string(),
            text: text.to_string(),
            asked_at: now,
            delivered: false,
        });
        self.emit(ServerEvent::Followup { question_id: question_id.into(), followup_id: followup_id.into(), reply: None });
        Ok(())
    }

    /// 等待者取走最早一則還沒交出的追問（每則只交一次）。
    pub fn followup_take(&self, question_id: &str) -> Option<(String, String)> {
        let mut v = self.inner.followups.lock().expect("followups lock");
        let slot = v.iter_mut().find(|f| f.question_id == question_id && !f.delivered)?;
        slot.delivered = true;
        Some((slot.id.clone(), slot.text.clone()))
    }

    /// 原 AI 回覆追問：只存成事件給手機，不改題目、倒數或答案（INV-31）。
    pub fn followup_reply(&self, question_id: &str, followup_id: &str, text: &str) -> Result<(), ServerError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ServerError::BadFollowup("回覆是空的".into()));
        }
        if text.chars().count() > FOLLOWUP_REPLY_MAX_CHARS {
            return Err(ServerError::BadFollowup(format!("回覆超過 {FOLLOWUP_REPLY_MAX_CHARS} 字")));
        }
        {
            let mut v = self.inner.followups.lock().expect("followups lock");
            let Some(i) = v.iter().position(|f| f.question_id == question_id && f.id == followup_id) else {
                return Err(ServerError::BadFollowup(format!("題目 {question_id} 沒有追問 {followup_id}（可能已逾時或回過了）")));
            };
            v.remove(i);
        }
        self.emit(ServerEvent::Followup {
            question_id: question_id.into(),
            followup_id: followup_id.into(),
            reply: Some(text.to_string()),
        });
        Ok(())
    }

    /// 移除問了超過 `max_age` 還沒回的追問，回 (題目 id, 追問 id)。
    pub fn followup_expire(&self, now: Ts, max_age: chrono::Duration) -> Vec<(String, String)> {
        let mut v = self.inner.followups.lock().expect("followups lock");
        let (old, keep): (Vec<_>, Vec<_>) = v.drain(..).partition(|f| now - f.asked_at >= max_age);
        *v = keep;
        old.into_iter().map(|f| (f.question_id, f.id)).collect()
    }

    /// 目前的交付狀態（沒有紀錄 = None）。
    pub async fn delivery_state(&self, answer_id: &str) -> Result<Option<DeliveryState>, ServerError> {
        Ok(store::get_delivery(&self.inner.pool, answer_id)
            .await?
            .and_then(|d| DeliveryState::parse(&d.state)))
    }

    /// 寫入交付狀態並同步題目 JSON（回執讀這份）。最終狀態只有第一個寫入者生效
    /// （`inv_single_continuation`）；回傳是否真的寫入。交給原對話即算 acked。
    pub async fn set_delivery(
        &self,
        question_id: &str,
        answer_id: &str,
        state: DeliveryState,
        reason: Option<String>,
        next_check_at: Option<Ts>,
        now: Ts,
    ) -> Result<bool, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let changed = store::put_delivery(
            pool,
            question_id,
            answer_id,
            state.as_str(),
            reason.as_deref(),
            next_check_at,
            now,
        )
        .await?;
        if !changed {
            return Ok(false);
        }
        self.sync_delivery_json(question_id, answer_id, state, reason, now).await?;
        Ok(true)
    }

    /// 結束一筆「交付中」（resuming）：只有仍是 resuming 的列能被改寫（追加成功、被鎖轉 held、轉開新對話）。
    pub async fn finish_delivery(
        &self,
        question_id: &str,
        answer_id: &str,
        state: DeliveryState,
        reason: Option<String>,
        next_check_at: Option<Ts>,
        now: Ts,
    ) -> Result<bool, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let changed =
            store::finish_delivery(pool, answer_id, state.as_str(), reason.as_deref(), next_check_at, now).await?;
        if changed {
            self.sync_delivery_json(question_id, answer_id, state, reason, now).await?;
        }
        Ok(changed)
    }

    async fn sync_delivery_json(
        &self,
        question_id: &str,
        answer_id: &str,
        state: DeliveryState,
        reason: Option<String>,
        now: Ts,
    ) -> Result<(), ServerError> {
        let pool = &self.inner.pool;
        if let Some(mut q) = store::get_question(pool, question_id).await? {
            let link = kunsu_core::continuity::conversation_link(&q.source);
            if let Some(a) = q.answer.as_mut().filter(|a| a.answer_id == answer_id) {
                // 同狀態重檢（held 每 60 秒）不追加帳本，避免一天上千行。
                let changed = a.delivery.as_ref().is_none_or(|d| d.state != state || d.reason != reason);
                a.delivery = Some(Delivery { state, reason, conversation_link: link, updated_at: now });
                if matches!(
                    state,
                    DeliveryState::LiveWaiter | DeliveryState::Resumed | DeliveryState::PickedUp | DeliveryState::Fresh
                ) && a.acked_at.is_none()
                {
                    a.acked_at = Some(now);
                }
                store::put_question(pool, &q).await?;
                // 交付狀態進帳本（INV-13）：rebuild-db 重放後交付與重建前一致，已交付的答案不會再交付一次。
                if changed {
                    let row = LedgerRow {
                        ts: now,
                        ledger_type: LedgerType::Delivery,
                        internal: q.internal,
                        row_id: None,
                        question: q.clone(),
                        answer: q.answer.clone(),
                        execution: Some(json!({ "delivery": { "state": state.as_str() } })),
                    };
                    self.append_ledger(&row, "delivery").await;
                }
            }
        }
        self.emit(ServerEvent::DeliveryChanged {
            question_id: question_id.to_string(),
            state: state.as_str().to_string(),
        });
        Ok(())
    }

    /// 只結束 reactor 列、不碰答案（暫時交付狀態不算執行，也不算接手）。
    pub async fn finish_reactor_run(&self, answer_id: &str, note: &str, now: Ts) -> Result<(), ServerError> {
        store::mark_reactor_done(&self.inner.pool, answer_id, note, now).await
    }

    /// `ask_report`：agent 回報處理結果與給使用者的回饋（AC-CC-12）。只寫 report／executed_at／acked_at，
    /// 永遠不碰答案本身、題目狀態（INV-7）與 Kunsu 自己的執行紀錄 `execution_result`（錢鏈結果與交付文字，
    /// AC-CC-14）。同內容重送冪等。
    pub async fn report_answer(
        &self,
        question_id: &str,
        answer_id: &str,
        status: ReportStatus,
        summary_md: &str,
        now: Ts,
    ) -> Result<Question, ServerError> {
        let summary = summary_md.trim();
        if summary.is_empty() {
            return Err(ServerError::BadReport("summary_md 不可為空：寫給使用者看的結果與回饋".into()));
        }
        if summary.chars().count() > 20_000 {
            return Err(ServerError::BadReport("summary_md 超過 20,000 字：請精簡，細節放 vault 檔案".into()));
        }
        let q = {
            let _g = self.inner.write_gate.lock().await;
            let pool = &self.inner.pool;
            let Some(mut q) = store::get_question(pool, question_id).await? else {
                return Err(ServerError::NotFound(question_id.to_string()));
            };
            let Some(a) = q.answer.as_mut() else {
                return Err(ServerError::BadReport(format!("題目 {question_id} 尚未作答，沒有可回報的答案")));
            };
            if a.answer_id != answer_id {
                return Err(ServerError::BadReport(format!(
                    "answer_id 不符：題目 {question_id} 目前的答案是 {}，請先 ask_get 取得正確 answer_id",
                    a.answer_id
                )));
            }
            let duplicate = a
                .report
                .as_ref()
                .is_some_and(|r| r.status == status && r.summary_md == summary);
            if !duplicate {
                a.report = Some(AgentReport { status, summary_md: summary.to_string(), reported_at: now });
                if a.executed_at.is_none() {
                    a.executed_at = Some(now);
                }
                if a.acked_at.is_none() {
                    a.acked_at = Some(now);
                }
                store::put_question(pool, &q).await?;
                let row = LedgerRow {
                    ts: now,
                    ledger_type: LedgerType::Execution,
                    internal: q.internal,
                    row_id: None,
                    question: q.clone(),
                    answer: q.answer.clone(),
                    execution: Some(json!({ "report": { "status": status, "summary_md": summary } })),
                };
                self.append_ledger(&row, "execution（ask_report）").await;
                self.emit(ServerEvent::LedgerAppended {
                    question_id: q.id.clone(),
                    ledger_type: LedgerType::Execution.as_str().to_string(),
                });
            }
            // 原對話自己讀走答案並回報（例如 Codex heartbeat）→ 交付記 picked_up、停止 reactor 的接續重試（AC-CC-09）。
            // 只對適用回答延續原對話的題（現行喚醒照舊）；和回報在同一段寫入鎖內，reactor 插不進來。重送時也補做。
            if kunsu_core::continuity::continuation_applies(&q.source, &q.wake_on_answer, q.internal)
                && store::put_delivery(pool, question_id, answer_id, DeliveryState::PickedUp.as_str(), None, None, now).await?
            {
                self.sync_delivery_json(question_id, answer_id, DeliveryState::PickedUp, None, now).await?;
            }
            q
        };
        Ok(self.get_question(question_id).await?.unwrap_or(q))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.inner.events_tx.subscribe()
    }

    pub fn policy(&self) -> Policy {
        self.inner.policy.read().expect("policy lock").clone()
    }

    /// policy 熱載入口（檔案 watcher 與設定頁儲存呼叫這裡）。
    pub fn set_policy(&self, policy: Policy) {
        *self.inner.policy.write().expect("policy lock") = policy;
    }

    /// 指定設定檔位置（CONTRACT §15；shell 依 `KUNSU_HOME_DIR` 注入，測試指 tempdir）。
    pub fn set_settings_paths(&self, paths: crate::settings::SettingsPaths) {
        *self.inner.settings_paths.write().expect("settings paths lock") = Some(paths);
    }

    pub(crate) fn settings_paths(&self) -> Option<crate::settings::SettingsPaths> {
        self.inner.settings_paths.read().expect("settings paths lock").clone()
    }

    pub(crate) fn settings_gate(&self) -> std::sync::MutexGuard<'_, ()> {
        self.inner.settings_gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// SQLite 路徑。
    pub fn db_path(&self) -> &std::path::Path {
        &self.inner.db_path
    }

    pub fn ledger(&self) -> &Ledger {
        &self.inner.ledger
    }

    /// crate 內協定層（reactor/HTTP）需要的 DB 面。外部呼叫端仍走服務方法。
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }

    /// Discord 發送器（reactor 死信 / inbox 壞檔告警共用同一實例）。
    pub fn discord(&self) -> Arc<dyn DiscordSender> {
        self.inner.discord.clone()
    }

    fn emit(&self, ev: ServerEvent) {
        let _ = self.inner.events_tx.send(ev); // 無訂閱者非錯誤
    }

    /// 協定層（reactor 執行流串流等）借道服務層廣播通道發事件。
    pub fn emit_event(&self, ev: ServerEvent) {
        self.emit(ev);
    }

    /// 組完整 GuardState：probe 情境旗標 + server 自身 kill-switch。
    async fn guard_state(&self, now: Ts) -> Result<GuardState, ServerError> {
        let snap = self.inner.guards.snapshot();
        let policy = self.policy();
        Ok(GuardState {
            screen_shared: snap.screen_shared,
            camera_on: snap.camera_on,
            mic_on: snap.mic_on,
            frontmost_fullscreen: snap.frontmost_fullscreen,
            quiet_hours: policy.in_quiet_hours(now),
            dnd: snap.dnd,
            kill_switch_until: self.dnd_status(now).await?.ends_at,
            forced_today_count: 0,
        })
    }

    /// OPEN.json 快照重寫。**vault 寫不進去不算致命**（2026-08-01 事故後改）：
    /// 真相仍在 SQLite，帳本是可重建的稽核副本。權限問題不該讓建題/作答整條路癱瘓——
    /// 改成記錄 + 標記降級（`ledger_degraded`），由背景重試與 doctor 讓它可見。
    /// 失敗時記下「待重寫」（資料庫 settings 也記一份），tick 每 30 秒重試；待重寫時拒絕重建（INV-13）。
    async fn rewrite_open(&self, now: Ts) -> Result<(), ServerError> {
        use std::sync::atomic::Ordering;
        let pool = &self.inner.pool;
        let open = store::list_open(pool).await?;
        match self.inner.ledger.rewrite_open(&open, now) {
            Ok(()) => {
                if self.inner.open_dirty.swap(false, Ordering::AcqRel) {
                    store::delete_setting(pool, OPEN_DIRTY_KEY).await?;
                    if store::ledger_outbox_count(pool).await? == 0
                        && self.inner.ledger_degraded.swap(false, Ordering::Relaxed)
                    {
                        self.inner.discord.send(crate::discord::DiscordMessage {
                            question_id: None,
                            reason: "ledger_recovered".to_string(),
                            text: "✅ Kunsu 帳本恢復寫入，待決題快照（OPEN.json）已重寫。".to_string(),
                        });
                    }
                }
            }
            Err(e) => {
                self.note_ledger_degraded("OPEN.json", &e);
                if !self.inner.open_dirty.swap(true, Ordering::AcqRel) {
                    store::put_setting(pool, OPEN_DIRTY_KEY, &now.to_rfc3339(), now).await?;
                }
            }
        }
        Ok(())
    }

    /// 帳本降級記錄：第一次大聲（log error + Discord），之後只 log，避免刷頻道。
    /// 寫帳本（INV-6 append-only）。先依序補寫待補寫佇列；寫不進去就把這一行排進佇列（不拖垮 server），權限恢復後
    /// 依序補寫。佇列沒清空前不能重建（INV-13）。
    async fn append_ledger(&self, row: &LedgerRow, what: &str) {
        // 同一行不論直接寫成、逾時後才寫成或之後補寫，都帶同一個 row_id，重建時只認一次。
        let mut row = row.clone();
        row.row_id.get_or_insert_with(|| uuid::Uuid::new_v4().to_string());
        let row = &row;
        let written = if self.flush_ledger_outbox().await {
            self.inner.ledger.append(row).map(|_| ())
        } else {
            Err(ServerError::LedgerBacklog(0))
        };
        if let Err(e) = written {
            self.note_ledger_degraded(what, &e);
            match serde_json::to_string(row) {
                Ok(json) => {
                    if let Err(e) = store::queue_ledger_row(&self.inner.pool, &json, row.ts).await {
                        tracing::error!("帳本待補寫佇列寫入失敗（{what}）：{e}");
                    }
                }
                Err(e) => tracing::error!("帳本行序列化失敗（{what}）：{e}"),
            }
        }
    }

    /// 依序補寫待補寫佇列；全部寫進帳本（或佇列本來就空）回 true。補寫完成後解除降級並告訴使用者。
    async fn flush_ledger_outbox(&self) -> bool {
        let pool = &self.inner.pool;
        let Ok(rows) = store::ledger_outbox(pool).await else { return false };
        if rows.is_empty() {
            return true;
        }
        let total = rows.len();
        for (seq, json) in rows {
            match serde_json::from_str::<LedgerRow>(&json) {
                Ok(row) => {
                    if self.inner.ledger.append(&row).is_err() {
                        return false;
                    }
                }
                Err(e) => tracing::error!("帳本待補寫佇列第 {seq} 行無法解析，略過：{e}"),
            }
            if store::delete_ledger_outbox(pool, seq).await.is_err() {
                return false;
            }
        }
        self.inner.ledger_degraded.store(false, std::sync::atomic::Ordering::Relaxed);
        self.inner.discord.send(crate::discord::DiscordMessage {
            question_id: None,
            reason: "ledger_recovered".to_string(),
            text: format!("✅ Kunsu 帳本恢復寫入，已依序補寫 {total} 行。"),
        });
        true
    }

    fn note_ledger_degraded(&self, what: &str, e: &ServerError) {
        tracing::error!("vault 帳本寫入失敗（{what}）：{e}");
        let first = !self.inner.ledger_degraded.swap(true, std::sync::atomic::Ordering::Relaxed);
        if first {
            self.inner.discord.send(crate::discord::DiscordMessage {
                question_id: None,
                reason: "ledger_degraded".to_string(),
                text: "⚠️ Kunsu 目前寫不進 vault 帳本（多半是 macOS 未授予完全磁碟存取權限）——\
系統照常運作、答案安全存在 app 資料庫，但稽核帳本會缺這段。\
修法：系統設定 → 隱私權與安全性 → 完全取得磁碟權限 → 加入 Kunsu.app。".to_string(),
            });
        }
    }

    // ------------------------------------------------------------------
    // 建題（ask_create 的服務面）
    // ------------------------------------------------------------------

    pub async fn create_question(
        &self,
        req: CreateRequest,
        now: Ts,
    ) -> Result<CreateOutcome, ServerError> {
        self.create_question_with_draft(req, None, now).await
    }

    /// 建題；草稿註冊時同時寫入版本參照（PRD §24）。
    async fn create_question_with_draft(
        &self,
        mut req: CreateRequest,
        draft: Option<kunsu_core::types::DraftRef>,
        now: Ts,
    ) -> Result<CreateOutcome, ServerError> {
        Self::normalize_continuation(&mut req);
        let _g = self.inner.write_gate.lock().await;
        let outcome = self.create_inner(req, draft, now).await?;
        self.rewrite_open(now).await?;
        Ok(outcome)
    }

    /// 有原對話身分 → 預設回答延續原對話（PRD §20）；doctor internal 題除外。
    /// 舊式 wake {mode: resume, session_id} 先把 session_id 搬進 source.conversation_id，避免改成
    /// continue 後失去原對話。
    fn normalize_continuation(req: &mut CreateRequest) {
        if !req.internal
            && let Some(conv) = kunsu_core::continuity::conversation_id(&req.source, &req.wake_on_answer)
        {
            if req.source.conversation_id.as_deref().is_none_or(|c| c.trim().is_empty()) {
                req.source.conversation_id = Some(conv);
            }
            req.wake_on_answer.mode = WakeMode::Continue;
        }
    }

    /// 不取 write gate 的內部建題（kongko 啟用題生成沿用）。
    async fn create_inner(
        &self,
        mut req: CreateRequest,
        draft: Option<kunsu_core::types::DraftRef>,
        now: Ts,
    ) -> Result<CreateOutcome, ServerError> {
        let pool = &self.inner.pool;
        validate_create(&req)?;
        // 題組原文沿用草稿的 Markdown 安全規則（CONTRACT §14）；草稿註冊已逐行檢查過，這裡只管 JSON 建題。
        if draft.is_none() && req.kind == Kind::Batch {
            for (index, item) in req.items.iter().enumerate() {
                if let Some(detail) = crate::draft::markdown_problem(&item.context_md) {
                    return Err(RejectReason::BatchItem {
                        index,
                        item_id: item.id.clone(),
                        problem: kunsu_core::validate::BatchItemProblem::Markdown { detail },
                    }
                    .into());
                }
            }
        }
        // 排程督導（AC-RC-01）：有 routine_id 就是排程呼叫，否則是互動；`kunsu-` 前綴保留給 Kunsu 自己的題
        // （doctor 冒煙、Kongko 啟用），一律互動。agent 有填就照填。所有建題路徑都經過這裡。
        let routine = req.source.routine_id.as_deref().map(str::trim).filter(|r| !r.is_empty());
        if routine.is_some_and(|r| r.starts_with("kunsu-")) {
            req.source.trigger = Some(kunsu_core::continuity::Trigger::Interactive);
        } else if req.source.trigger.is_none() {
            let scheduled = routine.is_some();
            req.source.trigger = Some(if scheduled {
                kunsu_core::continuity::Trigger::Scheduled
            } else {
                kunsu_core::continuity::Trigger::Interactive
            });
        }

        if let Some(mut existing) = store::get_question(pool, &req.id).await? {
            existing.ensure_force_cycle(&self.policy());
            if existing.state.is_terminal() {
                // 已答題永不原樣重現（harness 場景 11）：不重置、不通知，
                // 回讓 agent 看得到終態與答案。
                return Ok(CreateOutcome {
                    question_id: existing.id.clone(),
                    state: existing.state,
                    dedup: Dedup::TouchedExisting,
                    tier: existing.tier,
                    tier_downgrade_reason: existing.tier_downgrade_reason.clone(),
                    answer: existing.answer.clone(),
                });
            }
            // upsert 去重：只 touch last_seen/證據；不重置倒數、不發通知（INV-8）。
            // 草稿題的內容只能經由重新註冊改變（升版），JSON 重送只 touch。
            let merged = if existing.draft.is_some() {
                Question { last_seen: now, ..existing.clone() }
            } else {
                apply_upsert(&existing, &req, now)
            };
            store::put_question(pool, &merged).await?;
            self.emit(ServerEvent::QuestionTouched { question_id: merged.id.clone() });
            return Ok(CreateOutcome {
                question_id: merged.id.clone(),
                state: merged.state,
                dedup: Dedup::TouchedExisting,
                tier: merged.tier,
                tier_downgrade_reason: merged.tier_downgrade_reason.clone(),
                answer: None,
            });
        }

        let policy = self.policy();
        let skey = store::source_key(&req.source);

        // per-source 每日配額（upsert touch 不計入；internal 不計入——P1 定案 10）。
        if !req.internal {
            let used = store::quota_used_today(pool, &skey, now).await?;
            if used >= policy.quota.per_source_daily_create {
                return Err(ServerError::QuotaExceeded {
                    source_key: skey,
                    limit: policy.quota.per_source_daily_create,
                });
            }
        }

        // T0 分級准入仍決定風險級別，但不再決定是否進強制詢問週期。
        let requested = req.tier;
        let mut downgrade_reason: Option<String> = None;
        let mut effective = requested;
        if requested == Tier::T0 {
            let fails = t0_admission_failures(&req);
            if !fails.is_empty() {
                effective = Tier::T1;
                downgrade_reason =
                    Some(format!("T0 准入不合格，自動降 T1：{}", fails.join("；")));
            }
        }

        // 疲勞降載（CONTRACT §6 規則 10）：7 天滑動窗同 (product, kind) 略過率
        // 超過 policy.fatigue 閾值 → tier 再降一級；資料夠（n>=4）才算。
        // internal / 無 product / 已在地板 T3 不參與。
        if !req.internal
            && effective != Tier::T3
            && let Some(product) = req.product.clone()
        {
            let kind_str = enum_str(&req.kind);
            let window_start = (taipei_date(now)
                - Duration::days(policy.fatigue.skip_rate_window_d as i64))
            .to_string();
            let stats = store::fatigue_stats(pool, &product, &kind_str, &window_start).await?;
            if stats.engaged >= 4 {
                let rate = stats.skipped as f64 / stats.engaged as f64;
                if rate > policy.fatigue.skip_rate_downgrade {
                    let downgraded = next_tier_down(effective);
                    let msg = format!(
                        "fatigue_downgrade: {product}/{kind_str} 過去 {} 天略過率 {:.0}%（{}/{}）\
                         超過閾值 {:.0}%，tier {} 自動降一級為 {}",
                        policy.fatigue.skip_rate_window_d,
                        rate * 100.0,
                        stats.skipped,
                        stats.engaged,
                        policy.fatigue.skip_rate_downgrade * 100.0,
                        enum_str(&effective),
                        enum_str(&downgraded),
                    );
                    effective = downgraded;
                    downgrade_reason = Some(match downgrade_reason.take() {
                        Some(prev) => format!("{prev}；{msg}"),
                        None => msg.clone(),
                    });
                    // Discord 承認（週一次節流；CONTRACT §6 規則 10）。
                    let notified = store::count_events_since_date(
                        pool,
                        "fatigue_downgrade_notice",
                        &window_start,
                    )
                    .await?;
                    if notified == 0 {
                        self.inner.discord.send(DiscordMessage {
                            question_id: Some(req.id.clone()),
                            reason: "fatigue_downgrade".to_string(),
                            text: format!("😮‍💨 疲勞降載啟動：{msg}"),
                        });
                        store::insert_event(
                            pool,
                            None,
                            "fatigue_downgrade_notice",
                            &json!({"product": product, "kind": kind_str, "rate": rate}),
                            now,
                        )
                        .await?;
                    }
                }
            }
        }

        let mut q = Question::from_create(req, now);
        q.draft = draft;
        q.tier = effective;
        q.tier_downgrade_reason = downgrade_reason;
        q.start_force_cycle(now, &policy, false);
        // 到期參數在建題時固定在題目上（INV-26）：之後設定頁改到期時間或最長等待天數只影響新題。
        q.pin_expiry(&policy);
        if effective != requested {
            // 分級準確率計分資料（requested_tier/生效 tier 配對永久留存）。
            store::insert_event(
                pool,
                Some(&q.id),
                "tier_downgrade",
                &json!({
                    "question_id": q.id,
                    "source_key": skey,
                    "requested": q.requested_tier,
                    "effective": q.tier,
                    "reason": q.tier_downgrade_reason,
                }),
                now,
            )
            .await?;
        }

        // created → pending 立即推進（含 deadline 已過的即刻過期邊界）。
        let guards = self.guard_state(now).await?;
        let (s, fx) = transition(&q, Event::Tick, now, &policy, &guards);
        let acts = self.apply(&mut q, s, &fx, now, None).await?;
        debug_assert!(acts.is_empty(), "建題路徑不產生啟用題");
        self.emit(ServerEvent::QuestionCreated { question_id: q.id.clone(), tier: q.tier });

        Ok(CreateOutcome {
            question_id: q.id.clone(),
            state: q.state,
            dedup: Dedup::Created,
            tier: q.tier,
            tier_downgrade_reason: q.tier_downgrade_reason.clone(),
            answer: None,
        })
    }

    // ------------------------------------------------------------------
    // 查詢
    // ------------------------------------------------------------------

    pub async fn get_question(&self, id: &str) -> Result<Option<Question>, ServerError> {
        store::get_question(&self.inner.pool, id).await
    }

    pub async fn list_questions(
        &self,
        include_terminal: bool,
    ) -> Result<Vec<Question>, ServerError> {
        if include_terminal {
            store::list_all(&self.inner.pool).await
        } else {
            store::list_open(&self.inner.pool).await
        }
    }

    pub async fn board_pack(&self, now: Ts) -> Result<BoardPack, ServerError> {
        let mut open = store::list_open(&self.inner.pool).await?;
        // 紅燈優先、錢優先、資歷久優先。
        open.sort_by_key(|q| {
            (q.gate != Some(Gate::Red), !q.is_money_op(), q.first_proposed)
        });
        let (t0, batch): (Vec<Question>, Vec<Question>) =
            open.into_iter().partition(|q| q.tier == Tier::T0);
        Ok(BoardPack { generated_at: now, t0, batch })
    }

    pub async fn health(&self, now: Ts) -> Result<Health, ServerError> {
        let open = store::list_open(&self.inner.pool).await?;
        let oldest_pending_days = open
            .iter()
            .filter(|q| !q.internal)
            .map(|q| taipei_date(now).signed_duration_since(q.first_proposed).num_days())
            .max()
            .unwrap_or(0);
        Ok(Health {
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_s: self.inner.started_at.elapsed().as_secs(),
            pending_count: open.iter().filter(|q| !q.internal).count() as u32,
            overdue_count: open
                .iter()
                .filter(|q| !q.internal && q.force_at.is_some_and(|force_at| force_at <= now))
                .count() as u32,
            last_answer_at: store::last_answer_at(&self.inner.pool).await?,
            oldest_pending_days,
            ledger_backlog: store::ledger_outbox_count(&self.inner.pool).await?,
            open_json_dirty: self.inner.open_dirty.load(std::sync::atomic::Ordering::Acquire),
            ledger_stuck_s: self.inner.ledger.stuck_since().map(|t| (now - t).num_seconds().max(0)).unwrap_or(0),
        })
    }

    // ------------------------------------------------------------------
    // 作答／稍後／撤回／取代
    // ------------------------------------------------------------------

    pub async fn answer_question(
        &self,
        id: &str,
        input: AnswerInput,
        now: Ts,
    ) -> Result<Question, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let Some(mut q) = store::get_question(pool, id).await? else {
            return Err(ServerError::NotFound(id.to_string()));
        };
        if q.state.is_terminal() {
            // 作答競態只承認一次（answer-flow 場景 12）。
            return Err(match &q.answer {
                Some(a) => ServerError::AlreadyAnswered {
                    id: id.to_string(),
                    channel: enum_str(&a.channel),
                },
                None => ServerError::Terminal { id: id.to_string(), state: q.state },
            });
        }
        // INV-15 inv_money_chain_ui_only（CONTRACT §6 規則 6）：錢題只認使用者本人的裝置——
        // channel=ui（Tauri IPC 物理點擊）或 mobile（已配對手機，INV-28）。cli / discord / 其他一律拒——
        // 否則本機 Bearer token 洩漏＝繞過物理點擊動錢。
        if q.is_money_op() && !input.channel.is_ray_device() {
            return Err(ServerError::MoneyAnswerRequiresUi {
                id: id.to_string(),
                channel: enum_str(&input.channel),
            });
        }
        // 所見即所答（AC-EC-05）：草稿題從面板作答一定要帶看到的版本與內容雜湊。
        if q.draft.is_some()
            && input.channel.is_ray_device()
            && (input.revision.is_none() || input.content_hash.as_deref().is_none_or(str::is_empty))
        {
            return Err(ServerError::BadAnswer(format!(
                "題目 {id} 有版本，作答要帶面板顯示的版本與內容雜湊（請重新開啟面板再回答）"
            )));
        }
        // 使用者看的版本已被重新註冊取代 → 拒收，請他看過新內容。
        if let Some(d) = q.draft.as_ref() {
            let seen = input.revision.unwrap_or(d.revision);
            let hash_differs = input.content_hash.as_deref().is_some_and(|h| h != d.content_hash);
            if seen != d.revision || hash_differs {
                return Err(ServerError::StaleRevision { id: id.to_string(), seen, current: d.revision });
            }
        }
        let mut input = input;
        // 題組（CONTRACT §14）：逐項決定取代選項；每項恰好一次、文字與指示規則、每項雜湊綁定。壞載荷不得終結題目。
        let batch_items = if q.kind == kunsu_core::types::Kind::Batch {
            if input.choice_id.is_some() || input.edited_params.is_some() {
                return Err(ServerError::BadAnswer(
                    "題組沒有選項：逐項帶 items（採用、不處理或請 AI 改）".to_string(),
                ));
            }
            let Some(decisions) = input.items.take() else {
                return Err(ServerError::BadAnswer(
                    "題組要逐項帶決定（items）：只能在 Kunsu 面板逐項作答".to_string(),
                ));
            };
            // 整體指示先檢查：全部退回時每項的指示就是這一段，先查逐項會被說成「第 1 項的指示」（QA 第 1 輪 AC-BR-04）。
            input.free_text = input.free_text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
            if let Some(len) = input.free_text.as_ref().map(|t| t.chars().count())
                && len > kunsu_core::batch::MAX_NOTE_CHARS
            {
                return Err(ServerError::BadAnswer(format!(
                    "整體指示（套用到所有項目的那段）超過 2,000 字（{len} 字）"
                )));
            }
            let decisions = kunsu_core::batch::check_answer(&q.items, decisions, input.channel == Channel::Ui)
                .map_err(ServerError::BadAnswer)?;
            Some(decisions)
        } else {
            if input.items.is_some() {
                return Err(ServerError::BadAnswer(format!(
                    "只有題組（kind: batch）可以帶 items；題目 {id} 不是題組"
                )));
            }
            None
        };
        // 作答載荷驗證：至少選項或文字其一；choice 必須存在於 options（確認題另接受 `confirm`，AC-DS-15）；
        // 文字 trim 後非空。選項＋補充說明可同時送出（決策面板 AC-DS-06），空白補充視為沒有。壞載荷不得終結題目。
        if input.choice_id.is_some()
            && input.free_text.as_deref().is_some_and(|t| t.trim().is_empty())
        {
            input.free_text = None;
        }
        if batch_items.is_none() {
        match (&input.choice_id, &input.free_text) {
            (Some(cid), Some(_)) => {
                if !q.accepts_choice(cid) {
                    return Err(ServerError::BadAnswer(format!(
                        "choice_id {cid:?} 不存在於本題 options"
                    )));
                }
            }
            (None, None) => {
                return Err(ServerError::BadAnswer(
                    "choice_id 與 free_text 至少要有一個".to_string(),
                ));
            }
            (Some(cid), None) => {
                if !q.accepts_choice(cid) {
                    return Err(ServerError::BadAnswer(format!(
                        "choice_id {cid:?} 不存在於本題 options"
                    )));
                }
            }
            (None, Some(text)) => {
                if text.trim().is_empty() {
                    return Err(ServerError::BadAnswer(
                        "free_text trim 後為空：不接受空白作答".to_string(),
                    ));
                }
            }
        }
        }
        if let Some(edited) = &input.edited_params {
            // 所見即所簽：編輯後值即生效 params_line（紅線重檢在 UI/HTTP 層）。
            q.params_line = Some(edited.clone());
            // INV-16（CONTRACT §6 規則 7）：編輯改金額 → 同步改寫 prepare cmd；
            // 認不得/失敗 → 升 red 鎖一鍵（作答仍落 ledger，執行鎖 ≠ 作答鎖）。
            let target = kunsu_core::redline::parse_money_change(edited).map(|c| c.to);
            rewrite_prepare_cmd_for_edit(&mut q, target);
        }
        q.answer = Some(Answer {
            answer_id: Uuid::new_v4().to_string(),
            choice_id: input.choice_id,
            free_text: input.free_text,
            edited_params: input.edited_params,
            channel: input.channel,
            answered_at: now,
            reauthorized_at: None,
            signature: input.signature,
            acked_at: None,
            executed_at: None,
            execution_result: None,
            delivery: None,
            report: None,
            revision: q.draft.as_ref().map(|d| d.revision),
            content_hash: q.draft.as_ref().map(|d| d.content_hash.clone()),
            items: batch_items,
        });
        let policy = self.policy();
        let guards = self.guard_state(now).await?;
        let (s, fx) = transition(&q, Event::Answered { channel: input.channel }, now, &policy, &guards);
        let acts = self.apply(&mut q, s, &fx, now, None).await?;
        for kref in acts {
            self.spawn_activation(&q, &kref, None, now).await?;
        }
        // 不需喚醒的題（wake=none）：仍登記一筆待決交付，等待者在線就交給它，
        // 否則下一次 reactor 掃描定案為 no_continuation（AC-CC-03）。
        if q.wake_on_answer.mode == WakeMode::None
            && !q.internal
            && let Some(a) = q.answer.as_ref()
        {
            store::put_delivery(pool, &q.id, &a.answer_id, "pending", None, Some(now), now).await?;
        }
        self.rewrite_open(now).await?;
        Ok(q)
    }

    pub async fn dismiss_question(&self, id: &str, now: Ts) -> Result<Question, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let Some(mut q) = store::get_question(pool, id).await? else {
            return Err(ServerError::NotFound(id.to_string()));
        };
        if q.state.is_terminal() {
            return Err(ServerError::Terminal { id: id.to_string(), state: q.state });
        }
        let policy = self.policy();
        let guards = self.guard_state(now).await?;
        let (s, fx) = transition(&q, Event::Dismiss, now, &policy, &guards);
        let dismissed = fx.iter().any(|e| matches!(e, Effect::IncrementCycle));
        let acts = self.apply(&mut q, s, &fx, now, None).await?;
        debug_assert!(acts.is_empty());
        if dismissed {
            store::insert_event(
                pool,
                Some(&q.id),
                "force_cycle_dismissed",
                &json!({
                    "question_id": q.id,
                    "cycle_number": q.cycle_number,
                    "next_warn_at": q.warn_at,
                    "next_force_at": q.force_at,
                }),
                now,
            )
            .await?;
        }
        self.rewrite_open(now).await?;
        Ok(q)
    }

    /// 舊客戶端相容入口；新 UI 應呼叫 dismiss_question。
    pub async fn snooze_question(&self, id: &str, now: Ts) -> Result<Question, ServerError> {
        self.dismiss_question(id, now).await
    }

    pub async fn cancel_question(&self, id: &str, now: Ts) -> Result<Question, ServerError> {
        self.terminal_event(id, Event::Cancel, now).await
    }

    /// UI「這題不該問我／退回重擬」（P2 IPC 契約 answer_question type=reject）：
    /// 映射 cancelled + 落出題品質計分事件（reject 率供週報統計）。
    pub async fn reject_question(
        &self,
        id: &str,
        reason: Option<String>,
        now: Ts,
    ) -> Result<Question, ServerError> {
        // 退回記在題目上（不只在 events）：終態帳本行帶著它，重建後督導頁仍分得出「退回」與 agent 取消。
        let reason = reason.map(|r| r.trim().chars().take(2000).collect::<String>()).filter(|r| !r.is_empty());
        let rejection = kunsu_core::types::Rejection { at: now, reason: reason.clone() };
        let q = self.terminal_event_with(id, Event::Cancel, now, move |q| q.rejected = Some(rejection)).await?;
        store::insert_event(
            &self.inner.pool,
            Some(&q.id),
            "quality_reject",
            &json!({
                "question_id": q.id,
                "source_key": store::source_key(&q.source),
                "requested_tier": q.requested_tier,
                "reason": reason,
            }),
            now,
        )
        .await?;
        Ok(q)
    }

    pub async fn supersede_question(&self, id: &str, now: Ts) -> Result<Question, ServerError> {
        self.terminal_event(id, Event::Supersede, now).await
    }

    async fn terminal_event(
        &self,
        id: &str,
        event: Event,
        now: Ts,
    ) -> Result<Question, ServerError> {
        self.terminal_event_with(id, event, now, |_| {}).await
    }

    /// 同 `terminal_event`，但在轉終態（寫帳本）之前先改題目欄位，讓帳本快照帶著它。
    async fn terminal_event_with(
        &self,
        id: &str,
        event: Event,
        now: Ts,
        mark: impl FnOnce(&mut Question),
    ) -> Result<Question, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let Some(mut q) = store::get_question(&self.inner.pool, id).await? else {
            return Err(ServerError::NotFound(id.to_string()));
        };
        if q.state.is_terminal() {
            return Err(ServerError::Terminal { id: id.to_string(), state: q.state });
        }
        mark(&mut q);
        let policy = self.policy();
        let guards = self.guard_state(now).await?;
        let (s, fx) = transition(&q, event, now, &policy, &guards);
        let acts = self.apply(&mut q, s, &fx, now, None).await?;
        debug_assert!(acts.is_empty());
        self.rewrite_open(now).await?;
        Ok(q)
    }

    // ------------------------------------------------------------------
    // 游標收割（ask_answers_since / drain 的服務面）
    // ------------------------------------------------------------------

    /// at-least-once：帶 cursor 即 ack 其之前所有事件；不帶則沿用存檔游標。
    /// 同 cursor 重呼叫重收同批——寧可重複、絕不丟失。drain 到即標 acked_at。
    pub async fn answers_since(
        &self,
        consumer: &str,
        cursor: Option<i64>,
        now: Ts,
    ) -> Result<AnswerBatch, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let stored = store::get_cursor(pool, consumer).await?;
        let effective = cursor.or(stored).unwrap_or(0);
        store::put_cursor(pool, consumer, effective, now).await?;
        let rows = store::terminal_events_after(pool, effective).await?;
        let mut events = Vec::new();
        let mut next_cursor = effective;
        for row in rows {
            next_cursor = row.seq;
            let Some(mut q) = store::get_question(pool, &row.question_id).await? else {
                continue;
            };
            // 回答延續原對話的題：drain 收割不算接手，要真的交到原對話才算（AC-CC-16）。
            let continuation =
                kunsu_core::continuity::continuation_applies(&q.source, &q.wake_on_answer, q.internal);
            let handed = q.answer.as_ref().and_then(|a| a.delivery.as_ref()).is_some_and(|d| {
                matches!(
                    d.state,
                    DeliveryState::LiveWaiter | DeliveryState::Resumed | DeliveryState::PickedUp | DeliveryState::Fresh
                )
            });
            let mut newly_acked = false;
            if let Some(a) = q.answer.as_mut()
                && a.acked_at.is_none()
                && (!continuation || handed)
            {
                a.acked_at = Some(now);
                newly_acked = true;
                store::put_question(pool, &q).await?;
            }
            let snapshot = q.clone();
            // 終態投遞完成 → routed（事件行保留，redelivery 不受影響）。
            let policy = self.policy();
            let (s, _) = transition(&q, Event::DeliveryComplete, now, &policy, &GuardState::default());
            if s != q.state {
                q.state = s;
                store::put_question(pool, &q).await?;
            }
            // 第一次收割標 acked_at 也進帳本：重建後仍是「已有人接手」，不會誤報 24 小時沒人處理（INV-13）。
            if newly_acked {
                self.append_ack_row(&q, now).await;
            }
            events.push(TerminalEvent {
                seq: row.seq,
                terminal_type: row.terminal_type,
                question: snapshot,
            });
        }
        Ok(AnswerBatch { events, next_cursor })
    }

    // ------------------------------------------------------------------
    // 回執回寫（drain/reactor 之後呼叫）
    // ------------------------------------------------------------------

    pub async fn record_ack(&self, id: &str, now: Ts) -> Result<(), ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let Some(mut q) = store::get_question(pool, id).await? else {
            return Err(ServerError::NotFound(id.to_string()));
        };
        if let Some(a) = q.answer.as_mut()
            && a.acked_at.is_none()
        {
            a.acked_at = Some(now);
            store::put_question(pool, &q).await?;
            self.append_ack_row(&q, now).await;
        }
        Ok(())
    }

    /// 收割／回執標 acked_at 的帳本行（`delivery` 類，重建時以題目快照還原）。
    async fn append_ack_row(&self, q: &Question, now: Ts) {
        let row = LedgerRow {
            ts: now,
            ledger_type: LedgerType::Delivery,
            internal: q.internal,
            row_id: None,
            question: q.clone(),
            answer: q.answer.clone(),
            execution: Some(json!({ "acked": true })),
        };
        self.append_ledger(&row, "ack").await;
    }

    /// 執行完成回寫：answer.executed_at/execution_result + ledger `execution` 行
    /// + reactor_runs 標 done。
    pub async fn record_execution(
        &self,
        id: &str,
        result: &str,
        now: Ts,
    ) -> Result<(), ServerError> {
        self.write_execution(id, result, true, now).await
    }

    /// 改寫執行紀錄但不動 reactor 列（例如死信之後補上錢鏈結果，reactor 列維持 dead）。
    pub async fn amend_execution(&self, id: &str, result: &str, now: Ts) -> Result<(), ServerError> {
        self.write_execution(id, result, false, now).await
    }

    async fn write_execution(
        &self,
        id: &str,
        result: &str,
        finish_run: bool,
        now: Ts,
    ) -> Result<(), ServerError> {
        let _g = self.inner.write_gate.lock().await;
        self.write_execution_locked(id, result, finish_run, now).await
    }

    /// `write_execution` 的本體（呼叫端已持有寫入鎖）。
    async fn write_execution_locked(
        &self,
        id: &str,
        result: &str,
        finish_run: bool,
        now: Ts,
    ) -> Result<(), ServerError> {
        let pool = &self.inner.pool;
        let Some(mut q) = store::get_question(pool, id).await? else {
            return Err(ServerError::NotFound(id.to_string()));
        };
        let Some(a) = q.answer.as_mut() else {
            return Err(ServerError::Terminal { id: id.to_string(), state: q.state });
        };
        // Kunsu 自己的執行紀錄一律寫入；AI 回饋另存在 `answer.report`，兩者互不覆寫（AC-CC-14）。
        a.executed_at = Some(now);
        a.execution_result = Some(result.to_string());
        if a.acked_at.is_none() {
            a.acked_at = Some(now);
        }
        let answer_id = a.answer_id.clone();
        store::put_question(pool, &q).await?;
        if finish_run {
            store::mark_reactor_done(pool, &answer_id, result, now).await?;
        }
        let row = LedgerRow {
            ts: now,
            ledger_type: LedgerType::Execution,
            internal: q.internal,
            row_id: None,
            question: q.clone(),
            answer: q.answer.clone(),
            execution: Some(json!({ "result": result })),
        };
        self.append_ledger(&row, "execution").await;
        self.emit(ServerEvent::LedgerAppended {
            question_id: q.id.clone(),
            ledger_type: LedgerType::Execution.as_str().to_string(),
        });
        Ok(())
    }

    // ------------------------------------------------------------------
    // approval 單次消耗（POST /approvals/verify 的服務面；INV-9）
    // ------------------------------------------------------------------

    /// 消耗一份 approval record：nonce 唯一鍵落 DB + ledger `approval_consumed` 行。
    /// 簽章/TTL 驗證在 `approval::verify_and_consume`（呼叫端）；本方法只管
    /// 「活體 DB 單次消耗 + 真相層留痕」。
    pub async fn consume_approval(
        &self,
        question_id: &str,
        nonce: &str,
        record: &serde_json::Value,
        now: Ts,
    ) -> Result<ApprovalOutcome, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let Some(q) = store::get_question(pool, question_id).await? else {
            return Ok(ApprovalOutcome::QuestionMissing);
        };
        // INV-9：與錢鏈記結果（`settle_unverified_nonce`）在同一段寫入鎖內——Kunsu 已判定結束或被取代的授權永遠花不掉。
        if store::nonce_consumed(pool, nonce).await? {
            return Ok(ApprovalOutcome::AlreadyConsumed);
        }
        if store::money_outcome_of(pool, nonce).await?.is_some() {
            return Ok(ApprovalOutcome::AlreadySettled);
        }
        if store::newer_issue_exists(pool, question_id, nonce).await? {
            return Ok(ApprovalOutcome::Superseded);
        }
        // approval 消耗 fail closed：帳本還有積欠補不進去就不消耗（不排隊，INV-9／INV-13）。
        if !self.flush_ledger_outbox().await {
            return Err(ServerError::LedgerBacklog(store::ledger_outbox_count(pool).await.unwrap_or(0)));
        }
        let fresh = store::insert_approval_consumed(pool, nonce, question_id, record, now).await?;
        if !fresh {
            return Ok(ApprovalOutcome::AlreadyConsumed);
        }
        store::insert_event(
            pool,
            Some(question_id),
            "approval_consumed",
            &json!({"question_id": question_id, "nonce": nonce}),
            now,
        )
        .await?;
        let _ledger = self.inner.ledger.append(&LedgerRow {
            ts: now,
            ledger_type: LedgerType::ApprovalConsumed,
            internal: q.internal,
            row_id: Some(uuid::Uuid::new_v4().to_string()),
            question: q.clone(),
            answer: q.answer.clone(),
            execution: Some(json!({ "approval_record": record })),
        })?;
        self.emit(ServerEvent::ApprovalConsumed {
            question_id: question_id.to_string(),
            nonce: nonce.to_string(),
        });
        Ok(ApprovalOutcome::Consumed)
    }

    /// 簽發登記（verify 強化）：簽章鏈簽出 record 當下呼叫。verify 端只認
    /// 登記過的 nonce/token/hash/時間——非 server 簽發的 record 一律拒。
    #[allow(clippy::too_many_arguments)]
    pub async fn record_approval_issued(
        &self,
        question_id: &str,
        answer_id: &str,
        nonce: &str,
        executor_token: &str,
        param_hash: &str,
        answered_at_epoch: i64,
        now: Ts,
    ) -> Result<(), ServerError> {
        store::insert_approval_issued(
            &self.inner.pool,
            nonce,
            question_id,
            answer_id,
            executor_token,
            param_hash,
            answered_at_epoch,
            now,
        )
        .await
    }

    /// 簽發登記查詢（verify 用）。
    pub async fn approval_issued(
        &self,
        nonce: &str,
    ) -> Result<Option<store::IssuedApproval>, ServerError> {
        store::get_approval_issued(&self.inner.pool, nonce).await
    }

    // ------------------------------------------------------------------
    // 重新授權（決策/授權分離；15 分鐘 approval 過期的恢復通道）
    // ------------------------------------------------------------------

    /// 對已答的 yellow 錢題重新授權執行：答案（決策）一字不動，只更新
    /// `answer.reauthorized_at` 並把 reactor run 重新入列——簽章鏈會重跑
    /// prepare → 所見即所簽比對 → 以新授權時間簽出新 record（新 token/nonce）。
    /// 只允許 UI 通道呼叫（Tauri IPC）；紅燈題、非錢題、非執行型答案一律拒。
    pub async fn reauthorize_question(
        &self,
        id: &str,
        now: Ts,
    ) -> Result<Question, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let Some(mut q) = store::get_question(pool, id).await? else {
            return Err(ServerError::NotFound(id.to_string()));
        };
        // 全部檢查（UI 通道的黃燈執行型答案、任一次授權都沒有執行完成、不在跑、沒有結果不明、列已終結）與 `money_state` 共用。
        if let Some(reason) = reauthorize_refusal(pool, &q).await? {
            return Err(ServerError::BadReauthorize(reason));
        }
        let Some(ans) = q.answer.clone() else {
            return Err(ServerError::BadReauthorize(format!("題目 {id} 尚未作答，沒有可重新授權的決策")));
        };
        // 重建後 reactor 列已清空：沒有列時建立一列（被擋的錢題重建後仍可重新授權）。
        let requeued = store::requeue_reactor(pool, &ans.answer_id, now).await?
            || store::enqueue_reactor(pool, &q.id, &ans.answer_id, q.internal, now).await?;
        if !requeued {
            return Err(ServerError::BadReauthorize(
                "執行尚未終結（仍在執行或等待人工對帳），不能重新授權".to_string(),
            ));
        }
        if let Some(a) = q.answer.as_mut() {
            a.reauthorized_at = Some(now);
        }
        store::put_question(pool, &q).await?;
        store::insert_event(
            pool,
            Some(&q.id),
            "reauthorized",
            &json!({"question_id": q.id, "answer_id": ans.answer_id, "reauthorized_at": now}),
            now,
        )
        .await?;
        self.emit(ServerEvent::ReactorEnqueued {
            question_id: q.id.clone(),
            answer_id: ans.answer_id.clone(),
            dry_run: q.internal,
        });
        Ok(q)
    }

    // ------------------------------------------------------------------
    // ask_update 的服務面（保守欄位集：不動倒數、不動狀態、不動 tier）
    // ------------------------------------------------------------------

    /// 更新非終態題的補充欄位（evidence / body_md / deep_link）。
    /// 與 upsert 同精神：只 touch 資訊面 + last_seen；不重置倒數、不發通知。
    pub async fn update_question_meta(
        &self,
        id: &str,
        patch: UpdatePatch,
        now: Ts,
    ) -> Result<Question, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let Some(mut q) = store::get_question(pool, id).await? else {
            return Err(ServerError::NotFound(id.to_string()));
        };
        // 草稿題只能改草稿再註冊（升版、所見即所答）；不能從旁偷改內容。
        if q.draft.is_some() {
            return Err(ServerError::BadDraft(format!(
                "題目 {id} 是草稿題：請修改 ~/.kunsu/drafts/{id}/ 後重新註冊（kunsu ask <資料夾>），版本會跟著更新"
            )));
        }
        if q.state.is_terminal() {
            return Err(ServerError::Terminal { id: id.to_string(), state: q.state });
        }
        if let Some(ev) = patch.evidence {
            if ev.len() > 3 {
                return Err(kunsu_core::validate::RejectReason::TooManyEvidence { len: ev.len() }.into());
            }
            if let Some(index) = ev.iter().position(|e| e.source.trim().is_empty()) {
                return Err(
                    kunsu_core::validate::RejectReason::EvidenceMissingSource { index }.into()
                );
            }
            q.evidence = ev;
        }
        if let Some(body) = patch.body_md {
            q.body_md = body;
        }
        if let Some(link) = patch.deep_link {
            q.deep_link = Some(link);
        }
        if let Some(title) = patch.title {
            let len = title.chars().count();
            if len > 80 {
                return Err(kunsu_core::validate::RejectReason::TitleTooLong { len }.into());
            }
            q.title = title;
        }
        if let Some(line) = patch.question_line {
            let len = line.chars().count();
            if len > 24 {
                return Err(
                    kunsu_core::validate::RejectReason::QuestionLineTooLong { len }.into()
                );
            }
            q.question_line = line;
        }
        if let Some(opts) = patch.options {
            if q.kind == kunsu_core::types::Kind::Batch {
                return Err(ServerError::BadEdit(
                    "題組沒有選項；項目內容只能經由草稿重新註冊改變（JSON 題組建立後不可改）".into(),
                ));
            }
            if q.money.as_ref().is_some_and(|m| m.is_money_op) {
                return Err(ServerError::BadEdit(
                    "錢題選項只能走 edit_option（紅線重檢路徑），ask_update 拒絕".into(),
                ));
            }
            if q.kind == kunsu_core::types::Kind::Choice && !(2..=4).contains(&opts.len()) {
                return Err(ServerError::BadEdit(format!(
                    "choice 題選項數必須 2-4，收到 {}",
                    opts.len()
                )));
            }
            q.options = opts;
        }
        if let Some(wake) = patch.wake_on_answer {
            q.wake_on_answer = wake;
        }
        q.last_seen = now;
        store::put_question(pool, &q).await?;
        self.emit(ServerEvent::QuestionTouched { question_id: q.id.clone() });
        self.rewrite_open(now).await?;
        Ok(q)
    }

    // ------------------------------------------------------------------
    // 選項行內編輯（P2 IPC 契約 edit_option 的服務面）
    // ------------------------------------------------------------------

    /// 鉛筆行內編輯：選項 label 重寫 + params_line 重算（所見即所簽）+
    /// 紅線重檢。**超線升 red 必須落 DB**——reactor 的一鍵通道判準是
    /// `gate != red`（CONTRACT §6），只在 UI 視覺升紅會留下真的錢洞。
    /// 嚴格大於才升（恰 2,000 / 恰 20% 不升，P2 定案）。
    pub async fn edit_option(
        &self,
        id: &str,
        option_id: &str,
        new_to: f64,
        now: Ts,
    ) -> Result<(Question, kunsu_core::redline::RedlineResult), ServerError> {
        use kunsu_core::redline::{check_redline, parse_money_change, recalc_line};
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let Some(mut q) = store::get_question(pool, id).await? else {
            return Err(ServerError::NotFound(id.to_string()));
        };
        if q.state.is_terminal() {
            return Err(ServerError::Terminal { id: id.to_string(), state: q.state });
        }
        let Some(opt) = q.options.iter_mut().find(|o| o.id == option_id) else {
            return Err(ServerError::BadEdit(format!("選項不存在：{option_id}")));
        };
        let Some(parsed) = parse_money_change(&opt.label) else {
            return Err(ServerError::BadEdit("此選項無可編輯的金額變更".to_string()));
        };
        opt.label = recalc_line(&opt.label, new_to);
        let new_label = opt.label.clone();
        let redline = check_redline(parsed.from, new_to);
        // params_line 同步重算（所見即所簽）。
        match &q.params_line {
            Some(line) if parse_money_change(line).is_some() => {
                q.params_line = Some(recalc_line(line, new_to));
            }
            _ => {
                let entity = q
                    .money
                    .as_ref()
                    .and_then(|m| m.entity_id.clone())
                    .unwrap_or_else(|| q.id.clone());
                q.params_line = Some(format!("{entity}｜{new_label}"));
            }
        }
        // INV-16（CONTRACT §6 規則 7）：編輯改金額 → 同步改寫
        // money.executor_prepare_cmd 的數值引數；認不得/失敗 → 升 red 鎖一鍵。
        rewrite_prepare_cmd_for_edit(&mut q, Some(new_to));
        if redline.breached {
            q.gate = Some(Gate::Red); // 升紅落 DB：reactor 一鍵通道即刻失效
        }
        q.last_seen = now;
        store::put_question(pool, &q).await?;
        store::insert_event(
            pool,
            Some(&q.id),
            "option_edited",
            &json!({
                "question_id": q.id,
                "option_id": option_id,
                "from": parsed.from,
                "new_to": new_to,
                "breached": redline.breached,
            }),
            now,
        )
        .await?;
        self.emit(ServerEvent::QuestionTouched { question_id: q.id.clone() });
        self.rewrite_open(now).await?;
        Ok((q, redline))
    }

    // ------------------------------------------------------------------
    // Kongko 發佈事件（POST /events/kongko 的服務面）
    // ------------------------------------------------------------------

    pub async fn kongko_event(
        &self,
        ev: KongkoPublication,
        now: Ts,
    ) -> Result<KongkoOutcome, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        if ev.event_type != "ad_publication_confirmed" {
            return Err(ServerError::BadKongkoEvent(format!(
                "未知 type {:?}（只接受 ad_publication_confirmed）",
                ev.event_type
            )));
        }
        if ev.publication_id.trim().is_empty() || ev.batch_id.trim().is_empty() {
            return Err(ServerError::BadKongkoEvent(
                "publicationId 與 batchId 不可為空".to_string(),
            ));
        }
        // 冪等（P1 定案 5）：同 publicationId 重複投遞 no-op。
        let payload = serde_json::to_value(&ev)?;
        let fresh =
            store::record_kongko_event(pool, &ev.publication_id, &ev.batch_id, &payload, now)
                .await?;
        if !fresh {
            self.emit(ServerEvent::KongkoDuplicate { publication_id: ev.publication_id.clone() });
            return Ok(KongkoOutcome {
                duplicate: true,
                closed_question_id: None,
                activation_question_id: None,
            });
        }
        store::insert_event(pool, None, "kongko_event", &payload, now).await?;

        let Some(mut q) = store::open_question_by_kongko_ref(pool, &ev.batch_id).await? else {
            // 無匹配題：留痕（DB 事件行）不報錯、不誤結（kongko 場景 5）。
            return Ok(KongkoOutcome {
                duplicate: false,
                closed_question_id: None,
                activation_question_id: None,
            });
        };
        // INV-15 inv_money_chain_ui_only：kongko 發佈事件只結案素材題。
        // 命中的題若是錢題（agent 建錯），拒絕自動結案並大聲留痕——錢只認 UI 點擊。
        if q.is_money_op() {
            self.inner.discord.send(DiscordMessage {
                question_id: Some(q.id.clone()),
                reason: "kongko_money_refused".to_string(),
                text: format!(
                    "⚠️ Kongko batch {} 命中的題「{}」是動錢題，拒絕由發佈事件自動結案——錢題只能在 Kunsu app 或已配對的手機批准",
                    ev.batch_id, q.title
                ),
            });
            return Ok(KongkoOutcome {
                duplicate: false,
                closed_question_id: None,
                activation_question_id: None,
            });
        }
        // 題組只能在面板逐項作答（CONTRACT §14）：建題時已拒收「題組＋kongko_ref」，這裡防的是規則之前的舊資料——
        // 不用一段文字把題組結案（沒有逐項決定，agent 會拿到錯的格式）。
        if q.kind == kunsu_core::types::Kind::Batch {
            tracing::warn!("Kongko batch {} 命中題組 {}：題組只能在面板逐項作答，不由發佈事件結案", ev.batch_id, q.id);
            return Ok(KongkoOutcome {
                duplicate: false,
                closed_question_id: None,
                activation_question_id: None,
            });
        }
        // 發佈即同意（素材層）：結案 channel=kongko。
        q.answer = Some(Answer {
            answer_id: Uuid::new_v4().to_string(),
            choice_id: None,
            free_text: Some(format!("Kongko 發佈確認：publication {}", ev.publication_id)),
            edited_params: None,
            channel: Channel::Kongko,
            answered_at: now,
            reauthorized_at: None,
            signature: None,
            acked_at: None,
            executed_at: None,
            execution_result: None,
            delivery: None,
            report: None,
            revision: q.draft.as_ref().map(|d| d.revision),
            content_hash: q.draft.as_ref().map(|d| d.content_hash.clone()),
            items: None,
        });
        let policy = self.policy();
        let guards = self.guard_state(now).await?;
        let (s, fx) =
            transition(&q, Event::Answered { channel: Channel::Kongko }, now, &policy, &guards);
        let acts = self.apply(&mut q, s, &fx, now, None).await?;
        let mut activation_question_id = None;
        for kref in acts {
            activation_question_id = self.spawn_activation(&q, &kref, Some(&ev), now).await?;
        }
        self.rewrite_open(now).await?;
        Ok(KongkoOutcome {
            duplicate: false,
            closed_question_id: Some(q.id.clone()),
            activation_question_id,
        })
    }

    /// 發佈≠啟用：自動建獨立 T0 啟用題（真的走完整建題路徑：驗證＋T0 准入）。
    /// id 由 batchId 決定 → 重複事件天然 upsert 冪等。
    async fn spawn_activation(
        &self,
        source_q: &Question,
        kongko_ref: &str,
        ev: Option<&KongkoPublication>,
        now: Ts,
    ) -> Result<Option<String>, ServerError> {
        let req = build_activation_request(
            source_q,
            kongko_ref,
            ev,
            self.inner.ledger.vault_root().to_string_lossy().as_ref(),
            now,
        );
        match self.create_inner(req, None, now).await {
            Ok(outcome) => {
                self.emit(ServerEvent::ActivationQuestionCreated {
                    question_id: outcome.question_id.clone(),
                    kongko_ref: kongko_ref.to_string(),
                });
                Ok(Some(outcome.question_id))
            }
            Err(e) => {
                // 啟用題生成失敗必須大聲（安全方向：錢不會動，但使用者沒被問到）。
                self.inner.discord.send(DiscordMessage {
                    question_id: Some(source_q.id.clone()),
                    reason: "activation_create_failed".to_string(),
                    text: format!("⚠️ Kongko batch {kongko_ref} 發佈已結案，但啟用題生成失敗：{e}"),
                });
                Ok(None)
            }
        }
    }

    // ------------------------------------------------------------------
    // 勿擾（持久化 presentation guard + audit）
    // ------------------------------------------------------------------

    pub async fn kill_switch(&self, hours: u32, now: Ts) -> Result<Ts, ServerError> {
        let minutes = hours.checked_mul(60).ok_or_else(|| ServerError::BadDnd("時長溢位".into()))?;
        let status = self.set_dnd(minutes, "user", now).await?;
        let until = status.ends_at.expect("active DND has ends_at");
        let _g = self.inner.write_gate.lock().await;
        store::insert_event(
            &self.inner.pool,
            None,
            "fatigue",
            &json!({"type": "kill_switch", "hours": hours, "until": until}),
            now,
        )
        .await?;
        self.emit(ServerEvent::KillSwitch { until });
        Ok(until)
    }

    pub async fn dnd_status(&self, now: Ts) -> Result<DndStatus, ServerError> {
        let Some(raw) = store::get_setting(&self.inner.pool, DND_SETTING_KEY).await? else {
            return Ok(DndStatus { active: false, starts_at: None, ends_at: None, remaining_seconds: 0, actor: None });
        };
        let record: DndRecord = serde_json::from_str(&raw)?;
        if now >= record.ends_at {
            return Ok(DndStatus { active: false, starts_at: None, ends_at: None, remaining_seconds: 0, actor: None });
        }
        Ok(DndStatus {
            active: true,
            starts_at: Some(record.starts_at),
            ends_at: Some(record.ends_at),
            remaining_seconds: (record.ends_at - now).num_seconds().max(0),
            actor: Some(record.actor),
        })
    }

    pub async fn set_dnd(&self, duration_minutes: u32, actor: &str, now: Ts) -> Result<DndStatus, ServerError> {
        if !(1..=DND_MAX_MINUTES).contains(&duration_minutes) {
            return Err(ServerError::BadDnd(format!(
                "duration_minutes 必須在 1..={DND_MAX_MINUTES}（收到 {duration_minutes}）"
            )));
        }
        let actor = actor.trim();
        if actor.is_empty() || actor.chars().count() > 80 {
            return Err(ServerError::BadDnd("actor 必須是 1..=80 字的非空字串".into()));
        }
        let _g = self.inner.write_gate.lock().await;
        let previous = self.dnd_status(now).await?;
        let record = DndRecord {
            starts_at: now,
            ends_at: now + Duration::minutes(duration_minutes as i64),
            actor: actor.to_string(),
        };
        store::put_setting(
            &self.inner.pool,
            DND_SETTING_KEY,
            &serde_json::to_string(&record)?,
            now,
        )
        .await?;
        store::insert_event(
            &self.inner.pool,
            None,
            "dnd_changed",
            &json!({"actor": actor, "duration_minutes": duration_minutes, "old_ends_at": previous.ends_at, "new_ends_at": record.ends_at}),
            now,
        )
        .await?;
        let status = DndStatus {
            active: true,
            starts_at: Some(record.starts_at),
            ends_at: Some(record.ends_at),
            remaining_seconds: duration_minutes as i64 * 60,
            actor: Some(record.actor),
        };
        self.emit(ServerEvent::DndChanged { active: true, ends_at: status.ends_at });
        Ok(status)
    }

    pub async fn clear_dnd(&self, actor: &str, now: Ts) -> Result<DndStatus, ServerError> {
        let actor = actor.trim();
        if actor.is_empty() || actor.chars().count() > 80 {
            return Err(ServerError::BadDnd("actor 必須是 1..=80 字的非空字串".into()));
        }
        let _g = self.inner.write_gate.lock().await;
        let previous = self.dnd_status(now).await?;
        store::delete_setting(&self.inner.pool, DND_SETTING_KEY).await?;
        if previous.active {
            store::insert_event(
                &self.inner.pool,
                None,
                "dnd_changed",
                &json!({"actor": actor, "old_ends_at": previous.ends_at, "new_ends_at": null}),
                now,
            )
            .await?;
            self.emit(ServerEvent::DndChanged { active: false, ends_at: None });
        }
        Ok(DndStatus { active: false, starts_at: None, ends_at: None, remaining_seconds: 0, actor: None })
    }

    // ------------------------------------------------------------------
    // timer loop 內核（1s 掃 due rows；重啟從 DB 重建）
    // ------------------------------------------------------------------

    /// 掃描一輪 due rows 並餵 Tick。回傳處理題數。timer loop 每 1s 呼叫；
    /// 測試直接以受控 `now` 呼叫。
    pub async fn tick(&self, now: Ts) -> Result<u32, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        // 帳本待補寫佇列：權限恢復後不必等下一次寫入，tick 每 30 秒自動補寫一次（INV-13）。
        let backlog_due = {
            let mut last = self.inner.last_backlog_attempt.lock().expect("backlog lock");
            let due = last.is_none_or(|t| now - t >= chrono::Duration::seconds(30));
            if due {
                *last = Some(now);
            }
            due
        };
        self.check_ledger_stuck(now);
        let open_dirty = self.inner.open_dirty.load(std::sync::atomic::Ordering::Acquire);
        if backlog_due
            && (open_dirty || store::ledger_outbox_count(pool).await? > 0)
            && self.flush_ledger_outbox().await
            && open_dirty
        {
            self.rewrite_open(now).await?;
        }
        let policy = self.policy();
        let base_guards = self.guard_state(now).await?;
        let due = store::due_questions(pool, now).await?;
        let mut processed = 0u32;
        for mut q in due {
            q.ensure_force_cycle(&policy);
            let (s, fx) = transition(&q, Event::Tick, now, &policy, &base_guards);
            let acts = self.apply(&mut q, s, &fx, now, None).await?;
            debug_assert!(acts.is_empty(), "Tick 不產生啟用題");
            processed += 1;
        }
        self.check_unacked_answers(now).await?;
        if processed > 0 {
            self.rewrite_open(now).await?;
        }
        Ok(processed)
    }

    /// CONTRACT §6 規則 9：answered 且 24h 無任何 agent ack → Discord 告警；
    /// `no_ack_alert` 事件標記防重複（每題最多一次）。
    /// 在途的 vault 讀寫卡住超過 2 分鐘：發一次 Discord 請使用者重啟（卡在核心的呼叫不會自己回來，之後的寫入都只能排隊）。
    fn check_ledger_stuck(&self, now: Ts) {
        let Some(since) = self.inner.ledger.stuck_since() else { return };
        if (now - since).num_seconds() < LEDGER_STUCK_ALERT_SECS {
            return;
        }
        // 每一次卡住（以開始時刻區分）各發一次：上一次剛解除、下一筆馬上又卡住也看得見。
        let fresh = {
            let mut alerted = self.inner.ledger_stuck_alerted.lock().expect("stuck lock");
            let fresh = *alerted != Some(since);
            *alerted = Some(since);
            fresh
        };
        if fresh {
            self.inner.discord.send(crate::discord::DiscordMessage {
                question_id: None,
                reason: "ledger_stuck".to_string(),
                text: format!(
                    "⚠️ Kunsu 讀寫 vault 帳本卡住超過 {} 分鐘（系統層的檔案呼叫沒有回來），之後的帳本行都只能排隊。\
請重新啟動 Kunsu：答案都安全在 app 資料庫，重啟後會自動依序補寫帳本。",
                    LEDGER_STUCK_ALERT_SECS / 60
                ),
            });
        }
    }

    async fn check_unacked_answers(&self, now: Ts) -> Result<(), ServerError> {
        let pool = &self.inner.pool;
        let cutoff = now - Duration::hours(24);
        for q in store::answered_unacked(pool).await? {
            if q.internal {
                continue;
            }
            let Some(ans) = &q.answer else { continue };
            if ans.answered_at > cutoff {
                continue;
            }
            if store::has_event_for_question(pool, &q.id, "no_ack_alert").await? {
                continue;
            }
            // 交付已記 dead 的題：中斷或死信當下已告警過，不再重複。
            if ans.delivery.as_ref().is_some_and(|d| d.state == DeliveryState::Dead) {
                continue;
            }
            self.inner.discord.send(DiscordMessage {
                question_id: Some(q.id.clone()),
                reason: "no_ack_24h".to_string(),
                text: format!("⚠️ 你答的「{}」24 小時沒有任何 agent 處理", q.title),
            });
            store::insert_event(
                pool,
                Some(&q.id),
                "no_ack_alert",
                &json!({"question_id": q.id, "answered_at": ans.answered_at}),
                now,
            )
            .await?;
        }
        Ok(())
    }

    /// 本刻 guards 是否全綠（shell 的批次窗「開蓋板連答」判準；
    /// 含情境旗標、quiet hours 與 kill-switch，不含 forced 預算）。
    /// 畫布顯示失敗的診斷事件（BDD evidence-canvas 場景 9）。
    pub async fn record_canvas_failure(&self, question_id: &str, revision: u32, reason: &str, now: Ts) -> Result<(), ServerError> {
        let reason: String = reason.chars().take(500).collect();
        store::insert_event(
            &self.inner.pool,
            Some(question_id),
            "canvas_failed",
            &json!({"question_id": question_id, "revision": revision, "reason": reason}),
            now,
        )
        .await?;
        Ok(())
    }

    /// 某題的畫布失敗紀錄（原因文字，新到舊）。
    pub async fn canvas_failures(&self, question_id: &str) -> Result<Vec<String>, ServerError> {
        Ok(store::events_for_question(&self.inner.pool, question_id, "canvas_failed")
            .await?
            .iter()
            .filter_map(|v| v.get("reason").and_then(|r| r.as_str()).map(str::to_string))
            .collect())
    }

    /// 這個答案的 reactor 列（錢鏈、交付）還沒跑完。
    /// 這題錢的結果是否不明（agent 讀取面、等待者、追加訊息共用，CONTRACT §7）。
    pub async fn money_outcome_unknown(&self, q: &Question) -> Result<bool, ServerError> {
        money_outcome_unknown(&self.inner.pool, q).await
    }

    /// 記下一次授權（nonce）的錢鏈結果，並寫一行帶 `money_outcome` 的 execution 帳本行（重建用）。
    pub async fn record_money_outcome(&self, question_id: &str, nonce: &str, result: &str, now: Ts) -> Result<(), ServerError> {
        let _g = self.inner.write_gate.lock().await;
        self.record_money_outcome_locked(question_id, nonce, result, now).await
    }

    async fn record_money_outcome_locked(&self, question_id: &str, nonce: &str, result: &str, now: Ts) -> Result<(), ServerError> {
        self.record_money_outcome_row(question_id, nonce, result, false, now).await
    }

    /// 記一次授權的結果＋帳本行；`backfill` 標明是舊資料補結果（CONTRACT §7）。呼叫端已持有寫入鎖。
    async fn record_money_outcome_row(&self, question_id: &str, nonce: &str, result: &str, backfill: bool, now: Ts) -> Result<(), ServerError> {
        let pool = &self.inner.pool;
        store::put_money_outcome(pool, nonce, question_id, result, now).await?;
        if let Some(q) = store::get_question(pool, question_id).await? {
            let outcome = if backfill {
                json!({ "nonce": nonce, "result": result, "backfill": true })
            } else {
                json!({ "nonce": nonce, "result": result })
            };
            let row = LedgerRow {
                ts: now,
                ledger_type: LedgerType::Execution,
                internal: q.internal,
                row_id: None,
                question: q.clone(),
                answer: q.answer.clone(),
                execution: Some(json!({ "money_outcome": outcome })),
            };
            self.append_ledger(&row, "money_outcome").await;
        }
        Ok(())
    }

    /// commit 沒有通過 executor 驗證就失敗時記這次授權的結果（CONTRACT §7）：查消耗與記結果在同一段寫入鎖內，與
    /// `/approvals/verify` 的消耗（`consume_approval`）互斥。已被消耗 → 不記、回 false（呼叫端改記結果不明）；否則記下結果、回 true，
    /// 之後這個 nonce 一律驗證失敗（INV-9）。
    pub async fn settle_unverified_nonce(&self, question_id: &str, nonce: &str, result: &str, now: Ts) -> Result<bool, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        if store::nonce_consumed(&self.inner.pool, nonce).await? {
            return Ok(false);
        }
        self.record_money_outcome_locked(question_id, nonce, result, now).await?;
        Ok(true)
    }

    /// 已作答錢題的錢的狀態（CONTRACT §7）：shell 附在送給網頁的題目上（`money_state`），回執依它給按鈕。非錢題、還沒作答 → None。
    pub async fn money_state(&self, q: &Question) -> Result<Option<MoneyState>, ServerError> {
        let pool = &self.inner.pool;
        if !q.is_money_op() || q.answer.is_none() {
            return Ok(None);
        }
        if money_chain_running(pool, q).await? {
            return Ok(Some(MoneyState::Running));
        }
        if money_outcome_unknown(pool, q).await? {
            return Ok(Some(MoneyState::Unknown));
        }
        if reauthorize_refusal(pool, q).await?.is_none() {
            return Ok(Some(MoneyState::Reauthorizable));
        }
        Ok(Some(MoneyState::Settled))
    }

    /// 錢鏈列還沒跑完（錢鏈適用、這個答案的 reactor 列 queued／executing，含之後同一列的交付）：回執不給按鈕（CONTRACT §7）。
    pub async fn money_chain_running(&self, q: &Question) -> Result<bool, ServerError> {
        money_chain_running(&self.inner.pool, q).await
    }

    /// 錢鏈本身還在跑：錢鏈列還沒跑完，而且這一輪的錢鏈結果還沒寫進執行紀錄。agent 讀取面與追加訊息此時一律說「錢鏈還在跑」；
    /// 錢鏈跑完、同一列還在交付時照錢鏈結果說（CONTRACT §4、§8）。
    pub async fn money_result_pending(&self, q: &Question) -> Result<bool, ServerError> {
        Ok(money_chain_running(&self.inner.pool, q).await? && !chain_result_written(q))
    }

    /// 這題的錢已執行完成：任一次授權的結果是 `COMMIT_OK`／`RECONCILED_EXECUTED`，或執行紀錄以它們開頭（重啟恢復第 3 種）。
    pub async fn money_executed(&self, q: &Question) -> Result<bool, ServerError> {
        money_executed(&self.inner.pool, q).await
    }

    /// 啟動時與 `rebuild-db` 重放後的冪等補資料（CONTRACT §7「舊資料補結果」）：已被消耗卻沒有結果的授權，若這題執行紀錄開頭是
    /// `COMMIT_OK`／`RECONCILED_EXECUTED`，補上同樣開頭的結果並寫一行 `backfill` 帳本行。回傳補了幾筆。呼叫端已持有寫入鎖。
    async fn backfill_money_outcomes_locked(&self, now: Ts) -> Result<u32, ServerError> {
        let pool = &self.inner.pool;
        let pending = store::consumed_without_outcome(pool).await?;
        let mut per_question: HashMap<&str, usize> = HashMap::new();
        for (question_id, _) in &pending {
            *per_question.entry(question_id.as_str()).or_default() += 1;
        }
        let mut n = 0;
        for (question_id, nonce) in &pending {
            // 推論只對產生那個結果的那一次成立：同一題有兩次以上未記結果的授權（較早那次的結果真的不明）就整題不補，由使用者對帳。
            if per_question.get(question_id.as_str()).copied().unwrap_or(0) != 1 {
                continue;
            }
            let Some(q) = store::get_question(pool, question_id).await? else { continue };
            if !q.is_money_op() {
                continue;
            }
            let raw = q.answer.as_ref().and_then(|a| a.execution_result.as_deref()).unwrap_or("").trim_start();
            let Some(prefix) = BACKFILL_PREFIXES.iter().find(|p| raw.starts_with(*p)) else { continue };
            let result = format!("{prefix}: 補資料：migration 0007 之前的授權，依執行紀錄開頭推定");
            self.record_money_outcome_row(question_id, nonce, &result, true, now).await?;
            n += 1;
        }
        Ok(n)
    }

    /// 舊版重啟恢復把「executor 驗證前中斷」也標成 needs_reconcile（CONTRACT §7）：錢題的 needs_reconcile 列若沒有結果不明的授權、
    /// 錢也沒有執行完成，照重啟恢復第 2 種處理（簽發但沒有結果的授權記 `EXEC_BLOCKED`、執行紀錄同樣內容、列改 dead），不發 Discord。
    async fn settle_legacy_interrupted_money_locked(&self, now: Ts) -> Result<u32, ServerError> {
        let pool = &self.inner.pool;
        let mut n = 0;
        for run in store::needs_reconcile_runs(pool).await? {
            let Some(q) = store::get_question(pool, &run.question_id).await? else { continue };
            if !q.is_money_op() || q.answer.as_ref().is_none_or(|a| a.answer_id != run.answer_id) {
                continue;
            }
            if !store::unknown_money_nonces(pool, &q.id).await?.is_empty() || money_executed(pool, &q).await? {
                continue;
            }
            for nonce in store::issued_unsettled_nonces(pool, &q.id).await? {
                self.record_money_outcome_locked(&q.id, &nonce, INTERRUPTED_BEFORE_VERIFY, now).await?;
            }
            self.write_execution_locked(&q.id, INTERRUPTED_BEFORE_VERIFY, false, now).await?;
            store::mark_reactor_dead(pool, &run.answer_id, INTERRUPTED_BEFORE_VERIFY, now).await?;
            n += 1;
        }
        Ok(n)
    }

    /// 使用者人工對帳（只由 UI IPC 呼叫，CONTRACT §7）：結果不明的錢題標「錢已經動了／錢沒有動」。只接受確實有結果不明授權的題，
    /// 錢鏈還在跑時拒絕；某次授權已 COMMIT_OK 的題沒有結果不明的授權，永遠推翻不了。
    pub async fn resolve_money_outcome(&self, id: &str, executed: bool, now: Ts) -> Result<Question, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let Some(mut q) = store::get_question(pool, id).await? else {
            return Err(ServerError::NotFound(id.to_string()));
        };
        let Some(answer_id) = q.answer.as_ref().map(|a| a.answer_id.clone()) else {
            return Err(ServerError::BadReauthorize(format!("題目 {id} 尚未作答，沒有錢要對帳")));
        };
        if money_chain_running(pool, &q).await? {
            return Err(ServerError::BadReauthorize("錢鏈還在跑，等它跑完再看結果（以 Kunsu 回執為準）".to_string()));
        }
        let nonces = if q.is_money_op() { store::unknown_money_nonces(pool, id).await? } else { Vec::new() };
        if nonces.is_empty() {
            return Err(ServerError::BadReauthorize("這題沒有結果不明、等人工對帳的錢".to_string()));
        }
        let result = if executed {
            "RECONCILED_EXECUTED: 使用者人工對帳確認錢已經動了"
        } else {
            "RECONCILED_NOT_EXECUTED: 使用者人工對帳確認錢沒有動"
        };
        if let Some(a) = q.answer.as_mut() {
            a.execution_result = Some(result.to_string());
            a.executed_at = Some(now);
        }
        store::put_question(pool, &q).await?;
        for nonce in &nonces {
            self.record_money_outcome_locked(id, nonce, result, now).await?;
        }
        store::finish_reconciled_run(pool, &answer_id, result, now).await?;
        store::insert_event(pool, Some(id), "money_reconciled", &json!({"question_id": id, "executed": executed, "nonces": nonces}), now)
            .await?;
        self.emit(ServerEvent::LedgerAppended { question_id: id.to_string(), ledger_type: LedgerType::Execution.as_str().to_string() });
        Ok(q)
    }

    pub async fn execution_pending(&self, answer_id: &str) -> Result<bool, ServerError> {
        store::reactor_run_open(&self.inner.pool, answer_id).await
    }

    /// 隱私 guard（分享螢幕、攝影機、麥克風、前景全螢幕任一命中）：瀏海跑馬燈此時不顯示題目內容（AC-EN-12）。
    /// 到期被延後時的原因（BDD edge-notch 場景 10c）：與狀態機同一份 guard 快照。
    pub async fn defer_reasons(&self, now: Ts) -> Result<kunsu_core::guards::DeferReasons, ServerError> {
        let policy = self.policy();
        Ok(self.guard_state(now).await?.defer_reasons(now, &policy))
    }

    pub fn privacy_guard_active(&self) -> bool {
        let snap = self.inner.guards.snapshot();
        snap.screen_shared || snap.camera_on || snap.mic_on || snap.frontmost_fullscreen
    }

    pub async fn guards_all_green(&self, now: Ts) -> Result<bool, ServerError> {
        let policy = self.policy();
        let guards = self.guard_state(now).await?;
        Ok(!guards.blocks_force(now, &policy))
    }

    // ------------------------------------------------------------------
    // rebuild（inv_rebuild_from_ledger：SQLite 可整顆重建）
    // ------------------------------------------------------------------

    /// ledger（jsonl 終態）+ OPEN.json（pending）→ 完整重建 DB。
    /// consumer 游標重置的後果只是 at-least-once 重放（消化端冪等吸收）。
    pub async fn rebuild_db(&self) -> Result<RebuildReport, ServerError> {
        self.rebuild_db_at(chrono::Utc::now()).await
    }

    /// `rebuild_db` 的時鐘可注入版本：未定案的交付（pending／held）排在 `now` 重檢。
    pub async fn rebuild_db_at(&self, now: Ts) -> Result<RebuildReport, ServerError> {
        let _g = self.inner.write_gate.lock().await;
        let pool = &self.inner.pool;
        let refuse = ServerError::RebuildRefused;
        // 帳本還有沒補寫的變更（只在資料庫裡）：先補寫，補不進去就拒絕重建，不丟資料（INV-13）。
        if !self.flush_ledger_outbox().await {
            let n = store::ledger_outbox_count(pool).await.unwrap_or(0);
            return Err(refuse(format!(
                "帳本還有 {n} 行沒補寫（多半是 Kunsu 沒有完全磁碟權限），修好權限讓 Kunsu 補寫完再重建"
            )));
        }
        // OPEN.json 上次沒寫成：先重寫，仍失敗就拒絕（只在資料庫裡的待決題不能丟）。
        if self.inner.open_dirty.load(std::sync::atomic::Ordering::Acquire) {
            self.rewrite_open(now).await?;
            if self.inner.open_dirty.load(std::sync::atomic::Ordering::Acquire) {
                return Err(refuse(
                    "待決題快照 OPEN.json 寫不進 vault（多半是 Kunsu 沒有完全磁碟權限），修好權限後再重建".to_string(),
                ));
            }
        }
        // 先完整讀完 OPEN.json 與所有帳本才清資料庫：讀不到（權限、逾時）就拒絕，資料庫保持原樣（INV-13）。
        let open = self.inner.ledger.read_open().map_err(|e| refuse(format!("讀不到待決題快照 OPEN.json：{e}")))?;
        let (rows, bad) = self.inner.ledger.read_all_rows().map_err(|e| refuse(format!("讀不到 vault 帳本：{e}")))?;
        // 重建永不丟掉資料庫裡的題目：非 internal 的題在帳本與 OPEN.json 都找不到就拒絕（vault 搬家、帳本檔不見）。
        let mut known: std::collections::HashSet<String> = rows.iter().map(|r| r.question.id.clone()).collect();
        if let Some(o) = &open {
            known.extend(o.pending.iter().map(|q| q.id.clone()));
        }
        let missing: Vec<String> =
            store::list_all(pool).await?.into_iter().filter(|q| !q.internal && !known.contains(&q.id)).map(|q| q.id).collect();
        if !missing.is_empty() {
            let shown: Vec<&str> = missing.iter().take(10).map(String::as_str).collect();
            let bad_note = if bad.is_empty() {
                String::new()
            } else {
                let lines: Vec<&str> = bad.iter().take(5).map(String::as_str).collect();
                format!("；帳本有 {} 行壞掉無法解析（{}{}），這些題可能就在壞行裡", bad.len(), lines.join("；"), if bad.len() > 5 { "…" } else { "" })
            };
            return Err(refuse(format!(
                "資料庫有 {} 題在 vault 帳本與 OPEN.json 都找不到（{}{}），重建會把它們丟掉——先確認 vault 路徑、權限與帳本檔{bad_note}",
                missing.len(),
                shown.join("、"),
                if missing.len() > shown.len() { "…" } else { "" }
            )));
        }
        store::wipe_all(pool).await?;
        let mut report = RebuildReport::default();
        // 先 OPEN（pending 快照），再 ledger 依序重放（終態覆蓋較舊 pending 快照）。
        if let Some(open) = open {
            let policy = self.policy();
            for mut q in open.pending {
                q.ensure_force_cycle(&policy);
                store::put_question(pool, &q).await?;
                report.pending_restored += 1;
            }
        }
        report.bad_lines = bad;
        for row in rows {
            match row.ledger_type {
                LedgerType::Answered
                | LedgerType::Expired
                | LedgerType::Cancelled
                | LedgerType::Superseded => {
                    store::put_question(pool, &row.question).await?;
                    store::insert_event(
                        pool,
                        Some(&row.question.id),
                        "terminal",
                        &json!({
                            "type": row.ledger_type.as_str(),
                            "question_id": row.question.id,
                            "internal": row.internal,
                        }),
                        row.ts,
                    )
                    .await?;
                    report.terminal_rows += 1;
                }
                LedgerType::Execution | LedgerType::Delivery => {
                    store::put_question(pool, &row.question).await?;
                    // 每一次授權的錢鏈結果（CONTRACT §7）：重建後「結果不明」的判斷與重建前一致。
                    if let Some(o) = row.execution.as_ref().and_then(|e| e.get("money_outcome"))
                        && let (Some(nonce), Some(result)) = (o.get("nonce").and_then(|v| v.as_str()), o.get("result").and_then(|v| v.as_str()))
                    {
                        store::put_money_outcome(pool, nonce, &row.question.id, result, row.ts).await?;
                    }
                    report.execution_rows += 1;
                }
                // approval 消耗狀態恢復（survival.md 場景 5：單次性不因重建失效）。
                LedgerType::ApprovalConsumed => {
                    if let Some(rec) =
                        row.execution.as_ref().and_then(|e| e.get("approval_record"))
                        && let Some(nonce) = rec.get("nonce").and_then(|n| n.as_str())
                    {
                        store::insert_approval_consumed(
                            pool,
                            nonce,
                            &row.question.id,
                            rec,
                            row.ts,
                        )
                        .await?;
                    }
                    report.approval_rows += 1;
                }
            }
        }
        // 舊帳本（migration 0007 之前沒有 money_outcome 行）：已執行完成的授權補結果，不被誤判結果不明（CONTRACT §7，冪等）。
        self.backfill_money_outcomes_locked(now).await?;
        // 超過 24 小時的答案可能早就處理過、只是當時帳本缺行：不自動重做，列進報告請使用者確認（INV-13）。
        let window = chrono::Duration::hours(REBUILD_REQUEUE_WINDOW_HOURS);
        let item = |q: &Question, a: &Answer| RebuildItem { id: q.id.clone(), title: q.title.clone(), answered_at: a.answered_at };
        // 交付索引由題目上的交付狀態還原（帳本 `delivery` 行重放後的最後快照）：24 小時內的 pending／held 於重建時刻重檢，
        // resuming 交給卡住規則，其餘已定案不再交付（INV-13、INV-18）。
        for q in store::list_all(pool).await? {
            if let Some(a) = q.answer.as_ref()
                && let Some(d) = a.delivery.as_ref()
            {
                let old = now - a.answered_at > window && q.wake_on_answer.mode != WakeMode::None;
                let next = (!d.state.is_final() && !old).then_some(now);
                store::restore_delivery(pool, &q.id, &a.answer_id, d.state.as_str(), d.reason.as_deref(), next, d.updated_at).await?;
                if old && !d.state.is_final() && !q.internal {
                    report.stale.push(item(&q, a));
                }
            }
        }
        // 已回答、需要喚醒、還沒有交付也沒有執行紀錄的答案：重新排進 reactor，不能因為重建就永遠不交付。
        // 錢題若 approval 已消耗卻沒有執行紀錄，可能已經動過錢：不論作答多久都不重跑，請使用者人工對帳（先於 24 小時窗）。
        for q in store::list_all(pool).await? {
            let Some(a) = q.answer.as_ref() else { continue };
            if !matches!(q.state, State::Answered | State::Routed) {
                continue;
            }
            // 錢的結果不明（某一次授權已消耗卻沒有結果，或 EXEC_UNKNOWN 還沒對帳）：可能已經動過錢，不論作答多久、有沒有舊紀錄
            // 都不重跑，列進人工對帳請使用者在回執標記結果（先於 24 小時窗）。
            if money_outcome_unknown(pool, &q).await? {
                report.reconcile += 1;
                report.reconcile_items.push(item(&q, a));
                // 已記成 EXEC_UNKNOWN 的當下已告警過：列進報告，但不再逐題發 Discord。
                if a.execution_result.as_deref().is_some_and(|r| r.trim_start().starts_with("EXEC_UNKNOWN")) {
                    continue;
                }
                self.inner.discord.send(crate::discord::DiscordMessage {
                    question_id: Some(q.id.clone()),
                    reason: "rebuild_reconcile".to_string(),
                    text: format!(
                        "⚠️ 重建資料庫後，「{}」的 approval 已消耗但沒有之後的執行紀錄——錢可能已經動過，Kunsu 不會重跑，請人工對帳。",
                        q.title
                    ),
                });
                continue;
            }
            let untouched = a.delivery.is_none() && a.executed_at.is_none() && a.execution_result.is_none();
            if !untouched {
                continue;
            }
            // 不需喚醒的題：跟作答當下一樣登記待決交付，下一次 reactor 掃描定案 no_continuation（AC-CC-03）。
            // 錢題不論 wake 都要跑錢鏈，照下面的規則重新排入。
            if q.wake_on_answer.mode == WakeMode::None {
                if !q.internal {
                    store::put_delivery(pool, &q.id, &a.answer_id, "pending", None, Some(now), now).await?;
                }
                if !q.is_money_op() {
                    continue;
                }
            }
            if now - a.answered_at > window {
                if !q.internal {
                    report.stale.push(item(&q, a));
                }
                continue;
            }
            if store::enqueue_reactor(pool, &q.id, &a.answer_id, q.internal, now).await? {
                report.requeued += 1;
            }
        }
        report.stale_unexecuted = report.stale.len() as u32;
        // 重建清掉了 events（含「已告警」標記）：超過 24 小時還沒被認領的答案補上標記——已在重建報告請使用者確認，
        // 或重建前就告警過——重建後不再逐題發「24 小時沒有任何 agent 處理」。
        for q in store::answered_unacked(pool).await? {
            if let Some(a) = q.answer.as_ref()
                && !q.internal
                && now - a.answered_at > window
            {
                store::insert_event(
                    pool,
                    Some(&q.id),
                    "no_ack_alert",
                    &json!({"question_id": q.id, "answered_at": a.answered_at, "restored_by_rebuild": true}),
                    now,
                )
                .await?;
            }
        }
        if !report.stale.is_empty() {
            let listed: Vec<String> = report
                .stale
                .iter()
                .map(|i| format!("「{}」（{} 作答）", i.title, i.answered_at.with_timezone(&chrono::Local).format("%m/%d %H:%M")))
                .collect();
            self.inner.discord.send(crate::discord::DiscordMessage {
                question_id: None,
                reason: "rebuild_stale_unexecuted".to_string(),
                text: format!(
                    "⚠️ 重建資料庫後，有 {} 題超過 24 小時的答案在帳本裡沒有處理完的紀錄，Kunsu 不會自動重做（可能早就處理過、只是當時帳本缺行）。請確認：{}",
                    listed.len(),
                    listed.join("、")
                ),
            });
        }
        Ok(report)
    }

    // ------------------------------------------------------------------
    // Effect 解釋器（tests/common/mod.rs apply() 參考實作的 I/O 擴充版）
    // ------------------------------------------------------------------

    /// 套用 transition 回傳：欄位變更 → 落 DB → 副作用（ledger/廣播/Discord/
    /// reactor）。回傳需要生成啟用題的 kongko_refs（只有 Answered(kongko) 會有）。
    async fn apply(
        &self,
        q: &mut Question,
        new_state: State,
        fx: &[Effect],
        now: Ts,
        _forced_counter: Option<&mut u32>,
    ) -> Result<Vec<String>, ServerError> {
        let pool = &self.inner.pool;
        let from = q.state;
        q.state = new_state;
        let mut activations = Vec::new();

        // pass 1：純欄位變更（與 kunsu-core tests/common/mod.rs apply() 對齊）。
        for e in fx {
            match e {
                Effect::IncrementSnooze => q.snooze_count += 1,
                Effect::IncrementCycle => q.cycle_number = q.cycle_number.saturating_add(1),
                Effect::RestartForceCycle { cycle_started_at, warn_at, force_at } => {
                    q.cycle_started_at = Some(*cycle_started_at);
                    q.warn_at = Some(*warn_at);
                    q.force_at = Some(*force_at);
                }
                Effect::DowngradeTier { to } => {
                    q.tier = *to;
                    q.tier_downgrade_reason =
                        Some("snooze 封頂：連續稍後超過 2 次，自動降級永不再蓋板".to_string());
                }
                Effect::MarkForcedBudgetUsed => q.forced_today_used = true,
                Effect::ScheduleNext { at } => q.next_transition_at = *at,
                Effect::RunDefault { action } => self.run_default_fields(q, action, now),
                _ => {}
            }
        }
        store::put_question(pool, q).await?;
        if from != q.state {
            store::insert_event(
                pool,
                Some(&q.id),
                "state_changed",
                &json!({"from": from, "to": q.state}),
                now,
            )
            .await?;
            self.emit(ServerEvent::StateChanged {
                question_id: q.id.clone(),
                from,
                to: q.state,
            });
        }

        // pass 2：副作用。
        for e in fx {
            match e {
                Effect::Notify { kind } => {
                    self.emit(ServerEvent::Notify { question_id: q.id.clone(), kind: *kind });
                }
                Effect::MarkForcedBudgetUsed => {
                    if !q.internal {
                        store::insert_event(
                            pool,
                            Some(&q.id),
                            "forced_budget",
                            &json!({"question_id": q.id}),
                            now,
                        )
                        .await?;
                    }
                }
                Effect::Discord { reason } => {
                    let (code, text) = match reason {
                        DiscordReason::SnoozeCapDowngrade => (
                            "snooze_cap_downgrade",
                            format!("😴 「{}」連續稍後 3 次，已自動降 T1，永不再蓋板", q.title),
                        ),
                        DiscordReason::MoneyExpired => (
                            "money_expired",
                            format!("💸 錢類題「{}」超時未答：已歸檔，錢一分未動", q.title),
                        ),
                    };
                    if *reason == DiscordReason::SnoozeCapDowngrade {
                        store::insert_event(
                            pool,
                            Some(&q.id),
                            "fatigue",
                            &json!({"type": "snooze_cap_downgrade", "question_id": q.id}),
                            now,
                        )
                        .await?;
                    }
                    if !q.internal {
                        self.inner.discord.send(DiscordMessage {
                            question_id: Some(q.id.clone()),
                            reason: code.to_string(),
                            text,
                        });
                    }
                    self.emit(ServerEvent::DiscordQueued {
                        question_id: Some(q.id.clone()),
                        reason: code.to_string(),
                    });
                }
                Effect::RunDefault { action } => {
                    if action.on_expire == OnExpire::EscalateDiscord && !q.internal {
                        self.inner.discord.send(DiscordMessage {
                            question_id: Some(q.id.clone()),
                            reason: "expired_escalate".to_string(),
                            text: format!("⏰ 「{}」過期未答，依出生宣告升級到 Discord", q.title),
                        });
                    }
                }
                Effect::WriteLedger { terminal } => {
                    let lt: LedgerType = (*terminal).into();
                    store::insert_event(
                        pool,
                        Some(&q.id),
                        "terminal",
                        &json!({
                            "type": lt.as_str(),
                            "question_id": q.id,
                            "internal": q.internal,
                        }),
                        now,
                    )
                    .await?;
                    let row = LedgerRow {
                        ts: now,
                        ledger_type: lt,
                        internal: q.internal,
                        row_id: None,
                        question: q.clone(),
                        answer: q.answer.clone(),
                        execution: None,
                    };
                    self.append_ledger(&row, "append(effect)").await;
                    self.emit(ServerEvent::LedgerAppended {
                        question_id: q.id.clone(),
                        ledger_type: lt.as_str().to_string(),
                    });
                }
                Effect::SpawnReactor => {
                    if let Some(ans) = &q.answer {
                        // internal 題只 dry-run（P1 定案 10）；answer_id UNIQUE 冪等。
                        let newly = store::enqueue_reactor(
                            pool,
                            &q.id,
                            &ans.answer_id,
                            q.internal,
                            now,
                        )
                        .await?;
                        if newly {
                            self.emit(ServerEvent::ReactorEnqueued {
                                question_id: q.id.clone(),
                                answer_id: ans.answer_id.clone(),
                                dry_run: q.internal,
                            });
                        }
                    }
                }
                Effect::GenerateActivationQuestion { kongko_ref } => {
                    activations.push(kongko_ref.clone());
                }
                _ => {}
            }
        }
        Ok(activations)
    }

    /// RunDefault 的欄位面：pick_option 記自動選定（channel=auto_policy）；
    /// 帶 standing_policy_ref 的題標 24h 否決窗（P1 定案 3）。
    /// 錢類永不到 pick_option——core expire_effects 已擋，此處不再出現。
    fn run_default_fields(&self, q: &mut Question, action: &DefaultAction, now: Ts) {
        if action.on_expire == OnExpire::PickOption && q.answer.is_none() {
            q.answer = Some(Answer {
                answer_id: Uuid::new_v4().to_string(),
                choice_id: action.option_id.clone(),
                free_text: None,
                edited_params: None,
                channel: Channel::AutoPolicy,
                answered_at: now,
                reauthorized_at: None,
                signature: None,
                acked_at: None,
                executed_at: None,
                execution_result: None,
                delivery: None,
                report: None,
                revision: q.draft.as_ref().map(|d| d.revision),
                content_hash: q.draft.as_ref().map(|d| d.content_hash.clone()),
                items: None,
            });
            if q.standing_policy_ref.is_some() {
                q.veto_until = Some(now + Duration::hours(24));
            }
        }
    }
}

/// INV-16 `inv_edited_params_rewrite_prepare_cmd`（CONTRACT §6 規則 7）：
/// 編輯改動金額時同步改寫 `money.executor_prepare_cmd` 的數值引數
/// （prepare-budget `--new-budget N` / prepare-bid `--new-value N` 兩型）。
/// `new_to=None`＝編輯後金額解析不出來；連同改寫失敗一律升 red 鎖一鍵並寫
/// `tier_downgrade_reason`（作答/編輯本身仍成立——執行鎖 ≠ 作答鎖）。
fn rewrite_prepare_cmd_for_edit(q: &mut Question, new_to: Option<f64>) {
    let Some(money) = q.money.as_ref() else { return };
    if !money.is_money_op {
        return;
    }
    let Some(cmd) = money.executor_prepare_cmd.clone() else { return };
    let outcome = match new_to {
        Some(v) => kunsu_core::prepare_cmd::rewrite_prepare_cmd(&cmd, v),
        None => Err("編輯後的金額解析不出來（找不到 X → Y 金額變更）".to_string()),
    };
    match outcome {
        Ok(rewritten) => {
            if let Some(m) = q.money.as_mut() {
                m.executor_prepare_cmd = Some(rewritten);
            }
        }
        Err(e) => {
            q.gate = Some(Gate::Red);
            q.tier_downgrade_reason = Some(format!(
                "inv_edited_params_rewrite_prepare_cmd：編輯後無法安全改寫 prepare cmd（{e}）\
                 ——升紅鎖一鍵，拍板僅落 ledger 不執行"
            ));
        }
    }
}

/// 疲勞降載的降一級映射（T3 已是地板，不再降）。
fn next_tier_down(t: Tier) -> Tier {
    match t {
        Tier::T0 => Tier::T1,
        Tier::T1 => Tier::T2,
        Tier::T2 | Tier::T3 => Tier::T3,
    }
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn is_blank(s: Option<&str>) -> bool {
    s.is_none_or(|s| s.trim().is_empty())
}

/// T0 准入判準（P1 定案 2）。回傳不合格原因清單（空=合格）。
/// 「安全預設」的程式化判準：至少一個未標 recommended 的選項
/// （recommended 是執行型推薦；非推薦選項＝不執行的安全出口）。
fn t0_admission_failures(req: &CreateRequest) -> Vec<String> {
    let mut fails = Vec::new();
    let is_money = req.money.as_ref().is_some_and(|m| m.is_money_op);
    if !(is_money || req.irreversible) {
        fails.push("非動錢且未標 irreversible".to_string());
    }
    if is_blank(req.params_line.as_deref()) {
        fails.push("缺 params_line".to_string());
    }
    if is_blank(req.prediction_ref.as_deref()) {
        fails.push("缺 prediction_ref".to_string());
    }
    if req.deadline_at.is_none() {
        fails.push("缺 deadline_at（T0 必須有真實時限）".to_string());
    }
    let has_safe_default =
        !req.options.is_empty() && req.options.iter().any(|o| o.recommended != Some(true));
    if !has_safe_default {
        fails.push("options 缺安全預設（至少一個非執行型選項）".to_string());
    }
    fails
}

/// 從 publication ads 結構化資料抽 campaign entity（純數字）。
/// 全部 ads 必須帶同一個 campaign id 才算資料充分；否則 None。
fn extract_campaign_entity(ev: Option<&KongkoPublication>) -> Option<String> {
    let e = ev?;
    if e.ads.is_empty() {
        return None;
    }
    let mut ids = std::collections::BTreeSet::new();
    for ad in &e.ads {
        let id = ["campaign_id", "campaignId", "entity_id", "entityId"]
            .iter()
            .find_map(|k| {
                let v = ad.get(*k)?;
                match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                }
            })?;
        if id.is_empty() || id.len() > 32 || !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        ids.insert(id);
    }
    if ids.len() == 1 {
        ids.into_iter().next()
    } else {
        None
    }
}

/// 從 AdPublication 預填的獨立 T0 啟用題（kongko 場景 4 + Codex 覆檢 P1）。
/// id 由 batchId 決定（重送 upsert 冪等）；prediction_ref 沿用素材題、缺則
/// 指到約定路徑（由發佈落地 playbook 補寫檔）。
///
/// **啟用題必須進完整錢鏈**：publication 結構化資料充分（campaign entity +
/// 日預算 + product 皆合白名單格式）→ 組白名單 prepare 操作（campaign entity
/// + 現行預算），使用者點「啟用」走 prepare→比對→簽章→commit 全鏈；
///
/// 資料不足 → gate=red + body 註明缺什麼，**絕不呈現可一鍵**（red 鎖一鍵，
/// 執行走使用者本人終端）。金額 >NT$2,000 依 executor 紅線同值升紅。
fn build_activation_request(
    source_q: &Question,
    kongko_ref: &str,
    ev: Option<&KongkoPublication>,
    vault_cwd: &str,
    now: Ts,
) -> CreateRequest {
    let ads_n = ev.map(|e| e.ads.len()).unwrap_or(0);
    let amount_twd = ev.and_then(|e| {
        let total: i64 = e
            .ads
            .iter()
            .filter_map(|ad| {
                ["daily_budget_twd", "dailyBudgetTwd", "daily_budget", "budget_twd"]
                    .iter()
                    .find_map(|k| ad.get(k).and_then(|v| v.as_i64()))
            })
            .sum();
        (total > 0).then_some(total)
    });
    let entity = extract_campaign_entity(ev);
    let product = ev
        .and_then(|e| e.product.clone())
        .or_else(|| source_q.product.clone())
        .filter(|p| {
            !p.is_empty()
                && p.len() <= 40
                && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        });

    // 錢鏈資料充分性判定（白名單格式全過才算）。
    let mut missing: Vec<&str> = Vec::new();
    if entity.is_none() {
        missing.push("campaign entity（ads[].campaign_id 純數字）");
    }
    if amount_twd.is_none() {
        missing.push("日預算（ads[].daily_budget_twd）");
    }
    if product.is_none() {
        missing.push("product（白名單字元）");
    }
    let sufficient = missing.is_empty();

    // 資料不足或金額 >NT$2,000（executor 紅線同值）→ 紅燈鎖一鍵，絕不呈現可一鍵啟用。
    let gate = if !sufficient || amount_twd.is_some_and(|a| a > 2000) {
        Gate::Red
    } else {
        Gate::Yellow
    };

    let (money, params_line) = if sufficient {
        let e = entity.clone().expect("sufficient 已檢查");
        let a = amount_twd.expect("sufficient 已檢查");
        let p = product.clone().expect("sufficient 已檢查");
        (
            Money {
                is_money_op: true,
                amount_twd: Some(a),
                entity_id: Some(e.clone()),
                executor_prepare_cmd: Some(format!(
                    "python3 {} prepare-budget --entity {e} --new-budget {a} --product {p}",
                    crate::approval::EXECUTOR_SCRIPT_REL
                )),
            },
            // 所見即所簽：params_line 的目標值＝prepare 的 --new-budget（現行預算）。
            format!(
                "{e}｜啟用 batch {kongko_ref}：{ads_n} 則 PAUSED→ACTIVE，日預算 NT$0 → NT${a}"
            ),
        )
    } else {
        let budget_part = amount_twd
            .map(|a| format!("，日預算合計 NT${a}"))
            .unwrap_or_default();
        (
            Money { is_money_op: true, amount_twd, entity_id: None, executor_prepare_cmd: None },
            format!("啟用 batch {kongko_ref}：{ads_n} 則廣告 PAUSED→ACTIVE{budget_part}"),
        )
    };

    let mut body_md = format!(
        "Kongko batch `{kongko_ref}` 已發佈（{ads_n} 則廣告，Meta 端 PAUSED、零花費）。\n\
         發佈=素材上架同意；啟用=花錢同意，必須分開點擊。"
    );
    if !sufficient {
        body_md.push_str(&format!(
            "\n\n⚠️ **錢鏈資料不足，已升紅燈鎖一鍵**——缺：{}。\n\
             紅燈題拍板只落 ledger，實際啟用必須由使用者本人終端執行 executor。",
            missing.join("、")
        ));
    }

    let prediction_ref = source_q.prediction_ref.clone().unwrap_or_else(|| {
        format!(
            "projects/meta-ads/predictions/{}-kongko-{}-activation.md",
            taipei_date(now),
            kongko_ref
        )
    });
    let title: String = format!("啟用 Kongko 批次 {kongko_ref} 的廣告（發佈≠啟用）")
        .chars()
        .take(80)
        .collect();
    CreateRequest {
        id: format!("pa-activate-{kongko_ref}"),
        title,
        question_line: "要開始花錢啟用嗎？".to_string(),
        body_md,
        why_md: None,
        visual_symbol: source_q.visual_symbol.clone(),
        kind: Kind::Choice,
        tier: Tier::T0,
        product: ev.and_then(|e| e.product.clone()).or_else(|| source_q.product.clone()),
        gate: Some(gate),
        evidence: vec![EvidenceLine {
            text: format!("Kongko 發佈確認：batch {kongko_ref}（{ads_n} 則廣告，PAUSED）"),
            source: "kongko-webhook".to_string(),
        }],
        params_line: Some(params_line),
        options: vec![
            Opt {
                id: "activate".to_string(),
                label: "啟用（開始花錢）".to_string(),
                detail: None,
                recommended: Some(true),
            },
            Opt {
                id: "keep_paused".to_string(),
                label: "維持 PAUSED".to_string(),
                detail: None,
                recommended: None,
            },
        ],
        allow_free_text: true,
        money: Some(money),
        decision_ref: source_q.decision_ref.clone(),
        prediction_ref: Some(prediction_ref),
        kongko_ref: None, // 啟用題不再被發佈事件結案——它等使用者的第二次物理點擊
        deep_link: source_q.deep_link.clone(),
        irreversible: false,
        standing_policy_ref: None,
        internal: false,
        default_action: DefaultAction { on_expire: OnExpire::NoopLog, option_id: None },
        deadline_at: Some(now + Duration::hours(24)),
        wake_on_answer: Wake {
            mode: WakeMode::Fresh,
            session_id: None,
            playbook: Some("playbooks/approved-money-op.md".to_string()),
        },
        source: Source {
            agent: AgentKind::Other,
            session_id: None,
            cwd: vault_cwd.to_string(),
            routine_id: Some("kunsu-kongko".to_string()),
            ..Default::default()
        },
        first_proposed: None,
        items: vec![],
    }
}
