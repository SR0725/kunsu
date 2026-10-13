//! Reactor（CONTRACT §8）：串行 tokio task，一次一個喚醒。
//!
//! - answered 且 `wake_on_answer.mode=fresh` → spawn
//!   `claude -p --input-format text --output-format stream-json --verbose
//!   --permission-mode bypassPermissions --max-budget-usd 2`（prompt 走 stdin，避開 ARG_MAX；cwd=vault 根；
//!   不經 zsh alias、旗標顯式帶；`claude` 路徑 which 解析並快取）。
//! - `source.agent=codex` → `codex exec - --json -s workspace-write -C <vault>`
//!   （prompt 走 stdin）。
//! - 回答延續原對話（PRD §20）：交付階梯——等待者 → 原對話已結束就在原對話追加（resume）→
//!   原對話仍被活行程持有就 held → 都不行才開新對話；spawn 前先佔 `resuming`。
//! - 錢黃燈題（choice 執行型選項 + gate≠red）先走批准簽章鏈
//!   （approval::run_approval_chain）→ commit 丟進同一條串行 lane 執行。
//! - **dry_run 模式**（config 旗標，測試與 doctor 用）：不真 spawn commit/wake，
//!   只記錄命令行；internal 題永遠全乾跑（P1 定案 10）。
//! - agent 還沒開始處理的失敗才重試（3 次指數退避 → 死信 + Discord）；已開始處理或逾時不重來；產出回寫
//!   `service.record_execution`；`EXEC_OK:`/`EXEC_FAILED:` 前綴解析。

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use chrono::Utc;
use kunsu_core::continuity::{self, DeliveryState, LadderStep, ResumeFailure};
use kunsu_core::types::{AgentKind, Answer, Gate, ItemAction, ItemDecision, Question};
use serde::Serialize;
use tokio::io::AsyncWriteExt;

use crate::approval::{run_approval_chain, KeyProvider};
use crate::discord::DiscordMessage;
use crate::events::{ExecStatus, ExecStep, ServerEvent};
use crate::service::{ServerHandle, HELD_RECHECK_SECONDS};

/// 所有 agent 子行程都清掉繼承的主機身分，避免被當成另一段對話或巢狀 session（CONTRACT §8）。
pub(crate) const AGENT_SCRUB_ENV: [&str; 4] = ["CLAUDE_CODE_", "CLAUDE_PID", "CLAUDECODE", "CODEX_THREAD_ID"];

fn agent_scrub_env() -> Vec<String> {
    AGENT_SCRUB_ENV.iter().map(|p| p.to_string()).collect()
}

/// 一次子行程失敗：人話細節、完整 stdout／stderr、是否逾時。
#[derive(Debug, Clone)]
struct ExecFailure {
    detail: String,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

impl ExecFailure {
    fn simple(detail: String) -> Self {
        ExecFailure { detail, stdout: String::new(), stderr: String::new(), timed_out: false }
    }
}
use crate::store;

/// reactor 組態。路徑與二進位全部可注入（測試假 script、doctor dry-run）。
pub struct ReactorConfig {
    pub vault_root: PathBuf,
    /// true = 不真 spawn commit/wake，只記錄命令行（測試與 doctor 用）。
    pub dry_run: bool,
    /// 測試覆寫；None → `which claude` 解析並快取。
    pub claude_bin: Option<PathBuf>,
    pub codex_bin: Option<PathBuf>,
    /// Claude 活 session 登記檔（prod：`~/.claude/sessions/<pid>.json`）。
    pub sessions_dir: PathBuf,
    /// Claude 桌面 session 紀錄根目錄（prod：`~/Library/Application Support/Claude/claude-code-sessions`），
    /// 以 host_session_id 查目前的 CLI session id（PRD §20、AC-CC-08）。
    pub desktop_sessions_dir: PathBuf,
    /// 發問 session 自帶的 CLI 路徑（`source.cli_path`）只有落在這些目錄下才採用。
    pub trusted_cli_roots: Vec<PathBuf>,
    pub retry_base_ms: u64,
    pub max_attempts: u32,
    /// agent 子行程（追加／開新對話）的執行上限；逾時中止，串行處理不被卡死。
    pub exec_timeout: std::time::Duration,
}

impl ReactorConfig {
    pub fn new(vault_root: PathBuf) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        ReactorConfig {
            vault_root,
            dry_run: false,
            claude_bin: None,
            codex_bin: None,
            sessions_dir: PathBuf::from(&home).join(".claude").join("sessions"),
            desktop_sessions_dir: PathBuf::from(&home)
                .join("Library/Application Support/Claude/claude-code-sessions"),
            trusted_cli_roots: vec![PathBuf::from(&home).join("Library/Application Support/Claude")],
            retry_base_ms: 1000,
            max_attempts: 3,
            exec_timeout: std::time::Duration::from_secs(30 * 60),
        }
    }
}

/// 一條被組裝（可能已執行）的命令——dry-run 驗證與 doctor 檢視面。
#[derive(Debug, Clone, Serialize)]
pub struct PlannedCommand {
    pub question_id: String,
    /// prepare | commit | wake_claude | wake_codex
    pub kind: String,
    pub program: String,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    pub cwd: String,
    /// spawn 前要清掉的繼承環境變數前綴（resume 時清 `CLAUDE_CODE_*`，避免被當成發問 session 的子行程）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scrub_env_prefixes: Vec<String>,
    /// false = dry-run 只記錄未執行。
    pub executed: bool,
}

pub struct Reactor {
    handle: ServerHandle,
    cfg: ReactorConfig,
    key: Arc<dyn KeyProvider>,
    planned: StdMutex<Vec<PlannedCommand>>,
    claude_path: OnceLock<PathBuf>,
    /// 死信告警聚合窗（2026-08-01 骷髏牆事故：13 筆同根因死信＝13 則 Discord 轟炸）。
    /// (窗開始時間, 窗內死信數)：前 3 筆照發、第 4 筆發合併總結、之後同窗靜默只記帳本。
    dead_letter_window: StdMutex<(Option<chrono::DateTime<Utc>>, u32)>,
}


impl Reactor {
    pub fn new(handle: ServerHandle, cfg: ReactorConfig, key: Arc<dyn KeyProvider>) -> Arc<Self> {
        Arc::new(Reactor {
            handle,
            cfg,
            key,
            planned: StdMutex::new(Vec::new()),
            claude_path: OnceLock::new(),
            dead_letter_window: StdMutex::new((None, 0)),
        })
    }

    /// dry-run / doctor 檢視：目前為止組裝過的所有命令。
    pub fn planned(&self) -> Vec<PlannedCommand> {
        self.planned.lock().expect("planned lock").clone()
    }

