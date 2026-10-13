//! 回答延續原對話：錢鏈結果與 AI 回饋分開、agent 讀取面去敏、交付狀態可由帳本重建
//! （BDD conversation-continuity AC-CC-10／11／14；INVARIANTS 13、18、25）。源自 QA 第 4 輪的重現測試。

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
}

fn claude_source(env: &Env, conversation: &str, cli_path: Option<&Path>) -> Source {
    Source {
        agent: AgentKind::Claude,
        cwd: env.work().to_string_lossy().to_string(),
        host: Some(HostKind::ClaudeDesktop),
        conversation_id: Some(conversation.into()),
        host_session_id: Some("local_cc-1".into()),
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

async fn create(handle: &ServerHandle, id: &str, source: Source) {
    let mut req = base_req(id);
    req.source = source;
    handle.create_question(req, Utc::now()).await.unwrap();
}

async fn answer(handle: &ServerHandle, id: &str) -> String {
    handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id
}

async fn delivery_of(handle: &ServerHandle, id: &str) -> Option<kunsu_core::continuity::Delivery> {
    handle.get_question(id).await.unwrap().unwrap().answer.and_then(|a| a.delivery)
}

async fn mcp(handle: &ServerHandle, name: &str, arguments: Value) -> Result<Value, Value> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
    match handle_rpc(handle, body).await {
        McpReply::Json(v) if v.get("error").is_some() => Err(v["error"].clone()),
        McpReply::Json(v) => Ok(v["result"]["structuredContent"].clone()),
        McpReply::Accepted => panic!("tools/call 不得回 Accepted"),
    }
}

/// shell `to_ui` 的判準（src-tauri/src/ipc.rs execution_failure）：execution_result 以 EXEC_BLOCKED:／EXEC_FAILED: 開頭。
fn ui_shows_blocked(exec: Option<&str>) -> bool {
    exec.is_some_and(|r| r.starts_with("EXEC_BLOCKED:") || r.starts_with("EXEC_FAILED:"))
}

const OK_BODY: &str = "echo 'EXEC_OK: 原對話已接手'";
const SECRET_CHAIN: &str =
    "APPROVAL_SIGNED: /v/projects/meta-ads/data/executor_pending/approvals/qa4.json；COMMIT_OK: commit --token tk-QA4-SECRET --approval-file /v/approvals/qa4.json";

/// INV-25／AC-CC-14：agent 回報之後，所有 agent 讀取面仍不含錢鏈原始輸出（approval 路徑、executor token）。
#[tokio::test]
async fn inv_agent_reads_never_leak_money_secrets() {
    let env = Env::new().await;
    let mut req = money_t0_req("qa4-leak");
    req.source = claude_source(&env, "conv-leak", None);
    env.ctx.handle.create_question(req.clone(), now()).await.unwrap();
    let aid = env.ctx.handle.answer_question("qa4-leak", ui_answer("a"), now()).await.unwrap().answer.unwrap().answer_id;
    env.ctx.handle.record_execution("qa4-leak", SECRET_CHAIN, now()).await.unwrap();

    // 前提：回報前讀取面已去敏（第二輪 H 修正）。
    let before = mcp(&env.ctx.handle, "ask_get", json!({"question_id": "qa4-leak"})).await.unwrap().to_string();
    assert!(!before.contains("tk-QA4-SECRET"), "前提失敗：回報前就外洩：{before}");

    // 原對話照追加訊息的指示做完並回報。
    mcp(
        &env.ctx.handle,
        "ask_report",
        json!({"question_id": "qa4-leak", "answer_id": aid, "status": "done", "summary_md": "已確認錢鏈結果並更新日誌"}),
    )
    .await
    .unwrap();

    let mut args = serde_json::to_value(&req).unwrap();
    args["requested_tier"] = args["tier"].take();
    args.as_object_mut().unwrap().remove("tier");
    let mut leaks = Vec::new();
    for (tool, a) in [
        ("ask_get", json!({"question_id": "qa4-leak"})),
        ("ask_list", json!({"include_terminal": true})),
        ("ask_wait", json!({"question_id": "qa4-leak", "timeout_seconds": 1})),
        ("ask_create", args),
    ] {
        let out = mcp(&env.ctx.handle, tool, a).await.unwrap().to_string();
        for secret in ["tk-QA4-SECRET", "approvals/qa4.json"] {
            if out.contains(secret) {
                leaks.push(format!("{tool} → {secret}"));
            }
        }
    }
    assert!(leaks.is_empty(), "AI 回報後，agent 讀取面回了錢鏈原始輸出：{leaks:?}");
}

/// AC-CC-14：真的錢鏈（假 executor、commit 乾跑）→ 原對話回報 → ask_get 仍只給人話狀態。
#[tokio::test]
async fn agent_reads_stay_redacted_after_report_with_real_chain() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    write_fake_executor(&env.ctx);
    let mut req = money_t0_req("qa4-leak-e2e");
    req.source = claude_source(&env, "conv-leak-e2e", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    let aid = env.ctx.handle.answer_question("qa4-leak-e2e", ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    let mut cfg = env.cfg(Some(claude), None);
    cfg.dry_run = true;
    Reactor::new(env.ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key()))).process_pending().await.unwrap();
    let raw = env.ctx.handle.get_question("qa4-leak-e2e").await.unwrap().unwrap().answer.unwrap().execution_result.unwrap_or_default();
    assert!(raw.contains(FAKE_EXECUTOR_TOKEN), "前提：錢鏈原始輸出含 executor token：{raw}");
    let before = mcp(&env.ctx.handle, "ask_get", json!({"question_id": "qa4-leak-e2e"})).await.unwrap().to_string();
    assert!(!before.contains(FAKE_EXECUTOR_TOKEN), "前提：回報前已去敏");
    mcp(
        &env.ctx.handle,
        "ask_report",
        json!({"question_id": "qa4-leak-e2e", "answer_id": aid, "status": "done", "summary_md": "好"}),
    )
    .await
    .unwrap();
    let after = mcp(&env.ctx.handle, "ask_get", json!({"question_id": "qa4-leak-e2e"})).await.unwrap();
    let exec = after["question"]["answer"]["execution_result"].as_str().unwrap_or("").to_string();
    assert!(
        !exec.contains(FAKE_EXECUTOR_TOKEN) && !exec.contains("approvals/"),
        "回報後 ask_get 回了錢鏈原始輸出：{exec}"
    );
}

