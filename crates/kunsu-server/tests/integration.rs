//! kunsu-server 整合測試（CONTRACT §11）：tokio + 臨時 DB + 臨時 vault。
//! INVARIANTS 屬 server 層的各配一條同名測試；approvals 兩條（single_use/ttl）
//! 依契約留給 HTTP 層階段。

mod common;

use chrono::Duration;
use common::*;
use kunsu_core::policy::Policy;
use kunsu_core::state_machine::NotifyKind;
use kunsu_core::types::*;
use kunsu_server::{AnswerInput, Dedup, KongkoPublication, ServerError, ServerEvent};
use serde_json::json;

// ---------------------------------------------------------------------------
// 建題 / 驗證 / upsert
// ---------------------------------------------------------------------------

/// AC-QL-01：合法建題回 created 並立即進 pending；OPEN.json 同步。
#[tokio::test]
async fn create_valid_question_enters_pending() {
    let ctx = TestCtx::new().await;
    let out = ctx.handle.create_question(base_req("pa-x1"), now()).await.unwrap();
    assert_eq!(out.dedup, Dedup::Created);
    assert_eq!(out.state, State::Pending);
    let q = ctx.handle.get_question("pa-x1").await.unwrap().unwrap();
    assert_eq!(q.state, State::Pending);
    assert_eq!(q.requested_tier, Tier::T1, "requested_tier 原樣保存供計分");
    let open = read_open_pending(&ctx);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, "pa-x1");
}

/// INV-1：錢類題缺 prediction_ref 必被 reject，不落 DB。
#[tokio::test]
async fn inv_money_requires_prediction() {
    let ctx = TestCtx::new().await;
    let mut req = money_t0_req("pa-m1");
    req.prediction_ref = None;
    let err = ctx.handle.create_question(req, now()).await.unwrap_err();
    assert!(matches!(err, ServerError::Reject(_)), "拿到 {err:?}");
    assert!(ctx.handle.get_question("pa-m1").await.unwrap().is_none(), "DB 不產生任何列");
}

/// INV-2：建題面 pick_option 被拒；過期面錢類永不自動選定（雙防線的 server 驗收）。
#[tokio::test]
async fn inv_money_never_auto_expire() {
    let ctx = TestCtx::new().await;
    // 建題面：schema 即擋。
    let mut req = money_t0_req("pa-m2");
    req.default_action = DefaultAction { on_expire: OnExpire::PickOption, option_id: Some("a".into()) };
    let err = ctx.handle.create_question(req, now()).await.unwrap_err();
    assert!(matches!(err, ServerError::Reject(_)));

    // 過期面：合法錢題（noop_log）超時 → expired、無自動答案、Discord 留痕。
    let mut req = money_t0_req("pa-m3");
    req.tier = Tier::T1; // 走 T1 直接等 deadline，不糾纏蓋板
    req.deadline_at = Some(now() + Duration::hours(1));
    ctx.handle.create_question(req, now()).await.unwrap();
    ctx.handle.tick(now() + Duration::hours(1)).await.unwrap();
    let q = ctx.handle.get_question("pa-m3").await.unwrap().unwrap();
    assert_eq!(q.state, State::Expired);
    assert!(q.answer.is_none(), "錢永不因超時自動動");
    let msgs = ctx.discord.messages();
    assert!(msgs.iter().any(|m| m.reason == "money_expired"), "錢類過期必 Discord 留痕：{msgs:?}");
    let rows = read_ledger_rows(&ctx, now() + Duration::hours(1));
    assert!(rows
        .iter()
        .any(|r| r.question.id == "pa-m3" && r.ledger_type == kunsu_server::LedgerType::Expired));
}

/// INV-11（server 面）：gate=red 必須 choice；紅燈 choice 合法通過。
#[tokio::test]
async fn inv_red_needs_options() {
    let ctx = TestCtx::new().await;
    let mut req = base_req("pa-r1");
    req.gate = Some(Gate::Red);
    req.kind = Kind::Confirm;
    req.options = vec![];
    let err = ctx.handle.create_question(req, now()).await.unwrap_err();
    assert!(matches!(err, ServerError::Reject(_)));

    let mut req = base_req("pa-r2");
    req.gate = Some(Gate::Red);
    let out = ctx.handle.create_question(req, now()).await.unwrap();
    assert_eq!(out.dedup, Dedup::Created);
}

/// INV-8 + P1 定案 8：同 id upsert 只 touch last_seen/證據；倒數不重置、
/// first_proposed（client 提供）不動、不發新通知。
#[tokio::test]
async fn inv_upsert_no_reset() {
    let ctx = TestCtx::new().await;
    let mut req = money_t0_req("pa-u1");
    let backlog_date = kunsu_core::time::taipei_date(now()) - Duration::days(3);
    req.first_proposed = Some(backlog_date); // P1 定案 8：積壓天數誠實
    ctx.handle.create_question(req, now()).await.unwrap();
    let before = ctx.handle.get_question("pa-u1").await.unwrap().unwrap();
    assert_eq!(before.first_proposed, backlog_date, "尊重 client first_proposed");

    let mut rx = ctx.handle.subscribe();
    let mut again = money_t0_req("pa-u1");
    again.evidence =
        vec![EvidenceLine { text: "新數據快照".into(), source: "meta-api".into() }];
    let later = now() + Duration::minutes(10);
    let out = ctx.handle.create_question(again, later).await.unwrap();
    assert_eq!(out.dedup, Dedup::TouchedExisting);

    let after = ctx.handle.get_question("pa-u1").await.unwrap().unwrap();
    assert_eq!(after.first_proposed, backlog_date, "first_proposed 不動");
    assert_eq!(after.next_transition_at, before.next_transition_at, "倒數不重置");
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.last_seen, later, "last_seen 更新");
    assert_eq!(after.evidence[0].text, "新數據快照", "證據允許刷新");
    assert_eq!(after.state, State::Pending);

    let events = drain_events(&mut rx);
    assert!(
        events.iter().all(|e| !matches!(
            e,
            ServerEvent::Notify { .. } | ServerEvent::QuestionCreated { .. }
        )),
        "upsert 不發任何新通知：{events:?}"
    );
    assert!(events.iter().any(|e| matches!(e, ServerEvent::QuestionTouched { .. })));
}

/// harness 場景 11：已答題同 id 再建 → 不重現、不重置，agent 讀得到答案。
#[tokio::test]
async fn answered_question_never_reopens() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("pa-done"), now()).await.unwrap();
    ctx.handle.answer_question("pa-done", ui_answer("a"), now() + Duration::minutes(1)).await.unwrap();

    let out = ctx
        .handle
        .create_question(base_req("pa-done"), now() + Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(out.dedup, Dedup::TouchedExisting);
    assert_eq!(out.state, State::Answered);
    assert_eq!(out.answer.as_ref().unwrap().choice_id.as_deref(), Some("a"));
    let q = ctx.handle.get_question("pa-done").await.unwrap().unwrap();
    assert_eq!(q.state, State::Answered, "不重置回 pending");
}

