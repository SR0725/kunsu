//! HTTP/協定層整合測試（CONTRACT §5 + §4）：
//! auth 拒絕、/health 免 auth、kongko 冪等經 HTTP、/approvals/verify 的
//! inv_approval_single_use / inv_approval_ttl / 竄改拒絕、
//! MCP initialize→initialized→tools/list→ask_create→ask_get 全鏈、
//! ask_wait 逾時與作答回流、inv_no_answer_via_mcp。

mod common;

use chrono::Utc;
use common::*;
use serde_json::{json, Value};

async fn rpc(
    client: &reqwest::Client,
    base: &str,
    body: Value,
) -> (reqwest::StatusCode, Option<Value>) {
    let resp = client
        .post(format!("{base}/mcp"))
        .bearer_auth(TEST_TOKEN)
        .header("Accept", "application/json, text/event-stream")
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let text = resp.text().await.unwrap();
    let v = if text.trim().is_empty() { None } else { Some(serde_json::from_str(&text).unwrap()) };
    (status, v)
}

fn rpc_call(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

// ---------------------------------------------------------------------------
// auth / health / describe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_is_open_and_everything_else_needs_bearer() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();

    // /health 免 auth，含 P1 定案 4 的 oldest_pending_days
    let h: Value = client
        .get(format!("{base}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    for key in ["version", "uptime_s", "pending_count", "overdue_count", "oldest_pending_days"] {
        assert!(h.get(key).is_some(), "/health 缺欄位 {key}：{h}");
    }

    // 其他路由：無 token 401、錯 token 401、對 token 200
    let r = client.get(format!("{base}/describe")).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);
    let r = client
        .get(format!("{base}/describe"))
        .bearer_auth("0000000000000000000000000000dead")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);
    let r = client
        .get(format!("{base}/describe"))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let d: Value = r.json().await.unwrap();
    assert_eq!(d["tools"].as_array().unwrap().len(), 14, "/describe 應含 14 個 tool schema（含 ask_report、ask_register、ask_followup_reply）");
    assert!(d["state_machine"]["transitions"].is_array());
    assert_eq!(d["policy"]["force"]["cycle_min"], 15);
    assert_eq!(d["policy"]["force"]["warn_before_min"], 3);

    // MCP 面也吃同一道 auth
    let r = client
        .post(format!("{base}/mcp"))
        .json(&rpc_call(1, "tools/list", json!({})))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// kongko 經 HTTP：冪等（P1 定案 5）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn kongko_event_is_idempotent_over_http() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();

    let mut req = base_req("pa-batch-mat");
    req.kongko_ref = Some("batch-9".into());
    ctx.handle.create_question(req, Utc::now()).await.unwrap();

    let payload = json!({
        "type": "ad_publication_confirmed",
        "publicationId": "pub-1",
        "batchId": "batch-9",
        "product": "nova",
        "ads": [{"daily_budget_twd": 700}],
        "confirmedAt": "2026-07-31T06:00:00Z",
    });

    // 無 auth → 401
    let r = client.post(format!("{base}/events/kongko")).json(&payload).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);

    let first: Value = client
        .post(format!("{base}/events/kongko"))
        .bearer_auth(TEST_TOKEN)
        .json(&payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first["duplicate"], false);
    assert_eq!(first["closed_question_id"], "pa-batch-mat");
    assert_eq!(first["activation_question_id"], "pa-activate-batch-9", "發佈≠啟用：自動建獨立啟用題");

    // 同 publicationId 重複投遞：冪等吸收，不重複結案、不重複生成啟用題
    let second: Value = client
        .post(format!("{base}/events/kongko"))
        .bearer_auth(TEST_TOKEN)
        .json(&payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(second["duplicate"], true);
    assert!(second["closed_question_id"].is_null());

    // 非法 payload → 400 可讀原因
    let r = client
        .post(format!("{base}/events/kongko"))
        .bearer_auth(TEST_TOKEN)
        .json(&json!({"type": "unknown_event"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// /approvals/verify：INV-9 / INV-10 / 竄改拒絕
// ---------------------------------------------------------------------------

async fn verify(client: &reqwest::Client, base: &str, record: &Value) -> Value {
    client
        .post(format!("{base}/approvals/verify"))
        .bearer_auth(TEST_TOKEN)
        .json(&json!({"approval_record": record}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// 建一題已作答的錢類題（record 綁定的 question 必須存在於活體 DB）。
async fn answered_money_question(ctx: &TestCtx, id: &str) -> chrono::DateTime<Utc> {
    let now = Utc::now();
    let mut req = base_req(id);
    req.money = Some(kunsu_core::types::Money {
        is_money_op: true,
        amount_twd: Some(600),
        entity_id: Some(TEST_ENTITY.into()),
        // 非紅燈錢題必帶 prepare 命令（CONTRACT §2，QA continuity 第 10 輪）。
        executor_prepare_cmd: Some(whitelisted_prepare_cmd(TEST_ENTITY, 600, "nova")),
    });
    req.prediction_ref = Some("projects/meta-ads/predictions/2026-07-31-t.md".into());
    req.params_line = Some("adset-1 500 → 600".into());
    ctx.handle.create_question(req, now).await.unwrap();
    ctx.handle.answer_question(id, ui_answer("a"), now).await.unwrap();
    now
}

#[tokio::test]
async fn inv_approval_single_use() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();
    let now = answered_money_question(&ctx, "pa-money-1").await;

    let record = signed_record("pa-money-1", "tok16", "hash-a", now.timestamp(), "nonce-1");
    issue_record(&ctx, "pa-money-1", &record).await;
    let first = verify(&client, &base, &record).await;
    assert_eq!(first["valid"], true, "首次 verify 必須通過：{first}");
    assert_eq!(first["consumed_now"], true);

    // 同一 record 第二次必拒（單次消耗）
    let second = verify(&client, &base, &record).await;
    assert_eq!(second["valid"], false);
    let reason = second["reason"].as_str().unwrap();
    assert!(reason.contains("單次") || reason.contains("已被使用"), "reason 要講明單次消耗：{reason}");

    // 消耗記錄落 ledger（真相層留痕：type=approval_consumed）
    let rows = read_ledger_rows(&ctx, now);
    assert!(
        rows.iter().any(|r| r.ledger_type == kunsu_server::LedgerType::ApprovalConsumed),
        "ledger 應有 approval_consumed 行"
    );

    // INV-9（2026-10-09 錢 QA 第 11 輪）：Kunsu 已記下結果的授權（例如 commit 沒驗證就失敗 → EXEC_BLOCKED）事後花不掉。
    let blocked_q = "pa-money-blocked";
    let t = answered_money_question(&ctx, blocked_q).await;
    let blocked = signed_record(blocked_q, "tok16", "hash-a", t.timestamp(), "nonce-blocked");
    issue_record(&ctx, blocked_q, &blocked).await;
    let settled = ctx
        .handle
        .settle_unverified_nonce(blocked_q, "nonce-blocked", "EXEC_BLOCKED: executor commit 失敗：沒有驗證就結束", Utc::now())
        .await
        .unwrap();
    assert!(settled, "還沒被消耗的授權可以記成被擋");
    let late = verify(&client, &base, &blocked).await;
    assert_eq!(late["valid"], false, "已記成被擋的授權事後驗證必拒：{late}");
    assert!(late["reason"].as_str().unwrap_or_default().contains("結果"), "reason 要講明這次授權已有結果：{late}");

    // 被更新的簽發取代（重新授權後）的舊授權：驗證必拒；最新的那份照常通過。
    let reauth_q = "pa-money-superseded";
    let t = answered_money_question(&ctx, reauth_q).await;
    let old = signed_record(reauth_q, "tok16", "hash-a", t.timestamp(), "nonce-old");
    issue_record(&ctx, reauth_q, &old).await;
    let new = signed_record(reauth_q, "tok16", "hash-a", t.timestamp(), "nonce-new");
    issue_record(&ctx, reauth_q, &new).await;
    let stale = verify(&client, &base, &old).await;
    assert_eq!(stale["valid"], false, "被更新簽發取代的授權必拒：{stale}");
    assert!(stale["reason"].as_str().unwrap_or_default().contains("取代"), "reason 要講明已被取代：{stale}");
    let fresh = verify(&client, &base, &new).await;
    assert_eq!(fresh["valid"], true, "最新的授權照常通過：{fresh}");

    // 互斥：已被消耗的授權不能再記成被擋（呼叫端改記結果不明）。
    let again = ctx.handle.settle_unverified_nonce(reauth_q, "nonce-new", "EXEC_BLOCKED: x", Utc::now()).await.unwrap();
    assert!(!again, "已被 executor 消耗的授權不能記成被擋");
}

#[tokio::test]
async fn inv_approval_ttl() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();
    let now = answered_money_question(&ctx, "pa-money-ttl").await;

    // answered_at 距今 16 分鐘：> 900s 必拒
    let stale =
        signed_record("pa-money-ttl", "tok16", "hash-a", now.timestamp() - 960, "nonce-ttl");
    issue_record(&ctx, "pa-money-ttl", &stale).await;
    let out = verify(&client, &base, &stale).await;
    assert_eq!(out["valid"], false);
    assert!(out["reason"].as_str().unwrap().contains("TTL"), "reason 要講明 TTL：{out}");

    // 邊界內（10 分鐘前）通過
    let fresh =
        signed_record("pa-money-ttl", "tok16", "hash-a", now.timestamp() - 600, "nonce-ttl2");
    issue_record(&ctx, "pa-money-ttl", &fresh).await;
    let out = verify(&client, &base, &fresh).await;
    assert_eq!(out["valid"], true);
}

#[tokio::test]
async fn approval_tampering_is_rejected() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();
    let now = answered_money_question(&ctx, "pa-money-tamper").await;

    let record = signed_record("pa-money-tamper", "tok16", "hash-a", now.timestamp(), "nonce-t");
    issue_record(&ctx, "pa-money-tamper", &record).await;

    // param_hash 竄改：簽章覆蓋 param_hash → 驗章必敗
    let mut tampered = record.clone();
    tampered["param_hash"] = json!("hash-b");
    let out = verify(&client, &base, &tampered).await;
    assert_eq!(out["valid"], false);
    assert!(out["reason"].as_str().unwrap().contains("驗章"), "{out}");

    // hmac 本身竄改
    let mut bad_sig = record.clone();
    bad_sig["hmac"] = json!("deadbeef");
    let out = verify(&client, &base, &bad_sig).await;
    assert_eq!(out["valid"], false);

    // 缺欄位要回明確 reason
    let mut missing = record.clone();
    missing.as_object_mut().unwrap().remove("nonce");
    let out = verify(&client, &base, &missing).await;
    assert_eq!(out["valid"], false);
    assert!(out["reason"].as_str().unwrap().contains("nonce"), "{out}");

    // 全程沒消耗成功過 → 原 record 現在仍可通過（證明拒絕路徑不誤消耗）
    let out = verify(&client, &base, &record).await;
    assert_eq!(out["valid"], true);
}

// ---------------------------------------------------------------------------
// MCP：initialize → initialized → tools/list → ask_create → ask_get 全鏈
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mcp_full_chain_matches_migrate_backlog_client() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();

    // initialize：回 protocolVersion / capabilities / serverInfo + session header
    let resp = client
        .post(format!("{base}/mcp"))
        .bearer_auth(TEST_TOKEN)
        .header("Accept", "application/json, text/event-stream")
        .json(&rpc_call(1, "initialize", json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "migrate_backlog", "version": "1.0"},
        })))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert!(resp.headers().get("Mcp-Session-Id").is_some(), "initialize 要帶 session id header");
    let init: Value = resp.json().await.unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "kunsu");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    // notifications/initialized（無 id 的 notification）→ 202
    let (status, _) =
        rpc(&client, &base, json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
    assert_eq!(status, reqwest::StatusCode::ACCEPTED);

    // tools/list → 14 個 tool（含 ask_report、ask_register、ask_followup_reply）
    let (_, out) = rpc(&client, &base, rpc_call(2, "tools/list", json!({}))).await;
    let tools = out.unwrap()["result"]["tools"].as_array().unwrap().clone();
    assert_eq!(tools.len(), 14); // 2026-10-12 加 ask_followup_reply

    // ask_create：migrate_backlog.py 的 payload 形狀（requested_tier、無 tier）
    let ask = json!({
        "id": "pa-backlog-1",
        "title": "積壓分診題",
        "question_line": "要照提案執行嗎？",
        "body_md": "**積壓遷移**",
        "kind": "choice",
        "requested_tier": "t2",
        "product": "account",
        "gate": "yellow",
        "evidence": [{"text": "原始提案", "source": "pending-approvals.json"}],
        "options": [
            {"id": "approve", "label": "照提案執行", "detail": "走 gate"},
            {"id": "reject", "label": "否決封存", "detail": "不再重提"},
            {"id": "discuss", "label": "轉討論", "detail": "開互動 session"},
        ],
        "allow_free_text": true,
        "default_action": {"on_expire": "noop_log"},
        "wake_on_answer": {"mode": "none"},
        "first_proposed": "2026-07-10",
        "source": {"agent": "claude", "cwd": "/Users/you/notes",
                    "routine_id": "backlog-migration-2026-07-31"},
    });
    let (_, out) =
        rpc(&client, &base, rpc_call(3, "tools/call", json!({"name": "ask_create", "arguments": ask}))).await;
    let out = out.unwrap();
    assert!(out.get("error").is_none(), "ask_create 不應回錯：{out}");
    let sc = &out["result"]["structuredContent"];
    assert_eq!(sc["question_id"], "pa-backlog-1");
    assert_eq!(sc["state"], "pending");
    assert_eq!(sc["dedup"], "created");
    assert_eq!(sc["tier"], "t2");

    // ask_get：state + first_proposed 誠實沿用 client 值（P1 定案 8）
    let (_, out) = rpc(
        &client,
        &base,
        rpc_call(4, "tools/call", json!({"name": "ask_get", "arguments": {"question_id": "pa-backlog-1"}})),
    )
    .await;
    let q = &out.unwrap()["result"]["structuredContent"]["question"];
    assert_eq!(q["state"], "pending");
    assert_eq!(q["requested_tier"], "t2");
    assert_eq!(q["first_proposed"], "2026-07-10");

    // 同 id 重建 → touched_existing（upsert 冪等，migrate 重跑安全）
    let ask2 = json!({
        "id": "pa-backlog-1", "title": "積壓分診題", "question_line": "要照提案執行嗎？",
        "kind": "choice", "requested_tier": "t2",
        "options": [
            {"id": "approve", "label": "照提案執行"},
            {"id": "reject", "label": "否決封存"},
        ],
        "default_action": {"on_expire": "noop_log"},
        "source": {"agent": "claude", "cwd": "/x", "routine_id": "backlog-migration-2026-07-31"},
    });
    let (_, out) =
        rpc(&client, &base, rpc_call(5, "tools/call", json!({"name": "ask_create", "arguments": ask2}))).await;
    assert_eq!(out.unwrap()["result"]["structuredContent"]["dedup"], "touched_existing");

    // 驗證錯誤是 JSON-RPC error（agent 可讀原因）：錢類題缺 prediction_ref
    let bad = json!({
        "id": "pa-bad-money", "title": "t", "question_line": "q", "kind": "choice",
        "tier": "t1",
        "options": [{"id": "a", "label": "a"}, {"id": "b", "label": "b"}],
        "money": {"is_money_op": true},
        "params_line": "x 1 → 2",
        "default_action": {"on_expire": "noop_log"},
        "source": {"agent": "claude", "cwd": "/x"},
    });
    let (_, out) =
        rpc(&client, &base, rpc_call(6, "tools/call", json!({"name": "ask_create", "arguments": bad}))).await;
    let err = out.unwrap();
    let msg = err["error"]["message"].as_str().unwrap_or_default().to_string();
    assert!(msg.contains("prediction"), "錢類缺 prediction_ref 要有可讀原因：{msg}");
}

