//! QA 第 9 輪（回答延續原對話，BDD conversation-continuity；INV-9、13、18、25）：回頭驗證第 8 輪的修正——approval 已消耗卻
//! 沒有錢鏈結果的錢題拒絕重新授權、動錢題必須有執行型選項、收割標 `acked_at` 進帳本、拒絕重建列出壞行、卡住告警每次一則、
//! app 沒開時 doctor 數積欠——以及 main 新合進的「退回」（`rejected`）與開新對話 prompt 改走 stdin。
//!
//! 每個測試的斷言寫規格期望；失敗訊息寫實際行為。標「應綠」的是確認修正或新功能成立的對照組。
//! 時間一律注入（`rebuild_db_at`、`answers_since(now)`）；只有 R9-2 用 2 秒的子行程逾時模擬 commit 卡住。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration, Utc};
use common::*;
use kunsu_core::continuity::{DeliveryState, HostKind};
use kunsu_core::policy::Policy;
use kunsu_core::types::*;
use kunsu_server::ledger::{LedgerRow, LedgerType};
use kunsu_server::mcp::{handle_rpc, McpReply};
use kunsu_server::*;
use serde_json::{json, Value};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// 共用
// ---------------------------------------------------------------------------

struct Env {
    ctx: TestCtx,
    root: TempDir,
}

impl Env {
    async fn new() -> Env {
        let ctx = TestCtx::new().await;
        let root = TempDir::new().unwrap();
        for d in ["work", "registry", "desktop/acct/org", "spool"] {
            std::fs::create_dir_all(root.path().join(d)).unwrap();
        }
        Env { ctx, root }
    }
    fn work(&self) -> PathBuf {
        self.root.path().join("work")
    }
    fn spool(&self) -> PathBuf {
        self.root.path().join("spool")
    }
    fn log(&self) -> PathBuf {
        self.root.path().join("calls.log")
    }
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.log()).unwrap_or_default().lines().map(str::to_string).collect()
    }
    /// 假 CLI：把收到的參數（換行改空白）記一行，再執行 `body`。
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
    fn cfg(&self, claude: Option<PathBuf>) -> ReactorConfig {
        let mut cfg = ReactorConfig::new(self.ctx.vault.path().to_path_buf());
        cfg.claude_bin = claude;
        cfg.sessions_dir = self.root.path().join("registry");
        cfg.desktop_sessions_dir = self.root.path().join("desktop");
        cfg.retry_base_ms = 1;
        cfg.trusted_cli_roots.push(self.root.path().to_path_buf());
        cfg
    }
    fn reactor_on(&self, handle: &ServerHandle, claude: Option<PathBuf>) -> Arc<Reactor> {
        Reactor::new(handle.clone(), self.cfg(claude), Arc::new(StaticKeyProvider(test_key())))
    }
    /// Claude 桌面紀錄：這段對話已在 Claude App 封存（→ 開新對話，AC-CC-10）。
    fn archive(&self, host_session_id: &str, cli_id: &str) {
        let body = json!({"cliSessionId": cli_id, "isArchived": true});
        std::fs::write(self.root.path().join(format!("desktop/acct/org/{host_session_id}.json")), body.to_string()).unwrap();
    }
}

const OK_BODY: &str = "echo 'EXEC_OK: 原對話已接手'";

fn claude_source(env: &Env, conversation: &str, host_session: &str, cli_path: Option<&Path>) -> Source {
    Source {
        agent: AgentKind::Claude,
        cwd: env.work().to_string_lossy().to_string(),
        host: Some(HostKind::ClaudeDesktop),
        conversation_id: Some(conversation.into()),
        host_session_id: Some(host_session.into()),
        attended: Some(false),
        cli_path: cli_path.map(|p| p.to_string_lossy().to_string()),
        ..Default::default()
    }
}

fn fresh_wake() -> Wake {
    Wake { mode: WakeMode::Fresh, session_id: None, playbook: None }
}

fn base_req_with(id: &str, source: Source) -> CreateRequest {
    let mut req = base_req(id);
    req.source = source;
    req
}

/// 同 vault、全新空 DB；Discord 拿得到。
async fn fresh_db(ctx: &TestCtx) -> (ServerHandle, Arc<RecordingDiscord>, TempDir) {
    let db_dir = TempDir::new().unwrap();
    let discord = Arc::new(RecordingDiscord::new());
    let handle = ServerHandle::new(KunsuConfig {
        db_path: db_dir.path().join("kunsu.db"),
        vault_root: ctx.vault.path().to_path_buf(),
        policy: Policy::default(),
        discord: discord.clone(),
        guards: Arc::new(StubGuardProbe::new()),
    })
    .await
    .unwrap();
    (handle, discord, db_dir)
}

async fn mcp_on(handle: &ServerHandle, tool: &str, args: Value) -> Result<Value, String> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}});
    match handle_rpc(handle, body).await {
        McpReply::Json(v) => match v.get("error") {
            Some(e) => Err(e["message"].as_str().unwrap_or_default().to_string()),
            None => Ok(v["result"]["structuredContent"].clone()),
        },
        McpReply::Accepted => Err("accepted".into()),
    }
}

