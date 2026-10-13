//! QA 第 7 輪（回答延續原對話，BDD conversation-continuity AC-CC-11；INV-13）：回頭驗證第 6 輪的修正——
//! 重建先完整讀完帳本、24 小時重做窗、帳本同一時間只有一個在途讀寫、row_id 去重、拒絕重建的錯誤訊息——並找新問題。
//!
//! 每個測試的斷言寫的是規格期望；失敗訊息寫實際行為。標「應綠」的是確認修正成立的對照組。
//! 時間一律注入（`rebuild_db_at`、`tick(now)`、`process_due_deliveries(now)`），不等真實時間；
//! 只有模擬「vault 檔案呼叫卡在核心」時會吃到程式本身的 5／10 秒逾時。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration, Utc};
use common::*;
use kunsu_core::continuity::{DeliveryState, HostKind};
use kunsu_core::policy::Policy;
use kunsu_core::types::{AgentKind, Source, Wake, WakeMode};
use kunsu_server::ledger::{LedgerRow, LedgerType};
use kunsu_server::{
    ApprovalOutcome, KunsuConfig, Reactor, ReactorConfig, RecordingDiscord, ServerHandle, StaticKeyProvider,
    StubGuardProbe,
};
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
}

const OK_BODY: &str = "echo 'EXEC_OK: 原對話已接手'";

fn claude_source(env: &Env, conversation: &str, cli_path: Option<&Path>) -> Source {
    Source {
        agent: AgentKind::Claude,
        cwd: env.work().to_string_lossy().to_string(),
        host: Some(HostKind::ClaudeDesktop),
        conversation_id: Some(conversation.into()),
        host_session_id: Some("local_qa7-1".into()),
        attended: Some(false),
        cli_path: cli_path.map(|p| p.to_string_lossy().to_string()),
        ..Default::default()
    }
}

fn fresh_wake() -> Wake {
    Wake { mode: WakeMode::Fresh, session_id: None, playbook: None }
}

/// 同 vault、全新空 DB；Discord 拿得到（`fresh_db_same_vault` 的 Discord 拿不到）。
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

async fn outbox_count(ctx: &TestCtx) -> u64 {
    let pool = kunsu_server::db::open_pool(&ctx.db_path).await.unwrap();
    kunsu_server::store::ledger_outbox_count(&pool).await.unwrap()
}

fn mkfifo(path: &Path) {
    assert!(std::process::Command::new("/usr/bin/mkfifo").arg(path).status().unwrap().success());
}

/// 放行一個卡在「以讀取模式打開 FIFO」的執行緒：打開寫端（非阻塞）再關掉，讀端拿到 EOF。
fn release_fifo_reader(path: &Path) {
    use std::os::unix::fs::OpenOptionsExt;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(path).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn jsonl_files(ctx: &TestCtx) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(ctx.handle.ledger().asks_dir())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    out.sort();
    out
}

/// 逐檔只保留 `keep` 回 true 的帳本行（模擬某些行當年沒寫進帳本）。
fn keep_ledger_rows(ctx: &TestCtx, keep: impl Fn(&LedgerRow) -> bool) {
    for p in jsonl_files(ctx) {
        let kept: Vec<String> = std::fs::read_to_string(&p)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter(|l| serde_json::from_str::<LedgerRow>(l).map(|r| keep(&r)).unwrap_or(true))
            .map(str::to_string)
            .collect();
        std::fs::write(&p, kept.join("\n") + "\n").unwrap();
    }
}