#[tokio::test]
async fn mcp_ask_wait_timeout_answer_and_cap() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();

    ctx.handle.create_question(base_req("pa-wait"), Utc::now()).await.unwrap();

    // 短 timeout：pending_timeout（可無狀態續等）
    let (_, out) = rpc(
        &client,
        &base,
        rpc_call(1, "tools/call", json!({"name": "ask_wait", "arguments": {"question_id": "pa-wait", "timeout_seconds": 1}})),
    )
    .await;
    assert_eq!(out.unwrap()["result"]["structuredContent"]["status"], "pending_timeout");

    // timeout > 540 → 400 語義的 JSON-RPC error（P1 定案 9）
    let (_, out) = rpc(
        &client,
        &base,
        rpc_call(2, "tools/call", json!({"name": "ask_wait", "arguments": {"question_id": "pa-wait", "timeout_seconds": 600}})),
    )
    .await;
    let err = out.unwrap();
    assert_eq!(err["error"]["code"], -32602);
    assert!(err["error"]["message"].as_str().unwrap().contains("540"));

    // 掛著等 → 作答當下立即回 answered + answer（不等 timeout 走完）
    let handle = ctx.handle.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        handle.answer_question("pa-wait", ui_answer("a"), Utc::now()).await.unwrap();
    });
    let started = std::time::Instant::now();
    let (_, out) = rpc(
        &client,
        &base,
        rpc_call(3, "tools/call", json!({"name": "ask_wait", "arguments": {"question_id": "pa-wait", "timeout_seconds": 60}})),
    )
    .await;
    let sc = out.unwrap()["result"]["structuredContent"].clone();
    assert_eq!(sc["status"], "answered");
    assert_eq!(sc["answer"]["choice_id"], "a");
    assert!(started.elapsed() < std::time::Duration::from_secs(30), "應在作答當下返回，不是等滿 timeout");
}