/// vault 帳本所有行（依檔名＝日期排序、檔內依寫入順序）。
fn all_rows(ctx: &TestCtx) -> Vec<LedgerRow> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(ctx.handle.ledger().asks_dir())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    files.sort();
    files
        .iter()
        .flat_map(|p| std::fs::read_to_string(p).unwrap().lines().map(str::to_string).collect::<Vec<_>>())
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<LedgerRow>(&l).ok())
        .collect()
}

fn ack_rows(ctx: &TestCtx, id: &str) -> usize {
    all_rows(ctx)
        .iter()
        .filter(|r| r.question.id == id && r.ledger_type == LedgerType::Delivery)
        .filter(|r| r.execution.as_ref().and_then(|e| e.get("acked")).is_some())
        .count()
}

fn kunsu_bin() -> PathBuf {
    let bin = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("kunsu");
    assert!(bin.exists(), "找不到 {}：先跑 `cargo build -p kunsu-cli`（同一個 CARGO_TARGET_DIR）", bin.display());
    bin
}

/// 真的 `kunsu` 執行檔：HOME 指 tempdir、指向測試 server。
struct Cli {
    home: TempDir,
    bind: String,
}

impl Cli {
    async fn new(ctx: &TestCtx) -> Cli {
        let base = spawn_http(ctx).await;
        let home = TempDir::new().unwrap();
        let kunsu_home = home.path().join(".kunsu");
        std::fs::create_dir_all(&kunsu_home).unwrap();
        std::fs::write(kunsu_home.join("token"), TEST_TOKEN).unwrap();
        std::fs::write(kunsu_home.join("admin-token"), TEST_ADMIN_TOKEN).unwrap();
        Cli { home, bind: base.trim_start_matches("http://").to_string() }
    }

    fn command(&self, args: &[&str]) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(kunsu_bin());
        cmd.args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.home.path())
            .env("KUNSU_HOME_DIR", self.home.path().join(".kunsu"))
            .env("KUNSU_BIND", &self.bind)
            .env("KUNSU_VAULT_ROOT", self.home.path().join("vault"))
            .current_dir(self.home.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        cmd
    }
}

/// 假 executor：prepare 依 argv 誠實回填 spec；commit 依 vault 根的 `commit_mode` 檔決定——
/// `ok` 印成功；`hang`＝先打 `/approvals/verify`（消耗 approval，跟真的 executor 一樣在 fire 之前驗證）再卡住。
fn write_mode_executor(ctx: &TestCtx, verify_url: &str) {
    let body = format!(
        r#"#!/usr/bin/env python3
import json, os, sys, time, urllib.request

def flag(name, default=None):
    args = sys.argv[1:]
    return args[args.index(name) + 1] if name in args else default

token = "{FAKE_EXECUTOR_TOKEN}"
if len(sys.argv) > 1 and sys.argv[1] == "commit":
    mode = open("commit_mode").read().strip() if os.path.exists("commit_mode") else "ok"
    with open("commit_calls.log", "a") as f:
        f.write(mode + "\n")
    if mode == "hang":
        rec = json.load(open(flag("--approval-file")))
        req = urllib.request.Request("{verify_url}", data=json.dumps({{"approval_record": rec}}).encode(),
            headers={{"Content-Type": "application/json", "Authorization": "Bearer {TEST_TOKEN}"}})
        out = urllib.request.urlopen(req, timeout=5).read().decode()
        with open("commit_calls.log", "a") as f:
            f.write("verify " + out + "\n")
        sys.stdout.flush()
        time.sleep(30)
    print("commit 完成")
    sys.exit(0)
os.makedirs("projects/meta-ads/data/executor_pending", exist_ok=True)
spec = {{
    "action": "update_budget",
    "entity": flag("--entity"),
    "product": flag("--product"),
    "new_budget": int(flag("--new-budget", "0")),
    "current_budget": "500",
    "tier": "yellow",
    "guards": [],
    "_token": token,
    "prepared_at": 1753900000.0,
}}
with open(f"projects/meta-ads/data/executor_pending/{{token}}.json", "w") as f:
    json.dump(spec, f, ensure_ascii=False)
print(f"     python3 projects/meta-ads/scripts/executor.py commit --token {{token}}")
"#
    );
    let dir = ctx.vault.path().join("projects/meta-ads/scripts");
    std::fs::create_dir_all(&dir).unwrap();
    write_script(&dir, "executor.py", &body);
}

