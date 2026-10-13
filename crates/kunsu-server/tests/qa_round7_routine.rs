//! QA 第 7 輪（排程督導，BDD routine-check-in）：回頭驗證第 6 輪之後的修正在排程 check-in 上的樣子——等待中的 agent 讀得到
//! 使用者的退回（`ask_wait`／`kunsu wait` 回 `rejected {at, reason}`）、重建時 wake=none 的處理順序改動（錢題才重新排入 reactor）。
//!
//! CLI 測試需要先 `cargo build -p kunsu-cli`（找同一個 target 目錄下的 `kunsu`）；子行程一律 `env_clear()`，`KUNSU_BIND` 指向測試
//! server、`HOME`／`KUNSU_HOME_DIR` 指 tempdir。斷言寫規格期望；失敗訊息寫實際行為。標「應綠」的是確認行為成立的對照組。

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{Duration, Utc};
use common::*;
use kunsu_core::continuity::{DeliveryState, ReportStatus, RoutineKind, Trigger};
use kunsu_core::policy::Policy;
use kunsu_core::types::*;
use kunsu_server::*;
use serde_json::json;
use tempfile::TempDir;

fn kunsu_bin() -> PathBuf {
    let bin = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("kunsu");
    assert!(bin.exists(), "找不到 {}：先跑 `cargo build -p kunsu-cli`（同一個 CARGO_TARGET_DIR）", bin.display());
    bin
}

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
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd
    }
}

/// Claude 桌面排程任務的環境（`kunsu ask` 依這些補齊原對話與排程身分）。
fn scheduled_task_env() -> Vec<(&'static str, String)> {
    vec![
        ("CLAUDE_CODE_SESSION_ID", "3f1c2a9e-7b1d-4c55-9a51-2f0e8d6b1a77".to_string()),
        ("CLAUDE_CODE_ENTRYPOINT", "claude-desktop".to_string()),
        ("CLAUDE_CODE_HOST_SESSION_ID", "local_r7-diary".to_string()),
        ("CLAUDE_CODE_SESSION_ATTENDED", "0".to_string()),
        // 發問的對話行程還活著（測試行程本身），背景 `kunsu wait` 不會被當成孤兒。
        ("CLAUDE_PID", std::process::id().to_string()),
    ]
}

async fn fresh_db(ctx: &TestCtx) -> (ServerHandle, TempDir) {
    let db_dir = TempDir::new().unwrap();
    let handle = ServerHandle::new(KunsuConfig {
        db_path: db_dir.path().join("kunsu.db"),
        vault_root: ctx.vault.path().to_path_buf(),
        policy: Policy::default(),
        discord: Arc::new(RecordingDiscord::new()),
        guards: Arc::new(StubGuardProbe::new()),
    })
    .await
    .unwrap();
    (handle, db_dir)
}

// ---------------------------------------------------------------------------
// R7-1（應綠）：排程任務以真的 `kunsu ask --routine …` 建 check-in，背景掛著 `kunsu wait --timeout 0`；等待中使用者按「這題不該問我」
// 退回——背景等待在幾秒內結束（結束碼 0），印出 `cancelled` 與 `rejected` 的原因；排程身分完整；收割、重建後仍是「退回」。
// AC-RC-01、AC-RC-06、AC-RC-09；CONTRACT §7「等到被使用者退回的題回 rejected {at, reason}」。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r7_background_wait_learns_a_rejection_that_happens_while_waiting() {
    let ctx = TestCtx::new().await;
    let cli = Cli::new(&ctx).await;
    let env = scheduled_task_env();
    let extra: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let id = "rt-diary-20261009";
    let body = json!({
        "id": id, "title": "今天的日記", "question_line": "今天最重要的三件事？", "body_md": "寫下今天最重要的三件事。",
        "kind": "text", "tier": "t1", "evidence": [], "options": [], "default_action": {"on_expire": "noop_log"}
    })
    .to_string();
    let mut ask = cli
        .command(&["ask", "--json", "-", "--routine", "diary", "--routine-name", "每天寫日記", "--every", "24"], &extra)
        .spawn()
        .unwrap();
    {
        use tokio::io::AsyncWriteExt;
        let mut stdin = ask.stdin.take().unwrap();
        stdin.write_all(body.as_bytes()).await.unwrap();
    }
    let created = ask.wait_with_output().await.unwrap();
    assert!(created.status.success(), "建題：{}", String::from_utf8_lossy(&created.stderr));
    let q = ctx.handle.get_question(id).await.unwrap().expect("check-in 建好了");
    assert_eq!(q.source.trigger, Some(Trigger::Scheduled));
    assert_eq!(q.source.routine_kind, Some(RoutineKind::ClaudeScheduledTask));
    assert_eq!(q.wake_on_answer.mode, WakeMode::Continue, "有原對話身分 → 回答延續原對話");

    let waiter = cli.command(&["wait", id, "--timeout", "0"], &extra).spawn().unwrap();
    // 讓背景等待先掛上第一輪 ask_wait（真實時間 1 秒；之後的退回事件要把它叫醒）。
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    ctx.handle.reject_question(id, Some("今天休假，不用寫".into()), Utc::now()).await.unwrap();
    let out = tokio::time::timeout(std::time::Duration::from_secs(10), waiter.wait_with_output())
        .await
        .expect("退回後背景等待要在幾秒內結束")
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.code() == Some(0) && stdout.contains("\"cancelled\"") && stdout.contains("今天休假，不用寫"),
        "背景等待（結束碼 {:?}）：{stdout}",
        out.status.code()
    );

    ctx.handle.answers_since("routine-harvest", None, Utc::now()).await.unwrap();
    let (h2, _db) = fresh_db(&ctx).await;
    h2.rebuild_db_at(Utc::now()).await.unwrap();
    let after = h2.get_question(id).await.unwrap().unwrap();
    assert_eq!(after.state, State::Cancelled, "收割、重建後仍是退回（cancelled＋rejected）");
    assert_eq!(after.rejected.and_then(|r| r.reason).as_deref(), Some("今天休假，不用寫"));
    assert_eq!(after.source.routine_name.as_deref(), Some("每天寫日記"));
}