fn delivery_state_of(row: &LedgerRow) -> Option<String> {
    row.execution
        .as_ref()
        .and_then(|e| e.pointer("/delivery/state"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// R7-1：帳本寫入卡在核心、永遠不回來（權限給了也一樣）→ `in_flight` 永遠是 true → 之後每一筆寫入都只排隊、
// 待補寫佇列永遠補不進去、重建永遠被拒；而使用者只在一開始收到一次「寫不進帳本，請給完全磁碟權限」。
// 規格（AC-CC-11、CONTRACT §7）：權限恢復後依序補寫；帳本積欠要看得到（`kunsu doctor` 顯示積欠）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa7_hung_ledger_write_blocks_recovery_forever_without_signal() {
    let env = Env::new().await;
    let ctx = &env.ctx;
    let id = "qa7-hung";
    ctx.handle.create_question(base_req(id), Utc::now()).await.unwrap();
    ctx.handle.create_question(base_req("qa7-hung-2"), Utc::now()).await.unwrap();
    let t0 = Utc::now();
    let day = ctx.handle.ledger().day_file(t0);
    let saved = std::fs::read_to_string(&day).unwrap_or_default();
    let _ = std::fs::remove_file(&day);
    mkfifo(&day);

    // 權限問題期間：取消一題 → 帳本寫入卡在核心（FIFO 沒有讀端）→ 5 秒逾時 → 排進待補寫佇列。
    ctx.handle.cancel_question(id, t0).await.unwrap();
    let degraded_at_start = ctx.discord.messages().len();
    assert!(ctx.discord.messages().iter().any(|m| m.reason == "ledger_degraded"), "前提：帳本寫不進去");

    // 使用者照 Discord 指示給了權限，帳本檔恢復可寫；但那一次卡在核心的呼叫永遠不回來
    // （刪掉 FIFO 模擬：再也沒有人打得開它的讀端）。
    std::fs::remove_file(&day).unwrap();
    std::fs::write(&day, &saved).unwrap();

    // 之後 20 分鐘：tick 每 30 秒補寫一次，使用者也照常作答。
    for k in 1..=20 {
        ctx.handle.tick(t0 + Duration::seconds(31 * k)).await.unwrap();
    }
    ctx.handle.answer_question("qa7-hung-2", ui_answer("b"), t0 + Duration::minutes(11)).await.unwrap();
    for k in 21..=40 {
        ctx.handle.tick(t0 + Duration::seconds(31 * k)).await.unwrap();
    }

    let text = std::fs::read_to_string(&day).unwrap();
    let backlog = outbox_count(ctx).await;
    let caught_up = text.lines().any(|l| l.contains(id) && l.contains("\"cancelled\"")) && backlog == 0;
    let later: Vec<String> = ctx.discord.messages()[degraded_at_start..]
        .iter()
        .map(|m| format!("[{}] {}", m.reason, m.text))
        .collect();
    let health = serde_json::to_value(ctx.handle.health(t0 + Duration::minutes(21)).await.unwrap()).unwrap();
    let health_shows_backlog = health
        .as_object()
        .unwrap()
        .keys()
        .any(|k| k.contains("ledger") || k.contains("backlog") || k.contains("outbox"));
    let rebuild = ctx.handle.rebuild_db_at(t0 + Duration::minutes(21)).await;
    eprintln!(
        "R7-1 權限恢復 20 分鐘後：帳本補上取消行={caught_up}，待補寫佇列={backlog} 行，之後的 Discord={later:?}，\
         /health={health}，重建={:?}",
        rebuild.as_ref().err().map(|e| e.to_string())
    );
    assert!(
        caught_up || !later.is_empty() || health_shows_backlog,
        "帳本寫入卡在核心永遠不回來時，權限恢復後 20 分鐘帳本仍沒補寫（佇列 {backlog} 行、重建被拒），\
         而使用者除了一開始那則「請給權限」之外沒有任何訊號（Discord 沒有新訊息、/health 沒有積欠欄位）"
    );
}

// ---------------------------------------------------------------------------
// R7-2：CONTRACT §7「刪資料庫前先確認 `kunsu doctor` 顯示帳本沒有積欠」——doctor 與 /health 都不顯示帳本積欠。
// 而且 doctor 的 ledger_writable 是從 CLI 行程（終端機的權限）探測，app 本身寫不進去時照樣 ✅。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa7_doctor_and_health_show_ledger_backlog() {
    let ctx = TestCtx::new().await;
    let id = "qa7-doctor";
    ctx.handle.create_question(base_req(id), Utc::now()).await.unwrap();
    let t0 = Utc::now();
    let day = ctx.handle.ledger().day_file(t0);
    if !day.exists() {
        std::fs::write(&day, "").unwrap();
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&day, std::fs::Permissions::from_mode(0o444)).unwrap();
    ctx.handle.cancel_question(id, t0).await.unwrap();
    let backlog = outbox_count(&ctx).await;
    assert!(backlog > 0, "前提：帳本有積欠");

    let base = spawn_http(&ctx).await;
    let health: Value = reqwest::get(format!("{base}/health")).await.unwrap().json().await.unwrap();
    let health_text = health.to_string();
    let health_shows = ["ledger", "backlog", "outbox"].iter().any(|k| health_text.contains(k));

    // 真的跑一次 `kunsu doctor`（指向測試 server 與測試 vault；HOME 指 tempdir，不碰使用者的檔案）。
    let bin = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("kunsu");
    let doctor_out = if bin.exists() {
        let home = TempDir::new().unwrap();
        let kunsu_home = home.path().join(".kunsu");
        std::fs::create_dir_all(&kunsu_home).unwrap();
        std::fs::write(kunsu_home.join("token"), TEST_TOKEN).unwrap();
        std::fs::write(kunsu_home.join("admin-token"), TEST_ADMIN_TOKEN).unwrap();
        let out = tokio::process::Command::new(&bin)
            .arg("doctor")
            .env("HOME", home.path())
            .env("KUNSU_HOME_DIR", &kunsu_home)
            .env("KUNSU_BIND", base.trim_start_matches("http://"))
            .env("KUNSU_VAULT_ROOT", ctx.vault.path())
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .unwrap();
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        eprintln!("R7-2：找不到 {}（先 cargo build -p kunsu-cli），只檢查 /health", bin.display());
        None
    };
    std::fs::set_permissions(&day, std::fs::Permissions::from_mode(0o644)).unwrap();
    let doctor_shows = doctor_out
        .as_deref()
        .is_none_or(|o| ["積欠", "待補寫", "backlog", "沒補寫"].iter().any(|k| o.contains(k)));
    eprintln!("R7-2 /health={health}\nR7-2 doctor 輸出：\n{}", doctor_out.as_deref().unwrap_or("(未執行)"));
    assert!(
        health_shows && doctor_shows,
        "帳本有 {backlog} 行積欠（app 寫不進去），/health 與 `kunsu doctor` 都看不到；doctor 的 ledger_writable 還顯示 ✅"
    );
}

// ---------------------------------------------------------------------------
// R7-3：vault 讀取卡住時 `kunsu rebuild-db`（server 通道）拿到的錯誤：HTTP 500 `internal`、「vault 檔案寫入逾時…」
// （其實是讀取），再試一次變成「…這一筆先排進待補寫佇列」（其實什麼都沒排）；都沒說「拒絕重建、資料庫保持原樣」。
// CLI 原樣印出 `POST /admin/rebuild-db → {status}：{body}`（kunsu-cli main.rs post_json）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa7_rebuild_refusal_on_hung_vault_read_is_readable() {
    let ctx = TestCtx::new().await;
    let id = "qa7-hung-read";
    ctx.handle.create_question(base_req(id), Utc::now()).await.unwrap();
    let base = spawn_http(&ctx).await;
    let open = ctx.handle.ledger().open_json_path();
    std::fs::remove_file(&open).unwrap();
    mkfifo(&open);
    let client = reqwest::Client::new();
    let mut seen = Vec::new();
    for _ in 0..2 {
        let r = client
            .post(format!("{base}/admin/rebuild-db"))
            .bearer_auth(TEST_ADMIN_TOKEN)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        let status = r.status();
        let body: Value = r.json().await.unwrap();
        seen.push((status, body));
    }
    release_fifo_reader(&open);
    assert!(ctx.handle.get_question(id).await.unwrap().is_some(), "前提：拒絕重建時資料庫保持原樣");
    let cli: Vec<String> = seen.iter().map(|(s, b)| format!("POST /admin/rebuild-db → {s}：{b}")).collect();
    eprintln!("R7-3 CLI 會印出：\n  {}\n  {}", cli[0], cli[1]);
    let readable = |(s, b): &(reqwest::StatusCode, Value)| {
        let msg = b["error"].as_str().unwrap_or("");
        s.as_u16() == 409
            && b["code"] == "rebuild_refused"
            && msg.contains("資料庫保持原樣")
            && !msg.contains("寫入逾時")
            && !msg.contains("排進待補寫佇列")
    };
    assert!(
        seen.iter().all(readable),
        "vault 讀不到時重建的錯誤對 CLI 使用者不可讀：{cli:?}"
    );
}

