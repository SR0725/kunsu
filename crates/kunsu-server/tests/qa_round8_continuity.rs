//! QA 第 8 輪（回答延續原對話，BDD conversation-continuity；INV-13、18、25）：回頭驗證第 7 輪的修正——重建拒絕規則、
//! `no_ack_alert` 補標、`ledger_stuck` 告警、doctor 帳本積欠——以及 main 新合進的題組（`kind: batch`）與純確認題
//! （`choice_id = "confirm"`）在回答延續、重建、agent 讀取面上的行為。
//!
//! 每個測試的斷言寫規格期望；失敗訊息寫實際行為。標「應綠」的是確認修正或新功能成立的對照組。
//! 時間一律注入（`rebuild_db_at`、`tick(now)`、`process_due_deliveries(now)`、`health(now)`），不等真實時間；
//! 只有模擬「vault 檔案呼叫卡在核心」時會吃到程式本身的 5 秒寫入逾時。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration, Utc};
use common::*;
use kunsu_core::continuity::{DeliveryState, HostKind};
use kunsu_core::policy::Policy;
use kunsu_core::types::*;
use kunsu_server::ledger::LedgerRow;
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
    /// 讓原對話「仍被活行程持有」：登記檔 sessionId 命中、pid 是本測試行程（一定活著）。
    fn hold(&self, conversation: &str) {
        let body = json!({"pid": std::process::id(), "sessionId": conversation});
        std::fs::write(self.root.path().join("registry/1.json"), body.to_string()).unwrap();
    }
}

const OK_BODY: &str = "echo 'EXEC_OK: 原對話已接手'";

fn claude_source(env: &Env, conversation: &str, cli_path: Option<&Path>) -> Source {
    Source {
        agent: AgentKind::Claude,
        cwd: env.work().to_string_lossy().to_string(),
        host: Some(HostKind::ClaudeDesktop),
        conversation_id: Some(conversation.into()),
        host_session_id: Some("local_qa8-1".into()),
        attended: Some(false),
        cli_path: cli_path.map(|p| p.to_string_lossy().to_string()),
        ..Default::default()
    }
}

fn fresh_wake() -> Wake {
    Wake { mode: WakeMode::Fresh, session_id: None, playbook: None }
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

fn mkfifo(path: &Path) {
    assert!(std::process::Command::new("/usr/bin/mkfifo").arg(path).status().unwrap().success());
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

/// 逐檔只保留 `keep` 回 true 的帳本行。
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

async fn outbox_count(db_path: &Path) -> u64 {
    let pool = kunsu_server::db::open_pool(db_path).await.unwrap();
    kunsu_server::store::ledger_outbox_count(&pool).await.unwrap()
}

fn kunsu_bin() -> PathBuf {
    let bin = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("kunsu");
    assert!(bin.exists(), "找不到 {}：先跑 `cargo build -p kunsu-cli`（同一個 CARGO_TARGET_DIR）", bin.display());
    bin
}

// ---- 題組 ----

/// 第 1、2 項可編輯、第 3 項 AI 建議不處理、第 4 項只核可。
fn batch_items() -> Value {
    json!([
        {"id": "i1", "title": "陳同學", "subtitle": "第 5 章打不開", "context_md": "老師好，第 5 章影片一直轉圈圈。",
         "proposal": "陳同學你好，已經修好了，請重新整理頁面。", "editable": true},
        {"id": "i2", "title": "某品牌窗口（品牌合作）", "context_md": "想邀請您開箱冷萃新品。",
         "proposal": "您好，謝謝邀請！目前內容主軸是 AI 與開發。", "editable": true},
        {"id": "i3", "title": "某軟體廠商", "context_md": "升級企業方案享 8 折。",
         "proposal": "（不回）", "editable": true, "default": "skip", "default_reason": "促銷信"},
        {"id": "i4", "title": "張同學退款", "subtitle": "Sample Pro · NT$4,800", "context_md": "購買 3 天、進度 8%。",
         "proposal": "符合退費規則 → 建議全額退款 NT$4,800", "editable": false}
    ])
}

fn batch_req(id: &str, source: Source) -> CreateRequest {
    let mut v = serde_json::to_value(base_req(id)).unwrap();
    let o = v.as_object_mut().unwrap();
    o.insert("title".into(), json!("今天 4 件事的回覆"));
    o.insert("question_line".into(), json!("今天 4 件事的回覆"));
    o.insert("kind".into(), json!("batch"));
    o.insert("gate".into(), Value::Null);
    o.insert("options".into(), json!([]));
    o.insert("items".into(), batch_items());
    o.insert("source".into(), serde_json::to_value(source).unwrap());
    serde_json::from_value(v).expect("題組 JSON 要能解析")
}

async fn item_hashes(handle: &ServerHandle, id: &str) -> Vec<String> {
    let q = handle.get_question(id).await.unwrap().unwrap();
    let v = serde_json::to_value(&q).unwrap();
    v["items"].as_array().expect("題組要有 items").iter().map(|i| i["content_hash"].as_str().unwrap().to_string()).collect()
}

/// i1 改過、i2 請 AI 改、i3 不處理、i4 照建議。
fn decisions(h: &[String]) -> Value {
    json!([
        {"item_id": "i1", "action": "approve", "text": "陳同學你好，修好了！\n有問題再回信給我。", "content_hash": h[0]},
        {"item_id": "i2", "action": "revise", "note": "語氣再熱情一點", "content_hash": h[1]},
        {"item_id": "i3", "action": "skip", "content_hash": h[2]},
        {"item_id": "i4", "action": "approve", "content_hash": h[3]}
    ])
}

fn batch_answer(items: Value, instruction: Option<&str>) -> AnswerInput {
    serde_json::from_value(json!({"channel": "ui", "items": items, "free_text": instruction})).unwrap()
}

fn confirm_req(id: &str, source: Source) -> CreateRequest {
    let mut req = base_req(id);
    req.kind = Kind::Confirm;
    req.options = vec![];
    req.gate = None;
    req.question_line = "日記寫完了嗎？".into();
    req.source = source;
    req
}

// ---------------------------------------------------------------------------
// R8-1（應綠）：題組答案回到原對話——第 1 輪的逐項決定、使用者改過的最終文字、請 AI 改的下一輪 id 都在追加的訊息裡，
// 只追加一次；重建資料庫後逐項決定、每項雜湊與交付狀態都和重建前一致，不會再追加一次。AC-CC-06、AC-CC-11；AC-BR-05、06。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa8_batch_answer_goes_back_once_and_survives_rebuild() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let id = "qa8-batch-conv";
    let t = Utc::now();
    env.ctx.handle.create_question(batch_req(id, claude_source(&env, "conv-qa8-batch", Some(&claude))), t).await.unwrap();
    let h = item_hashes(&env.ctx.handle, id).await;
    env.ctx.handle.answer_question(id, batch_answer(decisions(&h), Some("整體再短一點")), t).await.unwrap();
    let r = env.reactor_on(&env.ctx.handle, Some(claude.clone()));
    r.process_pending().await.unwrap();
    let calls = env.calls();
    assert_eq!(calls.len(), 1, "題組答案只在原對話追加一次：{calls:?}");
    let call = &calls[0];
    for needle in ["--resume conv-qa8-batch", "第 1 輪", "陳同學你好，修好了！", "語氣再熱情一點", "qa8-batch-conv-r2", "整體再短一點"] {
        assert!(call.contains(needle), "追加的訊息缺「{needle}」：{call}");
    }
    let before = env.ctx.handle.get_question(id).await.unwrap().unwrap();
    let a_before = before.answer.clone().unwrap();
    assert_eq!(a_before.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Resumed));

    let (h2, _discord, _db) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db_at(Utc::now()).await.unwrap();
    let after = h2.get_question(id).await.unwrap().expect("重建後題組還在");
    let a_after = after.answer.clone().unwrap();
    assert_eq!(
        serde_json::to_value(&a_after.items).unwrap(),
        serde_json::to_value(&a_before.items).unwrap(),
        "重建後逐項決定（含每項雜湊）要和重建前一致"
    );
    assert_eq!(item_hashes(&h2, id).await, h, "重建後每項的內容雜湊要和重建前一致");
    assert_eq!(a_after.delivery.as_ref().map(|d| d.state), Some(DeliveryState::Resumed));
    let r2 = env.reactor_on(&h2, Some(claude));
    r2.process_pending().await.unwrap();
    r2.process_due_deliveries(Utc::now() + Duration::minutes(1)).await.unwrap();
    assert_eq!(env.calls().len(), 1, "重建後題組答案又被追加一次：report={report:?} calls={:?}", env.calls());
}