// ---------------------------------------------------------------------------
// R9-1：被擋過一次的錢題，使用者重新授權後第二次的 approval 已被 executor 消耗、commit 途中 app 當掉（這一次沒有任何錢鏈
// 結果）。重建資料庫後，執行紀錄還是第一次的 `EXEC_BLOCKED`，重新授權的檢查只看「有沒有錢鏈結果」，把舊的被擋當成這一次的
// 結果，於是放行——錢可能被動第二次；重建報告也沒列人工對帳。
// 規格：CONTRACT §7「approval 已消耗、卻沒有錢鏈結果的錢題一律拒絕重新授權——錢可能已經動過，只能人工對帳（重建前後都一樣）」；
// AC-CC-14「已執行完成的答案拒絕重新授權，錢不會被再動一次」；INV-9。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa9_reauth_after_rebuild_sees_a_later_consumed_approval_behind_an_old_block() {
    let ctx = TestCtx::new().await;
    let id = "qa9-stale-block";
    let t = Utc::now();
    let mut req = money_t0_req(id);
    req.wake_on_answer = fresh_wake();
    req.deadline_at = Some(t + Duration::hours(6));
    ctx.handle.create_question(req, t).await.unwrap();
    let aid = ctx.handle.answer_question(id, ui_answer("a"), t).await.unwrap().answer.unwrap().answer_id;
    // 第一次：prepare 被護欄擋下（沒有消耗 approval），執行紀錄 EXEC_BLOCKED（reactor 列 done）。
    ctx.handle
        .record_execution(id, "EXEC_BLOCKED: prepare 輸出解析不到 commit token（guard 升紅或 BLOCK？）", t + Duration::minutes(1))
        .await
        .unwrap();
    // 被擋的錢題可以重新授權（對照）。
    ctx.handle.reauthorize_question(id, t + Duration::minutes(10)).await.expect("前提：被擋的錢題可以重新授權");
    // 第二次：簽出新 approval，executor 驗證（消耗）後 commit 途中 app 當掉——這一次沒有任何錢鏈結果。
    let t2 = t + Duration::minutes(10);
    ctx.handle.record_approval_issued(id, &aid, "n-qa9-2", FAKE_EXECUTOR_TOKEN, "h2", t2.timestamp(), t2).await.unwrap();
    let rec = json!({"question_id": id, "nonce": "n-qa9-2", "param_hash": "h2"});
    let consumed = ctx.handle.consume_approval(id, "n-qa9-2", &rec, t2 + Duration::seconds(5)).await.unwrap();
    assert!(matches!(consumed, ApprovalOutcome::Consumed), "前提：第二次的 approval 已被 executor 消耗");
    // 對照：重建前 reactor 列還沒終結，重新授權被拒。
    assert!(ctx.handle.reauthorize_question(id, t2 + Duration::minutes(1)).await.is_err(), "前提：重建前被拒");

    let (h2, discord, _db) = fresh_db(&ctx).await;
    let report = h2.rebuild_db_at(t2 + Duration::minutes(5)).await.unwrap();
    let after = h2.reauthorize_question(id, t2 + Duration::minutes(6)).await;
    let result = h2.get_question(id).await.unwrap().unwrap().answer.unwrap().execution_result;
    let reconcile_alert = discord.messages().iter().any(|m| m.reason == "rebuild_reconcile");
    assert!(
        after.is_err(),
        "第二次 approval 已消耗、沒有結果，重建後重新授權卻放行（被舊的 {result:?} 當成這一次的結果）；重建報告 reconcile={}、人工對帳 Discord={reconcile_alert}",
        report.reconcile
    );
}

// ---------------------------------------------------------------------------
// R9-2：executor 已經驗證（消耗 approval）才卡住、被 30 分鐘逾時中止的 commit，被記成 `EXEC_BLOCKED`——agent 讀到「被安全
// 護欄擋下」，使用者也可以直接重新授權再跑一次。卡住被殺的 commit 錢可能已經動了（executor 在 verify 之後才 fire），不是護欄擋下。
// 規格：AC-CC-14「agent 讀到的錢題狀態依錢鏈結果判斷」「錢不會被再動一次」；CONTRACT §7「approval 已消耗…錢可能已經動過，
// 只能人工對帳」。（逾時不是「錢鏈結果」：嚴重度中，錢；只在 commit 卡超過執行上限時發生）
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa9_commit_killed_after_verify_is_not_a_guard_block() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    write_mode_executor(&ctx, &format!("{base}/approvals/verify"));
    std::fs::write(ctx.vault.path().join("commit_mode"), "hang").unwrap();
    let id = "qa9-commit-hang";
    let t = Utc::now();
    let mut req = money_t0_req(id);
    req.wake_on_answer = fresh_wake();
    req.deadline_at = Some(t + Duration::hours(6));
    ctx.handle.create_question(req, t).await.unwrap();
    ctx.handle.answer_question(id, ui_answer("a"), t).await.unwrap();

    let scratch = TempDir::new().unwrap();
    let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
    cfg.sessions_dir = scratch.path().join("registry");
    cfg.desktop_sessions_dir = scratch.path().join("desktop");
    cfg.claude_bin = Some(PathBuf::from("/usr/bin/true"));
    cfg.retry_base_ms = 1;
    cfg.exec_timeout = std::time::Duration::from_secs(2);
    let r = Reactor::new(ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key())));
    r.process_pending().await.unwrap();

    let log = std::fs::read_to_string(ctx.vault.path().join("commit_calls.log")).unwrap_or_default();
    let consumed = all_rows(&ctx).iter().any(|row| row.question.id == id && row.ledger_type == LedgerType::ApprovalConsumed);
    assert!(consumed && log.contains("\"valid\":true"), "前提：executor 已驗證並消耗 approval：{log}");
    let raw = ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap().execution_result.unwrap_or_default();
    let agent = mcp_on(&ctx.handle, "ask_get", json!({"question_id": id})).await.unwrap();
    let agent_status = agent["question"]["answer"]["execution_result"].as_str().unwrap_or_default().to_string();
    let reauth = ctx.handle.reauthorize_question(id, Utc::now()).await;
    assert!(
        reauth.is_err() && !agent_status.contains("護欄"),
        "verify 之後被逾時中止的 commit：執行紀錄 {raw:?}，agent 讀到 {agent_status:?}，重新授權 {}（錢可能已經動過）",
        if reauth.is_ok() { "放行" } else { "被拒" }
    );
}