// ---------------------------------------------------------------------------
// T0 准入（P1 定案 2）與配額
// ---------------------------------------------------------------------------

/// AC-QL-07：不合格 T0 程式化降 T1 並回寫可讀原因；合格 T0 保持。
#[tokio::test]
async fn t0_admission_downgrade_and_pass() {
    let ctx = TestCtx::new().await;
    // 不合格：非錢非不可逆、缺 params/prediction/deadline。
    let mut bad = base_req("pa-t0-bad");
    bad.tier = Tier::T0;
    let out = ctx.handle.create_question(bad, now()).await.unwrap();
    assert_eq!(out.tier, Tier::T1, "降級生效值");
    let reason = out.tier_downgrade_reason.unwrap();
    assert!(reason.contains("params_line"), "原因可讀：{reason}");
    let q = ctx.handle.get_question("pa-t0-bad").await.unwrap().unwrap();
    assert_eq!(q.requested_tier, Tier::T0, "requested_tier 保留供計分");
    assert_eq!(q.tier, Tier::T1);

    // 合格：六條件全齊。
    let out = ctx.handle.create_question(money_t0_req("pa-t0-ok"), now()).await.unwrap();
    assert_eq!(out.tier, Tier::T0);
    assert!(out.tier_downgrade_reason.is_none());

    // 全部選項都標 recommended → 缺安全預設 → 降級。
    let mut no_safe = money_t0_req("pa-t0-nosafe");
    for o in &mut no_safe.options {
        o.recommended = Some(true);
    }
    let out = ctx.handle.create_question(no_safe, now()).await.unwrap();
    assert_eq!(out.tier, Tier::T1);
    assert!(out.tier_downgrade_reason.unwrap().contains("安全預設"));
}

/// 強制詢問不再有每日題數預算；多題各自保有週期。
#[tokio::test]
async fn force_cycles_have_no_daily_budget() {
    let ctx = TestCtx::new().await;
    let out = ctx.handle.create_question(money_t0_req("pa-b1"), now()).await.unwrap();
    assert_eq!(out.tier, Tier::T0);
    let out = ctx.handle.create_question(money_t0_req("pa-b2"), now()).await.unwrap();
    assert_eq!(out.tier, Tier::T0);
    for id in ["pa-b1", "pa-b2"] {
        let q = ctx.handle.get_question(id).await.unwrap().unwrap();
        assert_eq!(q.force_at, Some(now() + Duration::minutes(15)));
    }
}

/// AC-AF-04：per-source 日配額；touch 不計入；internal 不計入；異 source 隔離。
#[tokio::test]
async fn per_source_daily_quota() {
    let mut policy = Policy::default();
    policy.quota.per_source_daily_create = 3;
    let ctx = TestCtx::with_policy(policy).await;
    for i in 0..3 {
        ctx.handle.create_question(base_req(&format!("pa-q{i}")), now()).await.unwrap();
    }
    let err = ctx.handle.create_question(base_req("pa-q4"), now()).await.unwrap_err();
    assert!(matches!(err, ServerError::QuotaExceeded { limit: 3, .. }), "拿到 {err:?}");
    // 同 id upsert touch 不計入配額。
    let out = ctx.handle.create_question(base_req("pa-q0"), now()).await.unwrap();
    assert_eq!(out.dedup, Dedup::TouchedExisting);
    // internal 不佔配額。
    let mut doc = base_req("pa-doctor");
    doc.internal = true;
    ctx.handle.create_question(doc, now()).await.unwrap();
    // 不同 source（routine）獨立配額。
    let mut other = base_req("pa-other");
    other.source.routine_id = Some("another-routine".into());
    let out = ctx.handle.create_question(other, now()).await.unwrap();
    assert_eq!(out.dedup, Dedup::Created);
}

// ---------------------------------------------------------------------------
// 作答全鏈（answered → ledger 行 → OPEN 更新 → 回執）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn answer_full_chain() {
    let ctx = TestCtx::new().await;
    let mut req = base_req("pa-a1");
    req.wake_on_answer =
        Wake { mode: WakeMode::Fresh, session_id: None, playbook: Some("playbooks/x.md".into()) };
    ctx.handle.create_question(req, now()).await.unwrap();

    let mut rx = ctx.handle.subscribe();
    let at = now() + Duration::minutes(5);
    let q = ctx.handle.answer_question("pa-a1", ui_answer("a"), at).await.unwrap();
    assert_eq!(q.state, State::Answered);
    let ans = q.answer.clone().unwrap();
    assert_eq!(ans.answer_id.len(), 36, "uuid v4 冪等鍵");
    assert_eq!(ans.channel, Channel::Ui);
    assert_eq!(ans.answered_at, at);

    // ledger 行：type=answered、全量快照＋答案。
    let rows = read_ledger_rows(&ctx, at);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].ledger_type, kunsu_server::LedgerType::Answered);
    assert_eq!(rows[0].question.id, "pa-a1");
    assert_eq!(rows[0].answer.as_ref().unwrap().answer_id, ans.answer_id);
    // OPEN.json 更新：不再 pending。
    assert!(read_open_pending(&ctx).is_empty());
    // reactor 入列（wake=fresh）。
    let events = drain_events(&mut rx);
    assert!(
        events.iter().any(|e| matches!(
            e,
            ServerEvent::ReactorEnqueued { dry_run: false, .. }
        )),
        "{events:?}"
    );

    // 回執回寫：executed_at + execution ledger 行（type 白名單第六值）。
    let done_at = at + Duration::minutes(3);
    ctx.handle.record_execution("pa-a1", "budget updated 500→600", done_at).await.unwrap();
    let q = ctx.handle.get_question("pa-a1").await.unwrap().unwrap();
    let ans = q.answer.unwrap();
    assert_eq!(ans.executed_at, Some(done_at));
    assert_eq!(ans.execution_result.as_deref(), Some("budget updated 500→600"));
    let rows = read_ledger_rows(&ctx, done_at);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].ledger_type, kunsu_server::LedgerType::Execution);
}

/// AC-AN-13：作答競態只承認第一個；答案不被覆寫、ledger 只記一次。
#[tokio::test]
async fn competing_answers_first_wins() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("pa-c1"), now()).await.unwrap();
    ctx.handle.answer_question("pa-c1", ui_answer("a"), now()).await.unwrap();
    let second = AnswerInput {
        choice_id: Some("b".into()),
        free_text: None,
        edited_params: None,
        channel: Channel::Discord,
        signature: None,
        revision: None,
        content_hash: None,
        items: None,
    };
    let err = ctx.handle.answer_question("pa-c1", second, now()).await.unwrap_err();
    assert!(matches!(err, ServerError::AlreadyAnswered { .. }));
    let q = ctx.handle.get_question("pa-c1").await.unwrap().unwrap();
    assert_eq!(q.answer.as_ref().unwrap().choice_id.as_deref(), Some("a"), "不覆寫");
    assert_eq!(q.answer.as_ref().unwrap().channel, Channel::Ui);
    assert_eq!(read_ledger_rows(&ctx, now()).len(), 1, "ledger 只記一次 answered");
}