// ---------------------------------------------------------------------------
// R8-2（應綠）：還沒回答的題組在重建後，使用者用重建前打開的面板（帶當時每項雜湊）作答照樣成立——
// 重建不會改變每項雜湊，不會讓使用者被迫重開面板。AC-BR-05、AC-CC-11。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa8_pending_batch_rebuilt_accepts_panel_hashes() {
    let env = Env::new().await;
    let id = "qa8-batch-pending";
    let t = Utc::now();
    env.ctx.handle.create_question(batch_req(id, claude_source(&env, "conv-qa8-bp", None)), t).await.unwrap();
    let h = item_hashes(&env.ctx.handle, id).await;
    let (h2, _discord, _db) = fresh_db(&env.ctx).await;
    let report = h2.rebuild_db_at(t + Duration::minutes(1)).await.unwrap();
    assert_eq!(report.pending_restored, 1, "{report:?}");
    let answered = h2.answer_question(id, batch_answer(decisions(&h), None), t + Duration::minutes(2)).await;
    assert!(answered.is_ok(), "重建後用重建前面板的雜湊作答被拒：{:?}", answered.err());
    assert_eq!(answered.unwrap().answer.unwrap().items.unwrap().len(), 4);
}

// ---------------------------------------------------------------------------
// R8-3（應綠）：題組的等待者——`ask_wait` 拿到含逐項決定的答案（所見即所送的文字原樣）、交付記 live_waiter，
// reactor 不追加。AC-CC-04、AC-BR-05。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa8_batch_waiter_gets_item_decisions() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let id = "qa8-batch-wait";
    let t = Utc::now();
    env.ctx.handle.create_question(batch_req(id, claude_source(&env, "conv-qa8-bw", Some(&claude))), t).await.unwrap();
    let h = item_hashes(&env.ctx.handle, id).await;
    let handle = env.ctx.handle.clone();
    let waiter =
        tokio::spawn(async move { mcp_on(&handle, "ask_wait", json!({"question_id": id, "timeout_seconds": 30})).await });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !env.ctx.handle.waiter_status(id, Utc::now()).0 {
        assert!(std::time::Instant::now() < deadline, "前提：等待者沒登記上");
        tokio::task::yield_now().await;
    }
    env.ctx.handle.answer_question(id, batch_answer(decisions(&h), None), Utc::now()).await.unwrap();
    let r = env.reactor_on(&env.ctx.handle, Some(claude));
    r.process_pending().await.unwrap();
    let got = tokio::time::timeout(std::time::Duration::from_secs(10), waiter).await.unwrap().unwrap().unwrap();
    assert_eq!(got["status"], "answered", "{got}");
    let items = got["answer"]["items"].as_array().expect("等待者拿到的答案要有逐項決定");
    assert_eq!(items.len(), 4);
    assert_eq!(items[0]["text"], "陳同學你好，修好了！\n有問題再回信給我。", "採用的最終文字要原樣（所見即所送）");
    assert_eq!(got["delivery"]["state"], "live_waiter", "{got}");
    assert!(env.calls().is_empty(), "等待者在線時 reactor 不該追加：{:?}", env.calls());
}

