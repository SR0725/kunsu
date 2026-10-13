//! QA 第 4 輪（排程督導，BDD routine-check-in）：回頭驗證第 3 輪修正（`kunsu ask --json` 拒絕不認得的旗標、
//! `--x=y` 寫法與 `kunsu-` 保留前綴），並以真的 `kunsu` 執行檔對測試 server 端對端跑一次排程建題。
//!
//! CLI 測試需要先 `cargo build -p kunsu-cli`（找同一個 target 目錄下的 `kunsu`）；子行程一律 `env_clear()`，
//! 只給測試需要的變數，`KUNSU_BIND` 指向測試 server、`HOME`／`KUNSU_HOME_DIR` 指 tempdir——不碰使用者的 app 與檔案。

mod common;

use std::path::PathBuf;

use chrono::Duration;
use common::*;
use kunsu_core::continuity::{HostKind, ReportStatus, RoutineKind, Trigger};
use kunsu_core::types::{DefaultAction, Kind, OnExpire, State, Tier};
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

    /// 執行 `kunsu <args>`，stdin 給 `input`；env 只有 PATH／HOME／KUNSU_* 與 `extra`。
    async fn run(&self, args: &[&str], input: &str, extra: &[(&str, &str)]) -> (i32, String, String) {
        use tokio::io::AsyncWriteExt;
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
            .stderr(std::process::Stdio::piped());
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(input.as_bytes()).await.unwrap();
        drop(stdin);
        let out = child.wait_with_output().await.unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }

    fn inbox_files(&self) -> usize {
        std::fs::read_dir(self.home.path().join(".kunsu/inbox")).map(|d| d.count()).unwrap_or(0)
    }
}