// ---------------------------------------------------------------------------
// R9-3（應綠）：新的錢題規則（choice＋執行型選項）不擋合法錢題：一般黃燈錢題、紅燈錢題、Kongko 啟用題（資料充分→黃燈、
// 資料不足→紅燈）都建得出來；純確認、文字題、沒有執行型選項的錢題拒收並說明。AC-CC-14；CONTRACT §2。
// ---------------------------------------------------------------------------

fn publication(pub_id: &str, batch: &str, ads: Value) -> KongkoPublication {
    serde_json::from_value(json!({
        "type": "ad_publication_confirmed", "publicationId": pub_id, "batchId": batch, "product": "nova",
        "ads": ads, "confirmedAt": "2026-10-09T06:30:00Z"
    }))
    .unwrap()
}

#[tokio::test]
async fn qa9_money_rule_keeps_legal_money_and_kongko_activation() {
    let ctx = TestCtx::new().await;
    let t = Utc::now();
    ctx.handle.create_question(money_t0_req("qa9-money-yellow"), t).await.expect("一般黃燈錢題");
    let mut red = money_t0_req("qa9-money-red");
    red.gate = Some(Gate::Red);
    ctx.handle.create_question(red, t).await.expect("紅燈錢題（有執行型選項）");

    for (n, ads) in [
        (1, json!([{"campaign_id": "120200000000001", "daily_budget_twd": 300}])),
        (2, json!([])),
    ] {
        let batch = format!("qa9-batch-{n}");
        let mut material = base_req(&format!("qa9-material-{n}"));
        material.kind = Kind::KongkoReview;
        material.options = vec![];
        material.kongko_ref = Some(batch.clone());
        ctx.handle.create_question(material, t).await.unwrap();
        let out = ctx.handle.kongko_event(publication(&format!("pub-{n}"), &batch, ads), t + Duration::minutes(1)).await.unwrap();
        let act = out.activation_question_id.unwrap_or_else(|| {
            panic!("Kongko 啟用題 {n} 沒有建出來：{:?}", ctx.discord.messages().iter().map(|m| m.text.clone()).collect::<Vec<_>>())
        });
        let q = ctx.handle.get_question(&act).await.unwrap().unwrap();
        assert!(q.is_money_op() && q.kind == Kind::Choice && q.options.iter().any(|o| o.recommended == Some(true)));
        assert_eq!(q.gate, Some(if n == 1 { Gate::Yellow } else { Gate::Red }), "啟用題 {n} 的燈號");
    }

    let mut no_exec = money_t0_req("qa9-money-no-exec");
    no_exec.options.iter_mut().for_each(|o| o.recommended = None);
    let mut text = money_t0_req("qa9-money-text");
    text.kind = Kind::Text;
    text.options = vec![];
    text.gate = None;
    for req in [no_exec, text] {
        let id = req.id.clone();
        let err = ctx.handle.create_question(req, t).await.expect_err("沒有執行型選項的錢題要拒收").to_string();
        assert!(err.contains("執行型選項"), "{id} 拒收訊息要說明：{err}");
    }
}

// ---------------------------------------------------------------------------
// R9-4：agent 只用 MCP 建錢題、沒帶原對話身分也沒設 `wake_on_answer`（預設 none）——使用者按「批准」後 reactor 根本沒有排進
// 這個答案（狀態機只在 wake≠none 時 SpawnReactor），錢鏈不會跑；agent 讀到的錢題狀態永遠是「錢鏈還在跑，以 Kunsu 回執為準」。
// 和第 8 輪擋下的「純確認錢題」同一類問題：使用者以為批准了、錢鏈永遠不跑、agent 永遠在等。
// 規格：CONTRACT §6／§8「使用者點批准 → server spawn prepare…reactor 跑 commit」「錢鏈先跑（§6），再走交付」；AC-CC-14
// 「狀態文字依錢鏈結果開頭判斷」；第 8 輪 CONTRACT §2「按下去錢鏈永遠不會跑，會讓 agent 誤以為已批准」。
// （money-approval 的相鄰問題；嚴重度中：錢沒動，但使用者的批准被靜默吞掉）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa9_money_question_without_wake_still_runs_the_chain() {
    let ctx = TestCtx::new().await;
    write_fake_executor(&ctx);
    let id = "qa9-money-no-wake";
    let t = Utc::now();
    let mut req = money_t0_req(id);
    req.deadline_at = Some(t + Duration::hours(6));
    assert_eq!(req.wake_on_answer.mode, WakeMode::None, "前提：agent 沒設 wake_on_answer");
    let created = ctx.handle.create_question(req, t).await;
    if created.is_err() {
        return; // 建題就拒收也符合規格（不會讓使用者以為批准了）。
    }
    ctx.handle.answer_question(id, ui_answer("a"), t).await.unwrap();
    let scratch = TempDir::new().unwrap();
    let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
    cfg.sessions_dir = scratch.path().join("registry");
    cfg.desktop_sessions_dir = scratch.path().join("desktop");
    cfg.claude_bin = Some(PathBuf::from("/usr/bin/true"));
    cfg.retry_base_ms = 1;
    let r = Reactor::new(ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key())));
    r.process_pending().await.unwrap();
    r.process_due_deliveries(Utc::now() + Duration::seconds(5)).await.unwrap();
    let kinds: Vec<String> = r.planned().iter().map(|c| c.kind.clone()).collect();
    let agent = mcp_on(&ctx.handle, "ask_get", json!({"question_id": id})).await.unwrap();
    let status = agent["question"]["answer"]["execution_result"].as_str().unwrap_or_default().to_string();
    let delivery = agent["question"]["answer"]["delivery"]["state"].clone();
    assert!(
        kinds.iter().any(|k| k == "commit"),
        "使用者批准了黃燈錢題，錢鏈卻沒跑（reactor 命令 {kinds:?}，交付 {delivery}）；agent 讀到 {status:?}"
    );
}

