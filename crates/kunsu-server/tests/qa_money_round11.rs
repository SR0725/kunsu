//! QA 錢的安全第 11 輪（BDD money-approval AC-MA-16..26、conversation-continuity AC-CC-11／14；INV-9、13、15、25）：
//! 回頭驗證第 10 輪的修正——錢的結果以每一次授權（nonce）判斷（`money_outcomes`，migration 0007）、重啟恢復記 `EXEC_UNKNOWN`、
//! 使用者在回執人工對帳（`resolve_money_outcome`）、重新授權、非紅燈錢題必帶 prepare 命令、沒有輸入的 spawn 一律 stdin=null——並找新問題。
//!
//! 錢鏈一律走真的簽章鏈：假 executor 的 commit 跟真的 executor 一樣，先打 `/approvals/verify`（消耗 approval）才 fire。
//! 模式寫在 vault 根的 `commit_mode`：
//!   `ok`（驗證後成功）、`before`（沒驗證就失敗）、`guard`（驗證後被 executor 護欄擋下、非零結束）、
//!   `hangbefore`（還沒驗證就卡住，用來模擬 app 在 executor 驗證之前當掉）、
//!   `gate`（驗證後等 vault 根出現 `release` 檔才 fire，用來在「已消耗、還沒有結果」的空檔裡觀察各個讀取面）。
//!
//! 斷言寫規格期望；失敗訊息寫實際行為。標「應綠」的是確認修正成立的對照組。時間一律注入或用牆鐘的先後順序；
//! 「app 當掉」以 8 秒內丟棄 reactor 的 future 模擬（子行程 kill_on_drop），再開一個新的 reactor 當作重啟。
//! UI 的判準照抄 shell 與網頁（見 `ui_*` 函式），讓伺服器狀態與回執畫面能在同一個測試裡對照。
//!
//! 2026-10-09 主線決定（使用者授權）後調整：回執的按鈕改由伺服器算好的 `money_state`（shell 附在題目上）決定（F2），所以 `ui_*`
//! 判準從「看執行紀錄字串」改成「看 `ServerHandle::money_state`」；舊資料補結果在 app 啟動時跑（F1），R11-1 以重開 handle 模擬升級後
//! 的那次啟動；被擋時 Kunsu 會刪掉 approval 檔（F4），R11-6 的晚到者改用 executor 在被擋前留下的副本。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration, Utc};
use common::*;
use kunsu_core::continuity::HostKind;
use kunsu_core::policy::Policy;
use kunsu_core::types::*;
use kunsu_server::mcp::{handle_rpc, McpReply};
use kunsu_server::*;
use serde_json::{json, Value};
use sqlx::Row;
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
    /// 假 CLI：執行 `body`。
    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.root.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
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
}

const OK_BODY: &str = "echo 'EXEC_OK: 原對話已接手'";
const HANG_BODY: &str = "sleep 30";

fn claude_source(env: &Env, conversation: &str, host_session: &str, cli_path: &Path) -> Source {
    Source {
        agent: AgentKind::Claude,
        cwd: env.work().to_string_lossy().to_string(),
        host: Some(HostKind::ClaudeDesktop),
        conversation_id: Some(conversation.into()),
        host_session_id: Some(host_session.into()),
        attended: Some(false),
        cli_path: Some(cli_path.to_string_lossy().to_string()),
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

/// shell 送給網頁的 `money_state`（src-tauri/src/ipc.rs `to_ui` 附 `ServerHandle::money_state`）。網頁的 `moneyActionsFor` 依它給按鈕。
async fn ui_state(handle: &ServerHandle, id: &str) -> Option<MoneyState> {
    let q = handle.get_question(id).await.unwrap().unwrap();
    handle.money_state(&q).await.unwrap()
}

/// 網頁回執的判準（ui/src/surface/MoneyActions.tsx `moneyActionsFor`）：`money_state = unknown` 才給「錢已經動了／錢沒有動」。
fn ui_offers_reconcile(state: Option<MoneyState>) -> bool {
    state == Some(MoneyState::Unknown)
}

/// 同上：`money_state = reauthorizable` 才給「重新授權並執行」。
fn ui_offers_reauthorize(state: Option<MoneyState>) -> bool {
    state == Some(MoneyState::Reauthorizable)
}

/// 這個 nonce 有沒有錢鏈結果。
async fn has_outcome(pool: &sqlx::SqlitePool, nonce: &str) -> bool {
    sqlx::query("SELECT 1 FROM money_outcomes WHERE nonce = ?1").bind(nonce).fetch_optional(pool).await.unwrap().is_some()
}

/// 帳本裡帶 `backfill` 的 money_outcome 行數。
fn backfill_rows(ctx: &TestCtx) -> usize {
    let dir = ctx.handle.ledger().asks_dir();
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            n += std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .filter(|l| serde_json::from_str::<Value>(l).is_ok_and(|v| v["execution"]["money_outcome"]["backfill"] == true))
                .count();
        }
    }
    n
}

/// shell `to_ui`（src-tauri/src/ipc.rs execution_failure）：以 EXEC_BLOCKED:／EXEC_FAILED: 開頭就進失敗畫面（文字依 money_state）。
fn ui_failed(raw: &str) -> bool {
    raw.starts_with("EXEC_BLOCKED:") || raw.starts_with("EXEC_FAILED:")
}

