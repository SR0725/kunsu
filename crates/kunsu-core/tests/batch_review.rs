//! 題組（PRD §25；BDD batch-review AC-BR-01）：`kind: batch` 的 serde、每項雜湊與建題驗證（table-driven）。
//! 請求以 JSON 組出來（agent 的實際 wire format），錯誤要指出第幾項、哪個欄位。

mod common;

use common::*;
use kunsu_core::*;
use serde_json::{json, Value};

fn item(id: &str) -> Value {
    json!({
        "id": id,
        "title": format!("寄件人 {id}"),
        "subtitle": "主旨",
        "context_md": "老師好，**第 5 章**影片打不開。",
        "proposal": "你好，謝謝回報！已經修好了。",
        "editable": true
    })
}

/// 合法題組：預設 2 項（i1、i2），可用 `items` 覆寫。
fn batch_json(items: Vec<Value>) -> Value {
    let mut v = serde_json::to_value(base_req()).unwrap();
    let o = v.as_object_mut().unwrap();
    o.insert("id".into(), json!("mail-1009"));
    o.insert("kind".into(), json!("batch"));
    o.insert("tier".into(), json!("t1"));
    o.insert("gate".into(), Value::Null);
    o.insert("options".into(), json!([]));
    o.insert("items".into(), Value::Array(items));
    v
}

fn batch_req(items: Vec<Value>) -> CreateRequest {
    serde_json::from_value(batch_json(items)).expect("題組 JSON 要能解析")
}

fn two() -> Vec<Value> {
    vec![item("i1"), item("i2")]
}

#[test]
fn batch_kind_parses_and_items_default_to_approve() {
    assert_eq!(serde_json::to_string(&serde_json::from_str::<Kind>("\"batch\"").unwrap()).unwrap(), "\"batch\"");
    let req = batch_req(two());
    let v = serde_json::to_value(&req).unwrap();
    assert_eq!(v["items"][0]["default"], "approve", "省略 default＝採用（PRD §25：每項預設採用）");
    assert_eq!(v["items"][1]["subtitle"], "主旨");
    validate_create(&req).expect("合法題組通過驗證");
}

#[test]
fn non_batch_questions_do_not_serialize_items() {
    let q = Question::from_create(base_req(), now());
    let v = serde_json::to_value(&q).unwrap();
    assert!(v.get("items").is_none(), "非題組不帶 items（舊 client 與 ledger 形狀不變）");
}

/// 每項內容雜湊由 Kunsu 算（agent 帶的值會被覆寫）；內容一樣雜湊一樣，任何欄位改了就不同。
#[test]
fn every_item_gets_a_server_computed_content_hash() {
    let mut forged = item("i2");
    forged["content_hash"] = json!("sha256:forged");
    let q = Question::from_create(batch_req(vec![item("i1"), forged]), now());
    let v = serde_json::to_value(&q).unwrap();
    let h1 = v["items"][0]["content_hash"].as_str().expect("i1 有雜湊").to_string();
    let h2 = v["items"][1]["content_hash"].as_str().expect("i2 有雜湊").to_string();
    assert!(h1.starts_with("sha256:") && h1.len() == 7 + 64, "{h1}");
    assert_ne!(h2, "sha256:forged", "agent 帶的雜湊一律覆寫");

    let again = Question::from_create(batch_req(vec![item("i1"), item("i2")]), now());
    let again = serde_json::to_value(&again).unwrap();
    assert_eq!(again["items"][0]["content_hash"], h1, "同內容同雜湊");

    for (field, value) in [
        ("proposal", json!("改過的草稿")),
        ("context_md", json!("另一封信")),
        ("title", json!("別人")),
        ("editable", json!(false)),
    ] {
        let mut changed = item("i1");
        changed[field] = value;
        let q = Question::from_create(batch_req(vec![changed, item("i2")]), now());
        let v = serde_json::to_value(&q).unwrap();
        assert_ne!(v["items"][0]["content_hash"], h1, "{field} 改了雜湊要變");
    }
}

