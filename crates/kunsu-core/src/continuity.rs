//! 回答延續原對話（PRD §20、BDD conversation-continuity）：交付階梯的純函數。
//!
//! 一個答案只會走一條交付路徑（`inv_single_continuation`）：
//! 等待中的原對話 → 原對話已結束時追加一輪 → 原對話仍被開著就等它空下來 → 都不行才開新對話。
//! 本模組零 I/O：登記檔掃描、pid 存活、resume 結果都由 server 層探測後以輸入傳入。

use serde::{Deserialize, Serialize};

use crate::types::{AgentKind, Source, Wake, WakeMode};

/// 發問的主機（由 `kunsu ask` 依環境變數判定，或 agent 自填）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKind {
    ClaudeDesktop,
    ClaudeCli,
    CodexApp,
    CodexCli,
    Other,
}

/// 這題是排程觸發還是互動 session 觸發（routine-check-in 場景 1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Scheduled,
    Interactive,
}

/// 排程種類（routine-check-in 場景 1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineKind {
    ClaudeScheduledTask,
    CodexCron,
    CodexHeartbeat,
    Other,
}

/// 答案交付狀態。`Pending`/`Held` 之外都是最終狀態，一旦寫入永不改變。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    /// 還沒決定（等待者租約內、或 reactor 尚未處理）。
    Pending,
    /// 原對話仍被活行程持有且沒有在等；定期重檢。
    Held,
    /// 已佔住交付權、正在原對話追加（spawn 中）；其他路徑不能再認領。
    Resuming,
    /// 交給等待中的原對話（`kunsu wait`／`ask_wait`）。
    LiveWaiter,
    /// 在原對話背景追加一輪。
    Resumed,
    /// 原對話自己（例如 Codex heartbeat）讀走答案並回報。
    PickedUp,
    /// 無法接續，開新對話處理。
    Fresh,
    /// 題目沒有原對話身分，也不需要喚醒。
    NoContinuation,
    /// 開新對話也失敗，進死信。
    Dead,
}

impl DeliveryState {
    pub fn is_final(self) -> bool {
        !matches!(self, DeliveryState::Pending | DeliveryState::Held)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryState::Pending => "pending",
            DeliveryState::Held => "held",
            DeliveryState::Resuming => "resuming",
            DeliveryState::LiveWaiter => "live_waiter",
            DeliveryState::Resumed => "resumed",
            DeliveryState::PickedUp => "picked_up",
            DeliveryState::Fresh => "fresh",
            DeliveryState::NoContinuation => "no_continuation",
            DeliveryState::Dead => "dead",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => DeliveryState::Pending,
            "held" => DeliveryState::Held,
            "resuming" => DeliveryState::Resuming,
            "live_waiter" => DeliveryState::LiveWaiter,
            "resumed" => DeliveryState::Resumed,
            "picked_up" => DeliveryState::PickedUp,
            "fresh" => DeliveryState::Fresh,
            "no_continuation" => DeliveryState::NoContinuation,
            "dead" => DeliveryState::Dead,
            _ => return None,
        })
    }
}

/// 交付紀錄（存在 `Answer.delivery`，UI 回執與 ledger 讀這份）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    pub state: DeliveryState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 打開原對話的 deep link（Claude App／Codex App）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_link: Option<String>,
    pub updated_at: crate::time::Ts,
}

/// agent 以 `ask_report` 回報的處理結果狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Done,
    Failed,
    Blocked,
}

/// agent 處理完答案後回傳的結果與給使用者的回饋（AI 回饋）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentReport {
    pub status: ReportStatus,
    pub summary_md: String,
    pub reported_at: crate::time::Ts,
}

/// 這題要接續的原對話 id：優先 `source.conversation_id`，相容舊的
/// `wake_on_answer {mode: resume, session_id}` 寫法。
pub fn conversation_id(source: &Source, wake: &Wake) -> Option<String> {
    let non_blank = |s: &Option<String>| s.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    non_blank(&source.conversation_id).or_else(|| {
        (wake.mode == WakeMode::Resume).then(|| non_blank(&wake.session_id)).flatten()
    })
}

/// 是否走「回答延續原對話」：有原對話身分即走（internal doctor 題除外）。
pub fn continuation_applies(source: &Source, wake: &Wake, internal: bool) -> bool {
    !internal && conversation_id(source, wake).is_some()
}

/// 打開原對話的 deep link：Claude 桌面用 `claude://code/continue?session=local_…`，
/// Codex（App 與 CLI 共用 thread store）用 `codex://threads/<id>`；其他回 None。
pub fn conversation_link(source: &Source) -> Option<String> {
    let safe = |s: &str| !s.is_empty() && s.len() <= 80 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    match source.host {
        Some(HostKind::ClaudeDesktop) => source
            .host_session_id
            .as_deref()
            .filter(|h| h.strip_prefix("local_").is_some_and(|rest| safe(rest) && !rest.contains('_')))
            .map(|h| format!("claude://code/continue?session={h}")),
        Some(HostKind::CodexApp) | Some(HostKind::CodexCli) => source
            .conversation_id
            .as_deref()
            .filter(|t| safe(t))
            .map(|t| format!("codex://threads/{t}")),
        _ => None,
    }
}

/// reactor 決策時的觀察值（全部由 server 層探測）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LadderInput {
    pub continuation: bool,
    pub wake_mode: WakeMode,
    pub current: DeliveryState,
    /// 有 ask_wait 正在這題上 long-poll。
    pub waiter_connected: bool,
    /// 最近 10 秒內有等待者（兩輪 long-poll 之間的空檔仍算在等）。
    pub waiter_lease_active: bool,
    /// 原對話仍被活行程持有（Claude 登記檔命中且 pid 存活）。
    pub conversation_held: bool,
}

