//! 決策面板改版的 server 面（BDD decision-surface AC-DS-06、AC-DS-08；evidence-canvas AC-EC-12）。

mod common;

use common::*;
use kunsu_server::AnswerInput;

fn choice_with_note(choice: &str, note: &str) -> AnswerInput {
    AnswerInput { free_text: Some(note.into()), ..ui_answer(choice) }
}

/// AC-DS-06：選項＋補充說明同時保存 choice_id 與 free_text。
#[tokio::test]
async fn answer_keeps_choice_and_note_together() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("ds-note"), now()).await.unwrap();
    let q = ctx
        .handle
        .answer_question("ds-note", choice_with_note("a", "加碼，但先 15%"), now())
        .await
        .unwrap();
    let a = q.answer.unwrap();
    assert_eq!(a.choice_id.as_deref(), Some("a"));
    assert_eq!(a.free_text.as_deref(), Some("加碼，但先 15%"));
}

/// 空白補充不算文字回答；不存在的選項仍拒收。
#[tokio::test]
async fn blank_note_is_dropped_and_unknown_choice_rejected() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("ds-blank"), now()).await.unwrap();
    let q = ctx
        .handle
        .answer_question("ds-blank", choice_with_note("b", "   "), now())
        .await
        .unwrap();
    let a = q.answer.unwrap();
    assert_eq!(a.choice_id.as_deref(), Some("b"));
    assert_eq!(a.free_text, None, "空白補充不保存");

    ctx.handle.create_question(base_req("ds-bad"), now()).await.unwrap();
    let err = ctx
        .handle
        .answer_question("ds-bad", choice_with_note("zz", "x"), now())
        .await;
    assert!(err.is_err(), "choice 必須存在於 options");
}

/// AC-DS-06：錢題的補充說明不改所見即所簽參數。
#[tokio::test]
async fn money_note_never_changes_params() {
    let ctx = TestCtx::new().await;
    let req = money_t0_req("ds-money");
    let params = req.params_line.clone();
    let prepare = req.money.as_ref().unwrap().executor_prepare_cmd.clone();
    ctx.handle.create_question(req, now()).await.unwrap();
    let q = ctx
        .handle
        .answer_question("ds-money", choice_with_note("a", "改成 NT$900 好了"), now())
        .await
        .unwrap();
    assert_eq!(q.params_line, params, "補充文字不是 edited_params");
    assert_eq!(q.money.unwrap().executor_prepare_cmd, prepare);
    assert_eq!(q.answer.unwrap().free_text.as_deref(), Some("改成 NT$900 好了"));
}

/// AC-DS-08／AC-EC-12：ask_create 可帶 why_md，題目保存；舊題沒有也合法。
#[tokio::test]
async fn why_md_is_stored_and_optional() {
    let ctx = TestCtx::new().await;
    let mut req = base_req("ds-why");
    req.why_md = Some("每日復盤發現停投 18 天。\n\n1. 09:30 讀取數據\n2. 現在 等你決定".into());
    ctx.handle.create_question(req, now()).await.unwrap();
    let q = ctx.handle.get_question("ds-why").await.unwrap().unwrap();
    assert!(q.why_md.unwrap().contains("現在 等你決定"));

    ctx.handle.create_question(base_req("ds-old"), now()).await.unwrap();
    assert_eq!(ctx.handle.get_question("ds-old").await.unwrap().unwrap().why_md, None);
    let schema = kunsu_server::mcp::tool_specs()
        .into_iter()
        .find(|s| s["name"] == "ask_create")
        .unwrap();
    assert!(schema["inputSchema"]["properties"]["why_md"].is_object(), "MCP schema 要公開 why_md");
}

fn confirm_req(id: &str) -> kunsu_core::types::CreateRequest {
    let mut req = base_req(id);
    req.kind = kunsu_core::types::Kind::Confirm;
    req.options = vec![];
    req
}

/// AC-DS-15：確認題（confirm、沒有選項）面板送 `choice_id = "confirm"`，server 必須接受並和補充一起保存；
/// 交給發問 AI 的訊息寫明使用者的選擇是「確認」。
#[tokio::test]
async fn confirm_only_question_accepts_confirm() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(confirm_req("ds-confirm"), now()).await.unwrap();
    let q = ctx
        .handle
        .answer_question("ds-confirm", choice_with_note("confirm", "照排程上線"), now())
        .await
        .expect("確認題要能用 confirm 作答");
    let a = q.answer.as_ref().unwrap();
    assert_eq!(a.choice_id.as_deref(), Some("confirm"));
    assert_eq!(a.free_text.as_deref(), Some("照排程上線"));
    let prompt = kunsu_server::reactor::continuation_prompt(&q, None);
    assert!(prompt.contains("使用者的選擇：確認"), "交給 AI 的訊息要寫「確認」：{prompt}");
}

/// AC-DS-15：`confirm` 不是萬用選項——一般選擇題送 confirm 仍拒收，壞載荷不終結題目。
#[tokio::test]
async fn confirm_is_not_a_wildcard_choice() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("ds-not-confirm"), now()).await.unwrap();
    let err = ctx.handle.answer_question("ds-not-confirm", ui_answer("confirm"), now()).await;
    assert!(err.is_err(), "選擇題的 choice 必須存在於 options");
    let q = ctx.handle.get_question("ds-not-confirm").await.unwrap().unwrap();
    assert!(!q.state.is_terminal(), "壞載荷不得終結題目");
}

/// AC-DS-15：錢類確認題按「確認」不觸發錢鏈（錢鏈只認黃燈錢題的執行型選項）。
#[tokio::test]
async fn money_confirm_never_runs_the_money_chain() {
    // 2026-10-09（QA continuity 第 8 輪）：動錢的純確認題按「確認」永遠不會跑錢鏈，會讓使用者以為批准了——建題時就拒收。
    let ctx = TestCtx::new().await;
    let mut req = money_t0_req("ds-money-confirm");
    req.kind = kunsu_core::types::Kind::Confirm;
    req.options = vec![];
    let err = ctx.handle.create_question(req, now()).await.expect_err("動錢的純確認題要被拒收");
    assert!(err.to_string().contains("執行型選項"), "{err}");
    // 動錢的選擇題沒有執行型選項（recommended）也拒收：沒有任何選項會跑錢鏈。
    let mut req = money_t0_req("ds-money-no-exec");
    for o in &mut req.options {
        o.recommended = None;
    }
    let err = ctx.handle.create_question(req, now()).await.expect_err("沒有執行型選項的錢題要被拒收");
    assert!(err.to_string().contains("執行型選項"), "{err}");
}