// ---------------------------------------------------------------------------
// R8-4（應綠）：純確認題（`choice_id = "confirm"`）。回到原對話的訊息寫「確認」；動錢的確認題不觸發錢鏈，
// agent 讀取面（ask_get）與追加訊息的錢鏈狀態都是「未觸發」，不是「錢鏈還在跑」。CONTRACT §2、AC-CC-06、AC-CC-14。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa8_confirm_reaches_original_conversation_and_money_stays_untriggered() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let t = Utc::now();
    // (a) 一般確認題
    let id = "qa8-confirm";
    env.ctx.handle.create_question(confirm_req(id, claude_source(&env, "conv-qa8-cf", Some(&claude))), t).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("confirm"), t).await.unwrap();
    // (b) 動錢的確認題（黃燈、沒有選項）：2026-10-09 起建題時就拒收（CONTRACT §2：動錢題必須有執行型選項），
    // 否則使用者按「確認」以為批准了、錢鏈卻永遠不會跑。
    let mid = "qa8-confirm-money";
    let mut req = money_t0_req(mid);
    req.kind = Kind::Confirm;
    req.options = vec![];
    req.deadline_at = Some(t + Duration::hours(6));
    req.source = claude_source(&env, "conv-qa8-cfm", Some(&claude));
    let err = env.ctx.handle.create_question(req, t).await.expect_err("動錢的純確認題要被拒收");
    assert!(err.to_string().contains("執行型選項"), "{err}");

    let r = env.reactor_on(&env.ctx.handle, Some(claude));
    r.process_pending().await.unwrap();
    let calls = env.calls();
    let plain = calls.iter().find(|c| c.contains("conv-qa8-cf ")).expect("一般確認題追加了一輪");
    assert!(plain.contains("使用者的選擇：確認（選項 id：confirm）"), "確認題的追加訊息要寫「確認」：{plain}");
    assert!(
        r.planned().iter().all(|c| c.kind != "prepare" && c.kind != "commit"),
        "確認題不觸發錢鏈：{:?}",
        r.planned().iter().map(|c| c.kind.clone()).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// R8-5（應綠）：重建拒絕規則不誤擋正常生命週期——待決、稍後再問過、已答＋收割（routed）、取消、這題不該問我、被取代、過期、
// 題組第 1 輪已答＋第 2 輪待決、確認題、internal 題（帳本行被刪也不擋）——在同一顆資料庫上重建照常成功，非 internal 題都還在。
// AC-CC-11、INV-13。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa8_rebuild_refusal_does_not_block_normal_lifecycle() {
    let ctx = TestCtx::new().await;
    let h = &ctx.handle;
    let t = Utc::now();
    h.create_question(base_req("qa8-l-pending"), t).await.unwrap();
    h.create_question(base_req("qa8-l-dismissed"), t).await.unwrap();
    h.dismiss_question("qa8-l-dismissed", t + Duration::minutes(1)).await.unwrap();
    h.create_question(base_req("qa8-l-routed"), t).await.unwrap();
    h.answer_question("qa8-l-routed", ui_answer("b"), t).await.unwrap();
    h.answers_since("qa8-consumer", None, t).await.unwrap();
    assert_eq!(h.get_question("qa8-l-routed").await.unwrap().unwrap().state, State::Routed, "前提：收割後 routed");
    h.create_question(base_req("qa8-l-cancel"), t).await.unwrap();
    h.cancel_question("qa8-l-cancel", t).await.unwrap();
    h.create_question(base_req("qa8-l-reject"), t).await.unwrap();
    h.reject_question("qa8-l-reject", Some("不該問我".into()), t).await.unwrap();
    h.create_question(base_req("qa8-l-super"), t).await.unwrap();
    h.supersede_question("qa8-l-super", t).await.unwrap();
    let mut exp = base_req("qa8-l-expired");
    exp.deadline_at = Some(t + Duration::minutes(5));
    h.create_question(exp, t).await.unwrap();
    h.tick(t + Duration::minutes(6)).await.unwrap();
    assert_eq!(h.get_question("qa8-l-expired").await.unwrap().unwrap().state, State::Expired, "前提：過期");
    let src = Source { agent: AgentKind::Claude, cwd: "/tmp".into(), ..Default::default() };
    h.create_question(batch_req("qa8-l-batch", src.clone()), t).await.unwrap();
    let hs = item_hashes(h, "qa8-l-batch").await;
    h.answer_question("qa8-l-batch", batch_answer(decisions(&hs), None), t).await.unwrap();
    h.create_question(batch_req("qa8-l-batch-r2", src.clone()), t).await.unwrap();
    h.create_question(confirm_req("qa8-l-confirm", src), t).await.unwrap();
    h.answer_question("qa8-l-confirm", ui_answer("confirm"), t).await.unwrap();
    let mut internal = base_req("qa8-l-internal");
    internal.internal = true;
    h.create_question(internal, t).await.unwrap();
    h.answer_question("qa8-l-internal", ui_answer("b"), t).await.unwrap();
    keep_ledger_rows(&ctx, |row| row.question.id != "qa8-l-internal");

    let before: Vec<(String, State)> = h
        .list_questions(true)
        .await
        .unwrap()
        .into_iter()
        .filter(|q| !q.internal)
        .map(|q| (q.id, q.state))
        .collect();
    let report = h.rebuild_db_at(t + Duration::minutes(10)).await;
    assert!(report.is_ok(), "正常生命週期的題目讓重建被拒：{:?}", report.err().map(|e| e.to_string()));
    for (id, state) in before {
        let q = h.get_question(&id).await.unwrap();
        assert!(q.is_some(), "重建後 {id} 不見了");
        let after = q.unwrap().state;
        // routed 只是收割記號（帳本沒有 routed 行），重建後回到 answered；UI 兩者都是「已回應」。
        let same = after == state || (state == State::Routed && after == State::Answered);
        assert!(same, "{id} 重建前 {state:?}、重建後 {after:?}");
    }
}