// ---------------------------------------------------------------------------
// R9-5（應綠）：Lane 1 收割先於 reactor 標 `acked_at`（帳本多一行 ack）——重建後 acked_at 與 routed 都在，答案照常排進 reactor
// 一次、只開一次新對話；再重建一次不再排、不再開；重建後重新收割（游標隨資料庫清空，at-least-once 重送舊事件）不再多寫 ack 行。
// AC-CC-11、AC-CC-16；INV-13；CONTRACT §7「收割第一次標 acked_at 時追加一行 delivery 帳本行」。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa9_harvest_ack_row_never_causes_a_second_delivery() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let id = "qa9-ack-row";
    let t_answer = Utc::now() - Duration::hours(1);
    let mut req = base_req(id);
    req.wake_on_answer = fresh_wake();
    env.ctx.handle.create_question(req, t_answer).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("b"), t_answer).await.unwrap();
    let first = env.ctx.handle.answers_since("mkt-daily", None, t_answer + Duration::minutes(1)).await.unwrap();
    assert_eq!(first.events.len(), 1, "前提：收割到這題");
    assert_eq!(ack_rows(&env.ctx, id), 1, "第一次收割標 acked_at 要追加一行 ack 帳本行");

    let (h2, _d2, _db2) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db_at(t_answer + Duration::minutes(5)).await.unwrap();
    let q2 = h2.get_question(id).await.unwrap().unwrap();
    assert!(q2.answer.as_ref().unwrap().acked_at.is_some() && q2.state == State::Routed, "重建後 acked_at／routed 要在：{:?}", q2.state);
    assert_eq!(report.requeued, 1, "收割不算交付：還沒開新對話的答案重建後照常排進 reactor：{report:?}");
    env.reactor_on(&h2, Some(claude.clone())).process_pending().await.unwrap();
    assert_eq!(env.calls().len(), 1, "只開一次新對話：{:?}", env.calls());

    let (h3, _d3, _db3) = fresh_db(&env.ctx).await;
    let report3 = h3.rebuild_db_at(Utc::now()).await.unwrap();
    assert_eq!(report3.requeued, 0, "已開過新對話的答案不再排：{report3:?}");
    env.reactor_on(&h3, Some(claude.clone())).process_pending().await.unwrap();
    assert_eq!(env.calls().len(), 1, "重建後不再開新對話：{:?}", env.calls());

    let before = ack_rows(&env.ctx, id);
    let again = h3.answers_since("mkt-daily", None, Utc::now()).await.unwrap();
    eprintln!("R9-5 重建後重新收割 consumer=mkt-daily：{} 筆舊事件重送（游標在資料庫裡，重建後從 0 開始）", again.events.len());
    assert_eq!(ack_rows(&env.ctx, id), before, "重新收割不能再多寫 ack 行");
    let q3 = h3.get_question(id).await.unwrap().unwrap();
    assert_eq!(q3.answer.unwrap().delivery.map(|d| d.state), Some(DeliveryState::Fresh));
}

// ---------------------------------------------------------------------------
// R9-6：使用者按「這題不該問我」退回——等待中的 agent（`ask_wait`／`kunsu wait`）只拿到 `{"status":"cancelled"}`，沒有退回標記也沒有
// 原因，分不出是使用者推掉還是別的 agent 取消；`kunsu wait` 以結束碼 0 結束。要另外 `ask_get` 才看得到 `rejected`。
// 規格：CONTRACT §9「`ask_get` 讀得到，agent 知道使用者推掉這題與原因」（成立）；answer-flow 場景 5／AC-AN-06「agent 經
// `ask_wait`／`ask_answers_since` 能看到退回狀態與理由」（ask_wait 不成立）。排程 check-in 的 SOP 正是背景 `kunsu wait`。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa9_waiting_agent_learns_ray_rejected_the_question() {
    let env = Env::new().await;
    let cli = Cli::new(&env.ctx).await;
    let id = "rt-diary-qa9-rej";
    let mut source = claude_source(&env, "conv-qa9-rej", "local_qa9-rej", None);
    source.routine_id = Some("diary".into());
    source.routine_name = Some("每天寫日記".into());
    source.expected_every_hours = Some(24.0);
    let mut req = base_req_with(id, source);
    req.kind = Kind::Text;
    req.options = vec![];
    req.gate = None;
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    env.ctx.handle.reject_question(id, Some("今天休假，不用寫".into()), Utc::now()).await.unwrap();

    // 對照：ask_get 與收割讀得到退回與原因。
    let got = mcp_on(&env.ctx.handle, "ask_get", json!({"question_id": id})).await.unwrap();
    assert_eq!(got["question"]["rejected"]["reason"], "今天休假，不用寫");
    let harvested = env.ctx.handle.answers_since("routine-harvest", None, Utc::now()).await.unwrap();
    let ev = harvested.events.iter().find(|e| e.question.id == id).expect("收割到退回的題");
    assert_eq!(ev.question.state, State::Cancelled, "收割後仍是 cancelled");
    assert!(ev.question.rejected.is_some(), "收割事件帶著 rejected");

    let waited = mcp_on(&env.ctx.handle, "ask_wait", json!({"question_id": id, "timeout_seconds": 1})).await.unwrap();
    let out = cli.command(&["wait", id, "--timeout", "5"]).output().await.unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        waited.to_string().contains("今天休假") && stdout.contains("今天休假"),
        "等待中的 agent 拿到的結果沒有退回與原因：ask_wait={waited}；kunsu wait（結束碼 {:?}）={stdout}",
        out.status.code()
    );
}

