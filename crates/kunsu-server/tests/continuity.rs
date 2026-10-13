//! 回答延續原對話（BDD conversation-continuity，AC-CC-01…14）的 server 整合測試。
//! 假 claude/codex 腳本記錄被呼叫的參數；登記檔、桌面 session 紀錄都在 tempdir。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration, Utc};
use common::*;
use kunsu_core::continuity::{DeliveryState, HostKind, ReportStatus};
use kunsu_core::types::{AgentKind, Source};
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
        std::fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
    /// 假 CLI：把參數寫進 calls.log，依參數決定輸出。
    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.root.path().join(name);
        let log = self.log();
        // 一次呼叫一行（prompt 可能多行，先壓成一行再記）。
        let content = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$(printf '%s ' \"$@\" | tr '\\n' ' ')\" >> {}\n{body}\n",
            log.display()
        );
        std::fs::write(&path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }
    fn reactor(&self, claude: Option<PathBuf>, codex: Option<PathBuf>) -> Arc<Reactor> {
        let mut cfg = ReactorConfig::new(self.ctx.vault.path().to_path_buf());
        cfg.claude_bin = claude;
        cfg.codex_bin = codex;
        cfg.sessions_dir = self.registry();
        cfg.desktop_sessions_dir = self.root.path().join("desktop");
        cfg.retry_base_ms = 1;
        cfg.trusted_cli_roots.push(self.root.path().to_path_buf());
        Reactor::new(self.ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key())))
    }
}

fn claude_source(env: &Env, conversation: &str, cli_path: Option<&Path>) -> Source {
    Source {
        agent: AgentKind::Claude,
        cwd: env.work().to_string_lossy().to_string(),
        host: Some(HostKind::ClaudeDesktop),
        conversation_id: Some(conversation.into()),
        host_session_id: Some("local_xyz-1".into()),
        attended: Some(false),
        cli_path: cli_path.map(|p| p.to_string_lossy().to_string()),
        routine_id: Some("daily-ads".into()),
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

async fn create(handle: &ServerHandle, id: &str, source: Source) {
    let mut req = base_req(id);
    req.source = source;
    handle.create_question(req, Utc::now()).await.unwrap();
}

async fn answer(handle: &ServerHandle, id: &str) -> String {
    let q = handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    q.answer.unwrap().answer_id
}

async fn delivery(handle: &ServerHandle, id: &str) -> Option<DeliveryState> {
    handle
        .get_question(id)
        .await
        .unwrap()
        .unwrap()
        .answer
        .and_then(|a| a.delivery)
        .map(|d| d.state)
}

async fn mcp(handle: &ServerHandle, name: &str, arguments: Value) -> Result<Value, Value> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
    match handle_rpc(handle, body).await {
        McpReply::Json(v) => {
            if v.get("error").is_some() {
                Err(v["error"].clone())
            } else {
                Ok(v["result"]["structuredContent"].clone())
            }
        }
        McpReply::Accepted => panic!("tools/call 不得回 Accepted"),
    }
}

const OK_BODY: &str = "echo 'EXEC_OK: 原對話已接手'";

// ---------------------------------------------------------------------------
// AC-CC-04 / AC-CC-11：等待者在線 → 直接交付，reactor 不追加、不開新對話
// ---------------------------------------------------------------------------

/// INV：同一答案只有一條交付路徑生效（等待者取走後永不再 resume／開新對話）。
#[tokio::test]
async fn inv_single_continuation() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    create(&env.ctx.handle, "cc-live", claude_source(&env, "conv-live", Some(&claude))).await;

    let h = env.ctx.handle.clone();
    let waiter = tokio::spawn(async move {
        mcp(&h, "ask_wait", json!({"question_id": "cc-live", "timeout_seconds": 30})).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    answer(&env.ctx.handle, "cc-live").await;
    let got = waiter.await.unwrap().unwrap();
    assert_eq!(got["status"], "answered");

    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    reactor.process_due_deliveries(Utc::now() + Duration::minutes(5)).await.unwrap();

    assert!(env.calls().is_empty(), "等待者已取走答案，不得再 resume 或開新對話：{:?}", env.calls());
    assert_eq!(delivery(&env.ctx.handle, "cc-live").await, Some(DeliveryState::LiveWaiter));
    let q = env.ctx.handle.get_question("cc-live").await.unwrap().unwrap();
    assert!(q.answer.unwrap().acked_at.is_some(), "交給等待者即算 acked");
}

// ---------------------------------------------------------------------------
// AC-CC-05：兩輪 long-poll 之間的 10 秒租約
// ---------------------------------------------------------------------------

#[tokio::test]
async fn waiter_lease_covers_gap_between_polls() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    create(&env.ctx.handle, "cc-lease", claude_source(&env, "conv-lease", Some(&claude))).await;

    let now = Utc::now();
    env.ctx.handle.waiter_enter("cc-lease", now);
    env.ctx.handle.waiter_exit("cc-lease", now); // 剛結束一輪 pending_timeout
    answer(&env.ctx.handle, "cc-lease").await;

    let reactor = env.reactor(Some(claude.clone()), None);
    reactor.process_pending().await.unwrap();
    assert!(env.calls().is_empty(), "租約內不得 resume");
    assert_eq!(delivery(&env.ctx.handle, "cc-lease").await, Some(DeliveryState::Pending));

    // 下一輪 ask_wait 立刻拿到答案並認領。
    let got = mcp(&env.ctx.handle, "ask_wait", json!({"question_id": "cc-lease", "timeout_seconds": 5}))
        .await
        .unwrap();
    assert_eq!(got["status"], "answered");
    reactor.process_due_deliveries(Utc::now() + Duration::seconds(30)).await.unwrap();
    assert!(env.calls().is_empty());
    assert_eq!(delivery(&env.ctx.handle, "cc-lease").await, Some(DeliveryState::LiveWaiter));

    // 租約過期沒人取走 → 進下一階（追加）。
    create(&env.ctx.handle, "cc-lease-2", claude_source(&env, "conv-lease-2", Some(&claude))).await;
    env.ctx.handle.waiter_enter("cc-lease-2", now - Duration::seconds(20));
    env.ctx.handle.waiter_exit("cc-lease-2", now - Duration::seconds(11));
    answer(&env.ctx.handle, "cc-lease-2").await;
    reactor.process_pending().await.unwrap();
    assert_eq!(delivery(&env.ctx.handle, "cc-lease-2").await, Some(DeliveryState::Resumed));
}

// ---------------------------------------------------------------------------
// AC-CC-06 / AC-CC-08 / AC-CC-13：原對話已結束 → 用桌面最新 CLI id 在原對話追加
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ended_claude_conversation_is_resumed_in_place() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    std::fs::write(
        env.root.path().join("desktop/acct/org/local_xyz-1.json"),
        json!({"sessionId": "local_xyz-1", "cliSessionId": "conv-new", "priorCliSessionIds": ["conv-old"]}).to_string(),
    )
    .unwrap();
    create(&env.ctx.handle, "cc-resume", claude_source(&env, "conv-old", Some(&claude))).await;
    let answer_id = answer(&env.ctx.handle, "cc-resume").await;

    let reactor = env.reactor(None, None);
    reactor.process_pending().await.unwrap();

    let planned = reactor.planned();
    let cmd = planned.iter().find(|c| c.kind == "resume_claude").expect("應在原對話追加");
    assert_eq!(cmd.program, claude.canonicalize().unwrap().to_string_lossy(), "用發問 session 的 CLI（解析後真實路徑）");
    assert_eq!(cmd.cwd, env.work().to_string_lossy(), "cwd = 原對話的工作目錄");
    let i = cmd.args.iter().position(|a| a == "--resume").expect("--resume");
    assert_eq!(cmd.args[i + 1], "conv-new", "以 host_session_id 查到的最新 CLI id");
    let p = cmd.args.iter().position(|a| a == "-p").expect("-p");
    let prompt = &cmd.args[p + 1];
    for needle in ["要不要加碼？", answer_id.as_str(), "ask_get", "ask_report", "加碼 20%"] {
        assert!(prompt.contains(needle), "追加訊息缺 {needle}：{prompt}");
    }
    assert!(cmd.scrub_env_prefixes.iter().any(|p| p == "CLAUDE_CODE_"), "清掉繼承的 CLAUDE_CODE_*");
    assert!(env.calls().iter().any(|c| c.contains("--resume conv-new")), "真的執行了：{:?}", env.calls());

    let q = env.ctx.handle.get_question("cc-resume").await.unwrap().unwrap();
    let d = q.answer.unwrap().delivery.expect("delivery");
    assert_eq!(d.state, DeliveryState::Resumed);
    assert_eq!(d.conversation_link.as_deref(), Some("claude://code/continue?session=local_xyz-1"));
}

