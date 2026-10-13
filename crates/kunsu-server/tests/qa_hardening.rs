//! QA 收貨後補的錢鏈硬規則（CONTRACT §6 規則 6–12）驗收測試：
//! INV-15 inv_money_chain_ui_only、INV-16 inv_edited_params_rewrite_prepare_cmd、
//! T1 批次窗接線、24h 無 ack 告警、疲勞降載、ask_update 嚴格化、
//! /ingest/discord-answer 端點。

mod common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use common::*;
use kunsu_core::NotifyKind;
use kunsu_core::types::{Channel, Gate, Money, Opt, State, Tier, Wake, WakeMode};
use kunsu_server::{AnswerInput, Reactor, ReactorConfig, ServerError, ServerEvent, StaticKeyProvider};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// 共用小工具
// ---------------------------------------------------------------------------

fn non_ui_answer(choice: &str, channel: Channel) -> AnswerInput {
    AnswerInput {
        choice_id: Some(choice.into()),
        free_text: None,
        edited_params: None,
        channel,
        signature: None,
        revision: None,
        content_hash: None,
        items: None,
    }
}

async fn rpc(client: &reqwest::Client, base: &str, body: Value) -> Value {
    client
        .post(format!("{base}/mcp"))
        .bearer_auth(TEST_TOKEN)
        .header("Accept", "application/json, text/event-stream")
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn rpc_call(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// 可被 edit_option 編輯的錢類題（option/params_line 都帶 300 → 360 金額變更）。
fn editable_money_req(id: &str, prepare_cmd: &str) -> kunsu_core::types::CreateRequest {
    let mut req = base_req(id);
    req.tier = Tier::T1;
    req.options = vec![
        Opt {
            id: "a".into(),
            label: "日預算 300 → 360（+20%）".into(),
            detail: None,
            recommended: Some(true),
        },
        Opt { id: "b".into(), label: "維持".into(), detail: None, recommended: None },
    ];
    req.money = Some(Money {
        is_money_op: true,
        amount_twd: Some(360),
        entity_id: Some(TEST_ENTITY.into()),
        executor_prepare_cmd: Some(prepare_cmd.to_string()),
    });
    req.prediction_ref = Some("projects/meta-ads/predictions/2026-07-31-edit.md".into());
    req.params_line = Some(format!("{TEST_ENTITY} 日預算 300 → 360"));
    req
}

const BUDGET_PREPARE_CMD: &str =
    "python3 projects/meta-ads/scripts/executor.py prepare-budget --entity 120000000000005 --new-budget 360";

// ---------------------------------------------------------------------------
// INV-15 inv_money_chain_ui_only（CONTRACT §6 規則 6）
// ---------------------------------------------------------------------------

/// 錢題只認 channel=ui：cli / discord 服務面一律拒；ui 正常作答。
#[tokio::test]
async fn inv_money_chain_ui_only() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(money_t0_req("pa-ui-only"), now()).await.unwrap();

    for channel in [Channel::Cli, Channel::Discord] {
        let err = ctx
            .handle
            .answer_question("pa-ui-only", non_ui_answer("a", channel), now())
            .await
            .unwrap_err();
        assert!(
            matches!(err, ServerError::MoneyAnswerRequiresUi { .. }),
            "channel={channel:?} 對錢題必拒，拿到 {err:?}"
        );
        assert!(err.to_string().contains("錢題只能在 Kunsu app 或已配對的手機批准"), "{err}");
        let q = ctx.handle.get_question("pa-ui-only").await.unwrap().unwrap();
        assert_eq!(q.state, State::Pending, "被拒的作答不得改變狀態");
        assert!(q.answer.is_none());
    }

    // ui 通道正常作答。
    let q = ctx.handle.answer_question("pa-ui-only", ui_answer("a"), now()).await.unwrap();
    assert_eq!(q.state, State::Answered);
    assert_eq!(q.answer.as_ref().unwrap().channel, Channel::Ui);

    // 已配對手機（channel=mobile，2026-10-12）同樣可以批准錢題。
    ctx.handle.create_question(money_t0_req("pa-mobile"), now()).await.unwrap();
    let q = ctx.handle.answer_question("pa-mobile", non_ui_answer("a", Channel::Mobile), now()).await.unwrap();
    assert_eq!(q.state, State::Answered);
    assert_eq!(q.answer.as_ref().unwrap().channel, Channel::Mobile);
    assert!(Channel::Mobile.is_ray_device() && Channel::Ui.is_ray_device());
    assert!(!Channel::Cli.is_ray_device() && !Channel::Discord.is_ray_device());
}

/// HTTP 面：/cli/answer 打錢題 → 400 + 可讀訊息；非錢題照常。
#[tokio::test]
async fn inv_money_chain_ui_only_http_cli_answer() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();
    // money_t0_req 的 deadline 錨在固定測試時鐘 → 建題也用固定時鐘，避免即刻過期。
    ctx.handle.create_question(money_t0_req("pa-cli-money"), now()).await.unwrap();
    ctx.handle.create_question(base_req("pa-cli-plain"), now()).await.unwrap();

    let resp = client
        .post(format!("{base}/cli/answer"))
        .bearer_auth(common::TEST_ADMIN_TOKEN)
        .json(&json!({"question_id": "pa-cli-money", "choice_id": "a"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "money_answer_ui_only");
    assert!(
        body["error"].as_str().unwrap().contains("錢題只能在 Kunsu app 或已配對的手機批准"),
        "{body}"
    );
    let q = ctx.handle.get_question("pa-cli-money").await.unwrap().unwrap();
    assert_eq!(q.state, State::Pending, "錢一分未動、題保持待答");

    // 非錢題 CLI 通道不受影響。
    let resp = client
        .post(format!("{base}/cli/answer"))
        .bearer_auth(common::TEST_ADMIN_TOKEN)
        .json(&json!({"question_id": "pa-cli-plain", "choice_id": "a"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

/// ui 通道作答錢題 → reactor 批准簽章鏈正常觸發（dry_run 下組出 commit 命令）。
#[tokio::test]
async fn ui_channel_still_triggers_money_chain() {
    let ctx = TestCtx::new().await;
    // 假 executor.py 放在 server 寫死的固定路徑（spec 由 argv 誠實回填）。
    write_fake_executor(&ctx);
    std::fs::create_dir_all(ctx.vault.path().join("playbooks")).unwrap();
    std::fs::write(ctx.vault.path().join("playbooks/approved-money-op.md"), "# 錢 playbook").unwrap();

    let mut req = money_t0_req("pa-ui-chain");
    req.wake_on_answer = Wake {
        mode: WakeMode::Fresh,
        session_id: None,
        playbook: Some("playbooks/approved-money-op.md".into()),
    };
    ctx.handle.create_question(req, now()).await.unwrap();
    ctx.handle.answer_question("pa-ui-chain", ui_answer("a"), now()).await.unwrap();

    let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
    cfg.dry_run = true;
    cfg.claude_bin = Some("/opt/fake/claude".into());
    cfg.retry_base_ms = 1;
    let reactor = Reactor::new(ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key())));
    reactor.process_pending().await.unwrap();

    assert!(
        reactor.planned().iter().any(|c| c.kind == "commit"),
        "ui 作答的黃燈錢題必須進批准簽章鏈：{:?}",
        reactor.planned()
    );
    assert!(
        ctx.vault
            .path()
            .join("projects/meta-ads/data/executor_pending/approvals/pa-ui-chain.json")
            .exists(),
        "approval record 必須落檔"
    );
}

/// kongko 發佈事件命中錢題（agent 建錯）→ 拒絕自動結案 + Discord 留痕。
#[tokio::test]
async fn kongko_refuses_to_close_money_question() {
    let ctx = TestCtx::new().await;
    let mut req = money_t0_req("pa-kg-money");
    req.kongko_ref = Some("batch-money".into());
    ctx.handle.create_question(req, now()).await.unwrap();

    let ev: kunsu_server::KongkoPublication = serde_json::from_value(json!({
        "type": "ad_publication_confirmed",
        "publicationId": "pub-money",
        "batchId": "batch-money",
        "ads": [],
    }))
    .unwrap();
    let out = ctx.handle.kongko_event(ev, now()).await.unwrap();
    assert!(out.closed_question_id.is_none(), "錢題不得被發佈事件結案");
    let q = ctx.handle.get_question("pa-kg-money").await.unwrap().unwrap();
    assert_eq!(q.state, State::Pending);
    assert!(q.answer.is_none());
    assert!(ctx.discord.messages().iter().any(|m| m.reason == "kongko_money_refused"));
}

// ---------------------------------------------------------------------------
// INV-16 inv_edited_params_rewrite_prepare_cmd（CONTRACT §6 規則 7）
// ---------------------------------------------------------------------------

/// edit_option 300→150：prepare cmd 目標數字同步改寫為 150。
#[tokio::test]
async fn inv_edited_params_rewrite_prepare_cmd() {
    let ctx = TestCtx::new().await;
    ctx.handle
        .create_question(editable_money_req("pa-edit", BUDGET_PREPARE_CMD), now())
        .await
        .unwrap();

    let (q, _redline) = ctx.handle.edit_option("pa-edit", "a", 150.0, now()).await.unwrap();
    let cmd = q.money.as_ref().unwrap().executor_prepare_cmd.as_deref().unwrap();
    assert!(cmd.contains("--new-budget 150"), "prepare cmd 必須改寫成編輯後值：{cmd}");
    assert!(!cmd.contains("360"), "舊值不得殘留：{cmd}");

    // 落 DB 持久化（重讀驗證）。
    let q = ctx.handle.get_question("pa-edit").await.unwrap().unwrap();
    assert!(
        q.money.unwrap().executor_prepare_cmd.unwrap().contains("--new-budget 150"),
        "改寫必須持久化"
    );
}

/// prepare-bid 型（--new-value）同樣被改寫。
#[tokio::test]
async fn edit_rewrites_prepare_bid_new_value() {
    let ctx = TestCtx::new().await;
    let cmd = "python3 projects/meta-ads/scripts/executor.py prepare-bid --entity 120000000000005 --new-value 360";
    ctx.handle.create_question(editable_money_req("pa-bid", cmd), now()).await.unwrap();
    let (q, _) = ctx.handle.edit_option("pa-bid", "a", 320.0, now()).await.unwrap();
    let cmd = q.money.unwrap().executor_prepare_cmd.unwrap();
    assert!(cmd.contains("--new-value 320"), "{cmd}");
}

/// 怪格式 prepare cmd 現在在建題就被白名單擋下（inv_prepare_cmd_whitelist）
/// ——連 DB 都進不去，比舊制「編輯時才升紅」更前面一道。
#[tokio::test]
async fn unrecognized_prepare_cmd_rejected_at_create() {
    let ctx = TestCtx::new().await;
    let weird = "python3 weird.py --amount 360";
    let err = ctx
        .handle
        .create_question(editable_money_req("pa-weird", weird), now())
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("inv_prepare_cmd_whitelist")
            || err.to_string().contains("白名單"),
        "{err}"
    );
    assert!(ctx.handle.get_question("pa-weird").await.unwrap().is_none(), "不落 DB");
}

/// answer 帶 edited_params 改金額：prepare cmd 同步改寫（ledger 快照即改寫後值）。
#[tokio::test]
async fn answer_edited_params_rewrites_prepare_cmd() {
    let ctx = TestCtx::new().await;
    ctx.handle
        .create_question(editable_money_req("pa-ans-edit", BUDGET_PREPARE_CMD), now())
        .await
        .unwrap();

    let input = AnswerInput {
        choice_id: Some("a".into()),
        free_text: None,
        edited_params: Some("adset-123 日預算 300 → 240".into()),
        channel: Channel::Ui,
        signature: None,
        revision: None,
        content_hash: None,
        items: None,
    };
    let q = ctx.handle.answer_question("pa-ans-edit", input, now()).await.unwrap();
    let cmd = q.money.as_ref().unwrap().executor_prepare_cmd.as_deref().unwrap();
    assert!(cmd.contains("--new-budget 240"), "{cmd}");
    assert_eq!(q.gate, Some(Gate::Yellow), "改寫成功不升紅");

    // ledger 的 answered 快照必須帶改寫後 cmd（所見即所簽的真相層證據）。
    let rows = read_ledger_rows(&ctx, now());
    let row = rows.iter().find(|r| r.question.id == "pa-ans-edit").unwrap();
    assert!(
        row.question
            .money
            .as_ref()
            .unwrap()
            .executor_prepare_cmd
            .as_deref()
            .unwrap()
            .contains("--new-budget 240"),
        "ledger 快照必須是改寫後值"
    );
}

/// answer 的 edited_params 解析不出金額 → 升 red 鎖一鍵（作答仍成立）。
#[tokio::test]
async fn unparsable_edited_params_locks_red_on_answer() {
    let ctx = TestCtx::new().await;
    ctx.handle
        .create_question(editable_money_req("pa-ans-bad", BUDGET_PREPARE_CMD), now())
        .await
        .unwrap();
    let input = AnswerInput {
        choice_id: Some("a".into()),
        free_text: None,
        edited_params: Some("全部暫停改用新素材".into()),
        channel: Channel::Ui,
        signature: None,
        revision: None,
        content_hash: None,
        items: None,
    };
    let q = ctx.handle.answer_question("pa-ans-bad", input, now()).await.unwrap();
    assert_eq!(q.state, State::Answered, "作答鎖 ≠ 執行鎖：答案仍落地");
    assert_eq!(q.gate, Some(Gate::Red), "解析不出金額必須鎖紅");
    assert!(q.tier_downgrade_reason.unwrap().contains("inv_edited_params_rewrite_prepare_cmd"));
}

// ---------------------------------------------------------------------------
// 統一強制詢問週期（舊 T1 批次窗已移除）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t1_uses_the_same_force_cycle_without_batch_window_side_effects() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("pa-cycle-t1"), now()).await.unwrap();
    let mut rx = ctx.handle.subscribe();

    ctx.handle.tick(now() + Duration::minutes(12)).await.unwrap();
    let question = ctx.handle.get_question("pa-cycle-t1").await.unwrap().unwrap();
    assert_eq!(question.state, State::Warned);
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| matches!(
        event,
        ServerEvent::Notify { kind: NotifyKind::WarnCountdown, .. }
    )));
    assert!(events.iter().all(|event| !matches!(event, ServerEvent::BatchWindow { .. })));
}

