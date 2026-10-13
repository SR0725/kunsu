//! QA 第 5 輪（排程督導，BDD routine-check-in）：回頭驗證第 4 輪的修正之外，檢查 main 新合進的題組（`kind: batch`）
//! 作為排程 check-in 時的伺服器面——排程身分、強制循環、AI 回饋、重建與收割一致——以及草稿註冊路徑套用排程預設。
//! 督導頁「你的回答」的顯示問題在 `ui/src/routines/qa_round5_routine.test.ts`（vitest）。
//!
//! 斷言寫規格期望；失敗訊息寫實際行為。標「應綠」的是確認行為成立的對照組。時間一律注入。

mod common;

use chrono::Duration;
use common::*;
use kunsu_core::continuity::{ReportStatus, RoutineKind, Trigger};
use kunsu_core::types::*;
use kunsu_server::mcp::{handle_rpc, McpReply};
use kunsu_server::*;
use serde_json::{json, Value};
use tempfile::TempDir;

async fn mcp(ctx: &TestCtx, tool: &str, args: Value) -> Result<Value, String> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}});
    match handle_rpc(&ctx.handle, body).await {
        McpReply::Json(v) => match v.get("error") {
            Some(e) => Err(e["message"].as_str().unwrap_or_default().to_string()),
            None => Ok(v["result"]["structuredContent"].clone()),
        },
        McpReply::Accepted => Err("accepted".into()),
    }
}

/// 每週回信 check-in：題組 2 項，排程身分由 agent 自填（`kunsu ask --routine` 會補的那些欄位）。
fn weekly_batch_json(id: &str) -> Value {
    json!({
        "id": id,
        "title": "這週的回信",
        "question_line": "這週 2 封回信",
        "body_md": "每週一次：看完 AI 擬好的回信再送出。",
        "kind": "batch",
        "tier": "t2",
        "evidence": [],
        "options": [],
        "default_action": {"on_expire": "noop_log"},
        "items": [
            {"id": "m1", "title": "陳同學", "context_md": "第 5 章打不開", "proposal": "已修好，請重新整理。", "editable": true},
            {"id": "m2", "title": "某品牌窗口", "context_md": "合作邀約", "proposal": "謝謝邀請，這次先婉拒。", "editable": true}
        ],
        "source": {
            "agent": "claude", "cwd": "/tmp",
            "routine_id": "weekly-mail", "routine_name": "每週回信", "routine_kind": "claude_scheduled_task",
            "expected_every_hours": 168
        }
    })
}

// ---------------------------------------------------------------------------
// R5-3（應綠）：題組當排程 check-in——排程身分照存（trigger 預設 scheduled）、一樣 12 分鐘警告 15 分鐘強制、回答與 AI 回饋
// 照存；重建資料庫＋收割後逐項決定、回饋與排程身分都一致。AC-RC-01、02、04、09。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r5_batch_check_in_keeps_routine_identity_through_rebuild_and_drain() {
    let ctx = TestCtx::new().await;
    let id = "rt-weekly-mail-20260731";
    let req: CreateRequest = serde_json::from_value(weekly_batch_json(id)).expect("題組 check-in JSON");
    ctx.handle.create_question(req, now()).await.expect("題組 check-in 建題");
    let q = ctx.handle.get_question(id).await.unwrap().unwrap();
    assert_eq!(q.source.trigger, Some(Trigger::Scheduled));
    assert_eq!(q.source.routine_kind, Some(RoutineKind::ClaudeScheduledTask));
    assert_eq!(q.source.expected_every_hours, Some(168.0));
    ctx.handle.tick(now() + Duration::minutes(12)).await.unwrap();
    let warned = ctx.handle.get_question(id).await.unwrap().unwrap().state;
    ctx.handle.tick(now() + Duration::minutes(15)).await.unwrap();
    let forced = ctx.handle.get_question(id).await.unwrap().unwrap().state;
    assert_eq!((warned, forced), (State::Warned, State::Forced), "題組 check-in 不降級");
    // 重送同一次排程只 touch（AC-RC-03）。
    let again = mcp(&ctx, "ask_create", weekly_batch_json(id)).await.unwrap();
    assert_eq!(again["dedup"], "touched_existing");

    let v = serde_json::to_value(ctx.handle.get_question(id).await.unwrap().unwrap()).unwrap();
    let h: Vec<String> = v["items"].as_array().unwrap().iter().map(|i| i["content_hash"].as_str().unwrap().to_string()).collect();
    let input: AnswerInput = serde_json::from_value(json!({"channel": "ui", "items": [
        {"item_id": "m1", "action": "approve", "text": "已修好，請重新整理頁面！", "content_hash": h[0]},
        {"item_id": "m2", "action": "skip", "content_hash": h[1]}
    ]}))
    .unwrap();
    let aid = ctx.handle.answer_question(id, input, now() + Duration::minutes(16)).await.unwrap().answer.unwrap().answer_id;
    ctx.handle
        .report_answer(id, &aid, ReportStatus::Done, "已送出 1 封、不處理 1 封", now() + Duration::minutes(20))
        .await
        .unwrap();
    let before = ctx.handle.get_question(id).await.unwrap().unwrap();

    let (h2, _db) = ctx.fresh_db_same_vault().await;
    h2.rebuild_db_at(now() + Duration::hours(1)).await.unwrap();
    h2.answers_since("qa-r5", None, now() + Duration::hours(2)).await.unwrap();
    let after = h2.get_question(id).await.unwrap().unwrap();
    assert_eq!(after.state, State::Routed);
    assert_eq!(after.source, before.source, "排程身分重建後一致");
    let (a, b) = (after.answer.unwrap(), before.answer.unwrap());
    assert_eq!(serde_json::to_value(&a.items).unwrap(), serde_json::to_value(&b.items).unwrap(), "逐項決定重建後一致");
    assert_eq!(a.report.map(|r| r.summary_md), Some("已送出 1 封、不處理 1 封".into()), "AI 回饋重建後一致");
}

