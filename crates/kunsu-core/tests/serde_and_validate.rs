//! §2 serde 形狀測試：snake_case、enum 白名單、未知欄位拒收、
//! 以及 validate_create 的其餘硬驗證 + 十秒可決 lint。

mod common;

use common::*;
use kunsu_core::*;

/// enum 序列化必須全 snake_case（agent 端 JSON 的穩定 wire format）。
#[test]
fn enums_serialize_snake_case() {
    assert_eq!(serde_json::to_string(&Kind::KongkoReview).unwrap(), "\"kongko_review\"");
    assert_eq!(serde_json::to_string(&Tier::T0).unwrap(), "\"t0\"");
    assert_eq!(serde_json::to_string(&Gate::Red).unwrap(), "\"red\"");
    assert_eq!(serde_json::to_string(&State::Superseded).unwrap(), "\"superseded\"");
    assert_eq!(serde_json::to_string(&Channel::AutoPolicy).unwrap(), "\"auto_policy\"");
    assert_eq!(serde_json::to_string(&OnExpire::EscalateDiscord).unwrap(), "\"escalate_discord\"");
    assert_eq!(serde_json::to_string(&WakeMode::None).unwrap(), "\"none\"");
    assert_eq!(serde_json::to_string(&AgentKind::Codex).unwrap(), "\"codex\"");
}

/// enum 白名單：未知 variant 直接 deserialize error（400 的第一道門）。
#[test]
fn enum_whitelist_rejects_unknown_variants() {
    assert!(serde_json::from_str::<Kind>("\"poll\"").is_err());
    assert!(serde_json::from_str::<Tier>("\"t5\"").is_err());
    assert!(serde_json::from_str::<Gate>("\"green\"").is_err());
    assert!(serde_json::from_str::<Channel>("\"mcp\"").is_err(), "MCP 不是作答通道");
    assert!(serde_json::from_str::<OnExpire>("\"auto_execute\"").is_err());
    assert!(serde_json::from_str::<AgentKind>("\"gemini\"").is_err());
}

/// CreateRequest 打錯欄位名要大聲失敗，不准無聲吞掉。
#[test]
fn create_request_rejects_unknown_fields() {
    let mut v = serde_json::to_value(base_req()).unwrap();
    v.as_object_mut().unwrap().insert("predicton_ref".into(), "typo".into());
    assert!(serde_json::from_value::<CreateRequest>(v).is_err());
}

/// Question 全量 JSON round-trip（ledger 快照的穩定性基礎）。
#[test]
fn question_json_roundtrip() {
    let mut q = Question::from_create(money_req(), now());
    q.answer = Some(Answer {
        answer_id: "5d3a7c1e-0000-4000-8000-000000000001".into(),
        choice_id: Some("a".into()),
        free_text: None,
        edited_params: None,
        channel: Channel::Ui,
        answered_at: now(),
        reauthorized_at: None,
        signature: Some("hmac:abc".into()),
        acked_at: None,
        executed_at: None,
        execution_result: None,
        delivery: None,
        report: None,
        revision: Some(2),
        content_hash: Some("sha256:abc".into()),
        items: None,
    });
    q.draft = Some(DraftRef {
        revision: 2,
        content_hash: "sha256:abc".into(),
        canvas: Some(CanvasKind::Tsx),
        registered_at: now(),
    });
    let json = serde_json::to_string(&q).unwrap();
    let back: Question = serde_json::from_str(&json).unwrap();
    assert_eq!(back, q);
    // 快照欄位確實是 snake_case
    assert!(json.contains("\"question_line\""));
    assert!(json.contains("\"first_proposed\""));
}

/// allow_free_text 省略時預設 true（恆 true 的預設面）。
#[test]
fn allow_free_text_defaults_true() {
    let mut v = serde_json::to_value(base_req()).unwrap();
    v.as_object_mut().unwrap().remove("allow_free_text");
    let req: CreateRequest = serde_json::from_value(v).unwrap();
    assert!(req.allow_free_text);
    // 顯式 false → validate 拒收
    let mut req = base_req();
    req.allow_free_text = false;
    assert_eq!(validate_create(&req), Err(RejectReason::FreeTextMustBeAllowed));
}

