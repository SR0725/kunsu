//! 題組（PRD §25；BDD batch-review AC-BR-01、05、06；CONTRACT §14）：建題驗證、逐項作答驗證、
//! 版本與每項雜湊綁定、帳本與 agent 讀取面、草稿註冊、回到原對話的訊息。
//!
//! 請求與答案都以 JSON 組出來（agent／UI 的實際 wire format）。

mod common;

use std::path::PathBuf;

use common::*;
use kunsu_core::types::*;
use kunsu_server::mcp::{handle_rpc, McpReply};
use kunsu_server::*;
use serde_json::{json, Value};
use tempfile::TempDir;

/// 第 1、2 項可編輯、第 3 項 AI 建議不處理、第 4 項只核可。
fn items() -> Value {
    json!([
        {"id": "i1", "title": "陳同學", "subtitle": "第 5 章打不開", "context_md": "老師好，第 5 章影片一直轉圈圈。",
         "proposal": "陳同學你好，已經修好了，請重新整理頁面。", "editable": true},
        {"id": "i2", "title": "某品牌窗口（品牌合作）", "context_md": "想邀請您開箱冷萃新品。",
         "proposal": "您好，謝謝邀請！目前內容主軸是 AI 與開發。", "editable": true},
        // 2026-10-09 作者：AI 建議不處理的可編輯項目給空草稿（不准「（不回）」這類佔位文字）。
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

/// 面板作答：每項帶面板顯示時的雜湊。
fn answer(items: Value, instruction: Option<&str>) -> AnswerInput {
    serde_json::from_value(json!({"channel": "ui", "items": items, "free_text": instruction})).unwrap()
}

/// 目前每項的雜湊（面板顯示的版本）。
async fn hashes(ctx: &TestCtx, id: &str) -> Vec<String> {
    let q = ctx.handle.get_question(id).await.unwrap().unwrap();
    let v = serde_json::to_value(&q).unwrap();
    v["items"]
        .as_array()
        .expect("題組要有 items")
        .iter()
        .map(|i| i["content_hash"].as_str().expect("每項都有雜湊").to_string())
        .collect()
}

/// 一份合法答案：i1 改過、i2 原樣採用、i3 不處理、i4 照建議。
async fn good_decisions(ctx: &TestCtx, id: &str) -> Value {
    let h = hashes(ctx, id).await;
    json!([
        {"item_id": "i1", "action": "approve", "text": "陳同學你好，修好了！\n有問題再回信給我。", "content_hash": h[0]},
        {"item_id": "i2", "action": "approve", "text": "您好，謝謝邀請！目前內容主軸是 AI 與開發。", "content_hash": h[1]},
        {"item_id": "i3", "action": "skip", "content_hash": h[2]},
        {"item_id": "i4", "action": "approve", "content_hash": h[3]}
    ])
}

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

fn reject_code(err: ServerError) -> String {
    match err {
        ServerError::Reject(r) => r.code().to_string(),
        other => panic!("應該是建題驗證拒收：{other}"),
    }
}

async fn still_open(ctx: &TestCtx, id: &str) {
    let q = ctx.handle.get_question(id).await.unwrap().unwrap();
    assert!(!q.state.is_terminal(), "壞答案不得終結題目：{:?}", q.state);
    assert!(q.answer.is_none());
}

// ---------------------------------------------------------------------------
// AC-BR-01：建題驗證
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ask_create_accepts_a_batch_and_points_at_the_bad_item() {
    let ctx = TestCtx::new().await;
    let out = mcp(&ctx, "ask_create", batch_json("mail-ok")).await.expect("合法題組建題成功");
    assert_eq!(out["dedup"], "created");
    let q = ctx.handle.get_question("mail-ok").await.unwrap().unwrap();
    assert_eq!(q.kind, serde_json::from_value::<Kind>(json!("batch")).unwrap());
    assert_eq!(hashes(&ctx, "mail-ok").await.len(), 4);

    // 第 3 項是 AI 建議不處理的可編輯項目，本來就可以是空草稿（2026-10-09 作者）；預設採用的第 2 項不行。
    let mut bad = batch_json("mail-bad");
    bad["items"][1]["proposal"] = json!("");
    let err = mcp(&ctx, "ask_create", bad).await.expect_err("建議空白要拒收");
    assert!(err.contains("第 2 項") && err.contains("proposal"), "{err}");
    assert!(ctx.handle.get_question("mail-bad").await.unwrap().is_none(), "拒收不建題");

    let mut money = batch_req("mail-money");
    money.money = Some(Money { is_money_op: true, amount_twd: Some(4800), entity_id: None, executor_prepare_cmd: None });
    assert_eq!(reject_code(ctx.handle.create_question(money, now()).await.unwrap_err()), "batch_money_forbidden");
}

/// JSON 建題的原文沿用 Markdown 安全規則；JSON 題組不能引用圖片（要附圖用草稿註冊）。
#[tokio::test]
async fn json_batch_context_md_goes_through_the_markdown_checks() {
    let ctx = TestCtx::new().await;
    for (n, md) in [
        (1, "看這裡<script>alert(1)</script>"),
        (2, "<img src=x onerror=alert(1)>"),
        (3, "[點我](javascript:alert(1))"),
        (4, "![截圖](https://evil.example/x.png)"),
        (2, "<iframe src=\"https://x\"></iframe>"),
    ] {
        let id = format!("mail-md-{n}-{}", md.len());
        let mut v = batch_json(&id);
        v["items"][n - 1]["context_md"] = json!(md);
        let req: CreateRequest = serde_json::from_value(v).unwrap();
        let err = ctx.handle.create_question(req, now()).await.expect_err(md);
        let msg = err.to_string();
        assert_eq!(reject_code(err), "batch_item_markdown", "{md}");
        assert!(msg.contains(&format!("第 {n} 項")) && msg.contains("context_md"), "{msg}");
        assert!(ctx.handle.get_question(&id).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn same_id_resend_only_touches_and_keeps_items() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(batch_req("mail-up"), now()).await.unwrap();
    let before = hashes(&ctx, "mail-up").await;
    let mut again = batch_json("mail-up");
    again["items"][0]["proposal"] = json!("偷改的草稿");
    let out = ctx.handle.create_question(serde_json::from_value(again).unwrap(), now()).await.unwrap();
    assert_eq!(out.dedup, Dedup::TouchedExisting);
    assert_eq!(hashes(&ctx, "mail-up").await, before, "JSON 題組建立後內容不可改（所見即所送）");
}

#[tokio::test]
async fn ask_update_cannot_add_options_to_a_batch() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(batch_req("mail-upd"), now()).await.unwrap();
    let patch: UpdatePatch =
        serde_json::from_value(json!({"options": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]})).unwrap();
    let err = ctx.handle.update_question_meta("mail-upd", patch, now()).await.expect_err("題組沒有選項");
    assert!(err.to_string().contains("題組"), "{err}");
}

// ---------------------------------------------------------------------------
// AC-BR-05：逐項作答、驗證、版本與雜湊綁定
// ---------------------------------------------------------------------------

#[tokio::test]
async fn batch_answer_is_recorded_item_by_item_in_question_order() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(batch_req("mail-ans"), now()).await.unwrap();
    let h = hashes(&ctx, "mail-ans").await;
    let mut decisions = good_decisions(&ctx, "mail-ans").await;
    decisions.as_array_mut().unwrap().reverse(); // 面板送來的順序不影響存法
    let q = ctx
        .handle
        .answer_question("mail-ans", answer(decisions, Some("  語氣再正式一點  ")), now())
        .await
        .expect("合法題組答案");
    assert_eq!(q.state, State::Answered);
    let a = serde_json::to_value(q.answer.as_ref().unwrap()).unwrap();
    assert_eq!(a["choice_id"], Value::Null);
    assert_eq!(a["free_text"], "語氣再正式一點", "整體指示存在 free_text（去頭尾空白）");
    let got: Vec<(&str, &str)> = a["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["item_id"].as_str().unwrap(), d["action"].as_str().unwrap()))
        .collect();
    assert_eq!(got, vec![("i1", "approve"), ("i2", "approve"), ("i3", "skip"), ("i4", "approve")]);
    assert_eq!(a["items"][0]["text"], "陳同學你好，修好了！\n有問題再回信給我。", "採用的文字原樣保存（所見即所送）");
    assert_eq!(a["items"][3].get("text"), None, "只核可的項目沒有文字");
    for (i, d) in a["items"].as_array().unwrap().iter().enumerate() {
        assert_eq!(d["content_hash"], h[i], "每項綁定作答當下的雜湊");
    }

    let again = ctx
        .handle
        .answer_question("mail-ans", answer(good_decisions(&ctx, "mail-ans").await, None), now())
        .await;
    assert!(matches!(again, Err(ServerError::AlreadyAnswered { .. })), "同一題只能作答一次");
}

#[tokio::test]
async fn every_item_must_be_decided_exactly_once() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(batch_req("mail-once"), now()).await.unwrap();
    let good = good_decisions(&ctx, "mail-once").await;

    let mut missing = good.clone();
    missing.as_array_mut().unwrap().pop();
    let mut duplicate = good.clone();
    let first = duplicate[0].clone();
    duplicate.as_array_mut().unwrap().push(first);
    let mut unknown = good.clone();
    unknown.as_array_mut().unwrap().push(json!({"item_id": "zz", "action": "skip", "content_hash": "sha256:x"}));

    for (name, decisions, needle) in [("缺第 4 項", missing, "i4"), ("i1 兩次", duplicate, "i1"), ("不存在的項目", unknown, "zz")] {
        let err = ctx.handle.answer_question("mail-once", answer(decisions, None), now()).await.expect_err(name);
        assert!(matches!(err, ServerError::BadAnswer(_)), "{name}：{err}");
        assert!(err.to_string().contains(needle), "{name}：{err}");
    }
    let err = ctx.handle.answer_question("mail-once", answer(json!([]), None), now()).await.expect_err("空的");
    assert!(err.to_string().contains("i1"), "{err}");
    still_open(&ctx, "mail-once").await;
}

#[tokio::test]
async fn item_text_and_note_rules() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(batch_req("mail-rules"), now()).await.unwrap();
    let good = good_decisions(&ctx, "mail-rules").await;
    let with = |index: usize, patch: Value| {
        let mut d = good.clone();
        let item = d[index].as_object_mut().unwrap();
        item.remove("text");
        item.remove("note");
        for (k, v) in patch.as_object().unwrap() {
            item.insert(k.clone(), v.clone());
        }
        d
    };
    let cases = [
        ("可編輯項目採用沒帶文字", with(0, json!({"action": "approve"})), "第 1 項"),
        ("可編輯項目採用文字空白", with(1, json!({"action": "approve", "text": "  \n "})), "第 2 項"),
        ("只核可項目帶文字", with(3, json!({"action": "approve", "text": "我自己寫"})), "第 4 項"),
        ("不處理帶文字", with(2, json!({"action": "skip", "text": "x"})), "第 3 項"),
        ("請 AI 改沒寫指示", with(1, json!({"action": "revise"})), "第 2 項"),
        ("請 AI 改指示空白", with(1, json!({"action": "revise", "note": "   "})), "第 2 項"),
        ("請 AI 改帶文字", with(1, json!({"action": "revise", "note": "短一點", "text": "x"})), "第 2 項"),
        ("草稿超過 5,000 字", with(0, json!({"action": "approve", "text": "字".repeat(5001)})), "5,000"),
        ("指示超過 2,000 字", with(1, json!({"action": "revise", "note": "改".repeat(2001)})), "2,000"),
    ];
    for (name, decisions, needle) in cases {
        let err = ctx.handle.answer_question("mail-rules", answer(decisions, None), now()).await.expect_err(name);
        assert!(matches!(err, ServerError::BadAnswer(_)), "{name}：{err}");
        assert!(err.to_string().contains(needle), "{name}：{err}");
    }
    let long_instruction = "改".repeat(2001);
    let err = ctx
        .handle
        .answer_question("mail-rules", answer(good.clone(), Some(&long_instruction)), now())
        .await
        .expect_err("整體指示太長");
    assert!(err.to_string().contains("2,000"), "{err}");
    still_open(&ctx, "mail-rules").await;

    // 邊界：5,000 字剛好可以，請 AI 改帶指示可以。
    let ok = with(0, json!({"action": "approve", "text": "字".repeat(5000)}));
    let mut ok = ok;
    ok[1] = json!({"item_id": "i2", "action": "revise", "note": "先道歉再說明", "content_hash": good[1]["content_hash"]});
    let q = ctx.handle.answer_question("mail-rules", answer(ok, None), now()).await.expect("邊界值合法");
    let a = serde_json::to_value(q.answer.unwrap()).unwrap();
    assert_eq!(a["items"][1]["note"], "先道歉再說明");
    assert_eq!(a["items"][1].get("text"), None);
}

/// AC-BR-04（QA 第 1 輪）：全部退回時那段整體指示超過 2,000 字，錯誤要說是整體指示，不是「第 1 項的指示」。
#[tokio::test]
async fn overall_instruction_error_names_the_overall_instruction() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(batch_req("mail-redo"), now()).await.unwrap();
    let h = hashes(&ctx, "mail-redo").await;
    let long = "改".repeat(2001);
    let decisions: Vec<Value> = h
        .iter()
        .enumerate()
        .map(|(i, hash)| json!({"item_id": format!("i{}", i + 1), "action": "revise", "note": long, "content_hash": hash}))
        .collect();
    let err = ctx
        .handle
        .answer_question("mail-redo", answer(Value::Array(decisions), Some(&long)), now())
        .await
        .expect_err("整體指示太長");
    let msg = err.to_string();
    assert!(msg.contains("整體指示（套用到所有項目的那段）超過 2,000 字"), "{msg}");
    assert!(!msg.contains("第 1 項"), "不能說成第 1 項的指示：{msg}");
    still_open(&ctx, "mail-redo").await;
}

