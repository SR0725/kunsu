//! CONTRACT §2 資料模型。serde 全部 snake_case；enum 走白名單解析
//! （serde 對未知 variant 直接 error，即為白名單）。

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

pub use crate::time::Ts;

/// 題型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Choice,
    Text,
    Confirm,
    KongkoReview,
    /// 題組（PRD §25）：一題放 1–20 個同類項目，逐項採用／不處理／請 AI 改。
    Batch,
}

/// 分級（app 守門後生效值 vs agent 原始請求值皆用此型別）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    T0,
    T1,
    T2,
    T3,
}

/// 燈號（沿用 meta-ads 語義）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Yellow,
    Red,
}

/// 狀態機狀態（§3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Created,
    Pending,
    Warned,
    Forced,
    Snoozed,
    Answered,
    Expired,
    Cancelled,
    Superseded,
    /// 終態投遞完成後標記。
    Routed,
}

impl State {
    /// answered / expired / cancelled / superseded / routed 都不再接受一般事件。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            State::Answered | State::Expired | State::Cancelled | State::Superseded | State::Routed
        )
    }
}

/// 作答通道。MCP 面永不出現（inv_no_answer_via_mcp 由 server 層保證）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Ui,
    Kongko,
    Discord,
    Cli,
    AutoPolicy,
    /// 已配對手機（PRD §30.8）：封包經裝置簽章驗證（INV-28）。
    Mobile,
}

impl Channel {
    /// 使用者本人在自己裝置上點擊的通道：電腦介面或已配對手機。只有這兩條能批准動錢（INV-15）。
    pub fn is_ray_device(self) -> bool {
        matches!(self, Channel::Ui | Channel::Mobile)
    }
}

/// 證據行；`source` 是身世標籤，必填（空字串在 validate 拒收）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceLine {
    pub text: String,
    pub source: String,
}

/// 選項。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Opt {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended: Option<bool>,
}

/// 錢類標記。`amount_twd` 以新台幣元計（非 cents）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Money {
    pub is_money_op: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_twd: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    /// 完整 prepare 命令字串；建題時不預先執行（§6 授權鏈第 1 步）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_prepare_cmd: Option<String>,
}

/// 超時預設動作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnExpire {
    NoopLog,
    EscalateDiscord,
    /// 錢類永不允許（inv_money_never_auto_expire；validate 拒收 + 狀態機拒跑雙重防線）。
    PickOption,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefaultAction {
    pub on_expire: OnExpire,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_id: Option<String>,
}

/// 作答後喚醒模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeMode {
    None,
    Resume,
    Fresh,
    /// 回答延續原對話（PRD §20）：有 `source.conversation_id` 時的預設。
    Continue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wake {
    pub mode: WakeMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playbook: Option<String>,
}