// ---------------------------------------------------------------------------
// R8-6（應綠）：OPEN.json 與帳本對同一題的狀態不同（寫完帳本 answered 行、還沒重寫 OPEN.json 就當機）——
// 重建以帳本的終態為準，不會把已回答的題還原成待決。INV-13。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa8_rebuild_prefers_ledger_terminal_over_stale_open_json() {
    let ctx = TestCtx::new().await;
    let t = Utc::now();
    let id = "qa8-stale-open";
    ctx.handle.create_question(base_req(id), t).await.unwrap();
    ctx.handle.create_question(base_req("qa8-stale-open-2"), t).await.unwrap();
    let open_path = ctx.handle.ledger().open_json_path();
    let snapshot_with_pending = std::fs::read_to_string(&open_path).unwrap();
    ctx.handle.answer_question(id, ui_answer("b"), t + Duration::minutes(1)).await.unwrap();
    // 當機：OPEN.json 停在作答前（題目仍待決）。
    std::fs::write(&open_path, snapshot_with_pending).unwrap();
    let report = ctx.handle.rebuild_db_at(t + Duration::minutes(2)).await.unwrap();
    let q = ctx.handle.get_question(id).await.unwrap().unwrap();
    assert_eq!(q.state, State::Answered, "OPEN.json 比帳本舊時，重建把已回答的題還原成 {:?}：{report:?}", q.state);
    assert!(q.answer.is_some());
    assert!(ctx.handle.get_question("qa8-stale-open-2").await.unwrap().unwrap().answer.is_none());
}

// ---------------------------------------------------------------------------
// R8-7：重建後「approval 已消耗、沒有執行紀錄」的錢題被列進人工對帳（reconcile），但 `reauthorize_question` 照樣放行：
// 重建清掉了 reactor 列，重新授權改用 `enqueue_reactor` 建一列 → 錢鏈（prepare → 簽章 → commit）再跑一次。
// 重建前同一題（reactor 列卡在 needs_reconcile）重新授權會被拒。
// 規格：CONTRACT §7「錢題若已有 approval_consumed 卻沒有執行紀錄，不論作答多久，都不重新排入」；BDD 場景 11「錢題若 approval 已
// 消耗卻沒有執行紀錄，不重跑，改發 Discord 待人工對帳」；AC-CC-14「錢不會被再動一次」。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa8_reauthorize_after_rebuild_does_not_rerun_reconcile_money() {
    let ctx = TestCtx::new().await;
    let id = "qa8-reconcile-money";
    let t = Utc::now();
    let mut req = money_t0_req(id);
    req.wake_on_answer = fresh_wake();
    req.deadline_at = Some(t + Duration::hours(6));
    ctx.handle.create_question(req, t).await.unwrap();
    let aid = ctx.handle.answer_question(id, ui_answer("a"), t).await.unwrap().answer.unwrap().answer_id;
    ctx.handle.record_approval_issued(id, &aid, "n-qa8-rc", FAKE_EXECUTOR_TOKEN, "h", t.timestamp(), t).await.unwrap();
    let record = json!({"question_id": id, "nonce": "n-qa8-rc", "param_hash": "h"});
    let consumed = ctx.handle.consume_approval(id, "n-qa8-rc", &record, t).await.unwrap();
    assert!(matches!(consumed, ApprovalOutcome::Consumed), "前提：approval 已消耗（executor 可能已 commit）");
    // 對照：重建前 reactor 列還在（排隊中／執行中），重新授權被拒。
    let before = ctx.handle.reauthorize_question(id, t + Duration::minutes(1)).await;
    assert!(before.is_err(), "前提：重建前重新授權被拒");
    // 執行紀錄沒進帳本（app 在 commit 中間當掉）；重建。
    let (h2, discord, _db) = fresh_db(&ctx).await;
    let report = h2.rebuild_db_at(t + Duration::minutes(5)).await.unwrap();
    assert_eq!(report.reconcile, 1, "前提：列進人工對帳：{report:?}");
    assert!(discord.messages().iter().any(|m| m.reason == "rebuild_reconcile"), "前提：發了人工對帳");
    let after = h2.reauthorize_question(id, t + Duration::minutes(6)).await;
    // 放行的話，reactor 真的會再跑一次錢鏈（假 executor：prepare 照引數回填 spec，commit 印出成功）。
    let mut kinds = Vec::new();
    if after.is_ok() {
        write_fake_executor(&ctx);
        let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
        let scratch = TempDir::new().unwrap();
        cfg.sessions_dir = scratch.path().join("registry");
        cfg.desktop_sessions_dir = scratch.path().join("desktop");
        cfg.claude_bin = Some(PathBuf::from("/usr/bin/true"));
        cfg.retry_base_ms = 1;
        let r = Reactor::new(h2.clone(), cfg, Arc::new(StaticKeyProvider(test_key())));
        r.process_pending().await.unwrap();
        kinds = r.planned().iter().filter(|c| c.executed).map(|c| c.kind.clone()).collect();
    }
    let result = h2.get_question(id).await.unwrap().unwrap().answer.unwrap().execution_result;
    assert!(
        after.is_err(),
        "重建後「approval 已消耗、等人工對帳」的錢題可以重新授權；reactor 接著實際執行了 {kinds:?}，執行紀錄={result:?}（錢鏈再跑一次）"
    );
}