    /// 背景 task：事件驅動（ReactorEnqueued）+ 每 2s 掃 DB 補漏（crash 恢復）。
    /// 單一 task 內 select，天然串行——一次只跑一個喚醒。
    pub fn spawn(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            let mut rx = this.handle.subscribe();
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = this.process_pending().await {
                            tracing::error!("reactor 掃描失敗（下一輪重試）：{e}");
                        }
                    }
                    ev = rx.recv() => {
                        if matches!(ev, Ok(ServerEvent::ReactorEnqueued { .. }))
                            && let Err(e) = this.process_pending().await
                        {
                            tracing::error!("reactor 消化失敗（下一輪重試）：{e}");
                        }
                    }
                }
            }
        })
    }

    /// 掃 DB 待處理列並逐一串行消化。回傳處理筆數（測試直接呼叫）。
    pub async fn process_pending(&self) -> Result<u32, crate::error::ServerError> {
        // 崩潰恢復（Codex 覆檢 P1 冪等 claim）：掃描開始時仍掛在 executing 的
        // 列＝上一個進程在外部副作用（commit/wake spawn）中間死掉。外部狀態
        // 未知 → **絕不盲目重發**，標 needs_reconcile + Discord 等人工對帳。
        for stale in store::executing_reactor_runs(self.handle.pool()).await? {
            let now = Utc::now();
            let question = self.handle.get_question(&stale.question_id).await.ok().flatten();
            let resuming = self.handle.delivery_state(&stale.answer_id).await.ok().flatten() == Some(DeliveryState::Resuming);
            // 錢題依每一次授權的狀態分三種（CONTRACT §7「重啟恢復」）。
            let mut money_unknown = false;
            let mut money_done = false;
            if let Some(q) = question.as_ref().filter(|q| q.is_money_op()) {
                let unknown = store::unknown_money_nonces(self.handle.pool(), &q.id).await.unwrap_or_default();
                // 讀不到就當作已執行完成（不重跑、不說錢沒有動）。
                money_done = unknown.is_empty() && self.handle.money_executed(q).await.unwrap_or(true);
                // 第 2 種：executor 還沒驗證任何一次授權、錢也沒有執行完成 → 錢沒有動，被擋、可重新授權。殘留的 commit 子行程
                // 剛好在這時驗證（settle 回 false）就改走第 1 種。
                money_unknown = !unknown.is_empty() || (!money_done && !self.recover_before_verify(q, &stale, now).await);
                if !money_unknown && !money_done {
                    continue;
                }
                if money_unknown {
                    // 第 1 種：executor 已用掉、卻沒有結果的授權記成 EXEC_UNKNOWN，執行紀錄也寫「結果不明」——agent、等待者、追加訊息與
                    // 回執一致讀到結果不明，之後交付改寫紀錄也不會讓它消失。已記成 EXEC_UNKNOWN 的保留原本的原因。
                    let text = "EXEC_UNKNOWN: Kunsu 在執行中途中斷，executor 已用掉授權但沒有回報結果——錢可能已經動過，請人工對帳";
                    for nonce in store::unknown_money_nonces(self.handle.pool(), &q.id).await.unwrap_or_default() {
                        if store::money_outcome_of(self.handle.pool(), &nonce).await.ok().flatten().is_none() {
                            let _ = self.handle.record_money_outcome(&q.id, &nonce, text, now).await;
                        }
                    }
                    let current = self.handle.get_question(&q.id).await.ok().flatten().unwrap_or_else(|| q.clone());
                    let already = current
                        .answer
                        .as_ref()
                        .and_then(|a| a.execution_result.as_deref())
                        .is_some_and(|r| r.trim_start().starts_with("EXEC_UNKNOWN"));
                    if !already && let Err(e) = self.handle.amend_execution(&q.id, text, now).await {
                        tracing::error!("結果不明回寫失敗（{}）：{e}", q.id);
                    }
                }
            }
            // 不動錢（或錢已經執行完成，第 3 種）、只是在原對話追加途中中斷：交給交付回收（已回報 → resumed；否則 dead＋一則告警），
            // 不發「可能已打到 Meta API」的錢鏈對帳訊息。
            if resuming && !money_unknown {
                let _ = store::mark_reactor_needs_reconcile(
                    self.handle.pool(),
                    &stale.answer_id,
                    "NEEDS_RECONCILE: 原對話追加途中中斷",
                    now,
                )
                .await;
                self.recover_interrupted_delivery(&stale.question_id, &stale.answer_id, now).await?;
                continue;
            }
            let reason = "NEEDS_RECONCILE: 上次執行在外部副作用中間中斷（可能已送出 API）——不自動重發，請人工對帳外部狀態後再決定";
            let _ = store::mark_reactor_needs_reconcile(
                self.handle.pool(),
                &stale.answer_id,
                reason,
                now,
            )
            .await;
            let _ = store::insert_event(
                self.handle.pool(),
                Some(&stale.question_id),
                "reactor_needs_reconcile",
                &serde_json::json!({
                    "question_id": stale.question_id,
                    "answer_id": stale.answer_id,
                }),
                now,
            )
            .await;
            let title = self
                .handle
                .get_question(&stale.question_id)
                .await
                .ok()
                .flatten()
                .map(|q| q.title)
                .unwrap_or_else(|| stale.question_id.clone());
            // 只有錢的結果不明才請使用者在回執標記錢有沒有動；錢已執行完成或不動錢的題，回執沒有那兩個按鈕（CONTRACT §7）。
            let text = if money_unknown {
                format!(
                    "🧾 「{title}」上次執行中斷在外部副作用中間（可能已打到 Meta API）。已標 needs_reconcile，**不會自動重發**——請人工對帳後在 Kunsu 回執標記「錢已經動了／錢沒有動」；標「沒有動」之後才能重新授權。"
                )
            } else if money_done {
                format!(
                    "🧾 「{title}」的錢已經執行完成（不會重跑）；之後的交付在途中中斷，可能已送出。已標 needs_reconcile，**不會自動重發**——請打開原對話確認。"
                )
            } else {
                format!(
                    "🧾 「{title}」上次執行中斷在外部副作用中間（可能已送出）。已標 needs_reconcile，**不會自動重發**——請確認原對話或外部狀態後再決定。"
                )
            };
            self.handle.discord().send(DiscordMessage {
                question_id: Some(stale.question_id.clone()),
                reason: "reactor_needs_reconcile".to_string(),
                text,
            });
        }

        let runs = store::pending_reactor_runs(self.handle.pool()).await?;
        let mut n = 0;
        for run in runs {
            self.process_run(&run).await;
            n += 1;
        }
        // 到期的交付重檢（等待者租約過期、held 對話釋放、wake=none 定案）。
        self.process_due_deliveries(Utc::now()).await?;
        Ok(n)
    }

    /// 重啟恢復第 2 種（CONTRACT §7）：executor 還沒驗證任何一次授權、錢也沒有執行完成——錢沒有動。這題簽發但沒有結果的授權在
    /// 同一段寫入鎖內記成被擋（之後 verify 一律拒絕，殘留的 commit 子行程晚到也花不掉）、執行紀錄寫同樣內容、刪掉 approval 檔、列改
    /// dead、Discord 說可以在回執重新授權；回答延續原對話的題照被擋的規則通知原對話。剛好有授權在這時被驗證（settle 回 false）或讀寫
    /// 失敗 → 回 false，呼叫端改走結果不明（不說錢沒有動）。
    async fn recover_before_verify(&self, q: &Question, stale: &store::ReactorRunRow, now: chrono::DateTime<Utc>) -> bool {
        let pool = self.handle.pool();
        let text = crate::service::INTERRUPTED_BEFORE_VERIFY;
        let nonces = match store::issued_unsettled_nonces(pool, &q.id).await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("驗證前中斷的授權讀不到（{}）：{e}", q.id);
                return false;
            }
        };
        for nonce in &nonces {
            match self.handle.settle_unverified_nonce(&q.id, nonce, text, now).await {
                Ok(true) => {}
                Ok(false) => return false,
                Err(e) => {
                    tracing::error!("驗證前中斷的授權記不進去（{}）：{e}", q.id);
                    return false;
                }
            }
        }
        if let Err(e) = self.handle.amend_execution(&q.id, text, now).await {
            tracing::error!("驗證前中斷回寫失敗（{}）：{e}", q.id);
        }
        let _ = store::mark_reactor_dead(pool, &stale.answer_id, text, now).await;
        crate::approval::remove_approval_file(&crate::approval::approval_file_path(&self.cfg.vault_root, &q.id));
        let _ = store::insert_event(
            pool,
            Some(&q.id),
            "money_interrupted_before_verify",
            &serde_json::json!({"question_id": q.id, "answer_id": stale.answer_id, "nonces": nonces}),
            now,
        )
        .await;
        self.handle.discord().send(DiscordMessage {
            question_id: Some(q.id.clone()),
            reason: "money_interrupted_before_verify".to_string(),
            text: format!(
                "🧾 「{}」的錢鏈在 executor 驗證授權之前中斷（Kunsu 重新啟動），錢沒有動。要執行的話，可以在 Kunsu 回執重新授權。",
                q.title
            ),
        });
        // 原對話也要知道錢鏈被擋下（同被擋的規則，只給人話狀態）。
        if continuity::continuation_applies(&q.source, &q.wake_on_answer, q.internal) {
            let current = self.handle.get_question(&q.id).await.ok().flatten().unwrap_or_else(|| q.clone());
            let dry = q.internal || self.cfg.dry_run;
            if let DeliverOutcome::Part(part) = self.deliver(&current, dry, Some("EXEC_BLOCKED"), Utc::now()).await {
                let prev = current.answer.as_ref().and_then(|a| a.execution_result.clone());
                let _ = self.handle.amend_execution(&q.id, &combine_execution(prev.as_deref(), &part), Utc::now()).await;
            }
        }
        true
    }

    /// 追加途中中斷的交付：agent 已回報 → resumed；否則 dead，並只告警一次。
    async fn recover_interrupted_delivery(
        &self,
        question_id: &str,
        answer_id: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<(), crate::error::ServerError> {
        let reported = self
            .handle
            .get_question(question_id)
            .await?
            .and_then(|q| q.answer)
            .is_some_and(|a| a.answer_id == answer_id && a.report.is_some());
        if reported {
            self.handle
                .finish_delivery(question_id, answer_id, DeliveryState::Resumed, Some("原對話已回報（追加途中 app 中斷）".into()), None, now)
                .await?;
            return Ok(());
        }
        let reason = "交付中斷：上次在原對話追加途中 app 中斷，可能已送出——請打開原對話確認".to_string();
        if self
            .handle
            .finish_delivery(question_id, answer_id, DeliveryState::Dead, Some(reason.clone()), None, now)
            .await?
            && !store::has_event_for_question(self.handle.pool(), question_id, "delivery_interrupted").await?
            // 錢題中斷時已發過錢鏈對帳訊息，不再多發一則。
            && !store::has_event_for_question(self.handle.pool(), question_id, "reactor_needs_reconcile").await?
        {
            store::insert_event(
                self.handle.pool(),
                Some(question_id),
                "delivery_interrupted",
                &serde_json::json!({"question_id": question_id, "answer_id": answer_id}),
                now,
            )
            .await?;
            self.handle.discord().send(DiscordMessage {
                question_id: Some(question_id.to_string()),
                reason: "delivery_interrupted".to_string(),
                text: format!("🧾 題目 {question_id} 的答案{reason}"),
            });
        }
        Ok(())
    }

    /// 到期的交付重檢（等待者租約、held 對話）。測試注入 `now` 快轉時間。
    pub async fn process_due_deliveries(
        &self,
        now: chrono::DateTime<Utc>,
    ) -> Result<u32, crate::error::ServerError> {
        // 卡在「追加中」：app 在追加途中中斷。可能已送出 → 不重做。
        for row in store::stuck_deliveries(self.handle.pool(), now - chrono::Duration::minutes(15)).await? {
            self.recover_interrupted_delivery(&row.question_id, &row.answer_id, now).await?;
        }
        let mut n = 0;
        for row in store::due_deliveries(self.handle.pool(), now).await? {
            let Some(q) = self.handle.get_question(&row.question_id).await? else { continue };
            let Some(answer) = q.answer.as_ref() else { continue };
            if answer.answer_id != row.answer_id {
                continue;
            }
            // 錢鏈或首輪交付還沒跑完 → 不搶先（避免訊息早於錢鏈結果送出）。
            if store::reactor_run_open(self.handle.pool(), &row.answer_id).await? {
                continue;
            }
            let dry = q.internal || self.cfg.dry_run;
            let money = answer.execution_result.clone();
            // 錢的結果不明時（以 nonce 判斷），追加訊息一律說「結果不明」，不沿用舊的被擋紀錄（CONTRACT §7）。
            let status = if self.handle.money_outcome_unknown(&q).await.unwrap_or(false)
                && !money.as_deref().is_some_and(|m| m.trim_start().starts_with("EXEC_UNKNOWN"))
            {
                Some("EXEC_UNKNOWN".to_string())
            } else {
                money.clone()
            };
            match self.deliver(&q, dry, status.as_deref(), now).await {
                DeliverOutcome::Part(text) => {
                    let combined = combine_execution(money.as_deref(), &text);
                    if let Err(e) = self.handle.record_execution(&q.id, &combined, Utc::now()).await {
                        tracing::error!("交付回寫失敗（{}）：{e}", q.id);
                    }
                }
                DeliverOutcome::Pending(_) | DeliverOutcome::Skip | DeliverOutcome::Dead => {}
            }
            n += 1;
        }
        Ok(n)
    }

    // ------------------------------------------------------------------
    // 交付階梯（回答延續原對話，PRD §20；BDD conversation-continuity）
    // ------------------------------------------------------------------

    async fn deliver(
        &self,
        q: &Question,
        dry: bool,
        money_summary: Option<&str>,
        now: chrono::DateTime<Utc>,
    ) -> DeliverOutcome {
        let Some(answer) = q.answer.as_ref() else { return DeliverOutcome::Skip };
        let current = self
            .handle
            .delivery_state(&answer.answer_id)
            .await
            .ok()
            .flatten()
            .unwrap_or(DeliveryState::Pending);
        let continuation =
            continuity::continuation_applies(&q.source, &q.wake_on_answer, q.internal);
        let original = continuity::conversation_id(&q.source, &q.wake_on_answer);
        let codex = continuity::is_codex(&q.source);
        let conv = match (&original, continuation && !codex) {
            (Some(id), true) => Some(self.resolve_claude_conversation(&q.source, id)),
            (id, _) => id.clone(),
        };
        let held = continuation
            && !codex
            && self.claude_conversation_held(&q.source, original.iter().chain(conv.iter()));
        let (waiter_connected, waiter_lease_active) = self.handle.waiter_status(&q.id, now);
        let step = continuity::next_step(&continuity::LadderInput {
            continuation,
            wake_mode: q.wake_on_answer.mode,
            current,
            waiter_connected,
            waiter_lease_active,
            conversation_held: held,
        });
        match step {
            LadderStep::Done => DeliverOutcome::Part(match current {
                DeliveryState::LiveWaiter => "已交給等待中的原對話".to_string(),
                DeliveryState::PickedUp => "原對話已自行取走答案".to_string(),
                other => format!("交付已定案：{}", other.as_str()),
            }),
            LadderStep::AwaitWaiter => {
                let text = "等待中的原對話將取走答案".to_string();
                self.mark(q, answer, DeliveryState::Pending, None, Some(now + chrono::Duration::seconds(3)), now).await;
                DeliverOutcome::Pending(text)
            }
            LadderStep::Hold => {
                let text = "原對話正開著，等它空下來就接續".to_string();
                self.mark(q, answer, DeliveryState::Held, Some(text.clone()), Some(now + chrono::Duration::seconds(HELD_RECHECK_SECONDS)), now).await;
                DeliverOutcome::Pending(text)
            }
            LadderStep::NoWake => {
                self.mark(q, answer, DeliveryState::NoContinuation, None, None, now).await;
                DeliverOutcome::Skip
            }
            LadderStep::Resume => {
                if !codex && self.claude_conversation_archived(&q.source) {
                    return self.fresh(q, answer, "原對話已封存".to_string(), dry, now).await;
                }
                let conv = conv.unwrap_or_default();
                self.resume(q, answer, &conv, dry, money_summary, now).await
            }
            LadderStep::Fresh { reason } => self.fresh(q, answer, reason, dry, now).await,
        }
    }

    /// 交付狀態落盤（internal doctor 題不寫，保持乾跑檢視乾淨）。
    async fn mark(
        &self,
        q: &Question,
        answer: &Answer,
        state: DeliveryState,
        reason: Option<String>,
        next_check_at: Option<chrono::DateTime<Utc>>,
        now: chrono::DateTime<Utc>,
    ) {
        if q.internal {
            return;
        }
        if let Err(e) = self
            .handle
            .set_delivery(&q.id, &answer.answer_id, state, reason, next_check_at, now)
            .await
        {
            tracing::error!("交付狀態寫入失敗（{}）：{e}", q.id);
        }
    }

    /// 佔住交付權（spawn 前）：只有 pending／held 能轉成 resuming；失敗＝別的路徑已定案。
    async fn claim_in_flight(&self, q: &Question, answer: &Answer, now: chrono::DateTime<Utc>) -> bool {
        if q.internal {
            return true;
        }
        self.handle
            .set_delivery(&q.id, &answer.answer_id, DeliveryState::Resuming, None, None, now)
            .await
            .unwrap_or(false)
    }

    /// 結束交付中的狀態（internal doctor 題不寫）。
    async fn finish(
        &self,
        q: &Question,
        answer: &Answer,
        state: DeliveryState,
        reason: Option<String>,
        next_check_at: Option<chrono::DateTime<Utc>>,
    ) {
        if q.internal {
            return;
        }
        if let Err(e) = self
            .handle
            .finish_delivery(&q.id, &answer.answer_id, state, reason, next_check_at, Utc::now())
            .await
        {
            tracing::error!("交付狀態結束失敗（{}）：{e}", q.id);
        }
    }

    async fn already_final(&self, answer: &Answer) -> DeliverOutcome {
        let state = self.handle.delivery_state(&answer.answer_id).await.ok().flatten();
        DeliverOutcome::Part(match state {
            Some(DeliveryState::LiveWaiter) => "已交給等待中的原對話".to_string(),
            Some(DeliveryState::PickedUp) => "原對話已自行取走答案".to_string(),
            Some(other) => format!("交付已定案：{}", other.as_str()),
            None => "交付已定案".to_string(),
        })
    }

    /// 在原對話追加一輪：先佔交付權再 spawn。被鎖 → held；找不到 → 開新對話；其他錯誤重試後開新對話。
    async fn resume(
        &self,
        q: &Question,
        answer: &Answer,
        conv: &str,
        dry: bool,
        money_summary: Option<&str>,
        now: chrono::DateTime<Utc>,
    ) -> DeliverOutcome {
        let prompt = continuation_prompt(q, money_summary);
        let mut cmd = if continuity::is_codex(&q.source) {
            self.build_codex_resume(q, conv, prompt)
        } else {
            self.build_claude_resume(q, conv, prompt)
        };
        cmd.executed = !dry;
        self.planned.lock().expect("planned lock").push(cmd.clone());
        if dry {
            self.mark(q, answer, DeliveryState::Resumed, Some("DRY_RUN".into()), None, now).await;
            return DeliverOutcome::Part(format!("DRY_RUN: {} {}", cmd.program, cmd.args.join(" ")));
        }
        if !self.claim_in_flight(q, answer, now).await {
            return self.already_final(answer).await;
        }
        let codex = continuity::is_codex(&q.source);
        let mut attempts = 0u32;
        loop {
            match self.exec_once_detailed(&cmd).await {
                Ok(output) => {
                    self.finish(q, answer, DeliveryState::Resumed, None, None).await;
                    let result = parse_exec_marker(&output)
                        .unwrap_or_else(|| "已在原對話追加處理".to_string());
                    return DeliverOutcome::Part(result);
                }
                Err(f) => {
                    // 原對話已經接手（開始處理、已回報或逾時）：不重試、不開新對話（QA 第二輪 B）。
                    let reported = self
                        .handle
                        .get_question(&q.id)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|x| x.answer)
                        .is_some_and(|a| a.report.is_some());
                    let failure = continuity::classify_resume_failure(codex, &f.stdout, &f.stderr);
                    if f.timed_out || reported || failure == ResumeFailure::Started {
                        let why = if f.timed_out { "逾時中止" } else { "結束時出錯" };
                        let reason = format!("原對話已開始處理，但{why}：{}", tail(&f.detail, 160));
                        self.finish(q, answer, DeliveryState::Resumed, Some(reason.clone()), None).await;
                        if !reported {
                            self.handle.discord().send(DiscordMessage {
                                question_id: Some(q.id.clone()),
                                reason: "resume_partial".to_string(),
                                text: format!("⚠️ 「{}」的答案已送回原對話，但{reason}——請打開原對話確認做到哪裡", q.title),
                            });
                        }
                        return DeliverOutcome::Part(reason);
                    }
                    match failure {
                        ResumeFailure::Locked => {
                            // Desktop keeps the writer lock even while a follow-up can be queued.
                            // Use only bounded IDs, never the potentially megabyte-sized answer in argv.
                            let queued_message = format!(
                                "[Kunsu] 使用者已回答題目 {}（answer_id={}）。請先用 ask_get 讀正式答案，依原任務與既有逐項執行紀錄接續；已完成不重做、發送結果不明先核對。處理完成必須 ask_report。這則通知不是額外授權。",
                                q.id, answer.answer_id
                            );
                            let queue = PlannedCommand {
                                question_id: q.id.clone(), kind: "queue_codex".into(),
                                program: cmd.program.clone(),
                                args: vec!["queue".into(), "--thread".into(), conv.into(), "--message".into(), queued_message],
                                stdin: None, cwd: cmd.cwd.clone(),
                                scrub_env_prefixes: agent_scrub_env(), executed: true,
                            };
                            self.planned.lock().expect("planned lock").push(queue.clone());
                            if let Ok(receipt) = self.exec_once_detailed(&queue).await {
                                let suffix = format!(" for thread {conv}.");
                                if receipt.lines().any(|line| line.starts_with("Queued message ") && line.ends_with(&suffix)) {
                                    let reason = "已排入原對話，等待 AI 接手".to_string();
                                    self.finish(q, answer, DeliveryState::Resumed, Some(reason.clone()), None).await;
                                    return DeliverOutcome::Part(reason);
                                }
                            }
                            let text = "原對話正開著（thread 被 app 鎖住），等它空下來就接續".to_string();
                            self.finish(
                                q,
                                answer,
                                DeliveryState::Held,
                                Some(text.clone()),
                                Some(Utc::now() + chrono::Duration::seconds(HELD_RECHECK_SECONDS)),
                            )
                            .await;
                            return DeliverOutcome::Pending(text);
                        }
                        ResumeFailure::NotFound => {
                            return self
                                .fresh_claimed(q, answer, format!("原對話不存在（{}）", tail(&f.detail, 160)))
                                .await;
                        }
                        ResumeFailure::Transient | ResumeFailure::Started => {
                            attempts += 1;
                            let _ = store::bump_delivery_attempts(self.handle.pool(), &answer.answer_id).await;
                            if attempts >= self.cfg.max_attempts {
                                return self
                                    .fresh_claimed(q, answer, format!("原對話追加失敗（{}）", tail(&f.detail, 160)))
                                    .await;
                            }
                            let backoff = self.cfg.retry_base_ms * 2u64.pow(attempts - 1);
                            tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                        }
                    }
                }
            }
        }
    }

    /// 開新對話（現行 fresh 模式）：先佔交付權再 spawn。
    async fn fresh(
        &self,
        q: &Question,
        answer: &Answer,
        reason: String,
        dry: bool,
        now: chrono::DateTime<Utc>,
    ) -> DeliverOutcome {
        if dry {
            let cmd = self.fresh_command(q, &reason, false);
            self.mark(q, answer, DeliveryState::Fresh, Some(reason), None, now).await;
            return DeliverOutcome::Part(format!("DRY_RUN: {} {}", cmd.program, cmd.args.join(" ")));
        }
        if !self.claim_in_flight(q, answer, now).await {
            return self.already_final(answer).await;
        }
        self.fresh_claimed(q, answer, reason).await
    }

    fn fresh_command(&self, q: &Question, reason: &str, executed: bool) -> PlannedCommand {
        let mut cmd = self.build_wake_command(q);
        if continuity::continuation_applies(&q.source, &q.wake_on_answer, q.internal) {
            let link = continuity::conversation_link(&q.source)
                .unwrap_or_else(|| "（無法直接開啟）".to_string());
            let note = format!("\n\n## 原對話\n- 無法接續原因：{reason}\n- 原對話連結：{link}");
            // 開新對話（Claude 與 Codex）的 prompt 都走 stdin。
            if let Some(stdin) = cmd.stdin.as_mut() {
                stdin.push_str(&note);
            }
        }
        cmd.executed = executed;
        self.planned.lock().expect("planned lock").push(cmd.clone());
        cmd
    }

    /// 已佔住交付權後開新對話；成功 → fresh，重試耗盡 → dead＋死信。
    async fn fresh_claimed(&self, q: &Question, answer: &Answer, reason: String) -> DeliverOutcome {
        let cmd = self.fresh_command(q, &reason, true);
        match self.exec_with_retries(&answer.answer_id, &cmd).await {
            Ok(output) => {
                self.finish(q, answer, DeliveryState::Fresh, Some(reason), None).await;
                let result = parse_exec_marker(&output)
                    .unwrap_or_else(|| format!("EXEC_OK 未標記；輸出尾：{}", tail(&output, 200)));
                if result.starts_with("EXEC_FAILED:") {
                    // 喚醒 agent 自己報失敗：記錄 + Discord，絕不靜默（不重試——它已跑完）。
                    self.handle.discord().send(DiscordMessage {
                        question_id: Some(q.id.clone()),
                        reason: "wake_exec_failed".to_string(),
                        text: format!("⚠️ 「{}」喚醒執行回報失敗：{result}", q.title),
                    });
                }
                DeliverOutcome::Part(result)
            }
            Err(f) if f.timed_out || continuity::agent_started(cmd.kind.contains("codex"), &f.stdout) => {
                // 新對話已開始處理：不重跑（避免同一答案做兩次），記錄並告訴使用者。
                let why = if f.timed_out { "逾時中止" } else { "結束時出錯" };
                let note = format!("{reason}；新對話已開始處理，但{why}：{}", tail(&f.detail, 160));
                self.finish(q, answer, DeliveryState::Fresh, Some(note.clone()), None).await;
                self.handle.discord().send(DiscordMessage {
                    question_id: Some(q.id.clone()),
                    reason: "wake_exec_failed".to_string(),
                    text: format!("⚠️ 「{}」喚醒的新對話處理到一半{why}——請確認做到哪裡", q.title),
                });
                DeliverOutcome::Part(format!("EXEC_FAILED: 新對話已開始處理，但{why}：{}", tail(&f.detail, 160)))
            }
            Err(f) => {
                self.finish(q, answer, DeliveryState::Dead, Some(f.detail.clone()), None).await;
                self.dead_letter(q, &answer.answer_id, &f.detail).await;
                DeliverOutcome::Dead
            }
        }
    }

    /// 桌面 session 的 CLI id 會換（priorCliSessionIds）：以 host_session_id 讀桌面紀錄取目前 id（AC-CC-08）。
    fn resolve_claude_conversation(&self, source: &kunsu_core::types::Source, original: &str) -> String {
        self.desktop_session_record(source)
            .and_then(|v| v.get("cliSessionId").and_then(|c| c.as_str()).map(str::to_string))
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| original.to_string())
    }

    /// 桌面 session 紀錄（以 host_session_id 找 `<accounts>/<org>/<local_…>.json`）。
    fn desktop_session_record(&self, source: &kunsu_core::types::Source) -> Option<serde_json::Value> {
        let host = source.host_session_id.as_deref().filter(|h| is_local_session_id(h))?;
        let file = format!("{host}.json");
        let accounts = std::fs::read_dir(&self.cfg.desktop_sessions_dir).ok()?;
        for acct in accounts.flatten() {
            let Ok(orgs) = std::fs::read_dir(acct.path()) else { continue };
            for org in orgs.flatten() {
                let Ok(raw) = std::fs::read_to_string(org.path().join(&file)) else { continue };
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    return Some(v);
                }
            }
        }
        None
    }

    /// 原對話已在 Claude App 封存（桌面紀錄 `isArchived: true`）→ 不追加，改開新對話（AC-CC-10）。
    fn claude_conversation_archived(&self, source: &kunsu_core::types::Source) -> bool {
        self.desktop_session_record(source)
            .and_then(|v| v.get("isArchived").and_then(serde_json::Value::as_bool))
            .unwrap_or(false)
    }

    /// 原對話是否仍被活行程持有：`~/.claude/sessions/<pid>.json` 的 sessionId／hostSessionId 命中且 pid 存活。
    fn claude_conversation_held<'a>(
        &self,
        source: &kunsu_core::types::Source,
        ids: impl Iterator<Item = &'a String>,
    ) -> bool {
        let ids: Vec<&String> = ids.collect();
        let Ok(entries) = std::fs::read_dir(&self.cfg.sessions_dir) else { return false };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
            let sid = v.get("sessionId").and_then(|x| x.as_str());
            let host = v.get("hostSessionId").and_then(|x| x.as_str());
            let matches = sid.is_some_and(|sid| ids.iter().any(|id| id.as_str() == sid))
                || (host.is_some() && host == source.host_session_id.as_deref());
            let pid = v.get("pid").and_then(|x| x.as_u64());
            if matches && pid.is_some_and(pid_alive) {
                return true;
            }
        }
        false
    }

    /// 發問 session 的 CLI 只在受信任目錄下才採用（防止 agent 自帶路徑讓 Kunsu 執行任意程式）。
    fn trusted_cli(&self, path: Option<&str>) -> Option<PathBuf> {
        let candidate = PathBuf::from(path?);
        let canonical = candidate.canonicalize().ok()?;
        if !canonical.is_file() {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if canonical.metadata().ok()?.permissions().mode() & 0o111 == 0 {
                return None;
            }
        }
        self.cfg
            .trusted_cli_roots
            .iter()
            .filter_map(|root| root.canonicalize().ok())
            .any(|root| canonical.starts_with(root))
            .then_some(canonical)
    }

    fn continuation_cwd(&self, q: &Question) -> String {
        let cwd = PathBuf::from(&q.source.cwd);
        if cwd.is_dir() {
            q.source.cwd.clone()
        } else {
            self.cfg.vault_root.to_string_lossy().to_string()
        }
    }

    fn build_claude_resume(&self, q: &Question, conv: &str, prompt: String) -> PlannedCommand {
        let program = self
            .trusted_cli(q.source.cli_path.as_deref())
            .unwrap_or_else(|| self.resolve_claude());
        PlannedCommand {
            question_id: q.id.clone(),
            kind: "resume_claude".to_string(),
            program: program.to_string_lossy().to_string(),
            args: vec![
                "-p".into(),
                prompt,
                "--resume".into(),
                conv.to_string(),
                "--output-format".into(),
                "stream-json".into(),
                "--verbose".into(),
                "--permission-mode".into(),
                "bypassPermissions".into(),
                "--max-budget-usd".into(),
                "2".into(),
            ],
            stdin: None,
            cwd: self.continuation_cwd(q),
            scrub_env_prefixes: agent_scrub_env(),
            executed: false,
        }
    }

    fn codex_program(&self) -> PathBuf {
        // Match the current desktop host/model catalog; the global npm CLI may lag behind it.
        let bundled = if cfg!(target_os = "macos") {
            Some(PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex"))
        } else {
            None
        };
        resolve_codex_binary(self.cfg.codex_bin.clone(), bundled, which("codex"))
    }

    fn build_codex_resume(&self, q: &Question, thread: &str, prompt: String) -> PlannedCommand {
        let program = self.codex_program();
        let cwd = self.continuation_cwd(q);
        PlannedCommand {
            question_id: q.id.clone(),
            kind: "resume_codex".to_string(),
            program: program.to_string_lossy().to_string(),
            args: vec![
                "exec".into(),
                "-s".into(),
                "workspace-write".into(),
                "-C".into(),
                cwd.clone(),
                "--json".into(),
                "--skip-git-repo-check".into(),
                "resume".into(),
                thread.to_string(),
                "-".into(),
            ],
            stdin: Some(prompt),
            cwd,
            scrub_env_prefixes: agent_scrub_env(),
            executed: false,
        }
    }

    async fn process_run(&self, run: &store::ReactorRunRow) {
        let now = Utc::now();
        let q = match self.handle.get_question(&run.question_id).await {
            Ok(Some(q)) => q,
            Ok(None) => {
                let _ = store::mark_reactor_dead(
                    self.handle.pool(),
                    &run.answer_id,
                    "題目不存在",
                    now,
                )
                .await;
                return;
            }
            Err(e) => {
                tracing::error!("reactor 讀題失敗（{}）：{e}", run.question_id);
                return;
            }
        };
        let Some(answer) = q.answer.clone() else {
            let _ = store::mark_reactor_dead(
                self.handle.pool(),
                &run.answer_id,
                "題目無答案",
                now,
            )
            .await;
            return;
        };
        // 現行 wake：起跑當下即標 acked_at（harness-roundtrip 場景 9）。
        // 回答延續原對話：真的交到原對話（等待者、追加、自行回報、開新對話）才算接手，
        // 所以 held 時 acked_at 保持空白，24 小時沒人處理告警才觸發得到。
        let continuation =
            continuity::continuation_applies(&q.source, &q.wake_on_answer, q.internal);
        if !continuation {
            let _ = self.handle.record_ack(&q.id, now).await;
        }

        // internal 永遠全乾跑（連 prepare 都不 spawn）；global dry_run 只擋 commit/wake。
        let fully_dry = q.internal || run.status == "dry_run";
        let dry = fully_dry || self.cfg.dry_run;

        // 冪等 claim（Codex 覆檢 P1）：任何外部副作用之前先把本列持久化為
        // executing——崩潰後恢復只會走 needs_reconcile 人工對帳，永不重發。
        if !dry {
            let _ = store::mark_reactor_executing(self.handle.pool(), &run.answer_id, now).await;
        }

        let mut parts: Vec<String> = Vec::new();

        // ---- 錢的授權鏈（CONTRACT §6；一鍵只覆蓋 gate=yellow）----
        if self.money_chain_applies(&q, &answer) {
            if fully_dry {
                let prep = q
                    .money
                    .as_ref()
                    .and_then(|m| m.executor_prepare_cmd.clone())
                    .unwrap_or_default();
                self.record_planned(&q.id, "prepare", "(internal-dry)", vec![prep], None, false);
                parts.push("DRY_RUN(internal): prepare 未執行".to_string());
            } else {
                match self.money_chain(&q).await {
                    Ok(desc) => parts.push(desc),
                    Err(reason) => {
                        // prepare 失敗/解析不到 token/guard 升紅/commit 被拒 →
                        // 執行流 fail 事件 + execution_blocked，絕不靜默。
                        self.blocked(&q, &reason).await;
                        // 原對話也要知道錢鏈被擋下（只給人話狀態，不轉貼原始輸出）。
                        if continuation {
                            let current = self.handle.get_question(&q.id).await.ok().flatten().unwrap_or(q.clone());
                            if let DeliverOutcome::Part(text) =
                                self.deliver(&current, dry, Some("EXEC_BLOCKED"), Utc::now()).await
                            {
                                let prev = current.answer.as_ref().and_then(|a| a.execution_result.clone());
                                let _ = self
                                    .handle
                                    .record_execution(&q.id, &combine_execution(prev.as_deref(), &text), Utc::now())
                                    .await;
                            }
                        }
                        return;
                    }
                }
            }
        }

        // ---- 交付：回答延續原對話（PRD §20）／現行 wake（CONTRACT §8）----
        let money_summary = parts.first().cloned();
        // 錢鏈一有結果就先寫進執行紀錄（也進帳本）：之後任何交付路徑失敗只在後面接原因，不會蓋掉錢的結果（AC-CC-14）。
        if let Some(money) = money_summary.as_deref()
            && let Err(e) = self.handle.amend_execution(&q.id, money, Utc::now()).await
        {
            tracing::error!("錢鏈結果回寫失敗（{}）：{e}", q.id);
        }
        match self.deliver(&q, dry, money_summary.as_deref(), Utc::now()).await {
            DeliverOutcome::Part(text) => parts.push(text),
            DeliverOutcome::Pending(note) => {
                if parts.is_empty() {
                    // 暫時交付狀態（等待者租約、held）不是執行也不是接手：只結束 reactor 列。
                    let _ = self.handle.finish_reactor_run(&run.answer_id, &note, Utc::now()).await;
                    return;
                }
            }
            DeliverOutcome::Skip => {}
            // 死信已在 dead_letter 一次寫成「錢鏈結果；交付失敗：…」。
            DeliverOutcome::Dead => return,
        }

        if parts.is_empty() {
            parts.push("NOOP: 無需執行（wake=none 且非錢鏈）".to_string());
        }
        let result = parts.join("；");
        if let Err(e) = self.handle.record_execution(&q.id, &result, Utc::now()).await {
            tracing::error!("reactor 回寫 execution 失敗（{}）：{e}", q.id);
        }
    }

    /// 錢鏈適用判準：channel=ui（INV-15 inv_money_chain_ui_only 的第二道防線；
    /// 服務層已拒非 ui 通道作答錢題，這裡再擋一次 fail closed）∧ is_money_op ∧
    /// 有 prepare cmd ∧ gate≠red ∧ 使用者選了執行型選項（recommended=true）。
    /// red 題作答只落 ledger 不執行（money-approval 場景 10）；
    /// 選安全選項＝不動錢，正常跳過。
    fn money_chain_applies(&self, q: &Question, answer: &Answer) -> bool {
        money_chain_applies_to(q, answer)
    }

    /// prepare → 白名單/所見即所簽 → 簽章 → 簽發登記 → approval 檔 → commit。
    /// commit 在 dry_run 只記錄。Err = 執行封鎖原因（呼叫端標 execution_blocked）。
    async fn money_chain(&self, q: &Question) -> Result<String, String> {
        self.emit_exec(&q.id, ExecStep::Prepare, ExecStatus::Start, None);
        let chain = match run_approval_chain(&self.cfg.vault_root, q, self.key.as_ref()).await {
            Ok(c) => c,
            Err(reason) => {
                self.emit_exec(&q.id, ExecStep::Prepare, ExecStatus::Fail, Some(&reason));
                return Err(reason);
            }
        };
        // executor token 不進事件（/events 與回執畫面都看得到事件文字，AC-CC-14）。
        self.emit_exec(&q.id, ExecStep::Prepare, ExecStatus::Ok, Some("已取得執行憑證"));

        // 簽發登記（verify 強化）：commit 之前必須落 approvals_issued——
        // executor 的 /approvals/verify 只認登記過的 record。登記失敗＝封鎖。
        let answer_id =
            q.answer.as_ref().map(|a| a.answer_id.clone()).unwrap_or_default();
        if let Err(e) = self
            .handle
            .record_approval_issued(
                &q.id,
                &answer_id,
                &chain.nonce,
                &chain.executor_token,
                &chain.param_hash,
                chain.answered_at_epoch,
                Utc::now(),
            )
            .await
        {
            return Err(format!("approval 簽發登記失敗（fail closed，不 commit）：{e}"));
        }

        let commit = PlannedCommand {
            question_id: q.id.clone(),
            kind: "commit".to_string(),
            program: chain.commit_argv[0].clone(),
            args: chain.commit_argv[1..].to_vec(),
            stdin: None,
            cwd: self.cfg.vault_root.to_string_lossy().to_string(),
            scrub_env_prefixes: Vec::new(),
            executed: !self.cfg.dry_run,
        };
        self.planned.lock().expect("planned lock").push(commit.clone());

        if self.cfg.dry_run {
            return Ok(format!(
                "APPROVAL_SIGNED: {}；DRY_RUN commit: {} {}",
                chain.approval_path.display(),
                commit.program,
                commit.args.join(" ")
            ));
        }

        self.emit_exec(&q.id, ExecStep::Api, ExecStatus::Start, None);
        // 每一次授權（nonce）都記一筆錢鏈結果（CONTRACT §7）：「結果不明」以 nonce 判斷，不受之後的交付紀錄影響。
        let outcome = |result: &str| {
            let (qid, nonce, result) = (q.id.clone(), chain.nonce.clone(), result.to_string());
            async move {
                if let Err(e) = self.handle.record_money_outcome(&qid, &nonce, &result, Utc::now()).await {
                    tracing::error!("錢鏈結果登記失敗（{qid}）：{e}");
                }
            }
        };
        match self.exec_once(&commit).await {
            Ok(output) => {
                self.emit_exec(&q.id, ExecStep::Api, ExecStatus::Ok, None);
                let result = format!("COMMIT_OK: {}", tail(&output, 300));
                outcome(&result).await;
                Ok(result)
            }
            Err(reason) => {
                // commit 被 executor 護欄拒絕＝封鎖不重試（重試只會重複被拒）。executor 的輸出可能印出 token 與
                // approval 檔路徑：進事件、Discord 與 execution_result 前先遮掉（AC-CC-14）。
                let reason = crate::approval::mask_chain_secrets(&reason, &chain.executor_token, &chain.approval_path);
                self.emit_exec(&q.id, ExecStep::Api, ExecStatus::Fail, Some(&reason));
                let blocked = format!("executor commit 失敗：{reason}");
                // 沒被消耗才是被擋：查消耗與記 EXEC_BLOCKED 在同一段寫入鎖內，與 /approvals/verify 的消耗互斥——記成被擋的授權
                // 之後一律驗證失敗（INV-9）。讀寫失敗 fail closed，當作已消耗（結果不明）。
                let settled = self
                    .handle
                    .settle_unverified_nonce(&q.id, &chain.nonce, &format!("EXEC_BLOCKED: {blocked}"), Utc::now())
                    .await
                    .unwrap_or_else(|e| {
                        tracing::error!("被擋的授權記不進去（{}）：{e}", q.id);
                        false
                    });
                // executor 已經驗證（消耗）這次的 approval：它可能已經打了 API，錢可能已經動過——不是護欄擋下，
                // 記成「結果不明」請使用者人工對帳，也不准重新授權（AC-CC-14）。
                if !settled {
                    self.handle.discord().send(DiscordMessage {
                        question_id: Some(q.id.clone()),
                        reason: "execution_unknown".to_string(),
                        text: format!(
                            "⚠️ 「{}」executor 已用掉這次的授權，但 commit 沒有回報結果（{reason}）——錢可能已經動過。Kunsu 不會重跑，請人工對帳。",
                            q.title
                        ),
                    });
                    let result = format!("EXEC_UNKNOWN: executor 已用掉授權但 commit 沒有回報結果：{reason}");
                    outcome(&result).await;
                    return Ok(result);
                }
                // 被擋的授權刪掉 approval 檔（best effort；真正的保證是 verify 拒絕已有結果的授權）。
                crate::approval::remove_approval_file(&chain.approval_path);
                Err(blocked)
            }
        }
    }

    /// 執行封鎖（prepare 失敗/解析不到 token/guard 升紅/commit 被拒）：
    /// execution_result 標 EXEC_BLOCKED + Discord + 死信行——絕不靜默。
    async fn blocked(&self, q: &Question, reason: &str) {
        let now = Utc::now();
        let text = format!("EXEC_BLOCKED: {reason}");
        if let Err(e) = self.handle.record_execution(&q.id, &text, now).await {
            tracing::error!("execution_blocked 回寫失敗（{}）：{e}", q.id);
        }
        self.handle.discord().send(DiscordMessage {
            question_id: Some(q.id.clone()),
            reason: "execution_blocked".to_string(),
            text: format!("⛔ 「{}」批准後執行被擋：{reason}", q.title),
        });
    }

    /// 死信：重試耗盡。EXEC_FAILED 回寫 + reactor_runs 標 dead + Discord。
    async fn dead_letter(&self, q: &Question, answer_id: &str, reason: &str) {
        let now = Utc::now();
        let dead = format!("死信（重試 {} 次仍失敗）：{reason}", self.cfg.max_attempts);
        // 錢鏈已有結果（已執行、被擋、乾跑）：保留在前面，只接上交付失敗原因（任何交付路徑都一樣，AC-CC-14）。
        let prev = self.handle.get_question(&q.id).await.ok().flatten().and_then(|c| c.answer).and_then(|a| a.execution_result);
        let text = match prev.as_deref().map(str::trim).filter(|p| is_money_result(p)) {
            Some(p) => format!("{p}；交付失敗：{dead}"),
            None => format!("EXEC_FAILED: {dead}"),
        };
        if let Err(e) = self.handle.record_execution(&q.id, &text, now).await {
            tracing::error!("死信回寫失敗（{}）：{e}", q.id);
        }
        let _ = store::mark_reactor_dead(self.handle.pool(), answer_id, reason, now).await;
        if !q.internal {
            // 告警聚合：10 分鐘窗內前 3 筆照發、第 4 筆發合併總結、之後同窗靜默（帳本照記）。
            let msg = {
                let mut w = self.dead_letter_window.lock().unwrap();
                let fresh = w.0.is_none_or(|t| (now - t).num_seconds() > 600);
                if fresh {
                    *w = (Some(now), 1);
                } else {
                    w.1 += 1;
                }
                match w.1 {
                    1..=3 => Some(format!(
                        "💀 「{}」喚醒重試 {} 次仍失敗，進死信：{reason}",
                        q.title, self.cfg.max_attempts
                    )),
                    4 => Some(
                        "⚠️ 死信連環：10 分鐘內已 4 筆（多半同一根因）。後續同窗死信只記帳本\
                         不再刷頻道——答案都安全在帳本裡，明早排程死信補掃會逐筆處理。"
                            .to_string(),
                    ),
                    _ => None,
                }
            };
            if let Some(text) = msg {
                self.handle.discord().send(DiscordMessage {
                    question_id: Some(q.id.clone()),
                    reason: "reactor_dead_letter".to_string(),
                    text,
                });
            }
        }
        self.handle.emit_event(ServerEvent::ReactorDead {
            question_id: q.id.clone(),
            answer_id: answer_id.to_string(),
            reason: reason.to_string(),
        });
    }

    fn emit_exec(&self, qid: &str, step: ExecStep, status: ExecStatus, line: Option<&str>) {
        self.handle.emit_event(ServerEvent::Execution {
            question_id: qid.to_string(),
            step,
            status,
            line: line.map(str::to_string),
        });
    }

    fn record_planned(
        &self,
        qid: &str,
        kind: &str,
        program: &str,
        args: Vec<String>,
        stdin: Option<String>,
        executed: bool,
    ) {
        self.planned.lock().expect("planned lock").push(PlannedCommand {
            question_id: qid.to_string(),
            kind: kind.to_string(),
            program: program.to_string(),
            args,
            stdin,
            cwd: self.cfg.vault_root.to_string_lossy().to_string(),
            scrub_env_prefixes: Vec::new(),
            executed,
        });
    }

    // ------------------------------------------------------------------
    // wake 命令組裝（CONTRACT §8 逐字）
    // ------------------------------------------------------------------

    fn resolve_claude(&self) -> PathBuf {
        if let Some(p) = &self.cfg.claude_bin {
            return p.clone();
        }
        self.claude_path
            .get_or_init(|| which("claude").unwrap_or_else(|| PathBuf::from("claude")))
            .clone()
    }

    fn build_wake_command(&self, q: &Question) -> PlannedCommand {
        let playbook = q
            .wake_on_answer
            .playbook
            .as_deref()
            .map(|rel| {
                // playbook 解析順序：①~/.kunsu/playbooks/<rel>（安裝時複製的實體檔，不碰
                // 受保護目錄）②vault_root/<rel>（相容舊寫法）。都讀不到就退化成「僅憑題目
                // 與答案判斷」並在 prompt 裡標明，絕不靜默。
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                let candidates = [
                    PathBuf::from(&home).join(".kunsu/playbooks").join(rel),
                    self.cfg.vault_root.join(rel),
                ];
                candidates
                    .iter()
                    .find_map(|p| std::fs::read_to_string(p).ok())
                    .unwrap_or_else(|| {
                        format!("（playbook {rel} 在 ~/.kunsu/playbooks 與 vault 皆讀不到——僅憑題目與答案自行判斷）")
                    })
            })
            .unwrap_or_default();
        let q_json = agent_question_json(q);
        let mut prompt = format!("{playbook}\n\n## 題目與答案\n{q_json}");
        // 題組：新對話也拿到逐項決定與規則（一字不改送出、下一輪題組），不只一包 JSON。
        if let Some(a) = q.answer.as_ref()
            && let Some(decisions) = a.items.as_deref()
        {
            prompt.push_str("\n\n");
            prompt.push_str(&batch_fresh_section(q, a, decisions));
        }
        let cwd = self.cfg.vault_root.to_string_lossy().to_string();

        if q.source.agent == AgentKind::Codex {
            let program = self.codex_program();
            return PlannedCommand {
                question_id: q.id.clone(),
                kind: "wake_codex".to_string(),
                program: program.to_string_lossy().to_string(),
                args: vec![
                    "exec".into(),
                    "-".into(),
                    "--json".into(),
                    "-s".into(),
                    "workspace-write".into(),
                    "-C".into(),
                    cwd.clone(),
                ],
                stdin: Some(prompt),
                cwd,
                scrub_env_prefixes: agent_scrub_env(),
                executed: false,
            };
        }

        // 現行 fresh 模式（回答延續原對話的 resume 走 build_claude_resume／build_codex_resume）。
        // prompt 走 stdin，不放進命令列參數：最大題組（20 項 × 5,000 字＋原文）的 prompt 超過 1 MB，會撞 macOS ARG_MAX
        // 而 spawn 失敗、死信（QA 第 1 輪 AC-BR-06）。`claude --help`：`--input-format` 只用於 --print，預設 text。
        let args: Vec<String> = vec![
            "-p".into(),
            "--input-format".into(),
            "text".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--permission-mode".into(),
            "bypassPermissions".into(),
            "--max-budget-usd".into(),
            "2".into(),
        ];
        PlannedCommand {
            question_id: q.id.clone(),
            kind: "wake_claude".to_string(),
            program: self.resolve_claude().to_string_lossy().to_string(),
            args,
            stdin: Some(prompt),
            cwd,
            scrub_env_prefixes: agent_scrub_env(),
            executed: false,
        }
    }

    // ------------------------------------------------------------------
    // spawn（不經 shell）＋重試退避
    // ------------------------------------------------------------------

    async fn exec_once(&self, cmd: &PlannedCommand) -> Result<String, String> {
        self.exec_once_detailed(cmd).await.map_err(|f| f.detail)
    }

    /// 執行一次（不經 shell）：同時收 stdout／stderr，超過 `exec_timeout` 中止子行程並保留已收到的輸出。
    async fn exec_once_detailed(&self, cmd: &PlannedCommand) -> Result<String, ExecFailure> {
        let mut c = tokio::process::Command::new(&cmd.program);
        c.args(&cmd.args).current_dir(&cmd.cwd).kill_on_drop(true);
        #[cfg(unix)]
        c.process_group(0);
        for (key, _) in std::env::vars_os() {
            let k = key.to_string_lossy();
            if cmd.scrub_env_prefixes.iter().any(|p| k.starts_with(p.as_str())) {
                c.env_remove(&key);
            }
        }
        c.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        // 沒有要給的輸入就給空的 stdin：不繼承 Kunsu 自己的 stdin（子行程讀 stdin 時才不會永遠等不到結尾）。
        if cmd.stdin.is_some() {
            c.stdin(std::process::Stdio::piped());
        } else {
            c.stdin(std::process::Stdio::null());
        }
        let mut child = c
            .spawn()
            .map_err(|e| ExecFailure::simple(format!("spawn 失敗（{}）：{e}", cmd.program)))?;
        // stdin 在背景寫：子行程不讀 stdin 時也受逾時保護（逾時中止後管線關閉，寫入自然結束）。
        if let Some(input) = &cmd.stdin
            && let Some(mut sin) = child.stdin.take()
        {
            let bytes = input.clone().into_bytes();
            tokio::spawn(async move {
                if let Err(e) = sin.write_all(&bytes).await {
                    tracing::warn!("stdin 寫入失敗：{e}");
                }
            });
        }
        // 邊跑邊收輸出：逾時中止後仍保留已收到的部分（判斷 agent 是否已開始處理）。
        let out_buf = Arc::new(StdMutex::new(Vec::new()));
        let err_buf = Arc::new(StdMutex::new(Vec::new()));
        let out_task = tokio::spawn(drain_pipe(child.stdout.take(), out_buf.clone()));
        let err_task = tokio::spawn(drain_pipe(child.stderr.take(), err_buf.clone()));
        let pid = child.id();
        let status = match tokio::time::timeout(self.cfg.exec_timeout, child.wait()).await {
            Ok(r) => Some(r.map_err(|e| ExecFailure::simple(format!("等待子程序失敗：{e}")))?),
            Err(_) => {
                // 整個行程群組一起停（agent 底下的子行程也會握著輸出管線）。
                #[cfg(unix)]
                if let Some(pid) = pid {
                    // SAFETY：killpg 只送訊號給 spawn 時建立的行程群組，不碰其他記憶體。
                    unsafe {
                        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
                    }
                }
                let _ = child.kill().await;
                None
            }
        };
        // 子行程結束後輸出管線很快關閉；有孫行程握著時最多再等 2 秒。
        let grace = std::time::Duration::from_secs(2);
        let _ = tokio::time::timeout(grace, out_task).await;
        let _ = tokio::time::timeout(grace, err_task).await;
        let stdout = String::from_utf8_lossy(&out_buf.lock().expect("out buf")).to_string();
        let stderr = String::from_utf8_lossy(&err_buf.lock().expect("err buf")).to_string();
        match status {
            Some(st) if st.success() => Ok(format!("{stdout}\n{stderr}")),
            Some(st) => Err(ExecFailure {
                detail: format!("非零退出（{st}）：{}", exec_failure_detail(&stdout, &stderr)),
                stdout,
                stderr,
                timed_out: false,
            }),
            None => Err(ExecFailure {
                detail: format!("超過 {} 仍未結束，已中止", human_duration(self.cfg.exec_timeout)),
                stdout,
                stderr,
                timed_out: true,
            }),
        }
    }

    /// 開新對話的重試：agent 還沒開始處理才重試；已開始或逾時立刻回報，不再跑一次。
    async fn exec_with_retries(&self, answer_id: &str, cmd: &PlannedCommand) -> Result<String, ExecFailure> {
        let codex = cmd.kind.contains("codex");
        let mut last = ExecFailure::simple(String::new());
        for attempt in 0..self.cfg.max_attempts {
            match self.exec_once_detailed(cmd).await {
                Ok(out) => return Ok(out),
                Err(f) => {
                    let _ = store::bump_reactor_attempts(self.handle.pool(), answer_id, (attempt + 1) as i64, Utc::now()).await;
                    if f.timed_out || continuity::agent_started(codex, &f.stdout) {
                        return Err(f);
                    }
                    last = f;
                    if attempt + 1 < self.cfg.max_attempts {
                        let backoff = self.cfg.retry_base_ms * 2u64.pow(attempt);
                        tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                    }
                }
            }
        }
        Err(last)
    }
}