impl Default for Wake {
    fn default() -> Self {
        Wake {
            mode: WakeMode::None,
            session_id: None,
            playbook: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Claude,
    Codex,
    #[default]
    Other,
}

/// 建題來源。原對話身分與排程身分（PRD §20/§21）全部選填：舊 client 不帶也合法。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Source {
    pub agent: AgentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routine_id: Option<String>,
    /// 發問主機（claude_desktop｜claude_cli｜codex_app｜codex_cli｜other）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<crate::continuity::HostKind>,
    /// 原對話 id：Claude CLI session id（`CLAUDE_CODE_SESSION_ID`）或 Codex thread id（`CODEX_THREAD_ID`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// Claude 桌面 session（`local_…`）；CLI id 會換，作答當下以它重新查。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_session_id: Option<String>,
    /// 是否有人在場（`CLAUDE_CODE_SESSION_ATTENDED`）；排程 session 為 false。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attended: Option<bool>,
    /// 發問 session 使用的 CLI 執行檔（`CLAUDE_CODE_EXECPATH`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_path: Option<String>,
    /// 排程觸發或互動觸發（routine-check-in）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<crate::continuity::Trigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routine_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routine_kind: Option<crate::continuity::RoutineKind>,
    /// 排程宣告的報到頻率（小時）；督導頁用來判斷「排程可能沒在跑」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_every_hours: Option<f64>,
}

/// 答案。簽章只在 approval 題出現；acked/executed 由 server 回填。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    /// uuid v4，作答當下由 server 生成；drain/reactor 的冪等鍵（CONTRACT §2）。
    #[serde(default)]
    pub answer_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_params: Option<String>,
    pub channel: Channel,
    pub answered_at: Ts,
    /// 短效執行授權的重新授權時間（決策/授權分離）：15 分鐘 approval 過期後
    /// 使用者在 UI 重新授權時由 server 蓋上；approval record 的 answered_at
    /// 以此為準（無則用 answered_at）。答案本身永不因重新授權改變。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reauthorized_at: Option<Ts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acked_at: Option<Ts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_at: Option<Ts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_result: Option<String>,
    /// 答案交付到哪裡（回答延續原對話，PRD §20）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<crate::continuity::Delivery>,
    /// agent 以 `ask_report` 回報的處理結果與 AI 回饋；永不改變答案本身（INV-7）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<crate::continuity::AgentReport>,
    /// 作答當下顯示的草稿版本與內容雜湊（證據畫布，PRD §24）；JSON 建題沒有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// 題組逐項決定（PRD §25）；題組的 `free_text` 是使用者的整體指示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<ItemDecision>>,
}

/// 題組項目的預設決定（PRD §25：每項預設採用；AI 建議不處理的預設不處理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemDefault {
    #[default]
    Approve,
    Skip,
}

/// 題組項目（CONTRACT §14）。`content_hash` 由 server 計算（`batch::seal_items`），agent 帶的值一律覆寫。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchItem {
    /// 題內唯一；路徑安全白名單同題目 id。
    pub id: String,
    /// 一眼認出是哪一項（寄件人＋主旨、留言者）；純文字、≤80 字。
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// 原文脈絡（來信全文、留言串、申請內容）；Markdown。
    pub context_md: String,
    /// AI 的草稿或建議；可編輯項目 ≤5,000 字；純文字。
    pub proposal: String,
    /// true＝使用者可改草稿；false＝只核可（照建議或不處理）。
    pub editable: bool,
    #[serde(default)]
    pub default: ItemDefault,
    /// `default: skip` 必填：AI 為什麼建議不處理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

/// 題組逐項決定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemAction {
    /// 採用：可編輯項目帶最後要送出的文字（一字不改送出）；只核可項目照建議執行。
    Approve,
    /// 不處理。
    Skip,
    /// 請 AI 改：附一句怎麼改；改好以下一輪題組送回。
    Revise,
}

/// 題組答案的一項（CONTRACT §14）。存進答案時 `content_hash` 一定是作答當下該項的雜湊。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDecision {
    pub item_id: String,
    pub action: ItemAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