/// AC-QL-14：cancel/supersede 留軌跡不刪列。
#[tokio::test]
async fn cancel_and_supersede_keep_trace() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("pa-k1"), now()).await.unwrap();
    ctx.handle.create_question(base_req("pa-k2"), now()).await.unwrap();
    ctx.handle.cancel_question("pa-k1", now()).await.unwrap();
    ctx.handle.supersede_question("pa-k2", now()).await.unwrap();
    assert_eq!(ctx.handle.get_question("pa-k1").await.unwrap().unwrap().state, State::Cancelled);
    assert_eq!(ctx.handle.get_question("pa-k2").await.unwrap().unwrap().state, State::Superseded);
    let rows = read_ledger_rows(&ctx, now());
    assert_eq!(rows.len(), 2);
}

// ---------------------------------------------------------------------------
// 過期預設（AC-QL-12/13）
// ---------------------------------------------------------------------------

/// 非錢題 pick_option 到期自動選定：channel=auto_policy；帶 standing_policy_ref
/// 的題標 24h 否決窗（P1 定案 3）。
#[tokio::test]
async fn expiry_pick_option_records_auto_policy() {
    let ctx = TestCtx::new().await;
    let mut req = base_req("pa-e1");
    req.default_action =
        DefaultAction { on_expire: OnExpire::PickOption, option_id: Some("b".into()) };
    req.deadline_at = Some(now() + Duration::hours(2));
    req.standing_policy_ref = Some("logs/asks/policies/keep-budget.md".into());
    ctx.handle.create_question(req, now()).await.unwrap();

    let at = now() + Duration::hours(2);
    ctx.handle.tick(at).await.unwrap();
    let q = ctx.handle.get_question("pa-e1").await.unwrap().unwrap();
    assert_eq!(q.state, State::Expired);
    let ans = q.answer.unwrap();
    assert_eq!(ans.channel, Channel::AutoPolicy);
    assert_eq!(ans.choice_id.as_deref(), Some("b"));
    assert_eq!(q.veto_until, Some(at + Duration::hours(24)), "24h 否決窗");
}

/// AC-QL-13：T1 題 48h 未答走宣告預設（noop_log 只留痕）。
#[tokio::test]
async fn t1_expires_after_48h() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("pa-t1"), now()).await.unwrap();
    ctx.handle.tick(now() + Duration::hours(48)).await.unwrap();
    let q = ctx.handle.get_question("pa-t1").await.unwrap().unwrap();
    assert_eq!(q.state, State::Expired);
    assert!(q.answer.is_none());
}

/// INV-14：pending 壽命 ≤14 天。T2 → superseded；其他 tier → expired。
#[tokio::test]
async fn inv_max_pending_age() {
    let ctx = TestCtx::new().await;
    let mut t2 = base_req("pa-t2");
    t2.tier = Tier::T2;
    let mut t3 = base_req("pa-t3");
    t3.tier = Tier::T3;
    ctx.handle.create_question(t2, now()).await.unwrap();
    ctx.handle.create_question(t3, now()).await.unwrap();
    ctx.handle.tick(now() + Duration::days(14)).await.unwrap();
    assert_eq!(ctx.handle.get_question("pa-t2").await.unwrap().unwrap().state, State::Superseded);
    assert_eq!(ctx.handle.get_question("pa-t3").await.unwrap().unwrap().state, State::Expired);
    assert!(read_open_pending(&ctx).is_empty(), "到期必轉終態，積壓 30 天機制上不可能");
}

// ---------------------------------------------------------------------------
// 強制鏈 / guards / snooze / kill-switch（INV-3/4/5）
// ---------------------------------------------------------------------------

/// INV-3：pending→(12min)→warned→(15min)→forced 全鏈。
#[tokio::test]
async fn inv_forced_gated() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(money_t0_req("pa-f1"), now()).await.unwrap();
    let mut rx = ctx.handle.subscribe();

    let warn_at = now() + Duration::minutes(12);
    ctx.handle.tick(warn_at).await.unwrap();
    let q = ctx.handle.get_question("pa-f1").await.unwrap().unwrap();
    assert_eq!(q.state, State::Warned);
    assert!(!q.forced_today_used, "legacy daily budget is no longer consumed");
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|e| matches!(
        e,
        ServerEvent::Notify { kind: NotifyKind::WarnCountdown, .. }
    )));

    let force_at = now() + Duration::minutes(15);
    ctx.handle.tick(force_at).await.unwrap();
    let q = ctx.handle.get_question("pa-f1").await.unwrap().unwrap();
    assert_eq!(q.state, State::Forced);
    let health = ctx.handle.health(force_at).await.unwrap();
    assert_eq!(health.overdue_count, 1);
}

/// INV-5：guards 不擋 warning，只 defer forced；解除後立即蓋板。
#[tokio::test]
async fn inv_guards_block_force() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(money_t0_req("pa-g1"), now()).await.unwrap();
    ctx.guards.set(kunsu_server::GuardSnapshot { camera_on: true, ..Default::default() });
    let warn_at = now() + Duration::minutes(12);
    ctx.handle.tick(warn_at).await.unwrap();
    assert_eq!(
        ctx.handle.get_question("pa-g1").await.unwrap().unwrap().state,
        State::Warned,
        "warning 不受 guard 阻擋"
    );
    ctx.handle.tick(now() + Duration::minutes(15)).await.unwrap();
    assert_eq!(ctx.handle.get_question("pa-g1").await.unwrap().unwrap().state, State::Warned);
    ctx.guards.set(kunsu_server::GuardSnapshot::default());
    ctx.handle.tick(now() + Duration::minutes(15) + Duration::seconds(1)).await.unwrap();
    assert_eq!(ctx.handle.get_question("pa-g1").await.unwrap().unwrap().state, State::Forced);
}

/// kill-switch：4h 免打擾 + fatigue event 落盤；窗過後恢復。
#[tokio::test]
async fn kill_switch_blocks_and_logs_fatigue() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(money_t0_req("pa-ks"), now()).await.unwrap();
    let until = ctx.handle.kill_switch(4, now()).await.unwrap();
    assert_eq!(until, now() + Duration::hours(4));
    ctx.handle.tick(now() + Duration::minutes(12)).await.unwrap();
    assert_eq!(ctx.handle.get_question("pa-ks").await.unwrap().unwrap().state, State::Warned);
    ctx.handle.tick(now() + Duration::minutes(15)).await.unwrap();
    assert_eq!(ctx.handle.get_question("pa-ks").await.unwrap().unwrap().state, State::Warned);
    ctx.handle.tick(now() + Duration::hours(4) + Duration::seconds(1)).await.unwrap();
    assert_eq!(ctx.handle.get_question("pa-ks").await.unwrap().unwrap().state, State::Forced);
}