fn human_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 { format!("{} 分鐘", secs.div_ceil(60)) } else { format!("{} 秒", secs.max(1)) }
}

/// 把子行程的輸出管線讀進共用緩衝區（逾時中止時仍拿得到已輸出的部分）。
async fn drain_pipe<R: tokio::io::AsyncRead + Unpin>(pipe: Option<R>, buf: Arc<StdMutex<Vec<u8>>>) {
    use tokio::io::AsyncReadExt;
    let Some(mut pipe) = pipe else { return };
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.lock().expect("pipe buf").extend_from_slice(&chunk[..n]),
        }
    }
}

/// 給 agent 看的題目 JSON（開新對話 prompt 用）：去掉簽章、prepare 命令與錢鏈原始輸出。
pub fn agent_question_json(q: &Question) -> String {
    let mut redacted = q.clone();
    if let Some(a) = redacted.answer.as_mut() {
        a.signature = None;
        a.execution_result = None;
    }
    if let Some(m) = redacted.money.as_mut() {
        m.executor_prepare_cmd = None;
    }
    serde_json::to_string_pretty(&redacted).unwrap_or_else(|_| "{}".to_string())
}

/// 錢鏈結果與交付文字合併（held 轉追加時不覆寫 COMMIT_OK 等錢鏈結果）。
fn combine_execution(prev: Option<&str>, text: &str) -> String {
    match prev.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) if p != text => format!("{p}；{text}"),
        _ => text.to_string(),
    }
}