/// 假 executor：prepare 依 argv 誠實回填 spec；commit 依 vault 根的 `commit_mode` 決定（見檔頭）。
/// prepare 另把自己拿到的 stdin 種類寫進 `prepare_stdin.log`（`null`＝/dev/null、`fifo`＝管線、`tty`、`other`）。
fn write_r11_executor(ctx: &TestCtx, verify_url: &str) {
    let body = format!(
        r#"#!/usr/bin/env python3
import json, os, stat, sys, time, urllib.request

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
        # 晚到者（例如拿到被擋人話狀態的 agent）事先留了一份 approval 檔副本（R11-6）。
        import shutil
        shutil.copy(flag("--approval-file"), "stolen_approval.json")
        sys.stderr.write("executor 設定檔讀不到，沒有驗證就結束\n")
        sys.exit(1)
    if mode == "hangbefore":
        sys.stdout.flush()
        time.sleep(30)
    rec = json.load(open(flag("--approval-file")))
    req = urllib.request.Request("{verify_url}", data=json.dumps({{"approval_record": rec}}).encode(),
        headers={{"Content-Type": "application/json", "Authorization": "Bearer {TEST_TOKEN}"}})
    out = urllib.request.urlopen(req, timeout=5).read().decode()
    log("verify " + out)
    if '"valid":true' not in out:
        sys.stderr.write("verify 失敗：" + out + "\n")
        sys.exit(3)
    if mode == "guard":
        sys.stderr.write("BLOCKED: 3 天冷卻期未滿\n")
        sys.exit(2)
    if mode == "gate":
        for _ in range(400):
            if os.path.exists("release"):
                break
            time.sleep(0.05)
    log("fired")
    print("commit 完成：日預算已更新")
    sys.exit(0)
try:
    st = os.fstat(0)
    if stat.S_ISFIFO(st.st_mode):
        kind = "fifo"
    elif stat.S_ISCHR(st.st_mode):
        kind = "null" if st.st_rdev == os.stat("/dev/null").st_rdev else "tty"
    else:
        kind = "other"
except OSError:
    kind = "closed"
with open("prepare_stdin.log", "a") as f:
    f.write(kind + "\n")
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

fn approval_file(ctx: &TestCtx, id: &str) -> PathBuf {
    ctx.vault.path().join(format!("projects/meta-ads/data/executor_pending/approvals/{id}.json"))
}

/// 模擬 app 在 reactor 跑到一半時當掉：8 秒內丟棄 reactor 的 future（子行程 kill_on_drop）。
async fn crash(r: &Arc<Reactor>) {
    let out = tokio::time::timeout(std::time::Duration::from_secs(8), r.process_pending()).await;
    assert!(out.is_err(), "前提：reactor 卡住還沒結束（模擬當掉）");
}

/// 第二條連線直接看資料庫（同一個 SQLite 檔，WAL）。
async fn side_pool(ctx: &TestCtx) -> sqlx::SqlitePool {
    kunsu_server::db::open_pool(&ctx.db_path).await.unwrap()
}

async fn outcomes(pool: &sqlx::SqlitePool, id: &str) -> Vec<(String, String)> {
    sqlx::query("SELECT nonce, result FROM money_outcomes WHERE question_id = ?1 ORDER BY recorded_at")
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get::<String, _>("nonce"), r.get::<String, _>("result")))
        .collect()
}

async fn consumed(pool: &sqlx::SqlitePool, id: &str) -> Vec<String> {
    sqlx::query("SELECT nonce FROM approvals_consumed WHERE question_id = ?1 ORDER BY consumed_at")
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<String, _>("nonce"))
        .collect()
}

async fn issued(pool: &sqlx::SqlitePool, id: &str) -> Vec<String> {
    sqlx::query("SELECT nonce FROM approvals_issued WHERE question_id = ?1 ORDER BY issued_at")
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<String, _>("nonce"))
        .collect()
}

async fn reactor_status(pool: &sqlx::SqlitePool, id: &str) -> Option<String> {
    sqlx::query("SELECT status FROM reactor_runs WHERE question_id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .unwrap()
        .map(|r| r.get::<String, _>("status"))
}

/// 把帳本裡帶 `money_outcome` 的 execution 行拿掉——等同 migration 0007 之前的舊帳本（舊版不寫這種行）。
fn strip_money_outcome_rows(ctx: &TestCtx) -> usize {
    let dir = ctx.handle.ledger().asks_dir();
    let mut removed = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let kept: Vec<&str> = text
            .lines()
            .filter(|l| {
                let v: Value = serde_json::from_str(l).unwrap_or(Value::Null);
                let legacy_drop = v["execution"].get("money_outcome").is_some();
                if legacy_drop {
                    removed += 1;
                }
                !legacy_drop
            })
            .collect();
        std::fs::write(&path, kept.join("\n") + "\n").unwrap();
    }
    removed
}