// ---------------------------------------------------------------------------
// AC-CC-07：原對話仍被活行程持有 → held；釋放後才追加
// ---------------------------------------------------------------------------

/// INV：永不對仍被活行程持有的原對話做背景追加（會分岔、答案被覆蓋）。
#[tokio::test]
async fn inv_never_resume_live_conversation() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let reg = env.registry().join(format!("{}.json", std::process::id()));
    std::fs::write(&reg, json!({"pid": std::process::id(), "sessionId": "conv-held"}).to_string()).unwrap();
    create(&env.ctx.handle, "cc-held", claude_source(&env, "conv-held", Some(&claude))).await;
    answer(&env.ctx.handle, "cc-held").await;

    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    assert!(env.calls().is_empty(), "活著的對話不得被背景追加（會分岔）");
    assert_eq!(delivery(&env.ctx.handle, "cc-held").await, Some(DeliveryState::Held));

    reactor.process_due_deliveries(Utc::now() + Duration::seconds(30)).await.unwrap();
    assert!(env.calls().is_empty(), "60 秒內不重檢");

    std::fs::remove_file(&reg).unwrap();
    reactor.process_due_deliveries(Utc::now() + Duration::seconds(61)).await.unwrap();
    assert_eq!(env.calls().len(), 1, "釋放後追加一次：{:?}", env.calls());
    assert_eq!(delivery(&env.ctx.handle, "cc-held").await, Some(DeliveryState::Resumed));
    reactor.process_due_deliveries(Utc::now() + Duration::seconds(200)).await.unwrap();
    assert_eq!(env.calls().len(), 1, "最終狀態後不再重做");
}

// ---------------------------------------------------------------------------
// AC-CC-07（Codex）/ AC-CC-09：thread 被 app 鎖住 → held；heartbeat 回報即 picked_up
// ---------------------------------------------------------------------------

#[tokio::test]
async fn codex_locked_thread_is_held_and_report_picks_it_up() {
    let env = Env::new().await;
    let codex = env.script(
        "codex.sh",
        "echo 'Error: thread t-1 already has an active writer' >&2\nexit 1",
    );
    create(&env.ctx.handle, "cc-codex", codex_source(&env, "019a0000-0000-7000-8000-000000000001")).await;
    let answer_id = answer(&env.ctx.handle, "cc-codex").await;

    let reactor = env.reactor(None, Some(codex));
    reactor.process_pending().await.unwrap();
    let planned = reactor.planned();
    let cmd = planned.iter().find(|c| c.kind == "resume_codex").expect("codex 追加命令");
    let cwd = env.work().to_string_lossy().to_string();
    assert_eq!(
        cmd.args,
        vec![
            "exec".to_string(),
            "-s".into(),
            "workspace-write".into(),
            "-C".into(),
            cwd,
            "--json".into(),
            "--skip-git-repo-check".into(),
            "resume".into(),
            "019a0000-0000-7000-8000-000000000001".into(),
            "-".into(),
        ]
    );
    assert!(cmd.stdin.as_deref().unwrap_or("").contains(&answer_id), "prompt 走 stdin 並帶 answer_id");
    assert_eq!(delivery(&env.ctx.handle, "cc-codex").await, Some(DeliveryState::Held), "被鎖 → held，不是死信");
    assert!(env.ctx.discord.messages().is_empty(), "held 不是失敗，不發死信告警");

    // heartbeat 在同一 thread 讀到答案並回報。
    mcp(
        &env.ctx.handle,
        "ask_report",
        json!({"question_id": "cc-codex", "answer_id": answer_id, "status": "done", "summary_md": "已調整"}),
    )
    .await
    .unwrap();
    assert_eq!(delivery(&env.ctx.handle, "cc-codex").await, Some(DeliveryState::PickedUp));
    let before = env.calls().len();
    reactor.process_due_deliveries(Utc::now() + Duration::seconds(61)).await.unwrap();
    assert_eq!(env.calls().len(), before, "picked_up 後停止重試");
}