/// AC-CC-14：回報早於錢鏈時，之後錢鏈被擋的結果照樣寫入，回執看得到被擋。
#[tokio::test]
async fn report_before_money_chain_keeps_blocked_result() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let mut req = money_t0_req("qa4-early");
    req.source = claude_source(&env, "conv-early", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    let aid = env.ctx.handle.answer_question("qa4-early", ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;

    // Codex heartbeat／原對話自己 ask_get 讀到答案（ask_get 不受錢鏈閘門限制），處理後回報。
    let got = mcp(&env.ctx.handle, "ask_get", json!({"question_id": "qa4-early"})).await.unwrap();
    assert_eq!(got["question"]["answer"]["choice_id"], "a", "ask_get 在錢鏈跑完前就給出答案");
    mcp(
        &env.ctx.handle,
        "ask_report",
        json!({"question_id": "qa4-early", "answer_id": aid, "status": "done", "summary_md": "已依使用者決定加碼 20%"}),
    )
    .await
    .unwrap();
    assert_eq!(delivery_of(&env.ctx.handle, "qa4-early").await.map(|d| d.state), Some(DeliveryState::PickedUp));

    // vault 沒有 executor.py → 錢鏈被擋。
    env.reactor(Some(claude), None).process_pending().await.unwrap();
    assert!(
        env.ctx.discord.messages().iter().any(|m| m.reason == "execution_blocked"),
        "前提：錢鏈確實被擋（Discord execution_blocked）"
    );
    let a = env.ctx.handle.get_question("qa4-early").await.unwrap().unwrap().answer.unwrap();
    assert!(
        ui_shows_blocked(a.execution_result.as_deref()),
        "錢鏈被擋，但題目上的 execution_result={:?}：回執不會顯示 execution_blocked，只看到 AI 回饋「已完成」（calls={:?}）",
        a.execution_result,
        env.calls()
    );
}

/// AC-CC-14：被擋 → 回報 → 重新授權 commit 成功，回執不再顯示被擋。
#[tokio::test]
async fn reauthorized_commit_after_report_shows_success() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let mut req = money_t0_req("qa4-reauth");
    req.source = claude_source(&env, "conv-reauth", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    let aid = env.ctx.handle.answer_question("qa4-reauth", ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;

    // 第一次：沒有 executor → 被擋；原對話收到「被擋」並回報。
    let reactor = env.reactor(Some(claude), None);
    reactor.process_pending().await.unwrap();
    let a = env.ctx.handle.get_question("qa4-reauth").await.unwrap().unwrap().answer.unwrap();
    assert!(ui_shows_blocked(a.execution_result.as_deref()), "前提：第一次被擋：{:?}", a.execution_result);
    env.ctx
        .handle
        .report_answer("qa4-reauth", &aid, ReportStatus::Blocked, "錢鏈被擋，等使用者重新授權", Utc::now())
        .await
        .unwrap();

    // 使用者修好後重新授權；這次 commit 成功（假 executor）。
    write_fake_executor(&env.ctx);
    env.ctx.handle.reauthorize_question("qa4-reauth", Utc::now()).await.unwrap();
    reactor.process_pending().await.unwrap();
    let rows = read_ledger_rows(&env.ctx, Utc::now());
    assert!(
        rows.iter().any(|r| r.execution.as_ref().is_some_and(|e| e.to_string().contains("COMMIT_OK"))),
        "前提：重新授權後 commit 成功（帳本有 COMMIT_OK）"
    );
    let a = env.ctx.handle.get_question("qa4-reauth").await.unwrap().unwrap().answer.unwrap();
    assert!(
        !ui_shows_blocked(a.execution_result.as_deref()),
        "commit 已成功，但回執仍顯示被擋：execution_result={:?}",
        a.execution_result
    );
}

/// AC-CC-10／14：錢已 commit、交付卻進死信 → 題目或帳本保留 COMMIT_OK，回執不顯示成執行失敗。
#[tokio::test]
async fn committed_money_survives_dead_delivery() {
    let env = Env::new().await;
    let claude = env.script(
        "claude.sh",
        "case \"$*\" in *--resume*) echo 'No conversation found with session ID: gone' >&2; exit 1;; esac\necho 'boom' >&2\nexit 1",
    );
    write_fake_executor(&env.ctx);
    let mut req = money_t0_req("qa4-money-dead");
    req.source = claude_source(&env, "conv-money-dead", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question("qa4-money-dead", ui_answer("a"), Utc::now()).await.unwrap();
    let reactor = env.reactor(Some(claude), None);
    let mut rx = env.ctx.handle.subscribe();
    reactor.process_pending().await.unwrap();
    assert!(reactor.planned().iter().any(|c| c.kind == "commit" && c.executed), "前提：commit 真的執行");
    let api_ok = drain_events(&mut rx).into_iter().any(|e| {
        matches!(e, kunsu_server::ServerEvent::Execution { step: kunsu_server::ExecStep::Api, status: kunsu_server::ExecStatus::Ok, .. })
    });
    assert!(api_ok, "前提：commit 成功（Execution Api Ok）");
    let a = env.ctx.handle.get_question("qa4-money-dead").await.unwrap().unwrap().answer.unwrap();
    assert_eq!(a.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Dead), "前提：交付死信");
    let rows = read_ledger_rows(&env.ctx, Utc::now());
    let ledger_has_commit = rows.iter().any(|r| {
        r.question.id == "qa4-money-dead" && r.execution.as_ref().is_some_and(|e| e.to_string().contains("COMMIT_OK"))
    });
    assert!(ledger_has_commit, "commit 已執行，帳本必須有 COMMIT_OK");
    let exec = a.execution_result.as_deref().unwrap_or("");
    assert!(
        exec.starts_with("COMMIT_OK") && exec.contains("交付失敗") && !ui_shows_blocked(Some(exec)),
        "回執要以錢鏈結果開頭並說明交付失敗，不能顯示成執行失敗；execution_result={exec:?}"
    );
}

/// INV-13／INV-18／AC-CC-11：錢題答案已被等待者取走，重建資料庫後不再追加一次。
#[tokio::test]
async fn rebuild_never_redelivers_waiter_taken_money_answer() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    write_fake_executor(&env.ctx);
    let mut req = money_t0_req("qa4-rb-money");
    req.source = claude_source(&env, "conv-rb-money", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();

    let h = env.ctx.handle.clone();
    let waiter = tokio::spawn(async move {
        mcp(&h, "ask_wait", json!({"question_id": "qa4-rb-money", "timeout_seconds": 30})).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    env.ctx.handle.answer_question("qa4-rb-money", ui_answer("a"), Utc::now()).await.unwrap();
    // 錢鏈（commit 只記錄）跑完 → 等待者拿到答案。
    let mut cfg = env.cfg(Some(claude.clone()), None);
    cfg.dry_run = true;
    let dry = Reactor::new(env.ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key())));
    dry.process_pending().await.unwrap();
    let got = waiter.await.unwrap().unwrap();
    assert_eq!(got["delivery"]["state"], "live_waiter", "前提：等待者取走：{got}");
    dry.process_due_deliveries(Utc::now() + Duration::seconds(5)).await.unwrap();
    assert!(env.calls().is_empty(), "前提：重建前沒有追加");

    // SQLite 整顆壞掉 → 由帳本重建（INV-13），reactor 照常掃描。
    let last = read_ledger_rows(&env.ctx, Utc::now())
        .into_iter()
        .filter(|r| r.question.id == "qa4-rb-money")
        .last()
        .unwrap();
    let ledger_state = last.question.answer.as_ref().and_then(|a| a.delivery.as_ref()).map(|d| d.state);
    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    let r2 = env.reactor_on(&h2, Some(claude), None);
    r2.process_pending().await.unwrap();
    r2.process_due_deliveries(Utc::now() + Duration::seconds(5)).await.unwrap();
    let d = h2.get_question("qa4-rb-money").await.unwrap().unwrap().answer.unwrap().delivery;
    assert!(
        env.calls().is_empty(),
        "等待者早已取走答案，重建後又在原對話追加了一次（帳本最後快照的交付狀態={ledger_state:?}；重建後={d:?}；calls={:?}）",
        env.calls()
    );
}

/// INV-13／AC-CC-11：held 的交付重建後保留，原對話釋放後照常追加。
#[tokio::test]
async fn rebuild_keeps_held_delivery_and_resumes_later() {
    let env = Env::new().await;
    let locked = env.script("codex.sh", "echo 'Error: thread x already has an active writer' >&2\nexit 1");
    create(&env.ctx.handle, "qa4-rb-held", codex_source(&env, "019a0000-0000-7000-8000-0000000000a4")).await;
    answer(&env.ctx.handle, "qa4-rb-held").await;
    env.reactor(None, Some(locked)).process_pending().await.unwrap();
    assert_eq!(delivery_of(&env.ctx.handle, "qa4-rb-held").await.map(|d| d.state), Some(DeliveryState::Held));

    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    let ok = env.script("codex-ok.sh", OK_BODY);
    let r2 = env.reactor_on(&h2, None, Some(ok));
    r2.process_pending().await.unwrap();
    r2.process_due_deliveries(Utc::now() + Duration::seconds(61)).await.unwrap();
    let d = h2.get_question("qa4-rb-held").await.unwrap().unwrap().answer.unwrap().delivery;
    assert_eq!(
        d.as_ref().map(|d| d.state),
        Some(DeliveryState::Resumed),
        "held 的答案重建後應繼續重檢、釋放後接續；實際交付={d:?}，calls={:?}",
        env.calls()
    );
}

/// INV-13／AC-CC-11：交給等待者的交付與 acked_at 重建後保留，不會誤發 24 小時告警。
#[tokio::test]
async fn rebuild_keeps_live_waiter_delivery_and_ack() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    create(&env.ctx.handle, "qa4-rb-lw", claude_source(&env, "conv-rb-lw", Some(&claude))).await;
    let t = Utc::now();
    env.ctx.handle.waiter_enter("qa4-rb-lw", t);
    env.ctx.handle.waiter_exit("qa4-rb-lw", t);
    answer(&env.ctx.handle, "qa4-rb-lw").await;
    let reactor = env.reactor(Some(claude.clone()), None);
    reactor.process_pending().await.unwrap();
    let got = mcp(&env.ctx.handle, "ask_wait", json!({"question_id": "qa4-rb-lw", "timeout_seconds": 2})).await.unwrap();
    assert_eq!(got["delivery"]["state"], "live_waiter");
    reactor.process_due_deliveries(Utc::now() + Duration::seconds(5)).await.unwrap();

    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    let a = h2.get_question("qa4-rb-lw").await.unwrap().unwrap().answer.unwrap();
    assert!(
        a.delivery.as_ref().is_some_and(|d| d.state == DeliveryState::LiveWaiter) && a.acked_at.is_some(),
        "重建後應仍是已交給等待者且已 acked；實際 delivery={:?} acked_at={:?}",
        a.delivery,
        a.acked_at
    );
}

/// 追加中（resuming）agent 已回報，app 中斷且 DB 遺失 → 重建後不再追加；15 分鐘後記 resumed、不告警。
#[tokio::test]
async fn rebuild_resuming_with_report_is_not_redone() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    create(&env.ctx.handle, "qa4-rb-rep", claude_source(&env, "conv-rb-rep", Some(&claude))).await;
    let aid = answer(&env.ctx.handle, "qa4-rb-rep").await;
    let t0 = Utc::now();
    assert!(env.ctx.handle.set_delivery("qa4-rb-rep", &aid, DeliveryState::Resuming, None, None, t0).await.unwrap());
    env.ctx.handle.report_answer("qa4-rb-rep", &aid, ReportStatus::Done, "處理好了", t0).await.unwrap();

    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    assert_eq!(
        h2.get_question("qa4-rb-rep").await.unwrap().unwrap().answer.unwrap().delivery.map(|d| d.state),
        Some(DeliveryState::Resuming),
        "重建還原 resuming"
    );
    let r2 = env.reactor_on(&h2, Some(claude), None);
    r2.process_pending().await.unwrap();
    r2.process_due_deliveries(Utc::now()).await.unwrap();
    assert!(env.calls().is_empty(), "15 分鐘內不重做：{:?}", env.calls());
    r2.process_due_deliveries(t0 + Duration::minutes(16)).await.unwrap();
    assert!(env.calls().is_empty(), "已回報的不重做：{:?}", env.calls());
    let d = h2.get_question("qa4-rb-rep").await.unwrap().unwrap().answer.unwrap().delivery.unwrap();
    assert_eq!(d.state, DeliveryState::Resumed);
}