/// 等子行程在假 executor 的紀錄裡出現某一行（真實子行程的同步，不是在等時鐘）。
async fn wait_for_log(ctx: &TestCtx, pred: impl Fn(&str) -> bool) {
    for _ in 0..200 {
        if pred(&commit_log(ctx)) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("等不到 executor 紀錄：{}", commit_log(ctx));
}

/// 一題已 commit 成功（COMMIT_OK，approval 已消耗）的 wake=none 黃燈錢題。
async fn committed(env: &Env, id: &str) {
    let h = &env.ctx.handle;
    h.create_question(money_req(id, None, WakeMode::None), Utc::now()).await.unwrap();
    h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    env.reactor_on(h, None).process_pending().await.unwrap();
    assert!(raw_result(h, id).await.starts_with("COMMIT_OK"), "前提：commit 成功：{:?}", raw_result(h, id).await);
}

// ---------------------------------------------------------------------------
// R11-1：migration 0007 之前已 commit 成功的錢題（舊資料庫已有 approvals_consumed，money_outcomes 是新表、空的）——升級後 agent 讀取面
// 全部變成「結果不明：…等使用者人工對帳」，回執卻寫「錢：已執行完成」、沒有任何對帳按鈕。沒有回填（backfill）。
// 規格：AC-MA-17「所有讀取面、追加訊息與回執一致」；AC-CC-14「狀態文字依錢鏈結果開頭判斷…COMMIT_OK 已執行完成」；CLAUDE.md Phase 5
// 「migration…齊全」。嚴重度高：每一題歷史上一鍵執行過的錢題都會被告訴 agent「錢可能已經動過、等人工對帳」，而使用者沒有按鈕可以解除。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa11_pre_migration_committed_money_still_reads_as_executed() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_r11_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "ok");
    let id = "qa11-legacy-ok";
    committed(&env, id).await;
    let h = &env.ctx.handle;
    assert_eq!(agent_money(h, id).await, "已執行完成", "前提：升級前 agent 讀到已執行完成");

    // 舊資料庫跑 migration 0007 之後的樣子：approvals_consumed 有這次的 nonce，money_outcomes 沒有任何一列。
    let pool = side_pool(&env.ctx).await;
    sqlx::query("DELETE FROM money_outcomes").execute(&pool).await.unwrap();

    // 升級後的那次啟動（F1：補資料在 app 啟動時跑）。
    let h = env.ctx.reopen().await;
    let raw = raw_result(&h, id).await;
    let agent = agent_money(&h, id).await;
    let state = ui_state(&h, id).await;
    assert!(
        agent == "已執行完成" && state == Some(MoneyState::Settled),
        "migration 0007 之前已 commit 成功的錢題：執行紀錄 {:?}（回執寫「錢：已執行完成」、對帳鈕 {}），agent 卻讀到 {agent:?}（money_state {state:?}）",
        raw.chars().take(40).collect::<String>(),
        if ui_offers_reconcile(state) { "有" } else { "沒有" },
    );
    let per_nonce = outcomes(&pool, id).await;
    assert!(per_nonce.len() == 1 && per_nonce[0].1.starts_with("COMMIT_OK"), "補上同樣開頭的結果：{per_nonce:?}");
    assert_eq!(backfill_rows(&env.ctx), 1, "補資料寫一行帶 backfill 的帳本行");
    // 冪等：再啟動一次不重複補、不重複寫帳本。
    let h3 = env.ctx.reopen().await;
    assert_eq!(agent_money(&h3, id).await, "已執行完成");
    assert_eq!(backfill_rows(&env.ctx), 1, "再啟動一次不重複寫帳本");
}

// ---------------------------------------------------------------------------
// R11-2：用 migration 0007 之前的帳本（execution 行沒有 money_outcome）重建資料庫——每一題歷史上 commit 成功的錢題都被列進人工對帳、
// 各發一則 Discord「approval 已消耗但沒有之後的執行紀錄」，agent 讀到「結果不明」。每重建一次就再發一次。
// 規格：AC-CC-11「重建資料庫後不重複交付…」、AC-CC-14「重建前後一樣」；INV-13；CONTRACT §7「重建時同一個判斷決定人工對帳」（舊帳本
// 沒有 money_outcome 行時要怎麼判斷沒有定義）。嚴重度高（同 R11-1，而且會對每一題舊錢題發 Discord）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa11_rebuild_from_a_pre_0007_ledger_does_not_flag_committed_money() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_r11_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "ok");
    let id = "qa11-legacy-ledger";
    committed(&env, id).await;
    let removed = strip_money_outcome_rows(&env.ctx);
    assert!(removed >= 1, "前提：帳本原本有 money_outcome 行");

    let (h2, discord2, _db) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db_at(Utc::now() + Duration::minutes(1)).await.unwrap();
    let alerts: Vec<String> =
        discord2.messages().iter().filter(|m| m.reason == "rebuild_reconcile").map(|m| m.text.clone()).collect();
    let agent = agent_money(&h2, id).await;
    let raw = raw_result(&h2, id).await;
    assert!(
        report.reconcile == 0 && alerts.is_empty() && agent == "已執行完成",
        "舊帳本（沒有 money_outcome 行）重建後：reconcile={}、Discord {alerts:?}、agent 讀到 {agent:?}；執行紀錄仍是 {:?}（回執寫「已執行完成」、對帳鈕 {}）",
        report.reconcile,
        raw.chars().take(30).collect::<String>(),
        if ui_offers_reconcile(ui_state(&h2, id).await) { "有" } else { "沒有" },
    );
    // 補資料寫了帶 backfill 的帳本行：再重建一次直接讀到，不再補第二行、也不列人工對帳。
    assert_eq!(backfill_rows(&env.ctx), 1, "重建補資料寫一行帶 backfill 的帳本行");
    let (h3, discord3, _db3) = fresh_db(&env.ctx).await;
    let again = h3.rebuild_db_at(Utc::now() + Duration::minutes(2)).await.unwrap();
    assert!(again.reconcile == 0 && discord3.messages().iter().all(|m| m.reason != "rebuild_reconcile"), "再重建：{again:?}");
    assert_eq!(backfill_rows(&env.ctx), 1, "再重建不重複補");
    assert_eq!(agent_money(&h3, id).await, "已執行完成");
}

