//! QA 第 6 輪（排程督導，BDD routine-check-in）：main 新合進的「退回」（使用者按「這題不該問我」→ 題目寫 `rejected {at, reason}`）
//! 與第 8 輪「收割標 acked_at 進帳本」在排程 check-in 上的伺服器面——收割、重建、再收割前後督導資料一致（AC-RC-09），
//! 逾時依預設自動選定的不會因收割或重建變成「已回應」。等待中的 agent 讀不到退回原因的問題見 `qa_round9_continuity.rs` R9-6。
//! 督導頁顯示面的檢查在 `ui/src/routines/qa_round6_routine.test.ts`（vitest）。
//!
//! 斷言寫規格期望；失敗訊息寫實際行為。標「應綠」的是確認行為成立的對照組。時間一律注入。

mod common;

use chrono::Duration;
use common::*;
use kunsu_core::continuity::{DeliveryState, HostKind, ReportStatus, Trigger};
use kunsu_core::types::*;
use kunsu_server::mcp::{handle_rpc, McpReply};
use kunsu_server::*;
use serde_json::{json, Value};

async fn mcp(handle: &ServerHandle, tool: &str, args: Value) -> Result<Value, String> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}});
    match handle_rpc(handle, body).await {
        McpReply::Json(v) => match v.get("error") {
            Some(e) => Err(e["message"].as_str().unwrap_or_default().to_string()),
            None => Ok(v["result"]["structuredContent"].clone()),
        },
        McpReply::Accepted => Err("accepted".into()),
    }
}

/// 每天寫日記的 check-in（文字題；宣告名稱與頻率）。
fn diary(id: &str) -> CreateRequest {
    let mut req = base_req(id);
    req.kind = Kind::Text;
    req.options = vec![];
    req.gate = None;
    req.title = "今天的日記".into();
    req.question_line = "今天最重要的三件事？".into();
    req.source.routine_id = Some("diary".into());
    req.source.routine_name = Some("每天寫日記".into());
    req.source.expected_every_hours = Some(24.0);
    req
}

fn state_and_rejection(q: &Question) -> (State, Option<String>, bool) {
    (q.state, q.rejected.as_ref().and_then(|r| r.reason.clone()), q.rejected.is_some())
}

// ---------------------------------------------------------------------------
// R6-1（應綠）：使用者退回一次 check-in、agent 取消另一次——收割、重建、再收割之後，退回的仍是 cancelled＋rejected（帶原因）、
// agent 取消的仍沒有 rejected；收割事件與 `ask_list` 都帶著 rejected。AC-RC-05、06、09；CONTRACT §9。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r6_rejected_check_in_stays_rejected_through_harvest_and_rebuild() {
    let ctx = TestCtx::new().await;
    let t = now();
    ctx.handle.create_question(diary("rt-diary-20260730"), t - Duration::days(1)).await.unwrap();
    ctx.handle.create_question(diary("rt-diary-20260731"), t).await.unwrap();
    ctx.handle.reject_question("rt-diary-20260731", Some("今天休假".into()), t + Duration::minutes(1)).await.unwrap();
    mcp(&ctx.handle, "ask_cancel", json!({"question_id": "rt-diary-20260730"})).await.unwrap();

    let first = ctx.handle.answers_since("routine-harvest", None, t + Duration::minutes(2)).await.unwrap();
    let ev = |b: &AnswerBatch, id: &str| b.events.iter().find(|e| e.question.id == id).map(|e| state_and_rejection(&e.question));
    assert_eq!(ev(&first, "rt-diary-20260731"), Some((State::Cancelled, Some("今天休假".into()), true)), "收割事件帶 rejected");
    assert_eq!(ev(&first, "rt-diary-20260730"), Some((State::Cancelled, None, false)));

    let (h2, _db) = ctx.fresh_db_same_vault().await;
    h2.rebuild_db_at(t + Duration::hours(1)).await.unwrap();
    let again = h2.answers_since("routine-harvest", None, t + Duration::hours(2)).await.unwrap();
    for (id, want) in [
        ("rt-diary-20260731", (State::Cancelled, Some("今天休假".to_string()), true)),
        ("rt-diary-20260730", (State::Cancelled, None, false)),
    ] {
        let q = h2.get_question(id).await.unwrap().unwrap();
        assert_eq!(state_and_rejection(&q), want, "{id} 重建＋再收割後");
        assert_eq!(ev(&again, id), Some(want.clone()), "{id} 重建後的收割事件");
    }
    let listed = mcp(&h2, "ask_list", json!({"include_terminal": true})).await.unwrap();
    let rejected: Vec<&Value> = listed["questions"].as_array().unwrap().iter().filter(|q| q.get("rejected").is_some()).collect();
    assert_eq!(rejected.len(), 1, "ask_list 只有使用者退回的那題帶 rejected：{listed}");
}