/// 回答延續原對話時追加到原對話的訊息（AC-CC-06/14）。不含任何授權秘密。
pub fn continuation_prompt(q: &Question, money_summary: Option<&str>) -> String {
    if let Some(a) = q.answer.as_ref()
        && let Some(decisions) = a.items.as_deref()
    {
        return batch_continuation_prompt(q, a, decisions);
    }
    let answer = q.answer.as_ref();
    let answer_id = answer.map(|a| a.answer_id.as_str()).unwrap_or("");
    let mut lines = vec![
        "[Kunsu] 使用者已回答你在這段對話提出的題目，請接續原本的工作。".to_string(),
        String::new(),
        format!("- 題目：{}", q.question_line),
        format!("- 題目 id：{}", q.id),
    ];
    if let Some(cid) = answer.and_then(|a| a.choice_id.as_deref()) {
        let label = q.choice_label(cid).unwrap_or("");
        lines.push(format!("- 使用者的選擇：{label}（選項 id：{cid}）"));
    }
    if let Some(text) = answer.and_then(|a| a.free_text.as_deref()).filter(|t| !t.trim().is_empty()) {
        lines.push(format!("- 使用者的文字回答：{text}"));
    }
    if let Some(params) = answer.and_then(|a| a.edited_params.as_deref()) {
        lines.push(format!("- 使用者修改後的參數：{params}"));
    }
    lines.push(format!("- answer_id：{answer_id}"));
    if q.is_money_op() {
        // 只給人話狀態，絕不轉貼錢鏈原始輸出（approval 路徑、token、nonce、簽章都留在 Kunsu）。
        let status = money_status(money_summary);
        lines.push(format!(
            "- 這是錢類題：執行一律由 Kunsu 錢鏈負責，你不要自行動錢。Kunsu 錢鏈狀態：{status}"
        ));
    }
    lines.push(String::new());
    lines.push(format!(
        "請先呼叫 Kunsu MCP `ask_get {{\"question_id\": \"{}\"}}` 確認正式答案，再繼續原本的工作。",
        q.id
    ));
    lines.push(format!(
        "做完後呼叫 `ask_report {{\"question_id\": \"{}\", \"answer_id\": \"{answer_id}\", \"status\": \"done|failed|blocked\", \"summary_md\": \"給使用者的結果與回饋\"}}`。",
        q.id
    ));
    lines.join("\n")
}