// ---------------------------------------------------------------------------
// R11-3：8/6 那類舊題（approval 已被 executor 消耗、沒有錢鏈結果、交付最後進死信，執行紀錄是 `EXEC_FAILED: 死信…`）——伺服器以 nonce
// 判斷「結果不明」（agent 讀到結果不明、重新授權被拒、重建列人工對帳），但回執：shell 把 `EXEC_FAILED:` 當失敗畫面（「執行沒有完成…修好原因後
// 可以在這裡重新授權」），網頁只看 `EXEC_UNKNOWN` 開頭才給對帳鈕——使用者沒有任何按鈕可以標「錢已經動了／錢沒有動」，這題永遠卡在結果不明。
// 規格：AC-MA-18「使用者在回執標『錢已經動了／錢沒有動』解除結果不明」；AC-MA-17「所有讀取面…與回執一致」。嚴重度高（觸發這次修正的真實
// 題目正是這一型，修完仍解除不了）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa11_dead_lettered_consumed_approval_gives_ray_the_reconcile_buttons() {
    let ctx = TestCtx::new().await;
    let h = &ctx.handle;
    let id = "qa11-0806";
    let t = Utc::now();
    h.create_question(money_req(id, None, WakeMode::Fresh), t).await.unwrap();
    let aid = h.answer_question(id, ui_answer("a"), t).await.unwrap().answer.unwrap().answer_id;
    let nonce = "n-qa11-0806";
    h.record_approval_issued(id, &aid, nonce, FAKE_EXECUTOR_TOKEN, "h", t.timestamp(), t).await.unwrap();
    h.consume_approval(id, nonce, &json!({"question_id": id, "nonce": nonce, "param_hash": "h"}), t).await.unwrap();
    h.record_execution(id, "EXEC_FAILED: 死信（重試 3 次仍失敗）：spawn 失敗", t).await.unwrap();

    let q = h.get_question(id).await.unwrap().unwrap();
    assert!(h.money_outcome_unknown(&q).await.unwrap(), "前提：伺服器以 nonce 判斷結果不明");
    assert!(agent_money(h, id).await.starts_with("結果不明"), "前提：agent 讀到結果不明");
    assert!(h.reauthorize_question(id, Utc::now()).await.is_err(), "前提：重新授權被拒");

    let (h2, _d2, _db) = fresh_db(&ctx).await;
    let report = h2.rebuild_db_at(Utc::now()).await.unwrap();
    let raw = raw_result(h, id).await;
    let raw2 = raw_result(&h2, id).await;
    let (state, state2) = (ui_state(h, id).await, ui_state(&h2, id).await);
    assert!(
        ui_offers_reconcile(state) && ui_offers_reconcile(state2),
        "伺服器說結果不明（重建 reconcile={}），回執卻{}失敗畫面、沒有對帳鈕：執行紀錄 {raw:?}（重建後 {raw2:?}）；money_state {state:?}／{state2:?}；\
         重新授權鈕 {}",
        report.reconcile,
        if ui_failed(&raw) { "進" } else { "不進" },
        if ui_offers_reauthorize(state) { "有" } else { "沒有" },
    );
    // 使用者在回執按「錢沒有動」：對帳成功（因為有結果不明的 nonce），之後可以重新授權。
    h.resolve_money_outcome(id, false, Utc::now()).await.expect("8/6 型可以人工對帳");
    assert_eq!(ui_state(h, id).await, Some(MoneyState::Reauthorizable));
}