// ---------------------------------------------------------------------------
// AC-CC-10：原對話不存在 → 開新對話（附原對話連結與原因）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_conversation_falls_back_to_fresh_with_link() {
    let env = Env::new().await;
    let claude = env.script(
        "claude.sh",
        "case \"$*\" in *--resume*) echo 'No conversation found with session ID: conv-gone' >&2; exit 1;; esac\necho 'EXEC_OK: 新對話處理完成'",
    );
    create(&env.ctx.handle, "cc-gone", claude_source(&env, "conv-gone", Some(&claude))).await;
    answer(&env.ctx.handle, "cc-gone").await;

    let reactor = env.reactor(Some(claude.clone()), None);
    reactor.process_pending().await.unwrap();

    let calls = env.calls();
    assert_eq!(calls.len(), 2, "先試追加一次（找不到不重試），再開新對話：{calls:?}");
    assert!(calls[0].contains("--resume"));
    assert!(!calls[1].contains("--resume"));
    let planned = reactor.planned();
    let fresh = planned.iter().find(|c| c.kind == "wake_claude").expect("fresh 命令");
    // 開新對話的 prompt 走 stdin（CONTRACT §8 第 5 步）。
    let prompt = fresh.stdin.as_deref().expect("prompt 走 stdin");
    assert!(prompt.contains("claude://code/continue?session=local_xyz-1"), "新對話附原對話連結");

    let q = env.ctx.handle.get_question("cc-gone").await.unwrap().unwrap();
    let a = q.answer.unwrap();
    let d = a.delivery.unwrap();
    assert_eq!(d.state, DeliveryState::Fresh);
    assert!(d.reason.unwrap_or_default().contains("原對話不存在"));
    assert_eq!(a.execution_result.as_deref(), Some("EXEC_OK: 新對話處理完成"));
}

// ---------------------------------------------------------------------------
// AC-CC-03：沒有原對話身分 → 現行行為並在交付紀錄說明
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_conversation_keeps_existing_behavior() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let mut req = base_req("cc-plain");
    req.source.cwd = env.work().to_string_lossy().to_string();
    req.wake_on_answer = kunsu_core::types::Wake {
        mode: kunsu_core::types::WakeMode::Fresh,
        session_id: None,
        playbook: None,
    };
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    answer(&env.ctx.handle, "cc-plain").await;
    create(&env.ctx.handle, "cc-none", Source { cwd: "/x".into(), ..Default::default() }).await;
    answer(&env.ctx.handle, "cc-none").await;

    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    assert!(reactor.planned().iter().any(|c| c.kind == "wake_claude" && c.question_id == "cc-plain"));
    let q = env.ctx.handle.get_question("cc-plain").await.unwrap().unwrap();
    let d = q.answer.unwrap().delivery.unwrap();
    assert_eq!(d.state, DeliveryState::Fresh);
    assert_eq!(d.reason.as_deref(), Some("沒有原對話可接續"));
    assert_eq!(delivery(&env.ctx.handle, "cc-none").await, Some(DeliveryState::NoContinuation));
}

// ---------------------------------------------------------------------------
// AC-CC-12：ask_report 記錄 AI 回饋，但永遠不改答案
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ask_report_records_feedback_without_touching_answer() {
    let env = Env::new().await;
    create(&env.ctx.handle, "cc-report", claude_source(&env, "conv-r", None)).await;

    let err = mcp(
        &env.ctx.handle,
        "ask_report",
        json!({"question_id": "cc-report", "answer_id": "x", "status": "done", "summary_md": "s"}),
    )
    .await;
    assert!(err.is_err(), "未作答的題不能回報");

    let answer_id = answer(&env.ctx.handle, "cc-report").await;
    let wrong = mcp(
        &env.ctx.handle,
        "ask_report",
        json!({"question_id": "cc-report", "answer_id": "not-this", "status": "done", "summary_md": "s"}),
    )
    .await;
    assert!(wrong.is_err(), "answer_id 不符必須拒絕");

    let args = json!({
        "question_id": "cc-report", "answer_id": answer_id, "status": "done",
        "summary_md": "**已寫入日記**，明天建議 23:00 前收工"
    });
    mcp(&env.ctx.handle, "ask_report", args.clone()).await.unwrap();
    let q = env.ctx.handle.get_question("cc-report").await.unwrap().unwrap();
    let a = q.answer.clone().unwrap();
    assert_eq!(a.choice_id.as_deref(), Some("a"), "答案本身不變");
    assert_eq!(q.state, kunsu_core::types::State::Answered);
    let report = a.report.expect("report");
    assert_eq!(report.status, ReportStatus::Done);
    assert!(report.summary_md.contains("已寫入日記"));
    assert_eq!(a.execution_result, None, "回報永不寫 Kunsu 自己的執行紀錄（AC-CC-14：錢鏈結果與 AI 回饋分開）");
    assert!(a.executed_at.is_some() && a.acked_at.is_some());

    let rows_before = read_ledger_rows(&env.ctx, Utc::now()).len();
    mcp(&env.ctx.handle, "ask_report", args).await.unwrap();
    assert_eq!(read_ledger_rows(&env.ctx, Utc::now()).len(), rows_before, "同內容重送冪等，不重複寫帳本");

    let specs = kunsu_server::mcp::tool_specs();
    assert_eq!(specs.len(), 14);
    assert!(specs.iter().any(|s| s["name"] == "ask_report"));
    assert!(!specs.iter().any(|s| s["name"] == "answer"), "MCP 面仍沒有作答方法");
}

