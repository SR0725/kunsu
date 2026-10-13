//! 服務層錯誤。HTTP 層之後把 `Reject`/`BadRequest` 類映射為 400、
//! `QuotaExceeded` 為 429 或 400、其他為 500；錯誤訊息必須是 agent 可讀的修法。

use kunsu_core::validate::RejectReason;
use kunsu_core::State;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// kunsu-core 硬驗證拒收（CONTRACT §2；HTTP 層回 400）。
    #[error("拒收：{0}")]
    Reject(#[from] RejectReason),

    /// per-source 每日建題配額超額（CONTRACT §4；把壓力逼回 agent 端合併）。
    #[error("source {source_key} 今日建題配額已滿（{limit} 題）：請合併題目，明日配額重置")]
    QuotaExceeded { source_key: String, limit: u32 },

    #[error("題目 {0} 不存在")]
    NotFound(String),

    /// 追問回覆拒絕（沒有這則追問、空白、太長；INV-31）。
    #[error("追問回覆拒絕：{0}")]
    BadFollowup(String),

    /// 作答競態：後到通道看到已 answered，不覆寫（answer-flow 場景 12）。
    #[error("題目 {id} 已被作答（channel={channel}），答案不覆寫")]
    AlreadyAnswered { id: String, channel: String },

    /// 錢鏈通道白名單（CONTRACT §6 規則 6 / INVARIANTS 15
    /// `inv_money_chain_ui_only`）：錢題只認使用者在 app 的物理點擊。
    #[error("錢題只能在 Kunsu app 或已配對的手機批准：題目 {id} 是動錢題，channel={channel} 不允許作答")]
    MoneyAnswerRequiresUi { id: String, channel: String },

    /// 對終態題做非法操作（cancel/supersede/snooze 已終態題）。
    #[error("題目 {id} 已在終態 {state:?}，不接受此操作")]
    Terminal { id: String, state: State },

    /// kongko webhook payload 非法（HTTP 層回 400，可讀原因）。
    #[error("kongko 事件非法：{0}")]
    BadKongkoEvent(String),

    /// 選項行內編輯拒絕（無金額變更可編輯等；IPC 層回人話）。
    #[error("編輯拒絕：{0}")]
    BadEdit(String),

    /// 作答載荷驗證失敗（P2：choice 必須存在於 options、free_text 非空、
    /// choice/text 互斥；HTTP 層回 400）。
    #[error("作答拒絕：{0}")]
    BadAnswer(String),

    /// 重新授權拒絕（決策/授權分離：只有已答的 yellow 錢題能 re-authorize）。
    #[error("重新授權拒絕：{0}")]
    BadReauthorize(String),

    #[error("勿擾設定拒絕：{0}")]
    BadDnd(String),

    /// `ask_report` 拒絕（未作答、answer_id 不符、空白回報；AC-CC-12）。
    #[error("回報拒絕：{0}")]
    BadReport(String),

    /// 草稿註冊的 I/O 失敗（快照寫入／讀取）。
    #[error("草稿處理失敗：{0}")]
    BadDraft(String),

    /// 作答時看的版本已被重新註冊取代（AC-EC-05）：請使用者看過新版本再決定。
    #[error("題目 {id} 的內容已更新為第 {current} 版（你看的是第 {seen} 版），請看過新內容再回答")]
    StaleRevision { id: String, seen: u32, current: u32 },

    /// 帳本待補寫佇列還有積欠、補不進去（多半是沒有完全磁碟權限）。
    #[error("帳本還有 {0} 行沒補寫（多半是 Kunsu 沒有完全磁碟權限）：先修好權限讓 Kunsu 補寫帳本，再做這件事，否則會丟掉只在資料庫裡的變更")]
    LedgerBacklog(u64),

    /// 重建前的安全檢查沒過（OPEN.json 待重寫、帳本讀不到或看起來被清空）：資料庫保持原樣。
    #[error("拒絕重建資料庫（資料庫保持原樣）：{0}")]
    RebuildRefused(String),

    #[error("DB 錯誤：{0}")]
    Db(#[from] sqlx::Error),

    #[error("migration 錯誤：{0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("I/O 錯誤：{0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 錯誤：{0}")]
    Json(#[from] serde_json::Error),
}