// ---------------------------------------------------------------------------
// R6-2（應綠）：回到原對話的 check-in——Lane 1 收割先到（交付還沒到原對話，不標 acked、不寫 ack 行），之後原對話自己回報
// （picked_up）。重建前後狀態、acked_at、AI 回饋與交付一致，再收割也不變。AC-RC-04、09；AC-CC-16。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r6_continued_check_in_harvested_before_report_survives_rebuild() {
    let ctx = TestCtx::new().await;
    let id = "rt-diary-20260801";
    let t = now();
    let mut req = diary(id);
    req.source.host = Some(HostKind::ClaudeCli);
    req.source.conversation_id = Some("conv-r6-diary".into());
    ctx.handle.create_question(req, t).await.unwrap();
    let input = AnswerInput { free_text: Some("1. 寫完 PRD 2. 健身 3. 早睡".into()), choice_id: None, ..ui_answer("x") };
    let aid = ctx.handle.answer_question(id, input, t + Duration::minutes(3)).await.unwrap().answer.unwrap().answer_id;
    ctx.handle.answers_since("routine-harvest", None, t + Duration::minutes(4)).await.unwrap();
    let harvested = ctx.handle.get_question(id).await.unwrap().unwrap();
    assert!(harvested.answer.as_ref().unwrap().acked_at.is_none(), "收割不算接手（AC-CC-16）");
    ctx.handle.report_answer(id, &aid, ReportStatus::Done, "已寫入 vault 日記，明天提醒早睡", t + Duration::minutes(10)).await.unwrap();
    let before = ctx.handle.get_question(id).await.unwrap().unwrap();
    assert_eq!(before.answer.as_ref().and_then(|a| a.delivery.as_ref()).map(|d| d.state), Some(DeliveryState::PickedUp));

    let (h2, _db) = ctx.fresh_db_same_vault().await;
    h2.rebuild_db_at(t + Duration::hours(1)).await.unwrap();
    h2.answers_since("routine-harvest", None, t + Duration::hours(2)).await.unwrap();
    let after = h2.get_question(id).await.unwrap().unwrap();
    let (a, b) = (after.answer.clone().unwrap(), before.answer.clone().unwrap());
    assert!(matches!(after.state, State::Answered | State::Routed), "重建後仍是已回應：{:?}", after.state);
    assert_eq!((a.acked_at, a.report.map(|r| r.summary_md), a.delivery.map(|d| d.state)), (b.acked_at, b.report.map(|r| r.summary_md), b.delivery.map(|d| d.state)));
    assert_eq!(after.source.trigger, Some(Trigger::Scheduled));
}

// ---------------------------------------------------------------------------
// R6-3（應綠）：逾時由 Kunsu 依預設自動選定的 check-in（channel=auto_policy、state=expired）——收割會標 acked_at 並寫 ack 行，
// 但狀態仍是 expired（不是 routed），重建後一樣：督導頁照舊算「未回應」，不會因為收割或重建變成「已回應」。AC-RC-06、09。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r6_auto_default_check_in_stays_missed_through_harvest_and_rebuild() {
    let ctx = TestCtx::new().await;
    let id = "rt-goals-20260731";
    let t = now();
    let mut req = diary(id);
    req.kind = Kind::Choice;
    req.options = vec![
        Opt { id: "y".into(), label: "有做到".into(), detail: None, recommended: None },
        Opt { id: "n".into(), label: "沒做到".into(), detail: None, recommended: None },
    ];
    req.default_action = DefaultAction { on_expire: OnExpire::PickOption, option_id: Some("y".into()) };
    req.deadline_at = Some(t + Duration::minutes(30));
    ctx.handle.create_question(req, t).await.unwrap();
    ctx.handle.tick(t + Duration::minutes(31)).await.unwrap();
    let expired = ctx.handle.get_question(id).await.unwrap().unwrap();
    assert_eq!(expired.state, State::Expired, "前提：逾時");
    assert_eq!(expired.answer.as_ref().map(|a| a.channel), Some(Channel::AutoPolicy), "前提：依預設自動選定");

    ctx.handle.answers_since("routine-harvest", None, t + Duration::minutes(40)).await.unwrap();
    let harvested = ctx.handle.get_question(id).await.unwrap().unwrap();
    assert_eq!(harvested.state, State::Expired, "收割後仍是 expired");
    let (h2, _db) = ctx.fresh_db_same_vault().await;
    h2.rebuild_db_at(t + Duration::hours(1)).await.unwrap();
    let rebuilt = h2.get_question(id).await.unwrap().unwrap();
    assert_eq!(
        (rebuilt.state, rebuilt.answer.as_ref().map(|a| a.channel), rebuilt.answer.as_ref().and_then(|a| a.acked_at).is_some()),
        (State::Expired, Some(Channel::AutoPolicy), true),
        "重建後仍是逾時自動選定、已收割"
    );
}

// ---------------------------------------------------------------------------
// R6-4（應綠）：退回原因只存去掉前後空白的文字；全空白等於沒寫原因（督導頁顯示「你退回了這題」）；過長的原因截在 2,000 字。
// AC-RC-06。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r6_rejection_reason_is_trimmed_and_bounded() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(diary("rt-diary-blank"), now()).await.unwrap();
    ctx.handle.create_question(diary("rt-diary-long"), now()).await.unwrap();
    let blank = ctx.handle.reject_question("rt-diary-blank", Some("   \n ".into()), now()).await.unwrap();
    assert!(blank.rejected.is_some() && blank.rejected.unwrap().reason.is_none(), "全空白＝沒寫原因");
    let long = ctx.handle.reject_question("rt-diary-long", Some(format!("  {}  ", "休".repeat(2500))), now()).await.unwrap();
    assert_eq!(long.rejected.unwrap().reason.map(|r| r.chars().count()), Some(2000));
    // 已退回的題不能再退回（終態）。
    assert!(ctx.handle.reject_question("rt-diary-long", Some("再一次".into()), now()).await.is_err());
}