/// 不准佔位文字（2026-10-09 作者）：AI 建議不處理的項目以空草稿註冊；使用者改成採用卻沒寫內容時 server 拒收，訊息和面板同一句。
#[tokio::test]
async fn empty_skip_draft_registers_and_cannot_be_approved_empty() {
    let env = Drafts::new().await;
    let md = "---\ntitle: 兩則留言的回覆\nquestion_line: 兩則留言的回覆\nkind: batch\nsource:\n  agent: claude\n  cwd: /tmp\nitems:\n  - id: b1\n    title: \"@dev_amy\"\n    context_md: 會開源嗎？\n    proposal: 還在自己用，穩定後會考慮！\n    editable: true\n  - id: b2\n    title: \"@troll_42\"\n    context_md: 又在業配\n    proposal: \"\"\n    editable: true\n    default: skip\n    default_reason: 情緒性留言\n---\n";
    let dir = env.write("cm-empty", md);
    let out = env.ctx.handle.register_draft(&dir, RegisterOptions::default(), now()).await.unwrap();
    assert!(out.ok, "空草稿的不處理項目要能註冊：{:#?}", out.issues);
    let h = hashes(&env.ctx, "cm-empty").await;
    let q = env.ctx.handle.get_question("cm-empty").await.unwrap().unwrap();
    let d = q.draft.clone().unwrap();
    let mut input = answer(
        json!([
            {"item_id": "b1", "action": "approve", "text": "還在自己用，穩定後會考慮！", "content_hash": h[0]},
            {"item_id": "b2", "action": "approve", "text": " ", "content_hash": h[1]}
        ]),
        None,
    );
    input.revision = Some(d.revision);
    input.content_hash = Some(d.content_hash.clone());
    let err = env.ctx.handle.answer_question("cm-empty", input, now()).await.expect_err("採用空白文字");
    assert!(err.to_string().contains("第 2 項沒有內容，先寫好再採用"), "{err}");
    still_open(&env.ctx, "cm-empty").await;
}