/// 完整題目（§2）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    /// 穩定 dedup id（沿用 pa-*；upsert 鍵）。
    pub id: String,
    /// ≤80 字。
    pub title: String,
    /// ≤24 字決策句（蓋板大字）。
    pub question_line: String,
    pub body_md: String,
    /// 「為什麼問你」脈絡（PRD §23）；舊題沒有時 UI 以 title＋body_md 首段代替。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why_md: Option<String>,
    /// 左側粒子視覺的受限文字輪廓（≤4 Unicode 字元）；空值依 source.agent。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_symbol: Option<String>,
    pub kind: Kind,
    /// app 守門後的生效值。
    pub tier: Tier,
    /// agent 原始請求值（分級準確率計分用）。
    pub requested_tier: Tier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<Gate>,
    /// ≤3 條。
    pub evidence: Vec<EvidenceLine>,
    /// 動錢題必填：精確變更一行（所見即所簽）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params_line: Option<String>,
    /// choice 題 2..=4。
    pub options: Vec<Opt>,
    /// 恆 true。
    pub allow_free_text: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub money: Option<Money>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prediction_ref: Option<String>,
    /// batchId；發佈事件自動結案鍵。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kongko_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
    /// P1 定案 1：不動錢但不可逆時標 true（T0 准入判準的一部分）。
    #[serde(default)]
    pub irreversible: bool,
    /// P1 定案 1/3：agent 建題時比對 vault policies 命中才填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing_policy_ref: Option<String>,
    /// P1 定案 1：server 守門降級時回寫的可讀原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_downgrade_reason: Option<String>,
    /// P1 定案 3：auto_policy 執行後的 24h 否決窗（可逆操作在窗內可一鍵 revert）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub veto_until: Option<Ts>,
    /// P1 定案 10：doctor 測試題標記——不佔 T0 預算、不推 Discord、reactor 只
    /// dry-run、ledger 行標 internal 供統計排除。
    #[serde(default)]
    pub internal: bool,
    pub default_action: DefaultAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<Ts>,
    pub wake_on_answer: Wake,
    pub source: Source,
    pub state: State,
    pub snooze_count: u8,
    pub forced_today_used: bool,
    /// upsert 不動（inv_upsert_no_reset）。台北本地日期。
    pub first_proposed: NaiveDate,
    pub last_seen: Ts,
    pub created_at: Ts,
    /// 本輪 15 分鐘強制詢問週期的牆鐘起點。internal 題為 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_started_at: Option<Ts>,
    /// 本輪進入三分鐘警告的牆鐘時刻。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn_at: Option<Ts>,
    /// 本輪必須顯示強制詢問介面的牆鐘時刻。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_at: Option<Ts>,
    /// 1-based 週期編號；每次未回答 dismiss 後遞增，無上限。
    #[serde(default)]
    pub cycle_number: u32,
    pub next_transition_at: Ts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Answer>,
    /// 以草稿註冊的題目（PRD §24）：目前版本、內容雜湊與畫布種類。JSON 建題為 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<DraftRef>,
    /// 題組項目（kind=batch，PRD §25）；其他題型為空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<BatchItem>,
    /// 使用者按「這題不該問我」推掉這題（督導頁標「退回」，PRD §21）；跟著終態快照進帳本，重建後仍在。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected: Option<Rejection>,
    /// 建題當下的「一般題到期時間」（`policy.expiry.t1_default_after_h` 快照，[`Question::pin_expiry`]）。之後設定頁改到期時間
    /// 只影響新題（INV-26）。None＝舊題，沿用目前 policy。upsert 與草稿重新註冊保留原值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry_after_h: Option<u32>,
    /// 建題當下的「題目最長等待天數」（`policy.expiry.t2_supersede_after_d` 快照，≤14；INV-14／INV-26）。None＝舊題，沿用目前 policy。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pending_d: Option<u32>,
}

/// INV-14：任何題的壽命上限（天）。
pub const MAX_PENDING_DAYS_CAP: u32 = 14;

/// 使用者推掉一題的時間與原因（原因選填）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    pub at: Ts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 草稿註冊的版本參照；內容快照在 `~/.kunsu/revisions/<id>/<revision>/`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftRef {
    pub revision: u32,
    /// `sha256:<hex>`，涵蓋 question.md、畫布與引用到的資產。
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<CanvasKind>,
    pub registered_at: Ts,
}

/// 證據畫布種類（Claude Artifact 模式）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CanvasKind {
    Tsx,
    Jsx,
    Html,
}

impl CanvasKind {
    pub fn file_name(self) -> &'static str {
        match self {
            CanvasKind::Tsx => "canvas.tsx",
            CanvasKind::Jsx => "canvas.jsx",
            CanvasKind::Html => "canvas.html",
        }
    }
}

/// 確認題（`kind = confirm` 且沒有選項）的作答值：決策面板「確認」送出的 `choice_id`（AC-DS-15）。
pub const CONFIRM_CHOICE_ID: &str = "confirm";

impl Question {
    /// 是否為動錢題。
    pub fn is_money_op(&self) -> bool {
        self.money.as_ref().is_some_and(|m| m.is_money_op)
    }