/// heartbeat 在 held 期間回報 → 帳本快照帶 picked_up（43df84e）→ 重建後仍 picked_up，不追加。
#[tokio::test]
async fn report_while_held_survives_rebuild_as_picked_up() {
    let env = Env::new().await;
    let locked = env.script("codex.sh", "echo 'Error: thread x already has an active writer' >&2\nexit 1");
    create(&env.ctx.handle, "qa4-rb-pick", codex_source(&env, "019a0000-0000-7000-8000-0000000000b4")).await;
    let aid = answer(&env.ctx.handle, "qa4-rb-pick").await;
    env.reactor(None, Some(locked)).process_pending().await.unwrap();
    assert_eq!(delivery_of(&env.ctx.handle, "qa4-rb-pick").await.map(|d| d.state), Some(DeliveryState::Held));
    mcp(
        &env.ctx.handle,
        "ask_report",
        json!({"question_id": "qa4-rb-pick", "answer_id": aid, "status": "done", "summary_md": "heartbeat 處理完"}),
    )
    .await
    .unwrap();
    let last = read_ledger_rows(&env.ctx, Utc::now()).into_iter().filter(|r| r.question.id == "qa4-rb-pick").last().unwrap();
    assert_eq!(
        last.question.answer.as_ref().and_then(|a| a.delivery.as_ref()).map(|d| d.state),
        Some(DeliveryState::PickedUp),
        "帳本快照帶 picked_up"
    );
    let before = env.calls().len();
    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    let ok = env.script("codex-ok.sh", OK_BODY);
    let r2 = env.reactor_on(&h2, None, Some(ok));
    r2.process_pending().await.unwrap();
    r2.process_due_deliveries(Utc::now() + Duration::seconds(61)).await.unwrap();
    assert_eq!(env.calls().len(), before, "picked_up 重建後不再追加");
    assert_eq!(
        h2.get_question("qa4-rb-pick").await.unwrap().unwrap().answer.unwrap().delivery.map(|d| d.state),
        Some(DeliveryState::PickedUp)
    );
}