/// 所見即所送：面板作答每項都要帶看到的雜湊；不符就拒收（內容被換過）。題組不收選項。
#[tokio::test]
async fn panel_answers_are_bound_to_each_item_hash() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(batch_req("mail-hash"), now()).await.unwrap();
    let good = good_decisions(&ctx, "mail-hash").await;

    let mut no_hash = good.clone();
    no_hash[1].as_object_mut().unwrap().remove("content_hash");
    let err = ctx.handle.answer_question("mail-hash", answer(no_hash, None), now()).await.expect_err("沒帶雜湊");
    assert!(err.to_string().contains("第 2 項"), "{err}");

    let mut stale = good.clone();
    stale[2]["content_hash"] = json!("sha256:0000");
    let err = ctx.handle.answer_question("mail-hash", answer(stale, None), now()).await.expect_err("雜湊不符");
    assert!(err.to_string().contains("第 3 項"), "{err}");

    let mut with_choice = answer(good.clone(), None);
    with_choice.choice_id = Some("a".into());
    let err = ctx.handle.answer_question("mail-hash", with_choice, now()).await.expect_err("題組沒有選項");
    assert!(err.to_string().contains("題組"), "{err}");
    still_open(&ctx, "mail-hash").await;
}

/// 題組只能在面板逐項作答：CLI／Discord 的載荷沒有逐項決定。非題組帶 items 也拒收。
#[tokio::test]
async fn batch_needs_item_decisions_and_items_need_a_batch() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(batch_req("mail-cli"), now()).await.unwrap();
    let cli: AnswerInput = serde_json::from_value(json!({"channel": "cli", "free_text": "全部照做"})).unwrap();
    let err = ctx.handle.answer_question("mail-cli", cli, now()).await.expect_err("沒有逐項決定");
    assert!(err.to_string().contains("逐項"), "{err}");
    still_open(&ctx, "mail-cli").await;

    ctx.handle.create_question(base_req("plain"), now()).await.unwrap();
    let mut input = answer(json!([{"item_id": "i1", "action": "skip"}]), None);
    input.choice_id = Some("a".into());
    let err = ctx.handle.answer_question("plain", input, now()).await.expect_err("非題組帶 items");
    assert!(err.to_string().contains("items"), "{err}");
    still_open(&ctx, "plain").await;
}

