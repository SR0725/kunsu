//! 回答延續原對話第 5 輪：錢鏈結果在任何交付死信路徑都保留、重建不遺漏也不重複、帳本待補寫佇列、
//! 回報只對適用接續的題改 picked_up、/events 去敏（BDD conversation-continuity AC-CC-11／12／14；INV-13、18、25）。
//! 源自 QA 第 5 輪的重現測試。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration, Utc};
use common::*;
use kunsu_core::continuity::{DeliveryState, HostKind, ReportStatus};
use kunsu_core::types::{AgentKind, Source};
use kunsu_server::ledger::LedgerType;
use kunsu_server::mcp::{handle_rpc, McpReply};
use kunsu_server::{Reactor, ReactorConfig, ServerHandle, StaticKeyProvider};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Env {
    ctx: TestCtx,
    root: TempDir,
}

impl Env {
    async fn new() -> Env {
        let ctx = TestCtx::new().await;
        let root = TempDir::new().unwrap();
        for d in ["work", "registry", "desktop/acct/org"] {
            std::fs::create_dir_all(root.path().join(d)).unwrap();
        }
        Env { ctx, root }
    }
    fn work(&self) -> PathBuf {
        self.root.path().join("work")
    }
    fn registry(&self) -> PathBuf {
        self.root.path().join("registry")
    }
    fn log(&self) -> PathBuf {
        self.root.path().join("calls.log")
    }
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.log()).unwrap_or_default().lines().map(str::to_string).collect()
    }
    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.root.path().join(name);
        let content = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$(printf '%s ' \"$@\" | tr '\\n' ' ')\" >> {}\n{body}\n",
            self.log().display()
        );
        std::fs::write(&path, content).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    fn cfg(&self, claude: Option<PathBuf>, codex: Option<PathBuf>) -> ReactorConfig {
        let mut cfg = ReactorConfig::new(self.ctx.vault.path().to_path_buf());
        cfg.claude_bin = claude;
        cfg.codex_bin = codex;
        cfg.sessions_dir = self.registry();
        cfg.desktop_sessions_dir = self.root.path().join("desktop");
        cfg.retry_base_ms = 1;
        cfg.trusted_cli_roots.push(self.root.path().to_path_buf());
        cfg
    }
    fn reactor_on(&self, handle: &ServerHandle, claude: Option<PathBuf>, codex: Option<PathBuf>) -> Arc<Reactor> {
        Reactor::new(handle.clone(), self.cfg(claude, codex), Arc::new(StaticKeyProvider(test_key())))
    }
    fn reactor(&self, claude: Option<PathBuf>, codex: Option<PathBuf>) -> Arc<Reactor> {
        self.reactor_on(&self.ctx.handle, claude, codex)
    }
    /// 讓原對話「被活行程持有」（Claude 登記檔命中且 pid＝本測試行程）。
    fn hold(&self, conversation: &str) -> PathBuf {
        let reg = self.registry().join(format!("{}.json", std::process::id()));
        std::fs::write(&reg, json!({"pid": std::process::id(), "sessionId": conversation}).to_string()).unwrap();
        reg
    }
}

fn claude_source(env: &Env, conversation: &str, cli_path: Option<&Path>) -> Source {
    Source {
        agent: AgentKind::Claude,
        cwd: env.work().to_string_lossy().to_string(),
        host: Some(HostKind::ClaudeDesktop),
        conversation_id: Some(conversation.into()),
        host_session_id: Some("local_qa5-1".into()),
        attended: Some(false),
        cli_path: cli_path.map(|p| p.to_string_lossy().to_string()),
        ..Default::default()
    }
}

fn codex_source(env: &Env, thread: &str) -> Source {
    Source {
        agent: AgentKind::Codex,
        cwd: env.work().to_string_lossy().to_string(),
        host: Some(HostKind::CodexApp),
        conversation_id: Some(thread.into()),
        ..Default::default()
    }
}

async fn mcp(handle: &ServerHandle, name: &str, arguments: Value) -> Result<Value, Value> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
    match handle_rpc(handle, body).await {
        McpReply::Json(v) if v.get("error").is_some() => Err(v["error"].clone()),
        McpReply::Json(v) => Ok(v["result"]["structuredContent"].clone()),
        McpReply::Accepted => panic!("tools/call 不得回 Accepted"),
    }
}