    /// 確認題：`kind = confirm` 且沒有選項，面板只給「確認」一個動作。
    pub fn is_confirm_only(&self) -> bool {
        self.kind == Kind::Confirm && self.options.is_empty()
    }

    /// `choice_id` 是不是本題合法的選擇：options 裡的 id；確認題另接受 [`CONFIRM_CHOICE_ID`]。
    pub fn accepts_choice(&self, choice_id: &str) -> bool {
        self.options.iter().any(|o| o.id == choice_id)
            || (self.is_confirm_only() && choice_id == CONFIRM_CHOICE_ID)
    }

    /// 給人或 agent 看的選擇文字：選項標籤；確認題的 `confirm` 是「確認」。
    pub fn choice_label(&self, choice_id: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|o| o.id == choice_id)
            .map(|o| o.label.as_str())
            .or_else(|| (self.is_confirm_only() && choice_id == CONFIRM_CHOICE_ID).then_some("確認"))
    }

    /// 把建題當下的到期參數固定在題目上（INV-26）：之後設定頁改到期時間或最長等待天數只影響新題。建題時呼叫一次；
    /// upsert／草稿重新註冊沿用既有題目的值，不再呼叫。
    pub fn pin_expiry(&mut self, policy: &crate::policy::Policy) {
        self.expiry_after_h = Some(policy.expiry.t1_default_after_h);
        self.max_pending_d = Some(policy.expiry.t2_supersede_after_d.min(MAX_PENDING_DAYS_CAP));
    }

    /// 這題的一般題到期時間（小時）：建題時固定的值；舊題沒有快照時用目前 policy。
    pub fn expiry_hours(&self, policy: &crate::policy::Policy) -> u32 {
        self.expiry_after_h.unwrap_or(policy.expiry.t1_default_after_h)
    }

    /// 這題的最長等待天數：建題時固定的值；舊題沒有快照時用目前 policy。永遠不超過 14 天（INV-14）。
    pub fn max_pending_days(&self, policy: &crate::policy::Policy) -> u32 {
        self.max_pending_d.unwrap_or(policy.expiry.t2_supersede_after_d).min(MAX_PENDING_DAYS_CAP)
    }

    /// 建立或重新開始一個持久化牆鐘週期。internal doctor 題永不強制。
    pub fn start_force_cycle(
        &mut self,
        started_at: Ts,
        policy: &crate::policy::Policy,
        increment: bool,
    ) {
        if self.internal {
            self.cycle_started_at = None;
            self.warn_at = None;
            self.force_at = None;
            self.cycle_number = 0;
            return;
        }
        let force_at = started_at + chrono::Duration::minutes(policy.force.cycle_min as i64);
        let warn_at = force_at - chrono::Duration::minutes(policy.force.warn_before_min as i64);
        self.cycle_started_at = Some(started_at);
        self.warn_at = Some(warn_at);
        self.force_at = Some(force_at);
        self.cycle_number = if increment {
            self.cycle_number.saturating_add(1).max(1)
        } else {
            self.cycle_number.max(1)
        };
        self.next_transition_at = warn_at;
    }

    /// 舊資料缺少週期欄位時，以原始 created_at 回填，避免重啟後重置倒數。
    /// 回傳是否有變動，供 server 決定是否持久化。
    pub fn ensure_force_cycle(&mut self, policy: &crate::policy::Policy) -> bool {
        if self.internal {
            let changed = self.cycle_started_at.is_some()
                || self.warn_at.is_some()
                || self.force_at.is_some()
                || self.cycle_number != 0;
            if changed {
                self.start_force_cycle(self.created_at, policy, false);
            }
            return changed;
        }
        if self.cycle_started_at.is_some() && self.warn_at.is_some() && self.force_at.is_some() {
            return false;
        }
        let started_at = self.cycle_started_at.unwrap_or(self.created_at);
        self.start_force_cycle(started_at, policy, false);
        true
    }

    /// 從通過 validate 的建題請求生出全新題目（state=created，倒數起點=now）。
    pub fn from_create(req: CreateRequest, now: Ts) -> Question {
        Question {
            id: req.id,
            title: req.title,
            question_line: req.question_line,
            body_md: req.body_md,
            why_md: req.why_md,
            visual_symbol: req.visual_symbol,
            kind: req.kind,
            tier: req.tier,
            requested_tier: req.tier,
            product: req.product,
            gate: req.gate,
            evidence: req.evidence,
            params_line: req.params_line,
            options: req.options,
            allow_free_text: req.allow_free_text,
            money: req.money,
            decision_ref: req.decision_ref,
            prediction_ref: req.prediction_ref,
            kongko_ref: req.kongko_ref,
            deep_link: req.deep_link,
            irreversible: req.irreversible,
            standing_policy_ref: req.standing_policy_ref,
            tier_downgrade_reason: None,
            veto_until: None,
            internal: req.internal,
            default_action: req.default_action,
            deadline_at: req.deadline_at,
            wake_on_answer: req.wake_on_answer,
            source: req.source,
            state: State::Created,
            snooze_count: 0,
            forced_today_used: false,
            // P1 定案 8：尊重 client 提供的 first_proposed（積壓天數誠實）；未提供才填今日。
            first_proposed: req
                .first_proposed
                .unwrap_or_else(|| crate::time::taipei_date(now)),
            last_seen: now,
            created_at: now,
            cycle_started_at: None,
            warn_at: None,
            force_at: None,
            cycle_number: 0,
            next_transition_at: now,
            answer: None,
            draft: None,
            items: crate::batch::seal_items(req.items),
            rejected: None,
            expiry_after_h: None,
            max_pending_d: None,
        }
    }
}