// ---------------------------------------------------------------------------
// R9-7（應綠）：開新對話的 prompt 走 stdin——原對話已封存 → 開新對話；假 claude 每次都讀完 stdin 後以 API 過載失敗：重試 3 次、
// 每次都從 stdin 拿到完整 prompt（含 answer_id、「## 原對話」與封存原因），命令列參數沒有 prompt；耗盡 → 交付 dead、死信
// `EXEC_FAILED: 死信…`、Discord；之後來的 `kunsu wait` 拿到答案與 dead，結束碼 0（交付失敗由等待者處理）。
// AC-CC-10、AC-CC-18；CONTRACT §8 第 5 步。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa9_fresh_prompt_through_stdin_retries_dead_letters_and_wait_exits_zero() {
    let env = Env::new().await;
    let spool = env.spool();
    let claude = env.script(
        "claude-fail.sh",
        &format!(
            "n=$(ls {d}/fail-*.txt 2>/dev/null | wc -l | tr -d ' ')\ncat > {d}/fail-$n.txt\necho 'API Error: 529 overloaded' >&2\nexit 1",
            d = spool.display()
        ),
    );
    let cli = Cli::new(&env.ctx).await;
    let id = "qa9-stdin-dead";
    env.archive("local_qa9-arch", "conv-qa9-arch");
    let req = base_req_with(id, claude_source(&env, "conv-qa9-arch", "local_qa9-arch", Some(&claude)));
    env.ctx.handle.create_question(req, Utc::now()).await.unwrap();
    let aid = env.ctx.handle.answer_question(id, ui_answer("b"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    let r = env.reactor_on(&env.ctx.handle, Some(claude.clone()));
    r.process_pending().await.unwrap();

    let mut inputs: Vec<String> = std::fs::read_dir(&spool)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("fail-"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .collect();
    inputs.sort();
    assert_eq!(inputs.len(), 3, "重試 3 次，每次都要從 stdin 拿到 prompt：{}", inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        assert!(
            input.contains(&aid) && input.contains("## 原對話") && input.contains("原對話已封存") && input.contains("## 題目與答案"),
            "第 {} 次 stdin 不完整（{} bytes）",
            i + 1,
            input.len()
        );
    }
    let calls = env.calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(calls.iter().all(|c| c.contains("--input-format text") && !c.contains(&aid)), "prompt 不在命令列參數：{calls:?}");
    let q = env.ctx.handle.get_question(id).await.unwrap().unwrap();
    let a = q.answer.unwrap();
    assert_eq!(a.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Dead));
    assert!(a.execution_result.as_deref().unwrap_or_default().starts_with("EXEC_FAILED: 死信"), "{:?}", a.execution_result);
    assert!(env.ctx.discord.messages().iter().any(|m| m.reason == "reactor_dead_letter"), "死信要發 Discord");

    let out = cli.command(&["wait", id, "--timeout", "5"]).output().await.unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code(), Some(0), "交付失敗（dead）照常結束碼 0：{stdout}");
    assert!(stdout.contains("\"dead\""), "{stdout}");
}