// ---------------------------------------------------------------------------
// R8-8（應綠）：重建補標 `no_ack_alert` 只補「重建當下已超過 24 小時」的答案。重建時才 23 小時、交付 held 的答案，
// 跨過 24 小時後照常發「你答的 X 24 小時沒有任何 agent 處理」。AC-CC-16。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa8_no_ack_backfill_does_not_swallow_alert_for_young_held_answer() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let id = "qa8-young-held";
    let t_answer = Utc::now() - Duration::hours(23);
    env.ctx.handle.create_question(base_req_with(id, claude_source(&env, "conv-qa8-yh", Some(&claude))), t_answer).await.unwrap();
    env.ctx.handle.answer_question(id, ui_answer("b"), t_answer).await.unwrap();
    env.hold("conv-qa8-yh");
    let r = env.reactor_on(&env.ctx.handle, Some(claude.clone()));
    r.process_pending().await.unwrap();
    let d = env.ctx.handle.get_question(id).await.unwrap().unwrap().answer.unwrap().delivery.unwrap();
    assert_eq!(d.state, DeliveryState::Held, "前提：原對話被開著 → held");
    let (h2, discord, _db) = fresh_db(&env.ctx).await;
    let t = Utc::now();
    let report = h2.rebuild_db_at(t).await.unwrap();
    assert!(report.stale.is_empty(), "23 小時的答案不是 stale：{report:?}");
    h2.tick(t + Duration::hours(2)).await.unwrap();
    let alerts: Vec<String> = discord.messages().iter().map(|m| m.reason.clone()).collect();
    assert!(
        alerts.iter().any(|r| r == "no_ack_24h"),
        "重建時 23 小時的 held 答案跨過 24 小時後沒有告警（被重建補標吞掉）：{alerts:?}"
    );
}

fn base_req_with(id: &str, source: Source) -> CreateRequest {
    let mut req = base_req(id);
    req.source = source;
    req
}

// ---------------------------------------------------------------------------
// R8-9（應綠）：帳本還原出「追加中」（resuming）且作答超過 24 小時的答案——重建補標 no_ack_alert，卡住規則把它記 dead 並
// 只發一則「交付中斷」，不再另發 24 小時告警，也不列進 stale 讓使用者收兩則。AC-CC-16、CONTRACT §8。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa8_old_resuming_after_rebuild_alerts_exactly_once() {
    let env = Env::new().await;
    let id = "qa8-old-resuming";
    let t_answer = Utc::now() - Duration::hours(30);
    env.ctx.handle.create_question(base_req_with(id, claude_source(&env, "conv-qa8-or", None)), t_answer).await.unwrap();
    let aid = env.ctx.handle.answer_question(id, ui_answer("b"), t_answer).await.unwrap().answer.unwrap().answer_id;
    // reactor 佔住交付權（resuming）後 app 被殺：帳本留下 resuming 行。
    assert!(env
        .ctx
        .handle
        .set_delivery(id, &aid, DeliveryState::Resuming, None, None, t_answer + Duration::seconds(5))
        .await
        .unwrap());
    let (h2, discord, _db) = fresh_db(&env.ctx).await;
    let t = Utc::now();
    let report = h2.rebuild_db_at(t).await.unwrap();
    let r2 = env.reactor_on(&h2, None);
    r2.process_due_deliveries(t).await.unwrap();
    h2.tick(t + Duration::seconds(1)).await.unwrap();
    h2.tick(t + Duration::hours(3)).await.unwrap();
    let mine: Vec<String> = discord
        .messages()
        .iter()
        .filter(|m| m.question_id.as_deref() == Some(id) || m.text.contains(id) || m.text.contains("測試題"))
        .map(|m| m.reason.clone())
        .collect();
    let d = h2.get_question(id).await.unwrap().unwrap().answer.unwrap().delivery.unwrap();
    assert_eq!(d.state, DeliveryState::Dead, "卡住的追加重建後記 dead");
    assert_eq!(
        mine.iter().filter(|r| *r == "delivery_interrupted" || *r == "no_ack_24h" || *r == "rebuild_stale_unexecuted").count(),
        1,
        "同一題重建後應恰好一則告警：{mine:?} report={report:?}"
    );
}