/// ledger 快照與 agent 讀取面（ask_get／ask_list／ask_wait）都帶逐項決定。
#[tokio::test]
async fn item_decisions_reach_the_ledger_and_agent_reads() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(batch_req("mail-read"), now()).await.unwrap();
    let decisions = good_decisions(&ctx, "mail-read").await;
    ctx.handle.answer_question("mail-read", answer(decisions, Some("全部早上寄")), now()).await.unwrap();

    let rows = read_ledger_rows(&ctx, now());
    let row = rows
        .iter()
        .map(|r| serde_json::to_value(r).unwrap())
        .find(|r| r["type"] == "answered" && r["question"]["id"] == "mail-read")
        .expect("answered 帳本行");
    assert_eq!(row["question"]["answer"]["items"].as_array().unwrap().len(), 4, "帳本可追溯每項決定");
    assert_eq!(row["question"]["items"].as_array().unwrap().len(), 4, "帳本保存當時的項目內容");

    let got = mcp(&ctx, "ask_get", json!({"question_id": "mail-read"})).await.unwrap();
    assert_eq!(got["question"]["answer"]["items"][0]["text"], "陳同學你好，修好了！\n有問題再回信給我。");
    assert_eq!(got["question"]["answer"]["free_text"], "全部早上寄");
    let list = mcp(&ctx, "ask_list", json!({"include_terminal": true})).await.unwrap();
    let listed = list["questions"].as_array().unwrap().iter().find(|q| q["id"] == "mail-read").unwrap().clone();
    assert_eq!(listed["answer"]["items"][2]["action"], "skip");
    let waited = mcp(&ctx, "ask_wait", json!({"question_id": "mail-read", "timeout_seconds": 1})).await.unwrap();
    assert_eq!(waited["status"], "answered");
    assert_eq!(waited["answer"]["items"][3]["action"], "approve");
}