/// INV-17 / AC-DND-01..07：勿擾持久化、24h 邊界與題目週期隔離。
#[tokio::test]
async fn inv_dnd_is_silent_and_bounded() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(money_t0_req("pa-dnd"), now()).await.unwrap();
    let before = ctx.handle.get_question("pa-dnd").await.unwrap().unwrap();

    assert!(ctx.handle.set_dnd(0, "ui", now()).await.is_err());
    assert!(ctx.handle.set_dnd(1441, "ui", now()).await.is_err());
    let status = ctx.handle.set_dnd(1440, "ui", now()).await.unwrap();
    assert!(status.active);
    assert_eq!(status.ends_at, Some(now() + Duration::hours(24)));
    assert_eq!(status.remaining_seconds, 86_400);

    ctx.handle.tick(now() + Duration::minutes(12)).await.unwrap();
    ctx.handle.tick(now() + Duration::minutes(15)).await.unwrap();
    let during = ctx.handle.get_question("pa-dnd").await.unwrap().unwrap();
    assert_eq!(during.state, State::Warned);
    assert_eq!(during.cycle_started_at, before.cycle_started_at);
    assert_eq!(during.warn_at, before.warn_at);
    assert_eq!(during.force_at, before.force_at);
    assert_eq!(during.cycle_number, before.cycle_number);
    assert!(during.answer.is_none());

    let reopened = ctx.reopen().await;
    let restored = reopened.dnd_status(now() + Duration::hours(1)).await.unwrap();
    assert!(restored.active, "restart 必須保留勿擾 ends_at");
    assert_eq!(restored.ends_at, status.ends_at);

    let cleared = reopened.clear_dnd("ui", now() + Duration::hours(1)).await.unwrap();
    assert!(!cleared.active);
    let cleared_again = reopened.clear_dnd("ui", now() + Duration::hours(1)).await.unwrap();
    assert!(!cleared_again.active, "clear 必須冪等");
    reopened.tick(now() + Duration::hours(1)).await.unwrap();
    assert_eq!(reopened.get_question("pa-dnd").await.unwrap().unwrap().state, State::Forced);
}

/// AC-DND-08：plugin/MCP 可以讀、設、清；tool surface 仍沒有 answer。
#[tokio::test]
async fn dnd_mcp_read_set_clear() {
    let ctx = TestCtx::new().await;
    let specs = kunsu_server::mcp::tool_specs();
    assert_eq!(specs.len(), 14);
    for name in ["dnd_get", "dnd_set", "dnd_clear"] {
        assert!(specs.iter().any(|spec| spec["name"] == name));
    }
    assert!(!specs.iter().any(|spec| spec["name"] == "answer"));

    async fn call(ctx: &TestCtx, name: &str, arguments: serde_json::Value) -> serde_json::Value {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        });
        match kunsu_server::mcp::handle_rpc(&ctx.handle, body).await {
            kunsu_server::mcp::McpReply::Json(value) => value["result"]["structuredContent"].clone(),
            kunsu_server::mcp::McpReply::Accepted => panic!("tools/call 不得回 Accepted"),
        }
    }

    let set = call(&ctx, "dnd_set", serde_json::json!({"duration_minutes": 60, "actor": "codex"})).await;
    assert_eq!(set["active"], true);
    assert_eq!(set["remaining_seconds"], 3600);
    let get = call(&ctx, "dnd_get", serde_json::json!({})).await;
    assert_eq!(get["active"], true);
    assert_eq!(get["actor"], "codex");
    let clear = call(&ctx, "dnd_clear", serde_json::json!({"actor": "codex"})).await;
    assert_eq!(clear["active"], false);
}

/// INV-4：每次未回答 dismiss 都重啟完整 15 分鐘，無上限也不降級。
#[tokio::test]
async fn inv_dismiss_restarts_without_cap() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(money_t0_req("pa-s1"), now()).await.unwrap();
    let mut cycle_start = now();
    for cycle in 1..=5u32 {
        ctx.handle.tick(cycle_start + Duration::minutes(12)).await.unwrap();
        ctx.handle.tick(cycle_start + Duration::minutes(15)).await.unwrap();
        assert_eq!(ctx.handle.get_question("pa-s1").await.unwrap().unwrap().state, State::Forced);
        let dismissed_at = cycle_start + Duration::minutes(15) + Duration::seconds(1);
        let q = ctx.handle.dismiss_question("pa-s1", dismissed_at).await.unwrap();
        assert_eq!(q.state, State::Pending);
        assert_eq!(q.cycle_number, cycle + 1);
        assert_eq!(q.warn_at, Some(dismissed_at + Duration::minutes(12)));
        assert_eq!(q.force_at, Some(dismissed_at + Duration::minutes(15)));
        assert_eq!(q.tier, Tier::T0);
        cycle_start = dismissed_at;
    }
    assert!(ctx.discord.messages().iter().all(|m| m.reason != "snooze_cap_downgrade"));
}

/// AC-QL-15：重啟後 timer 從 DB `next_transition_at` 重建，due 轉移補觸發。
#[tokio::test]
async fn timer_rebuilds_from_db_after_restart() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(money_t0_req("pa-restart"), now()).await.unwrap();
    let handle2 = ctx.reopen().await; // 模擬 app 被殺重啟
    handle2.tick(now() + Duration::minutes(12)).await.unwrap();
    assert_eq!(
        handle2.get_question("pa-restart").await.unwrap().unwrap().state,
        State::Warned,
        "排程由 DB 重建、不靠記憶體 timer"
    );
}

/// P1 定案 10：internal 題不進強制週期、不推 Discord、reactor 只 dry-run、
/// ledger 行標 internal。
#[tokio::test]
async fn internal_question_isolation() {
    let ctx = TestCtx::new().await;
    let mut req = money_t0_req("pa-doc");
    req.internal = true;
    req.wake_on_answer =
        Wake { mode: WakeMode::Fresh, session_id: None, playbook: Some("playbooks/x.md".into()) };
    ctx.handle.create_question(req, now()).await.unwrap();
    ctx.handle.tick(now() + Duration::minutes(31)).await.unwrap();
    let internal = ctx.handle.get_question("pa-doc").await.unwrap().unwrap();
    assert_eq!(internal.state, State::Pending);
    assert_eq!(internal.force_at, None);
    let health = ctx.handle.health(now() + Duration::minutes(31)).await.unwrap();
    assert_eq!(health.overdue_count, 0);
    assert_eq!(health.pending_count, 0, "internal 不進統計");

    let mut rx = ctx.handle.subscribe();
    ctx.handle.answer_question("pa-doc", ui_answer("b"), now() + Duration::hours(1)).await.unwrap();
    let rows = read_ledger_rows(&ctx, now() + Duration::hours(1));
    assert!(rows[0].internal, "ledger 行標 internal 供統計排除");
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|e| matches!(e, ServerEvent::ReactorEnqueued { dry_run: true, .. })));
    assert!(ctx.discord.messages().is_empty(), "internal 不推 Discord");
}

