//! 回答延續原對話第 6 輪：重建先讀完帳本、只重做近期答案、帳本單一在途寫入與 row_id 去重、OPEN.json 重寫、
//! 錢鏈不適用一律「未觸發」、事件去敏（BDD conversation-continuity AC-CC-11／14；INV-13、18、25）。源自 QA 第 6 輪。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration, Utc};
use common::*;
use kunsu_core::continuity::{DeliveryState, HostKind};
use kunsu_core::types::{AgentKind, Gate, Source};
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
}

fn claude_source(env: &Env, conversation: &str, cli_path: Option<&Path>) -> Source {
    Source {
        agent: AgentKind::Claude,
        cwd: env.work().to_string_lossy().to_string(),
        host: Some(HostKind::ClaudeDesktop),
        conversation_id: Some(conversation.into()),
        host_session_id: Some("local_qa6-1".into()),
        attended: Some(false),
        cli_path: cli_path.map(|p| p.to_string_lossy().to_string()),
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

/// 追加找不到對話 → 開新對話也一直失敗 → 死信。
const ALWAYS_FAIL: &str =
    "case \"$*\" in *--resume*) echo 'No conversation found with session ID: gone' >&2; exit 1;; esac\necho 'boom' >&2\nexit 1";
/// 追加找不到對話 → 開新對話已開始處理（assistant 事件）但非零結束。
const FRESH_STARTED_THEN_FAILS: &str = "case \"$*\" in *--resume*) echo 'No conversation found with session ID: gone' >&2; exit 1;; esac\nprintf '%s\\n' '{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"working\"}]}}'\necho 'budget exceeded' >&2\nexit 1";
const OK_BODY: &str = "echo 'EXEC_OK: 原對話已接手'";

// ---------------------------------------------------------------------------
// R6-1：錢題選安全選項（錢鏈未觸發）、交付失敗或新對話處理到一半出錯 → execution_result 以 EXEC_FAILED 開頭
// → agent 讀到「執行沒有完成（細節見 Kunsu 回執）」。規格：安全選項或紅燈是「未觸發」。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn safe_option_dead_letter_reads_untriggered() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", ALWAYS_FAIL);
    let id = "qa6-safe-dead";
    let mut req = money_t0_req(id);
    req.source = claude_source(&env, "conv-qa6-sd", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("b"), Utc::now()).await.unwrap();
    env.reactor(Some(claude), None).process_pending().await.unwrap();
    let a = env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap();
    assert_eq!(a.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Dead), "前提：交付死信");
    let got = mcp(&env.ctx.handle, "ask_get", json!({"question_id": id})).await.unwrap();
    let status = got["question"]["answer"]["execution_result"].as_str().unwrap_or("").to_string();
    assert!(
        status.contains("未觸發"),
        "安全選項沒有動錢，agent 讀到的錢鏈狀態卻是 {status:?}（原始 execution_result={:?}）",
        a.execution_result
    );
}

#[tokio::test]
async fn red_gate_fresh_partial_reads_untriggered() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", FRESH_STARTED_THEN_FAILS);
    let id = "qa6-red-partial";
    let mut req = money_t0_req(id);
    req.gate = Some(Gate::Red);
    req.source = claude_source(&env, "conv-qa6-rp", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    env.reactor(Some(claude), None).process_pending().await.unwrap();
    let a = env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap();
    assert_eq!(a.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Fresh), "前提：新對話已開始處理");
    let got = mcp(&env.ctx.handle, "ask_get", json!({"question_id": id})).await.unwrap();
    let status = got["question"]["answer"]["execution_result"].as_str().unwrap_or("").to_string();
    assert!(
        status.contains("未觸發"),
        "紅燈題錢鏈不觸發，agent 讀到的錢鏈狀態卻是 {status:?}（原始 execution_result={:?}）",
        a.execution_result
    );
}