// ---------------------------------------------------------------------------
// 草稿註冊（frontmatter 的 items:）
// ---------------------------------------------------------------------------

struct Drafts {
    ctx: TestCtx,
    home: TempDir,
}

impl Drafts {
    async fn new() -> Drafts {
        let ctx = TestCtx::new().await;
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join("drafts")).unwrap();
        ctx.handle.set_draft_paths(DraftPaths {
            drafts_root: home.path().join("drafts"),
            revisions_root: home.path().join("revisions"),
        });
        Drafts { ctx, home }
    }

    fn write(&self, id: &str, md: &str) -> PathBuf {
        let dir = self.home.path().join("drafts").join(id);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("question.md"), md).unwrap();
        dir
    }
}

/// frontmatter：`- id: b2` 在第 17 行。
fn draft_md(second_context: &str, second_proposal: &str) -> String {
    format!(
        "---\ntitle: 兩則留言的回覆\nquestion_line: 兩則留言的回覆\nkind: batch\nwhy: |\n  - 08:30 新留言\n  - 現在 等你確認\nsource:\n  agent: claude\n  cwd: /tmp\nitems:\n  - id: b1\n    title: \"@dev_amy\"\n    context_md: 會開源嗎？\n    proposal: 還在自己用，穩定後會考慮！\n    editable: true\n  - id: b2\n    title: \"@studio_k\"\n    context_md: |\n      {second_context}\n    proposal: {second_proposal}\n    editable: true\n---\n"
    )
}