// ---------------------------------------------------------------------------
// kongko（INV-12 + P1 定案 5）
// ---------------------------------------------------------------------------

fn publication(pub_id: &str, batch: &str) -> KongkoPublication {
    serde_json::from_value(json!({
        "type": "ad_publication_confirmed",
        "publicationId": pub_id,
        "batchId": batch,
        "product": "nova",
        "ads": [
            {"campaign_id": "120200000000001", "daily_budget_twd": 300},
            {"campaign_id": "120200000000001", "daily_budget_twd": 500},
        ],
        "confirmedAt": "2026-07-31T06:30:00Z"
    }))
    .unwrap()
}

/// INV-12：發佈事件結案素材題（channel=kongko）→ 必生成獨立 T0 啟用題；
/// 發佈≠啟用（啟用題保持 pending 等第二次物理點擊）；重複事件冪等。
#[tokio::test]
async fn inv_publish_not_activate() {
    let ctx = TestCtx::new().await;
    let mut req = base_req("pa-kg1");
    req.kind = Kind::KongkoReview;
    req.options = vec![];
    req.kongko_ref = Some("batch-123".into());
    req.deep_link = Some("http://localhost:3200/batches/batch-123".into());
    ctx.handle.create_question(req, now()).await.unwrap();

    let at = now() + Duration::minutes(30);
    let out = ctx.handle.kongko_event(publication("pub-1", "batch-123"), at).await.unwrap();
    assert!(!out.duplicate);
    assert_eq!(out.closed_question_id.as_deref(), Some("pa-kg1"));
    let act_id = out.activation_question_id.clone().unwrap();

    // 素材題：answered、channel=kongko、落 ledger。
    let material = ctx.handle.get_question("pa-kg1").await.unwrap().unwrap();
    assert_eq!(material.state, State::Answered);
    assert_eq!(material.answer.as_ref().unwrap().channel, Channel::Kongko);

    // 啟用題：獨立 id、T0、is_money_op、params/prediction 齊備、pending。
    let act = ctx.handle.get_question(&act_id).await.unwrap().unwrap();
    assert_ne!(act.id, material.id);
    assert_eq!(act.state, State::Pending, "發佈≠啟用：等使用者第二次點擊");
    assert_eq!(act.tier, Tier::T0);
    assert!(act.is_money_op());
    assert_eq!(act.money.as_ref().unwrap().amount_twd, Some(800));
    assert_eq!(act.gate, Some(Gate::Yellow), "NT$800 ≤ 2,000 → 黃燈");
    assert!(act.params_line.is_some());
    assert!(act.prediction_ref.is_some());
    // Codex 覆檢 P1：啟用題必須進完整錢鏈——由 publication 結構化資料組出
    // 白名單 prepare 操作（campaign entity + 現行預算）。
    let money = act.money.as_ref().unwrap();
    assert_eq!(money.entity_id.as_deref(), Some("120200000000001"));
    let cmd = money.executor_prepare_cmd.as_deref().expect("啟用題必帶白名單 prepare cmd");
    let op = kunsu_core::prepare_cmd::parse_prepare_cmd(cmd).expect("cmd 必須過白名單");
    assert_eq!(op.entity, "120200000000001");
    assert_eq!(op.target, 800);
    assert_eq!(op.product.as_deref(), Some("nova"));
    // 所見即所簽：params_line 的目標值 == prepare 的 --new-budget。
    let to = kunsu_core::redline::parse_money_change(act.params_line.as_deref().unwrap())
        .expect("params_line 必須含金額變更")
        .to;
    assert_eq!(to as i64, 800);

    // 冪等：同 publicationId 重送 → no-op，不重複結案、不重複生成。
    let n_before = ctx.handle.list_questions(true).await.unwrap().len();
    let rows_before = read_ledger_rows(&ctx, at).len();
    let out2 = ctx.handle.kongko_event(publication("pub-1", "batch-123"), at).await.unwrap();
    assert!(out2.duplicate);
    assert_eq!(ctx.handle.list_questions(true).await.unwrap().len(), n_before);
    assert_eq!(read_ledger_rows(&ctx, at).len(), rows_before);

    // 同 batch 不同 publicationId：素材題已終態、啟用題 kongko_ref=None → 不誤結。
    let out3 = ctx.handle.kongko_event(publication("pub-2", "batch-123"), at).await.unwrap();
    assert!(!out3.duplicate);
    assert!(out3.closed_question_id.is_none());
    assert!(out3.activation_question_id.is_none());
}

/// kongko 場景 5/6：無匹配題留痕不報錯；壞 payload 拒收。
#[tokio::test]
async fn kongko_unmatched_and_bad_payload() {
    let ctx = TestCtx::new().await;
    let out = ctx.handle.kongko_event(publication("pub-x", "batch-none"), now()).await.unwrap();
    assert!(!out.duplicate);
    assert!(out.closed_question_id.is_none());

    let mut bad = publication("pub-y", "batch-1");
    bad.event_type = "something_else".into();
    let err = ctx.handle.kongko_event(bad, now()).await.unwrap_err();
    assert!(matches!(err, ServerError::BadKongkoEvent(_)));
}

// ---------------------------------------------------------------------------
// 游標收割（at-least-once）
// ---------------------------------------------------------------------------

/// AC-HR-03/04：per-consumer 游標、同 cursor 重收同批、多 consumer 隔離、
/// drain 到即標 acked_at。
#[tokio::test]
async fn cursor_harvest_at_least_once() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("pa-h1"), now()).await.unwrap();
    ctx.handle.create_question(base_req("pa-h2"), now()).await.unwrap();
    ctx.handle.answer_question("pa-h1", ui_answer("a"), now()).await.unwrap();
    ctx.handle.cancel_question("pa-h2", now()).await.unwrap();

    let drain_at = now() + Duration::minutes(1);
    let batch = ctx.handle.answers_since("mkt-daily", None, drain_at).await.unwrap();
    assert_eq!(batch.events.len(), 2);
    assert_eq!(batch.events[0].terminal_type, "answered");
    assert_eq!(batch.events[1].terminal_type, "cancelled");
    let c1 = batch.next_cursor;
    // drain 到即標 acked_at（三態回執第二態）。
    assert_eq!(batch.events[0].question.answer.as_ref().unwrap().acked_at, Some(drain_at));

    // crash 場景：cursor 未推進，重呼叫重收同批——寧可重複、絕不丟失。
    let again = ctx.handle.answers_since("mkt-daily", None, drain_at).await.unwrap();
    assert_eq!(again.events.len(), 2, "at-least-once 重放");

    // 以 next_cursor 確認 → 空批。
    let confirmed = ctx.handle.answers_since("mkt-daily", Some(c1), drain_at).await.unwrap();
    assert!(confirmed.events.is_empty());
    // 之後不帶 cursor 沿用存檔游標 → 仍空。
    let after = ctx.handle.answers_since("mkt-daily", None, drain_at).await.unwrap();
    assert!(after.events.is_empty());

    // 另一 consumer 游標獨立，從頭收割。
    let other = ctx.handle.answers_since("ceo-weekly", None, drain_at).await.unwrap();
    assert_eq!(other.events.len(), 2);
}