// ---------------------------------------------------------------------------
// R6-2：/events 的 Prepare Fail 事件會帶 executor token（prepare 印出 token 但 spec 檔不在固定目錄時，
// 錯誤訊息含 `<token>.json` 路徑）。規格：`/events` 串流的執行事件不帶 executor token。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn events_prepare_fail_never_carries_token() {
    let env = Env::new().await;
    // 假 executor：印出 commit token，但沒有寫 pending spec。
    let body = format!(
        "#!/usr/bin/env python3\nprint('     python3 projects/meta-ads/scripts/executor.py commit --token {FAKE_EXECUTOR_TOKEN}')\n"
    );
    let dir = env.ctx.vault.path().join("projects/meta-ads/scripts");
    std::fs::create_dir_all(&dir).unwrap();
    write_script(&dir, "executor.py", &body);
    let id = "qa6-events-fail";
    let mut req = money_t0_req(id);
    req.source = claude_source(&env, "conv-qa6-ef", None);
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    let mut rx = env.ctx.handle.subscribe();
    let mut cfg = env.cfg(None, None);
    cfg.dry_run = true;
    Reactor::new(env.ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key()))).process_pending().await.unwrap();
    assert!(env.ctx.discord.messages().iter().any(|m| m.reason == "execution_blocked"), "前提：錢鏈被擋");
    let frames: Vec<String> = drain_events(&mut rx).iter().map(|e| serde_json::to_string(e).unwrap()).collect();
    let leaked: Vec<&String> = frames.iter().filter(|f| f.contains(FAKE_EXECUTOR_TOKEN)).collect();
    let discord: Vec<String> = env
        .ctx
        .discord
        .messages()
        .into_iter()
        .filter(|m| m.text.contains(FAKE_EXECUTOR_TOKEN))
        .map(|m| m.reason)
        .collect();
    eprintln!("R6-2 Discord 訊息含 token：{discord:?}");
    assert!(leaked.is_empty(), "/events 會把這些 frame 原樣送給持 agent token 的訂閱者：{leaked:?}");
}