// ---------------------------------------------------------------------------
// 24h 無 ack 告警（CONTRACT §6 規則 9）
// ---------------------------------------------------------------------------

/// answered 且 24h 無 ack → Discord 一次；重複 tick 不重複告警。
#[tokio::test]
async fn no_ack_alert_after_24h_with_dedup() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("pa-noack"), now()).await.unwrap();
    ctx.handle.answer_question("pa-noack", ui_answer("a"), now()).await.unwrap();

    let alert_count = |msgs: &[kunsu_server::DiscordMessage]| {
        msgs.iter().filter(|m| m.reason == "no_ack_24h").count()
    };

    // 23h：未滿 24h，不告警。
    ctx.handle.tick(now() + Duration::hours(23)).await.unwrap();
    assert_eq!(alert_count(&ctx.discord.messages()), 0);

    // 25h：告警一次，文案可讀。
    ctx.handle.tick(now() + Duration::hours(25)).await.unwrap();
    let msgs = ctx.discord.messages();
    assert_eq!(alert_count(&msgs), 1);
    let alert = msgs.iter().find(|m| m.reason == "no_ack_24h").unwrap();
    assert!(alert.text.contains("測試題"), "{}", alert.text);
    assert!(alert.text.contains("24 小時沒有任何 agent 處理"), "{}", alert.text);

    // 重複抑制：後續 tick 不再告警。
    ctx.handle.tick(now() + Duration::hours(26)).await.unwrap();
    ctx.handle.tick(now() + Duration::hours(48)).await.unwrap();
    assert_eq!(alert_count(&ctx.discord.messages()), 1, "同題永不重複告警");
}