/// 打錯項目欄位名要大聲失敗。
#[test]
fn unknown_item_field_is_rejected() {
    let mut bad = item("i2");
    bad.as_object_mut().unwrap().insert("contex_md".into(), json!("typo"));
    assert!(serde_json::from_value::<CreateRequest>(batch_json(vec![item("i1"), bad])).is_err());
}

#[test]
fn twenty_items_and_a_5000_char_draft_are_allowed() {
    let items: Vec<Value> = (1..=20).map(|i| item(&format!("i{i}"))).collect();
    validate_create(&batch_req(items)).expect("20 項是上限內");

    let mut long = item("i2");
    long["proposal"] = json!("字".repeat(5000));
    validate_create(&batch_req(vec![item("i1"), long])).expect("可編輯草稿 5,000 字是上限內");

    // 2026-10-09（QA continuity 第 9 輪）：只核可的建議也 ≤5,000 字，回原對話的訊息才塞得進命令列參數。
    let mut approve_only = item("i2");
    approve_only["editable"] = json!(false);
    approve_only["proposal"] = json!("建".repeat(5000));
    validate_create(&batch_req(vec![item("i1"), approve_only.clone()])).expect("只核可的建議 5,000 字是上限內");
    approve_only["proposal"] = json!("建議".repeat(3001));
    let err = validate_create(&batch_req(vec![item("i1"), approve_only])).expect_err("只核可的建議超過 5,000 字要拒收");
    assert_eq!(err.code(), "batch_item_proposal_too_long");

    let mut skip = item("i2");
    skip["default"] = json!("skip");
    skip["default_reason"] = json!("促銷信，不用回");
    validate_create(&batch_req(vec![item("i1"), skip])).expect("AI 建議不處理並寫明原因");
}