#[test]
fn validate_hard_rules() {
    // 合法題通過
    assert_eq!(validate_create(&base_req()), Ok(()));

    // id 空
    let mut req = base_req();
    req.id = " ".into();
    assert_eq!(validate_create(&req), Err(RejectReason::IdEmpty));

    // title > 80 字（中文以字元計）
    let mut req = base_req();
    req.title = "題".repeat(81);
    assert_eq!(validate_create(&req), Err(RejectReason::TitleTooLong { len: 81 }));
    req.title = "題".repeat(80);
    assert_eq!(validate_create(&req), Ok(()));

    // question_line > 24 字
    let mut req = base_req();
    req.question_line = "字".repeat(25);
    assert_eq!(validate_create(&req), Err(RejectReason::QuestionLineTooLong { len: 25 }));
    req.question_line = "字".repeat(24);
    assert_eq!(validate_create(&req), Ok(()));

    // visual_symbol 是 agent 可控、但嚴格受限的粒子輪廓。
    let mut req = base_req();
    req.visual_symbol = Some("CODEX".into());
    assert_eq!(validate_create(&req), Err(RejectReason::VisualSymbolTooLong { len: 5 }));
    req.visual_symbol = Some("⌬".into());
    assert_eq!(validate_create(&req), Ok(()));

    // evidence > 3 條
    let mut req = base_req();
    let line = req.evidence[0].clone();
    req.evidence = vec![line.clone(), line.clone(), line.clone(), line];
    assert_eq!(validate_create(&req), Err(RejectReason::TooManyEvidence { len: 4 }));

    // evidence 缺 source 身世標籤
    let mut req = base_req();
    req.evidence.push(EvidenceLine { text: "沒出處的數字".into(), source: "".into() });
    assert_eq!(validate_create(&req), Err(RejectReason::EvidenceMissingSource { index: 1 }));

    // pick_option 缺 option_id / 指到不存在選項
    let mut req = base_req();
    req.default_action = DefaultAction { on_expire: OnExpire::PickOption, option_id: None };
    assert_eq!(validate_create(&req), Err(RejectReason::PickOptionNeedsOptionId));
    req.default_action.option_id = Some("z".into());
    assert_eq!(
        validate_create(&req),
        Err(RejectReason::PickOptionUnknownOptionId { option_id: "z".into() })
    );
    req.default_action.option_id = Some("a".into());
    assert_eq!(validate_create(&req), Ok(()));
}

/// 十秒可決 lint：reject 且標 malformed。
#[test]
fn ten_second_lint_flags_malformed() {
    // choice 題無 options
    let mut req = base_req();
    req.options = vec![];
    let err = validate_create(&req).unwrap_err();
    assert_eq!(err, RejectReason::ChoiceOptionsOutOfRange { len: 0 });
    assert!(err.is_malformed());
    assert_eq!(err.code(), "malformed_choice_options");

    // choice 題 5 個選項也超標
    let mut req = base_req();
    let opt = req.options[0].clone();
    req.options = (0..5)
        .map(|i| Opt { id: format!("o{i}"), ..opt.clone() })
        .collect();
    assert!(validate_create(&req).unwrap_err().is_malformed());

    // 動錢題無 params_line
    let mut req = money_req();
    req.params_line = None;
    let err = validate_create(&req).unwrap_err();
    assert_eq!(err, RejectReason::MoneyNeedsParamsLine);
    assert!(err.is_malformed());

    // 非 lint 類錯誤不標 malformed
    assert!(!RejectReason::MoneyRequiresPrediction.is_malformed());
    assert!(!RejectReason::RedGateNeedsChoice.is_malformed());

    // 錯誤訊息是給 agent 讀的人話
    let msg = RejectReason::MoneyNeedsParamsLine.to_string();
    assert!(msg.contains("params_line"));
}