fn ui_shows_blocked(exec: Option<&str>) -> bool {
    exec.is_some_and(|r| r.starts_with("EXEC_BLOCKED:") || r.starts_with("EXEC_FAILED:"))
}

const OK_BODY: &str = "echo 'EXEC_OK: 原對話已接手'";
/// 追加找不到對話 → 開新對話也一直失敗 → 死信。
const ALWAYS_FAIL: &str =
    "case \"$*\" in *--resume*) echo 'No conversation found with session ID: gone' >&2; exit 1;; esac\necho 'boom' >&2\nexit 1";

fn rows_for(ctx: &TestCtx, id: &str) -> Vec<kunsu_server::ledger::LedgerRow> {
    let (rows, _) = ctx.handle.ledger().read_all_rows().unwrap();
    rows.into_iter().filter(|r| r.question.id == id).collect()
}

// ---------------------------------------------------------------------------
// N1：錢已 commit、原對話被開著（held）→ 之後追加與開新對話都失敗進死信 → COMMIT_OK 被死信蓋掉
// （第 4 輪發現 4 的修正只在 process_run 首輪，process_due_deliveries 的 held 之後路徑沒補）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn held_then_dead_keeps_commit_ok() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", ALWAYS_FAIL);
    write_fake_executor(&env.ctx);
    let reg = env.hold("conv-qa5-hd");
    let mut req = money_t0_req("qa5-held-dead");
    req.source = claude_source(&env, "conv-qa5-hd", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question("qa5-held-dead", ui_answer("a"), Utc::now()).await.unwrap();

    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    let a = env.ctx.handle.get_question("qa5-held-dead").await.unwrap().unwrap().answer.unwrap();
    assert_eq!(a.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Held), "前提：原對話開著 → held");
    assert!(a.execution_result.as_deref().unwrap_or("").starts_with("COMMIT_OK"), "前提：錢已 commit：{:?}", a.execution_result);

    // 原對話關掉 → 追加找不到 → 開新對話 3 次失敗 → 死信。
    std::fs::remove_file(&reg).unwrap();
    reactor.process_due_deliveries(Utc::now() + Duration::seconds(61)).await.unwrap();
    let a = env.ctx.handle.get_question("qa5-held-dead").await.unwrap().unwrap().answer.unwrap();
    assert_eq!(a.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Dead), "前提：交付死信");
    let dead_alerts = env.ctx.discord.messages().iter().filter(|m| m.reason == "reactor_dead_letter").count();
    assert_eq!(dead_alerts, 1, "死信告警恰好一則");

    let exec = a.execution_result.clone().unwrap_or_default();
    let got = mcp(&env.ctx.handle, "ask_get", json!({"question_id": "qa5-held-dead"})).await.unwrap();
    assert!(
        exec.starts_with("COMMIT_OK") && exec.contains("交付失敗") && !ui_shows_blocked(Some(&exec)),
        "錢已 commit 但 held 之後交付死信：回執應以錢鏈結果開頭並說明交付失敗，實際 execution_result={exec:?}；\
         agent 讀到的錢鏈狀態={}",
        got["question"]["answer"]["execution_result"]
    );
}

// ---------------------------------------------------------------------------
// N2：錢鏈被擋 → 通知原對話時交付死信 → EXEC_BLOCKED（被擋原因）被「EXEC_FAILED: 死信」蓋掉。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn blocked_then_dead_keeps_blocked_result() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", ALWAYS_FAIL);
    // 不寫 executor → prepare 失敗 → 錢鏈被擋。
    let mut req = money_t0_req("qa5-blocked-dead");
    req.source = claude_source(&env, "conv-qa5-bd", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question("qa5-blocked-dead", ui_answer("a"), Utc::now()).await.unwrap();
    env.reactor(Some(claude), None).process_pending().await.unwrap();
    assert!(env.ctx.discord.messages().iter().any(|m| m.reason == "execution_blocked"), "前提：錢鏈被擋");
    let a = env.ctx.handle.get_question("qa5-blocked-dead").await.unwrap().unwrap().answer.unwrap();
    assert_eq!(a.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Dead), "前提：通知原對話的交付死信");
    let exec = a.execution_result.clone().unwrap_or_default();
    let got = mcp(&env.ctx.handle, "ask_get", json!({"question_id": "qa5-blocked-dead"})).await.unwrap();
    assert!(
        exec.starts_with("EXEC_BLOCKED"),
        "錢鏈被擋的真實結果應保留（回執看得到被擋原因），實際 execution_result={exec:?}；agent 讀到={}",
        got["question"]["answer"]["execution_result"]
    );
}