// ---------------------------------------------------------------------------
// R6-3：帳本寫入卡住期間，tick 每 30 秒補寫一次，每次都在寫入鎖內卡 5 秒並留下一條卡住的寫入執行緒；
// 卡住的寫入在權限恢復後完成 → 同一行被寫進帳本多次。規格：依序補寫（同一行只補一次）、不拖垮 server。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backlog_flush_keeps_one_writer_and_dedupes_rows() {
    let env = Env::new().await;
    let id = "qa6-backlog";
    env.ctx.handle.create_question(base_req(id), Utc::now()).await.unwrap();
    env.ctx.handle.create_question(base_req("qa6-backlog-2"), Utc::now()).await.unwrap();
    let t0 = Utc::now();
    let day = env.ctx.handle.ledger().day_file(t0);
    let bak = day.with_extension("jsonl.bak");
    if day.exists() {
        std::fs::rename(&day, &bak).unwrap();
    }
    assert!(std::process::Command::new("/usr/bin/mkfifo").arg(&day).status().unwrap().success());

    // 一筆終態寫不進帳本 → 排進待補寫佇列（第 1 次嘗試卡住）。
    let s = std::time::Instant::now();
    env.ctx.handle.cancel_question(id, t0).await.unwrap();
    eprintln!("R6-3 cancel 花了 {:?}", s.elapsed());
    assert!(env.ctx.discord.messages().iter().any(|m| m.reason == "ledger_degraded"), "前提：帳本寫不進去");

    // 權限還沒恢復：tick 每 30 秒補寫一次。
    let mut tick_times = Vec::new();
    for k in 1..=3 {
        let s = std::time::Instant::now();
        env.ctx.handle.tick(t0 + Duration::seconds(31 * k)).await.unwrap();
        tick_times.push(s.elapsed());
    }
    eprintln!("R6-3 每次補寫 tick 在寫入鎖內花了 {tick_times:?}");

    // 同時使用者在 UI 作答：要等 tick 放開寫入鎖，再加上自己那一次補寫嘗試。
    let h = env.ctx.handle.clone();
    let tick = tokio::spawn(async move { h.tick(t0 + Duration::seconds(31 * 4)).await.unwrap() });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let s = std::time::Instant::now();
    env.ctx.handle.answer_question("qa6-backlog-2", ui_answer("a"), Utc::now()).await.unwrap();
    let answer_latency = s.elapsed();
    tick.await.unwrap();
    eprintln!("R6-3 帳本卡住時 UI 作答延遲 {answer_latency:?}");

    // 權限恢復：所有卡在 open 的寫入者一起完成（FIFO 讀端收到它們寫的行）。
    let late = drain_fifo(&day);
    let late_copies = late.iter().filter(|l| l.contains(id) && l.contains("\"cancelled\"")).count();
    eprintln!("R6-3 卡住的寫入執行緒數（每條都握著佇列頭那一行）：{}，其中取消那一行 {late_copies} 份", late.len());

    // 模擬真實情境：卡住的寫入在權限恢復後寫進真正的帳本檔，之後 tick 補寫佇列。
    std::fs::remove_file(&day).unwrap();
    let mut content = std::fs::read_to_string(&bak).unwrap_or_default();
    for l in &late {
        content.push_str(l);
        content.push('\n');
    }
    std::fs::write(&day, content).unwrap();
    env.ctx.handle.tick(t0 + Duration::seconds(31 * 6)).await.unwrap();
    assert!(env.ctx.discord.messages().iter().any(|m| m.reason == "ledger_recovered"), "前提：補寫完成");
    let rows = read_ledger_rows(&env.ctx, t0);
    // 同一行可能出現兩次（逾時的寫入後來寫成＋補寫），重建只認第一次：以 row_id 去重後只能有一行。
    let cancelled = rows
        .iter()
        .filter(|r| r.question.id == id && r.ledger_type == LedgerType::Cancelled)
        .map(|r| r.row_id.clone().unwrap_or_default())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    eprintln!("R6-3 帳本裡取消那一行出現 {cancelled} 次");

    assert!(
        late.len() <= 1 && cancelled == 1,
        "帳本卡住期間每次補寫都留下一條卡住的寫入執行緒（共 {} 條，tick 每次在寫入鎖內 {:?}，UI 作答延遲 {answer_latency:?}）；\
         權限恢復後同一行在帳本出現 {cancelled} 次",
        late.len(),
        tick_times
    );
}

// ---------------------------------------------------------------------------
// R6-4：OPEN.json 寫不進去時（同一個權限問題），新建的 pending 題只在資料庫裡；待補寫佇列是空的，重建照樣放行 →
// 題目消失。規格 INV-13：由 ledger + OPEN.json 完整重建；帳本寫不進去的變更排隊補寫，積欠未清時拒絕重建。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rebuild_after_open_json_outage_keeps_pending_questions() {
    let env = Env::new().await;
    env.ctx.handle.create_question(base_req("qa6-open-1"), Utc::now()).await.unwrap();
    let tmp = env.ctx.handle.ledger().asks_dir().join(".OPEN.json.tmp");
    assert!(std::process::Command::new("/usr/bin/mkfifo").arg(&tmp).status().unwrap().success());
    // 權限問題期間建題：OPEN.json 寫不進去（5 秒逾時），題目只在資料庫。
    env.ctx.handle.create_question(base_req("qa6-open-2"), Utc::now()).await.unwrap();
    assert!(env.ctx.discord.messages().iter().any(|m| m.reason == "ledger_degraded"), "前提：OPEN.json 寫不進去");
    // 寫入還卡著時（server 活著，`kunsu rebuild-db` 走同一個 server）：OPEN.json 待重寫 → 拒絕重建，資料庫保持原樣。
    let refused = env.ctx.handle.rebuild_db().await;
    assert!(refused.is_err(), "OPEN.json 待重寫時應拒絕重建：{refused:?}");
    assert!(env.ctx.handle.get_question("qa6-open-2").await.unwrap().is_some(), "拒絕重建時資料庫保持原樣");
    // 權限恢復：卡住的寫入完成（打開 FIFO 讀端放行），定期 tick 重寫 OPEN.json。
    drain_fifo(&tmp);
    let _ = std::fs::remove_file(&tmp);
    env.ctx.handle.tick(Utc::now() + Duration::seconds(31)).await.unwrap();

    // 重寫後連資料庫整顆換新也能從 OPEN.json 還原。
    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    let report = h2.rebuild_db().await;
    let restored = match &report {
        Ok(_) => h2.get_question("qa6-open-2").await.unwrap().is_some(),
        Err(_) => false,
    };
    assert!(
        restored,
        "權限問題期間建的 pending 題只在資料庫裡，重建卻放行並把它丟掉了：report={report:?}"
    );
}