// ---------------------------------------------------------------------------
// ledger（INV-6 / INV-13）
// ---------------------------------------------------------------------------

/// INV-6：任何寫入路徑不修改既有行——後寫只 append，前綴位元組不變。
#[tokio::test]
async fn inv_ledger_append_only() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("pa-l1"), now()).await.unwrap();
    ctx.handle.create_question(base_req("pa-l2"), now()).await.unwrap();
    ctx.handle.answer_question("pa-l1", ui_answer("a"), now()).await.unwrap();
    let path = ctx.handle.ledger().day_file(now());
    let bytes_1 = std::fs::read(&path).unwrap();

    ctx.handle.cancel_question("pa-l2", now()).await.unwrap();
    let bytes_2 = std::fs::read(&path).unwrap();
    assert!(bytes_2.len() > bytes_1.len());
    assert_eq!(&bytes_2[..bytes_1.len()], &bytes_1[..], "既有位元組永不被修改");

    ctx.handle.record_execution("pa-l1", "done", now()).await.unwrap();
    let bytes_3 = std::fs::read(&path).unwrap();
    assert_eq!(&bytes_3[..bytes_2.len()], &bytes_2[..]);
}

/// INV-13：SQLite 整顆換新後由 ledger + OPEN.json 完整重建；
/// 游標重置只造成 at-least-once 重放。
#[tokio::test]
async fn inv_rebuild_from_ledger() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("pa-rb1"), now()).await.unwrap();
    ctx.handle.create_question(base_req("pa-rb2"), now()).await.unwrap();
    ctx.handle.create_question(money_t0_req("pa-rb3"), now()).await.unwrap();
    let aid = ctx.handle.answer_question("pa-rb1", ui_answer("a"), now() + Duration::minutes(1)).await.unwrap().answer.unwrap().answer_id;
    // 交付狀態也要能重建（等待者已取走）。
    ctx.handle
        .set_delivery("pa-rb1", &aid, kunsu_core::continuity::DeliveryState::LiveWaiter, None, None, now() + Duration::minutes(1))
        .await
        .unwrap();
    ctx.handle.cancel_question("pa-rb2", now() + Duration::minutes(2)).await.unwrap();
    // pa-rb3 留 pending。

    let snap1 = ctx.handle.get_question("pa-rb1").await.unwrap().unwrap();
    let snap2 = ctx.handle.get_question("pa-rb2").await.unwrap().unwrap();
    let snap3 = ctx.handle.get_question("pa-rb3").await.unwrap().unwrap();

    // 全新空 DB（模擬整顆刪除）→ rebuild。
    let (handle2, _db) = ctx.fresh_db_same_vault().await;
    let report = handle2.rebuild_db().await.unwrap();
    assert_eq!(report.terminal_rows, 2);
    assert_eq!(report.pending_restored, 1);
    assert!(report.bad_lines.is_empty());

    assert_eq!(handle2.get_question("pa-rb1").await.unwrap().unwrap(), snap1, "含交付狀態");
    assert_eq!(snap1.answer.as_ref().and_then(|a| a.delivery.as_ref()).map(|d| d.state), Some(kunsu_core::continuity::DeliveryState::LiveWaiter));
    assert_eq!(handle2.get_question("pa-rb2").await.unwrap().unwrap(), snap2);
    assert_eq!(handle2.get_question("pa-rb3").await.unwrap().unwrap(), snap3, "pending 含倒數完整恢復");

    // 游標重置 → 終態事件可重新收割（消化端以 answer_id 冪等吸收）。
    let batch = handle2.answers_since("mkt-daily", None, now() + Duration::hours(1)).await.unwrap();
    assert_eq!(batch.events.len(), 2);

    // 帳本日檔不見了（例如 vault 搬家），OPEN.json 還在：資料庫裡已回答／已取消的題在帳本與 OPEN.json 都找不到
    // → 拒絕重建，資料庫保持原樣。
    let before = handle2.get_question("pa-rb1").await.unwrap().unwrap();
    for entry in std::fs::read_dir(ctx.handle.ledger().asks_dir()).unwrap().flatten() {
        if entry.path().extension().is_some_and(|e| e == "jsonl") {
            std::fs::remove_file(entry.path()).unwrap();
        }
    }
    let refused = handle2.rebuild_db().await;
    assert!(matches!(refused, Err(kunsu_server::ServerError::RebuildRefused(_))), "{refused:?}");
    assert_eq!(handle2.get_question("pa-rb1").await.unwrap().unwrap(), before, "拒絕重建時資料庫保持原樣");
}

// ---------------------------------------------------------------------------
// health / board_pack
// ---------------------------------------------------------------------------

/// P1 定案 4：/health 資料面含 oldest_pending_days（心跳行需要）。
#[tokio::test]
async fn health_reports_oldest_pending_days() {
    let ctx = TestCtx::new().await;
    let mut req = base_req("pa-old");
    req.first_proposed = Some(kunsu_core::time::taipei_date(now()) - Duration::days(3));
    ctx.handle.create_question(req, now()).await.unwrap();
    let health = ctx.handle.health(now()).await.unwrap();
    assert_eq!(health.pending_count, 1);
    assert_eq!(health.oldest_pending_days, 3);
    assert_eq!(health.overdue_count, 0);
    assert!(health.last_answer_at.is_none());

    ctx.handle.answer_question("pa-old", ui_answer("a"), now()).await.unwrap();
    let health = ctx.handle.health(now()).await.unwrap();
    assert_eq!(health.pending_count, 0);
    assert!(health.last_answer_at.is_some());
}

/// board_pack：T0 與批次清單分開；紅燈/錢優先、資歷久優先。
#[tokio::test]
async fn board_pack_partitions_and_sorts() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("pa-bp1"), now()).await.unwrap();
    let mut red = base_req("pa-bp2");
    red.gate = Some(Gate::Red);
    ctx.handle.create_question(red, now()).await.unwrap();
    ctx.handle.create_question(money_t0_req("pa-bp3"), now()).await.unwrap();

    let pack = ctx.handle.board_pack(now()).await.unwrap();
    assert_eq!(pack.t0.len(), 1);
    assert_eq!(pack.t0[0].id, "pa-bp3");
    assert_eq!(pack.batch.len(), 2);
    assert_eq!(pack.batch[0].id, "pa-bp2", "紅燈優先");
}