// ---------------------------------------------------------------------------
// R9-8（應綠）：開新對話前兩次失敗、第三次成功——三次都從 stdin 拿到同一份完整 prompt；交付 fresh（帶原因），之後的
// `kunsu wait` 結束碼 5（不要重複處理）。AC-CC-10、AC-CC-18。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa9_fresh_prompt_through_stdin_survives_transient_failures() {
    let env = Env::new().await;
    let spool = env.spool();
    let claude = env.script(
        "claude-flaky.sh",
        &format!(
            "n=$(ls {d}/flaky-*.txt 2>/dev/null | wc -l | tr -d ' ')\ncat > {d}/flaky-$n.txt\nif [ \"$n\" -lt 2 ]; then echo 'API Error: 529 overloaded' >&2; exit 1; fi\necho 'EXEC_OK: 新對話已處理'",
            d = spool.display()
        ),
    );
    let cli = Cli::new(&env.ctx).await;
    let id = "qa9-stdin-flaky";
    env.archive("local_qa9-flaky", "conv-qa9-flaky");
    env.ctx
        .handle
        .create_question(base_req_with(id, claude_source(&env, "conv-qa9-flaky", "local_qa9-flaky", Some(&claude))), Utc::now())
        .await
        .unwrap();
    let aid = env.ctx.handle.answer_question(id, ui_answer("b"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    env.reactor_on(&env.ctx.handle, Some(claude.clone())).process_pending().await.unwrap();

    let inputs: Vec<String> = (0..3).map(|n| std::fs::read_to_string(spool.join(format!("flaky-{n}.txt"))).unwrap_or_default()).collect();
    assert!(inputs.iter().all(|i| i.contains(&aid) && i == &inputs[0]), "三次 stdin 要是同一份完整 prompt：{:?}", inputs.iter().map(String::len).collect::<Vec<_>>());
    let a = env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap();
    assert_eq!(a.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Fresh), "{:?}", a.delivery);
    assert_eq!(a.execution_result.as_deref(), Some("EXEC_OK: 新對話已處理"));
    let out = cli.command(&["wait", id, "--timeout", "5"]).output().await.unwrap();
    assert_eq!(out.status.code(), Some(5), "{}", String::from_utf8_lossy(&out.stdout));
}

// ---------------------------------------------------------------------------
// R9-9：只核可（`editable: false`）項目的 AI 建議沒有長度上限；Claude 原對話追加的 prompt 仍放在命令列參數（`-p <prompt>`），
// 「採用：照你的建議執行：<proposal>」逐項附上，20 項長建議就超過 macOS ARG_MAX（1,048,576 bytes）——spawn 失敗被當成暫時性錯誤
// 重試 3 次，最後改開新對話，答案沒有回到原對話。開新對話已改走 stdin，追加這條沒有。
// 規格：AC-CC-06「已結束對話以 resume 追加」；PRD §25「題組…AI 依指示逐項執行」。（嚴重度低：要 20 項×數萬字的只核可建議）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa9_resume_prompt_for_long_approve_only_batch_reaches_the_original_conversation() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let id = "qa9-big-approve-only";
    // 2026-10-09 主線決定：只核可的建議也 ≤5,000 字（CONTRACT §14），超過的建題就拒收。
    let batch = |id: &str, proposal: &str| {
        let items: Vec<Value> = (1..=20)
            .map(|n| json!({"id": format!("r{n}"), "title": format!("退款申請 {n}"), "subtitle": "退".repeat(200), "context_md": "購買 3 天、進度 8%。", "proposal": proposal, "editable": false}))
            .collect();
        let mut v = serde_json::to_value(base_req(id)).unwrap();
        let o = v.as_object_mut().unwrap();
        o.insert("kind".into(), json!("batch"));
        o.insert("gate".into(), Value::Null);
        o.insert("options".into(), json!([]));
        o.insert("items".into(), Value::Array(items));
        o.insert("source".into(), serde_json::to_value(claude_source(&env, "conv-qa9-big", "local_qa9-big", Some(&claude))).unwrap());
        serde_json::from_value::<CreateRequest>(v).unwrap()
    };
    let too_long = format!("符合退費規則 → 建議全額退款。理由：{}", "x".repeat(60_000));
    let err = env.ctx.handle.create_question(batch("qa9-too-long", &too_long), Utc::now()).await.expect_err("超過 5,000 字的建議要拒收");
    assert!(err.to_string().contains("5,000") || err.to_string().contains("5000"), "{err}");
    let mut long_subtitle = batch("qa9-long-subtitle", "建議全額退款");
    long_subtitle.items[0].subtitle = Some("退".repeat(201));
    let err = env.ctx.handle.create_question(long_subtitle, Utc::now()).await.expect_err("副標超過 200 字要拒收");
    assert!(err.to_string().contains("200"), "{err}");
    // 上限內最大的題組（每項 5,000 個中文字、副標 200 字）照常回到原對話。
    let proposal = format!("建議全額退款：{}", "退".repeat(5_000 - 7));
    env.ctx.handle.create_question(batch(id, &proposal), Utc::now()).await.expect("上限內的只核可題組");
    let q = serde_json::to_value(env.ctx.handle.get_question(id).await.unwrap().unwrap()).unwrap();
    let decisions: Vec<Value> = q["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| json!({"item_id": i["id"], "action": "approve", "content_hash": i["content_hash"]}))
        .collect();
    let input: AnswerInput = serde_json::from_value(json!({"channel": "ui", "items": decisions})).unwrap();
    env.ctx.handle.answer_question(id, input, Utc::now()).await.expect("全部照建議");
    let r = env.reactor_on(&env.ctx.handle, Some(claude.clone()));
    r.process_pending().await.unwrap();
    let resume_bytes: usize = r
        .planned()
        .iter()
        .find(|c| c.kind == "resume_claude")
        .map(|c| c.args.iter().map(|a| a.len() + 1).sum())
        .unwrap_or(0);
    let d = env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap().delivery.unwrap();
    assert_eq!(
        d.state,
        DeliveryState::Resumed,
        "追加原對話的命令列參數 {resume_bytes} bytes（ARG_MAX 1,048,576），交付變成 {:?}（{:?}）",
        d.state,
        d.reason
    );
}