// ---------------------------------------------------------------------------
// R11-4：app 在 executor 驗證之前當掉（approval 沒被消耗）——重啟後 reactor 列標 needs_reconcile、Discord 叫使用者「在 Kunsu 回執標記『錢已經動了
// ／錢沒有動』」，但這題沒有結果不明的 nonce，執行紀錄也沒寫：(a) 回執沒有對帳鈕；(b) 第一次授權就當掉的題沒有重新授權鈕，重新授權過的題
// （紀錄還是上一次的 EXEC_BLOCKED）有重新授權鈕、按了卻被拒「執行尚未終結」；(c) agent 一直讀到「錢鏈還在跑，以 Kunsu 回執為準」，`ask_wait`
// 也立刻交出這個狀態。這題永遠卡住，`resolve_money_outcome` 為這種情況寫好的分支（`waiting`）在 UI 上走不到。
// 規格：AC-MA-18、AC-MA-19、AC-CC-14「還沒跑完是『錢鏈還在跑』」；CONTRACT §7 人工對帳。嚴重度中（錢沒動，但題目卡死、agent 被告知錯的狀態）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa11_crash_before_verify_leaves_ray_a_way_out() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_r11_executor(&env.ctx, &format!("{base}/approvals/verify"));
    let ok = env.script("claude-ok.sh", OK_BODY);
    let h = &env.ctx.handle;
    let (first, again) = ("qa11-crash-first", "qa11-crash-reauth");
    for id in [first, again] {
        h.create_question(money_req(id, None, WakeMode::Fresh), Utc::now()).await.unwrap();
    }
    // 第二題先被擋一次（沒驗證就失敗），使用者重新授權。
    set_mode(&env.ctx, "before");
    h.answer_question(again, ui_answer("a"), Utc::now()).await.unwrap();
    env.reactor_on(h, Some(ok.clone())).process_pending().await.unwrap();
    assert!(raw_result(h, again).await.starts_with("EXEC_BLOCKED"), "前提：第二題被擋");
    h.reauthorize_question(again, Utc::now()).await.unwrap();
    h.answer_question(first, ui_answer("a"), Utc::now()).await.unwrap();

    // 兩題的 commit 都在 executor 驗證之前卡住，app 當掉；第一個卡住就當掉，另一題在重啟後另外再跑一次。
    set_mode(&env.ctx, "hangbefore");
    crash(&env.reactor_on(h, Some(ok.clone()))).await;
    crash(&env.reactor_on(h, Some(ok.clone()))).await;
    let r = env.reactor_on(h, Some(ok.clone()));
    r.process_pending().await.unwrap();
    assert_eq!(verified(&env.ctx), 0, "前提：executor 沒驗證過：{}", commit_log(&env.ctx));
    let pool = side_pool(&env.ctx).await;
    let alert = env.ctx.discord.messages().into_iter().find(|m| m.reason == "reactor_needs_reconcile").map(|m| m.text).unwrap_or_default();

    let told: Vec<String> = env
        .ctx
        .discord
        .messages()
        .into_iter()
        .filter(|m| m.reason == "money_interrupted_before_verify")
        .map(|m| m.text)
        .collect();

    let mut stuck = Vec::new();
    for id in [first, again] {
        let raw = raw_result(h, id).await;
        let agent = agent_money(h, id).await;
        let state = ui_state(h, id).await;
        let row = reactor_status(&pool, id).await;
        let mut unsettled = 0;
        for n in issued(&pool, id).await {
            if !has_outcome(&pool, &n).await {
                unsettled += 1;
            }
        }
        let waited = mcp_on(h, "ask_wait", json!({"question_id": id, "timeout_seconds": 1})).await.unwrap();
        let reauth = h.reauthorize_question(id, Utc::now()).await.map(|_| "放行".to_string()).unwrap_or_else(|e| e.to_string());
        let ray_can_act = ui_offers_reconcile(state) || (ui_offers_reauthorize(state) && reauth == "放行");
        let blocked_dead = raw.starts_with("EXEC_BLOCKED") && raw.contains("錢沒有動") && row.as_deref() == Some("dead") && unsettled == 0;
        if !ray_can_act || agent == "錢鏈還在跑，以 Kunsu 回執為準" || !blocked_dead {
            stuck.push(format!(
                "{id}：reactor 列 {row:?}；執行紀錄 {raw:?}；沒有結果的簽發 nonce {unsettled} 個；money_state {state:?}；回執 對帳鈕 {}／重新授權鈕 {}\
                 （按了：{reauth}）；agent {agent:?}；ask_wait {} {:?}",
                if ui_offers_reconcile(state) { "有" } else { "沒有" },
                if ui_offers_reauthorize(state) { "有" } else { "沒有" },
                waited["status"],
                waited["answer"]["execution_result"],
            ));
        }
    }
    assert!(stuck.is_empty(), "重啟後（Discord：{alert:?}／{told:?}）：{stuck:#?}");
    assert!(
        told.len() == 2 && told.iter().all(|t| t.contains("錢沒有動") && t.contains("重新授權")),
        "Discord 要說錢沒有動、可以在回執重新授權（不是叫使用者標記錢已經動了／沒有動）：{told:?}；needs_reconcile 訊息：{alert:?}"
    );
}

// ---------------------------------------------------------------------------
// R11-5：commit 正在跑、executor 已驗證（消耗 approval）但還沒回報——(a) `ask_get` 讀到「結果不明…等使用者人工對帳」，不是「錢鏈還在跑」；
// (b) `resolve_money_outcome` 在錢鏈跑的時候被接受（使用者的「錢沒有動」寫進 money_outcomes、帳本與執行紀錄，之後才被真正的 COMMIT_OK 蓋掉）。
// 對照組（應綠）：錢鏈跑的時候重新授權被拒（reactor 列 executing），不會有兩條錢鏈同時跑；最後只 fire 一次、結果是 COMMIT_OK。
// 規格：AC-CC-14「還沒跑完是『錢鏈還在跑，以 Kunsu 回執為準』」；CONTRACT §7「resolve_money_outcome…未解決的 nonce」（跑到一半的授權不是
// 未解決，是還沒結束）。嚴重度中（錢不會被動兩次，但 agent 在每一次正常 commit 的空檔都被告知「錢可能已經動過、等人工對帳」，人工對帳也
// 可以在錢鏈跑的時候插隊寫進帳本）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa11_in_flight_commit_is_running_not_unknown_and_cannot_be_reconciled() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_r11_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "gate");
    let h = env.ctx.handle.clone();
    let id = "qa11-in-flight";
    h.create_question(money_req(id, None, WakeMode::None), Utc::now()).await.unwrap();
    h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    let r = env.reactor_on(&h, None);
    let task = tokio::spawn({
        let r = r.clone();
        async move { r.process_pending().await }
    });
    wait_for_log(&env.ctx, |l| l.contains("\"valid\":true")).await;

    // 空檔：approval 已消耗、commit 還沒回報。
    let state_during = ui_state(&h, id).await;
    let during = agent_money(&h, id).await;
    let reauth_during = h.reauthorize_question(id, Utc::now()).await.is_ok();
    let resolve_during = h.resolve_money_outcome(id, false, Utc::now()).await.map(|q| q.answer.unwrap().execution_result.unwrap_or_default());

    std::fs::write(env.ctx.vault.path().join("release"), "").unwrap();
    task.await.unwrap().unwrap();
    let after = raw_result(&h, id).await;
    assert!(!reauth_during, "對照組：錢鏈跑的時候不能重新授權");
    assert!(after.starts_with("COMMIT_OK") && fired(&env.ctx) == 1, "對照組：最後 COMMIT_OK、只 fire 一次：{after:?}；{}", commit_log(&env.ctx));
    let pool = side_pool(&env.ctx).await;
    assert!(
        during == "錢鏈還在跑，以 Kunsu 回執為準" && resolve_during.is_err() && state_during == Some(MoneyState::Running),
        "commit 跑到一半（executor 已驗證、還沒回報）：agent 讀到 {during:?}；money_state {state_during:?}；人工對帳 {:?}；最後 money_outcomes {:?}",
        resolve_during.map(|r| format!("被接受，執行紀錄改成 {r:?}")).unwrap_or_else(|e| format!("被拒：{e}")),
        outcomes(&pool, id).await,
    );
}