// ---------------------------------------------------------------------------
// R7-4：帳本 jsonl 不見了（vault 搬家、同步衝突改名、vault 路徑設錯），但 OPEN.json 還在——server 每次變更都會
// 重寫 OPEN.json，所以只要有一題待決它就不是空的 → 「帳本與 OPEN.json 都空」的守門沒擋 → 已回答的題全被清掉。
// 規格（BDD 場景 11）：「讀不到或帳本空白卻有資料時拒絕」。（CONTRACT §7 只寫「兩者都空」，比 BDD 窄。）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa7_rebuild_refuses_when_ledger_is_blank_but_db_has_answers() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("qa7-blank-answered"), Utc::now()).await.unwrap();
    ctx.handle.answer_question("qa7-blank-answered", ui_answer("b"), Utc::now()).await.unwrap();
    ctx.handle.create_question(base_req("qa7-blank-pending"), Utc::now()).await.unwrap();
    let elsewhere = TempDir::new().unwrap();
    for p in jsonl_files(&ctx) {
        std::fs::rename(&p, elsewhere.path().join(p.file_name().unwrap())).unwrap();
    }
    let report = ctx.handle.rebuild_db().await;
    let answered = ctx.handle.get_question("qa7-blank-answered").await.unwrap();
    assert!(
        report.is_err() && answered.is_some(),
        "帳本 jsonl 全不見、只剩 OPEN.json（1 題待決）時重建照樣放行，已回答的題被清掉：report={report:?} 已回答題還在={}",
        answered.is_some()
    );
}