// ---------------------------------------------------------------------------
// AC-CC-14：錢題接續的訊息不含任何授權秘密
// ---------------------------------------------------------------------------

#[tokio::test]
async fn money_continuation_prompt_has_no_approval_secrets() {
    let env = Env::new().await;
    let mut req = money_t0_req("cc-money");
    req.source = claude_source(&env, "conv-money", None);
    env.ctx.handle.create_question(req, now()).await.unwrap();
    let mut input = ui_answer("a");
    input.signature = Some("sig-SECRET-123".into());
    env.ctx.handle.answer_question("cc-money", input, now()).await.unwrap();
    let q = env.ctx.handle.get_question("cc-money").await.unwrap().unwrap();

    let prompt = kunsu_server::reactor::continuation_prompt(
        &q,
        Some("APPROVAL_SIGNED: /vault/executor_pending/approvals/cc-money.json；COMMIT_OK: token=abc nonce=n1"),
    );
    for secret in ["sig-SECRET-123", "approvals/cc-money.json", "nonce", "token=abc", "executor.py"] {
        assert!(!prompt.contains(secret), "追加訊息洩漏 {secret}：{prompt}");
    }
    assert!(prompt.contains("Kunsu 錢鏈"), "要說明由 Kunsu 錢鏈執行");
}

// ---------------------------------------------------------------------------
// AC-CC-02：MCP 直接傳原對話身分同樣受驗證
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mcp_accepts_identity_fields_and_rejects_unknown_host() {
    let env = Env::new().await;
    let mut payload = serde_json::to_value(base_req("cc-mcp")).unwrap();
    payload["source"] = json!({
        "agent": "codex", "cwd": "/w", "host": "codex_app", "conversation_id": "019a0000-0000-7000-8000-000000000003",
        "trigger": "scheduled", "routine_id": "r1", "routine_name": "日記", "expected_every_hours": 24
    });
    payload["requested_tier"] = payload["tier"].take();
    payload.as_object_mut().unwrap().remove("tier");
    mcp(&env.ctx.handle, "ask_create", payload.clone()).await.unwrap();
    let q = env.ctx.handle.get_question("cc-mcp").await.unwrap().unwrap();
    assert_eq!(q.source.conversation_id.as_deref(), Some("019a0000-0000-7000-8000-000000000003"));
    assert_eq!(q.wake_on_answer.mode, kunsu_core::types::WakeMode::Continue, "有原對話 → 預設接續");

    payload["id"] = json!("cc-mcp-bad");
    payload["source"]["host"] = json!("vscode");
    assert!(mcp(&env.ctx.handle, "ask_create", payload).await.is_err());
}

// ---------------------------------------------------------------------------
// QA 修正（2026-10-08 獨立 QA FAIL）
// ---------------------------------------------------------------------------

/// #1：等待者連線被切斷（排程結束殺掉 `kunsu wait`）也要釋放；租約過後照常追加。
#[tokio::test]
async fn killed_waiter_releases_and_answer_is_resumed() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    create(&env.ctx.handle, "cc-killed", claude_source(&env, "conv-killed", Some(&claude))).await;
    let h = env.ctx.handle.clone();
    let waiter = tokio::spawn(async move {
        mcp(&h, "ask_wait", json!({"question_id": "cc-killed", "timeout_seconds": 300})).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    waiter.abort(); // 排程 session 結束，背景等待被殺
    let _ = waiter.await;
    let (connected, _) = env.ctx.handle.waiter_status("cc-killed", Utc::now());
    assert!(!connected, "被切斷的等待者不能永遠算在線");

    answer(&env.ctx.handle, "cc-killed").await;
    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    reactor.process_due_deliveries(Utc::now() + Duration::seconds(15)).await.unwrap();
    assert_eq!(delivery(&env.ctx.handle, "cc-killed").await, Some(DeliveryState::Resumed));
}

/// #2：追加的那一輪照指示呼叫 ask_report，交付仍記為 resumed（先佔交付權再 spawn）。
#[tokio::test]
async fn resumed_agent_reporting_mid_run_keeps_resumed() {
    let env = Env::new().await;
    let slow = env.script("claude.sh", "sleep 1\necho 'EXEC_OK: 原對話已接手'");
    create(&env.ctx.handle, "cc-mid", claude_source(&env, "conv-mid", Some(&slow))).await;
    let answer_id = answer(&env.ctx.handle, "cc-mid").await;
    let reactor = env.reactor(Some(slow), None);
    let r = reactor.clone();
    let run = tokio::spawn(async move { r.process_pending().await });
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(delivery(&env.ctx.handle, "cc-mid").await, Some(DeliveryState::Resuming), "spawn 前先佔交付權");
    mcp(
        &env.ctx.handle,
        "ask_report",
        json!({"question_id": "cc-mid", "answer_id": answer_id, "status": "done", "summary_md": "已處理"}),
    )
    .await
    .unwrap();
    let late = mcp(&env.ctx.handle, "ask_wait", json!({"question_id": "cc-mid", "timeout_seconds": 2})).await.unwrap();
    run.await.unwrap().unwrap();
    assert_eq!(delivery(&env.ctx.handle, "cc-mid").await, Some(DeliveryState::Resumed));
    assert!(late["delivery"]["state"].is_string(), "遲到的等待者要看得到答案已交付到哪裡：{late}");
    let q = env.ctx.handle.get_question("cc-mid").await.unwrap().unwrap();
    assert_eq!(q.answer.unwrap().report.unwrap().summary_md, "已處理");
}

/// #3：舊式 wake {mode: resume, session_id} 仍接續那段對話，不退化成開新對話。
#[tokio::test]
async fn legacy_resume_wake_keeps_its_conversation() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let mut req = base_req("cc-legacy");
    req.source.cwd = env.work().to_string_lossy().to_string();
    req.wake_on_answer = kunsu_core::types::Wake {
        mode: kunsu_core::types::WakeMode::Resume,
        session_id: Some("legacy-sid-1".into()),
        playbook: None,
    };
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    let q = env.ctx.handle.get_question("cc-legacy").await.unwrap().unwrap();
    assert_eq!(q.source.conversation_id.as_deref(), Some("legacy-sid-1"));
    answer(&env.ctx.handle, "cc-legacy").await;
    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    assert!(reactor.planned().iter().any(|c| c.kind == "resume_claude" && c.args.contains(&"legacy-sid-1".to_string())));
}

