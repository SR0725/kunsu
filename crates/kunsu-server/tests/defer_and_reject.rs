//! 瀏海說出延後原因（BDD edge-notch 場景 10c）與督導頁「退回」（BDD routine-check-in 場景 5／6／9）。

mod common;

use chrono::Duration;
use common::*;
use kunsu_core::guards::DeferReason;
use kunsu_server::mcp::{handle_rpc, McpReply};
use kunsu_server::GuardSnapshot;
use serde_json::json;

/// 延後原因與狀態機用同一份 guard 快照：probe 的分享螢幕、Kunsu 勿擾（帶結束時刻）都列出來，依顯示優先序。
#[tokio::test]
async fn defer_reasons_reflect_probe_and_dnd() {
    let ctx = TestCtx::new().await;
    assert!(ctx.handle.defer_reasons(now()).await.unwrap().reasons.is_empty(), "沒有 guard 命中時是空的");
    ctx.guards.set(GuardSnapshot { screen_shared: true, mic_on: true, ..Default::default() });
    ctx.handle.set_dnd(30, "test", now()).await.unwrap();
    let r = ctx.handle.defer_reasons(now()).await.unwrap();
    assert_eq!(r.reasons, vec![DeferReason::ScreenShared, DeferReason::MicOn, DeferReason::Dnd]);
    assert_eq!(r.dnd_until, Some(now() + Duration::minutes(30)));
}

/// 使用者按「這題不該問我」：題目記下 rejected {at, reason}，終態帳本行帶著它，重建後仍分得出「退回」；agent 讀得到。
#[tokio::test]
async fn rejected_snapshot_survives_rebuild_and_reaches_the_agent() {
    let ctx = TestCtx::new().await;
    let id = "rt-diary-20261009";
    ctx.handle.create_question(base_req(id), now()).await.unwrap();
    ctx.handle.create_question(base_req("rt-diary-cancelled"), now()).await.unwrap();
    let q = ctx.handle.reject_question(id, Some("今天休假".into()), now() + Duration::minutes(1)).await.unwrap();
    let rej = q.rejected.clone().expect("退回要記在題目上");
    assert_eq!(rej.reason.as_deref(), Some("今天休假"));
    assert_eq!(rej.at, now() + Duration::minutes(1));
    // agent 取消的題沒有 rejected。
    let c = ctx.handle.cancel_question("rt-diary-cancelled", now()).await.unwrap();
    assert!(c.rejected.is_none());

    let rows = read_ledger_rows(&ctx, now());
    let row = rows.iter().find(|r| r.question.id == id).expect("終態帳本行");
    assert!(row.question.rejected.is_some(), "帳本快照帶著 rejected");

    let (h2, _db) = ctx.fresh_db_same_vault().await;
    h2.rebuild_db().await.unwrap();
    let restored = h2.get_question(id).await.unwrap().unwrap();
    assert_eq!(restored.rejected.as_ref().and_then(|r| r.reason.as_deref()), Some("今天休假"), "重建後仍是退回");

    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "ask_get", "arguments": {"question_id": id}}});
    let McpReply::Json(v) = handle_rpc(&ctx.handle, body).await else { panic!("要回 JSON") };
    assert_eq!(v["result"]["structuredContent"]["question"]["rejected"]["reason"], "今天休假", "{v}");
}