// ---------------------------------------------------------------------------
// R8-10：帳本寫入卡住超過 2 分鐘的 Discord（`ledger_stuck`）——同一次卡住只發一次、`/health` 看得到卡住秒數；
// 卡住解除（tick 看到沒有在途寫入）後重新武裝，下一次卡住再發一次。（應綠）
// ---------------------------------------------------------------------------

/// 用 FIFO 取代當日帳本檔：之後的帳本寫入會卡在 open（沒有讀端）。回傳原內容。
fn jam_day_file(ctx: &TestCtx, at: chrono::DateTime<Utc>) -> (PathBuf, String) {
    let day = ctx.handle.ledger().day_file(at);
    let saved = std::fs::read_to_string(&day).unwrap_or_default();
    let _ = std::fs::remove_file(&day);
    mkfifo(&day);
    (day, saved)
}

/// 放行卡住的寫入並把帳本檔換回一般檔（含卡住那一行）。
fn unjam_day_file(day: &Path, saved: &str) {
    let released = drain_fifo(day);
    std::fs::remove_file(day).unwrap();
    let mut content = saved.to_string();
    for l in released {
        content.push_str(&l);
        content.push('\n');
    }
    std::fs::write(day, content).unwrap();
}

fn stuck_alerts(discord: &RecordingDiscord) -> usize {
    discord.messages().iter().filter(|m| m.reason == "ledger_stuck").count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa8_ledger_stuck_alerts_once_per_episode_and_rearms() {
    let ctx = TestCtx::new().await;
    for i in 0..3 {
        ctx.handle.create_question(base_req(&format!("qa8-stuck-{i}")), Utc::now()).await.unwrap();
    }
    // 第一次卡住。
    let t1 = Utc::now();
    let (day, saved) = jam_day_file(&ctx, t1);
    ctx.handle.cancel_question("qa8-stuck-0", t1).await.unwrap();
    for k in 1..=20 {
        ctx.handle.tick(t1 + Duration::seconds(31 * k)).await.unwrap();
    }
    let health = ctx.handle.health(t1 + Duration::seconds(31 * 20)).await.unwrap();
    assert!(health.ledger_stuck_s >= 120, "卡住 10 分鐘，/health ledger_stuck_s={}", health.ledger_stuck_s);
    assert_eq!(stuck_alerts(&ctx.discord), 1, "同一次卡住要恰好一則 ledger_stuck");
    // 解除：卡住的寫入回來，tick 補寫並看到沒有在途寫入。
    unjam_day_file(&day, &saved);
    ctx.handle.tick(t1 + Duration::seconds(31 * 21)).await.unwrap();
    ctx.handle.tick(t1 + Duration::seconds(31 * 22)).await.unwrap();
    assert_eq!(ctx.handle.health(Utc::now()).await.unwrap().ledger_stuck_s, 0, "前提：解除");
    // 第二次卡住。
    let t2 = Utc::now();
    let saved2 = std::fs::read_to_string(&day).unwrap();
    let _ = std::fs::remove_file(&day);
    mkfifo(&day);
    ctx.handle.cancel_question("qa8-stuck-1", t2).await.unwrap();
    for k in 1..=10 {
        ctx.handle.tick(t2 + Duration::seconds(31 * k)).await.unwrap();
    }
    let n = stuck_alerts(&ctx.discord);
    unjam_day_file(&day, &saved2);
    assert_eq!(n, 2, "解除後第二次卡住超過 2 分鐘要再發一則（目前共 {n} 則）");
}

// ---------------------------------------------------------------------------
// R8-11：上一次卡住的寫入一回來、下一筆寫入（例如補寫佇列的那一行）立刻又卡住——中間沒有 tick 剛好看到「沒有在途寫入」，
// `ledger_stuck_alerted` 不會重置 → 第二次卡住超過 2 分鐘不再告警。規格（CONTRACT §7）：在途寫入卡住超過 2 分鐘發一次 Discord，
// 寫入結束後解除——每一次卡住都要被看見。（嚴重度低：要兩次卡住剛好落在同一個 tick 間隔內）
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa8_back_to_back_stuck_writes_alert_again() {
    let ctx = TestCtx::new().await;
    for i in 0..3 {
        ctx.handle.create_question(base_req(&format!("qa8-b2b-{i}")), Utc::now()).await.unwrap();
    }
    let t1 = Utc::now();
    let (day, saved) = jam_day_file(&ctx, t1);
    ctx.handle.cancel_question("qa8-b2b-0", t1).await.unwrap();
    for k in 1..=6 {
        ctx.handle.tick(t1 + Duration::seconds(31 * k)).await.unwrap();
    }
    assert_eq!(stuck_alerts(&ctx.discord), 1, "前提：第一次卡住發了一則");
    // 卡住的寫入回來了（讀端打開過又關掉），但帳本檔仍是 FIFO：下一筆寫入（先補寫佇列那一行）立刻又卡住。
    let _ = drain_fifo(&day);
    let t2 = Utc::now();
    ctx.handle.cancel_question("qa8-b2b-1", t2).await.unwrap();
    assert!(ctx.handle.ledger().stuck_since().is_some_and(|s| s >= t1), "前提：又有一筆在途寫入卡住");
    for k in 1..=10 {
        ctx.handle.tick(t2 + Duration::seconds(31 * k)).await.unwrap();
    }
    let n = stuck_alerts(&ctx.discord);
    let stuck_s = ctx.handle.health(t2 + Duration::seconds(310)).await.unwrap().ledger_stuck_s;
    unjam_day_file(&day, &saved);
    assert_eq!(
        n, 2,
        "第二次卡住 {stuck_s} 秒（/health 看得到）卻沒有再發 ledger_stuck：兩次卡住之間沒有 tick 看到「沒有在途寫入」，告警旗標沒重置"
    );
}

// ---------------------------------------------------------------------------
// R8-12：`kunsu doctor` 在 app（server）不通時完全不印 `ledger_backlog` 那一行——只有 health ❌。資料庫裡的待補寫佇列
// 照樣在（server 掛掉時佇列還在 SQLite），而 CONTRACT §7 要使用者「刪資料庫前先確認 `kunsu doctor` 顯示帳本沒有積欠」：
// app 沒開時 doctor 給不出這個答案，也沒說「無法確認」。規格：AC-CC-11「`/health` 與 `kunsu doctor` 看得到帳本積欠」；
// CONTRACT §5「kunsu doctor 印出 ledger_backlog 一行」。（`kunsu rebuild-db` 本地通道會先補寫，所以只影響「刪資料庫」這條路）
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa8_doctor_with_app_down_still_reports_ledger_backlog() {
    let ctx = TestCtx::new().await;
    let id = "qa8-doctor-down";
    ctx.handle.create_question(base_req(id), Utc::now()).await.unwrap();
    let t0 = Utc::now();
    let day = ctx.handle.ledger().day_file(t0);
    if !day.exists() {
        std::fs::write(&day, "").unwrap();
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&day, std::fs::Permissions::from_mode(0o444)).unwrap();
    ctx.handle.cancel_question(id, t0).await.unwrap();
    std::fs::set_permissions(&day, std::fs::Permissions::from_mode(0o644)).unwrap();
    let backlog = outbox_count(&ctx.db_path).await;
    assert!(backlog > 0, "前提：資料庫裡有待補寫的帳本行");
    // app 沒開：找一個沒人聽的 port。
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
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
        .env("KUNSU_BIND", format!("127.0.0.1:{port}"))
        .env("KUNSU_VAULT_ROOT", ctx.vault.path())
        .env("KUNSU_DB_PATH", &ctx.db_path)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    eprintln!("R8-12 doctor（app 沒開、資料庫有 {backlog} 行積欠）輸出：\n{stdout}");
    assert!(
        stdout.contains("ledger_backlog"),
        "app 沒開、資料庫有 {backlog} 行帳本積欠時，doctor 沒有 ledger_backlog 那一行（既沒顯示積欠，也沒說無法確認）"
    );
}

// ---------------------------------------------------------------------------
// R8-13：題目在帳本裡有行、只是那一行壞了（同步衝突截斷、手動編輯）——重建拒絕（正確），但錯誤訊息說「在 vault 帳本與
// OPEN.json 都找不到」，沒提到帳本有壞行、在哪一行。使用者打開帳本會看到那題明明在。規格：BDD 場景 11「錯誤寫明原因」。
// （嚴重度低：拒絕本身是對的）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa8_rebuild_refusal_names_bad_ledger_lines() {
    let ctx = TestCtx::new().await;
    let id = "qa8-bad-line";
    let t = Utc::now();
    ctx.handle.create_question(base_req(id), t).await.unwrap();
    ctx.handle.answer_question(id, ui_answer("b"), t).await.unwrap();
    // 把這題的 answered 行截斷成壞 JSON。
    for p in jsonl_files(&ctx) {
        let fixed: Vec<String> = std::fs::read_to_string(&p)
            .unwrap()
            .lines()
            .map(|l| {
                if l.contains(id) && l.contains("\"answered\"") {
                    l.chars().take(l.chars().count() / 2).collect()
                } else {
                    l.to_string()
                }
            })
            .collect();
        std::fs::write(&p, fixed.join("\n") + "\n").unwrap();
    }
    let err = ctx.handle.rebuild_db_at(t + Duration::minutes(1)).await.expect_err("前提：拒絕重建").to_string();
    eprintln!("R8-13 拒絕訊息：{err}");
    assert!(ctx.handle.get_question(id).await.unwrap().is_some(), "前提：資料庫保持原樣");
    assert!(
        err.contains("壞") || err.contains("無法解析") || err.contains(".jsonl:"),
        "帳本有那題的壞行，拒絕訊息卻只說「都找不到」、沒指出壞行：{err}"
    );
}