// ---------------------------------------------------------------------------
// R7-2（應綠）：沒有原對話身分的 check-in（只用 MCP 建題、wake=none）——使用者回答後 reactor 還沒定案交付就重建資料庫：重建不把它排進
// reactor（只有錢題不論 wake 都排入），只重新登記待決交付，下一次掃描定案「沒有原對話可接續」，不開任何對話；agent 用 `ask_report`
// 回饋後再重建，督導資料（回答、回饋、交付）一致。AC-RC-04、AC-RC-09、AC-CC-03；CONTRACT §7。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r7_wake_none_check_in_is_not_requeued_and_survives_rebuilds() {
    let ctx = TestCtx::new().await;
    let id = "rt-weekly-20261009";
    let mut req = base_req(id);
    req.kind = Kind::Text;
    req.options = vec![];
    req.gate = None;
    req.question_line = "這週目標做到幾成？".into();
    req.source.routine_id = Some("weekly".into());
    req.source.routine_name = Some("每週目標檢討".into());
    req.source.expected_every_hours = Some(168.0);
    assert_eq!(req.wake_on_answer.mode, WakeMode::None, "前提：沒有原對話身分、沒設 wake");
    ctx.handle.create_question(req, Utc::now()).await.unwrap();
    let answer = AnswerInput { choice_id: None, free_text: Some("七成，客戶專案拖到".into()), ..ui_answer("x") };
    let aid = ctx.handle.answer_question(id, answer, Utc::now()).await.unwrap().answer.unwrap().answer_id;

    let (h2, _db2) = fresh_db(&ctx).await;
    let report = h2.rebuild_db_at(Utc::now()).await.unwrap();
    let scratch = TempDir::new().unwrap();
    let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
    cfg.sessions_dir = scratch.path().join("registry");
    cfg.desktop_sessions_dir = scratch.path().join("desktop");
    cfg.claude_bin = Some(PathBuf::from("/usr/bin/false"));
    let r = Reactor::new(h2.clone(), cfg, Arc::new(StaticKeyProvider(test_key())));
    r.process_pending().await.unwrap();
    r.process_due_deliveries(Utc::now() + Duration::seconds(5)).await.unwrap();
    assert_eq!(report.requeued, 0, "非錢題 wake=none 不排進 reactor：{report:?}");
    assert!(r.planned().is_empty(), "不開任何對話：{:?}", r.planned());
    assert_eq!(h2.delivery_state(&aid).await.unwrap(), Some(DeliveryState::NoContinuation));

    h2.report_answer(id, &aid, ReportStatus::Done, "已寫進 vault 週檢討，下週先把客戶專案切小", Utc::now()).await.unwrap();
    let before = h2.get_question(id).await.unwrap().unwrap();
    let (h3, _db3) = fresh_db(&ctx).await;
    let report3 = h3.rebuild_db_at(Utc::now() + Duration::minutes(1)).await.unwrap();
    let after = h3.get_question(id).await.unwrap().unwrap();
    let (a0, a1) = (before.answer.unwrap(), after.answer.unwrap());
    assert_eq!(report3.requeued, 0);
    assert_eq!(a1.free_text, a0.free_text, "回答");
    assert_eq!(a1.report.map(|r| r.summary_md), a0.report.map(|r| r.summary_md), "AI 回饋");
    assert_eq!(a1.delivery.map(|d| d.state), Some(DeliveryState::NoContinuation), "交付");
    assert_eq!(after.source.trigger, Some(Trigger::Scheduled));
}