fn check_in_json(id: &str) -> String {
    json!({
        "id": id,
        "title": "今天的日記",
        "question_line": "今天最重要的三件事？",
        "body_md": "寫下今天最重要的三件事。",
        "kind": "text",
        "tier": "t1",
        "evidence": [],
        "options": [],
        "default_action": {"on_expire": "noop_log"}
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// R4-1（應綠）：真的 `kunsu ask --json - --routine …` 在 Claude 排程任務環境建題 → 原對話身分與排程身分都存好，
// 秘密（messaging token／socket）不進題目、不進帳本。AC-CC-01、AC-RC-01。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r4_cli_claude_scheduled_task_end_to_end() {
    let ctx = TestCtx::new().await;
    let cli = Cli::new(&ctx).await;
    let id = "rt-diary-20261009";
    let env = [
        ("CLAUDE_CODE_ENTRYPOINT", "claude-desktop"),
        ("CLAUDE_CODE_SESSION_ID", "8d1f0c2e-1111-4222-8333-944455556666"),
        ("CLAUDE_CODE_HOST_SESSION_ID", "local_8d1f0c2e-aaaa"),
        ("CLAUDE_CODE_SESSION_ATTENDED", "0"),
        ("CLAUDE_CODE_EXECPATH", "/Applications/Claude.app/claude"),
        ("CLAUDE_CODE_MESSAGING_TOKEN", "SECRET-MSG-TOKEN"),
        ("CLAUDE_CODE_MESSAGING_SOCKET", "/tmp/cc-socks/secret.sock"),
    ];
    let (code, out, err) = cli
        .run(&["ask", "--json", "-", "--routine", "diary", "--routine-name", "每天寫日記", "--every", "24"], &check_in_json(id), &env)
        .await;
    assert_eq!(code, 0, "stdout={out} stderr={err}");
    let q = ctx.handle.get_question(id).await.unwrap().expect("題目已建立");
    let s = &q.source;
    assert_eq!(s.host, Some(HostKind::ClaudeDesktop));
    assert_eq!(s.conversation_id.as_deref(), Some("8d1f0c2e-1111-4222-8333-944455556666"));
    assert_eq!(s.host_session_id.as_deref(), Some("local_8d1f0c2e-aaaa"));
    assert_eq!(s.attended, Some(false));
    assert_eq!(s.routine_id.as_deref(), Some("diary"));
    assert_eq!(s.routine_name.as_deref(), Some("每天寫日記"));
    assert_eq!(s.expected_every_hours, Some(24.0));
    assert_eq!(s.trigger, Some(Trigger::Scheduled));
    assert_eq!(s.routine_kind, Some(RoutineKind::ClaudeScheduledTask));
    let text = serde_json::to_string(&q).unwrap();
    assert!(!text.contains("SECRET-MSG-TOKEN") && !text.contains("cc-socks"), "秘密不得進題目：{text}");
    for p in std::fs::read_dir(ctx.handle.ledger().asks_dir()).unwrap().flatten() {
        let raw = std::fs::read_to_string(p.path()).unwrap_or_default();
        assert!(!raw.contains("SECRET-MSG-TOKEN"), "秘密不得進帳本／OPEN.json：{}", p.path().display());
    }
}

// ---------------------------------------------------------------------------
// R4-2（應綠）：第 3 輪修正——`kunsu ask --json` 遇到 `--every=24`、拼錯的旗標、`--routine kunsu-…` 一律結束碼 64、
// 不送出（server 沒有題目、也沒有落 inbox）。AC-RC-01b。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r4_cli_rejects_bad_routine_flags_without_sending() {
    let ctx = TestCtx::new().await;
    let cli = Cli::new(&ctx).await;
    let cases: [(&str, &[&str]); 6] = [
        ("rt-eq", &["ask", "--json", "-", "--routine", "diary", "--every=24"]),
        ("rt-typo", &["ask", "--json", "-", "--routine", "diary", "--evry", "24"]),
        ("rt-reserved", &["ask", "--json", "-", "--routine", "kunsu-diary"]),
        ("rt-unit", &["ask", "--json", "-", "--routine", "diary", "--every", "24h"]),
        ("rt-novalue", &["ask", "--json", "-", "--routine", "diary", "--routine-name"]),
        ("rt-orphan", &["ask", "--json", "-", "--every", "24"]),
    ];
    for (id, args) in cases {
        let (code, out, err) = cli.run(args, &check_in_json(id), &[]).await;
        assert_eq!(code, 64, "{args:?} 應以 64 結束：stdout={out} stderr={err}");
        assert!(ctx.handle.get_question(id).await.unwrap().is_none(), "{args:?} 不得建題");
    }
    assert_eq!(cli.inbox_files(), 0, "也不得落 inbox");
}

// ---------------------------------------------------------------------------
// R4-3：Codex 主機的排程一律被 CLI 標成 `codex_cron`。Codex heartbeat automation（每次回到同一 thread）與
// cron automation（每次開新 thread）從環境變數分不出來，CLI 卻猜 cron——heartbeat 的 check-in 存成錯的 routine_kind。
// 規格（場景 1）：routine_kind「可由環境推得時」才自動帶入。
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r4_cli_does_not_guess_codex_routine_kind() {
    let ctx = TestCtx::new().await;
    let cli = Cli::new(&ctx).await;
    let id = "rt-hb-20261009";
    let env = [
        ("CODEX_THREAD_ID", "0199c5a1-2b3c-7d4e-8f90-a1b2c3d4e5f6"),
        ("__CFBundleIdentifier", "com.openai.codex"),
    ];
    let (code, out, err) =
        cli.run(&["ask", "--json", "-", "--routine", "weekly-hb", "--routine-name", "每週檢討"], &check_in_json(id), &env).await;
    assert_eq!(code, 0, "stdout={out} stderr={err}");
    let q = ctx.handle.get_question(id).await.unwrap().expect("題目已建立");
    assert_eq!(q.source.host, Some(HostKind::CodexApp), "前提");
    assert_eq!(q.source.conversation_id.as_deref(), Some("0199c5a1-2b3c-7d4e-8f90-a1b2c3d4e5f6"), "前提");
    assert_eq!(
        q.source.routine_kind, None,
        "環境推不出是 heartbeat 還是 cron，CLI 卻自動填了 {:?}（heartbeat automation 的 check-in 會被記成 cron）",
        q.source.routine_kind
    );
}

// ---------------------------------------------------------------------------
// R4-4（應綠）：check-in 不因為是排程就降級——T3 的文字 check-in 一樣 12 分鐘警告、15 分鐘強制。AC-RC-02。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r4_low_tier_check_in_still_forced() {
    let ctx = TestCtx::new().await;
    let id = "rt-diary-t3";
    let mut req = base_req(id);
    req.kind = Kind::Text;
    req.options = vec![];
    req.gate = None;
    req.tier = Tier::T3;
    req.source.routine_id = Some("diary".into());
    req.source.routine_name = Some("每天寫日記".into());
    req.source.expected_every_hours = Some(24.0);
    ctx.handle.create_question(req, now()).await.unwrap();
    ctx.handle.tick(now() + Duration::minutes(12)).await.unwrap();
    let warned = ctx.handle.get_question(id).await.unwrap().unwrap().state;
    ctx.handle.tick(now() + Duration::minutes(15)).await.unwrap();
    let forced = ctx.handle.get_question(id).await.unwrap().unwrap().state;
    assert_eq!((warned, forced), (State::Warned, State::Forced));
}