/// 題組的回到原對話訊息（CONTRACT §14；BDD batch-review 場景 6）：逐項決定、採用的最終文字原樣附上（一字不改送出）、
/// 請 AI 改的項目以下一輪題組送回。題組不能動錢，沒有錢鏈狀態。
fn batch_continuation_prompt(q: &Question, answer: &Answer, decisions: &[ItemDecision]) -> String {
    let answer_id = answer.answer_id.as_str();
    let mut lines = vec![
        "[Kunsu] 使用者已回答你在這段對話提出的題組，請接續原本的工作。".to_string(),
        String::new(),
        format!("- 題目：{}", q.question_line),
        format!("- 題目 id：{}", q.id),
    ];
    lines.extend(batch_overview(q, answer, decisions));
    lines.push(format!("- answer_id：{answer_id}"));
    lines.push(String::new());
    lines.push(batch_sections(q, answer, decisions));
    lines.push(String::new());
    lines.push(format!(
        "請先呼叫 Kunsu MCP `ask_get {{\"question_id\": \"{}\"}}` 確認正式答案，再繼續原本的工作。",
        q.id
    ));
    lines.push(format!(
        "做完後呼叫 `ask_report {{\"question_id\": \"{}\", \"answer_id\": \"{answer_id}\", \"status\": \"done|failed|blocked\", \"summary_md\": \"已送出幾項、不處理幾項、重寫幾項（與新題組 id）；送不出去的逐項寫原因\"}}`。",
        q.id
    ));
    lines.join("\n")
}

