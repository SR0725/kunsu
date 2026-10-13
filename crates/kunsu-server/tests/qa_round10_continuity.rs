//! QA 第 10 輪（回答延續原對話，BDD conversation-continuity；錢的安全 INV-9、15、25 與 AC-CC-14）：回頭驗證第 9 輪的修正——
//! 「錢的結果不明」（最近一次 approval 消耗晚於最近一次錢鏈結果，或 `EXEC_UNKNOWN`）拒絕重新授權、重建時列人工對帳；commit 在
//! nonce 已消耗後失敗記 `EXEC_UNKNOWN`；錢題不論 wake 都進 reactor；等待者讀得到退回——並找新問題。
//!
//! 錢鏈一律走真的簽章鏈：假 executor 的 commit 跟真的 executor 一樣，先打 `/approvals/verify`（消耗 approval）才 fire；
//! 模式寫在 vault 根的 `commit_mode`：`ok`（驗證後成功）、`guard`（驗證後被 executor 護欄擋下、非零結束）、`before`（沒驗證就
//! 失敗）、`hang`（驗證後卡住，用來模擬 app 在 commit 途中當掉）。
//!
//! 每個測試的斷言寫規格期望；失敗訊息寫實際行為。標「應綠」的是確認修正成立的對照組。時間一律注入或用牆鐘的先後順序；
//! 「app 當掉」以 8 秒內丟棄 reactor 的 future 模擬（子行程 kill_on_drop），再開一個新的 reactor 當作重啟。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration, Utc};
use common::*;
use kunsu_core::continuity::{DeliveryState, HostKind, ReportStatus};
use kunsu_core::policy::Policy;
use kunsu_core::types::*;
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
        for d in ["work", "registry", "desktop/acct/org"] {
            std::fs::create_dir_all(root.path().join(d)).unwrap();
        }
        Env { ctx, root }
    }
    fn work(&self) -> PathBuf {
        self.root.path().join("work")
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
    fn reactor_on(&self, handle: &ServerHandle, claude: Option<PathBuf>) -> Arc<Reactor> {
        let mut cfg = ReactorConfig::new(self.ctx.vault.path().to_path_buf());
        cfg.claude_bin = claude;
        cfg.sessions_dir = self.root.path().join("registry");
        cfg.desktop_sessions_dir = self.root.path().join("desktop");
        cfg.retry_base_ms = 1;
        cfg.trusted_cli_roots.push(self.root.path().to_path_buf());
        Reactor::new(handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key())))
    }
    /// Claude 登記檔：這段對話仍被活行程（就是測試行程本身）持有（→ held，AC-CC-07）。
    fn hold(&self, conversation: &str) -> PathBuf {
        let path = self.root.path().join("registry/held.json");
        let body = json!({"sessionId": conversation, "pid": std::process::id()});
        std::fs::write(&path, body.to_string()).unwrap();
        path
    }
    /// Claude 桌面紀錄：這段對話已在 Claude App 封存（→ 開新對話，AC-CC-10）。
    fn archive(&self, host_session_id: &str, cli_id: &str) {
        let body = json!({"cliSessionId": cli_id, "isArchived": true});
        std::fs::write(self.root.path().join(format!("desktop/acct/org/{host_session_id}.json")), body.to_string()).unwrap();
    }
}

const OK_BODY: &str = "echo 'EXEC_OK: 原對話已接手'";
const FAIL_BODY: &str = "echo 'API Error: 529 overloaded' >&2\nexit 1";

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