/// #4：held 不算有人接手，24 小時「你答的 X 沒人處理」告警才觸發得到。
#[tokio::test]
async fn held_answers_stay_unacked_for_the_alarm() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    std::fs::write(
        env.registry().join(format!("{}.json", std::process::id())),
        json!({"pid": std::process::id(), "sessionId": "conv-held-ack"}).to_string(),
    )
    .unwrap();
    create(&env.ctx.handle, "cc-held-ack", claude_source(&env, "conv-held-ack", Some(&claude))).await;
    answer(&env.ctx.handle, "cc-held-ack").await;
    env.reactor(Some(claude), None).process_pending().await.unwrap();
    let q = env.ctx.handle.get_question("cc-held-ack").await.unwrap().unwrap();
    let a = q.answer.unwrap();
    assert_eq!(a.delivery.unwrap().state, DeliveryState::Held);
    assert!(a.acked_at.is_none(), "held 不是接手");
    assert!(a.execution_result.is_none(), "暫時狀態不寫 execution");
}

/// #5：開新對話給 agent 的題目 JSON 去掉簽章、prepare 命令與錢鏈原始輸出；MCP 讀取面也不回簽章。
#[tokio::test]
async fn agent_facing_question_json_is_redacted() {
    let env = Env::new().await;
    let mut req = money_t0_req("cc-redact");
    req.source = claude_source(&env, "conv-redact", None);
    env.ctx.handle.create_question(req, now()).await.unwrap();
    let mut input = ui_answer("a");
    input.signature = Some("sig-SECRET-9".into());
    env.ctx.handle.answer_question("cc-redact", input, now()).await.unwrap();
    env.ctx
        .handle
        .record_execution("cc-redact", "APPROVAL_SIGNED: /v/approvals/x.json；COMMIT_OK: --token tk-1", now())
        .await
        .unwrap();
    let q = env.ctx.handle.get_question("cc-redact").await.unwrap().unwrap();
    let json_text = kunsu_server::reactor::agent_question_json(&q);
    for secret in ["sig-SECRET-9", "executor.py", "approvals/x.json", "tk-1"] {
        assert!(!json_text.contains(secret), "agent 看到 {secret}：{json_text}");
    }
    let got = mcp(&env.ctx.handle, "ask_get", json!({"question_id": "cc-redact"})).await.unwrap();
    assert!(!got.to_string().contains("sig-SECRET-9"), "ask_get 不回簽章");
}

/// #11：沒有原對話身分的題，等待者不改變現行 fresh 行為（結果不取決於誰先跑）。
#[tokio::test]
async fn waiter_does_not_suppress_legacy_fresh_wake() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let mut req = base_req("cc-legacy-fresh");
    req.wake_on_answer = kunsu_core::types::Wake { mode: kunsu_core::types::WakeMode::Fresh, session_id: None, playbook: None };
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    let h = env.ctx.handle.clone();
    let waiter = tokio::spawn(async move {
        mcp(&h, "ask_wait", json!({"question_id": "cc-legacy-fresh", "timeout_seconds": 10})).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    answer(&env.ctx.handle, "cc-legacy-fresh").await;
    waiter.await.unwrap().unwrap();
    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    assert!(reactor.planned().iter().any(|c| c.kind == "wake_claude"), "現行 fresh 照跑");
}

// ---------------------------------------------------------------------------
// QA 第二輪（2026-10-08）
// ---------------------------------------------------------------------------

/// B／C：追加那一輪已經開始處理（有 assistant 事件），非零結束也不重試、不開新對話；工作內容裡的
/// 「找不到」「被鎖」字樣不影響判斷。
#[tokio::test]
async fn started_turn_is_not_retried_or_refreshed() {
    let env = Env::new().await;
    let body = r#"printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"No conversation found with session ID: x; already has an active writer"}]}}'
echo 'API overloaded' >&2
exit 1"#;
    let claude = env.script("claude.sh", body);
    create(&env.ctx.handle, "cc-started", claude_source(&env, "conv-started", Some(&claude))).await;
    answer(&env.ctx.handle, "cc-started").await;
    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    assert_eq!(env.calls().len(), 1, "只追加一次：{:?}", env.calls());
    assert!(!reactor.planned().iter().any(|c| c.kind == "wake_claude"), "不開新對話");
    let q = env.ctx.handle.get_question("cc-started").await.unwrap().unwrap();
    let d = q.answer.unwrap().delivery.unwrap();
    assert_eq!(d.state, DeliveryState::Resumed);
    assert!(d.reason.as_deref().is_some_and(|r| r.contains("出錯")), "回執說明處理到一半出錯：{d:?}");
    assert!(env.ctx.discord.messages().iter().any(|m| m.reason == "resume_partial"), "告訴使用者");
}

/// B：追加中 agent 已 ask_report，之後非零結束 → 仍記 resumed，不重試、不開新對話。
#[tokio::test]
async fn reported_then_failed_turn_stays_resumed() {
    let env = Env::new().await;
    let slow = env.script("claude.sh", "sleep 1\nexit 1");
    create(&env.ctx.handle, "cc-rep-fail", claude_source(&env, "conv-rep-fail", Some(&slow))).await;
    let answer_id = answer(&env.ctx.handle, "cc-rep-fail").await;
    let reactor = env.reactor(Some(slow), None);
    let r = reactor.clone();
    let run = tokio::spawn(async move { r.process_pending().await });
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    mcp(
        &env.ctx.handle,
        "ask_report",
        json!({"question_id": "cc-rep-fail", "answer_id": answer_id, "status": "done", "summary_md": "做完了"}),
    )
    .await
    .unwrap();
    run.await.unwrap().unwrap();
    assert_eq!(env.calls().len(), 1, "不重試：{:?}", env.calls());
    assert_eq!(delivery(&env.ctx.handle, "cc-rep-fail").await, Some(DeliveryState::Resumed));
}

/// C：Claude 的 stderr 出現 Codex 的「被鎖」字樣不是 held。
#[tokio::test]
async fn claude_lock_text_is_not_held() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", "echo 'thread x already has an active writer' >&2\nexit 1");
    create(&env.ctx.handle, "cc-not-locked", claude_source(&env, "conv-not-locked", Some(&claude))).await;
    answer(&env.ctx.handle, "cc-not-locked").await;
    env.reactor(Some(claude), None).process_pending().await.unwrap();
    assert_ne!(delivery(&env.ctx.handle, "cc-not-locked").await, Some(DeliveryState::Held));
}