// ---------------------------------------------------------------------------
// N4：答案已寫入、reactor 還沒跑就重建資料庫 → reactor_runs 被清空，答案永遠不會交付
// （INV-13「含每個答案的交付狀態」）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rebuild_before_reactor_still_delivers() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let mut req = base_req("qa5-rb-queued");
    req.source = claude_source(&env, "conv-qa5-rbq", Some(&claude));
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    env.ctx.handle.answer_question("qa5-rb-queued", ui_answer("a"), Utc::now()).await.unwrap();
    // reactor 還沒跑（app 在入列後、消化前崩潰／DB 損毀）。
    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    let r2 = env.reactor_on(&h2, Some(claude), None);
    r2.process_pending().await.unwrap();
    r2.process_due_deliveries(Utc::now() + Duration::seconds(61)).await.unwrap();
    let a = h2.get_question("qa5-rb-queued").await.unwrap().unwrap().answer.unwrap();
    assert_eq!(env.calls().len(), 1, "重建後照常交付一次：calls={:?}", env.calls());
    assert_eq!(a.delivery.map(|d| d.state), Some(DeliveryState::Resumed));
}

// ---------------------------------------------------------------------------
// 迴歸：錢題完整流程（commit → 追加 → 回報）重建後題目一致、不重複交付（應綠）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rebuild_after_money_resume_and_report_is_identical() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    write_fake_executor(&env.ctx);
    let mut req = money_t0_req("qa5-rb-full");
    req.source = claude_source(&env, "conv-qa5-full", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    let aid = env.ctx.handle.answer_question("qa5-rb-full", ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    let reactor = env.reactor(Some(claude.clone()), None);
    reactor.process_pending().await.unwrap();
    env.ctx.handle.report_answer("qa5-rb-full", &aid, ReportStatus::Done, "已確認", Utc::now()).await.unwrap();
    let before = env.ctx.handle.get_question("qa5-rb-full").await.unwrap().unwrap();
    let a = before.answer.clone().unwrap();
    assert_eq!(a.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Resumed));
    assert!(a.execution_result.as_deref().unwrap_or("").starts_with("COMMIT_OK"), "{:?}", a.execution_result);
    let calls_before = env.calls().len();

    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    let after = h2.get_question("qa5-rb-full").await.unwrap().unwrap();
    assert_eq!(after, before, "重建後題目（含錢鏈結果、AI 回饋、交付）一致");
    let r2 = env.reactor_on(&h2, Some(claude), None);
    r2.process_pending().await.unwrap();
    r2.process_due_deliveries(Utc::now() + Duration::minutes(20)).await.unwrap();
    assert_eq!(env.calls().len(), calls_before, "不重複交付");
}

// ---------------------------------------------------------------------------
// 迴歸：ask_report 帳本行與冪等（應綠）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn report_ledger_rows_are_idempotent() {
    let env = Env::new().await;
    let mut req = base_req("qa5-rep-rows");
    req.source = claude_source(&env, "conv-qa5-rows", None);
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    let aid = env.ctx.handle.answer_question("qa5-rep-rows", ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    let count = |rows: &[kunsu_server::ledger::LedgerRow], t: LedgerType| rows.iter().filter(|r| r.ledger_type == t).count();

    let args = json!({"question_id": "qa5-rep-rows", "answer_id": aid, "status": "done", "summary_md": "第一版"});
    let out = mcp(&env.ctx.handle, "ask_report", args.clone()).await.unwrap();
    assert_eq!(out["delivery"]["state"], "picked_up", "{out}");
    let r1 = rows_for(&env.ctx, "qa5-rep-rows");
    assert_eq!((count(&r1, LedgerType::Execution), count(&r1, LedgerType::Delivery)), (1, 1), "回報一行＋交付一行");
    // 帳本最後一行（交付）帶回報與 picked_up；回報那一行的快照還沒有交付狀態（先寫帳本再改交付）。
    let last = r1.last().unwrap();
    assert_eq!(last.ledger_type, LedgerType::Delivery);
    assert!(last.question.answer.as_ref().is_some_and(|a| a.report.is_some()));

    mcp(&env.ctx.handle, "ask_report", args).await.unwrap();
    let r2 = rows_for(&env.ctx, "qa5-rep-rows");
    assert_eq!(r2.len(), r1.len(), "同內容重送不寫帳本");

    let changed = json!({"question_id": "qa5-rep-rows", "answer_id": aid, "status": "failed", "summary_md": "第二版"});
    mcp(&env.ctx.handle, "ask_report", changed).await.unwrap();
    let r3 = rows_for(&env.ctx, "qa5-rep-rows");
    assert_eq!(
        (count(&r3, LedgerType::Execution), count(&r3, LedgerType::Delivery)),
        (2, 1),
        "內容不同視為更新（多一行回報），交付已定案不再多寫"
    );
    let a = env.ctx.handle.get_question("qa5-rep-rows").await.unwrap().unwrap().answer.unwrap();
    assert_eq!(a.report.unwrap().summary_md, "第二版");
    assert_eq!(a.choice_id.as_deref(), Some("a"));
    assert_eq!(a.execution_result, None);

    // 重建後 = 最後快照。
    let before = env.ctx.handle.get_question("qa5-rep-rows").await.unwrap().unwrap();
    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    assert_eq!(h2.get_question("qa5-rep-rows").await.unwrap().unwrap(), before);
}