fn money_req(id: &str, source: Option<Source>, wake: WakeMode) -> CreateRequest {
    let mut req = money_t0_req(id);
    req.deadline_at = Some(Utc::now() + Duration::hours(6));
    if let Some(s) = source {
        req.source = s;
    }
    req.wake_on_answer = Wake { mode: wake, session_id: None, playbook: None };
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

/// agent 讀到的錢題狀態（`ask_get`）。
async fn agent_money(handle: &ServerHandle, id: &str) -> String {
    let got = mcp_on(handle, "ask_get", json!({"question_id": id})).await.unwrap();
    got["question"]["answer"]["execution_result"].as_str().unwrap_or_default().to_string()
}

async fn raw_result(handle: &ServerHandle, id: &str) -> String {
    handle.get_question(id).await.unwrap().unwrap().answer.and_then(|a| a.execution_result).unwrap_or_default()
}

async fn delivery_state(handle: &ServerHandle, id: &str) -> Option<DeliveryState> {
    handle.get_question(id).await.unwrap().unwrap().answer.and_then(|a| a.delivery).map(|d| d.state)
}

/// shell `to_ui` 的判準（src-tauri/src/ipc.rs execution_failure）：以 EXEC_BLOCKED:／EXEC_FAILED: 開頭就進失敗畫面。
fn ui_failed(exec: &str) -> bool {
    exec.starts_with("EXEC_BLOCKED:") || exec.starts_with("EXEC_FAILED:")
}

/// 假 executor：prepare 依 argv 誠實回填 spec；commit 依 vault 根的 `commit_mode` 決定（見檔頭）。
fn write_mode_executor(ctx: &TestCtx, verify_url: &str) {
    let body = format!(
        r#"#!/usr/bin/env python3
import json, os, sys, time, urllib.request

def flag(name, default=None):
    args = sys.argv[1:]
    return args[args.index(name) + 1] if name in args else default

def log(line):
    with open("commit_calls.log", "a") as f:
        f.write(line + "\n")

token = "{FAKE_EXECUTOR_TOKEN}"
if len(sys.argv) > 1 and sys.argv[1] == "commit":
    mode = open("commit_mode").read().strip() if os.path.exists("commit_mode") else "ok"
    log("commit " + mode)
    if mode == "before":
        sys.stderr.write("executor 設定檔讀不到，沒有驗證就結束\n")
        sys.exit(1)
    rec = json.load(open(flag("--approval-file")))
    req = urllib.request.Request("{verify_url}", data=json.dumps({{"approval_record": rec}}).encode(),
        headers={{"Content-Type": "application/json", "Authorization": "Bearer {TEST_TOKEN}"}})
    out = urllib.request.urlopen(req, timeout=5).read().decode()
    log("verify " + out)
    if '"valid":true' not in out:
        sys.stderr.write("verify 失敗：" + out + "\n")
        sys.exit(3)
    if mode == "guard":
        # 真的 executor 被擋時也可能印出 token 與 approval 檔路徑：Kunsu 要先遮掉才進事件、Discord 與執行紀錄。
        sys.stderr.write(f"BLOCKED: 3 天冷卻期未滿（commit --token {{token}} --approval-file {{flag('--approval-file')}}）\n")
        sys.exit(2)
    if mode == "hang":
        sys.stdout.flush()
        time.sleep(30)
    log("fired")
    print("commit 完成：日預算已更新")
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

/// 重建後的資料庫也要有自己的 `/approvals/verify`（executor 驗證走 app 活體 DB）。
async fn spawn_http_on(handle: &ServerHandle) -> String {
    let state = AppState {
        handle: handle.clone(),
        token: TEST_TOKEN.to_string(),
        admin_token: TEST_ADMIN_TOKEN.to_string(),
        key: Arc::new(StaticKeyProvider(test_key())),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    format!("http://{addr}")
}

fn set_mode(ctx: &TestCtx, mode: &str) {
    std::fs::write(ctx.vault.path().join("commit_mode"), mode).unwrap();
}

fn commit_log(ctx: &TestCtx) -> String {
    std::fs::read_to_string(ctx.vault.path().join("commit_calls.log")).unwrap_or_default()
}

fn verified(ctx: &TestCtx) -> usize {
    commit_log(ctx).lines().filter(|l| l.starts_with("verify") && l.contains("\"valid\":true")).count()
}

fn fired(ctx: &TestCtx) -> usize {
    commit_log(ctx).lines().filter(|l| *l == "fired").count()
}

/// 模擬 app 在 commit 途中當掉：reactor 的 future 在 executor 驗證（消耗 approval）之後被丟棄，子行程跟著被殺。
async fn crash_during_commit(r: &Arc<Reactor>, ctx: &TestCtx) {
    let before = verified(ctx);
    let out = tokio::time::timeout(std::time::Duration::from_secs(8), r.process_pending()).await;
    assert!(out.is_err(), "前提：commit 卡住時 reactor 還沒結束（模擬當掉）");
    assert_eq!(verified(ctx), before + 1, "前提：executor 已驗證並消耗 approval：{}", commit_log(ctx));
}

fn kunsu_bin() -> PathBuf {
    let bin = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("kunsu");
    assert!(bin.exists(), "找不到 {}：先跑 `cargo build -p kunsu-cli`（同一個 CARGO_TARGET_DIR）", bin.display());
    bin
}

/// 真的 `kunsu wait`：HOME 指 tempdir、指向測試 server。
async fn kunsu_wait(base: &str, id: &str) -> (Option<i32>, String) {
    let home = TempDir::new().unwrap();
    let kunsu_home = home.path().join(".kunsu");
    std::fs::create_dir_all(&kunsu_home).unwrap();
    std::fs::write(kunsu_home.join("token"), TEST_TOKEN).unwrap();
    std::fs::write(kunsu_home.join("admin-token"), TEST_ADMIN_TOKEN).unwrap();
    let out = tokio::process::Command::new(kunsu_bin())
        .args(["wait", id, "--timeout", "5"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.path())
        .env("KUNSU_HOME_DIR", &kunsu_home)
        .env("KUNSU_BIND", base.trim_start_matches("http://"))
        .env("KUNSU_VAULT_ROOT", home.path().join("vault"))
        .current_dir(home.path())
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .unwrap();
    (out.status.code(), String::from_utf8_lossy(&out.stdout).to_string())
}

// ---------------------------------------------------------------------------
// R10-1（應綠）：正常 commit（executor 先驗證消耗 approval 才 fire）的錢題——原對話追加、開新對話、wake=none、交付死信四種——
// 重建資料庫後不被誤列人工對帳：`consumed_at` 由帳本行 ts 還原、`executed_at` 由快照還原，先後順序不變；重新授權一律以「已經執行
// 完成」拒絕；agent 讀到「已執行完成」；不重新排入 reactor。AC-CC-11、AC-CC-14；CONTRACT §7。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa10_committed_money_is_never_listed_for_reconcile_after_rebuild() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_mode_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "ok");
    let ok = env.script("claude-ok.sh", OK_BODY);
    let fail = env.script("claude-fail.sh", FAIL_BODY);
    let h = &env.ctx.handle;

    let ids = ["qa10-ok-resume", "qa10-ok-fresh", "qa10-ok-none", "qa10-ok-dead"];
    h.create_question(money_req(ids[0], Some(claude_source(&env, "conv-qa10-ok", "local_qa10-ok", Some(&ok))), WakeMode::Continue), Utc::now())
        .await
        .unwrap();
    h.create_question(money_req(ids[1], None, WakeMode::Fresh), Utc::now()).await.unwrap();
    h.create_question(money_req(ids[2], None, WakeMode::None), Utc::now()).await.unwrap();
    for id in &ids[..3] {
        h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    }
    let r = env.reactor_on(h, Some(ok.clone()));
    r.process_pending().await.unwrap();
    r.process_due_deliveries(Utc::now() + Duration::seconds(5)).await.unwrap();
    // 交付死信：原對話已封存 → 開新對話一直失敗 → 死信（錢已 commit）。
    env.archive("local_qa10-dead", "conv-qa10-dead");
    h.create_question(money_req(ids[3], Some(claude_source(&env, "conv-qa10-dead", "local_qa10-dead", Some(&ok))), WakeMode::Continue), Utc::now())
        .await
        .unwrap();
    h.answer_question(ids[3], ui_answer("a"), Utc::now()).await.unwrap();
    env.reactor_on(h, Some(fail.clone())).process_pending().await.unwrap();

    assert_eq!((verified(&env.ctx), fired(&env.ctx)), (4, 4), "前提：四題都驗證並 fire 一次：{}", commit_log(&env.ctx));
    let expect_delivery =
        [Some(DeliveryState::Resumed), Some(DeliveryState::Fresh), Some(DeliveryState::NoContinuation), Some(DeliveryState::Dead)];
    for (id, d) in ids.iter().zip(expect_delivery) {
        let raw = raw_result(h, id).await;
        assert!(raw.starts_with("COMMIT_OK"), "{id} 的執行紀錄要以 COMMIT_OK 開頭：{raw:?}");
        assert_eq!(delivery_state(h, id).await, d, "{id} 的交付");
        let err = h.reauthorize_question(id, Utc::now()).await.expect_err("已 commit 的錢題不能重新授權").to_string();
        assert!(err.contains("已經執行完成"), "{id} 重建前的拒絕原因：{err}");
    }
    assert!(raw_result(h, ids[3]).await.contains("交付失敗"), "死信要接在錢鏈結果後面");

    let (h2, discord2, _db) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db_at(Utc::now() + Duration::minutes(1)).await.unwrap();
    let mut wrong = Vec::new();
    for id in ids {
        let err = h2.reauthorize_question(id, Utc::now()).await.map(|_| "放行".to_string()).unwrap_or_else(|e| e.to_string());
        let agent = agent_money(&h2, id).await;
        if !err.contains("已經執行完成") || agent != "已執行完成" {
            wrong.push(format!("{id}：重新授權 {err:?}；agent 讀到 {agent:?}"));
        }
    }
    let reconcile_alerts: Vec<String> =
        discord2.messages().iter().filter(|m| m.reason == "rebuild_reconcile").map(|m| m.text.clone()).collect();
    assert!(
        report.reconcile == 0 && report.reconcile_items.is_empty() && reconcile_alerts.is_empty() && report.requeued == 0 && wrong.is_empty(),
        "正常 commit 的錢題重建後被誤判：reconcile={} items={:?} Discord={reconcile_alerts:?} requeued={}；{wrong:?}",
        report.reconcile,
        report.reconcile_items.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
        report.requeued
    );
}

// ---------------------------------------------------------------------------
// R10-2（應綠）：executor 驗證（消耗 approval）後被自己的護欄擋下、非零結束——記 `EXEC_UNKNOWN`、發 Discord `execution_unknown`
// （不帶 token）；原對話追加的訊息、交付死信、回執、`ask_report` 之後的讀取面、`ask_wait`、重建前後都一致讀成「結果不明」，
// 一律拒絕重新授權；重建不重複告警、不重新排入。AC-CC-14；CONTRACT §7、§8。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa10_unknown_outcome_reads_the_same_on_every_surface() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_mode_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "guard");
    let ok = env.script("claude-ok.sh", OK_BODY);
    let fail = env.script("claude-fail.sh", FAIL_BODY);
    let h = &env.ctx.handle;
    let (resumed, dead) = ("qa10-unknown-resume", "qa10-unknown-dead");
    h.create_question(money_req(resumed, Some(claude_source(&env, "conv-qa10-u1", "local_qa10-u1", Some(&ok))), WakeMode::Continue), Utc::now())
        .await
        .unwrap();
    let aid = h.answer_question(resumed, ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    let mut events = h.subscribe();
    env.reactor_on(h, Some(ok.clone())).process_pending().await.unwrap();
    env.archive("local_qa10-u2", "conv-qa10-u2");
    h.create_question(money_req(dead, Some(claude_source(&env, "conv-qa10-u2", "local_qa10-u2", Some(&ok))), WakeMode::Continue), Utc::now())
        .await
        .unwrap();
    h.answer_question(dead, ui_answer("a"), Utc::now()).await.unwrap();
    env.reactor_on(h, Some(fail.clone())).process_pending().await.unwrap();
    assert_eq!(verified(&env.ctx), 2, "前提：兩次 commit 都先驗證消耗了 approval：{}", commit_log(&env.ctx));

    let mut problems = Vec::new();
    let leaked_events: Vec<String> = drain_events(&mut events)
        .into_iter()
        .filter_map(|e| match e {
            ServerEvent::Execution { line: Some(line), .. } => Some(line),
            _ => None,
        })
        .filter(|l| l.contains(FAKE_EXECUTOR_TOKEN) || l.contains("approvals/"))
        .collect();
    if !leaked_events.is_empty() {
        problems.push(format!("/events 執行事件帶出 token 或 approval 路徑：{leaked_events:?}"));
    }
    let resume_prompt = env.calls().into_iter().find(|c| c.contains("--resume")).unwrap_or_default();
    if !resume_prompt.contains("結果不明") || resume_prompt.contains(FAKE_EXECUTOR_TOKEN) || resume_prompt.contains("approvals/") {
        problems.push(format!("原對話追加訊息：{resume_prompt}"));
    }
    let unknown_alerts: Vec<String> =
        env.ctx.discord.messages().iter().filter(|m| m.reason == "execution_unknown").map(|m| m.text.clone()).collect();
    if unknown_alerts.len() != 2 || unknown_alerts.iter().any(|t| t.contains(FAKE_EXECUTOR_TOKEN)) {
        problems.push(format!("Discord execution_unknown：{unknown_alerts:?}"));
    }
    for (id, d) in [(resumed, DeliveryState::Resumed), (dead, DeliveryState::Dead)] {
        let raw = raw_result(h, id).await;
        let agent = agent_money(h, id).await;
        let reauth = h.reauthorize_question(id, Utc::now()).await;
        if raw.contains(FAKE_EXECUTOR_TOKEN) || raw.contains("approvals/") {
            problems.push(format!("{id} 的執行紀錄沒遮掉 executor 輸出裡的 token／approval 路徑：{raw:?}"));
        }
        if !raw.starts_with("EXEC_UNKNOWN") || ui_failed(&raw) || !agent.starts_with("結果不明") || reauth.is_ok() {
            problems.push(format!("{id}：紀錄 {raw:?}、agent {agent:?}、重新授權 {}", if reauth.is_ok() { "放行" } else { "被拒" }));
        }
        if delivery_state(h, id).await != Some(d) {
            problems.push(format!("{id}：交付 {:?}", delivery_state(h, id).await));
        }
    }
    if !raw_result(h, dead).await.contains("交付失敗") {
        problems.push(format!("死信沒有接在錢鏈結果後面：{:?}", raw_result(h, dead).await));
    }
    // ask_report 之後：回饋另存，錢的讀取面不變。
    h.report_answer(resumed, &aid, ReportStatus::Blocked, "executor 說被冷卻期擋下，但 Kunsu 說結果不明，等使用者對帳", Utc::now())
        .await
        .unwrap();
    let after_report = agent_money(h, resumed).await;
    if !after_report.starts_with("結果不明") || !raw_result(h, resumed).await.starts_with("EXEC_UNKNOWN") {
        problems.push(format!("ask_report 之後 agent 讀到 {after_report:?}"));
    }
    // ask_create 重送已答題：回傳的答案同樣去敏、讀成結果不明。
    let mut resend = serde_json::to_value(money_req(dead, Some(claude_source(&env, "conv-qa10-u2", "local_qa10-u2", Some(&ok))), WakeMode::Continue)).unwrap();
    resend.as_object_mut().unwrap().remove("first_proposed");
    let again = mcp_on(h, "ask_create", resend).await.unwrap();
    let again_text = again.to_string();
    if !again_text.contains("結果不明") || again_text.contains(FAKE_EXECUTOR_TOKEN) || again_text.contains("approvals/") {
        problems.push(format!("ask_create 重送已答題：{again_text}"));
    }
    let waited = mcp_on(h, "ask_wait", json!({"question_id": dead, "timeout_seconds": 1})).await.unwrap();
    if waited["status"] != "answered" || !waited["answer"]["execution_result"].as_str().unwrap_or_default().starts_with("結果不明") {
        problems.push(format!("ask_wait：{waited}"));
    }

    let (h2, discord2, _db) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db_at(Utc::now() + Duration::minutes(1)).await.unwrap();
    for id in [resumed, dead] {
        let agent = agent_money(&h2, id).await;
        if h2.reauthorize_question(id, Utc::now()).await.is_ok() || !agent.starts_with("結果不明") {
            problems.push(format!("重建後 {id}：agent {agent:?}，重新授權放行或讀錯"));
        }
    }
    if report.requeued != 0 || discord2.messages().iter().any(|m| m.reason == "rebuild_reconcile") {
        problems.push(format!("重建：requeued={} reconcile={} Discord={:?}", report.requeued, report.reconcile, discord2.messages()));
    }
    assert!(problems.is_empty(), "EXEC_UNKNOWN 讀取面不一致：{problems:#?}");
}

// ---------------------------------------------------------------------------
// R10-3（應綠）：executor 沒驗證就失敗（approval 沒被消耗）→ `EXEC_BLOCKED`；重建前後都可以重新授權，重新授權後 commit 成功、只 fire
// 一次。AC-CC-14「被擋的錢題重建後可重新授權」；CONTRACT §7。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa10_block_before_verify_stays_reauthorizable_across_rebuild() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_mode_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "before");
    let ok = env.script("claude-ok.sh", OK_BODY);
    let id = "qa10-block-before";
    env.ctx.handle.create_question(money_req(id, None, WakeMode::Fresh), Utc::now()).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    env.reactor_on(&env.ctx.handle, Some(ok.clone())).process_pending().await.unwrap();
    let raw = raw_result(&env.ctx.handle, id).await;
    assert!(raw.starts_with("EXEC_BLOCKED") && verified(&env.ctx) == 0, "前提：沒驗證就失敗 → 被擋：{raw:?}");

    let (h2, _d2, _db) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db_at(Utc::now() + Duration::minutes(1)).await.unwrap();
    set_mode(&env.ctx, "ok");
    let base2 = spawn_http_on(&h2).await;
    write_mode_executor(&env.ctx, &format!("{base2}/approvals/verify"));
    let reauth = h2.reauthorize_question(id, Utc::now()).await;
    assert!(reauth.is_ok() && report.reconcile == 0, "被擋（沒消耗 approval）的錢題重建後要能重新授權：{:?}；reconcile={}", reauth.err(), report.reconcile);
    env.reactor_on(&h2, Some(ok)).process_pending().await.unwrap();
    let after = raw_result(&h2, id).await;
    assert!(after.starts_with("COMMIT_OK") && fired(&env.ctx) == 1, "重新授權後 commit 一次：{after:?}；{}", commit_log(&env.ctx));
}

// ---------------------------------------------------------------------------
// R10-4：被擋過一次的錢題（原對話正開著 → 交付 held），使用者重新授權；第二次 commit 在 executor 驗證（消耗 approval）之後 app 當掉。
// 重啟後 reactor 列標 needs_reconcile，重新授權被拒（對照組成立）。但原對話一關、held 重檢就在原對話追加，並把
// `combine_execution(EXEC_BLOCKED…, 追加結果)` 寫回執行紀錄：
//   (1) `executed_at` 被改成追加的時間——`money_outcome_unknown` 把它當成「錢鏈結果的時間」，晚於這次的消耗，於是不再算結果不明；
//   (2) `record_execution` 把 needs_reconcile 的 reactor 列改成 done（`mark_reactor_done` 不看狀態）。
// 兩道防線一起消失：重新授權放行、錢被動第二次；重建後也一樣（快照裡的 executed_at 同樣被改過）。追加的訊息還告訴原對話
// 「被安全護欄擋下」，實際上第二次的錢可能已經動過。
// 規格：AC-CC-14「錢的結果不明（approval 已消耗卻沒有之後的錢鏈結果…）拒絕重新授權並走人工對帳，重建前後一樣」；CONTRACT §7
// 「結果不明＝最近一次 approval 消耗晚於最近一次錢鏈結果」；INV-9。嚴重度高（錢可能被動兩次）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa10_held_recheck_after_a_crashed_reauth_must_not_hide_the_unknown_outcome() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_mode_executor(&env.ctx, &format!("{base}/approvals/verify"));
    let ok = env.script("claude-ok.sh", OK_BODY);
    let h = &env.ctx.handle;
    let id = "qa10-held-crash";
    let conv = "conv-qa10-held";
    h.create_question(money_req(id, Some(claude_source(&env, conv, "local_qa10-held", Some(&ok))), WakeMode::Continue), Utc::now())
        .await
        .unwrap();
    h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    let held = env.hold(conv);

    // 第一次：executor 沒驗證就失敗 → 被擋；原對話正開著 → held。
    set_mode(&env.ctx, "before");
    let r1 = env.reactor_on(h, Some(ok.clone()));
    r1.process_pending().await.unwrap();
    assert!(raw_result(h, id).await.starts_with("EXEC_BLOCKED"), "前提：第一次被擋");
    assert_eq!(delivery_state(h, id).await, Some(DeliveryState::Held), "前提：原對話正開著 → held");

    // 使用者重新授權；第二次 commit 在驗證（消耗 approval）之後卡住，app 當掉。
    h.reauthorize_question(id, Utc::now()).await.expect("前提：被擋的錢題可以重新授權");
    set_mode(&env.ctx, "hang");
    crash_during_commit(&r1, &env.ctx).await;

    // 重啟：卡在 executing 的列 → needs_reconcile；這時重新授權被拒（對照組）。
    let r2 = env.reactor_on(h, Some(ok.clone()));
    r2.process_pending().await.unwrap();
    assert!(env.ctx.discord.messages().iter().any(|m| m.reason == "reactor_needs_reconcile"), "前提：重啟後發人工對帳訊息");
    assert!(h.reauthorize_question(id, Utc::now()).await.is_err(), "前提：剛重啟時重新授權被拒");

    // 原對話關掉了：held 重檢 → 在原對話追加。
    std::fs::remove_file(held).unwrap();
    r2.process_due_deliveries(Utc::now() + Duration::seconds(61)).await.unwrap();
    let raw = raw_result(h, id).await;
    let prompt = env.calls().into_iter().rev().find(|c| c.contains("--resume")).unwrap_or_default();
    let told = if prompt.contains("被安全護欄擋下") { "被安全護欄擋下" } else if prompt.contains("結果不明") { "結果不明" } else { "（其他）" };

    // 重建（另一個空資料庫）後的判斷。
    let (h2, discord2, _db) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db_at(Utc::now()).await.unwrap();
    let after_rebuild = h2.reauthorize_question(id, Utc::now()).await.is_ok();
    let agent_after_rebuild = agent_money(&h2, id).await;

    // 重建前：再重新授權一次。
    let in_process = h.reauthorize_question(id, Utc::now()).await.is_ok();
    if in_process {
        set_mode(&env.ctx, "ok");
        r2.process_pending().await.unwrap();
    }
    assert!(
        !in_process && !after_rebuild,
        "第二次 approval 已消耗、沒有結果，原對話追加後卻不再算結果不明：重新授權 重建前 {}／重建後 {}；executor 驗證 {} 次（當掉的那次＋放行後再一次）、\
         放行後 fire {} 次；\
         執行紀錄 {raw:?}；追加訊息告訴原對話「{told}」；重建 reconcile={}（Discord {}）；重建後 agent 讀到 {agent_after_rebuild:?}",
        if in_process { "放行" } else { "被拒" },
        if after_rebuild { "放行" } else { "被拒" },
        verified(&env.ctx),
        fired(&env.ctx),
        report.reconcile,
        discord2.messages().iter().filter(|m| m.reason == "rebuild_reconcile").count(),
    );
}

// ---------------------------------------------------------------------------
// R10-5：第一次 commit 就在 executor 驗證（消耗 approval）之後 app 當掉——錢的結果不明。重新授權被拒、重建列人工對帳（成立），
// 但 agent 讀取面說「錢鏈還在跑，以 Kunsu 回執為準」：`ask_get`、`ask_wait`（reactor 列已是 needs_reconcile，等待者立刻拿到答案並
// 認領 live_waiter，原對話就以為錢鏈還在跑）、重建後的 `ask_get` 都一樣；Kunsu 回執也沒有錢的那一行（`moneyOutcome` 只看執行紀錄）。
// 規格：AC-CC-14「agent 讀到的錢題狀態…executor 已用掉授權卻沒有結果是『結果不明：錢可能已經動過，等使用者人工對帳』」；CONTRACT §7
// 「結果不明＝最近一次 approval 消耗晚於最近一次錢鏈結果（沒有結果也算）」。（CONTRACT §8「錢鏈適用但還沒有結果 → 錢鏈還在跑」與此
// 重疊，需要一起釐清。）嚴重度中：錢不會再動，但原對話與 agent 被告知錯的狀態，可能一直等或自己去查。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa10_crashed_first_commit_reads_as_unknown_not_still_running() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_mode_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "hang");
    let ok = env.script("claude-ok.sh", OK_BODY);
    let h = &env.ctx.handle;
    let id = "qa10-first-crash";
    h.create_question(money_req(id, Some(claude_source(&env, "conv-qa10-first", "local_qa10-first", Some(&ok))), WakeMode::Continue), Utc::now())
        .await
        .unwrap();
    h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    crash_during_commit(&env.reactor_on(h, Some(ok.clone())), &env.ctx).await;
    env.reactor_on(h, Some(ok.clone())).process_pending().await.unwrap();
    assert!(h.reauthorize_question(id, Utc::now()).await.is_err(), "前提：結果不明時重新授權被拒");

    let got = agent_money(h, id).await;
    let waited = mcp_on(h, "ask_wait", json!({"question_id": id, "timeout_seconds": 1})).await.unwrap();
    let waited_money = waited["answer"]["execution_result"].as_str().unwrap_or_default().to_string();
    let (h2, _d2, _db) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db_at(Utc::now() + Duration::minutes(1)).await.unwrap();
    let rebuilt = agent_money(&h2, id).await;
    assert_eq!(report.reconcile, 1, "重建要列人工對帳");
    assert!(
        got.starts_with("結果不明") && waited_money.starts_with("結果不明") && rebuilt.starts_with("結果不明"),
        "approval 已消耗、commit 途中當掉：ask_get 讀到 {got:?}；ask_wait 回 {} 並帶 {waited_money:?}（交付 {}）；重建後 ask_get {rebuilt:?}",
        waited["status"],
        waited["delivery"]["state"]
    );
}

// ---------------------------------------------------------------------------
// R10-6（應綠）：wake=none 的錢題（沒有原對話）——作答當下登記的待決交付已到期，但錢鏈還沒跑：交付重檢不搶先定案、等待者拿不到
// 答案；錢鏈跑完才定案 no_continuation，等待者拿到「已執行完成」，`kunsu wait` 結束碼 0。AC-CC-03、AC-CC-14；CONTRACT §7、§8。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa10_wake_none_money_waits_for_the_chain_before_any_delivery() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_mode_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "ok");
    let h = &env.ctx.handle;
    let id = "qa10-none-wait";
    h.create_question(money_req(id, None, WakeMode::None), Utc::now()).await.unwrap();
    let aid = h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    let r = env.reactor_on(h, None);

    r.process_due_deliveries(Utc::now() + Duration::seconds(5)).await.unwrap();
    let early_delivery = h.delivery_state(&aid).await.unwrap();
    let early = mcp_on(h, "ask_wait", json!({"question_id": id, "timeout_seconds": 1})).await.unwrap();
    assert_eq!(early_delivery, Some(DeliveryState::Pending), "錢鏈還沒跑，交付不能先定案（deliveries 表）");
    assert_eq!(early["status"], "pending_timeout", "錢鏈還沒跑，等待者不能先拿到答案：{early}");

    r.process_pending().await.unwrap();
    let waited = mcp_on(h, "ask_wait", json!({"question_id": id, "timeout_seconds": 1})).await.unwrap();
    let (code, stdout) = kunsu_wait(&base, id).await;
    assert!(fired(&env.ctx) == 1 && raw_result(h, id).await.starts_with("COMMIT_OK"), "錢鏈要跑：{}", commit_log(&env.ctx));
    assert_eq!(delivery_state(h, id).await, Some(DeliveryState::NoContinuation));
    assert_eq!(waited["answer"]["execution_result"], "已執行完成", "{waited}");
    assert!(code == Some(0) && stdout.contains("已執行完成"), "kunsu wait 結束碼 {code:?}：{stdout}");
}