/// A：drain 收割不能把還沒交到原對話的答案標成已接手；24 小時告警照常觸發。
#[tokio::test]
async fn drain_does_not_ack_undelivered_continuation() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    std::fs::write(
        env.registry().join(format!("{}.json", std::process::id())),
        json!({"pid": std::process::id(), "sessionId": "conv-drain"}).to_string(),
    )
    .unwrap();
    create(&env.ctx.handle, "cc-drain", claude_source(&env, "conv-drain", Some(&claude))).await;
    answer(&env.ctx.handle, "cc-drain").await;
    env.reactor(Some(claude), None).process_pending().await.unwrap();
    assert_eq!(delivery(&env.ctx.handle, "cc-drain").await, Some(DeliveryState::Held));

    env.ctx.handle.answers_since("lane1", None, Utc::now()).await.unwrap();
    let q = env.ctx.handle.get_question("cc-drain").await.unwrap().unwrap();
    assert!(q.answer.unwrap().acked_at.is_none(), "drain 不算接手");
    env.ctx.handle.tick(Utc::now() + Duration::hours(25)).await.unwrap();
    assert!(
        env.ctx.discord.messages().iter().any(|m| m.reason == "no_ack_24h" && m.question_id.as_deref() == Some("cc-drain")),
        "24 小時沒人處理要告警"
    );
}

/// D：卡在「追加中」但 agent 已回報 → resumed，不標 dead、不發 Discord；沒回報的只告警一次。
#[tokio::test]
async fn stuck_resuming_respects_reports_and_alerts_once() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    for id in ["cc-stuck-rep", "cc-stuck-none"] {
        create(&env.ctx.handle, id, claude_source(&env, &format!("conv-{id}"), Some(&claude))).await;
    }
    let t0 = Utc::now();
    let reported = answer(&env.ctx.handle, "cc-stuck-rep").await;
    let silent = answer(&env.ctx.handle, "cc-stuck-none").await;
    for (id, aid) in [("cc-stuck-rep", &reported), ("cc-stuck-none", &silent)] {
        assert!(env.ctx.handle.set_delivery(id, aid, DeliveryState::Resuming, None, None, t0).await.unwrap());
    }
    env.ctx.handle.report_answer("cc-stuck-rep", &reported, ReportStatus::Done, "處理好了", t0).await.unwrap();
    let reactor = env.reactor(Some(claude), None);
    reactor.process_due_deliveries(t0 + Duration::minutes(16)).await.unwrap();
    reactor.process_due_deliveries(t0 + Duration::minutes(17)).await.unwrap();
    assert_eq!(delivery(&env.ctx.handle, "cc-stuck-rep").await, Some(DeliveryState::Resumed));
    assert_eq!(delivery(&env.ctx.handle, "cc-stuck-none").await, Some(DeliveryState::Dead));
    let alerts: Vec<_> = env.ctx.discord.messages().into_iter().filter(|m| m.reason == "delivery_interrupted").collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].question_id.as_deref(), Some("cc-stuck-none"));
}

/// F：追加的子行程卡住會逾時中止，串行處理不被拖死；已送進原對話所以記 resumed＋原因。
#[tokio::test]
async fn hung_resume_times_out() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", "sleep 30");
    create(&env.ctx.handle, "cc-hung", claude_source(&env, "conv-hung", Some(&claude))).await;
    answer(&env.ctx.handle, "cc-hung").await;
    let mut cfg = ReactorConfig::new(env.ctx.vault.path().to_path_buf());
    cfg.claude_bin = Some(claude);
    cfg.sessions_dir = env.registry();
    cfg.desktop_sessions_dir = env.root.path().join("desktop");
    cfg.trusted_cli_roots.push(env.root.path().to_path_buf());
    // 時限留足空間：並行跑整個 workspace 時，假 claude 腳本光是啟動並寫下呼叫紀錄就可能超過 2 秒。
    cfg.exec_timeout = std::time::Duration::from_secs(5);
    let reactor = Reactor::new(env.ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key())));
    let started = std::time::Instant::now();
    reactor.process_pending().await.unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(25), "沒有被卡死（腳本本身要睡 30 秒）");
    assert_eq!(env.calls().len(), 1, "逾時不重試");
    let d = env.ctx.handle.get_question("cc-hung").await.unwrap().unwrap().answer.unwrap().delivery.unwrap();
    assert_eq!(d.state, DeliveryState::Resumed);
    assert!(d.reason.as_deref().is_some_and(|r| r.contains("逾時")), "{d:?}");
}