// ---------------------------------------------------------------------------
// 迴歸：Lane 1 drain（ask_answers_since）保留原貌——簽章與錢鏈原始輸出都在，回報後也一樣（應綠）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn drain_keeps_raw_money_record() {
    let env = Env::new().await;
    let mut req = money_t0_req("qa5-drain");
    req.source = claude_source(&env, "conv-qa5-drain", None);
    env.ctx.handle.create_question(req, now()).await.unwrap();
    let mut input = ui_answer("a");
    input.signature = Some("sig-QA5".into());
    let aid = env.ctx.handle.answer_question("qa5-drain", input, now()).await.unwrap().answer.unwrap().answer_id;
    env.ctx.handle.record_execution("qa5-drain", "APPROVAL_SIGNED: /v/approvals/qa5.json；COMMIT_OK: --token tk-QA5", now()).await.unwrap();
    env.ctx.handle.report_answer("qa5-drain", &aid, ReportStatus::Done, "好", now()).await.unwrap();
    let out = mcp(&env.ctx.handle, "ask_answers_since", json!({"consumer": "lane1"})).await.unwrap().to_string();
    for raw in ["sig-QA5", "tk-QA5", "approvals/qa5.json"] {
        assert!(out.contains(raw), "Lane 1 drain 應保留原貌（{raw}）：{out}");
    }
    let got = mcp(&env.ctx.handle, "ask_get", json!({"question_id": "qa5-drain"})).await.unwrap().to_string();
    for raw in ["sig-QA5", "tk-QA5", "approvals/qa5.json"] {
        assert!(!got.contains(raw), "ask_get 不得回 {raw}");
    }
}

// ---------------------------------------------------------------------------
// 迴歸：held 每 60 秒重檢不追加帳本；交付改變才追加（應綠）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn held_rechecks_do_not_flood_ledger() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let reg = env.hold("conv-qa5-flood");
    let mut req = base_req("qa5-flood");
    req.source = claude_source(&env, "conv-qa5-flood", Some(&claude));
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    env.ctx.handle.answer_question("qa5-flood", ui_answer("a"), Utc::now()).await.unwrap();
    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    let base = rows_for(&env.ctx, "qa5-flood").len();
    for k in 1..=5 {
        reactor.process_due_deliveries(Utc::now() + Duration::seconds(61 * k)).await.unwrap();
    }
    assert_eq!(rows_for(&env.ctx, "qa5-flood").len(), base, "同狀態重檢不追加帳本");
    std::fs::remove_file(&reg).unwrap();
    reactor.process_due_deliveries(Utc::now() + Duration::seconds(61 * 7)).await.unwrap();
    let rows = rows_for(&env.ctx, "qa5-flood");
    let states: Vec<_> = rows
        .iter()
        .filter(|r| r.ledger_type == LedgerType::Delivery)
        .filter_map(|r| r.execution.as_ref().and_then(|e| e.pointer("/delivery/state")).and_then(Value::as_str).map(str::to_string))
        .collect();
    // 只比對次數：測試用未來時間推進 reactor，接近午夜時三行會落在不同日檔，檔案順序不代表寫入順序。
    let mut sorted = states.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["held", "resumed", "resuming"], "交付改變才追加，每個狀態恰好一行：{states:?}");
}