// ---------------------------------------------------------------------------
// R9-10（應綠）：app 正常結束（WAL 已 checkpoint、`-wal`／`-shm` 已刪）後，doctor 唯讀打開資料庫仍數得到帳本積欠，不必退到
// 「無法確認」。AC-CC-11「`kunsu doctor` 看得到帳本積欠」；CONTRACT §5。
// 觀察（不算失敗）：唯讀打開會在資料庫旁留下空的 `kunsu.db-wal`／`kunsu.db-shm`（`offline_ledger_backlog` 註解寫「不建檔」）；
// 檔案是空的，下次 app 開啟照常使用，不影響資料。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa9_offline_backlog_reads_a_cleanly_closed_database() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("kunsu.db");
    let pool = kunsu_server::db::open_pool(&path).await.unwrap();
    kunsu_server::store::queue_ledger_row(&pool, "{}", Utc::now()).await.unwrap();
    kunsu_server::store::queue_ledger_row(&pool, "{}", Utc::now()).await.unwrap();
    pool.close().await;
    let listing = |d: &Path| {
        let mut v: Vec<String> = std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        v.sort();
        v
    };
    let before = listing(dir.path());
    let n = kunsu_server::db::offline_ledger_backlog(&path).await;
    let after = listing(dir.path());
    assert_eq!(n.as_ref().ok(), Some(&2), "正常關閉後的資料庫數不到積欠：{n:?}（檔案 {before:?}）");
    let sidecars: Vec<(String, u64)> = after
        .iter()
        .filter(|f| !before.contains(f))
        .map(|f| (f.clone(), std::fs::metadata(dir.path().join(f)).map(|m| m.len()).unwrap_or(0)))
        .collect();
    eprintln!("R9-10 唯讀檢查後新出現的檔案（名稱, bytes）：{sidecars:?}");
    // `-shm` 是 SQLite 的共享記憶體索引（32 KB），不是資料；`-wal` 必須是空的（唯讀檢查沒有寫入任何頁）。
    assert!(
        sidecars.iter().filter(|(f, _)| f.ends_with("-wal")).all(|(_, len)| *len == 0),
        "唯讀檢查不能寫入任何資料：{sidecars:?}"
    );
}

// ---------------------------------------------------------------------------
// R9-11（應綠）：`kunsu doctor` 的帳本積欠——(a) app 版本較舊、`/health` 沒有 `ledger_backlog` 欄位 →「無法確認」，不當成 0；
// (b) app 沒開、資料庫也打不開 →「無法確認，刪資料庫前先打開 Kunsu 再跑一次」；(c) app 沒開、資料庫正常關閉、有 2 行積欠 → 印出 2 行。
// AC-CC-11；CONTRACT §5。
// ---------------------------------------------------------------------------

async fn run_doctor(bind: &str, db_path: &Path) -> String {
    let home = TempDir::new().unwrap();
    let kunsu_home = home.path().join(".kunsu");
    std::fs::create_dir_all(&kunsu_home).unwrap();
    std::fs::write(kunsu_home.join("token"), TEST_TOKEN).unwrap();
    std::fs::write(kunsu_home.join("admin-token"), TEST_ADMIN_TOKEN).unwrap();
    let out = tokio::process::Command::new(kunsu_bin())
        .arg("doctor")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.path())
        .env("KUNSU_HOME_DIR", &kunsu_home)
        .env("KUNSU_BIND", bind)
        .env("KUNSU_VAULT_ROOT", home.path().join("vault"))
        .env("KUNSU_DB_PATH", db_path)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.contains("ledger_backlog")).collect::<Vec<_>>().join("\n")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa9_doctor_backlog_line_for_old_app_missing_db_and_closed_db() {
    // (a) 舊版 app：/health 只有舊欄位。
    let old = axum::Router::new().route(
        "/health",
        axum::routing::get(|| async { axum::Json(json!({"version": "0.0.9", "uptime_s": 1, "pending_count": 0})) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let old_bind = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move { axum::serve(listener, old).await.unwrap() });
    let scratch = TempDir::new().unwrap();
    let line_a = run_doctor(&old_bind, &scratch.path().join("none.db")).await;
    // (b)(c) app 沒開：找一個沒人聽的 port。
    let down = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().to_string()
    };
    let line_b = run_doctor(&down, &scratch.path().join("missing.db")).await;
    let closed = scratch.path().join("kunsu.db");
    let pool = kunsu_server::db::open_pool(&closed).await.unwrap();
    kunsu_server::store::queue_ledger_row(&pool, "{}", Utc::now()).await.unwrap();
    kunsu_server::store::queue_ledger_row(&pool, "{}", Utc::now()).await.unwrap();
    pool.close().await;
    let line_c = run_doctor(&down, &closed).await;
    eprintln!("R9-11 doctor：\n(a) {line_a}\n(b) {line_b}\n(c) {line_c}");
    assert!(line_a.contains("無法確認"), "舊版 app：{line_a}");
    assert!(line_b.contains("無法確認") && line_b.contains("刪資料庫前先打開 Kunsu"), "資料庫打不開：{line_b}");
    assert!(line_c.contains("2 行"), "app 沒開、資料庫正常關閉：{line_c}");
    assert!(!scratch.path().join("missing.db").exists(), "doctor 不能替不存在的資料庫建檔");
}