// ---------------------------------------------------------------------------
// R4-5（應綠）：重建＋收割前後督導資料一致：已回應（含 AI 回饋）、逾時依預設自動選定（未回應）、單純過期（未回應）、
// 取消——重建後題目一致；之後 drain 只把已回答的變 routed，未回應／取消不變。AC-RC-09。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r4_rebuild_then_drain_keeps_check_in_history() {
    let ctx = TestCtx::new().await;
    let routine = |id: &str| {
        let mut req = base_req(id);
        req.source.routine_id = Some("diary".into());
        req.source.routine_name = Some("每天寫日記".into());
        req.source.expected_every_hours = Some(24.0);
        req
    };
    // 已回應＋AI 回饋
    ctx.handle.create_question(routine("rt-diary-a"), now()).await.unwrap();
    let aid = ctx.handle.answer_question("rt-diary-a", ui_answer("a"), now() + Duration::minutes(1)).await.unwrap().answer.unwrap().answer_id;
    ctx.handle
        .report_answer("rt-diary-a", &aid, ReportStatus::Done, "已寫入日記", now() + Duration::minutes(2))
        .await
        .unwrap();
    // 逾時依預設自動選定
    let mut auto = routine("rt-diary-auto");
    auto.default_action = DefaultAction { on_expire: OnExpire::PickOption, option_id: Some("b".into()) };
    auto.deadline_at = Some(now() + Duration::hours(1));
    ctx.handle.create_question(auto, now()).await.unwrap();
    // 單純過期
    let mut expired = routine("rt-diary-exp");
    expired.deadline_at = Some(now() + Duration::hours(1));
    ctx.handle.create_question(expired, now()).await.unwrap();
    // 取消
    ctx.handle.create_question(routine("rt-diary-cx"), now()).await.unwrap();
    ctx.handle.cancel_question("rt-diary-cx", now() + Duration::minutes(3)).await.unwrap();
    ctx.handle.tick(now() + Duration::hours(2)).await.unwrap();

    let ids = ["rt-diary-a", "rt-diary-auto", "rt-diary-exp", "rt-diary-cx"];
    let mut before = Vec::new();
    for id in ids {
        before.push(ctx.handle.get_question(id).await.unwrap().unwrap());
    }
    let (h2, _db) = ctx.fresh_db_same_vault().await;
    h2.rebuild_db_at(now() + Duration::hours(3)).await.unwrap();
    for b in &before {
        let a = h2.get_question(&b.id).await.unwrap().unwrap();
        assert_eq!((a.state, &a.source, &a.answer), (b.state, &b.source, &b.answer), "{} 重建後不一致", b.id);
    }
    h2.answers_since("qa-r4", None, now() + Duration::hours(4)).await.unwrap();
    let state = |id: &str| {
        let h2 = h2.clone();
        let id = id.to_string();
        async move { h2.get_question(&id).await.unwrap().unwrap() }
    };
    let a = state("rt-diary-a").await;
    assert_eq!(a.state, State::Routed);
    assert_eq!(a.answer.unwrap().report.map(|r| r.summary_md), Some("已寫入日記".into()));
    let auto = state("rt-diary-auto").await;
    assert_eq!(auto.state, State::Expired, "逾時自動選定收割後仍是未回應");
    assert_eq!(auto.answer.map(|a| a.channel), Some(kunsu_core::types::Channel::AutoPolicy));
    assert_eq!(state("rt-diary-exp").await.state, State::Expired);
    assert_eq!(state("rt-diary-cx").await.state, State::Cancelled);
}