// ---------------------------------------------------------------------------
// R11-6：一次被擋下的授權（executor 沒驗證就失敗，Kunsu 記 `EXEC_BLOCKED`、這個 nonce 的結果是 EXEC_BLOCKED）事後仍能被 executor 驗證消耗：
// approval 檔留在固定路徑、TTL 15 分鐘內 `/approvals/verify` 照樣回 valid——之後 Kunsu 仍把它當「被擋、沒動」，回執給「重新授權並執行」，
// 重新授權放行，第二個 nonce 再 fire 一次。錢被動兩次。
// 情境：被擋的人話狀態（「被安全護欄擋下」）送回原對話後，agent 自己跑 `executor.py commit --approval-file …`（它有 vault 與 Bearer token）。
// 規格：INV-9（單次消耗以外，Kunsu 已經判定結束的授權不該再被消耗）；AC-CC-14「executor 已用掉授權…是『結果不明』…不是『被護欄擋下』」、
// 「已經執行完成的答案拒絕重新授權，錢不會被再動一次」；CONTRACT §7「沒消耗才是 EXEC_BLOCKED」。嚴重度高（錢會被動兩次；前提是有人在 15 分鐘內
// 用舊授權檔手動 commit）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa11_a_blocked_authorization_cannot_be_spent_later() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_r11_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "before");
    let h = &env.ctx.handle;
    let id = "qa11-stale-nonce";
    h.create_question(money_req(id, None, WakeMode::None), Utc::now()).await.unwrap();
    h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    env.reactor_on(h, None).process_pending().await.unwrap();
    let blocked = raw_result(h, id).await;
    assert!(blocked.starts_with("EXEC_BLOCKED") && verified(&env.ctx) == 0, "前提：沒驗證就失敗 → 被擋：{blocked:?}");
    let path = approval_file(&env.ctx, id);
    let stolen = env.ctx.vault.path().join("stolen_approval.json");
    assert!(stolen.exists(), "前提：晚到者手上有一份 approval 檔副本");
    // F4：記 EXEC_BLOCKED 時刪掉這次寫的 approval 檔（best effort）。
    let removed = !path.exists();

    // 15 分鐘內有人拿舊授權檔（副本）自己跑 commit。
    set_mode(&env.ctx, "ok");
    let out = std::process::Command::new("python3")
        .arg(env.ctx.vault.path().join("projects/meta-ads/scripts/executor.py"))
        .args(["commit", "--approval-file"])
        .arg(&stolen)
        .current_dir(env.ctx.vault.path())
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let late_spent = verified(&env.ctx) == 1;
    let agent_after = agent_money(h, id).await;
    let state_after = ui_state(h, id).await;
    let reauth = h.reauthorize_question(id, Utc::now()).await.is_ok();
    if reauth {
        env.reactor_on(h, None).process_pending().await.unwrap();
    }
    assert!(
        !late_spent && removed && fired(&env.ctx) == 1,
        "Kunsu 記成被擋（{}）的授權事後被 executor 驗證{}（exit {:?}）；approval 檔{}；之後 agent 讀到 {agent_after:?}、回執{}「重新授權並執行」、\
         重新授權{}，executor 一共 fire {} 次：{}",
        blocked.chars().take(20).collect::<String>(),
        if late_spent { "成功" } else { "失敗" },
        out.status.code(),
        if removed { "已刪除" } else { "還在固定路徑" },
        if ui_offers_reauthorize(state_after) { "有" } else { "沒有" },
        if reauth { "放行" } else { "被拒" },
        fired(&env.ctx),
        commit_log(&env.ctx).replace('\n', " | "),
    );
}