// ---------------------------------------------------------------------------
// R6-5（應綠）：approval 已簽發、未消耗（commit 前中斷）→ 重建後重新排隊，錢鏈只重跑一次。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rebuild_requeues_issued_but_unconsumed_money_once() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    write_fake_executor(&env.ctx);
    let id = "qa6-issued";
    let mut req = money_t0_req(id);
    req.source = claude_source(&env, "conv-qa6-iss", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    let aid = env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    env.ctx
        .handle
        .record_approval_issued(id, &aid, "n-qa6-iss", FAKE_EXECUTOR_TOKEN, "h", Utc::now().timestamp(), Utc::now())
        .await
        .unwrap();
    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    let report = h2.rebuild_db().await.unwrap();
    assert_eq!((report.requeued, report.reconcile), (1, 0), "{report:?}");
    let r2 = env.reactor_on(&h2, Some(claude), None);
    r2.process_pending().await.unwrap();
    r2.process_pending().await.unwrap();
    r2.process_due_deliveries(Utc::now() + Duration::minutes(20)).await.unwrap();
    let commits = r2.planned().iter().filter(|c| c.kind == "commit").count();
    assert_eq!(commits, 1, "錢鏈只重跑一次：{:?}", r2.planned());
    assert_eq!(env.calls().len(), 1, "交付一次：{:?}", env.calls());
}

// ---------------------------------------------------------------------------
// R6-6（應綠）：internal 題重建後重新排隊只乾跑，不 spawn。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rebuild_internal_stays_dry() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let id = "qa6-internal";
    let mut req = base_req(id);
    req.internal = true;
    req.source = claude_source(&env, "conv-qa6-int", Some(&claude));
    req.wake_on_answer = kunsu_core::types::Wake { mode: kunsu_core::types::WakeMode::Fresh, session_id: None, playbook: None };
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    let report = h2.rebuild_db().await.unwrap();
    eprintln!("R6-6 report={report:?}");
    let r2 = env.reactor_on(&h2, Some(claude), None);
    r2.process_pending().await.unwrap();
    assert!(env.calls().is_empty(), "internal 只乾跑：{:?}", env.calls());
    assert!(r2.planned().iter().all(|c| !c.executed), "{:?}", r2.planned());
}

// ---------------------------------------------------------------------------
// R6-7：wake=none、沒有原對話身分的題，作答後 reactor 還沒定案 no_continuation 就重建 → 交付紀錄遺失，
// 回執永遠沒有「沒有原對話可接續」。規格：重建前已回答、還沒被處理的答案重建後照常處理（AC-CC-03／11）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rebuild_settles_wake_none_answers() {
    let env = Env::new().await;
    let id = "qa6-none";
    let mut req = base_req(id);
    req.source = Source { cwd: "/x".into(), ..Default::default() };
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    let r2 = env.reactor_on(&h2, None, None);
    r2.process_pending().await.unwrap();
    r2.process_due_deliveries(Utc::now() + Duration::seconds(5)).await.unwrap();
    let d = h2.get_question(id).await.unwrap().unwrap().answer.unwrap().delivery;
    assert_eq!(
        d.as_ref().map(|d| d.state),
        Some(DeliveryState::NoContinuation),
        "重建前已回答、還沒定案的 wake=none 題，重建後應照常定案 no_continuation；實際={d:?}"
    );
}