/// AC-EN-12：瀏海 hover 的跑馬燈只在沒有隱私 guard（分享螢幕、攝影機、麥克風、前景全螢幕）時播題目內容；
/// 勿擾不算隱私 guard（勿擾時瀏海本來就隱藏）。
#[tokio::test]
async fn privacy_guard_covers_sharing_camera_mic_and_fullscreen() {
    let ctx = TestCtx::new().await;
    assert!(!ctx.handle.privacy_guard_active());
    for snap in [
        kunsu_server::GuardSnapshot { screen_shared: true, ..Default::default() },
        kunsu_server::GuardSnapshot { camera_on: true, ..Default::default() },
        kunsu_server::GuardSnapshot { mic_on: true, ..Default::default() },
        kunsu_server::GuardSnapshot { frontmost_fullscreen: true, ..Default::default() },
    ] {
        ctx.guards.set(snap);
        assert!(ctx.handle.privacy_guard_active(), "{snap:?}");
    }
    ctx.guards.set(kunsu_server::GuardSnapshot { dnd: true, ..Default::default() });
    assert!(!ctx.handle.privacy_guard_active());
}

// ---------------------------------------------------------------------------
// 排程督導（BDD routine-check-in AC-RC-01／09）
// ---------------------------------------------------------------------------

/// AC-RC-01：有 routine_id 的題 trigger 預設 scheduled；沒有的預設 interactive。
#[tokio::test]
async fn routine_trigger_defaults() {
    use kunsu_core::continuity::Trigger;
    let ctx = TestCtx::new().await;
    let mut req = base_req("rt-diary-20261008");
    req.source.routine_id = Some("diary".into());
    req.source.routine_name = Some("每天寫日記".into());
    req.source.expected_every_hours = Some(24.0);
    ctx.handle.create_question(req, now()).await.unwrap();
    let mut plain = base_req("pa-plain");
    plain.source.routine_id = None;
    ctx.handle.create_question(plain, now()).await.unwrap();
    let rt = ctx.handle.get_question("rt-diary-20261008").await.unwrap().unwrap();
    assert_eq!(rt.source.trigger, Some(Trigger::Scheduled));
    let plain = ctx.handle.get_question("pa-plain").await.unwrap().unwrap();
    assert_eq!(plain.source.trigger, Some(Trigger::Interactive));
}

/// AC-RC-09：SQLite 整顆重建後，排程身分、回答與 AI 回饋都還原。
#[tokio::test]
async fn routine_history_survives_rebuild() {
    let ctx = TestCtx::new().await;
    let mut req = base_req("rt-diary-20261007");
    req.source.routine_id = Some("diary".into());
    req.source.routine_name = Some("每天寫日記".into());
    req.source.expected_every_hours = Some(24.0);
    ctx.handle.create_question(req, now()).await.unwrap();
    let q = ctx.handle.answer_question("rt-diary-20261007", ui_answer("a"), now() + Duration::minutes(1)).await.unwrap();
    let answer_id = q.answer.unwrap().answer_id;
    ctx.handle
        .report_answer("rt-diary-20261007", &answer_id, kunsu_core::continuity::ReportStatus::Done, "已寫入日記", now() + Duration::minutes(5))
        .await
        .unwrap();
    let before = ctx.handle.get_question("rt-diary-20261007").await.unwrap().unwrap();

    let (handle2, _db) = ctx.fresh_db_same_vault().await;
    handle2.rebuild_db().await.unwrap();
    let after = handle2.get_question("rt-diary-20261007").await.unwrap().unwrap();
    assert_eq!(after.source.routine_id.as_deref(), Some("diary"));
    assert_eq!(after.source.routine_name.as_deref(), Some("每天寫日記"));
    assert_eq!(after.answer.as_ref().and_then(|a| a.report.as_ref()).map(|r| r.summary_md.as_str()), Some("已寫入日記"));
    assert_eq!(after, before, "重建後督導頁看到的內容一致");
}

/// AC-RC-01：Kunsu 自己的題（`kunsu-` 保留前綴，含內部建題路徑的 Kongko 啟用題）一律 interactive；agent 自填的 trigger 優先。
#[tokio::test]
async fn routine_trigger_reserved_prefix_and_agent_choice() {
    use kunsu_core::continuity::Trigger;
    let ctx = TestCtx::new().await;
    let mut doctor = base_req("pa-doctor-smoke");
    doctor.source.routine_id = Some("kunsu-doctor".into());
    ctx.handle.create_question(doctor, now()).await.unwrap();
    let mut chosen = base_req("rt-chosen");
    chosen.source.routine_id = Some("weekly".into());
    chosen.source.trigger = Some(Trigger::Interactive);
    ctx.handle.create_question(chosen, now()).await.unwrap();
    let mut req = base_req("pa-kg1");
    req.kind = Kind::KongkoReview;
    req.options = vec![];
    req.kongko_ref = Some("batch-123".into());
    req.deep_link = Some("http://localhost:3200/batches/batch-123".into());
    ctx.handle.create_question(req, now()).await.unwrap();
    let out = ctx.handle.kongko_event(publication("pub-1", "batch-123"), now() + Duration::minutes(30)).await.unwrap();

    let trigger = |q: Question| q.source.trigger;
    let act = ctx.handle.get_question(&out.activation_question_id.unwrap()).await.unwrap().unwrap();
    assert_eq!(act.source.routine_id.as_deref(), Some("kunsu-kongko"));
    assert_eq!(trigger(act), Some(Trigger::Interactive), "Kongko 啟用題");
    assert_eq!(trigger(ctx.handle.get_question("pa-doctor-smoke").await.unwrap().unwrap()), Some(Trigger::Interactive));
    assert_eq!(trigger(ctx.handle.get_question("rt-chosen").await.unwrap().unwrap()), Some(Trigger::Interactive));
}

/// AC-RC-01b：宣告頻率必須是有限正數，否則 400 bad_routine_cadence。
#[tokio::test]
async fn routine_cadence_must_be_positive() {
    let ctx = TestCtx::new().await;
    for (i, every) in [-24.0_f64, 0.0, f64::NAN, f64::INFINITY].into_iter().enumerate() {
        let mut req = base_req(&format!("rt-bad-{i}"));
        req.source.routine_id = Some("bad".into());
        req.source.expected_every_hours = Some(every);
        let err = ctx.handle.create_question(req, now()).await.unwrap_err();
        assert!(
            matches!(&err, ServerError::Reject(r) if r.code() == "bad_routine_cadence"),
            "expected_every_hours={every} 應被拒：{err:?}"
        );
    }
    let mut ok = base_req("rt-ok");
    ok.source.expected_every_hours = Some(1.5);
    ctx.handle.create_question(ok, now()).await.unwrap();
}

