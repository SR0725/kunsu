//! QA 題組第 1 輪（BDD batch-review AC-BR-01..08；PRD §25；CONTRACT §14）：獨立 QA 的重現測試。
//!
//! 每個測試的斷言寫的是規格期望；失敗訊息寫實際行為。標「應綠」的是確認現行實作成立的對照組。
//! 不連真實 server（4777）、不啟動真實 Claude／Codex：reactor 用暫存資料夾裡的假 `claude` 腳本。

mod common;

use common::*;
use kunsu_core::types::*;
use kunsu_server::*;
use serde_json::{json, Value};
use tempfile::TempDir;

/// 第 1、2 項可編輯、第 3 項 AI 建議不處理、第 4 項只核可（同 tests/batch_review.rs）。
fn items() -> Value {
    json!([
        {"id": "i1", "title": "陳同學", "subtitle": "第 5 章打不開", "context_md": "老師好，第 5 章影片一直轉圈圈。",
         "proposal": "陳同學你好，已經修好了，請重新整理頁面。", "editable": true},
        {"id": "i2", "title": "某品牌窗口（品牌合作）", "context_md": "想邀請您開箱冷萃新品。",
         "proposal": "您好，謝謝邀請！目前內容主軸是 AI 與開發。", "editable": true},
        // 2026-10-09 作者：AI 建議不處理的可編輯項目給空草稿，不准「（不回）」這類佔位文字（skill 範例同步改）。
        {"id": "i3", "title": "某軟體廠商", "context_md": "升級企業方案享 8 折。",
         "proposal": "", "editable": true, "default": "skip", "default_reason": "促銷信"},
        {"id": "i4", "title": "張同學退款", "subtitle": "Sample Pro · NT$4,800", "context_md": "購買 3 天、進度 8%。",
         "proposal": "符合退費規則 → 建議全額退款 NT$4,800", "editable": false}
    ])
}

fn batch_json(id: &str) -> Value {
    let mut v = serde_json::to_value(base_req(id)).unwrap();
    let o = v.as_object_mut().unwrap();
    o.insert("title".into(), json!("今天 4 件事的回覆"));
    o.insert("question_line".into(), json!("今天 4 件事的回覆"));
    o.insert("kind".into(), json!("batch"));
    o.insert("gate".into(), Value::Null);
    o.insert("options".into(), json!([]));
    o.insert("items".into(), items());
    v
}

fn batch_req(id: &str) -> CreateRequest {
    serde_json::from_value(batch_json(id)).expect("題組 JSON 要能解析")
}

async fn hashes(ctx: &TestCtx, id: &str) -> Vec<String> {
    let q = ctx.handle.get_question(id).await.unwrap().unwrap();
    q.items.iter().map(|i| i.content_hash.clone().expect("每項都有雜湊")).collect()
}

fn ui_items(items: Value, instruction: Option<&str>) -> AnswerInput {
    serde_json::from_value(json!({"channel": "ui", "items": items, "free_text": instruction})).unwrap()
}

// ---------------------------------------------------------------------------
// AC-BR-05：題組只能在面板逐項作答（CONTRACT §14「CLI／Discord 載荷沒有逐項決定，題組只能在面板作答」）
// ---------------------------------------------------------------------------

/// 期望：題組的答案一定有逐項決定；沒有面板逐項作答的通道都不能結案題組。
/// 實際（重現）：題組帶了 `kongko_ref` 時，Kongko 發佈事件（POST /events/kongko）走 `kongko_event`，
/// 直接寫一個 `channel=kongko`、`items=None`、`free_text="Kongko 發佈確認…"` 的答案並結案——繞過逐項驗證，
/// 回原對話的訊息也變回一般題的格式（沒有逐項決定、沒有「一字不改」）。
#[tokio::test]
async fn qa_br05_kongko_publish_event_cannot_close_a_batch() {
    let ctx = TestCtx::new().await;
    let mut v = batch_json("mail-kk");
    v["kongko_ref"] = json!("batch-kk-1");
    let req: CreateRequest = serde_json::from_value(v).unwrap();
    // 若建題時就拒收「題組＋kongko_ref」也算符合規格（題組不會被 Kongko 結案）。
    if ctx.handle.create_question(req, now()).await.is_err() {
        return;
    }
    let ev: KongkoPublication = serde_json::from_value(json!({
        "type": "ad_publication_confirmed",
        "publicationId": "pub-kk-1",
        "batchId": "batch-kk-1",
        "product": "nova",
        "ads": [],
        "confirmedAt": "2026-10-09T01:00:00Z"
    }))
    .unwrap();
    let out = ctx.handle.kongko_event(ev, now()).await.unwrap();
    let q = ctx.handle.get_question("mail-kk").await.unwrap().unwrap();
    assert!(
        !q.state.is_terminal() && q.answer.is_none(),
        "題組不該被 Kongko 發佈事件結案；實際：closed_question_id={:?}，state={:?}，answer.channel={:?}，answer.items={:?}，free_text={:?}",
        out.closed_question_id,
        q.state,
        q.answer.as_ref().map(|a| a.channel),
        q.answer.as_ref().map(|a| a.items.is_some()),
        q.answer.as_ref().and_then(|a| a.free_text.clone()),
    );
}