/// drain 過（acked_at 已標）的答案不告警。
#[tokio::test]
async fn acked_answer_never_alerts() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("pa-acked"), now()).await.unwrap();
    ctx.handle.answer_question("pa-acked", ui_answer("a"), now()).await.unwrap();
    // consumer drain → acked_at 落 DB。
    ctx.handle.answers_since("mkt-daily", None, now() + Duration::minutes(1)).await.unwrap();

    ctx.handle.tick(now() + Duration::hours(25)).await.unwrap();
    assert!(
        ctx.discord.messages().iter().all(|m| m.reason != "no_ack_24h"),
        "已 ack 的答案不得告警"
    );
}

// ---------------------------------------------------------------------------
// 疲勞降載（CONTRACT §6 規則 10）
// ---------------------------------------------------------------------------

/// 7 天窗同 (product, kind) 略過率超閾值（4/4 expired）→ 新題 tier 自動降一級。
#[tokio::test]
async fn fatigue_downgrade_after_skip_history() {
    let ctx = TestCtx::new().await;
    for i in 0..4 {
        ctx.handle.create_question(base_req(&format!("pa-fat-{i}")), now()).await.unwrap();
    }
    // T1 48h 未答 → 全部 expired（略過史）。
    let later = now() + Duration::hours(48);
    ctx.handle.tick(later).await.unwrap();
    for i in 0..4 {
        let q = ctx.handle.get_question(&format!("pa-fat-{i}")).await.unwrap().unwrap();
        assert_eq!(q.state, State::Expired);
    }

    // 同 (product=nova, kind=choice) 新題 → T1 降 T2 + 可讀原因。
    let out = ctx.handle.create_question(base_req("pa-fat-new"), later).await.unwrap();
    assert_eq!(out.tier, Tier::T2, "疲勞降載必須降一級");
    let reason = out.tier_downgrade_reason.clone().unwrap();
    assert!(reason.contains("fatigue_downgrade"), "{reason}");
    let q = ctx.handle.get_question("pa-fat-new").await.unwrap().unwrap();
    assert_eq!(q.requested_tier, Tier::T1, "requested_tier 保留供計分");
    assert_eq!(q.tier, Tier::T2);

    // Discord 承認週一次：第二題再降也只發一次。
    ctx.handle.create_question(base_req("pa-fat-new2"), later).await.unwrap();
    let n = ctx
        .discord
        .messages()
        .iter()
        .filter(|m| m.reason == "fatigue_downgrade")
        .count();
    assert_eq!(n, 1, "疲勞降載 Discord 承認每 7 天最多一次");
}