/// H：錢題的錢鏈原始輸出（approval 路徑、token）不經 MCP 讀取面回給 agent，只給人話狀態。
#[tokio::test]
async fn money_chain_output_is_summarized_for_agents() {
    let env = Env::new().await;
    let mut req = money_t0_req("cc-money-read");
    req.source = claude_source(&env, "conv-money-read", None);
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question("cc-money-read", ui_answer("a"), now()).await.unwrap();
    env.ctx
        .handle
        .record_execution("cc-money-read", "APPROVAL_SIGNED: /v/approvals/x.json；DRY_RUN: executor commit --token tk-1", now())
        .await
        .unwrap();
    for (tool, args) in [
        ("ask_get", json!({"question_id": "cc-money-read"})),
        ("ask_list", json!({"include_terminal": true})),
        ("ask_wait", json!({"question_id": "cc-money-read", "timeout_seconds": 1})),
    ] {
        let out = mcp(&env.ctx.handle, tool, args).await.unwrap().to_string();
        for secret in ["tk-1", "approvals/x.json"] {
            assert!(!out.contains(secret), "{tool} 回了 {secret}：{out}");
        }
    }
    let got = mcp(&env.ctx.handle, "ask_get", json!({"question_id": "cc-money-read"})).await.unwrap();
    assert_eq!(got["question"]["answer"]["execution_result"], "乾跑，未實際執行");
}

/// I：所有 agent 子行程都清掉繼承的主機身分（CLAUDE_CODE_*／CLAUDE_PID／CLAUDECODE／CODEX_THREAD_ID）。
#[tokio::test]
async fn agent_spawns_scrub_host_identity() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", "echo 'No conversation found with session ID: gone' >&2\nexit 1");
    let codex = env.script("codex.sh", OK_BODY);
    create(&env.ctx.handle, "cc-scrub-c", claude_source(&env, "conv-scrub", Some(&claude))).await;
    create(&env.ctx.handle, "cc-scrub-x", codex_source(&env, "019a0000-0000-7000-8000-000000000002")).await;
    answer(&env.ctx.handle, "cc-scrub-c").await;
    answer(&env.ctx.handle, "cc-scrub-x").await;
    let reactor = env.reactor(Some(claude), Some(codex));
    reactor.process_pending().await.unwrap();
    let planned = reactor.planned();
    for kind in ["resume_claude", "wake_claude", "resume_codex"] {
        let cmd = planned.iter().find(|c| c.kind == kind).unwrap_or_else(|| panic!("缺 {kind}：{planned:?}"));
        for prefix in ["CLAUDE_CODE_", "CLAUDE_PID", "CLAUDECODE", "CODEX_THREAD_ID"] {
            assert!(cmd.scrub_env_prefixes.iter().any(|p| p == prefix), "{kind} 沒清 {prefix}");
        }
    }
}

// ---------------------------------------------------------------------------
// QA 第三輪（2026-10-08）
// ---------------------------------------------------------------------------

/// #2：同 id 重送 ask_create 給 agent 看的既有答案也要去敏（不含簽章與錢鏈原始輸出）。
#[tokio::test]
async fn recreate_of_answered_question_is_redacted() {
    let env = Env::new().await;
    let mut req = money_t0_req("cc-recreate");
    req.source = claude_source(&env, "conv-recreate", None);
    env.ctx.handle.create_question(req.clone(), now()).await.unwrap();
    let mut input = ui_answer("a");
    input.signature = Some("sig-SECRET-9".into());
    env.ctx.handle.answer_question("cc-recreate", input, now()).await.unwrap();
    env.ctx
        .handle
        // 真實格式：真的執行時錢鏈結果以 COMMIT_OK 開頭（乾跑才是 APPROVAL_SIGNED…；DRY_RUN commit）。
        .record_execution("cc-recreate", "COMMIT_OK: commit --token tk-1 --approval-file /v/approvals/x.json", now())
        .await
        .unwrap();
    let mut args = serde_json::to_value(&req).unwrap();
    args["requested_tier"] = args["tier"].take();
    args.as_object_mut().unwrap().remove("tier");
    let out = mcp(&env.ctx.handle, "ask_create", args).await.unwrap().to_string();
    for secret in ["sig-SECRET-9", "tk-1", "approvals/x.json"] {
        assert!(!out.contains(secret), "ask_create 回了 {secret}：{out}");
    }
    assert!(out.contains("已執行完成"), "錢鏈結果以人話狀態呈現：{out}");
}

/// #4：AI 回報只記回饋，不覆寫錢鏈結果（回執靠它判斷有沒有被擋）。
#[tokio::test]
async fn report_keeps_money_chain_result() {
    let env = Env::new().await;
    let mut req = money_t0_req("cc-report-money");
    req.source = claude_source(&env, "conv-report-money", None);
    env.ctx.handle.create_question(req, now()).await.unwrap();
    let q = env.ctx.handle.answer_question("cc-report-money", ui_answer("a"), now()).await.unwrap();
    let answer_id = q.answer.unwrap().answer_id;
    env.ctx.handle.record_execution("cc-report-money", "EXEC_BLOCKED: guard 升紅", now()).await.unwrap();
    env.ctx.handle.report_answer("cc-report-money", &answer_id, ReportStatus::Done, "已加碼", now()).await.unwrap();
    let a = env.ctx.handle.get_question("cc-report-money").await.unwrap().unwrap().answer.unwrap();
    assert_eq!(a.execution_result.as_deref(), Some("EXEC_BLOCKED: guard 升紅"));
    assert_eq!(a.report.unwrap().summary_md, "已加碼");
}