/// 應綠：HTTP 的 CLI 與 Discord 作答端點都不能結案題組，錯誤說明要到面板逐項作答。
#[tokio::test]
async fn qa_br05_cli_and_discord_http_cannot_answer_a_batch() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();
    ctx.handle.create_question(batch_req("mail-http"), now()).await.unwrap();

    for (path, token, body) in [
        ("/cli/answer", TEST_ADMIN_TOKEN, json!({"question_id": "mail-http", "free_text": "全部照草稿寄"})),
        ("/cli/answer", TEST_ADMIN_TOKEN, json!({"question_id": "mail-http", "choice_id": "i1"})),
        (
            "/ingest/discord-answer",
            TEST_TOKEN,
            json!({"question_id": "mail-http", "free_text": "都可以", "discord_message_id": "m1"}),
        ),
    ] {
        let resp = client.post(format!("{base}{path}")).bearer_auth(token).json(&body).send().await.unwrap();
        let status = resp.status();
        let body: Value = resp.json().await.unwrap();
        assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "{path}：{body}");
        assert!(body["error"].as_str().unwrap_or_default().contains("題組"), "{path} 的錯誤要說明是題組：{body}");
    }
    let q = ctx.handle.get_question("mail-http").await.unwrap().unwrap();
    assert!(!q.state.is_terminal() && q.answer.is_none(), "題目維持未答：{:?}", q.state);
}

/// 應綠：CONTRACT §14「`ask_answers_since` 原樣帶出逐項決定」（既有測試只驗了 ask_get／ask_list／ask_wait）。
#[tokio::test]
async fn qa_br05_answers_since_carries_item_decisions() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(batch_req("mail-since"), now()).await.unwrap();
    let h = hashes(&ctx, "mail-since").await;
    let text = "  陳同學你好，修好了！\n\n有問題再回信給我。  ";
    let decisions = json!([
        {"item_id": "i1", "action": "approve", "text": text, "content_hash": h[0]},
        {"item_id": "i2", "action": "revise", "note": "先感謝再婉拒", "content_hash": h[1]},
        {"item_id": "i3", "action": "skip", "content_hash": h[2]},
        {"item_id": "i4", "action": "approve", "content_hash": h[3]}
    ]);
    ctx.handle.answer_question("mail-since", ui_items(decisions, None), now()).await.unwrap();
    let batch = ctx.handle.answers_since("qa-batch", None, now()).await.unwrap();
    let v = serde_json::to_value(&batch).unwrap();
    let ev = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e.to_string().contains("mail-since"))
        .unwrap_or_else(|| panic!("ask_answers_since 要有這題：{v}"))
        .clone();
    let found = ev.to_string();
    assert!(found.contains("先感謝再婉拒") && found.contains("\"revise\""), "逐項決定要帶出：{ev}");
    // 所見即所送：前後空白與空行都原樣保留。
    let stored = serde_json::to_string(text).unwrap();
    assert!(found.contains(&stored), "採用的文字要原樣（含前後空白與空行）：{ev}");
}

// ---------------------------------------------------------------------------
// AC-BR-06：第 2 輪題組的 id 必須是合法題目 id
// ---------------------------------------------------------------------------

/// 期望：回原對話的訊息要 agent 用 `next_round_id` 建下一輪題組，那個 id 一定要能建題。
/// 實際（重現）：題目 id 上限 120 字；原 id 118–120 字時 `x-r2` 變成 121–123 字，agent 照訊息建題會被
/// `id_invalid_format` 拒收，第 2 輪題組回不來。
/// 2026-10-09 作者核准的修法：題組 id 在建題時就留出輪次後綴的空間（不含 `-rN` 最多 116 字）。所以原本「120 字的題組
/// id 合法」這個前提改成「建題時就拒收並說明原因」，再用允許的最長 id（116 字）驗證第 2 輪 id 一定能建題。
#[tokio::test]
async fn qa_br06_next_round_id_of_a_long_id_is_still_creatable() {
    let ctx = TestCtx::new().await;
    let too_long = format!("mail-{}", "x".repeat(115)); // 120 字：一般題合法，題組沒有輪次後綴的空間
    assert!(kunsu_core::validate::valid_id(&too_long));
    let err = ctx.handle.create_question(batch_req(&too_long), now()).await.expect_err("120 字的題組 id 要在建題時就拒收");
    let msg = err.to_string();
    assert!(msg.contains("-r2") && msg.contains("116"), "拒收訊息要說明是留給輪次後綴：{msg}");

    let long_id = format!("mail-{}", "x".repeat(111)); // 116 字，題組允許的最長 id
    ctx.handle.create_question(batch_req(&long_id), now()).await.expect("116 字的題組 id 合法");
    let h = hashes(&ctx, &long_id).await;
    let decisions = json!([
        {"item_id": "i1", "action": "revise", "note": "再短一點", "content_hash": h[0]},
        {"item_id": "i2", "action": "skip", "content_hash": h[1]},
        {"item_id": "i3", "action": "skip", "content_hash": h[2]},
        {"item_id": "i4", "action": "skip", "content_hash": h[3]}
    ]);
    let q = ctx.handle.answer_question(&long_id, ui_items(decisions, None), now()).await.unwrap();
    let prompt = kunsu_server::reactor::continuation_prompt(&q, None);
    let next = kunsu_core::batch::next_round_id(&long_id);
    assert!(prompt.contains(&next), "訊息要指定下一輪 id：\n{prompt}");
    let created = ctx.handle.create_question(batch_req(&next), now()).await;
    assert!(
        created.is_ok(),
        "照訊息用 {} 字的 `{next}` 建第 2 輪題組要成功；實際被拒：{:?}",
        next.chars().count(),
        created.err().map(|e| e.to_string())
    );
}