// ---------------------------------------------------------------------------
// R6-8：錢鏈被擋（EXEC_BLOCKED）→ 重建資料庫 → 使用者修好 executor 想重新授權 → 被拒「執行尚未終結」
// （reactor_runs 被清空，requeue 找不到列）。規格：重建後狀態與重建前一致；被擋的錢題可重新授權。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn blocked_money_can_be_reauthorized_after_rebuild() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let id = "qa6-reauth-rb";
    let mut req = money_t0_req(id);
    req.source = claude_source(&env, "conv-qa6-rr", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    env.reactor(Some(claude), None).process_pending().await.unwrap();
    let a = env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap();
    assert!(a.execution_result.as_deref().unwrap_or("").starts_with("EXEC_BLOCKED"), "前提：被擋 {:?}", a.execution_result);
    // 重建前可以重新授權（對照組，不實際執行）。
    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    write_fake_executor(&env.ctx);
    let r = h2.reauthorize_question(id, Utc::now()).await;
    assert!(r.is_ok(), "重建後被擋的錢題無法重新授權：{:?}", r.err().map(|e| e.to_string()));
}

// ---------------------------------------------------------------------------
// R6-9（應綠）：帳本積欠補不進去時，approval 消耗 fail closed（不消耗、不排隊）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approval_consumption_fails_closed_with_backlog() {
    let env = Env::new().await;
    let id = "qa6-approval-backlog";
    env.ctx.handle.create_question(money_t0_req(id), now()).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    let day = env.ctx.handle.ledger().day_file(Utc::now());
    let _ = std::fs::rename(&day, day.with_extension("jsonl.bak"));
    assert!(std::process::Command::new("/usr/bin/mkfifo").arg(&day).status().unwrap().success());
    env.ctx.handle.record_execution(id, "EXEC_BLOCKED: x", Utc::now()).await.unwrap(); // 排進佇列
    let record = json!({"question_id": id, "nonce": "n-qa6-ab", "param_hash": "h"});
    let r = env.ctx.handle.consume_approval(id, "n-qa6-ab", &record, Utc::now()).await;
    // 權限恢復：卡住的寫入完成（打開 FIFO 讀端放行；它寫的那行已在佇列裡，之後補寫，重建以 row_id 去重）。
    drain_fifo(&day);
    let _ = std::fs::remove_file(&day);
    assert!(r.is_err(), "帳本積欠時 approval 消耗應 fail closed：{r:?}");
    // 積欠清掉後同一 nonce 仍可消耗（剛才沒有偷偷消耗）。
    std::fs::rename(day.with_extension("jsonl.bak"), &day).unwrap_or(());
    let r2 = env.ctx.handle.consume_approval(id, "n-qa6-ab", &record, Utc::now()).await.unwrap();
    assert!(matches!(r2, kunsu_server::ApprovalOutcome::Consumed), "{r2:?}");
}