/// AC-RC-04／09：回報與交付各有帳本行，最後一行帶 picked_up；相同回報重送不多寫帳本；新內容取代舊回饋。
#[tokio::test]
async fn routine_report_ledger_rows_carry_delivery() {
    use kunsu_core::continuity::{DeliveryState, ReportStatus};
    let ctx = TestCtx::new().await;
    let mut r = base_req("rt-diary-20261008");
    r.source.routine_id = Some("diary".into());
    // 排程 check-in 由 `kunsu ask` 自動帶上原對話身分（回答延續原對話才有 picked_up）。
    r.source.conversation_id = Some("conv-diary".into());
    ctx.handle.create_question(r, now()).await.unwrap();
    let aid = ctx.handle.answer_question("rt-diary-20261008", ui_answer("a"), now()).await.unwrap().answer.unwrap().answer_id;
    ctx.handle.report_answer("rt-diary-20261008", &aid, ReportStatus::Done, "已寫入", now()).await.unwrap();
    let (rows, _) = ctx.handle.ledger().read_all_rows().unwrap();
    let n = rows.len();
    let last = rows.iter().rev().find(|r| r.question.id == "rt-diary-20261008").unwrap();
    assert_eq!(last.ledger_type.as_str(), "delivery");
    assert_eq!(last.question.answer.as_ref().and_then(|a| a.delivery.as_ref()).map(|d| d.state), Some(DeliveryState::PickedUp));
    assert!(last.question.answer.as_ref().and_then(|a| a.report.as_ref()).is_some(), "交付行的快照也帶回饋");
    ctx.handle.report_answer("rt-diary-20261008", &aid, ReportStatus::Done, "已寫入", now()).await.unwrap();
    assert_eq!(ctx.handle.ledger().read_all_rows().unwrap().0.len(), n, "相同回報重送冪等");
    ctx.handle.report_answer("rt-diary-20261008", &aid, ReportStatus::Done, "更新：已寫入並加標籤", now()).await.unwrap();
    let q = ctx.handle.get_question("rt-diary-20261008").await.unwrap().unwrap();
    assert_eq!(q.answer.unwrap().report.unwrap().summary_md, "更新：已寫入並加標籤");
}

fn checkin(id: &str) -> kunsu_core::types::CreateRequest {
    let mut r = base_req(id);
    r.source.routine_id = Some("diary".into());
    r.source.routine_name = Some("每天寫日記".into());
    r.source.expected_every_hours = Some(24.0);
    r
}

/// AC-RC-09：排程收割前後與重建後，沒回答的 check-in 仍分得出未回應／已取消。
#[tokio::test]
async fn routine_drain_and_rebuild_keep_missed_and_cancelled() {
    let ctx = TestCtx::new().await;
    for id in ["rt-diary-20261006", "rt-diary-20261007"] {
        ctx.handle.create_question(checkin(id), now()).await.unwrap();
    }
    ctx.handle.cancel_question("rt-diary-20261007", now() + Duration::hours(1)).await.unwrap();
    ctx.handle.tick(now() + Duration::hours(49)).await.unwrap();
    let batch = ctx.handle.answers_since("lane1", None, now() + Duration::hours(50)).await.unwrap();
    assert_eq!(batch.events.len(), 2, "事件照常回傳");
    let states = |mut v: Vec<Question>| {
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v.into_iter().map(|q| q.state).collect::<Vec<_>>()
    };
    assert_eq!(states(ctx.handle.list_questions(true).await.unwrap()), vec![State::Expired, State::Cancelled]);
    let (handle2, _db) = ctx.fresh_db_same_vault().await;
    handle2.rebuild_db().await.unwrap();
    assert_eq!(states(handle2.list_questions(true).await.unwrap()), vec![State::Expired, State::Cancelled]);
}

/// AC-RC-01：`kunsu-` 前綴即使 agent 自填 scheduled 也是 interactive。
#[tokio::test]
async fn routine_reserved_prefix_overrides_agent_trigger() {
    use kunsu_core::continuity::Trigger;
    let ctx = TestCtx::new().await;
    let mut r = base_req("pa-kunsu-x");
    r.source.routine_id = Some("kunsu-x".into());
    r.source.trigger = Some(Trigger::Scheduled);
    ctx.handle.create_question(r, now()).await.unwrap();
    let q = ctx.handle.get_question("pa-kunsu-x").await.unwrap().unwrap();
    assert_eq!(q.source.trigger, Some(Trigger::Interactive));
}

/// AC-RC-01b：MCP 建題參數型別錯誤時，錯誤訊息點出欄位路徑。
#[tokio::test]
async fn mcp_create_type_errors_name_the_field() {
    use kunsu_server::mcp::{handle_rpc, McpReply};
    let ctx = TestCtx::new().await;
    let mut args = serde_json::to_value(checkin("rt-type")).unwrap();
    args["source"]["expected_every_hours"] = json!("24h");
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "ask_create", "arguments": args}});
    let McpReply::Json(v) = handle_rpc(&ctx.handle, body).await else { panic!("要回 JSON") };
    let msg = v["error"]["message"].as_str().unwrap_or_default().to_string();
    assert!(msg.contains("source.expected_every_hours"), "{v}");
}

/// AC-RC-01b（QA 督導第 3 輪）：草稿註冊帶進來的 source 型別錯時，錯誤點出欄位。
#[tokio::test]
async fn register_source_type_error_names_the_field() {
    use kunsu_server::mcp::{handle_rpc, McpReply};
    use kunsu_server::DraftPaths;
    let ctx = TestCtx::new().await;
    let home = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(home.path().join("drafts")).unwrap();
    ctx.handle.set_draft_paths(DraftPaths { drafts_root: home.path().join("drafts"), revisions_root: home.path().join("revisions") });
    let id = "rt-draft-type";
    let dir = home.path().join("drafts").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("question.md"),
        format!("---\nid: {id}\ntitle: 今天的日記\nquestion_line: 今天最重要的三件事？\nkind: text\ntier: t1\n---\n寫下今天最重要的三件事。"),
    )
    .unwrap();
    let source = json!({"agent": "claude", "cwd": "/tmp", "routine_id": "diary", "routine_name": "日記", "expected_every_hours": "24h"});
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "ask_register", "arguments": {"draft_dir": dir, "source": source}}});
    let McpReply::Json(v) = handle_rpc(&ctx.handle, body).await else { panic!("要回 JSON") };
    assert!(v.to_string().contains("expected_every_hours"), "錯誤沒點出欄位：{v}");
}

/// AC-QL-11（QA 督導第 3 輪）：給 agent 讀的 /describe 與規格一致——只有已回答的題收割後變 routed。
#[tokio::test]
async fn describe_matches_routed_rule() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let v: serde_json::Value =
        reqwest::Client::new().get(format!("{base}/describe")).bearer_auth(TEST_TOKEN).send().await.unwrap().json().await.unwrap();
    let routed = v["state_machine"]["transitions"].as_array().unwrap().iter().find(|t| t["to"] == "routed").cloned().unwrap();
    assert_eq!(routed["from"], "answered", "{routed}");
}