// ---------------------------------------------------------------------------
// R8-14：不需喚醒（wake=none）的答案被 Lane 1 收割（drain 標 acked_at、狀態 routed）——drain 不寫帳本，所以帳本最後一份快照
// 的 acked_at 是空的。作答後 24 小時內重建 → acked_at 回到空白、游標重置 → 跨過 24 小時就發「你答的 X 24 小時沒有任何
// agent 處理」，但它早就被收割了。規格：INV-13「重建含每個答案的交付狀態」；AC-CC-16 只有真的沒人接手才告警。
// （嚴重度低：下一次 drain 會再收割一次並補標；只在 drain 排程晚於 24 小時那一刻時誤報）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa8_drained_answer_is_not_false_alerted_after_rebuild() {
    let ctx = TestCtx::new().await;
    let id = "qa8-drained";
    let t_answer = Utc::now() - Duration::hours(2);
    ctx.handle.create_question(base_req(id), t_answer).await.unwrap();
    ctx.handle.answer_question(id, ui_answer("b"), t_answer).await.unwrap();
    // reactor 定案 no_continuation（帳本 delivery 行），之後 Lane 1 收割。
    let scratch = TempDir::new().unwrap();
    let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
    cfg.sessions_dir = scratch.path().join("registry");
    cfg.desktop_sessions_dir = scratch.path().join("desktop");
    let r = Reactor::new(ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key())));
    r.process_due_deliveries(t_answer + Duration::seconds(5)).await.unwrap();
    ctx.handle.answers_since("mkt-daily", None, t_answer + Duration::minutes(5)).await.unwrap();
    let q = ctx.handle.get_question(id).await.unwrap().unwrap();
    assert!(q.answer.as_ref().unwrap().acked_at.is_some() && q.state == State::Routed, "前提：已收割");
    let (h2, discord, _db) = fresh_db(&ctx).await;
    let t = Utc::now();
    h2.rebuild_db_at(t).await.unwrap();
    let acked = h2.get_question(id).await.unwrap().unwrap().answer.unwrap().acked_at;
    h2.tick(t + Duration::hours(23)).await.unwrap();
    let alerts: Vec<String> = discord.messages().iter().map(|m| format!("[{}] {}", m.reason, m.text)).collect();
    assert!(
        !alerts.iter().any(|m| m.starts_with("[no_ack_24h]")),
        "已被 Lane 1 收割的答案重建後 acked_at={acked:?}，跨過 24 小時被當成沒人處理：{alerts:?}"
    );
}