/// 資料不夠（n<4）不啟動疲勞降載。
#[tokio::test]
async fn fatigue_needs_minimum_sample() {
    let ctx = TestCtx::new().await;
    for i in 0..3 {
        ctx.handle.create_question(base_req(&format!("pa-few-{i}")), now()).await.unwrap();
    }
    let later = now() + Duration::hours(48);
    ctx.handle.tick(later).await.unwrap(); // 3 題全 expired，但 n=3 < 4
    let out = ctx.handle.create_question(base_req("pa-few-new"), later).await.unwrap();
    assert_eq!(out.tier, Tier::T1, "n<4 不降級");
    assert!(out.tier_downgrade_reason.is_none());
}

// ---------------------------------------------------------------------------
// ask_update 嚴格化（CONTRACT §6 規則 11）
// ---------------------------------------------------------------------------

/// UpdatePatch deny_unknown_fields：未知欄位（含 answer）→ 400 語義的 RPC error。
#[tokio::test]
async fn ask_update_rejects_unknown_fields() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();
    ctx.handle.create_question(base_req("pa-strict"), Utc::now()).await.unwrap();

    // 帶 answer 欄位（變相作答企圖）→ 拒。
    let out = rpc(
        &client,
        &base,
        rpc_call(1, "tools/call", json!({"name": "ask_update", "arguments": {
            "question_id": "pa-strict",
            "answer": {"choice_id": "a", "channel": "ui"},
        }})),
    )
    .await;
    assert_eq!(out["error"]["code"], -32602, "未知欄位必須 400 級拒收：{out}");
    assert!(out["error"]["message"].as_str().unwrap().contains("answer"), "{out}");
    let q = ctx.handle.get_question("pa-strict").await.unwrap().unwrap();
    assert!(q.answer.is_none(), "絕不因 ask_update 產生答案");

    // 打錯欄位名也大聲失敗，不靜默吞掉。
    let out = rpc(
        &client,
        &base,
        rpc_call(2, "tools/call", json!({"name": "ask_update", "arguments": {
            "question_id": "pa-strict",
            "body__md": "typo",
        }})),
    )
    .await;
    assert_eq!(out["error"]["code"], -32602);

    // 合法欄位照常。
    let out = rpc(
        &client,
        &base,
        rpc_call(3, "tools/call", json!({"name": "ask_update", "arguments": {
            "question_id": "pa-strict",
            "body_md": "更新內文",
        }})),
    )
    .await;
    assert!(out.get("error").is_none(), "{out}");
}