// ---------------------------------------------------------------------------
// R7-5：24 小時重做窗只套用在「完全沒有交付紀錄」的答案。60 天前的答案若帳本只留下 pending 交付行
// （等待者租約內記的；之後的 resuming／resumed／execution 行當年沒寫進去），重建後還原成 pending → reactor 在原對話
// 再追加一次。規格（BDD 場景 11）：只自動重做 24 小時內還沒處理的答案，更舊的列出來請使用者確認。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa7_rebuild_does_not_redeliver_old_pending_delivery() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let id = "qa7-old-pending";
    let mut req = base_req(id);
    req.source = claude_source(&env, "conv-qa7-op", Some(&claude));
    let t_old = Utc::now() - Duration::days(60);
    env.ctx.handle.create_question(req, t_old).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), t_old).await.unwrap();
    // 原對話的等待者剛結束一輪（10 秒租約內）→ reactor 記 pending；租約過後在原對話追加。
    env.ctx.handle.waiter_enter(id, Utc::now());
    env.ctx.handle.waiter_exit(id, Utc::now());
    let r = env.reactor_on(&env.ctx.handle, Some(claude.clone()));
    r.process_pending().await.unwrap();
    let d = env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap().delivery;
    assert_eq!(d.map(|d| d.state), Some(DeliveryState::Pending), "前提：租約內記 pending");
    r.process_due_deliveries(Utc::now() + Duration::seconds(20)).await.unwrap();
    assert_eq!(env.calls().len(), 1, "前提：已在原對話追加一次");
    // 當年帳本降級：pending 那行有寫進去，之後的交付與執行行沒有。
    keep_ledger_rows(&env.ctx, |row| {
        row.question.id != id
            || match row.ledger_type {
                LedgerType::Execution => false,
                LedgerType::Delivery => delivery_state_of(row).as_deref() == Some("pending"),
                _ => true,
            }
    });
    let (h2, _discord, _db) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db().await.unwrap();
    let r2 = env.reactor_on(&h2, Some(claude));
    r2.process_pending().await.unwrap();
    r2.process_due_deliveries(Utc::now() + Duration::seconds(20)).await.unwrap();
    assert_eq!(
        env.calls().len(),
        1,
        "60 天前已追加過的答案（帳本只剩 pending 交付行）重建後又在原對話追加一次：report={report:?} calls={:?}",
        env.calls()
    );
}