/// 開新對話的 prompt 附上的題組段落（題目 JSON 之外，讓新對話也照同一套規則做）。
fn batch_fresh_section(q: &Question, answer: &Answer, decisions: &[ItemDecision]) -> String {
    let mut lines = vec!["## 題組：使用者的逐項決定（照這裡做）".to_string()];
    lines.extend(batch_overview(q, answer, decisions));
    lines.push(String::new());
    lines.push(batch_sections(q, answer, decisions));
    lines.join("\n")
}

/// 第幾輪、總數與整體指示。
fn batch_overview(q: &Question, answer: &Answer, decisions: &[ItemDecision]) -> Vec<String> {
    use kunsu_core::batch;
    let (_, round) = batch::round_of(&q.id);
    let c = batch::counts(&q.items, decisions);
    let mut summary = format!("採用 {} 項", c.approve);
    if c.edited > 0 {
        summary.push_str(&format!("（其中 {} 項使用者改過）", c.edited));
    }
    summary.push_str(&format!(" · 不處理 {} · 請 AI 改 {}", c.skip, c.revise));
    let mut lines = vec![format!("- 題組：第 {round} 輪，共 {} 項；{summary}", decisions.len())];
    if let Some(text) = answer.free_text.as_deref().filter(|t| !t.trim().is_empty()) {
        lines.push(format!("- 使用者的整體指示：{text}（適用整批；不得因此改動採用項目的最終文字）"));
    }
    lines
}