// ---------------------------------------------------------------------------
// 觀察：沒有原對話身分的現行 fresh 題，agent 在 reactor 跑之前 ask_report → 交付記 picked_up，現行 wake 被跳過。
// 規格（場景 3）：沒有原對話身分時維持現行行為。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn report_on_legacy_question_keeps_legacy_wake() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let mut req = base_req("qa5-legacy");
    req.wake_on_answer = kunsu_core::types::Wake { mode: kunsu_core::types::WakeMode::Fresh, session_id: None, playbook: None };
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    let aid = env.ctx.handle.answer_question("qa5-legacy", ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    env.ctx.handle.report_answer("qa5-legacy", &aid, ReportStatus::Done, "我先處理了", Utc::now()).await.unwrap();
    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    let d = env.ctx.handle.get_question("qa5-legacy").await.unwrap().unwrap().answer.unwrap().delivery;
    assert!(
        reactor.planned().iter().any(|c| c.kind == "wake_claude"),
        "沒有原對話身分的題，ask_report 不應改變現行 fresh wake；delivery={d:?}"
    );
}

// ---------------------------------------------------------------------------
// 觀察：`GET /events`（agent 同一把 Bearer token）逐筆轉發 ServerEvent；錢鏈 prepare 成功的 Execution 事件
// line 就是 executor token。INV-25 列舉的讀取面不含 /events，但它同樣是 agent 可讀的面。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn events_stream_never_carries_executor_token() {
    let env = Env::new().await;
    write_fake_executor(&env.ctx);
    let mut req = money_t0_req("qa5-events");
    req.source = claude_source(&env, "conv-qa5-ev", None);
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question("qa5-events", ui_answer("a"), Utc::now()).await.unwrap();
    let mut rx = env.ctx.handle.subscribe();
    let mut cfg = env.cfg(None, None);
    cfg.dry_run = true;
    Reactor::new(env.ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key()))).process_pending().await.unwrap();
    let frames: Vec<String> = drain_events(&mut rx).iter().map(|e| serde_json::to_string(e).unwrap()).collect();
    let leaked: Vec<&String> = frames.iter().filter(|f| f.contains(FAKE_EXECUTOR_TOKEN)).collect();
    assert!(leaked.is_empty(), "/events 會把這些 frame 原樣送給持 agent token 的訂閱者：{leaked:?}");
}

// ---------------------------------------------------------------------------
// N6：錢題選安全選項（錢鏈未觸發）→ 追加成功後 execution_result 只有交付文字 → agent_view 一律換成
// 「已送出，結果見 Kunsu 回執」，和追加訊息裡的「未觸發」互相矛盾。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn safe_option_money_status_is_not_triggered() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", "echo '{\"type\":\"result\"}'");
    let mut req = money_t0_req("qa5-safe");
    req.source = claude_source(&env, "conv-qa5-safe", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question("qa5-safe", ui_answer("b"), Utc::now()).await.unwrap();
    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    let raw = env.ctx.handle.get_question("qa5-safe").await.unwrap().unwrap().answer.unwrap();
    assert_eq!(raw.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Resumed), "前提：已追加");
    let prompt = reactor.planned().iter().find(|c| c.kind == "resume_claude").map(|c| c.args[1].clone()).unwrap();
    assert!(prompt.contains("未觸發"), "前提：追加訊息說錢鏈未觸發：{prompt}");
    let got = mcp(&env.ctx.handle, "ask_get", json!({"question_id": "qa5-safe"})).await.unwrap();
    let status = got["question"]["answer"]["execution_result"].as_str().unwrap_or("").to_string();
    assert!(
        status.contains("未觸發"),
        "安全選項沒有動錢，agent 讀到的錢鏈狀態卻是 {status:?}（原始 execution_result={:?}）",
        raw.execution_result
    );
}