/// 下一步。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LadderStep {
    /// 已有最終交付，什麼都不做。
    Done,
    /// 等待者在線或租約內：不動，稍後重看。
    AwaitWaiter,
    /// 原對話仍被開著：記 held，稍後重檢。
    Hold,
    /// 在原對話追加一輪。
    Resume,
    /// 開新對話（現行 fresh 模式）。
    Fresh { reason: String },
    /// 不需要喚醒任何 agent。
    NoWake,
}

pub fn next_step(input: &LadderInput) -> LadderStep {
    if input.current.is_final() {
        return LadderStep::Done;
    }
    if !input.continuation {
        return match input.wake_mode {
            WakeMode::None => LadderStep::NoWake,
            _ => LadderStep::Fresh { reason: "沒有原對話可接續".into() },
        };
    }
    // 等待者本身就在原對話裡：它在線或租約內 → 交給它，絕不因對話活著而 held。
    if input.waiter_connected || input.waiter_lease_active {
        return LadderStep::AwaitWaiter;
    }
    if input.conversation_held {
        return LadderStep::Hold;
    }
    LadderStep::Resume
}

/// resume 失敗輸出分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeFailure {
    /// Codex App 正在寫這個 thread（`already has an active writer`）→ held。只屬於 Codex。
    Locked,
    /// 開始處理前就找不到對話（Claude `No conversation found`、Codex `no rollout found`）→ 開新對話。
    NotFound,
    /// agent 已經開始處理（Claude assistant 事件、Codex item 事件）→ 不重試、不開新對話。
    Started,
    /// 其他錯誤：重試，耗盡後開新對話。
    Transient,
}

/// agent 是否已開始處理這一輪：Claude stream-json 出現 `assistant` 事件；Codex `--json` 出現 `item.*` 事件。
/// 只有 init／thread.started 不算——模型還沒讀到答案。
pub fn agent_started(codex: bool, stdout: &str) -> bool {
    json_events(stdout).any(|ev| {
        let ty = ev.get("type").and_then(serde_json::Value::as_str).unwrap_or("");
        if codex {
            return ty.starts_with("item.");
        }
        // Claude 的 API 錯誤（過載、限流、驗證失敗…）以帶 `error`、model 為 `<synthetic>` 的 assistant 事件回報：不算開始處理。
        let synthetic = ev.pointer("/message/model").and_then(serde_json::Value::as_str) == Some("<synthetic>");
        ty == "assistant" && ev.get("error").is_none() && !synthetic
    })
}

fn json_events(stdout: &str) -> impl Iterator<Item = serde_json::Value> + '_ {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
}

/// 結尾的錯誤事件訊息（Codex `error`／`turn.failed`、Claude `result` is_error）。不含 agent 的工作內容。
fn error_event_text(stdout: &str) -> String {
    let mut out = String::new();
    for ev in json_events(stdout) {
        let ty = ev.get("type").and_then(serde_json::Value::as_str).unwrap_or("");
        let text = match ty {
            "error" => ev.get("message").cloned(),
            "turn.failed" => ev.pointer("/error/message").cloned(),
            "result" if ev.get("is_error").and_then(serde_json::Value::as_bool) == Some(true) => {
                ev.get("result").or_else(|| ev.get("error")).or_else(|| ev.get("errors")).cloned()
            }
            _ => None,
        };
        if let Some(t) = text {
            out.push_str(&t.to_string());
            out.push('\n');
        }
    }
    out
}

/// 分類一輪失敗的追加：已開始處理 → Started；否則只看 stderr 與錯誤事件（QA 第二輪 C）。
pub fn classify_resume_failure(codex: bool, stdout: &str, stderr: &str) -> ResumeFailure {
    if agent_started(codex, stdout) {
        return ResumeFailure::Started;
    }
    let text = format!("{stderr}\n{}", error_event_text(stdout)).to_ascii_lowercase();
    if codex && text.contains("already has an active writer") {
        ResumeFailure::Locked
    } else if text.contains("no conversation found") || text.contains("no rollout found") {
        ResumeFailure::NotFound
    } else {
        ResumeFailure::Transient
    }
}

/// `kunsu ask` 依環境變數補齊原對話身分（只補 agent 沒填的欄位；不碰任何 token）。
pub fn host_from_env(bundle_id: Option<&str>, entrypoint: Option<&str>, has_codex_thread: bool) -> Option<HostKind> {
    if has_codex_thread {
        return Some(if bundle_id == Some("com.openai.codex") { HostKind::CodexApp } else { HostKind::CodexCli });
    }
    match entrypoint {
        Some("claude-desktop") => Some(HostKind::ClaudeDesktop),
        Some(e) if !e.trim().is_empty() => Some(HostKind::ClaudeCli),
        _ if bundle_id == Some("com.anthropic.claudefordesktop") => Some(HostKind::ClaudeDesktop),
        _ => None,
    }
}

/// 便利：題目來源是不是 Codex 系主機（決定 resume 命令型態）。
pub fn is_codex(source: &Source) -> bool {
    matches!(source.host, Some(HostKind::CodexApp) | Some(HostKind::CodexCli))
        || source.agent == AgentKind::Codex
}

/// UI「打開原對話」放行檢查：只接受 [`conversation_link`] 會產生的兩種形狀。
pub fn is_conversation_link(link: &str) -> bool {
    let safe = |s: &str, allow_underscore: bool| {
        !s.is_empty()
            && s.len() <= 80
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || (allow_underscore && c == '_'))
    };
    if let Some(rest) = link.strip_prefix("claude://code/continue?session=local_") {
        return safe(rest, false);
    }
    if let Some(rest) = link.strip_prefix("codex://threads/") {
        return safe(rest, true);
    }
    false
}