/// 「逐項決定」與「接下來」兩段：採用的最終文字放在 fenced block 原樣保留；請 AI 改附指示與下一輪的 id。
fn batch_sections(q: &Question, answer: &Answer, decisions: &[ItemDecision]) -> String {
    use kunsu_core::batch;
    let (_, round) = batch::round_of(&q.id);
    let c = batch::counts(&q.items, decisions);
    let mut lines = vec!["## 逐項決定".to_string()];
    for (n, d) in decisions.iter().enumerate() {
        let item = q.items.iter().find(|i| i.id == d.item_id);
        let name = match item {
            Some(i) => match i.subtitle.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(sub) => format!("{}｜{sub}", i.title),
                None => i.title.clone(),
            },
            None => d.item_id.clone(),
        };
        let head = format!("{}. {name}（item_id: {}）", n + 1, d.item_id);
        lines.push(String::new());
        match d.action {
            ItemAction::Approve => match (item, d.text.as_deref()) {
                (Some(i), Some(text)) => {
                    let edited = if batch::is_edited(i, d) { "（使用者改過）" } else { "" };
                    lines.push(format!("{head}— 採用{edited}。最終文字（一字不改送出）："));
                    let fence = "`".repeat(longest_backtick_run(text).max(2) + 1);
                    lines.push(format!("{fence}text"));
                    lines.push(text.to_string());
                    lines.push(fence);
                }
                (Some(i), None) => lines.push(format!("{head}— 採用：照你的建議執行：{}", i.proposal)),
                (None, _) => lines.push(format!("{head}— 採用")),
            },
            ItemAction::Skip => lines.push(format!("{head}— 不處理")),
            ItemAction::Revise => {
                lines.push(format!("{head}— 請 AI 改：{}", d.note.as_deref().unwrap_or("")));
            }
        }
    }
    lines.push(String::new());
    lines.push("## 接下來".to_string());
    lines.push(
        "1. 採用的項目：用上面的最終文字逐項執行，一字不改（不要潤稿、不要加稱呼或簽名、不要改標點與換行）；只核可的項目照你的建議執行。"
            .to_string(),
    );
    lines.push("2. 不處理的項目：不要動。".to_string());
    if c.revise > 0 {
        let next = batch::next_round_id(&q.id);
        lines.push(format!(
            "3. 請 AI 改的 {} 項：先做完採用的項目，再依每項指示{}重寫，然後建第 {} 輪題組：id 用 `{next}`，只放這 {} 項（沿用原 item_id），why 第一行寫「第 {} 輪：{} 退回的 {} 項」。",
            c.revise,
            if answer.free_text.as_deref().is_some_and(|t| !t.trim().is_empty()) { "與整體指示" } else { "" },
            round + 1,
            c.revise,
            round + 1,
            q.id,
            c.revise,
        ));
    }
    lines.join("\n")
}