// ---------------------------------------------------------------------------
// R5-4（應綠）：草稿註冊（`kunsu ask <資料夾>`／ask_register）也套用排程預設——frontmatter 帶 routine_id 就是 scheduled；
// `kunsu-` 保留前綴一律 interactive；宣告頻率 0 拒收。AC-RC-01、01b（「所有建題路徑都套用這個預設」）。
// ---------------------------------------------------------------------------

fn draft_md(id: &str, routine_id: &str, every: &str) -> String {
    format!(
        "---\nid: {id}\ntitle: 今天的日記\nquestion_line: 今天最重要的三件事？\nkind: text\ntier: t1\nsource:\n  agent: claude\n  cwd: /tmp\n  routine_id: {routine_id}\n  routine_name: 每天寫日記\n  expected_every_hours: {every}\n---\n寫下今天最重要的三件事。\n"
    )
}

#[tokio::test]
async fn r5_draft_registration_applies_routine_defaults() {
    let ctx = TestCtx::new().await;
    let home = TempDir::new().unwrap();
    ctx.handle.set_draft_paths(DraftPaths {
        drafts_root: home.path().join("drafts"),
        revisions_root: home.path().join("revisions"),
    });
    let write = |id: &str, md: String| {
        let dir = home.path().join("drafts").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("question.md"), md).unwrap();
        dir
    };
    let dir = write("rt-diary-d1", draft_md("rt-diary-d1", "diary", "24"));
    let out = ctx.handle.register_draft(&dir, RegisterOptions::default(), now()).await.unwrap();
    assert!(out.ok, "{:?}", out.issues);
    let q = ctx.handle.get_question("rt-diary-d1").await.unwrap().unwrap();
    assert_eq!(q.source.trigger, Some(Trigger::Scheduled), "草稿 check-in 要是 scheduled");

    let dir = write("kunsu-like", draft_md("kunsu-like", "kunsu-doctor", "24"));
    let out = ctx.handle.register_draft(&dir, RegisterOptions::default(), now()).await.unwrap();
    assert!(out.ok, "{:?}", out.issues);
    let q = ctx.handle.get_question("kunsu-like").await.unwrap().unwrap();
    assert_eq!(q.source.trigger, Some(Trigger::Interactive), "kunsu- 保留前綴一律 interactive");

    let dir = write("rt-diary-d0", draft_md("rt-diary-d0", "diary", "0"));
    let out = ctx.handle.register_draft(&dir, RegisterOptions::default(), now()).await;
    let refused = match &out {
        Ok(o) => !o.ok,
        Err(_) => true,
    };
    assert!(refused, "宣告頻率 0 的草稿 check-in 要拒收：{out:?}");
    assert!(ctx.handle.get_question("rt-diary-d0").await.unwrap().is_none());
}