// ---------------------------------------------------------------------------
// R6-11：vault 讀不到（沒有完全磁碟權限）但待補寫佇列是空的 → 重建守門放行 → 先清空資料庫，再「讀到」空帳本，
// 回報成功，所有題目消失。規格 INV-13：不丟掉只在資料庫裡的資料；帳本有問題時拒絕重建。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rebuild_refuses_when_vault_is_unreadable() {
    let env = Env::new().await;
    env.ctx.handle.create_question(base_req("qa6-perm-1"), Utc::now()).await.unwrap();
    env.ctx.handle.create_question(base_req("qa6-perm-2"), Utc::now()).await.unwrap();
    env.ctx.handle.answer_question("qa6-perm-2", ui_answer("a"), Utc::now()).await.unwrap();
    let logs = env.ctx.vault.path().join("logs");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&logs, std::fs::Permissions::from_mode(0o000)).unwrap();
    let report = env.ctx.handle.rebuild_db().await;
    std::fs::set_permissions(&logs, std::fs::Permissions::from_mode(0o755)).unwrap();
    let q1 = env.ctx.handle.get_question("qa6-perm-1").await.unwrap();
    let q2 = env.ctx.handle.get_question("qa6-perm-2").await.unwrap();
    assert!(
        q1.is_some() && q2.is_some(),
        "vault 讀不到時重建照樣清空資料庫：report={report:?} q1={} q2={}",
        q1.is_some(),
        q2.is_some()
    );
}

/// 同 R6-11，但讀檔失敗是回錯誤（例如權限被拒 EPERM／EACCES）：重建回錯誤，可是資料庫已經先被清空。
#[tokio::test]
async fn rebuild_error_keeps_database_intact() {
    let env = Env::new().await;
    env.ctx.handle.create_question(base_req("qa6-perm-3"), Utc::now()).await.unwrap();
    let open = env.ctx.handle.ledger().open_json_path();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o000)).unwrap();
    let report = env.ctx.handle.rebuild_db().await;
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o644)).unwrap();
    let q = env.ctx.handle.get_question("qa6-perm-3").await.unwrap();
    assert!(q.is_some(), "重建讀檔失敗（{report:?}），資料庫卻已被清空");
}

// ---------------------------------------------------------------------------
// R6-10：帳本在第 5 輪之前降級時，execution 行直接遺失（沒有待補寫佇列）。這類歷史答案在帳本裡看起來
// 「已回答、從沒處理過」→ 重建後全部重新排進 reactor → 幾十天前的答案再喚醒一次 agent。
// 規格 INV-13：重建永不讓已交付的答案再交付一次。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rebuild_does_not_rewake_old_answers() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let id = "qa6-old";
    let mut req = base_req(id);
    req.wake_on_answer = kunsu_core::types::Wake { mode: kunsu_core::types::WakeMode::Fresh, session_id: None, playbook: None };
    let t_old = Utc::now() - Duration::days(60);
    env.ctx.handle.create_question(req, t_old).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), t_old).await.unwrap();
    env.reactor(Some(claude.clone()), None).process_pending().await.unwrap();
    assert_eq!(env.calls().len(), 1, "前提：60 天前已喚醒並處理");
    // 模擬第 5 輪以前的帳本降級：execution／delivery 行沒寫進去（當時沒有待補寫佇列，錯誤只記 log）。
    let dir = env.ctx.handle.ledger().asks_dir();
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "jsonl") {
            let kept: Vec<String> = std::fs::read_to_string(&p)
                .unwrap()
                .lines()
                .filter(|l| !(l.contains(id) && (l.contains("\"type\":\"execution\"") || l.contains("\"type\":\"delivery\""))))
                .map(str::to_string)
                .collect();
            std::fs::write(&p, kept.join("\n") + "\n").unwrap();
        }
    }
    let (h2, _db) = env.ctx.fresh_db_same_vault().await;
    let report = h2.rebuild_db().await.unwrap();
    assert_eq!(report.stale_unexecuted, 1, "更舊的未處理答案列在報告裡請使用者確認：{report:?}");
    let r2 = env.reactor_on(&h2, Some(claude), None);
    r2.process_pending().await.unwrap();
    assert_eq!(
        env.calls().len(),
        1,
        "60 天前已處理過的答案（帳本缺 execution 行）在重建後又喚醒一次 agent：report={report:?} calls={:?}",
        env.calls()
    );
}