// ---------------------------------------------------------------------------
// R10-7：動錢黃燈題沒帶 `money.executor_prepare_cmd` 也建得出來。面板照樣顯示「批准並執行」（`ui/src/surface/decision.ts`
// primaryLabel；`checkOneClick` 不看 prepare 命令），使用者按下去錢鏈永遠不會跑（`money_chain_applies_to` 要有 prepare 命令），
// agent 讀到「未觸發（安全選項或紅燈題只記錄決定）」——使用者選的明明是黃燈題的執行型選項。Kunsu 自己建的 Kongko 啟用題在資料
// 不足、組不出 prepare 命令時一律升紅鎖一鍵，agent 自建的題卻沒有同樣的防線。
// 規格：money-approval 場景 1「agent 建一題動錢黃燈題，帶 money.executor_prepare_cmd」；CONTRACT §2「按下去錢鏈永遠不會跑，會讓 agent
// 誤以為已批准」（第 8、9 輪同一類問題）；AC-CC-14「狀態文字依錢鏈結果開頭判斷…安全選項或紅燈是未觸發」。嚴重度中（錢沒動，但使用者
// 的批准被靜默吞掉、agent 被告知錯的原因）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa10_yellow_money_without_prepare_command_is_not_a_silent_approve_button() {
    let env = Env::new().await;
    let h = &env.ctx.handle;
    let id = "qa10-no-prepare";
    let mut req = money_req(id, None, WakeMode::Fresh);
    req.money.as_mut().unwrap().executor_prepare_cmd = None;
    assert_eq!(req.gate, Some(Gate::Yellow), "前提：黃燈");
    let created = h.create_question(req, Utc::now()).await;
    if created.is_err() {
        return; // 建題就拒收符合規格。
    }
    let stored = h.get_question(id).await.unwrap().unwrap();
    if stored.gate == Some(Gate::Red) {
        return; // 升紅鎖一鍵也符合規格（同 Kongko 啟用題）。
    }
    h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    let r = env.reactor_on(h, Some(env.script("claude-ok.sh", OK_BODY)));
    r.process_pending().await.unwrap();
    let kinds: Vec<String> = r.planned().iter().map(|c| c.kind.clone()).collect();
    let agent = agent_money(h, id).await;
    panic!(
        "黃燈錢題沒有 prepare 命令也建得出來（gate {:?}），使用者選執行型選項後錢鏈沒跑（reactor 命令 {kinds:?}），agent 讀到 {agent:?}",
        stored.gate
    );
}