/// INV-7 inv_no_answer_via_mcp：tools/list 枚舉一遍證明無作答面 +
/// 對不存在 tool 的呼叫被拒。
#[tokio::test]
async fn inv_no_answer_via_mcp() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();

    let (_, out) = rpc(&client, &base, rpc_call(1, "tools/list", json!({}))).await;
    let tools = out.unwrap()["result"]["tools"].as_array().unwrap().clone();
    let mut names: Vec<String> =
        tools.iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "ask_answers_since",
            "ask_cancel",
            "ask_create",
            "ask_followup_reply",
            "ask_get",
            "ask_list",
            "ask_register",
            "ask_report",
            "ask_supersede",
            "ask_update",
            "ask_wait",
            "dnd_clear",
            "dnd_get",
            "dnd_set",
        ],
        "MCP 面必須恰好是 11 個 ask 與 3 個 dnd tool——ask_report 只記錄執行結果、ask_register 只建題、ask_followup_reply 只轉文字（INV-31），不存在任何作答/批准寫入"
    );
    // 讀取面（ask_answers_since 是收割已發生的答案）之外，無任何名字帶寫入答案語義
    for banned in ["answer", "approve", "sign"] {
        assert!(
            !names.iter().any(|n| n.contains(banned) && n != "ask_answers_since"),
            "tool 名不得含 {banned}"
        );
    }

    // 對作答類/不存在 tool 的呼叫一律拒絕
    for tool in ["answer_question", "ask_answer", "approve", "kunsu_answer"] {
        let (_, out) = rpc(
            &client,
            &base,
            rpc_call(2, "tools/call", json!({"name": tool, "arguments": {"question_id": "pa-x", "choice_id": "a"}})),
        )
        .await;
        let err = out.unwrap();
        assert!(err.get("error").is_some(), "tools/call {tool} 必須被拒");
        assert_eq!(err["error"]["code"], -32602);
    }

    // schema 檢查：8 個 inputSchema 的頂層屬性沒有任何作答欄位
    for t in &tools {
        if let Some(props) = t["inputSchema"]["properties"].as_object() {
            for banned in ["choice_id", "free_text", "edited_params", "signature", "answer"] {
                assert!(
                    !props.contains_key(banned),
                    "{} schema 不得收作答欄位 {banned}",
                    t["name"]
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MCP 其餘工具面：list / update / cancel / supersede / answers_since
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mcp_list_update_cancel_supersede_answers_since() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();
    let now = Utc::now();

    ctx.handle.create_question(base_req("pa-a"), now).await.unwrap();
    ctx.handle.create_question(base_req("pa-b"), now).await.unwrap();
    ctx.handle.create_question(base_req("pa-c"), now).await.unwrap();

    // ask_update：補充證據不重置倒數
    let before = ctx.handle.get_question("pa-a").await.unwrap().unwrap();
    let (_, out) = rpc(
        &client,
        &base,
        rpc_call(1, "tools/call", json!({"name": "ask_update", "arguments": {
            "question_id": "pa-a",
            "evidence": [{"text": "新數據", "source": "stripe"}],
            "body_md": "更新內文",
        }})),
    )
    .await;
    assert!(out.unwrap().get("error").is_none());
    let after = ctx.handle.get_question("pa-a").await.unwrap().unwrap();
    assert_eq!(after.evidence[0].source, "stripe");
    assert_eq!(after.body_md, "更新內文");
    assert_eq!(after.next_transition_at, before.next_transition_at, "ask_update 不得重置倒數");

    // cancel / supersede
    let (_, out) = rpc(
        &client,
        &base,
        rpc_call(2, "tools/call", json!({"name": "ask_cancel", "arguments": {"question_id": "pa-b"}})),
    )
    .await;
    assert_eq!(out.unwrap()["result"]["structuredContent"]["state"], "cancelled");
    let (_, out) = rpc(
        &client,
        &base,
        rpc_call(3, "tools/call", json!({"name": "ask_supersede", "arguments": {"question_id": "pa-c"}})),
    )
    .await;
    assert_eq!(out.unwrap()["result"]["structuredContent"]["state"], "superseded");

    // ask_list：預設只回非終態
    let (_, out) = rpc(&client, &base, rpc_call(4, "tools/call", json!({"name": "ask_list", "arguments": {}}))).await;
    let qs = out.unwrap()["result"]["structuredContent"]["questions"].as_array().unwrap().clone();
    assert_eq!(qs.len(), 1);
    assert_eq!(qs[0]["id"], "pa-a");

    // ask_answers_since：游標收割兩筆終態
    let (_, out) = rpc(
        &client,
        &base,
        rpc_call(5, "tools/call", json!({"name": "ask_answers_since", "arguments": {"consumer": "mkt-daily"}})),
    )
    .await;
    let batch = out.unwrap()["result"]["structuredContent"].clone();
    let events = batch["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert!(batch["next_cursor"].as_i64().unwrap() > 0);
}