// ---------------------------------------------------------------------------
// R11-7：錢已經 COMMIT_OK、之後 app 在原對話追加途中當掉（reactor 列 executing → needs_reconcile）。Discord 寫「可能已打到 Meta API…請在回執標記
// 『錢已經動了／錢沒有動』」（回執其實寫「錢：已執行完成」，沒有按鈕）；而 `resolve_money_outcome(executed=false)` 因為 reactor 列是
// needs_reconcile 而被接受，把 COMMIT_OK 改寫成 RECONCILED_NOT_EXECUTED——重新授權只看執行紀錄開頭，於是放行、錢被動第二次。
// money_outcomes 明明記著這個 nonce 是 COMMIT_OK。
// 規格：AC-MA-18「『已經動了』永不能」、AC-CC-14「已經執行完成的答案拒絕重新授權（錢不會被再動一次）」；CONTRACT §7「以每一次授權（nonce）判斷」。
// 嚴重度中（伺服器端少一道防線；UI 目前不給這題按鈕，但 IPC 照收，Discord 又叫使用者去標記）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa11_reconcile_cannot_unmake_a_committed_outcome() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_r11_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "ok");
    let hang = env.script("claude-hang.sh", HANG_BODY);
    let ok = env.script("claude-ok.sh", OK_BODY);
    let h = &env.ctx.handle;
    let id = "qa11-commit-then-crash";
    h.create_question(money_req(id, Some(claude_source(&env, "conv-qa11-cc", "local_qa11-cc", &hang)), WakeMode::Continue), Utc::now())
        .await
        .unwrap();
    h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    crash(&env.reactor_on(h, Some(hang.clone()))).await;
    assert!(raw_result(h, id).await.starts_with("COMMIT_OK") && fired(&env.ctx) == 1, "前提：錢已 commit，當掉在追加途中");
    env.reactor_on(h, Some(ok.clone())).process_pending().await.unwrap();
    let pool = side_pool(&env.ctx).await;
    let alert = env.ctx.discord.messages().into_iter().find(|m| m.reason == "reactor_needs_reconcile").map(|m| m.text).unwrap_or_default();
    let status = reactor_status(&pool, id).await;
    let per_nonce = outcomes(&pool, id).await;

    let state = ui_state(h, id).await;
    let interrupted = env.ctx.discord.messages().into_iter().any(|m| m.reason == "delivery_interrupted");
    let resolved = h.resolve_money_outcome(id, false, Utc::now()).await.is_ok();
    let raw = raw_result(h, id).await;
    let reauth = h.reauthorize_question(id, Utc::now()).await.is_ok();
    if reauth {
        env.reactor_on(h, Some(ok.clone())).process_pending().await.unwrap();
    }
    assert!(
        state == Some(MoneyState::Settled) && alert.is_empty() && interrupted,
        "錢已 COMMIT_OK、中斷在交付：money_state {state:?}（回執不給按鈕）；Discord 不該叫使用者標記錢有沒有動：{alert:?}；要發交付中斷：{interrupted}"
    );
    assert!(
        !resolved && !reauth && fired(&env.ctx) == 1,
        "COMMIT_OK 之後追加途中當掉（reactor 列 {status:?}、money_outcomes {per_nonce:?}；Discord：{alert}）：人工對帳「錢沒有動」{}，執行紀錄變成 {:?}，\
         重新授權{}，executor 一共 fire {} 次",
        if resolved { "被接受" } else { "被拒" },
        raw.chars().take(30).collect::<String>(),
        if reauth { "放行" } else { "被拒" },
        fired(&env.ctx),
    );
}

// ---------------------------------------------------------------------------
// F6 補充（實作時發現）：同一個 reactor 列在錢鏈跑完之後接著做交付（在原對話追加）。追加訊息已經告訴原對話「Kunsu 錢鏈狀態：
// 已執行完成」並請它 `ask_get` 確認——這時 `ask_get` 不能因為 reactor 列還是 executing 就說「錢鏈還在跑」（跟追加訊息矛盾）。
// 「錢鏈還在跑」指錢鏈本身：列在 queued／executing、而且這一輪的錢鏈結果還沒寫進執行紀錄。回執的按鈕（money_state）仍以列為準，
// 列結束前不給按鈕（人工對帳、重新授權在列結束前一律被拒）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa11_agent_reads_the_money_result_while_the_delivery_runs() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_r11_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "ok");
    let marker = env.root.path().join("resumed");
    let slow = env.script("claude-slow.sh", &format!("touch '{}'\nsleep 30", marker.display()));
    let h = env.ctx.handle.clone();
    let id = "qa11-delivery-after-commit";
    h.create_question(money_req(id, Some(claude_source(&env, "conv-qa11-dl", "local_qa11-dl", &slow)), WakeMode::Continue), Utc::now())
        .await
        .unwrap();
    h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    let r = env.reactor_on(&h, Some(slow.clone()));
    let task = tokio::spawn({
        let r = r.clone();
        async move { r.process_pending().await }
    });
    for _ in 0..200 {
        if marker.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(marker.exists(), "前提：錢鏈跑完、正在原對話追加");
    let raw = raw_result(&h, id).await;
    let agent = agent_money(&h, id).await;
    let state = ui_state(&h, id).await;
    let resolve = h.resolve_money_outcome(id, false, Utc::now()).await.is_ok();
    let reauth = h.reauthorize_question(id, Utc::now()).await.is_ok();
    task.abort();
    assert!(
        raw.starts_with("COMMIT_OK") && agent == "已執行完成" && state == Some(MoneyState::Running) && !resolve && !reauth,
        "錢鏈已 COMMIT_OK、正在追加：執行紀錄 {:?}；agent 讀到 {agent:?}（追加訊息說已執行完成）；money_state {state:?}；對帳 {resolve}；重新授權 {reauth}",
        raw.chars().take(30).collect::<String>()
    );
}