/// AC-BR-01：違反時拒收，錯誤碼可分支、訊息指出第幾項哪個欄位。
#[test]
fn batch_create_rejections_name_the_item_and_field() {
    type Mutate = fn(&mut Value);
    let cases: Vec<(&str, Mutate, &str, &[&str])> = vec![
        ("沒有項目", |v| v["items"] = json!([]), "batch_items_out_of_range", &["1", "20", "0"]),
        (
            "21 項",
            |v| v["items"] = Value::Array((1..=21).map(|i| item(&format!("i{i}"))).collect()),
            "batch_items_out_of_range",
            &["21"],
        ),
        ("非題組帶 items", |v| v["kind"] = json!("choice"), "items_only_for_batch", &["items"]),
        (
            "題組帶選項",
            |v| v["options"] = json!([{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]),
            "batch_no_options",
            &["options"],
        ),
        (
            "題組是錢題",
            |v| v["money"] = json!({"is_money_op": true, "amount_twd": 600}),
            "batch_money_forbidden",
            &["錢"],
        ),
        ("id 有路徑", |v| v["items"][1]["id"] = json!("../x"), "batch_item_id_invalid", &["第 2 項", "id"]),
        ("id 以 . 開頭", |v| v["items"][1]["id"] = json!(".hidden"), "batch_item_id_invalid", &["第 2 項"]),
        ("id 空白", |v| v["items"][0]["id"] = json!(""), "batch_item_id_invalid", &["第 1 項"]),
        ("id 重複", |v| v["items"][1]["id"] = json!("i1"), "batch_item_id_duplicate", &["第 2 項", "i1"]),
        ("標題空白", |v| v["items"][1]["title"] = json!("  "), "batch_item_title_empty", &["第 2 項", "title"]),
        (
            "標題太長",
            |v| v["items"][0]["title"] = json!("名".repeat(81)),
            "batch_item_title_too_long",
            &["第 1 項", "title", "81"],
        ),
        ("建議空白", |v| v["items"][1]["proposal"] = json!(" \n"), "batch_item_proposal_empty", &["第 2 項", "proposal"]),
        (
            "可編輯草稿太長",
            |v| v["items"][1]["proposal"] = json!("字".repeat(5001)),
            "batch_item_proposal_too_long",
            &["第 2 項", "proposal", "5001"],
        ),
        (
            "預設不處理沒寫原因",
            |v| v["items"][1]["default"] = json!("skip"),
            "batch_item_skip_needs_reason",
            &["第 2 項", "default_reason"],
        ),
    ];
    for (name, mutate, code, needles) in cases {
        let mut v = batch_json(two());
        mutate(&mut v);
        let req: CreateRequest = serde_json::from_value(v).unwrap_or_else(|e| panic!("{name}：JSON 要能解析：{e}"));
        let err = validate_create(&req).expect_err(name);
        assert_eq!(err.code(), code, "{name}：{err}");
        let msg = err.to_string();
        for needle in needles {
            assert!(msg.contains(needle), "{name}：訊息要提到 {needle:?}：{msg}");
        }
    }
}

// ---------------------------------------------------------------------------
// QA 第 1 輪（2026-10-09，作者核准）
// ---------------------------------------------------------------------------

/// AC-BR-05：Kongko 發佈事件沒有逐項決定，不能結案題組 → 題組建題時就拒收 `kongko_ref`。
#[test]
fn batch_with_kongko_ref_is_rejected() {
    let mut v = batch_json(two());
    v["kongko_ref"] = json!("batch-kk-1");
    let req: CreateRequest = serde_json::from_value(v).unwrap();
    let err = validate_create(&req).expect_err("題組帶 kongko_ref 要拒收");
    assert_eq!(err.code(), "batch_kongko_ref");
    let msg = err.to_string();
    assert!(msg.contains("kongko_ref") && msg.contains("逐項"), "訊息要說明為什麼：{msg}");
    // 一般題照舊可以帶 kongko_ref（素材題由發佈事件結案）。
    let mut plain = serde_json::to_value(base_req()).unwrap();
    plain["kongko_ref"] = json!("batch-kk-1");
    validate_create(&serde_json::from_value(plain).unwrap()).expect("一般題的 kongko_ref 不受影響");
}

/// AC-BR-06：第 2 輪以後的題組 id 是 `<原 id>-rN`，一定要是合法題目 id → 題組 id（不含輪次後綴）最多 116 字。
#[test]
fn batch_id_leaves_room_for_the_round_suffix() {
    let with_id = |id: &str| {
        let mut v = batch_json(two());
        v["id"] = json!(id);
        serde_json::from_value::<CreateRequest>(v).unwrap()
    };
    let max = format!("mail-{}", "x".repeat(111)); // 116 字
    validate_create(&with_id(&max)).expect("116 字的題組 id 合法");
    let r2 = batch::next_round_id(&max);
    assert!(validate::valid_id(&r2), "{r2} 要是合法 id");
    validate_create(&with_id(&r2)).expect("第 2 輪 id 合法");
    let r99 = format!("{max}-r99");
    assert!(validate::valid_id(&r99), "到第 99 輪都合法");

    let long = format!("mail-{}", "x".repeat(112)); // 117 字
    let err = validate_create(&with_id(&long)).expect_err("117 字的題組 id 沒有輪次後綴的空間");
    assert_eq!(err.code(), "batch_id_too_long");
    let msg = err.to_string();
    for needle in ["117", "116", "-r2", "120"] {
        assert!(msg.contains(needle), "訊息要提到 {needle:?}：{msg}");
    }
    let full = format!("mail-{}", "x".repeat(115)); // 120 字
    assert_eq!(validate_create(&with_id(&full)).unwrap_err().code(), "batch_id_too_long");

    // 一般題不受影響：120 字照樣合法。
    let mut plain = serde_json::to_value(base_req()).unwrap();
    plain["id"] = json!(full);
    validate_create(&serde_json::from_value(plain).unwrap()).expect("一般題 120 字 id 合法");
}

/// 2026-10-09 作者：AI 建議不處理的可編輯項目給空草稿或真的回覆，不准佔位文字 → 空草稿只在這種項目上允許。
#[test]
fn skip_default_item_may_have_an_empty_draft() {
    let mut skip = item("i2");
    skip["proposal"] = json!("");
    skip["default"] = json!("skip");
    skip["default_reason"] = json!("促銷信");
    validate_create(&batch_req(vec![item("i1"), skip.clone()])).expect("不處理的可編輯項目可以是空草稿");

    let mut approve = item("i2");
    approve["proposal"] = json!("");
    let err = validate_create(&batch_req(vec![item("i1"), approve])).unwrap_err();
    assert_eq!(err.code(), "batch_item_proposal_empty", "預設採用的項目一定要有草稿");

    let mut fixed = skip;
    fixed["editable"] = json!(false);
    let err = validate_create(&batch_req(vec![item("i1"), fixed])).unwrap_err();
    assert_eq!(err.code(), "batch_item_proposal_empty", "只核可的項目一定要有建議");
}

/// 可編輯項目採用時最終文字不能是空的（只有空白也算）；錯誤訊息和面板同一句。單項指示的錯誤寫明是「請 AI 改」的指示。
#[test]
fn approving_an_empty_text_names_the_item() {
    let items = batch::seal_items(batch_req(two()).items);
    let decision = |id: &str, action: &str, text: Option<&str>, note: Option<&str>| -> ItemDecision {
        serde_json::from_value(json!({"item_id": id, "action": action, "text": text, "note": note})).unwrap()
    };
    let err = batch::check_answer(
        &items,
        vec![decision("i1", "skip", None, None), decision("i2", "approve", Some("  \n "), None)],
        false,
    )
    .unwrap_err();
    assert_eq!(err, "第 2 項沒有內容，先寫好再採用");
    let err = batch::check_answer(
        &items,
        vec![decision("i1", "revise", None, Some(&"改".repeat(2001))), decision("i2", "skip", None, None)],
        false,
    )
    .unwrap_err();
    assert!(err.contains("第 1 項的指示（請 AI 改）超過 2,000 字"), "{err}");
}

/// 題組是錢題時先講「題組不能動錢」，而不是叫 agent 去補 prediction。
#[test]
fn money_batch_is_rejected_before_money_field_checks() {
    let mut v = batch_json(two());
    v["money"] = json!({"is_money_op": true});
    let req: CreateRequest = serde_json::from_value(v).unwrap();
    assert_eq!(validate_create(&req).unwrap_err().code(), "batch_money_forbidden");
}

/// 草稿重新註冊（升版）換掉 items 並重算每項雜湊；週期不動（沿用 apply_draft_update 的合約）。
#[test]
fn draft_update_replaces_items_and_rehashes() {
    let original = Question::from_create(batch_req(two()), now());
    let before = serde_json::to_value(&original).unwrap();
    let mut changed = item("i2");
    changed["proposal"] = json!("第 2 版草稿");
    let incoming = batch_req(vec![item("i1"), changed, item("i3")]);
    let draft = DraftRef { revision: 2, content_hash: "sha256:v2".into(), canvas: None, registered_at: now() };
    let updated = upsert::apply_draft_update(&original, &incoming, draft, now());
    let after = serde_json::to_value(&updated).unwrap();
    assert_eq!(after["items"].as_array().unwrap().len(), 3);
    assert_eq!(after["items"][0]["content_hash"], before["items"][0]["content_hash"], "沒改的項目雜湊不變");
    assert_ne!(after["items"][1]["content_hash"], before["items"][1]["content_hash"], "改過的項目雜湊要變");
    assert!(after["items"][2]["content_hash"].as_str().is_some_and(|h| h.starts_with("sha256:")));
}

#[test]
fn assistant_single_reply_keeps_editable_batch_flow() {
    validate_create(&batch_req(vec![item("i1")])).expect("one reply or one revision remains an editable batch");
    assert!(validate_create(&batch_req(vec![])).is_err());
}