// ---------------------------------------------------------------------------
// /ingest/discord-answer（CONTRACT §6 規則 12）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ingest_discord_answer_endpoint() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();
    let ingest = format!("{base}/ingest/discord-answer");

    ctx.handle.create_question(base_req("pa-dc-plain"), now()).await.unwrap();
    ctx.handle.create_question(money_t0_req("pa-dc-money"), now()).await.unwrap();
    let mut red = base_req("pa-dc-red");
    red.gate = Some(Gate::Red);
    ctx.handle.create_question(red, now()).await.unwrap();

    // 無 auth → 401。
    let r = client
        .post(&ingest)
        .json(&json!({"question_id": "pa-dc-plain", "choice_id": "a", "discord_message_id": "m1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 非錢題：成功落 ledger、channel=discord。
    let r = client
        .post(&ingest)
        .bearer_auth(TEST_TOKEN)
        .json(&json!({"question_id": "pa-dc-plain", "choice_id": "a", "discord_message_id": "m1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["question"]["answer"]["channel"], "discord");
    let rows = read_ledger_rows(&ctx, Utc::now());
    let row = rows.iter().find(|r| r.question.id == "pa-dc-plain").expect("ledger 必有 answered 行");
    assert_eq!(row.ledger_type, kunsu_server::LedgerType::Answered);
    assert_eq!(row.answer.as_ref().unwrap().channel, Channel::Discord);

    // 錢題 → 400 拒絕（INV-15）。
    let r = client
        .post(&ingest)
        .bearer_auth(TEST_TOKEN)
        .json(&json!({"question_id": "pa-dc-money", "choice_id": "a", "discord_message_id": "m2"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["code"], "money_answer_ui_only");
    assert!(body["error"].as_str().unwrap().contains("錢題只能在 Kunsu app 或已配對的手機批准"), "{body}");
    let q = ctx.handle.get_question("pa-dc-money").await.unwrap().unwrap();
    assert_eq!(q.state, State::Pending);

    // 紅燈題（非錢）：允許——只落 ledger 不執行（語義同 UI 拍板）。
    let r = client
        .post(&ingest)
        .bearer_auth(TEST_TOKEN)
        .json(&json!({"question_id": "pa-dc-red", "free_text": "選 b，理由見討論串", "discord_message_id": "m3"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let q = ctx.handle.get_question("pa-dc-red").await.unwrap().unwrap();
    assert_eq!(q.state, State::Answered);
    assert!(q.answer.as_ref().unwrap().executed_at.is_none(), "紅燈只落 ledger 不執行");

    // 缺 discord_message_id / 缺作答內容 → 400。
    for bad in [
        json!({"question_id": "pa-dc-plain", "choice_id": "a"}),
        json!({"question_id": "pa-dc-plain", "discord_message_id": "m4"}),
        json!({"question_id": "pa-dc-plain", "choice_id": "a", "discord_message_id": ""}),
    ] {
        let r = client.post(&ingest).bearer_auth(TEST_TOKEN).json(&bad).send().await.unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST, "payload {bad} 必拒");
    }
}

#[tokio::test]
async fn ask_update_rewrites_copy_fields_but_rejects_money_options() {
    let ctx = common::TestCtx::new().await;
    let now = common::now();
    // 非錢題：title/question_line/options 全可重寫
    let req = common::base_req("pa-humanize-1");
    ctx.handle.create_question(req, now).await.unwrap();
    let patch = serde_json::from_value::<kunsu_server::service::UpdatePatch>(serde_json::json!({
        "title": "要你決定：Acme 要不要補一支對照素材",
        "question_line": "補對照素材？",
        "options": [
            {"id": "a", "label": "補（建議）", "recommended": true},
            {"id": "b", "label": "先不補，等月底"}
        ]
    }))
    .unwrap();
    let q = ctx.handle.update_question_meta("pa-humanize-1", patch, now).await.unwrap();
    assert_eq!(q.title, "要你決定：Acme 要不要補一支對照素材");
    assert_eq!(q.question_line, "補對照素材？");
    assert_eq!(q.options.len(), 2);
    // 超長 question_line 拒絕
    let bad = serde_json::from_value::<kunsu_server::service::UpdatePatch>(serde_json::json!({
        "question_line": "這一句故意寫得非常非常非常非常非常非常長超過二十四個字元的上限"
    }))
    .unwrap();
    assert!(ctx.handle.update_question_meta("pa-humanize-1", bad, now).await.is_err());
    // 錢題：options 重寫被拒（title 仍可）
    let mreq = common::money_t0_req("pa-humanize-money");
    ctx.handle.create_question(mreq, now).await.unwrap();
    let mpatch = serde_json::from_value::<kunsu_server::service::UpdatePatch>(serde_json::json!({
        "options": [
            {"id": "a", "label": "改個說法", "recommended": true},
            {"id": "b", "label": "維持現狀"}
        ]
    }))
    .unwrap();
    assert!(ctx.handle.update_question_meta("pa-humanize-money", mpatch, now).await.is_err());
}