// ---------------------------------------------------------------------------
// R7-6：錢題 approval 已消耗、帳本沒有執行紀錄，作答超過 24 小時 → 被歸進「舊答案請確認」（stale），
// 不發「重建後需要人工對帳」、reconcile 計數 0。規格（CONTRACT §7、BDD 場景 11）：錢題若已有 approval_consumed
// 卻沒有執行紀錄，不重新排入，發 Discord「重建後需要人工對帳」（錢可能已經動過）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa7_old_money_with_consumed_approval_still_gets_reconcile() {
    let ctx = TestCtx::new().await;
    let id = "qa7-old-money";
    let mut req = money_t0_req(id);
    req.wake_on_answer = fresh_wake();
    ctx.handle.create_question(req, now()).await.unwrap();
    let aid = ctx.handle.answer_question(id, ui_answer("a"), now()).await.unwrap().answer.unwrap().answer_id;
    ctx.handle
        .record_approval_issued(id, &aid, "n-qa7-om", FAKE_EXECUTOR_TOKEN, "h", now().timestamp(), now())
        .await
        .unwrap();
    let record = json!({"question_id": id, "nonce": "n-qa7-om", "param_hash": "h"});
    let consumed = ctx.handle.consume_approval(id, "n-qa7-om", &record, now()).await.unwrap();
    assert!(matches!(consumed, ApprovalOutcome::Consumed), "前提：approval 已消耗（錢可能已經動過）");
    // 執行紀錄沒進帳本；三天後重建。
    let (h2, discord, _db) = fresh_db(&ctx).await;
    let report = h2.rebuild_db_at(now() + Duration::days(3)).await.unwrap();
    let msgs: Vec<String> = discord.messages().iter().map(|m| format!("[{}] {}", m.reason, m.text)).collect();
    let reconcile_msg = discord.messages().iter().any(|m| m.reason == "rebuild_reconcile");
    assert!(
        report.reconcile == 1 && reconcile_msg,
        "approval 已消耗、作答超過 24 小時的錢題被當成一般舊答案，沒有「錢可能已經動過，請人工對帳」：report={report:?} discord={msgs:?}"
    );
}

// ---------------------------------------------------------------------------
// R7-7：重建報告只有 stale_unexecuted 的「數字」。server 不在時 `kunsu rebuild-db` 走本地重建，Discord 是
// LoggingDiscord（CLI 沒有 log 輸出），使用者只看得到 `"stale_unexecuted":1`，不知道是哪幾題。
// 規格（CONTRACT §7）：更舊的不自動重做，列在重建報告（stale_unexecuted）並發 Discord 請使用者確認。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa7_rebuild_report_lists_which_answers_are_stale() {
    let ctx = TestCtx::new().await;
    let id = "qa7-stale-list";
    let mut req = base_req(id);
    req.title = "QA7 舊答案要確認".into();
    req.wake_on_answer = fresh_wake();
    ctx.handle.create_question(req, now()).await.unwrap();
    ctx.handle.answer_question(id, ui_answer("a"), now()).await.unwrap();
    let (h2, _discord, _db) = fresh_db(&ctx).await;
    let report = h2.rebuild_db_at(now() + Duration::days(3)).await.unwrap();
    assert_eq!(report.stale_unexecuted, 1, "前提：{report:?}");
    let printed = serde_json::to_string(&report).unwrap();
    eprintln!("R7-7 CLI 本地重建印出：rebuild-db（本地）完成：{printed}");
    assert!(
        printed.contains(id) || printed.contains("QA7 舊答案要確認"),
        "重建報告只有 stale_unexecuted 數字，沒有列出是哪幾題：{printed}"
    );
}

// ---------------------------------------------------------------------------
// R7-8：重建把 events 一起清掉（含 `no_ack_alert` 防重複標記）。超過 24 小時、已列進「舊答案請確認」那一則的答案，
// 下一次 tick 又各自發一則「你答的 X 24 小時沒有任何 agent 處理」——同一批答案告警兩次、每題一則。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa7_stale_answers_are_not_alerted_again_one_by_one() {
    let ctx = TestCtx::new().await;
    for i in 0..3 {
        let mut req = base_req(&format!("qa7-stale-{i}"));
        req.wake_on_answer = fresh_wake();
        ctx.handle.create_question(req, now()).await.unwrap();
        ctx.handle.answer_question(&format!("qa7-stale-{i}"), ui_answer("a"), now()).await.unwrap();
    }
    let (h2, discord, _db) = fresh_db(&ctx).await;
    let t = now() + Duration::days(3);
    let report = h2.rebuild_db_at(t).await.unwrap();
    assert_eq!(report.stale_unexecuted, 3, "前提：{report:?}");
    h2.tick(t + Duration::seconds(1)).await.unwrap();
    let reasons: Vec<String> = discord.messages().iter().map(|m| m.reason.clone()).collect();
    let per_question = reasons.iter().filter(|r| *r == "no_ack_24h").count();
    eprintln!("R7-8 重建後第一次 tick 的 Discord：{reasons:?}");
    assert_eq!(
        per_question, 0,
        "已在重建報告請使用者確認的 3 題舊答案，下一次 tick 又各發一則「24 小時沒有任何 agent 處理」：{reasons:?}"
    );
}