// ---------------------------------------------------------------------------
// R10-8（應綠）：Kongko 啟用題（Kunsu 自己建的錢題）——資料充分的黃燈題批准後走完整錢鏈（verify→fire 一次），agent 讀到「已執行完成」，
// 排程身分是 interactive（`kunsu-` 保留前綴，不進督導頁）；金額 >NT$2,000 的紅燈題批准只記錄決定、錢鏈不跑、agent 讀到「未觸發」、
// 不能重新授權。AC-CC-14、AC-RC-01；PRD §14「發佈≠啟用」；INV-11、INV-12。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa10_kongko_activation_runs_the_full_chain_and_red_never_one_click() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_mode_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "ok");
    let h = &env.ctx.handle;
    let mut acts = Vec::new();
    for (n, budget) in [(1, 300), (2, 2500)] {
        let batch = format!("qa10-batch-{n}");
        let mut material = base_req(&format!("qa10-material-{n}"));
        material.kind = Kind::KongkoReview;
        material.options = vec![];
        material.kongko_ref = Some(batch.clone());
        h.create_question(material, Utc::now()).await.unwrap();
        let ev: KongkoPublication = serde_json::from_value(json!({
            "type": "ad_publication_confirmed", "publicationId": format!("pub-qa10-{n}"), "batchId": batch, "product": "nova",
            "ads": [{"campaign_id": "120200000000001", "daily_budget_twd": budget}], "confirmedAt": "2026-10-09T06:30:00Z"
        }))
        .unwrap();
        let act = h.kongko_event(ev, Utc::now()).await.unwrap().activation_question_id.expect("啟用題");
        h.answer_question(&act, ui_answer("activate"), Utc::now()).await.unwrap();
        acts.push(act);
    }
    let ok = env.script("claude-ok.sh", OK_BODY);
    env.reactor_on(h, Some(ok)).process_pending().await.unwrap();
    let yellow = h.get_question(&acts[0]).await.unwrap().unwrap();
    let red = h.get_question(&acts[1]).await.unwrap().unwrap();
    assert_eq!((yellow.gate, red.gate), (Some(Gate::Yellow), Some(Gate::Red)));
    assert_eq!(yellow.source.trigger, Some(kunsu_core::continuity::Trigger::Interactive), "Kongko 啟用題是 interactive");
    assert!(raw_result(h, &acts[0]).await.starts_with("COMMIT_OK"), "黃燈啟用題走完整錢鏈：{:?}", raw_result(h, &acts[0]).await);
    assert_eq!((verified(&env.ctx), fired(&env.ctx)), (1, 1), "只有黃燈題 fire：{}", commit_log(&env.ctx));
    assert_eq!(agent_money(h, &acts[0]).await, "已執行完成");
    assert!(agent_money(h, &acts[1]).await.starts_with("未觸發"), "紅燈題 agent 讀到：{}", agent_money(h, &acts[1]).await);
    assert!(h.reauthorize_question(&acts[1], Utc::now()).await.is_err(), "紅燈題不能重新授權");
}