#[tokio::test]
async fn batch_drafts_register_with_items_and_check_each_context() {
    let env = Drafts::new().await;

    // 原文不安全 → 不建題，問題對到該項的行號、說第幾項。
    let dir = env.write("cm-bad", &draft_md("<iframe src=\"x\"></iframe>", "私訊我"));
    let out = env.ctx.handle.register_draft(&dir, RegisterOptions::default(), now()).await.unwrap();
    assert!(!out.ok);
    let issue = out.issues.iter().find(|i| i.code == "markdown_forbidden_tag").unwrap_or_else(|| panic!("{:#?}", out.issues));
    assert_eq!(issue.line, Some(17), "對到 `- id: b2` 那一行");
    assert!(issue.message.contains("第 2 項") && issue.message.contains("context_md"), "{}", issue.message);

    // 合法：原文可以引用草稿裡的圖片，收進版本快照。
    let dir = env.write("cm-ok", &draft_md("看截圖：![留言串](./assets/thread.png)", "私訊我你們的團隊規模"));
    std::fs::write(dir.join("assets/thread.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    let out = env.ctx.handle.register_draft(&dir, RegisterOptions::default(), now()).await.unwrap();
    assert!(out.ok, "{:#?}", out.issues);
    assert_eq!(out.revision, Some(1));
    let bundle = env.ctx.handle.evidence_bundle("cm-ok", 1).await.unwrap().expect("版本快照");
    assert!(bundle.assets.contains_key("./assets/thread.png"), "原文引用的圖片要收進快照");
    let h1 = hashes(&env.ctx, "cm-ok").await;
    assert_eq!(h1.len(), 2);

    // 改草稿重新註冊 → 升版、該項雜湊改變；用舊雜湊作答被拒。
    env.write("cm-ok", &draft_md("看截圖：![留言串](./assets/thread.png)", "私訊我，我先看適不適合"));
    let out = env.ctx.handle.register_draft(&dir, RegisterOptions::default(), now()).await.unwrap();
    assert!(out.ok, "{:#?}", out.issues);
    assert_eq!(out.revision, Some(2));
    let h2 = hashes(&env.ctx, "cm-ok").await;
    assert_eq!(h1[0], h2[0]);
    assert_ne!(h1[1], h2[1]);

    let q = env.ctx.handle.get_question("cm-ok").await.unwrap().unwrap();
    let d = q.draft.clone().unwrap();
    let decide = |hash: &str| {
        json!([
            {"item_id": "b1", "action": "approve", "text": "還在自己用，穩定後會考慮！", "content_hash": h2[0]},
            {"item_id": "b2", "action": "approve", "text": "私訊我，我先看適不適合", "content_hash": hash}
        ])
    };
    let mut stale = answer(decide(&h1[1]), None);
    stale.revision = Some(d.revision);
    stale.content_hash = Some(d.content_hash.clone());
    let err = env.ctx.handle.answer_question("cm-ok", stale, now()).await.expect_err("舊雜湊");
    assert!(err.to_string().contains("第 2 項"), "{err}");

    let mut fresh = answer(decide(&h2[1]), None);
    fresh.revision = Some(d.revision);
    fresh.content_hash = Some(d.content_hash.clone());
    let q = env.ctx.handle.answer_question("cm-ok", fresh, now()).await.expect("看過新版的答案");
    let a = q.answer.unwrap();
    assert_eq!(a.revision, Some(2));
}

// ---------------------------------------------------------------------------
// AC-BR-06：答案回到原對話時的訊息
// ---------------------------------------------------------------------------

#[tokio::test]
async fn continuation_prompt_lists_each_decision_and_the_next_round() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(batch_req("mail-1009"), now()).await.unwrap();
    let h = hashes(&ctx, "mail-1009").await;
    let edited = "陳同學你好，修好了！\n```\n若還是不行請回信\n```";
    let decisions = json!([
        {"item_id": "i1", "action": "approve", "text": edited, "content_hash": h[0]},
        {"item_id": "i2", "action": "revise", "note": "先感謝再婉拒，留合作窗口", "content_hash": h[1]},
        {"item_id": "i3", "action": "skip", "content_hash": h[2]},
        {"item_id": "i4", "action": "approve", "content_hash": h[3]}
    ]);
    let q = ctx.handle.answer_question("mail-1009", answer(decisions, Some("語氣正式一點")), now()).await.unwrap();
    let prompt = kunsu_server::reactor::continuation_prompt(&q, None);
    for needle in [
        "題組",
        "第 1 輪",
        "陳同學",
        "某品牌窗口（品牌合作）",
        "某軟體廠商",
        "張同學退款",
        "item_id: i1",
        "使用者改過",
        "請 AI 改",
        "先感謝再婉拒，留合作窗口",
        "不處理",
        "語氣正式一點",
        "一字不改",
        "mail-1009-r2",
        "ask_report",
    ] {
        assert!(prompt.contains(needle), "訊息要有 {needle:?}：\n{prompt}");
    }
    assert!(prompt.contains(edited), "採用的最終文字原樣保留（含換行與反引號）：\n{prompt}");
    assert!(prompt.contains("符合退費規則 → 建議全額退款 NT$4,800"), "只核可項目照建議執行：\n{prompt}");
    assert!(prompt.contains("採用 2"), "總數：\n{prompt}");

    // 第 2 輪的題再被退回 → 第 3 輪。
    ctx.handle.create_question(batch_req("mail-1009-r2"), now()).await.unwrap();
    let h = hashes(&ctx, "mail-1009-r2").await;
    let decisions = json!([
        {"item_id": "i1", "action": "revise", "note": "再短一點", "content_hash": h[0]},
        {"item_id": "i2", "action": "skip", "content_hash": h[1]},
        {"item_id": "i3", "action": "skip", "content_hash": h[2]},
        {"item_id": "i4", "action": "skip", "content_hash": h[3]}
    ]);
    let q = ctx.handle.answer_question("mail-1009-r2", answer(decisions, None), now()).await.unwrap();
    let prompt = kunsu_server::reactor::continuation_prompt(&q, None);
    assert!(prompt.contains("第 2 輪") && prompt.contains("mail-1009-r3"), "{prompt}");
}

/// 開新對話（原對話接不上或 wake=fresh）時，新對話同樣拿到逐項決定與規則，不只一包 JSON。
#[tokio::test]
async fn fresh_conversation_prompt_carries_the_batch_rules() {
    let ctx = TestCtx::new().await;
    let bin = TempDir::new().unwrap();
    let claude = write_script(bin.path(), "claude.sh", "#!/bin/sh\necho 'EXEC_OK: 已處理'\n");
    let mut v = batch_json("mail-fresh");
    v["wake_on_answer"] = json!({"mode": "fresh"});
    ctx.handle.create_question(serde_json::from_value(v).unwrap(), now()).await.unwrap();
    let h = hashes(&ctx, "mail-fresh").await;
    let edited = "陳同學你好，修好了！\n有問題再回信給我。";
    let decisions = json!([
        {"item_id": "i1", "action": "approve", "text": edited, "content_hash": h[0]},
        {"item_id": "i2", "action": "revise", "note": "婉拒但留窗口", "content_hash": h[1]},
        {"item_id": "i3", "action": "skip", "content_hash": h[2]},
        {"item_id": "i4", "action": "approve", "content_hash": h[3]}
    ]);
    ctx.handle.answer_question("mail-fresh", answer(decisions, None), now()).await.unwrap();

    let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
    cfg.claude_bin = Some(claude);
    cfg.retry_base_ms = 1;
    let reactor = Reactor::new(ctx.handle.clone(), cfg, std::sync::Arc::new(StaticKeyProvider(test_key())));
    reactor.process_pending().await.unwrap();
    let planned = reactor.planned();
    let fresh = planned.iter().find(|c| c.kind == "wake_claude").expect("開新對話的命令");
    // 開新對話的 prompt 走 stdin（CONTRACT §8 第 5 步；QA 第 1 輪：放在命令列參數會撞 ARG_MAX）。
    let prompt = fresh.stdin.as_deref().expect("prompt 走 stdin");
    for needle in ["逐項決定", "一字不改", "item_id: i2", "婉拒但留窗口", "mail-fresh-r2", edited] {
        assert!(prompt.contains(needle), "新對話的 prompt 要有 {needle:?}：\n{prompt}");
    }
}