/// 文字裡最長的連續反引號（fenced block 的圍欄要比它長，內容才能原樣保留）。
fn longest_backtick_run(text: &str) -> usize {
    let mut best = 0;
    let mut run = 0;
    for ch in text.chars() {
        if ch == '`' {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

/// 錢鏈結果的人話狀態（給 agent 看；原始輸出含 approval 路徑與 token，只留在 Kunsu）。
/// 錢鏈的人話狀態：只看執行紀錄的開頭（錢鏈結果永遠在最前面；交付文字不算）。
pub fn money_status(raw: Option<&str>) -> &'static str {
    let s = raw.unwrap_or("").trim_start();
    if s.starts_with("COMMIT_OK") {
        "已執行完成"
    } else if s.starts_with("RECONCILED_EXECUTED") {
        "已執行完成（使用者人工對帳確認）"
    } else if s.starts_with("RECONCILED_NOT_EXECUTED") {
        "沒有執行（使用者人工對帳確認）"
    } else if s.starts_with("EXEC_UNKNOWN") {
        "結果不明：executor 已用掉授權但沒有回報結果，錢可能已經動過，等使用者人工對帳"
    } else if s.starts_with("EXEC_BLOCKED") {
        "被安全護欄擋下（細節見 Kunsu 回執）"
    } else if s.starts_with("DRY_RUN") || s.starts_with("APPROVAL_SIGNED") {
        "乾跑，未實際執行"
    } else if s.starts_with("EXEC_FAILED") {
        "執行沒有完成（細節見 Kunsu 回執）"
    } else {
        "未觸發（安全選項或紅燈題只記錄決定）"
    }
}

/// 錢鏈還沒跑完時 agent 讀到的狀態。
pub const MONEY_PENDING: &str = "錢鏈還在跑，以 Kunsu 回執為準";

/// 執行紀錄是否以錢鏈結果開頭。
pub fn is_money_result(s: &str) -> bool {
    ["COMMIT_OK", "EXEC_BLOCKED", "EXEC_UNKNOWN", "RECONCILED_EXECUTED", "RECONCILED_NOT_EXECUTED", "DRY_RUN", "APPROVAL_SIGNED"]
        .iter()
        .any(|p| s.starts_with(p))
}

/// 錢鏈適用判準（純函數；reactor 與 agent 讀取面共用）：channel∈{ui, mobile} ∧ is_money_op ∧ 有 prepare cmd ∧ gate≠red ∧
/// 使用者選了執行型選項（recommended=true）。
pub fn money_chain_applies_to(q: &Question, answer: &Answer) -> bool {
    if !answer.channel.is_ray_device() {
        return false;
    }
    let money_ready = q.is_money_op() && q.money.as_ref().is_some_and(|m| m.executor_prepare_cmd.is_some());
    if !money_ready || q.gate == Some(Gate::Red) {
        return false;
    }
    answer
        .choice_id
        .as_ref()
        .is_some_and(|cid| q.options.iter().any(|o| &o.id == cid && o.recommended == Some(true)))
}

/// 交付一步的結果。
enum DeliverOutcome {
    /// 已定案，這段文字寫進 execution。
    Part(String),
    /// 尚未定案（等待者租約或 held），這段文字是暫時狀態說明。
    Pending(String),
    /// 不需要喚醒，也不寫 execution。
    Skip,
    /// 開新對話也失敗，已進死信。
    Dead,
}

fn is_local_session_id(s: &str) -> bool {
    s.strip_prefix("local_").is_some_and(|rest| {
        !rest.is_empty() && rest.len() <= 80 && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

fn pid_alive(pid: u64) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `EXEC_OK:` / `EXEC_FAILED:` 前綴解析（CONTRACT §8）。
pub fn parse_exec_marker(output: &str) -> Option<String> {
    for line in output.lines() {
        if let Some(i) = line.find("EXEC_OK:") {
            return Some(line[i..].trim().to_string());
        }
        if let Some(i) = line.find("EXEC_FAILED:") {
            return Some(line[i..].trim().to_string());
        }
    }
    None
}

/// 沒有注入路徑時找 claude（中轉 AI 與 reactor 同一套找法）。
pub(crate) fn default_claude_bin() -> PathBuf {
    which("claude").unwrap_or_else(|| PathBuf::from("claude"))
}

/// 沒有注入路徑時找 codex：優先 ChatGPT 桌面版內附的 CLI（跟桌面同版本）。
pub(crate) fn default_codex_bin() -> PathBuf {
    let bundled = cfg!(target_os = "macos").then(|| PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex"));
    resolve_codex_binary(None, bundled, which("codex"))
}

fn resolve_codex_binary(injected: Option<PathBuf>, bundled: Option<PathBuf>, fallback: Option<PathBuf>) -> PathBuf {
    injected.or_else(|| bundled.filter(|path| {
        if !path.is_file() { return false; }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            path.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        { true }
    })).or(fallback).unwrap_or_else(|| PathBuf::from("codex"))
}

/// `which`：PATH 掃描（不經 shell、無 zsh alias）＋ GUI app 補救清單。
/// launchd 啟動的 GUI app PATH 只有 /usr/bin:/bin:/usr/sbin:/sbin——2026-08-01 作者清空
/// 14 題時 13 個 reactor 全因 spawn claude ENOENT 進死信，根因就是這行沒兜底。
fn which(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".npm-global/bin"));
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".bun/bin"));
        dirs.push(home.join(".cargo/bin"));
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));
    for dir in dirs {
        let candidate = dir.join(name);
        if candidate.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = candidate.metadata()
                    && meta.permissions().mode() & 0o111 != 0
                {
                    return Some(candidate);
                }
            }
            #[cfg(not(unix))]
            return Some(candidate);
        }
    }
    None
}

fn tail(s: &str, n: usize) -> String {
    let t = s.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() <= n {
        t.to_string()
    } else {
        chars[chars.len() - n..].iter().collect()
    }
}

fn exec_failure_detail(stdout: &str, stderr: &str) -> String {
    for line in stdout.lines().chain(stderr.lines()).rev() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("is_error").and_then(|v| v.as_bool()) != Some(true) {
            continue;
        }
        if let Some(reason) = value.get("result").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
        {
            return humanize_exec_failure(reason, &value);
        }
        if let Some(reason) = value
            .pointer("/message/content/0/text")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            return humanize_exec_failure(reason, &value);
        }
    }

    tail(&format!("{stdout}\n{stderr}"), 400)
}

fn humanize_exec_failure(reason: &str, value: &serde_json::Value) -> String {
    let auth_failed = value.get("error").and_then(|v| v.as_str()) == Some("authentication_failed")
        || reason.to_ascii_lowercase().contains("not logged in");
    if auth_failed {
        "Claude Code 尚未登入；請先在 Terminal 執行 `claude auth login`".to_string()
    } else {
        tail(reason, 400)
    }
}

#[cfg(test)]
mod tests {
    use super::{exec_failure_detail, resolve_codex_binary};
    use std::path::PathBuf;

    #[test]
    fn stream_json_auth_failure_keeps_human_reason() {
        let stdout = concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Not logged in · Please run /login"}]},"error":"authentication_failed"}"#,
            "\n",
            r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login","usage":{"input_tokens":0},"terminal_reason":"completed"}"#,
        );

        assert_eq!(
            exec_failure_detail(stdout, ""),
            "Claude Code 尚未登入；請先在 Terminal 執行 `claude auth login`"
        );
    }
    /// AC-LP-08：桌面版存在但不是可執行檔時，仍需安全退回 PATH。
    #[test]
    fn codex_binary_prefers_executable_desktop_runtime_and_preserves_override() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bundled = dir.path().join("desktop-codex");
        let fallback = dir.path().join("path-codex");
        let injected = dir.path().join("injected-codex");
        let choose = |over: Option<PathBuf>| resolve_codex_binary(over, Some(bundled.clone()), Some(fallback.clone()));
        assert_eq!(choose(None), fallback);
        std::fs::create_dir(&bundled).unwrap();
        assert_eq!(choose(None), fallback);
        std::fs::remove_dir(&bundled).unwrap();
        std::fs::write(&bundled, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&bundled, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(choose(None), fallback);
        std::fs::set_permissions(&bundled, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(choose(None), bundled);
        assert_eq!(choose(Some(injected.clone())), injected);
        assert_eq!(resolve_codex_binary(None, None, None), PathBuf::from("codex"));
    }

}