// ---------------------------------------------------------------------------
// R11-8：沒有輸入的 spawn 一律 stdin=null——錢鏈的 prepare（approval.rs `Command::output()`，tokio 預設繼承 stdin）仍繼承 Kunsu 自己的 stdin。
// 規格：705cdc5「spawns without input get a null stdin」（本輪修正項）；`kunsu serve` 從終端機啟動時，prepare 會拿到那個終端機。嚴重度低。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa11_prepare_spawn_gets_a_null_stdin() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_r11_executor(&env.ctx, &format!("{base}/approvals/verify"));
    set_mode(&env.ctx, "ok");
    let h = &env.ctx.handle;
    let id = "qa11-prepare-stdin";
    h.create_question(money_req(id, None, WakeMode::None), Utc::now()).await.unwrap();
    h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    // 讓這個測試行程的 stdin 變成一條管線：繼承就看得出來（fifo），給 null 才是 /dev/null。
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY：pipe/dup/dup2/close 只操作這個測試建立的檔案描述元，結束前還原 fd 0。
    let saved = unsafe {
        assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
        let saved = libc::dup(0);
        assert!(libc::dup2(fds[0], 0) >= 0);
        saved
    };
    env.reactor_on(h, None).process_pending().await.unwrap();
    unsafe {
        libc::dup2(saved, 0);
        libc::close(saved);
        libc::close(fds[0]);
        libc::close(fds[1]);
    }
    let seen = std::fs::read_to_string(env.ctx.vault.path().join("prepare_stdin.log")).unwrap_or_default();
    assert!(raw_result(h, id).await.starts_with("COMMIT_OK"), "前提：錢鏈跑完");
    assert_eq!(seen.trim(), "null", "prepare 子行程的 stdin（null＝/dev/null、fifo＝繼承了 Kunsu 的 stdin）");
}

// ---------------------------------------------------------------------------
// R11-9（應綠）：同一題多次授權——第一次被擋（沒驗證，nonce A＝EXEC_BLOCKED）→ 重新授權、executor 驗證後被護欄擋（nonce B＝EXEC_UNKNOWN，
// 拒絕重新授權）→ 使用者對帳「錢沒有動」（B＝RECONCILED_NOT_EXECUTED）→ 重新授權 → 成功（nonce C＝COMMIT_OK）。每個 nonce 各記自己的結果；
// approvals_consumed 只有 B、C；重建後 money_outcomes 與 approvals_consumed 一樣；agent 讀到「已執行完成」、不能再重新授權、不列人工對帳。
// AC-MA-17／18／19、AC-CC-14；CONTRACT §7。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa11_each_authorization_keeps_its_own_outcome_across_rebuild() {
    let env = Env::new().await;
    let base = spawn_http(&env.ctx).await;
    write_r11_executor(&env.ctx, &format!("{base}/approvals/verify"));
    let h = &env.ctx.handle;
    let id = "qa11-three-nonces";
    h.create_question(money_req(id, None, WakeMode::None), Utc::now()).await.unwrap();
    h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();

    set_mode(&env.ctx, "before");
    env.reactor_on(h, None).process_pending().await.unwrap();
    assert!(raw_result(h, id).await.starts_with("EXEC_BLOCKED"));
    h.reauthorize_question(id, Utc::now()).await.expect("被擋 → 可重新授權");

    set_mode(&env.ctx, "guard");
    env.reactor_on(h, None).process_pending().await.unwrap();
    assert!(raw_result(h, id).await.starts_with("EXEC_UNKNOWN"), "驗證後失敗 → 結果不明：{:?}", raw_result(h, id).await);
    assert!(h.reauthorize_question(id, Utc::now()).await.is_err(), "結果不明 → 拒絕重新授權");
    h.resolve_money_outcome(id, false, Utc::now()).await.expect("使用者對帳：錢沒有動");
    assert!(agent_money(h, id).await.contains("沒有執行"));
    h.reauthorize_question(id, Utc::now()).await.expect("對帳沒動 → 可重新授權");

    set_mode(&env.ctx, "ok");
    env.reactor_on(h, None).process_pending().await.unwrap();
    assert!(raw_result(h, id).await.starts_with("COMMIT_OK"));
    assert_eq!((verified(&env.ctx), fired(&env.ctx)), (2, 1), "B、C 驗證；只有 C fire：{}", commit_log(&env.ctx));

    let pool = side_pool(&env.ctx).await;
    let nonces = issued(&pool, id).await;
    assert_eq!(nonces.len(), 3, "三次授權三個 nonce");
    let by_nonce = |list: &[(String, String)], n: &str| list.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()).unwrap_or_default();
    let before = outcomes(&pool, id).await;
    let prefixes: Vec<String> = nonces.iter().map(|n| by_nonce(&before, n).split(':').next().unwrap_or_default().to_string()).collect();
    assert_eq!(prefixes, ["EXEC_BLOCKED", "RECONCILED_NOT_EXECUTED", "COMMIT_OK"], "每個 nonce 各記自己的結果：{before:?}");
    let used = consumed(&pool, id).await;
    assert_eq!(used, nonces[1..].to_vec(), "只有 B、C 被 executor 消耗");
    assert!(h.reauthorize_question(id, Utc::now()).await.is_err(), "已執行完成 → 不能再授權");

    let (h2, discord2, db2) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db_at(Utc::now() + Duration::minutes(1)).await.unwrap();
    let pool2 = kunsu_server::db::open_pool(&db2.path().join("kunsu.db")).await.unwrap();
    let after = outcomes(&pool2, id).await;
    let after_prefixes: Vec<String> = nonces.iter().map(|n| by_nonce(&after, n).split(':').next().unwrap_or_default().to_string()).collect();
    assert_eq!(after_prefixes, prefixes, "重建後每個 nonce 的結果不變：{after:?}");
    assert_eq!(consumed(&pool2, id).await, used, "重建後消耗紀錄不變");
    assert_eq!(agent_money(&h2, id).await, "已執行完成");
    assert!(h2.reauthorize_question(id, Utc::now()).await.is_err(), "重建後仍不能再授權");
    assert!(
        report.reconcile == 0 && report.requeued == 0 && !discord2.messages().iter().any(|m| m.reason == "rebuild_reconcile"),
        "重建不列人工對帳、不重新排入：{report:?}"
    );
}