// ---------------------------------------------------------------------------
// R8-15（應綠）：真的 `kunsu wait`（CLI 執行檔、HOME 指 tempdir、指向測試 server）——
// (a) 題組：背景等待者拿到含逐項決定的答案 JSON、結束碼 0、交付 live_waiter，reactor 不追加；
// (b) 答案已在原對話追加（resumed）後才來的等待者：印出答案與交付去向、結束碼 5；
// (c) 發問對話行程（$CLAUDE_PID）已不在：結束碼 4。AC-CC-04、AC-CC-18、AC-BR-05。
// ---------------------------------------------------------------------------

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

    fn command(&self, args: &[&str], extra: &[(&str, &str)]) -> tokio::process::Command {
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
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa8_cli_wait_batch_resumed_and_orphan_exit_codes() {
    let env = Env::new().await;
    let claude = env.script("claude.sh", OK_BODY);
    let cli = Cli::new(&env.ctx).await;
    let t = Utc::now();

    // (a) 題組＋背景等待者
    let id = "qa8-cli-batch";
    env.ctx.handle.create_question(batch_req(id, claude_source(&env, "conv-qa8-cli-b", Some(&claude))), t).await.unwrap();
    let h = item_hashes(&env.ctx.handle, id).await;
    let child = cli.command(&["wait", id, "--timeout", "0"], &[]).spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !env.ctx.handle.waiter_status(id, Utc::now()).0 {
        assert!(std::time::Instant::now() < deadline, "前提：kunsu wait 沒登記成等待者");
        tokio::task::yield_now().await;
    }
    env.ctx.handle.answer_question(id, batch_answer(decisions(&h), None), Utc::now()).await.unwrap();
    let r = env.reactor_on(&env.ctx.handle, Some(claude.clone()));
    r.process_pending().await.unwrap();
    let out = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output()).await.unwrap().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code(), Some(0), "題組等待者結束碼：{stdout}");
    let res: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(res["answer"]["items"].as_array().map(Vec::len), Some(4), "{stdout}");
    assert_eq!(res["delivery"]["state"], "live_waiter", "{stdout}");
    assert!(env.calls().is_empty(), "等待者拿走了，reactor 不追加：{:?}", env.calls());

    // (b) 已追加後才來的等待者
    let id2 = "qa8-cli-late";
    env.ctx.handle.create_question(base_req_with(id2, claude_source(&env, "conv-qa8-cli-l", Some(&claude))), t).await.unwrap();
    env.ctx.handle.answer_question(id2, ui_answer("b"), Utc::now()).await.unwrap();
    r.process_pending().await.unwrap();
    assert_eq!(env.calls().len(), 1, "前提：已在原對話追加");
    let out = cli.command(&["wait", id2, "--timeout", "5"], &[]).output().await.unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code(), Some(5), "答案已在原對話追加，遲到的等待者要結束碼 5：{stdout}");
    assert!(stdout.contains("\"resumed\""), "{stdout}");

    // (c) 發問對話行程已不在 → 結束碼 4
    let id3 = "qa8-cli-orphan";
    env.ctx.handle.create_question(base_req_with(id3, claude_source(&env, "conv-qa8-cli-o", Some(&claude))), t).await.unwrap();
    let mut gone = std::process::Command::new("/usr/bin/true").spawn().unwrap();
    let gone_pid = gone.id().to_string();
    gone.wait().unwrap();
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        cli.command(&["wait", id3, "--timeout", "0"], &[("CLAUDE_PID", gone_pid.as_str())]).output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(out.status.code(), Some(4), "發問對話已不在：{}", String::from_utf8_lossy(&out.stderr));
}