// ---------------------------------------------------------------------------
// R7-9（應綠）：24 小時窗看的是作答時間（answered_at），不是建題時間或帳本行時間；邊界 23:59 內重做、25 小時列 stale。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa7_requeue_window_keys_on_answered_at() {
    let ctx = TestCtx::new().await;
    let t = now() + Duration::days(10);
    let cases = [
        ("qa7-win-created-old", t - Duration::hours(72), t - Duration::hours(1)),
        ("qa7-win-edge", t - Duration::hours(72), t - Duration::hours(23) - Duration::minutes(59)),
        ("qa7-win-stale", t - Duration::hours(25), t - Duration::hours(25)),
    ];
    for (id, created, answered) in cases {
        let mut req = base_req(id);
        req.wake_on_answer = fresh_wake();
        ctx.handle.create_question(req, created).await.unwrap();
        ctx.handle.answer_question(id, ui_answer("a"), answered).await.unwrap();
    }
    let (h2, _discord, _db) = fresh_db(&ctx).await;
    let report = h2.rebuild_db_at(t).await.unwrap();
    assert_eq!((report.requeued, report.stale_unexecuted), (2, 1), "{report:?}");
}

// ---------------------------------------------------------------------------
// R7-10（應綠）：row_id 去重不會吃掉不同的行——內容相同但 row_id 不同的行都保留；同一 row_id 的重複行只留第一行；
// 舊行（沒有 row_id）即使逐字相同也不去重。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa7_row_id_dedup_never_drops_distinct_rows() {
    let ctx = TestCtx::new().await;
    let id = "qa7-dedup";
    let mut req = base_req(id);
    req.source = Source {
        agent: AgentKind::Claude,
        cwd: "/w".into(),
        host: Some(HostKind::ClaudeCli),
        conversation_id: Some("conv-qa7-dedup".into()),
        ..Default::default()
    };
    ctx.handle.create_question(req, now()).await.unwrap();
    let aid = ctx.handle.answer_question(id, ui_answer("a"), now()).await.unwrap().answer.unwrap().answer_id;
    let t = now() + Duration::minutes(1);
    // 同一時間、同內容送兩次（冪等，只一行）、再送不同內容（更新回饋，新的一行）。
    use kunsu_core::continuity::ReportStatus;
    ctx.handle.report_answer(id, &aid, ReportStatus::Done, "完成", t).await.unwrap();
    ctx.handle.report_answer(id, &aid, ReportStatus::Done, "完成", t).await.unwrap();
    ctx.handle.report_answer(id, &aid, ReportStatus::Done, "完成（補充）", t).await.unwrap();
    let file = ctx.handle.ledger().day_file(now());
    let raw = std::fs::read_to_string(&file).unwrap();
    let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
    let distinct = lines.len();
    // 模擬逾時後才寫成＋補寫：同一行（同 row_id）再出現一次。
    let dup = lines.iter().find(|l| l.contains("\"type\":\"execution\"")).unwrap().to_string();
    // 舊版（沒有 row_id）的兩行逐字相同的行。
    let mut legacy: Value = serde_json::from_str(&dup).unwrap();
    legacy.as_object_mut().unwrap().remove("row_id");
    let legacy = legacy.to_string();
    let mut content = raw.clone();
    for l in [&dup, &legacy, &legacy] {
        content.push_str(l);
        content.push('\n');
    }
    std::fs::write(&file, content).unwrap();
    let (rows, bad) = ctx.handle.ledger().read_all_rows().unwrap();
    assert!(bad.is_empty(), "{bad:?}");
    assert_eq!(rows.len(), distinct + 2, "同 row_id 的重複行只留一行，其餘（含舊行）都保留：{} 行", rows.len());
    let reports = rows.iter().filter(|r| r.ledger_type == LedgerType::Execution && r.row_id.is_some()).count();
    assert_eq!(reports, 2, "兩則不同內容的回饋都在");
}