/// #3：錢題走等待者時，錢鏈還沒跑完就不交付；跑完後等待者拿到的答案帶人話的錢鏈狀態。
#[tokio::test]
async fn waiter_receives_money_answer_after_the_chain() {
    let env = Env::new().await;
    let mut req = money_t0_req("cc-wait-money");
    req.source = claude_source(&env, "conv-wait-money", None);
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question("cc-wait-money", ui_answer("a"), now()).await.unwrap();
    let early = mcp(&env.ctx.handle, "ask_wait", json!({"question_id": "cc-wait-money", "timeout_seconds": 1})).await.unwrap();
    assert_eq!(early["status"], "pending_timeout", "錢鏈還在排隊：先不交付：{early}");

    let h = env.ctx.handle.clone();
    let waiter = tokio::spawn(async move {
        mcp(&h, "ask_wait", json!({"question_id": "cc-wait-money", "timeout_seconds": 10})).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    env.reactor(None, None).process_pending().await.unwrap();
    let got = waiter.await.unwrap().unwrap();
    assert_eq!(got["status"], "answered", "{got}");
    assert!(got["answer"]["execution_result"].is_string(), "交付時已有錢鏈狀態：{got}");
    assert!(!got.to_string().contains("--token"), "{got}");
}

/// #5：Claude 的 API 錯誤（帶 error 的 assistant 事件）不是「已開始處理」：照常重試，最後開新對話。
#[tokio::test]
async fn api_error_turn_is_retried_not_resumed() {
    let env = Env::new().await;
    let body = r#"printf '%s\n' '{"type":"assistant","error":"overloaded","message":{"model":"<synthetic>","content":[{"type":"text","text":"API Error: 529"}]}}'
exit 1"#;
    let claude = env.script("claude.sh", body);
    create(&env.ctx.handle, "cc-api-error", claude_source(&env, "conv-api-error", Some(&claude))).await;
    answer(&env.ctx.handle, "cc-api-error").await;
    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    let resumes = reactor.planned().iter().filter(|c| c.kind == "resume_claude").count();
    assert!(resumes >= 1 && env.calls().len() > 1, "暫時性錯誤要重試：{:?}", env.calls());
    assert_ne!(delivery(&env.ctx.handle, "cc-api-error").await, Some(DeliveryState::Resumed));
}

/// #6：原對話已封存 → 不追加，改開新對話並寫明原因。
#[tokio::test]
async fn archived_conversation_starts_fresh() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    std::fs::write(
        env.root.path().join("desktop/acct/org/local_xyz-1.json"),
        json!({"sessionId": "local_xyz-1", "cliSessionId": "conv-arch", "isArchived": true}).to_string(),
    )
    .unwrap();
    create(&env.ctx.handle, "cc-archived", claude_source(&env, "conv-arch", Some(&claude))).await;
    answer(&env.ctx.handle, "cc-archived").await;
    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    assert!(!reactor.planned().iter().any(|c| c.kind == "resume_claude"), "封存的對話不追加");
    let d = env.ctx.handle.get_question("cc-archived").await.unwrap().unwrap().answer.unwrap().delivery.unwrap();
    assert_eq!(d.state, DeliveryState::Fresh);
    assert!(d.reason.as_deref().is_some_and(|r| r.contains("封存")), "{d:?}");
}

/// #8：追加中斷已告警（dead）的題，24 小時不再重複告警。
#[tokio::test]
async fn interrupted_delivery_is_not_alarmed_twice() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    create(&env.ctx.handle, "cc-twice", claude_source(&env, "conv-twice", Some(&claude))).await;
    let t0 = Utc::now();
    let aid = answer(&env.ctx.handle, "cc-twice").await;
    assert!(env.ctx.handle.set_delivery("cc-twice", &aid, DeliveryState::Resuming, None, None, t0).await.unwrap());
    env.reactor(Some(claude), None).process_due_deliveries(t0 + Duration::minutes(16)).await.unwrap();
    env.ctx.handle.tick(t0 + Duration::hours(25)).await.unwrap();
    let alerts: Vec<_> = env
        .ctx
        .discord
        .messages()
        .into_iter()
        .filter(|m| m.question_id.as_deref() == Some("cc-twice"))
        .map(|m| m.reason)
        .collect();
    assert_eq!(alerts, vec!["delivery_interrupted".to_string()], "只告警一次");
}

#[tokio::test]
async fn codex_locked_thread_queues_once_without_claiming_work_done() {
    let env = Env::new().await;
    let thread = "019a0000-0000-7000-8000-000000000001";
    let codex = env.script("codex-queue.sh", &format!(
        "case \"$1\" in queue) echo 'Queued message q1 for thread {thread}.'; exit 0;; esac\necho 'Error: thread already has an active writer' >&2\nexit 1"));
    create(&env.ctx.handle, "cq-ok", codex_source(&env, thread)).await;
    answer(&env.ctx.handle, "cq-ok").await;
    let reactor = env.reactor(None, Some(codex));
    reactor.process_pending().await.unwrap();
    let q = env.ctx.handle.get_question("cq-ok").await.unwrap().unwrap();
    let a = q.answer.unwrap();
    assert!(a.report.is_none());
    assert_eq!(a.delivery.unwrap().reason.as_deref(), Some("已排入原對話，等待 AI 接手"));
    let count = env.calls().len();
    reactor.process_due_deliveries(Utc::now() + Duration::seconds(61)).await.unwrap();
    assert_eq!(env.calls().len(), count);
    assert!(env.calls().iter().any(|c| c.contains("queue --thread")));
}

#[tokio::test]
async fn codex_queue_wrong_thread_is_not_accepted() {
    let env = Env::new().await;
    let codex = env.script("codex-queue-wrong.sh", "case \"$1\" in queue) echo 'Queued message q1 for thread other.'; exit 0;; esac\necho 'already has an active writer' >&2\nexit 1");
    create(&env.ctx.handle, "cq-wrong", codex_source(&env, "019a0000-0000-7000-8000-000000000001")).await;
    answer(&env.ctx.handle, "cq-wrong").await;
    env.reactor(None, Some(codex)).process_pending().await.unwrap();
    assert_eq!(delivery(&env.ctx.handle, "cq-wrong").await, Some(DeliveryState::Held));
}