// ---------------------------------------------------------------------------
// R10-9（應綠）：wake=none 的錢題在 reactor 處理前資料庫就被刪掉——重建後依規則重新排入（24 小時內），錢鏈跑一次（verify→fire 一次），
// 交付定案 no_continuation；再重建一次不再排入、重新授權以「已經執行完成」拒絕。超過 24 小時的同類答案不自動重做，列進重建報告。
// AC-CC-11、AC-CC-14；CONTRACT §7「重建時 wake=none 的錢題也依同樣規則重新排入」。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa10_wake_none_money_answered_before_rebuild_runs_once_after_it() {
    let env = Env::new().await;
    set_mode(&env.ctx, "ok");
    let (fresh_id, old_id) = ("qa10-none-rebuild", "qa10-none-old");
    let h = &env.ctx.handle;
    h.create_question(money_req(fresh_id, None, WakeMode::None), Utc::now()).await.unwrap();
    h.answer_question(fresh_id, ui_answer("a"), Utc::now()).await.unwrap();
    let old_at = Utc::now() - Duration::hours(30);
    let mut old = money_req(old_id, None, WakeMode::None);
    old.deadline_at = Some(old_at + Duration::hours(6));
    h.create_question(old, old_at).await.unwrap();
    h.answer_question(old_id, ui_answer("a"), old_at).await.unwrap();

    // reactor 還沒跑，資料庫就沒了 → 重建。
    let (h2, discord2, _db) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db_at(Utc::now()).await.unwrap();
    let base2 = spawn_http_on(&h2).await;
    write_mode_executor(&env.ctx, &format!("{base2}/approvals/verify"));
    let r = env.reactor_on(&h2, None);
    r.process_pending().await.unwrap();
    r.process_due_deliveries(Utc::now() + Duration::seconds(5)).await.unwrap();
    assert_eq!(report.requeued, 1, "24 小時內的 wake=none 錢題重新排入一次：{report:?}");
    assert!(report.stale.iter().any(|i| i.id == old_id), "超過 24 小時的列進報告：{:?}", report.stale);
    assert!(discord2.messages().iter().any(|m| m.reason == "rebuild_stale_unexecuted"), "超過 24 小時的發一則請使用者確認");
    assert_eq!((verified(&env.ctx), fired(&env.ctx)), (1, 1), "只有 24 小時內那題跑錢鏈一次：{}", commit_log(&env.ctx));
    assert!(raw_result(&h2, fresh_id).await.starts_with("COMMIT_OK"));
    assert_eq!(delivery_state(&h2, fresh_id).await, Some(DeliveryState::NoContinuation));

    let (h3, _d3, _db3) = fresh_db(&env.ctx).await;
    let report3 = h3.rebuild_db_at(Utc::now() + Duration::minutes(1)).await.unwrap();
    env.reactor_on(&h3, None).process_pending().await.unwrap();
    let err = h3.reauthorize_question(fresh_id, Utc::now()).await.expect_err("已 commit").to_string();
    assert!(
        report3.requeued == 0 && fired(&env.ctx) == 1 && err.contains("已經執行完成") && report3.reconcile == 0,
        "再重建一次：requeued={} reconcile={} fire {} 次；重新授權 {err}",
        report3.requeued,
        report3.reconcile,
        fired(&env.ctx)
    );
}