fn default_true() -> bool {
    true
}

/// `ask_create` 的請求體：Question 減去 server 管的欄位。
/// `deny_unknown_fields`：agent 打錯欄位名（如 `predicton_ref`）要 400 大聲失敗，
/// 不准無聲吞掉。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub id: String,
    pub title: String,
    pub question_line: String,
    #[serde(default)]
    pub body_md: String,
    /// 「為什麼問你」脈絡（Markdown；以時間開頭的清單呈現為時間軸，PRD §23）。
    #[serde(default)]
    pub why_md: Option<String>,
    /// 左側粒子視覺的受限文字輪廓（≤4 Unicode 字元）；不是 URL 或任意 UI。
    #[serde(default)]
    pub visual_symbol: Option<String>,
    pub kind: Kind,
    /// agent 的初始分級請求；server 守門後寫入 Question.tier / requested_tier。
    pub tier: Tier,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub gate: Option<Gate>,
    #[serde(default)]
    pub evidence: Vec<EvidenceLine>,
    #[serde(default)]
    pub params_line: Option<String>,
    #[serde(default)]
    pub options: Vec<Opt>,
    #[serde(default = "default_true")]
    pub allow_free_text: bool,
    #[serde(default)]
    pub money: Option<Money>,
    #[serde(default)]
    pub decision_ref: Option<String>,
    #[serde(default)]
    pub prediction_ref: Option<String>,
    #[serde(default)]
    pub kongko_ref: Option<String>,
    #[serde(default)]
    pub deep_link: Option<String>,
    /// P1 定案 1：不動錢但不可逆時標 true。
    #[serde(default)]
    pub irreversible: bool,
    /// P1 定案 1/3：agent 端比對 vault policies 命中才填。
    #[serde(default)]
    pub standing_policy_ref: Option<String>,
    /// P1 定案 10：doctor 測試題標記。
    #[serde(default)]
    pub internal: bool,
    pub default_action: DefaultAction,
    #[serde(default)]
    pub deadline_at: Option<Ts>,
    #[serde(default)]
    pub wake_on_answer: Wake,
    pub source: Source,
    /// P1 定案 8：client 可自帶 first_proposed（積壓天數誠實原則）；未提供 server 填今日。
    #[serde(default)]
    pub first_proposed: Option<NaiveDate>,
    /// 題組項目（kind=batch，2..=20）。
    #[serde(default)]
    pub items: Vec<BatchItem>,
}