// ---------------------------------------------------------------------------
// R7-12（應綠）：OPEN.json 待重寫的標記跨 app 重啟仍在：重啟後 vault 還壞著 → 重建拒絕；權限恢復後 tick 重寫，
// 換一顆全新資料庫也能還原那段期間建的待決題（CONTRACT §7 open_json_dirty）。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa7_open_json_dirty_survives_restart() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("qa7-dirty-1"), Utc::now()).await.unwrap();
    let tmp = ctx.handle.ledger().asks_dir().join(".OPEN.json.tmp");
    mkfifo(&tmp);
    ctx.handle.create_question(base_req("qa7-dirty-2"), Utc::now()).await.unwrap();
    // app 重啟（同一顆資料庫），vault 仍寫不進去。
    let restarted = ctx.reopen().await;
    let refused = restarted.rebuild_db().await;
    assert!(
        matches!(refused, Err(kunsu_server::ServerError::RebuildRefused(_))),
        "重啟後 OPEN.json 仍待重寫，應拒絕重建：{refused:?}"
    );
    assert!(restarted.get_question("qa7-dirty-2").await.unwrap().is_some());
    // 權限恢復：卡住的寫入放行；重啟後的 server 由 tick 重寫 OPEN.json。
    drain_fifo(&tmp);
    let _ = std::fs::remove_file(&tmp);
    restarted.tick(Utc::now() + Duration::seconds(31)).await.unwrap();
    let (h3, _discord, _db) = fresh_db(&ctx).await;
    h3.rebuild_db().await.unwrap();
    assert!(h3.get_question("qa7-dirty-2").await.unwrap().is_some(), "重寫後 OPEN.json 含重啟前那段期間建的題");
}

// ---------------------------------------------------------------------------
// R7-11：第 6 輪只遮了 prepare 階段的錯誤。commit 被 executor 拒絕時，`executor commit 失敗：<stdout/stderr 尾>`
// 原樣進 `/events` 的 Execution(Api, Fail) 與 Discord「批准後執行被擋」——executor 的錯誤輸出（例如 Python
// traceback 的檔案路徑、「token X 驗證失敗」）就把 approval 路徑與 executor token 帶出去。
// 規格（CONTRACT §8、AC-CC-14）：執行事件、execution_blocked 的 Discord 訊息與 /events 都先遮掉 executor token 與 approval 路徑。
// ---------------------------------------------------------------------------

fn write_commit_rejecting_executor(ctx: &TestCtx) {
    let body = format!(
        r#"#!/usr/bin/env python3
import json, os, sys

def flag(name, default=None):
    args = sys.argv[1:]
    return args[args.index(name) + 1] if name in args else default

token = "{FAKE_EXECUTOR_TOKEN}"
if len(sys.argv) > 1 and sys.argv[1] == "commit":
    print("Traceback (most recent call last):", file=sys.stderr)
    print("FileNotFoundError: [Errno 2] No such file or directory: '" + flag("--approval-file", "") + "'", file=sys.stderr)
    print("token " + flag("--token", "") + " 驗證失敗，未執行", file=sys.stderr)
    sys.exit(1)
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

#[tokio::test]
async fn qa7_commit_failure_masks_token_and_approval_path() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    write_commit_rejecting_executor(&env.ctx);
    let id = "qa7-commit-fail";
    let mut req = money_t0_req(id);
    req.source = claude_source(&env, "conv-qa7-cf", Some(&claude));
    env.ctx.handle.create_question(req, now()).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    let mut rx = env.ctx.handle.subscribe();
    env.reactor_on(&env.ctx.handle, Some(claude)).process_pending().await.unwrap();
    let exec = env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap().execution_result.unwrap_or_default();
    assert!(exec.starts_with("EXEC_BLOCKED"), "前提：commit 被擋：{exec}");
    let leaks = |s: &str| s.contains(FAKE_EXECUTOR_TOKEN) || s.contains("executor_pending/approvals");
    let frames: Vec<String> = drain_events(&mut rx)
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .filter(|f| leaks(f))
        .collect();
    let discord: Vec<String> = env
        .ctx
        .discord
        .messages()
        .into_iter()
        .filter(|m| leaks(&m.text))
        .map(|m| format!("[{}] {}", m.reason, m.text))
        .collect();
    assert!(
        frames.is_empty() && discord.is_empty(),
        "commit 失敗時 executor token／approval 路徑原樣外流：\n/events={frames:#?}\nDiscord={discord:#?}"
    );
}