// ---------------------------------------------------------------------------
// AC-BR-06：開新對話的 prompt 必須交得出去（命令列參數長度）
// ---------------------------------------------------------------------------

/// 期望：任何通過建題驗證、使用者照規則作答的題組，答案都交得到 agent（原對話或新對話）。
/// 實際（重現）：開新對話把「題目 JSON（含每項 proposal 與答案逐項文字）＋逐項決定段落（再一次最終文字）」整包放進
/// `claude -p <prompt>` 的單一命令列參數（最終文字在 JSON 與段落裡各出現一次）。20 項 × 5,000 字草稿＋作答文字
/// （建題與作答的上限內）、每封來信 4,000 字（context_md 沒有上限）時，參數約 1.15 MB，超過 macOS ARG_MAX
/// （1,048,576 bytes，含環境變數），spawn 失敗「Argument list too long」，重試 3 次後死信，答案交不出去。
/// （來信很短時約 913 KB、來信 2,000 字時約 1.03 MB，還擠得進去；回原對話的 continuation prompt 只放一次文字，約 300 KB。）
#[tokio::test]
async fn qa_br06_fresh_prompt_for_a_max_size_batch_is_deliverable() {
    let ctx = TestCtx::new().await;
    let bin = TempDir::new().unwrap();
    let claude = write_script(bin.path(), "claude.sh", "#!/bin/sh\necho 'EXEC_OK: 已處理'\n");

    let mut v = batch_json("mail-max");
    let draft = "你".repeat(5000); // 可編輯草稿上限
    let edited = "好".repeat(5000); // 作答文字上限
    let letter = "來".repeat(4000); // 一封 4,000 字的來信（context_md 沒有上限）
    v["items"] = Value::Array(
        (1..=20)
            .map(|n| {
                json!({"id": format!("i{n}"), "title": format!("第 {n} 封"), "context_md": letter,
                       "proposal": draft, "editable": true})
            })
            .collect(),
    );
    v["wake_on_answer"] = json!({"mode": "fresh"});
    ctx.handle.create_question(serde_json::from_value(v).unwrap(), now()).await.expect("20 項 × 5,000 字合法");
    let h = hashes(&ctx, "mail-max").await;
    let decisions: Vec<Value> = h
        .iter()
        .enumerate()
        .map(|(i, hash)| json!({"item_id": format!("i{}", i + 1), "action": "approve", "text": edited, "content_hash": hash}))
        .collect();
    ctx.handle.answer_question("mail-max", ui_items(Value::Array(decisions), None), now()).await.expect("合法答案");

    let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
    cfg.claude_bin = Some(claude);
    cfg.retry_base_ms = 1;
    let reactor = Reactor::new(ctx.handle.clone(), cfg, std::sync::Arc::new(StaticKeyProvider(test_key())));
    reactor.process_pending().await.unwrap();
    let planned = reactor.planned();
    let fresh = planned.iter().find(|c| c.kind == "wake_claude").expect("開新對話的命令");
    let arg_bytes: usize = fresh.args.iter().map(|a| a.len() + 1).sum();
    let q = ctx.handle.get_question("mail-max").await.unwrap().unwrap();
    let delivery = q.answer.as_ref().and_then(|a| a.delivery.clone());
    let delivered = delivery.as_ref().is_some_and(|d| d.state == kunsu_core::continuity::DeliveryState::Fresh);
    assert!(
        delivered,
        "合法的最大題組答案要能交給新對話；實際命令列參數共 {arg_bytes} bytes（macOS ARG_MAX 1,048,576 含環境變數），交付狀態 {:?}，執行結果 {:?}",
        delivery.as_ref().map(|d| d.state),
        q.answer.as_ref().and_then(|a| a.execution_result.clone()),
    );
}
