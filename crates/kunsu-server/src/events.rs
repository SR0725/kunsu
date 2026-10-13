//! 通知/Discord/UI 事件全走 tokio broadcast（CONTRACT：Monitor 預武裝 WS、
//! Tauri shell 通知、Discord 鏡像都是這條通道的訂閱者）。本階段只廣播不真發。

use kunsu_core::state_machine::NotifyKind;
use kunsu_core::time::Ts;
use kunsu_core::types::{State, Tier};
use serde::Serialize;

/// 服務層對外事件。WS `/events` 之後每個 frame 就是一筆序列化的 ServerEvent。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    QuestionCreated { question_id: String, tier: Tier },
    /// upsert 去重 touch（不發通知，僅供 UI 刷新證據）。
    QuestionTouched { question_id: String },
    StateChanged { question_id: String, from: State, to: State },
    /// 通知/浮條/蓋板（shell 層解讀 NotifyKind 呈現）。
    Notify { question_id: String, kind: NotifyKind },
    /// Discord 鏡像已排入發送器（本階段 stub 只記錄）。
    DiscordQueued { question_id: Option<String>, reason: String },
    LedgerAppended { question_id: String, ledger_type: String },
    ReactorEnqueued { question_id: String, answer_id: String, dry_run: bool },
    /// 發佈≠啟用：kongko 結案後自動建的獨立 T0 啟用題。
    ActivationQuestionCreated { question_id: String, kongko_ref: String },
    /// 同 publicationId 重複投遞被冪等吸收。
    KongkoDuplicate { publication_id: String },
    /// T1 批次窗（CONTRACT §6 規則 8）：timer 跨越 policy t1_batch 時刻（台北，
    /// 每窗每日一次）且存在非終態 T1。shell 收到 → 系統通知 +（guards 全綠時）
    /// 開蓋板連答；Discord 靜默略過。
    BatchWindow { window: String, count: u32 },
    /// 全域免打擾（fatigue event 已落盤）。
    KillSwitch { until: Ts },
    /// Kunsu 勿擾狀態變更；shell 必須重新對帳 notification/capsule/overlay。
    DndChanged { active: bool, ends_at: Option<Ts> },
    /// approval record 被 `/approvals/verify` 單次消耗（INV-9 落 DB+ledger 後廣播）。
    ApprovalConsumed { question_id: String, nonce: String },
    /// 執行流串流（Tauri shell 之後映射為 `kunsu://execution` frame；P2 IPC 契約）。
    Execution {
        question_id: String,
        step: ExecStep,
        status: ExecStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<String>,
    },
    /// reactor 重試耗盡進死信（Discord 告警已排入）。
    ReactorDead { question_id: String, answer_id: String, reason: String },
    /// 答案交付狀態變更（回答延續原對話；回執刷新）。
    DeliveryChanged { question_id: String, state: String },
    /// 手機「問本人」的追問：`reply` 為 None 是新追問（叫醒等待者），Some 是原 AI 的回覆（INV-31：只是文字）。
    Followup { question_id: String, followup_id: String, reply: Option<String> },
}

/// 執行流步驟（P2 IPC 契約 `kunsu://execution` payload.step）。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecStep {
    Prepare,
    Guard,
    Api,
    Ledger,
}

/// 執行流狀態（P2 IPC 契約 payload.status）。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecStatus {
    Start,
    Ok,
    Fail,
}