// ---------------------------------------------------------------------------
// N7：帳本降級（vault 寫入逾時，server 照常運作）期間的交付變更不在帳本 → 照降級訊息「授權後可 rebuild-db 回填」
// 去重建 → 交付回到較舊快照（pending）→ 已交給等待者的答案又在原對話追加一次（INV-13／INV-18）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn degraded_ledger_is_backfilled_before_rebuild() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let id = "qa5-degraded";
    let mut req = base_req(id);
    req.source = claude_source(&env, "conv-qa5-deg", Some(&claude));
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    let t = Utc::now();
    env.ctx.handle.waiter_enter(id, t);
    env.ctx.handle.waiter_exit(id, t);
    env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    env.reactor(Some(claude.clone()), None).process_pending().await.unwrap();
    assert_eq!(
        env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap().delivery.map(|d| d.state),
        Some(DeliveryState::Pending),
        "前提：租約內 → pending（帳本有這一行）"
    );

    let day = env.ctx.handle.ledger().day_file(Utc::now());
    let bak = day.with_extension("jsonl.bak");
    std::fs::rename(&day, &bak).unwrap();
    assert!(std::process::Command::new("/usr/bin/mkfifo").arg(&day).status().unwrap().success());
    let got = mcp(&env.ctx.handle, "ask_wait", json!({"question_id": id, "timeout_seconds": 2})).await.unwrap();
    // 權限恢復：卡住的寫入完成（它寫的行已在待補寫佇列，之後補寫，重建以 row_id 去重）。
    drain_fifo(&day);
    std::fs::remove_file(&day).unwrap();
    std::fs::rename(&bak, &day).unwrap();
    assert_eq!(got["delivery"]["state"], "live_waiter", "前提：等待者取走：{got}");
    assert!(env.ctx.discord.messages().iter().any(|m| m.reason == "ledger_degraded"), "前提：帳本降級（只寫進 DB）");
    // 權限恢復：server 的定期 tick 自動補寫積欠的帳本行。
    env.ctx.handle.tick(Utc::now() + Duration::seconds(31)).await.unwrap();
    assert!(env.ctx.discord.messages().iter().any(|m| m.reason == "ledger_recovered"), "補寫完成要告訴使用者");

    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    let r2 = env.reactor_on(&h2, Some(claude), None);
    r2.process_pending().await.unwrap();
    r2.process_due_deliveries(Utc::now() + Duration::seconds(15)).await.unwrap();
    let d = h2.get_question(id).await.unwrap().unwrap().answer.unwrap().delivery;
    assert!(
        env.calls().is_empty(),
        "等待者已取走答案，帳本降級後重建又在原對話追加了一次：重建後交付={d:?} calls={:?}",
        env.calls()
    );
}

// ---------------------------------------------------------------------------
// N3b：同 N3，但讓帳本寫入卡住（仿 2026-08-01 vault 受保護目錄事故：append 在 write gate 內最多卡 5 秒）。
// 回報先開始、reactor 200ms 後才重檢 → 舊實作（回報與 picked_up 同一把鎖）reactor 佔不到交付權；
// 新實作在兩段鎖之間把 gate 交給排隊中的 reactor → 原對話已回報仍被追加一輪。決定性重現。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn report_and_pickup_are_one_write() {
    let env = Env::new().await;
    let locked = env.script("codex-locked.sh", "echo 'Error: thread x already has an active writer' >&2\nexit 1");
    let ok = env.script("codex-ok.sh", OK_BODY);
    let id = "qa5-stall";
    let mut req = base_req(id);
    req.source = codex_source(&env, "019a0000-0000-7000-8000-00000000c0de");
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    let aid = env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    env.reactor(None, Some(locked)).process_pending().await.unwrap();
    assert_eq!(
        env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap().delivery.map(|d| d.state),
        Some(DeliveryState::Held)
    );
    let calls_before = env.calls().len();

    // 帳本日檔換成 FIFO：append 的 open 會卡住直到 5 秒逾時（server 降級照常運作）。
    let day = env.ctx.handle.ledger().day_file(Utc::now());
    std::fs::rename(&day, day.with_extension("jsonl.bak")).unwrap();
    let st = std::process::Command::new("/usr/bin/mkfifo").arg(&day).status().unwrap();
    assert!(st.success());

    let h = env.ctx.handle.clone();
    let aid2 = aid.clone();
    let report = tokio::spawn(async move {
        h.report_answer(id, &aid2, ReportStatus::Done, "heartbeat 處理完", Utc::now()).await.unwrap()
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let r = env.reactor(None, Some(ok));
    let r_probe = r.clone();
    let t0 = std::time::Instant::now();
    let due = tokio::spawn(async move {
        let n = r.process_due_deliveries(Utc::now() + Duration::seconds(61)).await.unwrap();
        eprintln!("QA5 stall: reactor done after {:?}, processed {n}", t0.elapsed());
    });
    let reported = report.await.unwrap();
    eprintln!("QA5 stall: report done after {:?}", t0.elapsed());
    due.await.unwrap();
    eprintln!("QA5 stall: planned={:?}", r_probe.planned().iter().map(|c| c.kind.clone()).collect::<Vec<_>>());
    let _ = std::fs::remove_file(&day);

    let a = env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap();
    let appended = env.calls().len() - calls_before;
    assert!(reported.answer.as_ref().is_some_and(|a| a.report.is_some()), "前提：回報已記錄");
    assert!(
        appended == 0 && a.delivery.as_ref().map(|d| d.state) == Some(DeliveryState::PickedUp),
        "原對話已先 ask_report（heartbeat 取走），reactor 仍在原對話追加了 {appended} 輪；最終交付={:?}",
        a.delivery
    );
}

/// INV-13：帳本積欠（待補寫佇列）補不進去時拒絕重建，不丟掉只在資料庫裡的變更。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rebuild_refuses_while_ledger_backlog_cannot_flush() {
    let env = Env::new().await;
    let id = "rb-backlog";
    env.ctx.handle.create_question(base_req(id), Utc::now()).await.unwrap();
    let day = env.ctx.handle.ledger().day_file(Utc::now());
    let _ = std::fs::rename(&day, day.with_extension("jsonl.bak"));
    assert!(std::process::Command::new("/usr/bin/mkfifo").arg(&day).status().unwrap().success());
    env.ctx.handle.cancel_question(id, Utc::now()).await.unwrap();
    assert!(env.ctx.discord.messages().iter().any(|m| m.reason == "ledger_degraded"), "前提：帳本寫不進去");
    let err = env.ctx.handle.rebuild_db().await;
    let _ = std::fs::remove_file(&day);
    assert!(err.is_err(), "帳本還有沒補寫的變更時不能重建：{err:?}");
    assert_eq!(env.ctx.handle.get_question(id).await.unwrap().unwrap().state, kunsu_core::types::State::Cancelled, "資料庫沒被清掉");
}

/// INV-13／AC-CC-11：錢題 approval 已消耗卻沒有執行紀錄就重建 → 不重新跑錢鏈，改發 Discord 待人工對帳。
#[tokio::test]
async fn rebuild_never_reruns_money_whose_approval_was_consumed() {
    let env = Env::new().await;
    write_fake_executor(&env.ctx);
    let id = "rb-money-consumed";
    let mut req = money_t0_req(id);
    req.source = claude_source(&env, "conv-rb-mc", None);
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    let record = json!({"question_id": id, "nonce": "n-rb-1", "param_hash": "h"});
    env.ctx.handle.consume_approval(id, "n-rb-1", &record, Utc::now()).await.unwrap();
    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    let report = h2.rebuild_db().await.unwrap();
    assert_eq!((report.reconcile, report.requeued), (1, 0), "待人工對帳（同時發 Discord），不重新排隊");
    let r2 = env.reactor_on(&h2, None, None);
    r2.process_pending().await.unwrap();
    assert!(!r2.planned().iter().any(|c| c.kind == "commit" || c.kind == "prepare"), "不重跑錢鏈：{:?}", r2.planned());
}

/// AC-CC-14：已執行完成（COMMIT_OK）的錢題拒絕重新授權。
#[tokio::test]
async fn executed_money_refuses_reauthorization() {
    let env = Env::new().await;
    write_fake_executor(&env.ctx);
    let id = "reauth-done";
    let claude = env.script("claude.sh", OK_BODY);
    let mut req = money_t0_req(id);
    req.source = claude_source(&env, "conv-reauth", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    env.reactor(Some(claude), None).process_pending().await.unwrap();
    let a = env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap();
    assert!(a.execution_result.as_deref().unwrap_or("").starts_with("COMMIT_OK"), "前提：{:?}", a.execution_result);
    assert!(env.ctx.handle.reauthorize_question(id, Utc::now()).await.is_err(), "錢已經動了，不能再授權一次");
}
